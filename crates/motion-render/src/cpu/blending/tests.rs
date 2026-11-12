use super::*;
use crate::PreparedCpuFrame;
use crate::prepared::PreparedItem;
use crate::raster::prepare::RenderPrepare;
use motion_core::Shape;
use motion_core::{
    BlendMode, Color, DrawCommand, Node, NodeModifiers, PixelSize, RasterLimits, Rect, Size,
    ValidationErrorKind, ValidationLimits,
};

#[test]
#[cfg(feature = "gpu")]
#[ignore = "requires native GPU; no fonts, window or encoder"]
fn all_blend_spaces_modes_and_box_filters_match_gpu() -> motion_core::Result {
    use motion_core::{BlendSpace, Filter, FilterOperation, Vector2};
    let limits = RasterLimits::default();
    let mut gpu = crate::gpu::Gpu::new()?;
    let info = gpu.info().clone();
    let modes = [
        BlendMode::SourceOver,
        BlendMode::Plus,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Overlay,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ];
    let filters = [
        Filter::Blur { sigma: 2.0 },
        Filter::DropShadow {
            offset: Vector2::new(0.5, 0.5),
            sigma: 2.0,
            color: Color::from_hex(0x1080c0),
        },
        Filter::Glow {
            threshold: 0.2,
            sigma: 2.0,
            intensity: 0.5,
        },
        Filter::Custom(vec![
            FilterOperation::Blur { sigma: 2.0 },
            FilterOperation::CompositeWithSource {
                mode: BlendMode::Hue,
            },
        ]),
    ];
    let mut maximum = 0.0_f32;
    let mut cases = 0;
    for space in [BlendSpace::LinearRgb, BlendSpace::Srgb] {
        let backdrop = Node::from(
            Rect::new(0.5, 0.5, 7.5, 7.5)
                .fill(Color::from_rgb_float(0.1, 0.6, 0.3).with_alpha(0.8)),
        );
        let foreground = Node::from(
            Rect::new(2.0, 2.0, 6.0, 6.0)
                .fill(Color::from_rgb_float(0.9, 0.2, 0.5).with_alpha(0.6)),
        );
        let mut scenes = modes
            .into_iter()
            .map(|mode| Node::group([backdrop.clone(), foreground.clone().blend_mode(mode)]))
            .collect::<Vec<_>>();
        scenes.extend(filters.iter().map(|filter| {
            Node::group([backdrop.clone(), foreground.clone().filter(filter.clone())])
        }));
        for scene in scenes {
            let prepared = scene
                .compile(
                    Size::new(8.0, 8.0),
                    PixelSize::new(8, 8),
                    ValidationLimits::default(),
                )?
                .prepare(ValidationLimits::default(), limits)?
                .blend_space(space);
            let cpu = prepared.compose_linear(limits)?;
            let rendered = gpu.render(&prepared, crate::gpu::Plan::new(&prepared, limits)?)?;
            for row in 0..8 {
                for column in 0..8 {
                    for (first, second) in cpu
                        .pixel(column, row)
                        .unwrap()
                        .into_iter()
                        .zip(rendered.pixel(column, row).unwrap())
                    {
                        maximum = maximum.max((first - second).abs());
                    }
                }
            }
            let cpu = cpu.to_rgba8_frame(false, limits)?;
            let rendered = rendered.to_rgba8_frame(false, limits)?;
            for (first, second) in cpu.pixels().iter().zip(rendered.pixels()) {
                for (first, second) in first.iter().zip(second) {
                    assert!(first.abs_diff(*second) <= 1);
                }
            }
            let replay = gpu
                .render(&prepared, crate::gpu::Plan::new(&prepared, limits)?)?
                .to_rgba8_frame(false, limits)?;
            assert_eq!(rendered.pixels(), replay.pixels());
            cases += 1;
        }
    }
    assert!(maximum <= 5e-6, "blend/filter difference {maximum}");
    eprintln!(
        "_blend_filters adapter={} backend={:?} cases={cases} max_linear_difference={maximum} rgba_tolerance=1 replay=exact",
        info.name, info.backend
    );
    Ok(())
}

