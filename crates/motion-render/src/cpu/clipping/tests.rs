use super::*;
use motion_core::{
    BlendMode, Color, DisplayList, DrawCommand, FillRule, MaskMode, Node, NodeModifiers, Path2D,
    PathClip, Pivot, Point, Rect, RectClip, Row, Shape, ShapeClip, Size, Transform,
    ValidationErrorKind, ValidationLimits,
};

fn list(node: &Node, size: PixelSize) -> DisplayList {
    node.compile(
        Size::new(f64::from(size.width), f64::from(size.height)),
        size,
        ValidationLimits::default(),
    )
    .unwrap()
    .into_editable()
}

fn prepare(node: &Node, size: PixelSize) -> PreparedFrame {
    list(node, size)
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap()
}

fn tile(width: f64, height: f64, color: Color) -> Node {
    Rect::new(0.0, 0.0, width, height).fill(color).into()
}

#[test]
fn unified_clip_preserves_coverage_multiplication_and_geometric_intersection() {
    let bounds = Rect::new(0.5, 0.0, 2.0, 1.0);
    let source = tile(2.0, 1.0, Color::WHITE);
    let coverage = prepare(
        &Node::group([source.clone().clip(bounds)]).clip(bounds),
        PixelSize::new(2, 1),
    )
    .render_linear(RasterLimits::default())
    .unwrap();
    let geometric = prepare(
        &Node::group([source.clip_with(bounds, motion_core::ClipMode::Geometric)])
            .clip_with(bounds, motion_core::ClipMode::Geometric),
        PixelSize::new(2, 1),
    )
    .render_linear(RasterLimits::default())
    .unwrap();
    let half = 128.0_f32 / 255.0;
    assert_eq!(coverage.pixel(0, 0).unwrap()[3], half * half);
    assert_eq!(geometric.pixel(0, 0).unwrap()[3], half);
}

fn rectangle(bounds: Rect) -> PathClip {
    PathClip::new(Path2D::from_rect(bounds).unwrap())
}

fn outline(points: &[(f64, f64)]) -> Path2D {
    let mut path = Path2D::new();
    for (index, (horizontal, vertical)) in points.iter().enumerate() {
        let point = Point::new(*horizontal, *vertical);
        if index == 0 {
            path.try_move_to(point).unwrap();
        } else {
            path.try_line_to(point).unwrap();
        }
    }
    path.try_close().unwrap();
    path
}

#[test]
fn generic_shapes_intersect_before_fractional_coverage_is_quantized() {
    struct Triangle;

    impl motion_core::Shape for Triangle {
        fn path(&self) -> Result<Path2D, motion_core::ValidationError> {
            let mut path = Path2D::new();
            path.try_move_to(Point::ORIGIN)?;
            path.try_line_to(Point::new(1.0, 0.0))?;
            path.try_line_to(Point::new(0.0, 1.0))?;
            path.try_close()?;
            Ok(path)
        }
    }
    let inner = tile(1.0, 1.0, Color::WHITE).clip_with(Triangle, motion_core::ClipMode::Geometric);
    let node = Node::group([inner]).clip_with(Triangle, motion_core::ClipMode::Geometric);
    let output = prepare(&node, PixelSize::new(1, 1))
        .render_linear(RasterLimits::default())
        .unwrap();
    assert_eq!(output.pixel(0, 0), Some([128.0 / 255.0; 4]));
}

#[test]
fn generic_shapes_and_rectangles_share_one_geometric_domain_in_either_order() {
    let lower = outline(&[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)]);
    let upper = outline(&[(1.0, 1.0), (0.0, 1.0), (1.0, 0.0)]);
    for (inner, outer) in [(lower.clone(), upper.clone()), (upper, lower.clone())] {
        let node = Node::group([
            tile(1.0, 1.0, Color::WHITE).clip_with(inner, motion_core::ClipMode::Geometric)
        ])
        .clip_with(outer, motion_core::ClipMode::Geometric);
        assert_eq!(
            prepare(&node, PixelSize::new(1, 1))
                .render_linear(RasterLimits::default())
                .unwrap()
                .pixel(0, 0),
            Some([0.0; 4])
        );
    }
    let rect = Rect::new(0.5, 0.0, 1.0, 1.0);
    let nodes = [
        Node::group([
            tile(1.0, 1.0, Color::WHITE).clip_with(lower.clone(), motion_core::ClipMode::Geometric)
        ])
        .clip_with(rect, motion_core::ClipMode::Geometric),
        Node::group([
            tile(1.0, 1.0, Color::WHITE).clip_with(rect, motion_core::ClipMode::Geometric)
        ])
        .clip_with(lower.clone(), motion_core::ClipMode::Geometric),
        Node::group([
            tile(1.0, 1.0, Color::WHITE).clip_with(lower, motion_core::ClipMode::Geometric)
        ])
        .clip_with(rect, motion_core::ClipMode::Geometric),
    ];
    for node in nodes {
        let commands = list(&node, PixelSize::new(1, 1));
        let frame = commands
            .prepare(ValidationLimits::default(), RasterLimits::default())
            .unwrap();
        assert_eq!(frame.items().len(), commands.commands.len());
        let pixel = frame
            .render_linear(RasterLimits::default())
            .unwrap()
            .pixel(0, 0)
            .unwrap();
        for channel in pixel {
            assert!((channel - 32.0 / 255.0).abs() <= f32::EPSILON);
        }
    }
}

#[test]
fn generic_shape_intersections_respect_holes_concavity_and_self_crossings() {
    let mut donut = Path2D::new();
    for (left, right) in [(0.0, 3.0), (1.0, 2.0)] {
        donut.try_move_to(Point::new(left, 0.0)).unwrap();
        donut.try_line_to(Point::new(right, 0.0)).unwrap();
        donut.try_line_to(Point::new(right, 1.0)).unwrap();
        donut.try_line_to(Point::new(left, 1.0)).unwrap();
        donut.try_close().unwrap();
    }
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let mut clip = ShapeClip::new(&donut).unwrap();
        clip.rule = rule;
        let inner = tile(3.0, 1.0, Color::WHITE).clip_with(clip, motion_core::ClipMode::Geometric);
        let node = Node::group([inner]).clip_with(donut.clone(), motion_core::ClipMode::Geometric);
        let output = prepare(&node, PixelSize::new(3, 1))
            .render_linear(RasterLimits::default())
            .unwrap();
        assert_eq!(output.pixel(0, 0), Some([1.0; 4]));
        assert_eq!(output.pixel(2, 0), Some([1.0; 4]));
        assert_eq!(
            output.pixel(1, 0),
            Some([if rule == FillRule::EvenOdd { 0.0 } else { 1.0 }; 4])
        );
    }
    let mut hole = outline(&[(0.0, 0.0), (3.0, 0.0), (3.0, 3.0), (0.0, 3.0)]);
    hole.try_move_to(Point::new(1.0, 1.0)).unwrap();
    hole.try_line_to(Point::new(1.0, 2.0)).unwrap();
    hole.try_line_to(Point::new(2.0, 2.0)).unwrap();
    hole.try_line_to(Point::new(2.0, 1.0)).unwrap();
    hole.try_close().unwrap();
    let node = Node::group([
        tile(3.0, 3.0, Color::WHITE).clip_with(hole.clone(), motion_core::ClipMode::Geometric)
    ])
    .clip_with(hole, motion_core::ClipMode::Geometric);
    let output = prepare(&node, PixelSize::new(3, 3))
        .render_linear(RasterLimits::default())
        .unwrap();
    for (index, pixel) in output.pixels().iter().enumerate() {
        assert_eq!(*pixel, [if index == 4 { 0.0 } else { 1.0 }; 4]);
    }
    let concave = outline(&[
        (0.0, 0.0),
        (2.0, 0.0),
        (2.0, 1.0),
        (1.0, 1.0),
        (1.0, 2.0),
        (0.0, 2.0),
    ]);
    let mut islands = Path2D::new();
    for (left, right) in [(0.0, 0.5), (1.5, 2.0)] {
        islands.try_move_to(Point::new(left, 0.0)).unwrap();
        islands.try_line_to(Point::new(right, 0.0)).unwrap();
        islands.try_line_to(Point::new(right, 2.0)).unwrap();
        islands.try_line_to(Point::new(left, 2.0)).unwrap();
        islands.try_close().unwrap();
    }
    let node = Node::group([
        tile(2.0, 2.0, Color::WHITE).clip_with(concave, motion_core::ClipMode::Geometric)
    ])
    .clip_with(islands, motion_core::ClipMode::Geometric);
    assert_eq!(
        prepare(&node, PixelSize::new(2, 2))
            .render_linear(RasterLimits::default())
            .unwrap()
            .pixels(),
        &[
            [128.0 / 255.0; 4],
            [128.0 / 255.0; 4],
            [128.0 / 255.0; 4],
            [0.0; 4]
        ]
    );
    let crossing = outline(&[(0.0, 0.0), (1.0, 1.0), (0.0, 1.0), (1.0, 0.0)]);
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let mut clip = ShapeClip::new(&crossing).unwrap();
        clip.rule = rule;
        let node =
            Node::group([tile(1.0, 1.0, Color::WHITE)
                .clip_with(clip.clone(), motion_core::ClipMode::Geometric)])
            .clip_with(clip, motion_core::ClipMode::Geometric);
        assert_eq!(
            prepare(&node, PixelSize::new(1, 1))
                .render_linear(RasterLimits::default())
                .unwrap()
                .pixel(0, 0),
            Some([128.0 / 255.0; 4])
        );
    }
}

