use super::*;
use motion_core::{Color, PixelSize, Point, RasterLimits, Size};
use motion_render::RenderPrepare;

fn tile(id: &str, rect: Rect, color: Color) -> VectorItem {
    let mut path = VectorPath::filled(Path2D::from_rect(rect).unwrap(), color);
    path.id = Some(id.into());
    VectorItem::Path(path)
}

#[test]
fn baseline_builder_isolates_cloned_trees_and_preserves_cached_surface() -> motion_core::Result {
    let rect = Rect::new(0.0, 0.0, 4.0, 4.0);
    let tree = VectorTree::new(
        rect,
        VectorGroup {
            children: vec![tile("tile", rect, Color::WHITE)],
            ..Default::default()
        },
    )?;
    let surface = tree.surface()?;
    let copy = tree.clone().with_baseline(3.0);
    assert_eq!(tree.data.baseline, None);
    assert_eq!(copy.data.baseline, Some(3.0));
    assert!(!Arc::ptr_eq(&tree.data, &copy.data));
    assert!(Arc::ptr_eq(&surface, &copy.surface()?));
    assert_eq!(tree.ids(), copy.ids());
    assert_eq!(copy.clone().with_baseline(2.0).data.baseline, Some(2.0));
    assert_eq!(copy.data.baseline, Some(3.0));
    Ok(())
}

#[test]
fn groups_ids_source_order_affine_and_isolation_share_nodes() -> motion_core::Result<()> {
    let rect = Rect::new(0.0, 0.0, 2.0, 2.0);
    let group = VectorGroup {
        id: Some("root".into()),
        transform: Transform::scale_non_uniform(2.0, 1.0).then(Transform::translate(1.0, 1.0)),
        opacity: 0.5,
        clip: Some(Path2D::from_rect(rect)?.into()),
        children: vec![tile("a", rect, Color::WHITE), tile("b", rect, Color::WHITE)],
        ..Default::default()
    };
    let tree = VectorTree::new(Rect::new(0.0, 0.0, 6.0, 4.0), group)?;
    assert_eq!(tree.ids()["root"], Vec::<usize>::new());
    assert!(matches!(tree.get("a"), Some(VectorItem::Path(_))));
    let node = tree.node();
    assert_eq!(
        node.measure(ValidationLimits::default())?,
        Rect::new(1.0, 1.0, 5.0, 3.0)
    );
    let frame = node
        .compile(
            Size::new(6.0, 4.0),
            PixelSize::new(6, 4),
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), RasterLimits::default())?
        .render_linear(RasterLimits::default())?;
    assert_eq!(frame.linear_pixel(2, 1), Some([0.5; 4]));
    assert_eq!(frame.linear_pixel(0, 0), Some([0.0; 4]));
    assert_eq!(
        node.clone()
            .at(Point::new(9.0, 4.0))
            .measure(ValidationLimits::default())?,
        Rect::new(7.0, 3.0, 11.0, 5.0)
    );
    assert!(node.measure(ValidationLimits::default()).is_ok());
    let duplicate = VectorGroup {
        children: vec![tile("x", rect, Color::WHITE), tile("x", rect, Color::WHITE)],
        ..Default::default()
    };
    assert!(VectorTree::new(rect, duplicate).is_err());
    let mut root = tree.root().clone();
    if let VectorItem::Path(path) = &mut root.children[0] {
        path.source = Some(4..5)
    };
    if let VectorItem::Path(path) = &mut root.children[1] {
        path.source = Some(0..1)
    };
    let ordered = VectorTree::new(rect, root)?;
    assert_eq!(ordered.leaves()[0].address, vec![1]);
    assert_eq!(ordered.paint_leaves()[0].address, vec![0]);
    Ok(())
}

