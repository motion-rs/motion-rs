use super::*;
use crate::prepared::PreparedItem;
use crate::raster::prepare::RenderPrepare;
use crate::{PreparedCpuFrame, RasterLimits};
use motion_core::Shape;
use motion_core::{
    BlendMode, Canvas, Color, DisplayList, DrawCommand, DrawContext, Drawable, MaskMode, Node,
    NodeModifiers, Path2D, PixelSize, Rect, Size, Transform, ValidationError, ValidationErrorKind,
    ValidationLimits,
};

fn tile(width: f64, height: f64, color: Color) -> Node {
    Rect::new(0.0, 0.0, width, height).fill(color).into()
}

fn list(node: &Node, size: PixelSize) -> DisplayList {
    node.compile(
        Size::new(f64::from(size.width), f64::from(size.height)),
        size,
        ValidationLimits::default(),
    )
    .unwrap()
    .into_editable()
}

fn prepared(node: &Node, size: PixelSize) -> PreparedCpuFrame {
    list(node, size)
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap()
}

fn push() -> DrawCommand {
    DrawCommand::PushLayer {
        opacity: 0.0,
        bounds: Rect::new(0.0, 0.0, 2.0, 1.0),
        blend_mode: BlendMode::SourceOver,
        clip: None,
    }
}

fn begin() -> DrawCommand {
    DrawCommand::BeginMask {
        mode: MaskMode::Alpha,
    }
}

fn fill() -> DrawCommand {
    DrawCommand::FillRect {
        bounds: Rect::new(0.0, 0.0, 2.0, 1.0),
        transform: Transform::IDENTITY,
        paint: Color::WHITE.into(),
    }
}

#[test]
fn alpha_and_linear_luminance_do_not_multiply_alpha_twice() {
    let red = [0.5, 0.0, 0.0, 0.5];
    assert_eq!(coverage(red, MaskMode::Alpha), 0.5);
    assert_eq!(coverage(red, MaskMode::Luminance), 0.1063);
    assert_eq!(coverage([0.0, 0.5, 0.0, 0.5], MaskMode::Luminance), 0.3576);
    assert_eq!(coverage([0.0, 0.0, 0.5, 0.5], MaskMode::Luminance), 0.0361);
    assert_eq!(coverage([2.0, 2.0, 2.0, 0.5], MaskMode::Luminance), 1.0);
    assert_eq!(coverage([-1.0, -1.0, -1.0, 0.5], MaskMode::Luminance), 0.0);
    for mode in [MaskMode::Alpha, MaskMode::Luminance] {
        assert_eq!(coverage([f32::MAX, 1.0, 1.0, 0.0], mode), 0.0);
    }
}

#[test]
fn node_content_mask_scales_overlapping_children_once_and_preserves_mode() {
    let tile = |color| Node::from(Rect::new(0.0, 0.0, 2.0, 1.0).fill(color));
    for (mode, expected) in [
        (MaskMode::Alpha, 0.5),
        (MaskMode::Luminance, 0.1063),
        (MaskMode::InvertedAlpha, 0.5),
        (MaskMode::InvertedLuminance, (1.0_f64 - 0.1063) as f32),
    ] {
        let mask = tile(Color::from_hex(0xff0000).with_alpha(0.5));
        let node = Node::group([tile(Color::WHITE), tile(Color::WHITE)]).mask(mask, mode);
        let list = node
            .compile(
                Size::new(2.0, 1.0),
                PixelSize::new(2, 1),
                ValidationLimits::default(),
            )
            .unwrap();
        assert!(list.commands.iter().any(
            |command| matches!(command, DrawCommand::BeginMask { mode: found } if *found == mode)
        ));
        let prepared = list
            .prepare(ValidationLimits::default(), RasterLimits::default())
            .unwrap();
        assert_eq!(prepared.items.len(), list.commands.len());
        assert!(
            prepared.items.iter().any(
                |item| matches!(item, PreparedItem::BeginMask { mode: found } if *found == mode)
            )
        );
        assert_eq!(
            prepared
                .compose_linear(RasterLimits::default())
                .unwrap()
                .pixels(),
            &[[expected; 4]; 2]
        );
        assert!(node.measure(ValidationLimits::default()).is_ok());
    }
}