#[test]
fn generic_curves_reuse_fill_flattening_tolerance_without_repeat_fading() {
    let mut quadratic = Path2D::new();
    quadratic.try_move_to(Point::ORIGIN).unwrap();
    quadratic
        .try_quadratic_to(Point::new(0.5, 2.0), Point::new(1.0, 0.0))
        .unwrap();
    let mut cubic = Path2D::new();
    cubic.try_move_to(Point::ORIGIN).unwrap();
    cubic
        .try_cubic_to(
            Point::new(0.0, 1.0),
            Point::new(1.0, 1.0),
            Point::new(1.0, 0.0),
        )
        .unwrap();
    for path in [quadratic, cubic] {
        for tolerance in [1.0 / 256.0, 1.0 / 1024.0] {
            let expected: Node = path.clone().fill(Color::WHITE).into();
            let repeated = Node::group([tile(1.0, 1.0, Color::WHITE)
                .clip_with(path.clone(), motion_core::ClipMode::Geometric)])
            .clip_with(path.clone(), motion_core::ClipMode::Geometric);
            let render = |node: &Node| {
                list(node, PixelSize::new(1, 1))
                    .prepare_with_tolerance(
                        ValidationLimits::default(),
                        RasterLimits::default(),
                        tolerance,
                    )
                    .unwrap()
                    .render_linear(RasterLimits::default())
                    .unwrap()
            };
            assert_eq!(render(&expected).pixels(), render(&repeated).pixels());
        }
    }
}

#[test]
fn generic_shapes_snapshot_once_without_measuring_clip_layout() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct Counted {
        calls: Arc<AtomicUsize>,
        path: Path2D,
    }

    impl motion_core::Shape for Counted {
        fn path(&self) -> Result<Path2D, motion_core::ValidationError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.path.clone())
        }

        fn bounding_box(&self) -> Result<Rect, motion_core::ValidationError> {
            panic!("clip layout must not be measured")
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let mut shape = Counted {
        calls: calls.clone(),
        path: outline(&[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)]),
    };
    let clip = ShapeClip::new(&shape as &dyn motion_core::Shape).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    shape.path.try_line_to(Point::new(1.0, 1.0)).unwrap();
    let original = tile(1.0, 1.0, Color::WHITE);
    let clipped = Node::group([original.clone(), original.clone()])
        .clip_with(clip, motion_core::ClipMode::Geometric);
    assert_eq!(
        clipped.measure(ValidationLimits::default()).unwrap(),
        original.measure(ValidationLimits::default()).unwrap()
    );
    for _ in 0..2 {
        assert_eq!(
            prepare(&clipped, PixelSize::new(1, 1))
                .render_linear(RasterLimits::default())
                .unwrap()
                .pixel(0, 0),
            Some([128.0 / 255.0; 4])
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let replaced = clipped.clone().clip_with(
        Rect::new(0.0, 0.0, 1.0, 1.0),
        motion_core::ClipMode::Geometric,
    );
    assert_eq!(
        prepare(&replaced, PixelSize::new(1, 1))
            .render_linear(RasterLimits::default())
            .unwrap()
            .pixel(0, 0),
        Some([1.0; 4])
    );
    assert_eq!(
        prepare(&clipped, PixelSize::new(1, 1))
            .render_linear(RasterLimits::default())
            .unwrap()
            .pixel(0, 0),
        Some([128.0 / 255.0; 4])
    );
}

#[test]
fn generic_shape_transforms_follow_contain_mapping_and_preserve_layout() {
    let path = Path2D::from_rect(Rect::new(0.0, 0.0, 1.0, 2.0)).unwrap();
    let mut clip = ShapeClip::new(&path).unwrap();
    clip.transform = Transform::translate(1.0, 0.0);
    let node = tile(4.0, 2.0, Color::WHITE)
        .clip_with(clip, motion_core::ClipMode::Geometric)
        .translate(1.0, 1.0);
    let commands = node
        .compile(
            Size::new(6.0, 4.0),
            PixelSize::new(12, 12),
            ValidationLimits::default(),
        )
        .unwrap();
    match &commands.commands[0] {
        DrawCommand::PushShapeLayer { shape_clip, .. } => assert_eq!(
            shape_clip.transform.apply(Point::ORIGIN),
            Point::new(4.0, 4.0)
        ),
        _ => panic!("expected transformed Shape clip"),
    }
    let output = commands
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap()
        .render_linear(RasterLimits::default())
        .unwrap();
    for point in [(4, 4), (5, 7)] {
        assert_eq!(output.pixel(point.0, point.1), Some([1.0; 4]));
    }
    for point in [(3, 4), (6, 4), (4, 3), (4, 8)] {
        assert_eq!(output.pixel(point.0, point.1), Some([0.0; 4]));
    }
    let triangle = outline(&[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)]);
    let mut clip = ShapeClip::new(&triangle).unwrap();
    clip.transform = Transform::from_coefficients([1.0, 1.0, -1.0, 1.0, 1.0, 0.0]);
    let output = prepare(
        &tile(2.0, 2.0, Color::WHITE).clip_with(clip, motion_core::ClipMode::Geometric),
        PixelSize::new(2, 2),
    )
    .render_linear(RasterLimits::default())
    .unwrap();
    assert_eq!(
        output.pixels(),
        &[[128.0 / 255.0; 4], [128.0 / 255.0; 4], [0.0; 4], [0.0; 4]]
    );
}

#[test]
fn generic_shapes_preserve_extended_colors_across_unclipped_isolation() {
    let triangle = outline(&[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)]);
    let color = Color::from_rgb_float(2.0, -0.125, 0.5);
    for opacity in [0.0, 0.5, 1.0e-50] {
        let leaf =
            tile(1.0, 1.0, color).clip_with(triangle.clone(), motion_core::ClipMode::Geometric);
        let node = Node::group([Node::group([leaf]).opacity(opacity)])
            .clip_with(triangle.clone(), motion_core::ClipMode::Geometric);
        let expected = color.to_linear_premultiplied().map(|channel| {
            let faded = (f64::from(channel as f32) * opacity) as f32;
            (f64::from(faded) * (128.0 / 255.0)) as f32
        });
        assert_eq!(
            prepare(&node, PixelSize::new(1, 1))
                .render_linear(RasterLimits::default())
                .unwrap()
                .pixel(0, 0),
            Some(expected)
        );
    }
}

