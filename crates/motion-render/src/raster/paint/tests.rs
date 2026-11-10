use super::*;
use motion_core::Shape;
use motion_core::drawing::{
    GradientExtend, GradientSpace, ImagePattern, LinearGradient, RadialGradient, SweepGradient,
};
use motion_core::{Color, DisplayList};
use motion_core::{DrawCommand, Node, PixelSize, Point, Sampling, Size, ValidationLimits};

#[test]
fn row_sampling_matches_scalar_geometry_and_mapping_bits() -> motion_core::Result {
    use motion_core::drawing::paint::Geometry;
    use motion_core::{Color, GradientUnits};
    for geometry in [
        Geometry::Linear(Point::new(0.1, 0.2), Point::new(0.9, 0.8)),
        Geometry::Radial(Point::new(0.5, 0.5), 0.7),
        Geometry::RadialFocal(Point::new(0.5, 0.5), 0.7, Point::new(0.4, 0.6)),
        Geometry::Sweep(Point::new(0.5, 0.5), -1.0, 4.0),
    ] {
        for units in [GradientUnits::UserSpace, GradientUnits::ObjectBoundingBox] {
            for extend in [
                GradientExtend::Pad,
                GradientExtend::Repeat,
                GradientExtend::Reflect,
            ] {
                for transform in [
                    Transform::IDENTITY,
                    Transform::scale_non_uniform(-0.5, 2.0).then(Transform::translate(12.25, -3.0)),
                    Transform::rotate(0.37),
                    Transform::from_coefficients([1.0, 0.25, 0.5, 1.0, 0.25, 0.5]),
                ] {
                    let gradient = Gradient {
                        geometry: geometry.clone(),
                        units,
                        extend,
                        space: GradientSpace::LinearRgb,
                        stops: std::sync::Arc::from([
                            (0.0, Color::BLACK),
                            (0.5, Color::WHITE),
                            (0.5, Color::rgb(20, 120, 200)),
                            (1.0, Color::WHITE),
                        ]),
                    };
                    let source = Source::new(
                        &Paint::Linear(LinearGradient(gradient)),
                        Rect::new(-1.0, -2.0, 81.0, 37.0),
                        transform,
                        None,
                    )?;
                    let [xx, yx, xy, yy, _, _] = source.inverse.coefficients();
                    for row in [0_u32, 1, 17, 103] {
                        let mut sampling = PaintRow {
                            source: Some(&source),
                            solid: [0.0; 4],
                            coefficients: source.inverse.coefficients(),
                            vertical: [xy * (f64::from(row) + 0.5), yy * (f64::from(row) + 0.5)],
                            horizontal: [xx * 0.5, yx * 0.5],
                            column: 0,
                            row,
                            ready: true,
                            incremental: [xx, yx].iter().all(|value| {
                                (value * 65_536.0).fract() == 0.0 && value.abs() <= 16_384.0
                            }),
                        };
                        for column in (0..256).chain([511, 4095, 1_048_576]) {
                            assert_eq!(
                                sampling.sample(column)?.map(f32::to_bits),
                                source
                                    .sample(Point::new(
                                        f64::from(column) + 0.5,
                                        f64::from(row) + 0.5
                                    ))?
                                    .map(f32::to_bits),
                                "{geometry:?} {units:?} {extend:?} {transform:?} ({column},{row})"
                            );
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
fn static_gradient_tables_share_across_mapping_but_not_palette_and_stay_bounded()
-> Result<(), ValidationError> {
    let paint: Paint = motion_core::LinearGradient::new(Point::ORIGIN, Point::new(8.0, 0.0))
        .stop(0.0, motion_core::Color::WHITE)
        .stop(1.0, motion_core::Color::BLACK)
        .into();
    let bounds = Rect::new(0.0, 0.0, 8.0, 4.0);
    let mut cache = TableCache::default();
    let first = Source::new(&paint, bounds, Transform::IDENTITY, Some(&mut cache))?;
    let shifted = Source::new(
        &paint,
        bounds,
        Transform::translate(3.0, 2.0),
        Some(&mut cache),
    )?;
    assert!(std::sync::Arc::ptr_eq(
        &first.gradient_table,
        &shifted.gradient_table
    ));
    let reference = Source::new(&paint, bounds, Transform::translate(3.0, 2.0), None)?;
    assert_eq!(shifted.gradient_table, reference.gradient_table);
    for column in 0..12 {
        assert_eq!(
            shifted.sample(Point::new(f64::from(column), 3.0))?,
            reference.sample(Point::new(f64::from(column), 3.0))?
        );
    }
    let relocated: Paint =
        motion_core::LinearGradient::new(Point::new(2.0, 1.0), Point::new(6.0, 2.0))
            .stop(0.0, motion_core::Color::WHITE)
            .stop(1.0, motion_core::Color::BLACK)
            .into();
    assert!(std::sync::Arc::ptr_eq(
        &first.gradient_table,
        &Source::new(&relocated, bounds, Transform::IDENTITY, Some(&mut cache))?.gradient_table
    ));
    let invalid: Paint = motion_core::LinearGradient::new(Point::ORIGIN, Point::new(8.0, 0.0))
        .stop(f64::NAN, motion_core::Color::WHITE)
        .stop(1.0, motion_core::Color::BLACK)
        .into();
    assert!(Source::new(&invalid, bounds, Transform::IDENTITY, Some(&mut cache)).is_err());
    for index in 0..65 {
        let changed: Paint = motion_core::LinearGradient::new(Point::ORIGIN, Point::new(8.0, 0.0))
            .stop(0.0, motion_core::Color::rgb(index, 120, 180))
            .stop(1.0, motion_core::Color::BLACK)
            .into();
        let source = Source::new(&changed, bounds, Transform::IDENTITY, Some(&mut cache))?;
        assert!(!std::sync::Arc::ptr_eq(
            &first.gradient_table,
            &source.gradient_table
        ));
        assert!(cache.entries.len() <= 64 && cache.retained_bytes() <= 8 * 1024 * 1024);
    }
    let restored = Source::new(&paint, bounds, Transform::IDENTITY, Some(&mut cache))?;
    assert_eq!(restored.gradient_table, first.gradient_table);
    assert!(!std::sync::Arc::ptr_eq(
        &restored.gradient_table,
        &first.gradient_table
    ));
    Ok(())
}

#[test]
fn paints_validate_hidden_commands_budget_failures_and_composition_blend_space()
-> motion_core::Result {
    use crate::Composition;
    use motion_core::{BlendSpace, Color, DrawContext, NodeModifiers};
    let invalid_gradient = LinearGradient::new(Point::ORIGIN, Point::ORIGIN)
        .stop(0.0, Color::BLACK)
        .stop(1.0, Color::WHITE);
    assert!(
        Node::from(Rect::new(0.0, 0.0, 8.0, 8.0).fill(invalid_gradient.clone()))
            .opacity(0.0)
            .compile(
                Size::new(8.0, 8.0),
                PixelSize::new(8, 8),
                ValidationLimits::default()
            )
            .is_err()
    );
    let list = DisplayList {
        output_size: PixelSize::new(8, 8),
        background: None,
        commands: vec![DrawCommand::FillRect {
            bounds: Rect::new(0.0, 0.0, 8.0, 8.0),
            transform: Transform::IDENTITY,
            paint: invalid_gradient.into(),
        }],
    };
    assert!(list.preflight(ValidationLimits::default()).is_err());
    let valid = LinearGradient::new(Point::ORIGIN, Point::new(1.0, 0.0))
        .stop(0.0, Color::BLACK)
        .stop(1.0, Color::WHITE);
    let shape = Rect::new(0.0, 0.0, 8.0, 8.0).fill(valid);
    assert!(
        !DrawContext::default()
            .record(&shape, ValidationLimits::default())?
            .is_empty()
    );
    let list = Node::from(shape).compile(
        Size::new(8.0, 8.0),
        PixelSize::new(8, 8),
        ValidationLimits::default(),
    )?;
    assert!(
        list.prepare(
            ValidationLimits::default(),
            RasterLimits {
                max_bytes: 4096,
                ..RasterLimits::default()
            }
        )
        .is_err()
    );
    assert!(
        list.prepare(
            ValidationLimits::default(),
            RasterLimits {
                max_work: 100,
                ..RasterLimits::default()
            }
        )
        .is_err()
    );
    let scene = || Node::from(Rect::new(0.0, 0.0, 2.0, 2.0).fill(Color::WHITE.with_alpha(0.5)));
    let linear = Composition::new(2, 2, crate::FrameClock::new(30, 150))
        .with_options(crate::RenderOptions::default().background(Color::BLACK))?
        .render_frame(0, Size::new(2.0, 2.0), |_| scene())?;
    let encoded = Composition::new(2, 2, crate::FrameClock::new(30, 150))
        .with_options(
            crate::RenderOptions::default()
                .background(Color::BLACK)
                .blend_space(BlendSpace::Srgb),
        )?
        .render_frame(0, Size::new(2.0, 2.0), |_| scene())?;
    assert_ne!(linear.pixels(), encoded.pixels());
    assert!((127..=129).contains(&((encoded.pixels()[0] >> 16) & 255)));
    assert!(
        Composition::new(2, 2, crate::FrameClock::new(30, 150))
            .with_options(crate::RenderOptions::default().blend_space(BlendSpace::Srgb))?
            .frame_renderer(Size::new(2.0, 2.0))
            .is_ok()
    );
    let collapsed = Node::from(
        Rect::new(0.0, 0.0, 8.0, 8.0).fill(
            LinearGradient::new(Point::ORIGIN, Point::new(1.0, 0.0))
                .stop(0.0, Color::BLACK)
                .stop(1.0, Color::WHITE),
        ),
    )
    .scale(0.0)
    .compile(
        Size::new(8.0, 8.0),
        PixelSize::new(8, 8),
        ValidationLimits::default(),
    )?
    .prepare(ValidationLimits::default(), RasterLimits::default())?
    .render_linear(RasterLimits::default())?;
    assert_eq!(collapsed.pixel(0, 0), Some([0.0; 4]));
    let empty = Node::from(
        Rect::new(0.0, 0.0, 0.0, 8.0).fill(
            LinearGradient::new(Point::ORIGIN, Point::new(1.0, 0.0))
                .stop(0.0, Color::BLACK)
                .stop(1.0, Color::WHITE),
        ),
    )
    .compile(
        Size::new(8.0, 8.0),
        PixelSize::new(8, 8),
        ValidationLimits::default(),
    )?
    .prepare(ValidationLimits::default(), RasterLimits::default())?
    .render_linear(RasterLimits::default())?;
    assert_eq!(empty.pixel(0, 0), Some([0.0; 4]));
    Ok(())
}

#[test]
#[cfg(feature = "gpu")]
#[ignore = "requires native GPU; no fonts, window or encoder"]
fn spatial_paints_match_gpu_and_replay() -> motion_core::Result {
    use motion_core::{GradientUnits, NodeModifiers};
    let limits = RasterLimits::default();
    let mut gpu = crate::gpu::Gpu::new()?;
    let info = gpu.info().clone();
    let mut paints = Vec::new();
    for space in [
        GradientSpace::Oklab,
        GradientSpace::LinearRgb,
        GradientSpace::Srgb,
    ] {
        for units in [GradientUnits::ObjectBoundingBox, GradientUnits::UserSpace] {
            for extend in [
                GradientExtend::Pad,
                GradientExtend::Repeat,
                GradientExtend::Reflect,
            ] {
                let first = Color::from_rgb_float(0.1, 0.3, 1.1).with_alpha(0.8);
                let second = Color::from_rgb_float(1.0, 0.2, 0.1).with_alpha(0.4);
                paints.push(Paint::from(
                    LinearGradient::new(Point::ORIGIN, Point::new(1.0, 0.7))
                        .stop(0.0, first)
                        .stop(1.0, second)
                        .space(space)
                        .units(units)
                        .extend(extend),
                ));
                paints.push(Paint::from(
                    RadialGradient::new(Point::new(0.5, 0.5), 0.5)
                        .stop(0.0, first)
                        .stop(1.0, second)
                        .space(space)
                        .units(units)
                        .extend(extend),
                ));
                paints.push(Paint::from(
                    SweepGradient::new(Point::new(0.5, 0.5), -1.0, 4.0)
                        .stop(0.0, first)
                        .stop(1.0, second)
                        .space(space)
                        .units(units)
                        .extend(extend),
                ));
            }
        }
    }
    let image = std::sync::Arc::new(motion_content::ImageAsset::from_rgba8(
        2,
        2,
        vec![
            255, 0, 0, 255, 0, 255, 0, 0, 0, 0, 255, 128, 255, 255, 255, 255,
        ],
    )?);
    for extend in [
        GradientExtend::Pad,
        GradientExtend::Repeat,
        GradientExtend::Reflect,
    ] {
        paints.push(
            ImagePattern::from_asset(image.clone())
                .sampling(Sampling::Bilinear)
                .transform(Transform::scale(1.5))
                .extend(extend)
                .into(),
        );
    }
    let mut maximum = 0.0_f32;
    for (paint, filtered) in paints
        .iter()
        .map(|paint| (paint, false))
        .chain(paints.iter().take(3).map(|paint| (paint, true)))
    {
        let mut node = Node::from(Rect::new(1.0, 1.0, 7.0, 7.0).fill(paint.clone()))
            .rotate(0.12)
            .translate(0.1, -0.2);
        if filtered {
            node = node
                .filter(motion_core::Filter::Blur { sigma: 1.25 })
                .clip_with(
                    Rect::new(0.5, 0.5, 7.5, 7.5),
                    motion_core::ClipMode::Geometric,
                )
                .opacity(0.6);
        }
        let prepared = node
            .compile(
                Size::new(8.0, 8.0),
                PixelSize::new(8, 8),
                ValidationLimits::default(),
            )?
            .prepare(ValidationLimits::default(), limits)?;
        let cpu = prepared.render_linear(limits)?;
        let output = gpu.render(&prepared, crate::gpu::Plan::new(&prepared, limits)?)?;
        for row in 0..8 {
            for column in 0..8 {
                for (first, second) in cpu
                    .pixel(column, row)
                    .unwrap()
                    .into_iter()
                    .zip(output.pixel(column, row).unwrap())
                {
                    maximum = maximum.max((first - second).abs());
                }
            }
        }
        let cpu = cpu.to_srgb_rgba8(false, limits)?;
        let output = output.to_srgb_rgba8(false, limits)?;
        for (first, second) in cpu.pixels().iter().zip(output.pixels()) {
            for (first, second) in first.iter().zip(second) {
                assert!(first.abs_diff(*second) <= 1);
            }
        }
        let replay = gpu
            .render(&prepared, crate::gpu::Plan::new(&prepared, limits)?)?
            .to_srgb_rgba8(false, limits)?;
        assert_eq!(output.pixels(), replay.pixels());
    }
    assert!(
        maximum <= 5e-6,
        "maximum spatial paint difference {maximum}"
    );
    eprintln!(
        "_spatial_paints adapter={} backend={:?} cases={} max_linear_difference={maximum} rgba_tolerance=1 replay=exact",
        info.name,
        info.backend,
        paints.len() + 3
    );
    Ok(())
}

#[test]
fn spatial_paints_render_analytic_pixels_and_preserve_linear_image_alpha()
-> Result<(), ValidationError> {
    let limits = RasterLimits::default();
    let bounds = Rect::new(0.0, 0.0, 8.0, 8.0);
    let gradient = LinearGradient::new(Point::ORIGIN, Point::new(1.0, 0.0))
        .stop(0.0, Color::BLACK)
        .stop(1.0, Color::WHITE)
        .space(GradientSpace::LinearRgb);
    let prepared = Node::from(bounds.fill(gradient))
        .compile(
            Size::new(8.0, 8.0),
            PixelSize::new(8, 8),
            ValidationLimits::default(),
        )?
        .prepare(ValidationLimits::default(), limits)?;
    assert_eq!(
        prepared.render_linear(limits)?.pixel(3, 3),
        Some([0.4375, 0.4375, 0.4375, 1.0])
    );
    let image = std::sync::Arc::new(
        motion_content::ImageAsset::from_rgba8(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 0])
            .map_err(|_| ValidationError::invalid("paint", "image", "fixture failed"))?,
    );
    let prepared = Node::from(
        bounds.fill(
            ImagePattern::from_asset(image)
                .sampling(Sampling::Bilinear)
                .extend(GradientExtend::Pad),
        ),
    )
    .compile(
        Size::new(8.0, 8.0),
        PixelSize::new(8, 8),
        ValidationLimits::default(),
    )?
    .prepare(ValidationLimits::default(), limits)?;
    assert_eq!(
        prepared.paint_at(0, Point::new(1.0, 0.5))?,
        [0.5, 0.0, 0.0, 0.5]
    );
    assert_eq!(
        prepared.render_linear(limits)?.pixel(0, 0),
        Some([1.0, 0.0, 0.0, 1.0])
    );
    let _protocol = [
        Paint::from(RadialGradient::new(Point::new(0.5, 0.5), 0.5)),
        Paint::from(SweepGradient::new(
            Point::ORIGIN,
            0.0,
            std::f64::consts::TAU,
        )),
    ];
    let _commands: Vec<DrawCommand> = Vec::new();
    Ok(())
}