#[test]
fn inverted_coverage_complements_clamped_linear_coverage_not_mask_color() {
    for pixel in [
        [0.0; 4],
        [f32::MAX, -f32::MAX, f32::MAX, 0.0],
        [0.5, 0.0, 0.0, 0.5],
        [0.0, 0.5, 0.0, 0.5],
        [0.0, 0.0, 0.5, 0.5],
        [2.0, 2.0, 2.0, 0.5],
        [-1.0, -1.0, -1.0, 0.5],
        [f32::from_bits(1); 4],
        [f32::MIN_POSITIVE; 4],
        [1.0_f32.next_down(); 4],
        [1.0; 4],
    ] {
        for (ordinary, inverted) in [
            (MaskMode::Alpha, MaskMode::InvertedAlpha),
            (MaskMode::Luminance, MaskMode::InvertedLuminance),
        ] {
            assert_eq!(coverage(pixel, inverted), 1.0 - coverage(pixel, ordinary));
        }
    }
    assert_eq!(coverage([f32::MAX; 4], MaskMode::InvertedLuminance), 0.0);
    assert_eq!(
        coverage(
            [-f32::MAX, -f32::MAX, -f32::MAX, 1.0],
            MaskMode::InvertedLuminance
        ),
        1.0
    );
    assert_eq!(coverage([0.0; 4], MaskMode::InvertedAlpha), 1.0);
    assert_eq!(
        coverage(
            [f32::MAX, f32::MAX, f32::MAX, 0.0],
            MaskMode::InvertedLuminance
        ),
        1.0
    );
    assert_eq!(
        coverage([0.5, 0.0, 0.0, 0.5], MaskMode::InvertedLuminance),
        1.0 - 0.1063
    );
}

#[test]
fn inverted_masks_preserve_only_source_in_empty_and_unpainted_regions() {
    let size = PixelSize::new(5, 1);
    for mode in [MaskMode::InvertedAlpha, MaskMode::InvertedLuminance] {
        let source = tile(2.0, 1.0, Color::WHITE).translate(1.0, 0.0);
        let original = source.measure(ValidationLimits::default()).unwrap();
        for artwork in [
            Node::empty(),
            tile(100.0, 100.0, Color::WHITE).translate(100.0, 0.0),
            tile(2.0, 1.0, Color::WHITE).opacity(0.0),
            tile(
                2.0,
                1.0,
                Color::from_rgb_float(1e16, -1e16, 1e16).with_alpha(0.0),
            ),
        ] {
            let node = source.clone().mask(artwork, mode);
            assert_eq!(node.measure(ValidationLimits::default()).unwrap(), original);
            let mut commands = list(&node, size);
            commands.background = Some(Color::from_hex(0x0000ff));
            let frame = commands
                .prepare(ValidationLimits::default(), RasterLimits::default())
                .unwrap();
            assert_eq!(
                frame
                    .compose_linear(RasterLimits::default())
                    .unwrap()
                    .pixels(),
                &[
                    [0.0, 0.0, 1.0, 1.0],
                    [1.0; 4],
                    [1.0; 4],
                    [0.0, 0.0, 1.0, 1.0],
                    [0.0, 0.0, 1.0, 1.0]
                ]
            );
            let [
                PreparedItem::PushLayer {
                    layer: crate::prepared::PreparedLayer { bounds, .. },
                },
                ..,
            ] = frame.items[..]
            else {
                panic!("expected source capture")
            };
            assert_eq!(
                (bounds.left, bounds.top, bounds.right, bounds.bottom),
                (1, 0, 3, 1)
            );
        }
        let partial = source.clone().mask(tile(1.0, 1.0, Color::WHITE), mode);
        assert_eq!(
            prepared(&partial, size)
                .compose_linear(RasterLimits::default())
                .unwrap()
                .pixels(),
            &[[0.0; 4], [0.0; 4], [1.0; 4], [0.0; 4], [0.0; 4]]
        );
        let edge = source.mask(tile(1.0, 1.0, Color::WHITE).translate(0.5, 0.0), mode);
        let coverage = (1.0_f64 - f64::from(128.0_f32 / 255.0)) as f32;
        assert_eq!(
            prepared(&edge, size)
                .compose_linear(RasterLimits::default())
                .unwrap()
                .pixels(),
            &[[0.0; 4], [coverage; 4], [coverage; 4], [0.0; 4], [0.0; 4]]
        );
    }
    let source = tile(
        2.0,
        1.0,
        Color::from_rgb_float(2.0, -0.5, 0.25).with_alpha(0.75),
    );
    let reference = prepared(&source, PixelSize::new(2, 1))
        .compose_linear(RasterLimits::default())
        .unwrap();
    for mode in [MaskMode::InvertedAlpha, MaskMode::InvertedLuminance] {
        let node = source.clone().mask(Node::empty(), mode);
        assert_eq!(
            prepared(&node, PixelSize::new(2, 1))
                .compose_linear(RasterLimits::default())
                .unwrap()
                .pixels(),
            reference.pixels()
        );
    }
}