#[test]
fn generic_shapes_keep_mask_domains_and_path_coverage_independent() {
    let rect = Rect::new(0.5, 0.0, 2.0, 1.0);
    let path = Path2D::from_rect(rect).unwrap();
    let size = PixelSize::new(2, 1);
    for mode in [
        MaskMode::Alpha,
        MaskMode::InvertedAlpha,
        MaskMode::Luminance,
        MaskMode::InvertedLuminance,
    ] {
        let mask = tile(2.0, 1.0, Color::from_hex(0xff0000))
            .clip_with(path.clone(), motion_core::ClipMode::Geometric)
            .opacity(0.5);
        let source = Node::group([tile(2.0, 1.0, Color::BLACK), tile(2.0, 1.0, Color::BLACK)])
            .clip_with(path.clone(), motion_core::ClipMode::Geometric)
            .clip(rectangle(rect))
            .mask(mask, mode)
            .opacity(0.5);
        let node = Node::group([Node::group([source]).opacity(0.75)])
            .clip_with(path.clone(), motion_core::ClipMode::Geometric)
            .blend_mode(BlendMode::Multiply);
        let mut commands = list(&node, size);
        commands.background = Some(Color::WHITE);
        let output = commands
            .prepare(ValidationLimits::default(), RasterLimits::default())
            .unwrap()
            .render_linear(RasterLimits::default())
            .unwrap();
        for column in 0..2 {
            let geometry = if column == 0 { 128.0 / 255.0 } else { 1.0 };
            let artwork = f64::from((0.5 * geometry) as f32);
            let coverage = match mode {
                MaskMode::Alpha => artwork,
                MaskMode::InvertedAlpha => 1.0 - artwork,
                MaskMode::Luminance => 0.2126 * artwork,
                MaskMode::InvertedLuminance => 1.0 - 0.2126 * artwork,
            };
            let masked = coverage as f32;
            let inner = (f64::from(masked) * 0.5 * geometry) as f32;
            let middle = (f64::from(inner) * 0.75) as f32;
            let alpha = (f64::from(middle) * geometry) as f32;
            let channel = (1.0 - f64::from(alpha)) as f32;
            assert_eq!(
                output.pixel(column, 0),
                Some([channel, channel, channel, 1.0]),
                "{mode:?}"
            );
        }
    }
}

#[test]
fn generic_shapes_validate_hidden_empty_edited_and_composed_geometry() {
    let size = PixelSize::new(1, 1);
    let invalid = ShapeClip::new(&Rect::new(f64::NAN, 0.0, 1.0, 1.0)).unwrap();
    let mut invalid_transform = ShapeClip::new(&Path2D::new()).unwrap();
    invalid_transform.transform = Transform::translate(f64::INFINITY, 0.0);
    for clip in [invalid, invalid_transform] {
        for node in [
            Node::empty().clip_with(clip.clone(), motion_core::ClipMode::Geometric),
            tile(1.0, 1.0, Color::WHITE)
                .clip_with(clip, motion_core::ClipMode::Geometric)
                .opacity(0.0),
        ] {
            let error = node
                .compile(Size::new(1.0, 1.0), size, ValidationLimits::default())
                .unwrap_err();
            assert_eq!(error.kind, ValidationErrorKind::InvalidValue);
            assert!(error.path.contains(".shape_clip") || error.path.contains(".rect_clip"));
            assert!(node.measure(ValidationLimits::default()).is_err());
        }
    }
    let mut clip = ShapeClip::new(&Path2D::new()).unwrap();
    clip.transform = Transform::scale(2.0);
    let node = Node::empty()
        .clip_with(clip, motion_core::ClipMode::Geometric)
        .pivot(Pivot::Point(Point::ORIGIN))
        .scale(f64::MAX);
    let error = node
        .compile(Size::new(1.0, 1.0), size, ValidationLimits::default())
        .unwrap_err();
    assert!(error.path.contains(".shape_clip"));
    let node =
        tile(1.0, 1.0, Color::WHITE).clip_with(Path2D::new(), motion_core::ClipMode::Geometric);
    assert_eq!(
        prepare(&node, size)
            .render_linear(RasterLimits::default())
            .unwrap()
            .pixel(0, 0),
        Some([0.0; 4])
    );
    assert!(
        node.clone()
            .opacity(0.0)
            .compile(Size::new(1.0, 1.0), size, ValidationLimits::default())
            .is_ok()
    );
    let invalid_source: Node = Rect::new(f64::NAN, 0.0, 1.0, 1.0).fill(Color::WHITE).into();
    assert_eq!(
        invalid_source
            .clip_with(Path2D::new(), motion_core::ClipMode::Geometric)
            .compile(Size::new(1.0, 1.0), size, ValidationLimits::default())
            .unwrap_err()
            .kind,
        ValidationErrorKind::InvalidValue
    );
    let mut edited = list(&node, size);
    if let DrawCommand::PushShapeLayer { shape_clip, .. } = &mut edited.commands[0] {
        shape_clip.transform = Transform::scale(f64::NAN);
    }
    assert_eq!(
        edited
            .prepare(
                ValidationLimits::default(),
                RasterLimits {
                    max_bytes: 1,
                    ..RasterLimits::default()
                }
            )
            .unwrap_err()
            .kind,
        ValidationErrorKind::InvalidValue
    );
    let mut curve = Path2D::new();
    curve.try_move_to(Point::ORIGIN).unwrap();
    curve
        .try_quadratic_to(Point::new(1.0e12, 1.0e12), Point::new(1.0, 0.0))
        .unwrap();
    let hidden = tile(1.0, 1.0, Color::WHITE)
        .clip_with(Rect::default(), motion_core::ClipMode::Geometric)
        .clip_with(curve, motion_core::ClipMode::Geometric)
        .opacity(0.0);
    let error = list(&hidden, size)
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap_err();
    assert_eq!(
        (error.kind, error.field.as_str()),
        (ValidationErrorKind::InvalidValue, "geometry")
    );
}

