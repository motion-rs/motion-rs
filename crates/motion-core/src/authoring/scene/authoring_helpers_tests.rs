//! Unit tests for scene callbacks, scene adapters and authoring helpers.

use super::*;
use crate::ValidationErrorKind;

#[test]
fn scene_closures_infer_context_and_groups_accept_empty_and_mixed_children() {
    let cache = crate::SceneCache::default();
    let context = SceneContext::new(
        Time::ZERO,
        crate::FrameRate::from(30),
        crate::Size::new(8.0, 8.0),
        &cache,
    );
    let build = crate::scene(|context| {
        crate::group![Node::empty(), context.bounds().fill(crate::Color::WHITE),]
    });
    assert_eq!(
        build
            .render(&context)
            .measure(ValidationLimits::default())
            .unwrap(),
        context.bounds()
    );
    assert!(crate::group![].measure(ValidationLimits::default()).is_ok());
}

#[test]
fn scene_accepts_drawables_and_results_without_losing_original_errors() {
    let cache = crate::SceneCache::default();
    let diagnostics = crate::RequestDiagnostics::default();
    let context = SceneContext::new(
        Time::ZERO,
        crate::FrameRate::from(30),
        crate::Size::new(8.0, 8.0),
        &cache,
    )
    .with_diagnostics(&diagnostics);
    let drawable = crate::scene(|context| context.bounds().fill(crate::Color::WHITE));
    assert_eq!(
        drawable(&context)
            .measure(ValidationLimits::default())
            .unwrap(),
        context.bounds()
    );
    let success = crate::scene(|context| -> crate::Result<_> {
        Ok(context.bounds().fill(crate::Color::WHITE))
    });
    assert!(
        success(&context)
            .measure(ValidationLimits::default())
            .is_ok()
    );
    let failure = crate::scene(|_| -> std::io::Result<Node> {
        Err(std::io::Error::other("scene resource failed"))
    });
    assert!(
        failure(&context)
            .measure(ValidationLimits::default())
            .is_err()
    );
    let errors = diagnostics.errors();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].kind(), crate::ErrorKind::Io);
    assert!(
        errors[0].build_errors()[0]
            .downcast_ref::<std::io::Error>()
            .is_some()
    );
}

#[test]
fn chainable_paths_retain_errors_and_valid_polylines_match_eager_verbs() {
    use crate::{Color, Path2D, Shape, Transform};
    let points = [Point::ORIGIN, Point::new(4.0, 0.0), Point::new(4.0, 4.0)];
    let path = Path2D::polyline(points).closed();
    let mut expected = Path2D::new();
    expected.try_move_to(points[0]).unwrap();
    expected.try_line_to(points[1]).unwrap();
    expected.try_line_to(points[2]).unwrap();
    expected.try_close().unwrap();
    assert_eq!(path, expected);
    let mut path = Path2D::new();
    path.move_to(Point::ORIGIN)
        .line_to(Point::new(f64::NAN, 0.0))
        .close();
    assert!(path.transformed(&Transform::IDENTITY).is_err());
    let error = Node::from(path.fill(Color::WHITE))
        .opacity(0.0)
        .measure(ValidationLimits::default())
        .unwrap_err();
    assert_eq!(error.field, "coordinates");
}

#[test]
fn authoring_arcs_match_checked_geometry_and_defer_invalid_radii() {
    use crate::{Color, Path2D, Shape, Vector2};
    let mut path = Path2D::new();
    path.arc_to(
        Point::ORIGIN,
        Vector2::new(4.0, 4.0),
        0.0,
        std::f64::consts::FRAC_PI_2,
    );
    let mut checked = Path2D::new();
    checked
        .try_arc_to(
            Point::ORIGIN,
            Vector2::new(4.0, 4.0),
            0.0,
            std::f64::consts::FRAC_PI_2,
        )
        .unwrap();
    assert_eq!(path, checked);
    path.arc_to(Point::ORIGIN, Vector2::new(-1.0, 1.0), 0.0, 1.0);
    assert!(
        Node::from(path.fill(Color::WHITE))
            .measure(ValidationLimits::default())
            .is_err()
    );
}

#[test]
fn deferred_path_errors_survive_cached_metrics_strokes_clips_and_raw_commands() {
    use crate::{
        ClipMode, Color, FillRule, MorphPath, Paint, Path2D, Shape, StrokeStyle, Transform,
    };
    let mut path = Path2D::polyline([
        Point::ORIGIN,
        Point::new(4.0, 0.0),
        Point::new(4.0, 4.0),
        Point::new(0.0, 4.0),
    ])
    .closed();
    assert_eq!(path.try_length().unwrap(), 16.0);
    let style = StrokeStyle::default();
    style.outline(&path, "stroke").unwrap();
    path.line_to(Point::new(f64::NAN, 0.0));
    assert!(path.axis_aligned_rectangle().is_none());
    assert!(path.try_length().is_err());
    assert!(style.outline(&path, "stroke").is_err());
    assert!(MorphPath::new(path.clone()).is_err());
    assert!(
        Node::from(path.clone().stroke(1.0, Color::WHITE))
            .measure(ValidationLimits::default())
            .is_err()
    );
    assert!(
        Node::empty()
            .clip_with(path.clone(), ClipMode::Geometric)
            .measure(ValidationLimits::default())
            .is_err()
    );

    struct RawPath(Path2D);

    impl Drawable for RawPath {
        fn bounds(&self, _: &DrawContext) -> Rect {
            Rect::new(0.0, 0.0, 4.0, 4.0)
        }

        fn draw(
            &self,
            _: &DrawContext,
            canvas: &mut crate::Canvas<'_>,
        ) -> Result<(), ValidationError> {
            canvas.fill_path(
                Arc::new(self.0.clone()),
                Transform::IDENTITY,
                Paint::from(Color::WHITE),
                FillRule::NonZero,
            )
        }
    }
    assert!(
        DrawContext::default()
            .record(&RawPath(path), ValidationLimits::default())
            .is_err()
    );
}