#[test]
fn inverted_masks_resolve_artwork_before_source_clip_opacity_and_blending() {
    let size = PixelSize::new(2, 1);
    let half_red = || tile(2.0, 1.0, Color::from_hex(0xff0000).with_alpha(0.5));
    for (mode, expected) in [
        (MaskMode::InvertedAlpha, 0.25),
        (
            MaskMode::InvertedLuminance,
            (1.0_f64 - 0.2126 * 0.75) as f32,
        ),
    ] {
        let artwork = Node::group([half_red(), half_red()]);
        let source = Node::group([tile(2.0, 1.0, Color::WHITE), tile(2.0, 1.0, Color::WHITE)]);
        let node = source.mask(artwork, mode);
        assert_eq!(
            prepared(&node, size)
                .compose_linear(RasterLimits::default())
                .unwrap()
                .pixels(),
            &[[expected; 4]; 2]
        );
    }
    let artwork = tile(2.0, 1.0, Color::WHITE)
        .clip(Path2D::from_rect(Rect::new(0.0, 0.0, 1.0, 1.0)).unwrap())
        .opacity(0.5)
        .blend_mode(BlendMode::Multiply);
    for mode in [MaskMode::InvertedAlpha, MaskMode::InvertedLuminance] {
        let source = Node::group([tile(2.0, 1.0, Color::BLACK), tile(2.0, 1.0, Color::BLACK)])
            .mask(artwork.clone(), mode)
            .clip(Path2D::from_rect(Rect::new(0.5, 0.0, 2.0, 1.0)).unwrap())
            .opacity(0.5)
            .blend_mode(BlendMode::Multiply);
        let scene = Node::group([tile(2.0, 1.0, Color::WHITE), source]);
        let alpha = (0.25_f64 * (128.0 / 255.0)) as f32;
        let value = (1.0 - f64::from(alpha)) as f32;
        assert_eq!(
            prepared(&scene, size)
                .compose_linear(RasterLimits::default())
                .unwrap()
                .pixels(),
            &[[value, value, value, 1.0], [0.5, 0.5, 0.5, 1.0]]
        );
        let mask = tile(2.0, 1.0, Color::WHITE).mask(Node::empty(), mode);
        let nested = tile(2.0, 1.0, Color::WHITE).mask(mask, mode);
        assert_eq!(
            prepared(&nested, size)
                .compose_linear(RasterLimits::default())
                .unwrap()
                .pixels(),
            &[[0.0; 4]; 2]
        );
    }
    let gray = tile(
        2.0,
        1.0,
        Color::from_rgb_float(0.5, 0.5, 0.5).with_alpha(0.5),
    );
    let node = tile(2.0, 1.0, Color::WHITE).mask(gray, MaskMode::InvertedLuminance);
    for pixel in prepared(&node, size)
        .compose_linear(RasterLimits::default())
        .unwrap()
        .pixels()
    {
        for value in pixel {
            assert!((*value - 0.89297943).abs() <= f32::EPSILON);
        }
    }
}