#[test]
fn encoded_space_has_independent_source_over_result_and_keeps_transparent_pixels_exact() {
    use motion_core::drawing::BlendSpace;
    let mut linear = [0.0, 0.0, 0.0, 1.0];
    let mut encoded = linear;
    covered(&mut linear, [1.0; 4], 0.5, BlendSpace::LinearRgb, "test").unwrap();
    covered(&mut encoded, [1.0; 4], 0.5, BlendSpace::Srgb, "test").unwrap();
    assert_eq!(linear, [0.5, 0.5, 0.5, 1.0]);
    for channel in &encoded[..3] {
        assert!((f64::from(*channel) - 0.21404114048223255).abs() < 1e-7);
    }
    let before = encoded;
    composite_in_space(
        &mut encoded,
        [0.0; 4],
        BlendMode::Hue,
        BlendSpace::Srgb,
        "test",
    )
    .unwrap();
    assert_eq!(encoded, before);
}

const MODES: [BlendMode; 13] = [
    BlendMode::SourceOver,
    BlendMode::Plus,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::Difference,
    BlendMode::Exclusion,
    BlendMode::Overlay,
    BlendMode::HardLight,
    BlendMode::SoftLight,
    BlendMode::ColorDodge,
    BlendMode::ColorBurn,
];

fn prepared(node: &Node) -> PreparedCpuFrame {
    node.compile(
        Size::new(2.0, 1.0),
        PixelSize::new(2, 1),
        ValidationLimits::default(),
    )
    .unwrap()
    .prepare(ValidationLimits::default(), RasterLimits::default())
    .unwrap()
}

fn tile(color: Color) -> Node {
    Rect::new(0.0, 0.0, 2.0, 1.0).fill(color).into()
}

fn reference(destination: [f32; 4], source: [f32; 4], mode: BlendMode) -> [f64; 4] {
    let source_alpha = f64::from(source[3]);
    let backdrop_alpha = f64::from(destination[3]);
    let mut result = [0.0; 4];
    for channel in 0..3 {
        let source_straight = if source_alpha == 0.0 {
            0.0
        } else {
            f64::from(source[channel]) / source_alpha
        };
        let backdrop_straight = if backdrop_alpha == 0.0 {
            0.0
        } else {
            f64::from(destination[channel]) / backdrop_alpha
        };
        let mixed = match mode {
            BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity => {
                unreachable!("separable reference does not test these modes")
            }
            BlendMode::SourceOver => source_straight,
            BlendMode::LinearPlus => return reference(destination, source, BlendMode::Plus),
            BlendMode::Plus => {
                result[channel] = f64::from(source[channel]) + f64::from(destination[channel]);
                continue;
            }
            BlendMode::Multiply => source_straight * backdrop_straight,
            BlendMode::Screen => 1.0 - (1.0 - source_straight) * (1.0 - backdrop_straight),
            BlendMode::Darken => {
                if source_straight < backdrop_straight {
                    source_straight
                } else {
                    backdrop_straight
                }
            }
            BlendMode::Lighten => {
                if source_straight > backdrop_straight {
                    source_straight
                } else {
                    backdrop_straight
                }
            }
            BlendMode::Difference => {
                source_straight.max(backdrop_straight) - source_straight.min(backdrop_straight)
            }
            BlendMode::Exclusion => {
                source_straight * (1.0 - backdrop_straight)
                    + backdrop_straight * (1.0 - source_straight)
            }
            BlendMode::Overlay | BlendMode::HardLight => {
                let (pivot, other) = if mode == BlendMode::Overlay {
                    (backdrop_straight, source_straight)
                } else {
                    (source_straight, backdrop_straight)
                };
                if pivot <= 0.5 {
                    other * (2.0 * pivot)
                } else {
                    let highlighted = 2.0 * pivot - 1.0;
                    1.0 - (1.0 - other) * (1.0 - highlighted)
                }
            }
            BlendMode::SoftLight => {
                if source_straight <= 0.5 {
                    (1.0 - 2.0 * source_straight) * backdrop_straight.powi(2)
                        + 2.0 * source_straight * backdrop_straight
                } else {
                    let curve = if backdrop_straight <= 0.25 {
                        16.0 * backdrop_straight.powi(3) - 12.0 * backdrop_straight.powi(2)
                            + 4.0 * backdrop_straight
                    } else {
                        backdrop_straight.sqrt()
                    };
                    2.0 * (1.0 - source_straight) * backdrop_straight
                        + (2.0 * source_straight - 1.0) * curve
                }
            }
            BlendMode::ColorDodge => {
                if backdrop_straight <= 0.0 {
                    0.0
                } else if source_straight >= 1.0 {
                    1.0
                } else {
                    (backdrop_straight / (1.0 - source_straight)).min(1.0)
                }
            }
            BlendMode::ColorBurn => {
                if backdrop_straight >= 1.0 {
                    1.0
                } else if source_straight <= 0.0 {
                    0.0
                } else {
                    (1.0 - (1.0 - backdrop_straight) / source_straight).max(0.0)
                }
            }
        };
        let replaced_source = (1.0 - backdrop_alpha) * source_straight + backdrop_alpha * mixed;
        result[channel] =
            source_alpha * replaced_source + (1.0 - source_alpha) * f64::from(destination[channel]);
    }
    result[3] = if mode == BlendMode::Plus {
        (source_alpha + backdrop_alpha).min(1.0)
    } else {
        1.0 - (1.0 - source_alpha) * (1.0 - backdrop_alpha)
    };
    result
}

