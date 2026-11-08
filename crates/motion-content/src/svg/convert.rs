//! Conversion of parsed usvg trees into vector groups, paints, clips, and masks.

use super::*;

pub(super) fn group(
    source: &usvg::Group,
    depth: usize,
    count: &mut usize,
    diagnostics: &mut Vec<SvgDiagnostic>,
) -> motion_core::Result<VectorGroup> {
    *count += 1;
    if depth > 40 || *count > 4096 {
        return Err(unsupported("SVG conversion exceeds depth/item allowance"));
    }
    let mut children = Vec::new();
    for node in source.children() {
        *count += 1;
        if *count > 4096 {
            return Err(unsupported("SVG conversion exceeds 4096 source nodes"));
        }
        match node {
            usvg::Node::Group(child) => children.push(VectorItem::Group(group(
                child,
                depth + 1,
                count,
                diagnostics,
            )?)),
            usvg::Node::Path(path) => children.push(VectorItem::Group(path_group(path)?)),
            usvg::Node::Text(text) => {
                let mut flattened = group(text.flattened(), depth + 1, count, diagnostics)?;
                flattened.id = id(text.id());
                children.push(VectorItem::Group(flattened));
            }
            usvg::Node::Image(image) => {
                let size = image.size();
                let rect = Rect::new(0.0, 0.0, f64::from(size.width()), f64::from(size.height()));
                let child = match image.kind() {
                    usvg::ImageKind::PNG(bytes)
                    | usvg::ImageKind::JPEG(bytes)
                    | usvg::ImageKind::WEBP(bytes) => {
                        let mut item = crate::VectorImage::new(
                            Arc::new(crate::ImageAsset::from_bytes(bytes)?),
                            rect,
                        );
                        if image.rendering_mode() == usvg::ImageRendering::OptimizeSpeed {
                            item.sampling = crate::Sampling::Nearest;
                        }
                        VectorItem::Image(item)
                    }
                    usvg::ImageKind::SVG(tree) => {
                        let mut root = group(tree.root(), depth + 1, count, diagnostics)?;

                        fn namespace(group: &mut VectorGroup, prefix: &str) {
                            if let Some(id) = &mut group.id {
                                *id = format!("{prefix}{id}").into();
                            }
                            for item in &mut group.children {
                                match item {
                                    VectorItem::Group(group) => namespace(group, prefix),
                                    VectorItem::Path(path) => {
                                        if let Some(id) = &mut path.id {
                                            *id = format!("{prefix}{id}").into();
                                        }
                                    }
                                    VectorItem::Image(image) => {
                                        if let Some(id) = &mut image.id {
                                            *id = format!("{prefix}{id}").into();
                                        }
                                    }
                                }
                            }
                        }
                        let owner = if image.id().is_empty() {
                            source.id()
                        } else {
                            image.id()
                        };
                        let prefix = if owner.is_empty() {
                            format!("image-{count}::")
                        } else {
                            format!("{owner}::")
                        };
                        namespace(&mut root, &prefix);
                        VectorItem::Group(root)
                    }
                    _ => return Err(unsupported("Animated SVG image resource is unsupported")),
                };
                children.push(VectorItem::Group(VectorGroup {
                    id: id(image.id()),
                    opacity: if image.is_visible() { 1.0 } else { 0.0 },
                    children: vec![child],
                    ..Default::default()
                }));
            }
        }
    }
    let mut content = VectorGroup {
        children,
        ..Default::default()
    };
    for filter in source.filters() {
        match filter_group(filter, content.clone()) {
            Ok(group) => content = group,
            Err(error) => diagnostics.push(SvgDiagnostic {
                element: id(filter.id()),
                message: error.to_string().into(),
            }),
        }
    }
    if let Some(clip) = source.clip_path() {
        if clip.clip_path().is_none() && clip.root().children().len() == 1 {
            if let usvg::Node::Path(path) = &clip.root().children()[0] {
                let geometry = if path.is_visible() {
                    geometry(path.data())?
                } else {
                    Path2D::new()
                };
                let mut path_clip: motion_core::PathClip = geometry.into();
                path_clip.rule = path
                    .fill()
                    .map_or(FillRule::NonZero, |fill| rule(fill.rule()));
                path_clip.transform =
                    matrix(clip.root().transform()).then(matrix(clip.transform()));
                content = VectorGroup {
                    clip: Some(path_clip),
                    children: vec![VectorItem::Group(content)],
                    ..Default::default()
                };
            } else {
                content = VectorGroup {
                    mask: Some(clip_mask(clip, depth + 1, count, diagnostics)?),
                    children: vec![VectorItem::Group(content)],
                    ..Default::default()
                };
            }
        } else {
            content = VectorGroup {
                mask: Some(clip_mask(clip, depth + 1, count, diagnostics)?),
                children: vec![VectorItem::Group(content)],
                ..Default::default()
            };
        }
    }
    if let Some(mask) = source.mask() {
        content = VectorGroup {
            mask: Some(content_mask(mask, depth + 1, count, diagnostics)?),
            children: vec![VectorItem::Group(content)],
            ..Default::default()
        };
    }
    Ok(VectorGroup {
        id: id(source.id()),
        transform: matrix(source.transform()),
        opacity: f64::from(source.opacity().get()),
        blend_mode: blend(source.blend_mode()),
        isolate: source.isolate(),
        children: vec![VectorItem::Group(content)],
        ..Default::default()
    })
}