#[cfg(feature = "text")]
#[test]
#[ignore = "requires explicitly supplied MOTION_TEST_FONT; no implicit font discovery"]
fn text_tree_preserves_actual_rich_paragraph_pixels_bounds_and_reading_order()
-> motion_core::Result<()> {
    let path = std::env::var_os("MOTION_TEST_FONT")
        .ok_or_else(|| std::io::Error::other("Set MOTION_TEST_FONT"))?;
    let mut fonts = crate::FontLibrary::new();
    fonts.load_file(path)?;
    if let Some(path) = std::env::var_os("MOTION_TEST_EMOJI_FONT") {
        fonts.load_file(path)?;
    }
    let context = DrawContext::default().with_fonts(fonts.clone());
    let text = crate::Text::rich([
        crate::TextSpan::new("Ab ").fill(Color::rgb(235, 30, 75)),
        crate::TextSpan::new("12\n\u{05d0}\u{05d1}"),
    ])
    .family("Arial")
    .size(18.0)
    .stroke(0.3, Color::WHITE)
    .fonts(&fonts);
    let tree = text.vector_tree(&context)?;
    assert_eq!(
        tree.node().measure(ValidationLimits::default())?,
        text.local_bounds(&context)?
    );
    let render = |node: Node| -> motion_core::Result<motion_render::FrameBuffer> {
        let frame = node
            .compile(
                Size::new(80.0, 40.0),
                PixelSize::new(160, 80),
                ValidationLimits::default(),
            )?
            .prepare(ValidationLimits::default(), RasterLimits::default())?
            .render_linear(RasterLimits::default())?;
        Ok(frame.to_format(
            motion_render::PixelFormat::SrgbRgba8 {
                premultiplied: false,
            },
            RasterLimits::default(),
        )?)
    };
    assert_eq!(
        render(Node::from(text.clone()))?.bytes(),
        render(tree.node())?.bytes()
    );
    assert!(tree.leaves().windows(2).all(
        |pair| pair[0].source.as_ref().unwrap().start <= pair[1].source.as_ref().unwrap().start
    ));
    assert_eq!(
        render(tree.write(1.0, WriteOptions::default())?)?.bytes(),
        render(tree.node())?.bytes()
    );
    assert_eq!(
        render(text.write_with_context(1.0, &context)?)?.bytes(),
        render(tree.node())?.bytes()
    );
    if std::env::var_os("MOTION_TEST_EMOJI_FONT").is_some() {
        let emoji = crate::Text::new("A\u{1f600}")
            .family("Arial")
            .size(18.0)
            .fonts(&fonts);
        let tree = emoji.vector_tree(&context)?;
        assert!(
            tree.paint_leaves()
                .iter()
                .any(|leaf| matches!(leaf.item, VectorItem::Image(_)))
        );
        let reference = render(Node::from(emoji))?;
        let actual = render(tree.node())?;
        assert!(
            reference
                .bytes()
                .iter()
                .zip(actual.bytes())
                .all(|(a, b)| a.abs_diff(*b) <= 1)
        );
    }
    Ok(())
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires actual native GPU; no fonts, window or encoder"]
fn vector_effects_match_cpu_with_clips_images_morphs_and_surface_points() -> motion_core::Result<()>
{
    use motion_core::Shape;
    let mut group = VectorGroup {
        transform: Transform::from_coefficients([1.0, 0.1, 0.2, 1.0, 2.0, 2.0]),
        opacity: 0.6,
        clip: Some(Path2D::from_rect(Rect::new(0.0, 0.0, 20.0, 16.0))?.into()),
        ..Default::default()
    };
    group.children.push(tile(
        "shape",
        Rect::new(1.0, 1.0, 9.0, 9.0),
        Color::rgb(235, 30, 75),
    ));
    let mut picture = VectorImage::new(
        Arc::new(crate::ImageAsset::from_rgba8(
            2,
            1,
            vec![255, 0, 0, 128, 0, 0, 255, 255],
        )?),
        Rect::new(10.0, 2.0, 18.0, 10.0),
    );
    picture.id = Some("picture".into());
    group.children.push(VectorItem::Image(picture));
    let tree = VectorTree::new(Rect::new(0.0, 0.0, 32.0, 24.0), group)?;
    let mut target = tree.root().clone();
    target.transform = Transform::translate(5.0, 4.0);
    target.opacity = 0.8;
    target.children[0] = VectorItem::Path(VectorPath {
        id: Some("shape".into()),
        path: Arc::new(
            motion_core::Star::new(motion_core::Point::new(7.0, 7.0), 5, 6.0, 2.5)?.path()?,
        ),
        fill: Some((Color::rgb(0, 200, 255).into(), FillRule::NonZero)),
        stroke: None,
        source: None,
    });
    target.children.push(tile(
        "birth",
        Rect::new(17.0, 12.0, 21.0, 16.0),
        Color::WHITE,
    ));
    let target = VectorTree::new(tree.view_box(), target)?;
    let morph = Morph::new(&tree, &target)?;
    let limits = RasterLimits::default();
    let mut gpu = motion_render::gpu::Gpu::new()?;
    let mut cases = 0;
    for progress in [0.0, 0.2, 0.5, 0.8, 1.0, 0.5] {
        let points = tree.sample_points(32, 42)?;
        let dots = Node::group(points.iter().map(|point| {
            Node::from(
                motion_core::Circle::new(*point, 0.15)
                    .unwrap()
                    .fill(Color::WHITE),
            )
        }));
        for node in [
            tree.node(),
            tree.write(progress, WriteOptions::default())?,
            morph.sample(progress)?,
            dots,
        ] {
            let prepared = node
                .compile(
                    Size::new(32.0, 24.0),
                    PixelSize::new(64, 48),
                    ValidationLimits::default(),
                )?
                .prepare(ValidationLimits::default(), limits)?;
            let expected = prepared.render_linear(limits)?;
            let actual = gpu.render_linear(&prepared, limits)?;
            let replay = gpu.render_linear(&prepared, limits)?;
            assert_eq!(actual, replay);
            let a = expected.to_format(
                motion_render::PixelFormat::SrgbRgba8 {
                    premultiplied: false,
                },
                limits,
            )?;
            let b = actual.to_format(
                motion_render::PixelFormat::SrgbRgba8 {
                    premultiplied: false,
                },
                limits,
            )?;
            assert!(
                a.bytes()
                    .iter()
                    .zip(b.bytes())
                    .all(|(a, b)| a.abs_diff(*b) <= 1),
                "case={cases},progress={progress}"
            );
            cases += 1;
        }
    }
    assert_eq!(cases, 24);
    Ok(())
}