#[test]
fn nonseparable_modes_match_analytic_colors_and_alpha() {
    for (mode, expected) in [
        (
            BlendMode::Hue,
            [1.0, 0.4142857142857143, 0.4142857142857143],
        ),
        (
            BlendMode::Color,
            [1.0, 0.4142857142857143, 0.4142857142857143],
        ),
        (BlendMode::Saturation, [0.0, 1.0, 0.0]),
        (BlendMode::Luminosity, [0.0, 0.3 / 0.59, 0.0]),
    ] {
        let mut opaque = [0.0, 1.0, 0.0, 1.0];
        composite(&mut opaque, [1.0, 0.0, 0.0, 1.0], mode, "test").unwrap();
        for channel in 0..3 {
            assert!((f64::from(opaque[channel]) - expected[channel]).abs() < 1e-6);
        }
        let mut transparent = [0.0, 1.0, 0.0, 1.0];
        composite(&mut transparent, [0.0; 4], mode, "test").unwrap();
        assert_eq!(transparent, [0.0, 1.0, 0.0, 1.0]);
        let mut partial = [0.0, 1.0, 0.0, 1.0];
        composite(&mut partial, [0.5, 0.0, 0.0, 0.5], mode, "test").unwrap();
        for channel in 0..3 {
            let backdrop = if channel == 1 { 1.0 } else { 0.0 };
            assert!(
                (f64::from(partial[channel]) - (expected[channel] + backdrop) * 0.5).abs() < 1e-6
            );
        }
    }
}

#[test]
fn semitransparent_modes_match_analytic_linear_results() {
    for (mode, expected) in [
        (BlendMode::SourceOver, [0.35, 0.25, 0.15, 0.75]),
        (BlendMode::Plus, [0.4, 0.35, 0.3, 1.0]),
        (BlendMode::Multiply, [0.23, 0.205, 0.15, 0.75]),
        (BlendMode::Screen, [0.37, 0.32, 0.3, 0.75]),
        (BlendMode::Darken, [0.25, 0.25, 0.15, 0.75]),
        (BlendMode::Lighten, [0.35, 0.275, 0.3, 0.75]),
        (BlendMode::Difference, [0.3, 0.2, 0.3, 0.75]),
        (BlendMode::Exclusion, [0.34, 0.29, 0.3, 0.75]),
    ] {
        let mut destination = [0.1, 0.2, 0.3, 0.5];
        composite(&mut destination, [0.3, 0.15, 0.0, 0.5], mode, "test").unwrap();
        for (actual, expected) in destination.into_iter().zip(expected) {
            assert!(
                (actual - expected).abs() <= 1e-7,
                "{mode:?}: {actual} != {expected}"
            );
        }
    }
}

#[test]
fn additional_separable_modes_match_opaque_linear_results() {
    for (mode, expected) in [
        (BlendMode::Overlay, [0.375, 0.625, 0.25, 1.0]),
        (BlendMode::HardLight, [0.625, 0.375, 0.25, 1.0]),
        (BlendMode::SoftLight, [0.375, 0.65625, 0.25, 1.0]),
        (BlendMode::ColorDodge, [1.0, 1.0, 0.5, 1.0]),
        (BlendMode::ColorBurn, [0.0, 0.0, 0.0, 1.0]),
    ] {
        let mut destination = [0.25, 0.75, 0.25, 1.0];
        composite(&mut destination, [0.75, 0.25, 0.5, 1.0], mode, "test").unwrap();
        assert_eq!(destination, expected, "{mode:?}");
    }
}