pub(super) fn rect(value: usvg::NonZeroRect) -> Rect {
    Rect::new(
        f64::from(value.x()),
        f64::from(value.y()),
        f64::from(value.right()),
        f64::from(value.bottom()),
    )
}

fn resource_tree(root: VectorGroup) -> motion_core::Result<Arc<VectorTree>> {
    let checked = VectorTree::new(Rect::default(), root)?;
    let bounds = checked
        .node()
        .measure(motion_core::ValidationLimits::default())?;
    Ok(Arc::new(VectorTree::new(bounds, checked.root().clone())?))
}

fn clip_mask(
    source: &usvg::ClipPath,
    depth: usize,
    count: &mut usize,
    diagnostics: &mut Vec<SvgDiagnostic>,
) -> motion_core::Result<crate::vector::VectorMask> {
    fn content(
        source: &usvg::Group,
        depth: usize,
        count: &mut usize,
        diagnostics: &mut Vec<SvgDiagnostic>,
    ) -> motion_core::Result<VectorGroup> {
        *count += 1;
        if depth > 40 || *count > 4096 {
            return Err(unsupported("SVG clip resource allowance exhausted"));
        }
        let mut children = Vec::new();
        for node in source.children() {
            *count += 1;
            match node {
                usvg::Node::Path(path) => {
                    let mut native = VectorPath::filled(geometry(path.data())?, Color::WHITE);
                    native.fill.as_mut().unwrap().1 = path
                        .fill()
                        .map_or(FillRule::NonZero, |fill| rule(fill.rule()));
                    children.push(VectorItem::Group(VectorGroup {
                        opacity: if path.is_visible() { 1.0 } else { 0.0 },
                        children: vec![VectorItem::Path(native)],
                        ..Default::default()
                    }));
                }
                usvg::Node::Group(group) => children.push(VectorItem::Group(content(
                    group,
                    depth + 1,
                    count,
                    diagnostics,
                )?)),
                usvg::Node::Text(text) => children.push(VectorItem::Group(content(
                    text.flattened(),
                    depth + 1,
                    count,
                    diagnostics,
                )?)),
                usvg::Node::Image(_) => {
                    return Err(unsupported("Bitmap clipPath children are unsupported"));
                }
            }
        }
        let mut group = VectorGroup {
            transform: matrix(source.transform()),
            children,
            ..Default::default()
        };
        if let Some(clip) = source.clip_path() {
            group.mask = Some(clip_mask(clip, depth + 1, count, diagnostics)?);
        }
        Ok(group)
    }
    let mut root = content(source.root(), depth, count, diagnostics)?;
    root.transform = root.transform.then(matrix(source.transform()));
    if let Some(extra) = source.clip_path() {
        root.mask = Some(clip_mask(extra, depth + 1, count, diagnostics)?);
    }
    let tree = resource_tree(root)?;
    Ok(crate::vector::VectorMask {
        rect: tree.view_box(),
        tree,
        mode: motion_core::MaskMode::Alpha,
    })
}