#[test]
fn inverted_modes_cannot_bypass_hidden_validation_or_source_limits() {
    let size = PixelSize::new(2, 1);
    for mode in [MaskMode::InvertedAlpha, MaskMode::InvertedLuminance] {
        for source in [Node::empty(), tile(2.0, 1.0, Color::WHITE).opacity(0.0)] {
            let mask = tile(2.0, 1.0, Color::WHITE).translate(f64::NAN, 1e20);
            let error = source
                .clone()
                .mask(mask, mode)
                .compile(Size::new(2.0, 1.0), size, ValidationLimits::default())
                .unwrap_err();
            assert_eq!(error.kind, ValidationErrorKind::InvalidValue);
            assert!(error.path.contains(".mask"));
            let error = source
                .mask(tile(2.0, 1.0, Color::WHITE).opacity(f64::NAN), mode)
                .compile(Size::new(2.0, 1.0), size, ValidationLimits::default())
                .unwrap_err();
            assert_eq!(error.kind, ValidationErrorKind::InvalidValue);
            assert!(error.path.contains(".mask"));
        }
        let node = tile(2.0, 1.0, Color::WHITE).mask(tile(2.0, 1.0, Color::WHITE), mode);
        let exact = ValidationLimits {
            max_nodes: 6,
            max_depth: 3,
        };
        let commands = node.compile(Size::new(2.0, 1.0), size, exact).unwrap();
        assert_eq!(commands.preflight(exact).unwrap().nodes, 6);
        for limits in [
            ValidationLimits {
                max_nodes: 5,
                ..exact
            },
            ValidationLimits {
                max_depth: 2,
                ..exact
            },
        ] {
            assert_eq!(
                commands.preflight(limits).unwrap_err().kind,
                ValidationErrorKind::StructuralLimit
            );
            assert_eq!(
                node.compile(Size::new(2.0, 1.0), size, limits)
                    .unwrap_err()
                    .kind,
                ValidationErrorKind::StructuralLimit
            );
        }
        let hidden = node
            .opacity(0.0)
            .compile(Size::new(2.0, 1.0), size, ValidationLimits::default())
            .unwrap();
        assert!(
            hidden
                .commands
                .iter()
                .any(|command| matches!(command, motion_core::DrawCommand::BeginMask { .. }))
        );
    }
}

#[test]
fn masks_follow_source_coordinates_without_changing_layout_or_snapshots() {
    let original = tile(4.0, 2.0, Color::WHITE).translate(1.0, 1.0);
    let mask = tile(1.0, 2.0, Color::WHITE).translate(1.0, 0.0);
    let node = original.clone().mask(mask, MaskMode::Alpha);
    assert_eq!(
        original.measure(ValidationLimits::default()).unwrap(),
        node.measure(ValidationLimits::default()).unwrap()
    );
    let commands = node
        .compile(
            Size::new(6.0, 4.0),
            PixelSize::new(12, 12),
            ValidationLimits::default(),
        )
        .unwrap();
    let output = commands
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap()
        .compose_linear(RasterLimits::default())
        .unwrap();
    for (x, y) in [(4, 4), (5, 7)] {
        assert_eq!(output.pixel(x, y), Some([1.0; 4]));
    }
    for (x, y) in [(3, 4), (6, 4), (4, 3), (4, 8)] {
        assert_eq!(output.pixel(x, y), Some([0.0; 4]));
    }
    assert_eq!(list(&original, PixelSize::new(6, 4)).commands.len(), 1);
    assert!(original.measure(ValidationLimits::default()).is_ok());
    let outside = original.clone().mask(
        tile(100.0, 100.0, Color::WHITE).translate(20.0, 20.0),
        MaskMode::Alpha,
    );
    assert_eq!(
        original.measure(ValidationLimits::default()).unwrap(),
        outside.measure(ValidationLimits::default()).unwrap()
    );
    let replaced = node.mask(tile(4.0, 2.0, Color::BLACK), MaskMode::Alpha);
    assert_eq!(
        list(&replaced, PixelSize::new(6, 4))
            .commands
            .iter()
            .filter(|item| matches!(item, DrawCommand::BeginMask { .. }))
            .count(),
        1
    );
    assert_eq!(MaskMode::default(), MaskMode::Alpha);
}