#[test]
fn additional_separable_modes_match_semitransparent_linear_results() {
    for (mode, expected) in [
        (BlendMode::Overlay, [0.34375, 0.40625, 0.25, 0.75]),
        (BlendMode::HardLight, [0.40625, 0.34375, 0.25, 0.75]),
        (BlendMode::SoftLight, [0.34375, 0.4140625, 0.25, 0.75]),
        (BlendMode::ColorDodge, [0.5, 0.5, 0.3125, 0.75]),
        (BlendMode::ColorBurn, [0.25, 0.25, 0.1875, 0.75]),
    ] {
        let mut destination = [0.125, 0.375, 0.125, 0.5];
        composite(&mut destination, [0.375, 0.125, 0.25, 0.5], mode, "test").unwrap();
        assert_eq!(destination, expected, "{mode:?}");
    }
}

#[test]
fn dodge_and_burn_endpoints_take_precedence_over_singular_ratios() {
    for (mode, backdrop, source, expected) in [
        (
            BlendMode::ColorDodge,
            [0.0, 0.25, 0.75, 1.0],
            [1.0; 4],
            [0.0, 1.0, 1.0, 1.0],
        ),
        (
            BlendMode::ColorBurn,
            [1.0, 0.25, 0.75, 1.0],
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
        ),
    ] {
        let mut destination = backdrop;
        composite(&mut destination, source, mode, "test").unwrap();
        assert_eq!(destination, expected);
    }
}

#[test]
fn soft_light_cubic_square_root_and_half_source_boundaries_are_continuous() {
    for backdrop in [
        0.0_f32,
        0.125,
        0.25_f32.next_down(),
        0.25,
        0.25_f32.next_up(),
        0.5,
        1.0,
    ] {
        for source in [
            0.0_f32,
            0.5_f32.next_down(),
            0.5,
            0.5_f32.next_up(),
            0.75,
            1.0,
        ] {
            let mut actual = [backdrop, backdrop, backdrop, 1.0];
            let expected = reference(actual, [source, source, source, 1.0], BlendMode::SoftLight);
            composite(
                &mut actual,
                [source, source, source, 1.0],
                BlendMode::SoftLight,
                "test",
            )
            .unwrap();
            assert!((f64::from(actual[0]) - expected[0]).abs() < 1e-7);
            if source == 0.5 {
                assert_eq!(actual[0], backdrop);
            }
        }
    }
    let mut actual = [0.125, 0.25, 1.0, 1.0];
    composite(
        &mut actual,
        [0.75, 0.75, 0.75, 1.0],
        BlendMode::SoftLight,
        "test",
    )
    .unwrap();
    assert_eq!(actual, [0.234375, 0.375, 1.0, 1.0]);
}

#[test]
fn additional_modes_extend_rgb_without_global_input_clipping() {
    for (mode, expected) in [
        (BlendMode::Overlay, [4.0, -2.0, 0.375, 1.0]),
        (BlendMode::HardLight, [-2.0, 4.0, 0.625, 1.0]),
        (BlendMode::SoftLight, [6.0, -20.0, 0.375, 1.0]),
        (BlendMode::ColorDodge, [1.0, 0.0, 1.0, 1.0]),
        (BlendMode::ColorBurn, [1.0, 0.25, 0.0, 1.0]),
    ] {
        let mut actual = [2.0, -0.5, 0.25, 1.0];
        composite(&mut actual, [-0.5, 2.0, 0.75, 1.0], mode, "test").unwrap();
        assert_eq!(actual, expected, "{mode:?}");
    }
}

