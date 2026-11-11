use super::*;
use crate::Shape;
use crate::ValidationErrorKind;
use crate::{Color, DisplayList, DrawCommand, PixelSize, Size, ValidatedDisplayList};

#[test]
fn measured_text_groups_align_real_baselines_without_inferring_geometry()
-> Result<(), ValidationError> {
    let first = Rect::new(0.0, 0.0, 10.0, 20.0);
    let second = Rect::new(0.0, 0.0, 5.0, 10.0);
    let large = Node::group([first.fill(crate::Color::WHITE)])
        .with_text_metrics(15.0, first)
        .pivot(Pivot::Point(Point::ORIGIN))
        .scale(2.0);
    let small = Node::group([second.fill(crate::Color::WHITE)])
        .with_text_metrics(6.0, second)
        .translate(77.0, 88.0);
    let row = Row::new()
        .alignment(Alignment::Baseline)
        .gap(3.0)
        .children([large.clone(), small]);
    assert_eq!(
        row.measure(ValidationLimits::default())?,
        Rect::new(0.0, 0.0, 28.0, 40.0)
    );
    let list = row.compile(
        crate::Size::new(64.0, 64.0),
        crate::PixelSize::new(64, 64),
        ValidationLimits::default(),
    )?;
    let transforms = list
        .commands
        .iter()
        .filter_map(|command| {
            if let crate::DrawCommand::FillRect { transform, .. } = command {
                Some(*transform)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(transforms[0].apply(Point::new(0.0, 15.0)).y, 30.0);
    assert_eq!(
        transforms[1].apply(Point::new(0.0, 6.0)),
        Point::new(23.0, 30.0)
    );
    assert!(
        Row::new()
            .alignment(Alignment::Baseline)
            .children([large.rotate(0.1)])
            .measure(ValidationLimits::default())
            .is_err()
    );
    assert!(
        Row::new()
            .alignment(Alignment::Baseline)
            .children([Node::group([first.fill(crate::Color::WHITE)])])
            .measure(ValidationLimits::default())
            .is_err()
    );
    Ok(())
}

trait CompileFixture {
    fn compile_fixture(
        &self,
        limits: ValidationLimits,
    ) -> Result<ValidatedDisplayList, ValidationError>;
}

impl CompileFixture for Node {
    fn compile_fixture(
        &self,
        limits: ValidationLimits,
    ) -> Result<ValidatedDisplayList, ValidationError> {
        self.compile(Size::new(640.0, 360.0), PixelSize::new(640, 360), limits)
    }
}

fn bounded(rect: Rect) -> Node {
    Node::from(rect.fill(Color::WHITE))
}

#[test]
fn spatial_placement_preserves_local_coordinates_and_places_the_selected_pivot() {
    let limits = ValidationLimits::default();
    let rect = Rect::new(10.0, 20.0, 50.0, 80.0);
    assert_eq!(bounded(rect).measure(limits).unwrap(), rect);
    assert_eq!(
        bounded(rect)
            .at(Point::new(100.0, 200.0))
            .measure(limits)
            .unwrap(),
        Rect::new(80.0, 170.0, 120.0, 230.0)
    );
    assert_eq!(
        bounded(rect)
            .pivot(Pivot::TopLeft)
            .at(Point::new(100.0, 200.0))
            .measure(limits)
            .unwrap(),
        Rect::new(100.0, 200.0, 140.0, 260.0)
    );
    let changed = bounded(rect)
        .translate(3.0, -4.0)
        .rotate(std::f64::consts::FRAC_PI_2)
        .scale_xy(-2.0, 0.5);
    assert_eq!(
        changed.measure(limits).unwrap(),
        Rect::new(18.0, 6.0, 48.0, 86.0)
    );
    let reordered = bounded(rect)
        .scale_xy(-2.0, 0.5)
        .rotate(std::f64::consts::FRAC_PI_2)
        .translate(1.0, -1.0)
        .translate(2.0, -3.0);
    assert_eq!(
        changed.measure(limits).unwrap(),
        reordered.measure(limits).unwrap()
    );
    assert_eq!(
        bounded(rect).scale(0.0).measure(limits).unwrap(),
        Rect::new(30.0, 50.0, 30.0, 50.0)
    );
}

#[test]
fn spatial_measurement_unions_parent_space_bounds_without_losing_degenerate_geometry() {
    let limits = ValidationLimits::default();
    let first = bounded(Rect::new(10.0, 20.0, 50.0, 80.0)).translate(5.0, 6.0);
    let point = bounded(Rect::new(-8.0, -4.0, -8.0, -4.0));
    let group = Node::group([Node::empty(), first, point]).translate(2.0, 3.0);
    assert_eq!(
        group.measure(limits).unwrap(),
        Rect::new(-6.0, -1.0, 57.0, 89.0)
    );
    assert_eq!(Node::empty().measure(limits).unwrap(), Rect::default());
    assert_eq!(
        leaf(1.0).measure(limits).unwrap(),
        Rect::new(1.0, 0.0, 2.0, 1.0)
    );
    assert!(
        leaf(1.0)
            .translate(2.0, 3.0)
            .compile_fixture(limits)
            .is_ok()
    );
    assert!(leaf(1.0).rotate(0.2).compile_fixture(limits).is_ok());
    assert!(
        leaf(1.0)
            .pivot(Pivot::Point(Point::ORIGIN))
            .rotate(0.2)
            .compile_fixture(limits)
            .is_ok()
    );
}

#[test]
fn spatial_bridge_preserves_group_opacity_and_matches_corner_mapping() {
    let limits = ValidationLimits::default();
    let rect = Rect::new(10.0, 20.0, 50.0, 80.0);
    let node = bounded(rect)
        .scale_xy(1.25, -0.75)
        .rotate(0.4)
        .at(Point::new(60.0, 70.0))
        .opacity(0.5);
    let expected = node.measure(limits).unwrap();
    let scene = node.compile_fixture(limits).unwrap();
    assert!(matches!(
        scene.commands[0],
        DrawCommand::PushLayer { opacity: 0.5, .. }
    ));
    let mapped = scene
        .commands
        .iter()
        .find_map(|command| match command {
            DrawCommand::FillRect {
                bounds, transform, ..
            } => Some(crate::display_list::mapped_bounds(*bounds, *transform, "test").unwrap()),
            _ => None,
        })
        .unwrap();
    for (actual, expected) in [
        (mapped.left, expected.left),
        (mapped.top, expected.top),
        (mapped.right, expected.right),
        (mapped.bottom, expected.bottom),
    ] {
        assert!((actual - expected).abs() < 0.00002);
    }
    let rounded = bounded(rect)
        .opacity(1.0 - f64::EPSILON)
        .compile_fixture(limits)
        .unwrap();
    assert!(
        matches!(rounded.commands[0], DrawCommand::PushLayer { opacity, .. } if opacity == 1.0 - f64::EPSILON)
    );
    let pure = leaf(1.0)
        .translate(2.0, 3.0)
        .compile_fixture(limits)
        .unwrap();
    let DrawCommand::FillRect { transform, .. } = pure.commands[0] else {
        panic!("rectangle")
    };
    assert_eq!(transform.apply(Point::ORIGIN), Point::new(2.0, 3.0));
}

#[test]
fn spatial_f64_measurement_is_separate_from_fallible_reference_narrowing() {
    let limits = ValidationLimits::default();
    let rect = Rect::new(10.0, 20.0, 50.0, 80.0);
    let wide = bounded(rect).translate(1e40, 0.0);
    assert!(wide.measure(limits).is_ok());
    assert!(wide.compile_fixture(limits).is_ok());
    let tiny = bounded(rect).translate(f64::from_bits(1), 0.0);
    assert!(tiny.measure(limits).is_ok());
    assert!(tiny.compile_fixture(limits).is_ok());
    for invalid in [
        bounded(Rect::new(2.0, 0.0, 1.0, 2.0)),
        bounded(Rect::new(f64::NAN, 0.0, 1.0, 1.0)),
        bounded(rect).translate(f64::INFINITY, 0.0),
        bounded(rect).pivot(Pivot::Relative(f64::NAN, 0.0)),
    ] {
        assert!(invalid.measure(limits).is_err());
        assert!(invalid.opacity(0.0).compile_fixture(limits).is_err());
    }
}

#[test]
fn spatial_nested_groups_compose_about_parent_bounds_without_mutating_snapshots() {
    let limits = ValidationLimits::default();
    let rect = Rect::new(10.0, 20.0, 50.0, 80.0);
    let original = bounded(rect);
    let child = original.clone().translate(5.0, 6.0);
    let parent = Node::group([child])
        .scale(2.0)
        .rotate(std::f64::consts::FRAC_PI_2);
    let expected = Rect::new(-25.0, 16.0, 95.0, 96.0);
    assert_eq!(parent.measure(limits).unwrap(), expected);
    assert_eq!(original.measure(limits).unwrap(), rect);
    let scene = parent.compile_fixture(limits).unwrap();
    let DrawCommand::FillRect {
        bounds, transform, ..
    } = scene.commands[0]
    else {
        panic!("rectangle")
    };
    let actual = crate::display_list::mapped_bounds(bounds, transform, "test").unwrap();
    for (actual, expected) in [
        (actual.left, expected.left),
        (actual.top, expected.top),
        (actual.right, expected.right),
        (actual.bottom, expected.bottom),
    ] {
        assert!((actual - expected).abs() < 0.00002);
    }
    assert!(
        !scene
            .commands
            .iter()
            .any(|command| matches!(command, DrawCommand::PushLayer { .. }))
    );
}

fn leaf(marker: f64) -> Node {
    Node::from(Rect::new(marker, 0.0, marker + 1.0, 1.0).fill(Color::WHITE))
}

fn markers(scene: &DisplayList) -> Vec<f64> {
    scene
        .commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::FillRect { bounds, .. } => Some(bounds.left),
            _ => None,
        })
        .collect()
}

#[test]
fn stable_z_order_is_local_to_each_parent_even_when_neutral_groups_flatten() {
    let node = Node::group([
        Node::group([leaf(1.0).z_index(5), leaf(2.0).z_index(-1)]).z_index(0),
        leaf(3.0).z_index(0),
        leaf(4.0).z_index(-1),
    ]);
    let scene = node.compile_fixture(ValidationLimits::default()).unwrap();
    assert_eq!(markers(&scene), vec![4.0, 2.0, 1.0, 3.0]);
    assert!(
        !scene
            .commands
            .iter()
            .any(|command| matches!(command, DrawCommand::PushLayer { .. }))
    );
}

#[test]
fn group_opacity_is_isolated_once_and_keeps_child_opacity_unchanged() {
    let node = Node::group([leaf(1.0), leaf(2.0).opacity(0.75)]).opacity(0.25);
    let scene = node.compile_fixture(ValidationLimits::default()).unwrap();
    assert_eq!(scene.commands.len(), 6);
    assert!(matches!(
        scene.commands[0],
        DrawCommand::PushLayer { opacity: 0.25, .. }
    ));
    assert!(matches!(scene.commands[1], DrawCommand::FillRect { .. }));
    assert!(matches!(
        scene.commands[2],
        DrawCommand::PushLayer { opacity: 0.75, .. }
    ));
    assert!(matches!(scene.commands[3], DrawCommand::FillRect { .. }));
}

#[test]
fn snapshots_share_storage_and_modifiers_do_not_mutate_the_original() {
    let original = Node::group([leaf(1.0)]);
    let cloned = original.clone();
    assert!(Arc::ptr_eq(&original.data, &cloned.data));
    let changed = cloned.opacity(0.5).named("copy").z_index(9);
    assert_eq!(original.data.opacity, 1.0);
    assert_eq!(original.data.name, None);
    assert_eq!(changed.data.name.as_deref(), Some("copy"));
    let (NodeKind::Group(original_children), NodeKind::Group(changed_children)) =
        (&original.data.kind, &changed.data.kind)
    else {
        panic!("group");
    };
    assert!(Arc::ptr_eq(
        &original_children[0].data,
        &changed_children[0].data
    ));
    assert!(
        Node::from(None::<Node>)
            .compile_fixture(ValidationLimits::default())
            .unwrap()
            .commands
            .is_empty()
    );
    let collected: Node = [leaf(3.0), leaf(4.0)].into_iter().collect();
    assert_eq!(
        markers(
            &collected
                .compile_fixture(ValidationLimits::default())
                .unwrap()
        ),
        vec![3.0, 4.0]
    );
    assert!(
        changed
            .visible(false)
            .compile_fixture(ValidationLimits::default())
            .unwrap()
            .commands
            .is_empty()
    );
}

#[test]
fn validation_counts_original_structure_and_checks_zero_opacity_content() {
    let limits = ValidationLimits {
        max_nodes: 2,
        max_depth: 64,
    };
    let error = Node::group([leaf(1.0)])
        .compile_fixture(limits)
        .unwrap_err();
    assert_eq!(error.kind, ValidationErrorKind::StructuralLimit);
    assert_eq!(error.field, "max_nodes");
    let mut deep = Node::empty();
    for _ in 0..8 {
        deep = Node::group([deep]);
    }
    let error = deep
        .compile_fixture(ValidationLimits {
            max_nodes: 32,
            max_depth: 4,
        })
        .unwrap_err();
    assert_eq!(error.field, "max_depth");
    let invalid = bounded(Rect::new(0.0, 0.0, f64::NAN, 1.0))
        .opacity(0.0)
        .named("hidden");
    let error = invalid
        .compile_fixture(ValidationLimits::default())
        .unwrap_err();
    assert_eq!(error.field, "bounds");
    assert!(error.path.contains("hidden"));
    for opacity in [f64::NAN, f64::INFINITY, -0.5, 1.5] {
        assert_eq!(
            leaf(1.0)
                .opacity(opacity)
                .compile_fixture(ValidationLimits::default())
                .unwrap_err()
                .field,
            "opacity"
        );
    }
    assert!(
        Node::empty()
            .compile_fixture(ValidationLimits {
                max_nodes: 0,
                max_depth: 1
            })
            .is_err()
    );
    assert!(
        leaf(1.0)
            .opacity(f64::from_bits(1))
            .compile_fixture(ValidationLimits::default())
            .is_ok()
    );
}

#[test]
fn scene_closures_duration_wrappers_and_dynamic_sharing_use_explicit_context() {
    use crate::{FrameRate, SceneCache, Size, TimeLiteral};
    let callback = |context: &SceneContext<'_>| leaf(context.seconds());
    assert_eq!(callback.duration(), None);
    let scene = callback.with_duration(2.seconds());
    assert_eq!(scene.duration(), Some(2.seconds()));
    let shared: Arc<dyn Scene> = Arc::new(scene);
    let cache = SceneCache::new(0);
    let context = SceneContext::new(
        1.seconds().after_origin(),
        FrameRate::from(30),
        Size::new(16.0, 16.0),
        &cache,
    );
    let snapshot = shared.render(&context);
    std::thread::scope(|scope| {
        let handle = scope.spawn(|| {
            shared
                .render(&context)
                .compile_fixture(ValidationLimits::default())
                .unwrap()
        });
        assert_eq!(
            markers(&handle.join().unwrap()),
            markers(
                &snapshot
                    .compile_fixture(ValidationLimits::default())
                    .unwrap()
            )
        );
    });
    assert_eq!(
        shared.clone().with_duration(Duration::ZERO).duration(),
        Some(Duration::ZERO)
    );
    assert!(std::panic::catch_unwind(|| callback.with_duration(Duration::from_ticks(-1))).is_err());
}

#[test]
fn shared_reference_payloads_and_empty_nodes_cannot_bypass_structure_budgets() {
    let shared = leaf(1.0);
    let node = Node::group([shared.clone(), shared]);
    let exact = ValidationLimits {
        max_nodes: 5,
        max_depth: 3,
    };
    assert_eq!(
        markers(&node.compile_fixture(exact).unwrap()),
        vec![1.0, 1.0]
    );
    let error = node
        .compile_fixture(ValidationLimits {
            max_nodes: 4,
            ..exact
        })
        .unwrap_err();
    assert_eq!(error.field, "max_nodes");
    let error = node
        .compile_fixture(ValidationLimits {
            max_depth: 2,
            ..exact
        })
        .unwrap_err();
    assert_eq!(error.field, "max_depth");
    let empty = Node::group(vec![Node::empty(); 3]);
    assert!(
        empty
            .compile_fixture(ValidationLimits {
                max_nodes: 4,
                max_depth: 2
            })
            .unwrap()
            .commands
            .is_empty()
    );
    assert_eq!(
        empty
            .compile_fixture(ValidationLimits {
                max_nodes: 3,
                max_depth: 2
            })
            .unwrap_err()
            .field,
        "max_nodes"
    );
}