#[test]
fn nested_mask_artwork_clip_opacity_and_backdrop_order_are_isolated() {
    let half_white = || tile(2.0, 1.0, Color::WHITE.with_alpha(0.5));
    let nested = tile(2.0, 1.0, Color::WHITE).mask(
        half_white().mask(half_white(), MaskMode::Alpha),
        MaskMode::Alpha,
    );
    assert_eq!(
        prepared(&nested, PixelSize::new(2, 1))
            .compose_linear(RasterLimits::default())
            .unwrap()
            .pixels(),
        &[[0.25; 4]; 2]
    );
    let artwork = Node::group([half_white(), half_white()]);
    let grouped = tile(2.0, 1.0, Color::WHITE).mask(artwork, MaskMode::Alpha);
    assert_eq!(
        prepared(&grouped, PixelSize::new(2, 1))
            .compose_linear(RasterLimits::default())
            .unwrap()
            .pixels(),
        &[[0.75; 4]; 2]
    );
    let artwork = tile(2.0, 1.0, Color::WHITE)
        .clip(Path2D::from_rect(Rect::new(0.0, 0.0, 1.0, 1.0)).unwrap())
        .opacity(0.5);
    let source = tile(2.0, 1.0, Color::WHITE).mask(artwork, MaskMode::Luminance);
    let parent = Node::group([source]).translate(1.0, 0.0);
    assert_eq!(
        prepared(&parent, PixelSize::new(3, 1))
            .compose_linear(RasterLimits::default())
            .unwrap()
            .pixels(),
        &[[0.0; 4], [0.5; 4], [0.0; 4]]
    );
    let clip = Path2D::from_rect(Rect::new(0.5, 0.0, 2.0, 1.0)).unwrap();
    let source = tile(2.0, 1.0, Color::BLACK)
        .mask(half_white(), MaskMode::Alpha)
        .clip(clip)
        .opacity(0.5)
        .blend_mode(BlendMode::Multiply);
    let scene = Node::group([tile(2.0, 1.0, Color::WHITE), source]);
    let output = prepared(&scene, PixelSize::new(2, 1))
        .compose_linear(RasterLimits::default())
        .unwrap();
    let alpha = (0.25_f64 * (128.0 / 255.0)) as f32;
    let value = (1.0 - f64::from(alpha)) as f32;
    assert_eq!(output.pixel(0, 0), Some([value, value, value, 1.0]));
    assert_eq!(output.pixel(1, 0), Some([0.75, 0.75, 0.75, 1.0]));
    for (mode, mask, visible) in [
        (MaskMode::Alpha, Node::empty(), false),
        (MaskMode::Alpha, tile(2.0, 1.0, Color::BLACK), true),
        (MaskMode::Luminance, tile(2.0, 1.0, Color::BLACK), false),
        (
            MaskMode::Alpha,
            tile(2.0, 1.0, Color::WHITE).blend_mode(BlendMode::Multiply),
            true,
        ),
    ] {
        let mut source = list(
            &tile(2.0, 1.0, Color::WHITE).mask(mask, mode),
            PixelSize::new(2, 1),
        );
        source.background = Some(Color::BLACK);
        let output = source
            .prepare(ValidationLimits::default(), RasterLimits::default())
            .unwrap()
            .compose_linear(RasterLimits::default())
            .unwrap();
        assert_eq!(
            output.pixel(0, 0),
            Some(if visible {
                [1.0; 4]
            } else {
                [0.0, 0.0, 0.0, 1.0]
            })
        );
    }
}