#[test]
fn every_mode_composites_with_saved_filter_input_under_geometric_clipping() {
    for mode in MODES {
        let content = tile(Color::from_hex(0x75b4de).with_alpha(0.6));
        let unfiltered = prepared(&content)
            .compose_linear(RasterLimits::default())
            .unwrap();
        let mut expected = unfiltered.pixel(0, 0).unwrap();
        composite(
            &mut expected,
            unfiltered.pixel(0, 0).unwrap(),
            mode,
            "reference",
        )
        .unwrap();
        let node = content
            .filter(motion_core::Filter::Custom(vec![
                motion_core::FilterOperation::CompositeWithSource { mode },
            ]))
            .clip_with(
                Rect::new(0.0, 0.0, 1.0, 1.0),
                motion_core::ClipMode::Geometric,
            );
        let frame = prepared(&node);
        let actual = frame.compose_linear(RasterLimits::default()).unwrap();
        assert_eq!(actual.pixel(0, 0), Some(expected), "{mode:?}");
        assert_eq!(actual.pixel(1, 0), Some([0.0; 4]));
        assert_eq!(
            frame.compose_linear(RasterLimits::default()).unwrap(),
            actual
        );
        assert!(
            node.clone()
                .opacity(0.0)
                .compile(
                    Size::new(2.0, 1.0),
                    PixelSize::new(2, 1),
                    ValidationLimits::default()
                )
                .is_ok()
        );
    }
}

#[test]
fn node_mode_survives_commands_and_preparation() {
    let tile = || Node::from(Rect::new(0.0, 0.0, 1.0, 1.0).fill(Color::WHITE)).opacity(0.5);
    let node = Node::group([tile(), tile().blend_mode(BlendMode::Multiply)]);
    let list = node
        .compile(
            Size::new(1.0, 1.0),
            PixelSize::new(1, 1),
            ValidationLimits::default(),
        )
        .unwrap();
    assert!(list.commands.iter().any(|command| matches!(
        command,
        DrawCommand::PushLayer {
            blend_mode: BlendMode::Multiply,
            ..
        }
    )));
    let prepared = list
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    assert!(prepared.items.iter().any(|item| matches!(
        item,
        PreparedItem::PushLayer {
            layer: crate::prepared::PreparedLayer {
                blend_mode: BlendMode::Multiply,
                ..
            }
        }
    )));
    assert_eq!(
        prepared
            .compose_linear(RasterLimits::default())
            .unwrap()
            .pixel(0, 0),
        Some([0.75; 4])
    );
    assert!(node.measure(ValidationLimits::default()).is_ok());
}

#[test]
fn alpha_endpoints_and_generated_pixels_match_straight_color_reference() {
    let alphas = [
        0.0,
        f32::from_bits(1),
        f32::MIN_POSITIVE,
        1.0 / 255.0,
        0.25,
        0.5,
        0.75,
        1.0_f32.next_down(),
        1.0,
    ];
    for mode in MODES {
        for source_alpha in alphas {
            for backdrop_alpha in alphas {
                for color in [[0.0, 0.5, 1.0], [1.0, 0.125, 0.75]] {
                    let source = [
                        color[0] * source_alpha,
                        color[1] * source_alpha,
                        color[2] * source_alpha,
                        source_alpha,
                    ];
                    let destination = [
                        0.75 * backdrop_alpha,
                        0.25 * backdrop_alpha,
                        backdrop_alpha,
                        backdrop_alpha,
                    ];
                    let expected = reference(destination, source, mode);
                    let mut actual = destination;
                    composite(&mut actual, source, mode, "test").unwrap();
                    if backdrop_alpha == 0.0 {
                        assert_eq!(actual, source, "{mode:?}");
                    }
                    if source_alpha == 0.0 {
                        assert_eq!(actual, destination, "{mode:?}");
                    }
                    for (value, expected) in actual.into_iter().zip(expected) {
                        assert!(
                            (f64::from(value) - expected).abs() <= 2e-7,
                            "{mode:?}: {actual:?} != {expected}"
                        );
                    }
                    if mode == BlendMode::SourceOver {
                        let mut original = destination;
                        blend(&mut original, source, 1.0, "test").unwrap();
                        assert_eq!(actual.map(f32::to_bits), original.map(f32::to_bits));
                    }
                }
            }
        }
    }
}