fn content_mask(
    source: &usvg::Mask,
    depth: usize,
    count: &mut usize,
    diagnostics: &mut Vec<SvgDiagnostic>,
) -> motion_core::Result<crate::vector::VectorMask> {
    let mut root = group(source.root(), depth, count, diagnostics)?;
    if let Some(extra) = source.mask() {
        root.mask = Some(content_mask(extra, depth + 1, count, diagnostics)?);
    }
    Ok(crate::vector::VectorMask {
        tree: resource_tree(root)?,
        rect: rect(source.rect()),
        mode: match source.kind() {
            usvg::MaskType::Alpha => motion_core::MaskMode::Alpha,
            usvg::MaskType::Luminance => motion_core::MaskMode::Luminance,
        },
    })
}

fn blend(value: usvg::BlendMode) -> motion_core::BlendMode {
    use motion_core::BlendMode as Native;
    use usvg::BlendMode::*;
    match value {
        Normal => Native::SourceOver,
        Multiply => Native::Multiply,
        Screen => Native::Screen,
        Overlay => Native::Overlay,
        Darken => Native::Darken,
        Lighten => Native::Lighten,
        ColorDodge => Native::ColorDodge,
        ColorBurn => Native::ColorBurn,
        HardLight => Native::HardLight,
        SoftLight => Native::SoftLight,
        Difference => Native::Difference,
        Exclusion => Native::Exclusion,
        Hue => Native::Hue,
        Saturation => Native::Saturation,
        Color => Native::Color,
        Luminosity => Native::Luminosity,
        PlusLighter => Native::Plus,
    }
}

fn rule(value: usvg::FillRule) -> FillRule {
    match value {
        usvg::FillRule::NonZero => FillRule::NonZero,
        usvg::FillRule::EvenOdd => FillRule::EvenOdd,
    }
}

fn spread(value: usvg::SpreadMethod) -> motion_core::GradientExtend {
    match value {
        usvg::SpreadMethod::Pad => motion_core::GradientExtend::Pad,
        usvg::SpreadMethod::Repeat => motion_core::GradientExtend::Repeat,
        usvg::SpreadMethod::Reflect => motion_core::GradientExtend::Reflect,
    }
}