#[test]
fn finite_mask_boundaries_and_mask_render_overflow_preserve_sources() {
    for alpha in [
        0.0,
        f32::from_bits(1),
        f32::MIN_POSITIVE,
        1.0 / 255.0,
        0.5,
        1.0_f32.next_down(),
        1.0,
    ] {
        let pixel = [alpha; 4];
        assert_eq!(coverage(pixel, MaskMode::Alpha), f64::from(alpha));
        if alpha > 0.0 {
            assert!(coverage(pixel, MaskMode::Luminance) > 0.0);
            assert!((coverage(pixel, MaskMode::Luminance) / f64::from(alpha) - 1.0).abs() <= 2e-15);
        }
        let mask = tile(1.0, 1.0, Color::WHITE.with_alpha(f64::from(alpha)));
        let node = tile(1.0, 1.0, Color::WHITE).mask(mask, MaskMode::Alpha);
        assert_eq!(
            prepared(&node, PixelSize::new(1, 1))
                .compose_linear(RasterLimits::default())
                .unwrap()
                .pixel(0, 0),
            Some([alpha; 4])
        );
    }
    for (color, expected) in [
        (Color::from_rgb_float(2.0, 2.0, 2.0), 1.0),
        (Color::from_rgb_float(-1.0, -1.0, -1.0), 0.0),
    ] {
        let node = tile(1.0, 1.0, Color::WHITE).mask(tile(1.0, 1.0, color), MaskMode::Luminance);
        assert_eq!(
            prepared(&node, PixelSize::new(1, 1))
                .compose_linear(RasterLimits::default())
                .unwrap()
                .pixel(0, 0),
            Some([expected; 4])
        );
    }
    let gray = tile(
        1.0,
        1.0,
        Color::from_rgb_float(0.5, 0.5, 0.5).with_alpha(0.5),
    );
    let node = tile(1.0, 1.0, Color::WHITE).mask(gray, MaskMode::Luminance);
    let output = prepared(&node, PixelSize::new(1, 1))
        .compose_linear(RasterLimits::default())
        .unwrap();
    for value in output.pixel(0, 0).unwrap() {
        assert!((value - 0.10702057).abs() < 1e-8);
    }
    let extended = tile(1.0, 1.0, Color::from_rgb_float(2.0, 0.0, 0.0)).mask(
        tile(1.0, 1.0, Color::WHITE.with_alpha(0.5)),
        MaskMode::Alpha,
    );
    let output = prepared(&extended, PixelSize::new(1, 1))
        .compose_linear(RasterLimits::default())
        .unwrap();
    let pixel = output.pixel(0, 0).unwrap();
    assert!(pixel[0] > 1.0);
    assert_eq!([pixel[1], pixel[2], pixel[3]], [0.0, 0.0, 0.5]);
    let hdr = tile(1.0, 1.0, Color::from_rgb_float(1e16, 0.0, 0.0));
    let artwork = Node::group([hdr.clone(), hdr.clone().blend_mode(BlendMode::Plus)]);
    let frame = prepared(
        &tile(1.0, 1.0, Color::WHITE).mask(artwork, MaskMode::Alpha),
        PixelSize::new(1, 1),
    );
    let original = frame.clone();
    let error = frame.compose_linear(RasterLimits::default()).unwrap_err();
    assert_eq!(
        (error.kind, error.field.as_str()),
        (ValidationErrorKind::InvalidValue, "color")
    );
    assert_eq!(frame, original);
    assert!(
        prepared(&hdr, PixelSize::new(1, 1))
            .compose_linear(RasterLimits::default())
            .is_ok()
    );
}