#[test]
fn group_mode_and_opacity_apply_once_against_the_immediate_parent() {
    let black = tile(Color::BLACK);
    let white = tile(Color::WHITE);
    let isolated = Node::group([
        white.clone(),
        Node::group([black.clone(), black.clone()])
            .opacity(0.5)
            .blend_mode(BlendMode::Multiply),
    ]);
    let individual = Node::group([
        white.clone(),
        black.clone().opacity(0.5).blend_mode(BlendMode::Multiply),
        black.clone().opacity(0.5).blend_mode(BlendMode::Multiply),
    ]);
    assert_eq!(
        prepared(&isolated)
            .compose_linear(RasterLimits::default())
            .unwrap()
            .pixel(0, 0),
        Some([0.5, 0.5, 0.5, 1.0])
    );
    assert_eq!(
        prepared(&individual)
            .compose_linear(RasterLimits::default())
            .unwrap()
            .pixel(0, 0),
        Some([0.25, 0.25, 0.25, 1.0])
    );
    let nested = Node::group([
        black,
        Node::group([white.blend_mode(BlendMode::Multiply)]).opacity(0.5),
    ]);
    assert_eq!(
        prepared(&nested)
            .compose_linear(RasterLimits::default())
            .unwrap()
            .pixel(0, 0),
        Some([0.5, 0.5, 0.5, 1.0])
    );
}

#[test]
fn snapshots_default_modes_and_reference_boundaries_remain_explicit() {
    let original = tile(Color::WHITE);
    let changed = original.clone().blend_mode(BlendMode::Multiply);
    let size = PixelSize::new(2, 1);
    let design = Size::new(2.0, 1.0);
    let default = original
        .compile(design, size, ValidationLimits::default())
        .unwrap();
    let explicit = original
        .clone()
        .blend_mode(BlendMode::SourceOver)
        .compile(design, size, ValidationLimits::default())
        .unwrap();
    assert_eq!(default, explicit);
    assert_eq!(default.commands.len(), 1);
    assert!(
        original
            .compile(design, size, ValidationLimits::default())
            .is_ok()
    );
    assert_eq!(
        original.measure(ValidationLimits::default()).unwrap(),
        changed.measure(ValidationLimits::default()).unwrap()
    );
    let list = changed
        .compile(design, size, ValidationLimits::default())
        .unwrap();
    assert_eq!(list.commands.len(), 3);
    assert!(matches!(
        list.commands[0],
        DrawCommand::PushLayer {
            opacity: 1.0,
            blend_mode: BlendMode::Multiply,
            ..
        }
    ));
    let hidden = changed
        .clone()
        .opacity(0.0)
        .compile(design, size, ValidationLimits::default())
        .unwrap();
    assert!(hidden.commands.iter().any(|command| matches!(
        command,
        DrawCommand::PushLayer {
            blend_mode: BlendMode::Multiply,
            ..
        }
    )));
    assert_eq!(
        changed
            .compile(
                design,
                size,
                ValidationLimits {
                    max_nodes: 2,
                    max_depth: 64
                }
            )
            .unwrap_err()
            .field,
        "max_nodes"
    );
    let invalid = changed.opacity(0.0).translate(f64::NAN, 1000.0);
    assert_eq!(
        invalid
            .compile(design, size, ValidationLimits::default())
            .unwrap_err()
            .kind,
        ValidationErrorKind::InvalidValue
    );
    assert_eq!(BlendMode::default(), BlendMode::SourceOver);
}

#[test]
fn every_mode_obeys_exact_and_short_render_limits_and_recovers() {
    for mode in MODES {
        let node = Node::group([
            tile(Color::from_hex(0x204080)),
            tile(Color::WHITE).opacity(0.5).blend_mode(mode),
        ]);
        let frame = prepared(&node);
        let original = frame.clone();
        let rendered = frame.compose_linear(RasterLimits::default()).unwrap();
        let exact = RasterLimits {
            max_pixels: 2,
            max_bytes: rendered.stats().peak_bytes,
            max_work: rendered.stats().work,
        };
        assert_eq!(frame.compose_linear(exact).unwrap(), rendered);
        for (limits, field) in [
            (
                RasterLimits {
                    max_pixels: 1,
                    ..exact
                },
                "max_pixels",
            ),
            (
                RasterLimits {
                    max_bytes: exact.max_bytes - 1,
                    ..exact
                },
                "max_bytes",
            ),
            (
                RasterLimits {
                    max_work: exact.max_work - 1,
                    ..exact
                },
                "max_work",
            ),
        ] {
            let error = frame.compose_linear(limits).unwrap_err();
            assert_eq!(
                (error.kind, error.field.as_str()),
                (ValidationErrorKind::StructuralLimit, field)
            );
            assert_eq!(frame, original);
            assert_eq!(frame.compose_linear(exact).unwrap(), rendered);
        }
    }
}