#[test]
fn generic_shape_structural_depth_and_raster_budgets_are_shared_and_retryable() {
    let triangle = outline(&[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)]);
    let node =
        tile(1.0, 1.0, Color::WHITE).clip_with(triangle.clone(), motion_core::ClipMode::Geometric);
    let size = PixelSize::new(1, 1);
    let exact = ValidationLimits {
        max_nodes: 8,
        max_depth: 2,
    };
    let commands = node.compile(Size::new(1.0, 1.0), size, exact).unwrap();
    assert_eq!(commands.preflight(exact).unwrap().nodes, 8);
    for limits in [
        ValidationLimits {
            max_nodes: 7,
            ..exact
        },
        ValidationLimits {
            max_depth: 1,
            ..exact
        },
    ] {
        assert_eq!(
            node.compile(Size::new(1.0, 1.0), size, limits)
                .unwrap_err()
                .kind,
            ValidationErrorKind::StructuralLimit
        );
    }
    let size = PixelSize::new(2, 1);
    let child = Node::group([node.clone(), node])
        .clip_with(
            Rect::new(0.25, 0.0, 1.0, 1.0),
            motion_core::ClipMode::Geometric,
        )
        .opacity(0.5)
        .mask(
            tile(1.0, 1.0, Color::WHITE)
                .clip_with(triangle.clone(), motion_core::ClipMode::Geometric),
            MaskMode::InvertedAlpha,
        );
    let source = list(
        &Node::group([child.clone()]).clip_with(triangle, motion_core::ClipMode::Geometric),
        size,
    );
    let snapshot = source.clone();
    let frame = source
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    let prepared_snapshot = frame.clone();
    let output = frame.render_linear(RasterLimits::default()).unwrap();
    for render in [false, true] {
        let exact = RasterLimits {
            max_pixels: 2,
            max_bytes: if render {
                output.stats().peak_bytes
            } else {
                frame.stats().peak_bytes
            },
            max_work: if render {
                output.stats().work
            } else {
                frame.stats().work
            },
        };
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
            let error = if render {
                frame.render_linear(limits).unwrap_err()
            } else {
                source
                    .prepare(ValidationLimits::default(), limits)
                    .unwrap_err()
            };
            assert_eq!(
                (error.kind, error.field.as_str()),
                (ValidationErrorKind::StructuralLimit, field)
            );
            assert_eq!(source, snapshot);
            assert_eq!(frame, prepared_snapshot);
            if render {
                assert_eq!(frame.render_linear(exact).unwrap(), output);
            } else {
                assert_eq!(
                    source.prepare(ValidationLimits::default(), exact).unwrap(),
                    frame
                );
            }
        }
    }
    let one = prepare(&child, size);
    let two = prepare(&Node::group([child.clone(), child]), size);
    assert_eq!(
        one.render_linear(RasterLimits::default())
            .unwrap()
            .stats()
            .peak_bytes
            - one.stats().retained_bytes,
        two.render_linear(RasterLimits::default())
            .unwrap()
            .stats()
            .peak_bytes
            - two.stats().retained_bytes
    );
    for depth in [128, 129] {
        let mut commands = Vec::new();
        for _ in 0..depth {
            commands.push(DrawCommand::PushShapeLayer {
                opacity: 1.0,
                bounds: Rect::new(0.0, 0.0, 1.0, 1.0),
                blend_mode: BlendMode::SourceOver,
                clip: None,
                rect_clip: None,
                shape_clip: ShapeClip::new(&Path2D::new()).unwrap(),
            });
        }
        for _ in 0..depth {
            commands.push(DrawCommand::PopLayer);
        }
        let source = DisplayList {
            output_size: size,
            background: None,
            commands,
        };
        let limits = ValidationLimits {
            max_nodes: 1000,
            max_depth: 128,
        };
        if depth == 128 {
            source
                .prepare(limits, RasterLimits::default())
                .unwrap()
                .render_linear(RasterLimits::default())
                .unwrap();
        } else {
            assert_eq!(source.preflight(limits).unwrap_err().field, "max_depth");
        }
    }
}

#[test]
fn rectangular_intersection_preserves_a_shared_fractional_edge_once() {
    let bounds = Rect::new(0.5, 0.0, 2.0, 1.0);
    let inner = tile(2.0, 1.0, Color::WHITE).clip_with(bounds, motion_core::ClipMode::Geometric);
    let node = Node::group([inner]).clip_with(bounds, motion_core::ClipMode::Geometric);
    let output = prepare(&node, PixelSize::new(2, 1))
        .render_linear(RasterLimits::default())
        .unwrap();
    assert_eq!(output.pixel(0, 0), Some([128.0 / 255.0; 4]));
    assert_eq!(output.pixel(1, 0), Some([1.0; 4]));
}

#[test]
fn rectangles_intersect_geometrically_even_when_disjoint_inside_one_pixel() {
    let intervals = [
        (0.0, 0.25),
        (0.25, 0.75),
        (0.5, 1.0),
        (0.0, 1.0),
        (0.1, 0.9),
    ];
    for (outer_left, outer_right) in intervals {
        for (inner_left, inner_right) in intervals {
            let inner = tile(1.0, 1.0, Color::WHITE).clip_with(
                Rect::new(inner_left, 0.0, inner_right, 1.0),
                motion_core::ClipMode::Geometric,
            );
            let node = Node::group([inner]).clip_with(
                Rect::new(outer_left, 0.0, outer_right, 1.0),
                motion_core::ClipMode::Geometric,
            );
            let width = (outer_right.min(inner_right) - outer_left.max(inner_left)).max(0.0);
            let expected = (width * 255.0).round() as f32 / 255.0;
            let pixel = prepare(&node, PixelSize::new(1, 1))
                .render_linear(RasterLimits::default())
                .unwrap()
                .pixel(0, 0)
                .unwrap();
            for channel in pixel {
                assert!(
                    (channel - expected).abs() <= f32::EPSILON,
                    "{outer_left}..{outer_right} intersect {inner_left}..{inner_right}: {channel} != {expected}"
                );
            }
        }
    }
    let node = Node::group([tile(1.0, 1.0, Color::WHITE).clip_with(
        Rect::new(0.25, 0.0, 0.75, 1.0),
        motion_core::ClipMode::Geometric,
    )])
    .clip_with(
        Rect::new(0.0, 0.25, 1.0, 0.75),
        motion_core::ClipMode::Geometric,
    );
    assert_eq!(
        prepare(&node, PixelSize::new(1, 1))
            .render_linear(RasterLimits::default())
            .unwrap()
            .pixel(0, 0),
        Some([64.0 / 255.0; 4])
    );
    let lower = RectClip {
        bounds: Rect::new(0.0, 0.0, 1.0, 1.0),
        transform: Transform::from_coefficients([1.0, 0.0, 1.0, 1.0, 0.0, 0.0]),
    };
    let upper = RectClip {
        bounds: lower.bounds,
        transform: Transform::from_coefficients([1.0, 1.0, 0.0, 1.0, 0.0, 0.0]),
    };
    let node = Node::group([
        tile(1.0, 1.0, Color::WHITE).clip_with(lower, motion_core::ClipMode::Geometric)
    ])
    .clip_with(upper, motion_core::ClipMode::Geometric);
    assert_eq!(
        prepare(&node, PixelSize::new(1, 1))
            .render_linear(RasterLimits::default())
            .unwrap()
            .pixel(0, 0),
        Some([0.0; 4])
    );
}

#[test]
fn rectangular_domains_cross_unclipped_isolation_without_refading() {
    let rect = Rect::new(0.5, 0.0, 2.0, 1.0);
    let color = Color::from_rgb_float(2.0, -0.125, 0.5);
    for opacity in [0.0, 0.5, 1.0e-50] {
        let leaf = tile(2.0, 1.0, color).clip_with(rect, motion_core::ClipMode::Geometric);
        let middle = Node::group([leaf]).opacity(opacity);
        let outer = Node::group([middle]).clip_with(rect, motion_core::ClipMode::Geometric);
        let frame = prepare(&outer, PixelSize::new(2, 1));
        let output = frame.render_linear(RasterLimits::default()).unwrap();
        for column in 0..2 {
            let covered = if column == 0 { 128.0 / 255.0 } else { 1.0 };
            let expected = color.to_linear_premultiplied().map(|channel| {
                let faded = (f64::from(channel as f32) * opacity) as f32;
                (f64::from(faded) * covered) as f32
            });
            assert_eq!(output.pixel(column, 0), Some(expected));
        }
    }
}