#[test]
fn malformed_mask_phases_and_absolute_stack_limit_fail_before_preparation() {
    let cases = [
        vec![begin()],
        vec![DrawCommand::EndMask],
        vec![push(), DrawCommand::EndMask, DrawCommand::PopLayer],
        vec![push(), begin(), DrawCommand::PopLayer, DrawCommand::EndMask],
        vec![push(), begin(), push(), DrawCommand::EndMask],
        vec![
            push(),
            begin(),
            DrawCommand::EndMask,
            fill(),
            DrawCommand::PopLayer,
        ],
        vec![
            push(),
            begin(),
            DrawCommand::EndMask,
            begin(),
            DrawCommand::EndMask,
            DrawCommand::PopLayer,
        ],
        vec![push(), begin(), begin()],
        vec![push(), begin()],
    ];
    for commands in cases {
        let source = DisplayList {
            output_size: PixelSize::new(2, 1),
            background: None,
            commands,
        };
        let error = source.preflight(ValidationLimits::default()).unwrap_err();
        assert_eq!(error.kind, ValidationErrorKind::InvalidValue);
        assert_eq!(
            source
                .prepare(ValidationLimits::default(), RasterLimits::default())
                .unwrap_err(),
            error
        );
    }
    let limits = ValidationLimits {
        max_nodes: 1000,
        max_depth: 128,
    };
    for (depth, accepted) in [(128, true), (129, false)] {
        let mut commands = vec![push(); depth];
        commands.extend(std::iter::repeat_n(DrawCommand::PopLayer, depth));
        let source = DisplayList {
            output_size: PixelSize::new(2, 1),
            background: None,
            commands,
        };
        if accepted {
            assert_eq!(source.preflight(limits).unwrap().max_depth, 128);
        } else {
            assert_eq!(source.preflight(limits).unwrap_err().field, "max_depth");
        }
    }
    for extra in [false, true] {
        let mut commands = Vec::new();
        for _ in 0..64 {
            commands.extend([push(), begin()]);
        }
        if extra {
            commands.push(fill());
        }
        for _ in 0..64 {
            commands.extend([DrawCommand::EndMask, DrawCommand::PopLayer]);
        }
        let source = DisplayList {
            output_size: PixelSize::new(2, 1),
            background: None,
            commands,
        };
        if extra {
            assert_eq!(source.preflight(limits).unwrap_err().field, "max_depth");
        } else {
            assert_eq!(source.preflight(limits).unwrap().max_depth, 128);
        }
    }
}

#[test]
fn mask_branches_share_structural_limits_and_validate_even_when_hidden() {
    let size = PixelSize::new(2, 1);
    let node = tile(2.0, 1.0, Color::WHITE).mask(tile(2.0, 1.0, Color::WHITE), MaskMode::Alpha);
    let exact = ValidationLimits {
        max_nodes: 6,
        max_depth: 3,
    };
    let commands = node.compile(Size::new(2.0, 1.0), size, exact).unwrap();
    assert_eq!(commands.preflight(exact).unwrap().nodes, 6);
    for limits in [
        ValidationLimits {
            max_nodes: 5,
            ..exact
        },
        ValidationLimits {
            max_depth: 2,
            ..exact
        },
    ] {
        assert_eq!(
            commands.preflight(limits).unwrap_err().kind,
            ValidationErrorKind::StructuralLimit
        );
        assert_eq!(
            node.compile(Size::new(2.0, 1.0), size, limits)
                .unwrap_err()
                .kind,
            ValidationErrorKind::StructuralLimit
        );
    }
    for source in [Node::empty(), tile(2.0, 1.0, Color::WHITE).opacity(0.0)] {
        let mask = tile(2.0, 1.0, Color::WHITE).translate(f64::NAN, 1000.0);
        let invalid = source.clone().mask(mask, MaskMode::Alpha);
        let error = invalid
            .compile(Size::new(2.0, 1.0), size, ValidationLimits::default())
            .unwrap_err();
        assert_eq!(error.kind, ValidationErrorKind::InvalidValue);
        assert!(error.path.contains(".mask"));
        let nan_opacity = source.mask(
            tile(2.0, 1.0, Color::WHITE).opacity(f64::NAN),
            MaskMode::Alpha,
        );
        let error = nan_opacity
            .compile(Size::new(2.0, 1.0), size, ValidationLimits::default())
            .unwrap_err();
        assert_eq!(error.kind, ValidationErrorKind::InvalidValue);
        assert!(error.path.contains(".mask"));
    }
    let hidden = node
        .opacity(0.0)
        .compile(Size::new(2.0, 1.0), size, ValidationLimits::default())
        .unwrap();
    assert!(
        hidden
            .commands
            .iter()
            .any(|command| matches!(command, motion_core::DrawCommand::BeginMask { .. }))
    );
}