fn painted(
    path: Path2D,
    source: &usvg::Paint,
    opacity: f64,
    fill_rule: FillRule,
) -> motion_core::Result<VectorItem> {
    use motion_core::{GradientSpace, GradientUnits, Paint};
    let (paint, transform) = match source {
        usvg::Paint::Color(color) => (
            Paint::Solid(Color::rgb(color.red, color.green, color.blue).with_alpha(opacity)),
            Transform::IDENTITY,
        ),
        usvg::Paint::LinearGradient(source) => {
            let mut gradient = motion_core::LinearGradient::new(
                Point::new(f64::from(source.x1()), f64::from(source.y1())),
                Point::new(f64::from(source.x2()), f64::from(source.y2())),
            )
            .space(GradientSpace::Srgb)
            .units(GradientUnits::UserSpace)
            .extend(spread(source.spread_method()));
            for stop in source.stops() {
                let color = stop.color();
                gradient = gradient.stop(
                    f64::from(stop.offset().get()),
                    Color::rgb(color.red, color.green, color.blue)
                        .with_alpha(opacity * f64::from(stop.opacity().get())),
                );
            }
            (gradient.into(), matrix(source.transform()))
        }
        usvg::Paint::RadialGradient(source) => {
            if source.fr().get() != 0.0 {
                return Err(unsupported(
                    "SVG radial gradients with nonzero focal radius are unsupported",
                ));
            }
            let center = Point::new(f64::from(source.cx()), f64::from(source.cy()));
            let radius = f64::from(source.r().get());
            let mut focus = Point::new(f64::from(source.fx()), f64::from(source.fy()));
            let distance = center.distance(focus);
            if distance >= radius {
                focus = center + (focus - center) * (radius * (1.0 - 1e-6) / distance);
            }
            let mut gradient = motion_core::RadialGradient::new(center, radius)
                .focal(focus)
                .space(GradientSpace::Srgb)
                .units(GradientUnits::UserSpace)
                .extend(spread(source.spread_method()));
            for stop in source.stops() {
                let color = stop.color();
                gradient = gradient.stop(
                    f64::from(stop.offset().get()),
                    Color::rgb(color.red, color.green, color.blue)
                        .with_alpha(opacity * f64::from(stop.opacity().get())),
                );
            }
            (gradient.into(), matrix(source.transform()))
        }
        usvg::Paint::Pattern(_) => {
            return Err(unsupported(
                "SVG vector pattern paints are unsupported; no raster substitute is used",
            ));
        }
    };
    let inverse = transform
        .inverse()
        .ok_or_else(|| unsupported("Noninvertible SVG paint transform"))?;
    let mut item = VectorPath::filled(path.transformed(&inverse)?, paint);
    item.fill.as_mut().unwrap().1 = fill_rule;
    Ok(VectorItem::Group(VectorGroup {
        transform,
        children: vec![VectorItem::Path(item)],
        ..Default::default()
    }))
}

fn path_group(source: &usvg::Path) -> motion_core::Result<VectorGroup> {
    let path = geometry(source.data())?;
    let mut children = Vec::new();
    let fill = source
        .fill()
        .map(|fill| {
            painted(
                path.clone(),
                fill.paint(),
                f64::from(fill.opacity().get()),
                rule(fill.rule()),
            )
        })
        .transpose()?;
    let stroke = source
        .stroke()
        .map(|stroke| -> motion_core::Result<VectorItem> {
            let mut geometry = path.clone();
            if let Some(pattern) = stroke.dasharray() {
                geometry = geometry.dashed(
                    &pattern.iter().copied().map(f64::from).collect::<Vec<_>>(),
                    f64::from(stroke.dashoffset()),
                )?;
            }
            let style = motion_core::StrokeStyle {
                width: f64::from(stroke.width().get()),
                cap: match stroke.linecap() {
                    usvg::LineCap::Butt => motion_core::LineCap::Butt,
                    usvg::LineCap::Round => motion_core::LineCap::Round,
                    usvg::LineCap::Square => motion_core::LineCap::Square,
                },
                join: match stroke.linejoin() {
                    usvg::LineJoin::Miter => motion_core::LineJoin::Miter {
                        limit: f64::from(stroke.miterlimit().get()),
                    },
                    usvg::LineJoin::Round => motion_core::LineJoin::Round,
                    usvg::LineJoin::Bevel => motion_core::LineJoin::Bevel,
                    usvg::LineJoin::MiterClip => {
                        return Err(unsupported("SVG miter-clip joins are unsupported"));
                    }
                },
            };
            painted(
                style.outline(&geometry, "svg.stroke")?,
                stroke.paint(),
                f64::from(stroke.opacity().get()),
                FillRule::NonZero,
            )
        })
        .transpose()?;
    if source.paint_order() == usvg::PaintOrder::StrokeAndFill {
        children.extend(stroke);
        children.extend(fill);
    } else {
        children.extend(fill);
        children.extend(stroke);
    }
    Ok(VectorGroup {
        id: id(source.id()),
        opacity: if source.is_visible() { 1.0 } else { 0.0 },
        children,
        ..Default::default()
    })
}