#[test]
fn hdr_is_not_gamut_clipped_and_overflow_is_transactional() {
    let mut extended = [0.5, 0.25, 0.0, 0.75];
    composite(
        &mut extended,
        [2.0, -0.5, 1.0, 0.5],
        BlendMode::Plus,
        "test",
    )
    .unwrap();
    assert_eq!(extended, [2.5, -0.25, 1.0, 1.0]);
    let mut screen = [0.5, 0.5, 0.5, 1.0];
    composite(
        &mut screen,
        [2.0, -1.0, 0.5, 1.0],
        BlendMode::Screen,
        "test",
    )
    .unwrap();
    assert_eq!(screen, [1.5, 0.0, 0.75, 1.0]);
    for mode in [
        BlendMode::SourceOver,
        BlendMode::Plus,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Exclusion,
        BlendMode::Overlay,
        BlendMode::HardLight,
        BlendMode::SoftLight,
    ] {
        let mut destination = [f32::MAX, 0.0, 0.0, 1.0];
        let original = destination;
        let error = composite(
            &mut destination,
            [f32::MAX, 0.0, 0.0, 0.5],
            mode,
            "test.layer",
        )
        .unwrap_err();
        assert_eq!(
            (error.kind, error.field.as_str(), error.path.as_str()),
            (ValidationErrorKind::InvalidValue, "color", "test.layer")
        );
        assert_eq!(destination, original);
    }
    let hdr = tile(Color::from_rgb_float(1e16, 0.0, 0.0));
    let failing = prepared(&Node::group([
        hdr.clone(),
        hdr.clone().blend_mode(BlendMode::Plus),
    ]));
    let original = failing.clone();
    let error = failing.compose_linear(RasterLimits::default()).unwrap_err();
    assert_eq!(
        (error.kind, error.field.as_str()),
        (ValidationErrorKind::InvalidValue, "color")
    );
    assert_eq!(failing, original);
    assert!(
        prepared(&hdr)
            .compose_linear(RasterLimits::default())
            .is_ok()
    );
}

#[test]
fn transparent_black_source_never_changes_the_backdrop() {
    // `pop_layer` and the Glow pass skip pixels whose source is exactly transparent black, which
    // is only valid while compositing such a source is the identity for every mode and space.
    let modes = [
        BlendMode::SourceOver,
        BlendMode::Plus,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Overlay,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
        BlendMode::LinearPlus,
    ];
    let backdrops = [
        [0.0; 4],
        [0.25, 0.5, 0.75, 1.0],
        [0.1, 0.05, 0.0, 0.4],
        [3.0, 0.0, 1.5, 1.0],
        [0.2, 0.2, 0.2, 0.0],
        [-0.0, 0.0, -0.0, 0.5],
    ];
    for space in [
        motion_core::drawing::BlendSpace::LinearRgb,
        motion_core::drawing::BlendSpace::Srgb,
    ] {
        for mode in modes {
            for backdrop in backdrops {
                // Scaling by a finite factor keeps negative zero equal to zero as well.
                for source in [[0.0; 4], [-0.0; 4]] {
                    let mut destination = backdrop;
                    composite_in_space(&mut destination, source, mode, space, "test").unwrap();
                    assert_eq!(
                        destination.map(f32::to_bits),
                        backdrop.map(f32::to_bits),
                        "{mode:?} {space:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn plus_linear_matches_generic_composite_bit_for_bit() {
    let values = [0.0, -0.0, 0.1, 0.5, 1.0, 3.5, 1.0e30, f32::MAX];
    let alphas = [0.0, 0.25, 0.5, 1.0];
    for backdrop_color in values {
        for source_color in values {
            for backdrop_alpha in alphas {
                for source_alpha in alphas {
                    let backdrop = [backdrop_color, backdrop_color * 0.5, 0.2, backdrop_alpha];
                    let source = [source_color, 0.3, source_color * 0.25, source_alpha];
                    let mut expected = backdrop;
                    let mut actual = backdrop;
                    let generic = composite_in_space(
                        &mut expected,
                        source,
                        BlendMode::Plus,
                        motion_core::drawing::BlendSpace::LinearRgb,
                        "test",
                    );
                    let fast = plus_linear(&mut actual, source, "test");
                    assert_eq!(generic.is_ok(), fast.is_ok());
                    assert_eq!(expected.map(f32::to_bits), actual.map(f32::to_bits));
                }
            }
        }
    }
}