#[test]
fn preparation_render_and_sibling_mask_lifetimes_obey_shared_budgets() {
    let size = PixelSize::new(4, 2);
    for mode in [
        MaskMode::Alpha,
        MaskMode::Luminance,
        MaskMode::InvertedAlpha,
        MaskMode::InvertedLuminance,
    ] {
        let node =
            tile(4.0, 2.0, Color::WHITE).mask(tile(4.0, 2.0, Color::WHITE.with_alpha(0.5)), mode);
        let source = list(&node, size);
        let frame = source
            .prepare(ValidationLimits::default(), RasterLimits::default())
            .unwrap();
        let exact = RasterLimits {
            max_pixels: 8,
            max_bytes: frame.stats().peak_bytes,
            max_work: frame.stats().work,
        };
        assert_eq!(
            source.prepare(ValidationLimits::default(), exact).unwrap(),
            frame
        );
        for (limits, field) in [
            (
                RasterLimits {
                    max_pixels: 7,
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
            let error = source
                .prepare(ValidationLimits::default(), limits)
                .unwrap_err();
            assert_eq!(
                (error.kind, error.field.as_str()),
                (ValidationErrorKind::StructuralLimit, field)
            );
            assert_eq!(
                source.prepare(ValidationLimits::default(), exact).unwrap(),
                frame
            );
        }
        let original = frame.clone();
        let output = frame.compose_linear(RasterLimits::default()).unwrap();
        let exact = RasterLimits {
            max_pixels: 8,
            max_bytes: output.stats().peak_bytes,
            max_work: output.stats().work,
        };
        assert_eq!(frame.compose_linear(exact).unwrap(), output);
        for (limits, field) in [
            (
                RasterLimits {
                    max_pixels: 7,
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
            assert_eq!(frame.compose_linear(exact).unwrap(), output);
        }
        let siblings = prepared(&Node::group([node.clone(), node.clone()]), size);
        let double = siblings.compose_linear(RasterLimits::default()).unwrap();
        assert_eq!(
            output.stats().peak_bytes - frame.stats().retained_bytes,
            double.stats().peak_bytes - siblings.stats().retained_bytes
        );
    }
}

#[test]
fn mask_draw_callbacks_run_once_per_occurrence_and_not_during_measurement() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct Counted(Arc<AtomicUsize>);

    impl Drawable for Counted {
        fn bounds(&self, _: &DrawContext) -> Result<Rect, ValidationError> {
            Ok(Rect::new(0.0, 0.0, 2.0, 1.0))
        }

        fn draw(&self, _: &DrawContext, canvas: &mut Canvas<'_>) -> Result<(), ValidationError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            canvas.fill_rect(Rect::new(0.0, 0.0, 2.0, 1.0), Color::WHITE)
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let mask = Node::from(Counted(calls.clone()));
    let node =
        tile(2.0, 1.0, Color::WHITE).mask(Node::group([mask.clone(), mask]), MaskMode::Alpha);
    node.measure(ValidationLimits::default()).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let frame = prepared(&node, PixelSize::new(2, 1));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    frame.compose_linear(RasterLimits::default()).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let empty = Node::empty().mask(Node::from(Counted(calls.clone())), MaskMode::Alpha);
    assert!(list(&empty, PixelSize::new(2, 1)).commands.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}