#[test]
fn rectangle_clips_follow_local_parent_contain_rotation_shear_and_reflection() {
    let mut clip = RectClip::new(Rect::new(0.0, 0.0, 1.0, 2.0));
    clip.transform = Transform::translate(1.0, 0.0);
    let node = tile(4.0, 2.0, Color::WHITE)
        .clip_with(clip, motion_core::ClipMode::Geometric)
        .translate(1.0, 1.0);
    let commands = node
        .compile(
            Size::new(6.0, 4.0),
            PixelSize::new(12, 12),
            ValidationLimits::default(),
        )
        .unwrap();
    match &commands.commands[0] {
        DrawCommand::PushRectLayer { rect_clip, .. } => assert_eq!(
            rect_clip.transform.apply(Point::ORIGIN),
            Point::new(4.0, 4.0)
        ),
        _ => panic!("expected a device-space rectangle layer"),
    }
    let output = commands
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap()
        .render_linear(RasterLimits::default())
        .unwrap();
    for point in [(4, 4), (5, 7)] {
        assert_eq!(output.pixel(point.0, point.1), Some([1.0; 4]));
    }
    for point in [(3, 4), (6, 4), (4, 3), (4, 8)] {
        assert_eq!(output.pixel(point.0, point.1), Some([0.0; 4]));
    }

    let diamond = RectClip {
        bounds: Rect::new(0.0, 0.0, 1.0, 1.0),
        transform: Transform::from_coefficients([1.0, 1.0, -1.0, 1.0, 1.0, 0.0]),
    };
    let node = Node::group([
        tile(2.0, 2.0, Color::WHITE).clip_with(diamond, motion_core::ClipMode::Geometric)
    ])
    .clip_with(
        Rect::new(0.0, 0.0, 2.0, 2.0),
        motion_core::ClipMode::Geometric,
    );
    assert_eq!(
        prepare(&node, PixelSize::new(2, 2))
            .render_linear(RasterLimits::default())
            .unwrap()
            .pixels(),
        &[[128.0 / 255.0; 4]; 4]
    );

    for (horizontal, vertical, angle) in [
        (1.0, 1.0, std::f64::consts::FRAC_PI_4),
        (-1.0, 1.0, 0.0),
        (1.0, -1.0, std::f64::consts::FRAC_PI_2),
        (1.5, 0.75, std::f64::consts::FRAC_PI_4),
    ] {
        let rect = Rect::new(0.25, 0.25, 1.75, 1.75);
        let single = tile(2.0, 2.0, Color::WHITE).clip_with(rect, motion_core::ClipMode::Geometric);
        let repeated =
            Node::group([single.clone()]).clip_with(rect, motion_core::ClipMode::Geometric);
        let place = |node: Node| {
            node.pivot(Pivot::Point(Point::ORIGIN))
                .scale_xy(horizontal, vertical)
                .rotate(angle)
                .translate(2.0, 1.0)
        };
        let one = prepare(&place(single), PixelSize::new(6, 6))
            .render_linear(RasterLimits::default())
            .unwrap();
        let two = prepare(&place(repeated), PixelSize::new(6, 6))
            .render_linear(RasterLimits::default())
            .unwrap();
        assert_eq!(
            one.pixels(),
            two.pixels(),
            "{horizontal}, {vertical}, {angle}"
        );
    }
}

#[test]
fn rectangle_snapshots_and_overlapping_groups_preserve_layout() {
    let original = tile(2.0, 1.0, Color::WHITE);
    let rect = Rect::new(0.5, 0.0, 2.0, 1.0);
    let clipped = Node::group([original.clone(), original.clone()])
        .clip_with(rect, motion_core::ClipMode::Geometric);
    let output = prepare(&clipped, PixelSize::new(2, 1))
        .render_linear(RasterLimits::default())
        .unwrap();
    assert_eq!(output.pixels(), &[[128.0 / 255.0; 4], [1.0; 4]]);
    assert_eq!(
        original.measure(ValidationLimits::default()).unwrap(),
        clipped.measure(ValidationLimits::default()).unwrap()
    );
    assert_eq!(list(&original, PixelSize::new(2, 1)).commands.len(), 1);
    let replaced = clipped.clone().clip_with(
        Rect::new(0.0, 0.0, 2.0, 1.0),
        motion_core::ClipMode::Geometric,
    );
    assert_eq!(
        prepare(&replaced, PixelSize::new(2, 1))
            .render_linear(RasterLimits::default())
            .unwrap()
            .pixels(),
        &[[1.0; 4]; 2]
    );
    assert_eq!(
        prepare(&clipped, PixelSize::new(2, 1))
            .render_linear(RasterLimits::default())
            .unwrap()
            .pixels(),
        output.pixels()
    );
    let row = Row::new().gap(1.0).children([
        original
            .clone()
            .clip_with(Rect::default(), motion_core::ClipMode::Geometric),
        original,
    ]);
    assert_eq!(
        row.measure(ValidationLimits::default()).unwrap(),
        Rect::new(0.0, 0.0, 5.0, 1.0)
    );
    let output = prepare(&row, PixelSize::new(5, 1))
        .render_linear(RasterLimits::default())
        .unwrap();
    assert_eq!(
        output.pixels(),
        &[[0.0; 4], [0.0; 4], [0.0; 4], [1.0; 4], [1.0; 4]]
    );
    assert!(
        clipped
            .opacity(0.0)
            .compile(
                Size::new(5.0, 1.0),
                PixelSize::new(5, 1),
                ValidationLimits::default()
            )
            .is_ok()
    );
}

#[test]
fn rectangles_normalize_ancestors_but_keep_path_and_content_masks_independent() {
    let size = PixelSize::new(2, 1);
    let rect = Rect::new(0.5, 0.0, 2.0, 1.0);
    let covered = 128.0_f64 / 255.0;
    for mode in [
        MaskMode::Alpha,
        MaskMode::InvertedAlpha,
        MaskMode::Luminance,
        MaskMode::InvertedLuminance,
    ] {
        let mask = tile(2.0, 1.0, Color::from_hex(0xff0000))
            .clip_with(rect, motion_core::ClipMode::Geometric)
            .opacity(0.5);
        let source = Node::group([tile(2.0, 1.0, Color::BLACK), tile(2.0, 1.0, Color::BLACK)])
            .clip_with(rect, motion_core::ClipMode::Geometric)
            .clip(rectangle(rect))
            .mask(mask, mode)
            .opacity(0.5);
        let node = Node::group([source])
            .clip_with(rect, motion_core::ClipMode::Geometric)
            .blend_mode(BlendMode::Multiply);
        let mut commands = list(&node, size);
        commands.background = Some(Color::WHITE);
        let frame = commands
            .prepare(ValidationLimits::default(), RasterLimits::default())
            .unwrap();
        assert_eq!(frame.items().len(), commands.commands.len());
        let output = frame.render_linear(RasterLimits::default()).unwrap();
        for column in 0..2 {
            let geometric = if column == 0 { covered } else { 1.0 };
            let artwork = f64::from((0.5 * geometric) as f32);
            let coverage = match mode {
                MaskMode::Alpha => artwork,
                MaskMode::InvertedAlpha => 1.0 - artwork,
                MaskMode::Luminance => 0.2126 * artwork,
                MaskMode::InvertedLuminance => 1.0 - 0.2126 * artwork,
            };
            let masked = coverage as f32;
            let inner = (f64::from(masked) * 0.5 * geometric) as f32;
            let alpha = (f64::from(inner) * geometric) as f32;
            let channel = (1.0 - f64::from(alpha)) as f32;
            assert_eq!(
                output.pixel(column, 0),
                Some([channel, channel, channel, 1.0]),
                "{mode:?}"
            );
        }
    }
    let source = Node::group([
        tile(2.0, 1.0, Color::WHITE),
        tile(2.0, 1.0, Color::BLACK)
            .clip_with(rect, motion_core::ClipMode::Geometric)
            .opacity(0.5)
            .blend_mode(BlendMode::Multiply),
    ])
    .clip_with(rect, motion_core::ClipMode::Geometric);
    let output = prepare(&source, size)
        .render_linear(RasterLimits::default())
        .unwrap();
    assert_eq!(
        output.pixel(0, 0),
        Some([64.0 / 255.0, 64.0 / 255.0, 64.0 / 255.0, 128.0 / 255.0])
    );
    assert_eq!(output.pixel(1, 0), Some([0.5, 0.5, 0.5, 1.0]));
}