#[test]
fn tuple_groups_scene_outputs_and_push_preserve_child_commands() {
    use crate::{Circle, Color, PixelSize, Shape};
    let mixed = Node::group((
        Rect::new(0.0, 0.0, 2.0, 2.0).fill(Color::WHITE),
        Circle::new(Point::new(4.0, 4.0), 1.0).fill(Color::BLACK),
    ));
    let mut nodes = Node::group(());
    nodes.push(mixed.clone());
    nodes.push(Rect::new(6.0, 0.0, 8.0, 2.0).fill(Color::WHITE));
    assert_eq!(
        nodes
            .compile(
                crate::Size::new(8.0, 8.0),
                PixelSize::new(8, 8),
                ValidationLimits::default()
            )
            .unwrap()
            .commands
            .len(),
        3
    );
    let tuple: Node = (mixed, Node::empty()).into();
    assert_eq!(
        tuple.measure(ValidationLimits::default()).unwrap(),
        Rect::new(0.0, 0.0, 5.0, 5.0)
    );
}

#[test]
fn memo_reuses_nodes_and_distinguishes_float_inputs() {
    let cache = crate::SceneCache::default();
    let context = SceneContext::new(
        Time::ZERO,
        crate::FrameRate::from(30),
        crate::Size::new(8.0, 8.0),
        &cache,
    );
    let builds = std::cell::Cell::new(0);
    for value in [0.0, 0.0, -0.0, -0.0] {
        let node = context.memo(crate::key::F64(value), || {
            builds.set(builds.get() + 1);
            Rect::new(0.0, 0.0, 2.0, 2.0).fill(crate::Color::WHITE)
        });
        assert_eq!(
            node.measure(ValidationLimits::default()).unwrap(),
            Rect::new(0.0, 0.0, 2.0, 2.0)
        );
    }
    assert_eq!(builds.get(), 2);
}

#[test]
fn track_scene_accepts_drawables_tuples_and_result_outputs() {
    use crate::{Color, Scene, TimeLiteral, Timeline};
    let timeline = Timeline::new(crate::Size::new(8.0, 8.0), 30)
        .track(|track| {
            track
                .scene(1.seconds(), |_| {
                    Rect::new(0.0, 0.0, 2.0, 2.0).fill(Color::WHITE)
                })
                .scene(1.seconds(), |_| {
                    (
                        Node::empty(),
                        Rect::new(0.0, 0.0, 3.0, 3.0).fill(Color::WHITE),
                    )
                })
                .scene(1.seconds(), |_| Ok::<_, crate::Error>(Node::empty()))
        })
        .build()
        .unwrap();
    assert_eq!(Scene::duration(&timeline), Some(3.seconds()));
    assert!(timeline.validate().is_ok());
}

#[test]
fn deferred_layout_matches_eager_placement_and_rejects_invalid_parameters() {
    use crate::{Color, PixelSize, Side, TimeLiteral};
    let cache = crate::SceneCache::default();
    let context = SceneContext::new(
        Time::ZERO,
        crate::FrameRate::from(30),
        crate::Size::new(100.0, 100.0),
        &cache,
    );
    let reference = Rect::new(10.0, 10.0, 50.0, 30.0).fill(Color::WHITE).into();
    let content = Rect::new(0.0, 0.0, 8.0, 4.0).fill(Color::WHITE);
    let deferred = content
        .clone()
        .next_to(&reference, Side::Right, 6.0)
        .align_to(&reference, Side::Bottom);
    let eager = context
        .try_align_to(
            context
                .try_next_to(content, &reference, Side::Right, 6.0)
                .unwrap(),
            &reference,
            Side::Bottom,
        )
        .unwrap();
    assert_eq!(
        deferred.measure(ValidationLimits::default()).unwrap(),
        eager.measure(ValidationLimits::default()).unwrap()
    );
    assert!(
        Node::empty()
            .next_to(&reference, Side::Right, f64::NAN)
            .measure(ValidationLimits::default())
            .is_err()
    );
    let invalid = crate::camera(
        &context,
        Node::empty(),
        crate::Camera {
            zoom: 0.0,
            ..crate::Camera::default()
        },
    );
    assert_eq!(
        invalid
            .measure(ValidationLimits::default())
            .unwrap_err()
            .path,
        "camera"
    );
    assert!(
        context
            .motion_blur(1_i64.seconds(), 0, |_| Node::empty())
            .compile(
                context.size(),
                PixelSize::new(100, 100),
                ValidationLimits::default()
            )
            .is_err()
    );
}

#[test]
fn relative_layout_references_share_node_and_depth_limits() {
    use crate::{Color, PixelSize, Side, Size};
    let mut node: Node = Rect::new(0.0, 0.0, 1.0, 1.0).fill(Color::WHITE).into();
    for _ in 0..20 {
        let reference = node.clone();
        node = node.next_to(&reference, Side::Right, 0.0);
    }
    for limits in [
        ValidationLimits {
            max_nodes: 64,
            ..ValidationLimits::default()
        },
        ValidationLimits {
            max_depth: 4,
            ..ValidationLimits::default()
        },
    ] {
        assert_eq!(
            node.measure(limits).unwrap_err().kind,
            ValidationErrorKind::StructuralLimit
        );
        assert_eq!(
            node.compile(Size::new(8.0, 8.0), PixelSize::new(8, 8), limits)
                .unwrap_err()
                .kind,
            ValidationErrorKind::StructuralLimit
        );
    }
}