#[test]
fn empty_hidden_invalid_rectangles_and_edited_lists_validate_before_allocation() {
    let size = PixelSize::new(2, 1);
    for rect in [Rect::default(), Rect::new(100.0, 100.0, 101.0, 101.0)] {
        let frame = prepare(
            &tile(2.0, 1.0, Color::WHITE).clip_with(rect, motion_core::ClipMode::Geometric),
            size,
        );
        assert_eq!(
            frame
                .render_linear(RasterLimits::default())
                .unwrap()
                .pixels(),
            &[[0.0; 4]; 2]
        );
    }
    let invalid = [
        RectClip::new(Rect::new(f64::NAN, 0.0, 1.0, 1.0)),
        RectClip::new(Rect::new(1.0, 0.0, 0.0, 1.0)),
        RectClip {
            bounds: Rect::new(0.0, 0.0, 1.0, 1.0),
            transform: Transform::translate(f64::INFINITY, 0.0),
        },
    ];
    for clip in invalid {
        for node in [
            Node::empty().clip_with(clip, motion_core::ClipMode::Geometric),
            tile(2.0, 1.0, Color::WHITE)
                .clip_with(clip, motion_core::ClipMode::Geometric)
                .translate(100.0, 0.0)
                .opacity(0.0),
        ] {
            for error in [
                node.measure(ValidationLimits::default()).unwrap_err(),
                node.compile(Size::new(2.0, 1.0), size, ValidationLimits::default())
                    .unwrap_err(),
            ] {
                assert_eq!(error.kind, ValidationErrorKind::InvalidValue);
                assert!(error.path.contains(".rect_clip"));
            }
        }
    }
    let empty = Node::empty()
        .clip_with(
            RectClip {
                bounds: Rect::new(0.0, 0.0, 1.0, 1.0),
                transform: Transform::scale(2.0),
            },
            motion_core::ClipMode::Geometric,
        )
        .pivot(Pivot::Point(Point::ORIGIN))
        .scale(f64::MAX);
    let error = empty
        .compile(Size::new(2.0, 1.0), size, ValidationLimits::default())
        .unwrap_err();
    assert_eq!(error.kind, ValidationErrorKind::InvalidValue);
    assert!(error.path.contains(".rect_clip"));
    let mut commands = list(
        &tile(2.0, 1.0, Color::WHITE).clip_with(
            Rect::new(0.0, 0.0, 2.0, 1.0),
            motion_core::ClipMode::Geometric,
        ),
        size,
    );
    if let DrawCommand::PushRectLayer { rect_clip, .. } = &mut commands.commands[0] {
        rect_clip.bounds.right = f64::NAN;
    }
    let error = commands
        .prepare(
            ValidationLimits::default(),
            RasterLimits {
                max_bytes: 1,
                ..RasterLimits::default()
            },
        )
        .unwrap_err();
    assert_eq!(error.kind, ValidationErrorKind::InvalidValue);
    assert!(error.path.contains(".rect_clip"));
}

#[test]
fn rectangle_structural_limits_and_mask_phase_boundaries_remain_bounded() {
    let size = PixelSize::new(1, 1);
    let rect = Rect::new(0.0, 0.0, 1.0, 1.0);
    let node = tile(1.0, 1.0, Color::WHITE).clip_with(rect, motion_core::ClipMode::Geometric);
    let exact = ValidationLimits {
        max_nodes: 4,
        max_depth: 2,
    };
    let commands = node.compile(Size::new(1.0, 1.0), size, exact).unwrap();
    assert_eq!(commands.preflight(exact).unwrap().nodes, 4);
    for limits in [
        ValidationLimits {
            max_nodes: 3,
            ..exact
        },
        ValidationLimits {
            max_depth: 1,
            ..exact
        },
    ] {
        assert_eq!(
            node.compile(Size::new(1.0, 1.0), size, limits)
                .unwrap_err()
                .kind,
            ValidationErrorKind::StructuralLimit
        );
    }
    let source_heavy = Node::group([Node::empty(), Node::empty(), Node::empty()])
        .clip_with(rect, motion_core::ClipMode::Geometric);
    assert_eq!(
        source_heavy
            .compile(Size::new(1.0, 1.0), size, exact)
            .unwrap_err()
            .field,
        "max_nodes"
    );
    let push = || DrawCommand::PushRectLayer {
        opacity: 1.0,
        bounds: rect,
        blend_mode: BlendMode::SourceOver,
        clip: None,
        rect_clip: rect.into(),
    };
    for depth in [128, 129] {
        let mut commands = Vec::new();
        commands.extend((0..depth).map(|_| push()));
        commands.extend((0..depth).map(|_| DrawCommand::PopLayer));
        let commands = DisplayList {
            output_size: size,
            background: None,
            commands,
        };
        let limits = ValidationLimits {
            max_nodes: 1000,
            max_depth: 128,
        };
        if depth == 128 {
            let frame = commands.prepare(limits, RasterLimits::default()).unwrap();
            assert_eq!(
                frame
                    .render_linear(RasterLimits::default())
                    .unwrap()
                    .pixel(0, 0),
                Some([0.0; 4])
            );
        } else {
            assert_eq!(commands.preflight(limits).unwrap_err().field, "max_depth");
        }
    }
    let crossing = DisplayList {
        output_size: size,
        background: None,
        commands: vec![
            push(),
            DrawCommand::BeginMask {
                mode: MaskMode::Alpha,
            },
            DrawCommand::PopLayer,
            DrawCommand::EndMask,
        ],
    };
    assert_eq!(
        crossing
            .preflight(ValidationLimits::default())
            .unwrap_err()
            .field,
        "layers"
    );
}

#[test]
fn rectangular_preparation_and_render_budgets_are_exact_and_retryable() {
    let size = PixelSize::new(8, 4);
    let rect = Rect::new(0.5, 0.25, 7.5, 3.75);
    let child = Node::group([tile(8.0, 4.0, Color::WHITE), tile(8.0, 4.0, Color::WHITE)])
        .clip_with(rect, motion_core::ClipMode::Geometric)
        .clip(rectangle(rect))
        .opacity(0.75)
        .mask(
            tile(8.0, 4.0, Color::WHITE.with_alpha(0.5))
                .clip_with(rect, motion_core::ClipMode::Geometric),
            MaskMode::InvertedAlpha,
        );
    let node = Node::group([child.clone()]).clip_with(
        Rect::new(0.25, 0.0, 7.75, 4.0),
        motion_core::ClipMode::Geometric,
    );
    let source = list(&node, size);
    let source_snapshot = source.clone();
    let frame = source
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    let snapshot = frame.clone();
    let output = frame.render_linear(RasterLimits::default()).unwrap();
    for rendering in [false, true] {
        let exact = RasterLimits {
            max_pixels: 32,
            max_bytes: if rendering {
                output.stats().peak_bytes
            } else {
                frame.stats().peak_bytes
            },
            max_work: if rendering {
                output.stats().work
            } else {
                frame.stats().work
            },
        };
        for (limits, field) in [
            (
                RasterLimits {
                    max_pixels: 31,
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
            let error = if rendering {
                frame.render_linear(limits).unwrap_err()
            } else {
                source
                    .prepare(ValidationLimits::default(), limits)
                    .unwrap_err()
            };
            assert_eq!(
                (error.kind, error.field.as_str()),
                (ValidationErrorKind::StructuralLimit, field)
            );
            assert_eq!(source, source_snapshot);
            assert_eq!(frame, snapshot);
            if rendering {
                assert_eq!(frame.render_linear(exact).unwrap(), output);
            } else {
                assert_eq!(
                    source.prepare(ValidationLimits::default(), exact).unwrap(),
                    frame
                );
            }
        }
    }
    let one = prepare(&child, size);
    let two = prepare(&Node::group([child.clone(), child]), size);
    let one_peak = one
        .render_linear(RasterLimits::default())
        .unwrap()
        .stats()
        .peak_bytes
        - one.stats().retained_bytes;
    let two_peak = two
        .render_linear(RasterLimits::default())
        .unwrap()
        .stats()
        .peak_bytes
        - two.stats().retained_bytes;
    assert_eq!(one_peak, two_peak);
}

#[test]
fn rectangle_intersection_rejects_unresolvable_visible_cancellation() {
    let huge = RectClip {
        bounds: Rect::new(-1.0e12, -1.0e12, 1.0e12, 0.0),
        transform: Transform::rotate(std::f64::consts::FRAC_PI_4)
            .then(Transform::translate(0.5, 0.5)),
    };
    let source = list(
        &tile(1.0, 1.0, Color::WHITE).clip_with(huge, motion_core::ClipMode::Geometric),
        PixelSize::new(1, 1),
    );
    let error = source
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap_err();
    assert_eq!(
        (error.kind, error.field.as_str()),
        (ValidationErrorKind::InvalidValue, "rect_clip")
    );
    let huge = tile(1.0, 1.0, Color::WHITE).clip_with(
        Rect::new(-1.0e12, -1.0e12, 1.0e12, 1.0e12),
        motion_core::ClipMode::Geometric,
    );
    assert_eq!(
        prepare(&huge, PixelSize::new(1, 1))
            .render_linear(RasterLimits::default())
            .unwrap()
            .pixel(0, 0),
        Some([1.0; 4])
    );
}

#[test]
fn prepared_mask_preserves_fractional_coverage_instead_of_using_its_bounds() {
    let node: motion_core::Node = Rect::new(0.5, 0.0, 2.0, 1.0).fill(Color::WHITE).into();
    let frame = node
        .compile(
            motion_core::Size::new(2.0, 1.0),
            PixelSize::new(2, 1),
            ValidationLimits::default(),
        )
        .unwrap()
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    let bounds = PixelRect {
        left: 0,
        top: 0,
        right: 2,
        bottom: 1,
    };
    let record = ClipRecord {
        bounds,
        strips: 0..frame.strips.len(),
        spans: 0..frame.spans.len(),
    };
    let mut budget = Budget::new(frame.size, RasterLimits::default()).unwrap();
    let mask = ClipMask::new(&frame, &record, bounds, &mut budget, "test.clip").unwrap();
    assert_eq!([mask.alpha(0, 0), mask.alpha(1, 0)], [128, 255]);
    assert_eq!(mask.bytes(), 2);
}

#[test]
fn overlapping_children_receive_the_path_clip_once_per_completed_group() {
    let tile = || Node::from(Rect::new(0.0, 0.0, 2.0, 1.0).fill(Color::WHITE));
    let path = Path2D::from_rect(Rect::new(0.5, 0.0, 2.0, 1.0)).unwrap();
    let node = Node::group([tile(), tile()]).clip(path);
    let frame = node
        .compile(
            motion_core::Size::new(2.0, 1.0),
            PixelSize::new(2, 1),
            ValidationLimits::default(),
        )
        .unwrap()
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    assert_eq!(frame.clips().len(), 1);
    let output = frame.render_linear(RasterLimits::default()).unwrap();
    assert_eq!(output.pixel(0, 0), Some([128.0 / 255.0; 4]));
    assert_eq!(output.pixel(1, 0), Some([1.0; 4]));
}

#[test]
fn local_clip_transform_follows_node_rotation_and_contain_mapping() {
    let original = tile(4.0, 2.0, Color::WHITE).translate(1.0, 1.0);
    let mut clip = rectangle(Rect::new(0.0, 0.0, 1.0, 2.0));
    clip.transform = Transform::translate(1.0, 0.0);
    let node = original.clone().clip(clip);
    assert_eq!(
        original.measure(ValidationLimits::default()).unwrap(),
        node.measure(ValidationLimits::default()).unwrap()
    );
    let list = node
        .compile(
            Size::new(6.0, 4.0),
            PixelSize::new(12, 12),
            ValidationLimits::default(),
        )
        .unwrap();
    match &list.commands[0] {
        DrawCommand::PushLayer {
            clip: Some(clip), ..
        } => assert_eq!(clip.transform.apply(Point::ORIGIN), Point::new(4.0, 4.0)),
        _ => panic!("expected transformed clip"),
    }
    let output = list
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap()
        .render_linear(RasterLimits::default())
        .unwrap();
    for point in [(4, 4), (5, 7)] {
        assert_eq!(output.pixel(point.0, point.1), Some([1.0; 4]));
    }
    for point in [(3, 4), (6, 4), (4, 3), (4, 8)] {
        assert_eq!(output.pixel(point.0, point.1), Some([0.0; 4]));
    }
    let rotated = tile(2.0, 2.0, Color::WHITE)
        .clip(rectangle(Rect::new(0.0, 0.0, 1.0, 2.0)))
        .rotate(std::f64::consts::FRAC_PI_2);
    let output = prepare(&rotated, PixelSize::new(2, 2))
        .render_linear(RasterLimits::default())
        .unwrap();
    assert_eq!(output.pixels(), &[[1.0; 4], [1.0; 4], [0.0; 4], [0.0; 4]]);
}

#[test]
fn curves_and_fill_rules_generate_mask_coverage_without_paint_records() {
    let mut curve = Path2D::new();
    curve.try_move_to(Point::ORIGIN).unwrap();
    curve
        .try_quadratic_to(Point::new(0.5, 2.0), Point::new(1.0, 0.0))
        .unwrap();
    let frame = prepare(
        &tile(1.0, 1.0, Color::WHITE).clip(curve),
        PixelSize::new(1, 1),
    );
    assert_eq!(
        frame
            .render_linear(RasterLimits::default())
            .unwrap()
            .pixel(0, 0),
        Some([169.0 / 255.0; 4])
    );
    assert_eq!(frame.paints().len(), 1);
    let mut path = Path2D::new();
    for (left, right) in [(0.0, 3.0), (1.0, 2.0)] {
        path.try_move_to(Point::new(left, 0.0)).unwrap();
        path.try_line_to(Point::new(right, 0.0)).unwrap();
        path.try_line_to(Point::new(right, 1.0)).unwrap();
        path.try_line_to(Point::new(left, 1.0)).unwrap();
        path.try_close().unwrap();
    }
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let mut clip = PathClip::new(path.clone());
        clip.rule = rule;
        let frame = prepare(
            &tile(3.0, 1.0, Color::WHITE).clip(clip),
            PixelSize::new(3, 1),
        );
        let output = frame.render_linear(RasterLimits::default()).unwrap();
        assert_eq!(output.pixel(0, 0), Some([1.0; 4]));
        assert_eq!(output.pixel(2, 0), Some([1.0; 4]));
        assert_eq!(
            output.pixel(1, 0),
            Some(if rule == FillRule::NonZero {
                [1.0; 4]
            } else {
                [0.0; 4]
            })
        );
        assert_eq!(frame.clips().len(), 1);
        assert_eq!(frame.paints().len(), 1);
    }
}

#[test]
fn nested_masks_opacity_and_blending_apply_in_the_documented_order() {
    let clip = rectangle(Rect::new(0.5, 0.0, 1.0, 1.0));
    let inner = tile(1.0, 1.0, Color::WHITE).clip(clip.clone());
    let nested = Node::group([inner]).clip(clip.clone());
    let covered = 128.0_f32 / 255.0;
    let expected = (f64::from(covered) * (128.0 / 255.0)) as f32;
    assert_eq!(
        prepare(&nested, PixelSize::new(1, 1))
            .render_linear(RasterLimits::default())
            .unwrap()
            .pixel(0, 0),
        Some([expected; 4])
    );
    let blended = Node::group([
        tile(2.0, 1.0, Color::WHITE),
        tile(2.0, 1.0, Color::BLACK)
            .clip(clip)
            .opacity(0.5)
            .blend_mode(BlendMode::Multiply),
    ]);
    let output = prepare(&blended, PixelSize::new(2, 1))
        .render_linear(RasterLimits::default())
        .unwrap();
    let channel = (1.0 - f64::from(covered * 0.5)) as f32;
    assert_eq!(output.pixel(0, 0), Some([channel, channel, channel, 1.0]));
    assert_eq!(output.pixel(1, 0), Some([1.0; 4]));
}

#[test]
fn empty_offscreen_clips_and_invalid_hidden_geometry_remain_explicit() {
    let size = PixelSize::new(2, 1);
    for clip in [
        PathClip::new(Path2D::new()),
        rectangle(Rect::new(100.0, 100.0, 101.0, 101.0)),
    ] {
        let frame = prepare(&tile(2.0, 1.0, Color::WHITE).clip(clip), size);
        assert_eq!(frame.clips().len(), 1);
        assert_eq!(
            frame
                .render_linear(RasterLimits::default())
                .unwrap()
                .pixels(),
            &[[0.0; 4]; 2]
        );
    }
    let mut invalid = rectangle(Rect::new(0.0, 0.0, 2.0, 1.0));
    invalid.transform = Transform::translate(f64::NAN, 0.0);
    for node in [
        Node::empty().clip(invalid.clone()),
        tile(2.0, 1.0, Color::WHITE).clip(invalid).opacity(0.0),
    ] {
        let error = node
            .compile(Size::new(2.0, 1.0), size, ValidationLimits::default())
            .unwrap_err();
        assert_eq!(error.kind, ValidationErrorKind::InvalidValue);
        assert!(error.path.contains(".clip"));
    }
    let node = tile(2.0, 1.0, Color::WHITE)
        .clip(rectangle(Rect::new(0.5, 0.0, 2.0, 1.0)))
        .opacity(0.0);
    let mut edited = list(&node, size);
    if let DrawCommand::PushLayer {
        clip: Some(clip), ..
    } = &mut edited.commands[0]
    {
        clip.transform = Transform::scale(f64::MAX);
    }
    let error = edited.preflight(ValidationLimits::default()).unwrap_err();
    assert_eq!(
        (error.kind, error.field.as_str()),
        (ValidationErrorKind::InvalidValue, "coordinates")
    );
    assert!(error.path.contains(".clip.verbs["));
}

#[test]
fn clip_verbs_share_structural_limits_and_snapshots_preserve_old_boundaries() {
    let original = tile(2.0, 1.0, Color::WHITE);
    let clip = rectangle(Rect::new(0.5, 0.0, 2.0, 1.0));
    let clipped = original.clone().clip(clip);
    let size = PixelSize::new(2, 1);
    let exact = ValidationLimits {
        max_nodes: 8,
        max_depth: 2,
    };
    let commands = clipped.compile(Size::new(2.0, 1.0), size, exact).unwrap();
    assert_eq!(commands.preflight(exact).unwrap().nodes, 8);
    for limits in [
        ValidationLimits {
            max_nodes: 7,
            ..exact
        },
        ValidationLimits {
            max_depth: 1,
            ..exact
        },
    ] {
        assert_eq!(
            commands.preflight(limits).unwrap_err().kind,
            ValidationErrorKind::StructuralLimit
        );
        assert_eq!(
            clipped
                .compile(Size::new(2.0, 1.0), size, limits)
                .unwrap_err()
                .kind,
            ValidationErrorKind::StructuralLimit
        );
    }
    assert_eq!(list(&original, size).commands.len(), 1);
    assert_eq!(
        original.measure(exact).unwrap(),
        clipped.measure(exact).unwrap()
    );
    assert!(original.measure(exact).is_ok());
    assert!(
        clipped
            .clone()
            .opacity(0.0)
            .compile(Size::new(2.0, 1.0), size, exact)
            .is_ok()
    );
    let replaced = clipped.clip(rectangle(Rect::new(0.0, 0.0, 2.0, 1.0)));
    let frame = prepare(&replaced, size);
    assert_eq!(frame.clips().len(), 1);
    assert_eq!(
        frame
            .render_linear(RasterLimits::default())
            .unwrap()
            .pixels(),
        &[[1.0; 4]; 2]
    );
}

#[test]
fn preparation_and_render_budgets_include_clip_storage_and_recover() {
    let size = PixelSize::new(16, 5);
    let node = tile(16.0, 5.0, Color::WHITE).clip(rectangle(Rect::new(0.5, 0.0, 15.5, 5.0)));
    let source = list(&node, size);
    let original_source = source.clone();
    let frame = source
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    let prep = frame.stats();
    let exact = RasterLimits {
        max_pixels: 80,
        max_bytes: prep.peak_bytes,
        max_work: prep.work,
    };
    assert_eq!(
        source.prepare(ValidationLimits::default(), exact).unwrap(),
        frame
    );
    for (limits, field) in [
        (
            RasterLimits {
                max_pixels: 79,
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
        assert_eq!(source, original_source);
        assert_eq!(
            source.prepare(ValidationLimits::default(), exact).unwrap(),
            frame
        );
    }
    let original = frame.clone();
    let output = frame.render_linear(RasterLimits::default()).unwrap();
    let exact = RasterLimits {
        max_pixels: 80,
        max_bytes: output.stats().peak_bytes,
        max_work: output.stats().work,
    };
    assert_eq!(frame.render_linear(exact).unwrap(), output);
    for (limits, field) in [
        (
            RasterLimits {
                max_pixels: 79,
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
        let error = frame.render_linear(limits).unwrap_err();
        assert_eq!(
            (error.kind, error.field.as_str()),
            (ValidationErrorKind::StructuralLimit, field)
        );
        assert_eq!(frame, original);
        assert_eq!(frame.render_linear(exact).unwrap(), output);
    }
}

#[test]
fn mask_subregions_full_spans_and_sibling_release_are_bounded() {
    let size = PixelSize::new(16, 5);
    let node = tile(16.0, 5.0, Color::WHITE).clip(rectangle(Rect::new(0.5, 0.0, 15.5, 5.0)));
    let frame = prepare(&node, size);
    let record = &frame.clips()[0];
    assert!(!record.strips.is_empty() && !record.spans.is_empty());
    let bounds = PixelRect {
        left: 4,
        top: 3,
        right: 13,
        bottom: 5,
    };
    let mut budget = Budget::new(size, RasterLimits::default()).unwrap();
    let mask = ClipMask::new(&frame, record, bounds, &mut budget, "test.mask").unwrap();
    assert_eq!(mask.bytes(), 18);
    for row in 3..5 {
        for column in 4..13 {
            assert_eq!(mask.alpha(column, row), 255);
        }
    }
    let exact = RasterLimits {
        max_pixels: 80,
        max_bytes: budget.peak_bytes(),
        max_work: budget.work_done(),
    };
    for limits in [
        RasterLimits {
            max_bytes: exact.max_bytes - 1,
            ..exact
        },
        RasterLimits {
            max_work: exact.max_work - 1,
            ..exact
        },
    ] {
        assert!(
            ClipMask::new(
                &frame,
                record,
                bounds,
                &mut Budget::new(size, limits).unwrap(),
                "test.mask"
            )
            .is_err()
        );
    }
    assert!(
        ClipMask::new(
            &frame,
            record,
            bounds,
            &mut Budget::new(size, exact).unwrap(),
            "test.mask"
        )
        .is_ok()
    );
    let single = frame.render_linear(RasterLimits::default()).unwrap();
    let siblings = prepare(&Node::group([node.clone(), node]), size);
    let double = siblings.render_linear(RasterLimits::default()).unwrap();
    assert_eq!(
        single.stats().peak_bytes - frame.stats().retained_bytes,
        double.stats().peak_bytes - siblings.stats().retained_bytes
    );
}
