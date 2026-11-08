use super::*;
use crate::{Color, Node, NodeModifiers, Pivot, Point, Row, ValidationErrorKind};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn stroke_expands_butt_line_into_nonzero_fill() -> Result<(), crate::ValidationError> {
    use super::{DrawCommand, DrawContext, FillRule, Shape};
    use crate::{Color, Path2D, Point, Rect, ValidationLimits};
    let mut path = Path2D::new();
    path.try_move_to(Point::new(1.0, 1.0))?;
    path.try_line_to(Point::new(5.0, 1.0))?;
    let commands = DrawContext::default().record(
        &path.clone().stroke(2.0, Color::WHITE),
        ValidationLimits::default(),
    )?;
    assert_eq!(commands.len(), 1);
    let DrawCommand::Fill {
        path: outline,
        rule,
        ..
    } = &commands[0]
    else {
        panic!("stroke must expand into fill geometry");
    };
    assert_eq!(*rule, FillRule::NonZero);
    assert_eq!(outline.bounding_box(), Rect::new(1.0, 0.0, 5.0, 2.0));
    assert_eq!(path.verbs().len(), 2);
    Ok(())
}

trait CompileFixture {
    fn compile_fixture(
        &self,
        limits: ValidationLimits,
    ) -> Result<crate::ValidatedDisplayList, ValidationError>;
}

impl CompileFixture for Node {
    fn compile_fixture(
        &self,
        limits: ValidationLimits,
    ) -> Result<crate::ValidatedDisplayList, ValidationError> {
        self.compile(
            crate::Size::new(640.0, 360.0),
            crate::PixelSize::new(640, 360),
            limits,
        )
    }
}

fn curved_path() -> Path2D {
    let mut path = Path2D::new();
    path.try_move_to(Point::ORIGIN).unwrap();
    path.try_quadratic_to(Point::new(5.0, 20.0), Point::new(10.0, 0.0))
        .unwrap();
    path.try_cubic_to(
        Point::new(12.0, -8.0),
        Point::new(18.0, -8.0),
        Point::new(20.0, 0.0),
    )
    .unwrap();
    path.try_close().unwrap();
    path
}

#[test]
fn path_recording_preserves_verbs_paint_rule_and_snapshot() {
    let mut path = curved_path();
    let before = path.clone();
    let color = Color::from_rgb_float(1.25, -0.125, 0.333);
    let commands = DrawContext::default()
        .record(&path.clone().fill(color), ValidationLimits::default())
        .unwrap();
    path.try_line_to(Point::new(30.0, 40.0)).unwrap();
    let DrawCommand::Fill {
        path: recorded,
        paint,
        rule,
        ..
    } = &commands[0]
    else {
        panic!("expected path fill")
    };
    assert_eq!(recorded.verbs(), before.verbs());
    assert_ne!(recorded.verbs(), path.verbs());
    assert_eq!(*paint, Paint::Solid(color));
    assert_eq!(*rule, FillRule::NonZero);
}

#[test]
fn even_odd_path_lowering_preserves_curve_commands_and_checks_all_controls() {
    let node = curved_path()
        .fill(Color::WHITE)
        .fill_rule(FillRule::EvenOdd)
        .translate(7.0, 9.0);
    let list = node
        .compile(
            crate::Size::new(64.0, 64.0),
            crate::PixelSize::new(64, 64),
            ValidationLimits::default(),
        )
        .unwrap();
    let DrawCommand::Fill {
        path,
        transform,
        rule,
        ..
    } = &list.commands[0]
    else {
        panic!("curve")
    };
    assert_eq!(path.verbs(), curved_path().verbs());
    assert_eq!(*rule, FillRule::EvenOdd);
    assert_eq!(transform.apply(Point::ORIGIN), Point::new(7.0, 9.0));
    for control in [Point::new(1.0e-100, 1.0), Point::new(4.0e38, 1.0)] {
        let mut path = Path2D::new();
        path.try_move_to(Point::ORIGIN).unwrap();
        path.try_quadratic_to(control, Point::new(2.0, 2.0))
            .unwrap();
        let node = path
            .fill(Color::WHITE)
            .fill_rule(FillRule::EvenOdd)
            .named("control-range");
        assert!(node.measure(ValidationLimits::default()).is_ok());
        let list = node.compile_fixture(ValidationLimits::default()).unwrap();
        let DrawCommand::Fill { path, .. } = &list.commands[0] else {
            panic!("control")
        };
        assert!(path.verbs().iter().any(
            |verb| matches!(verb, crate::PathVerb::QuadraticTo(first, _) if *first == control)
        ));
    }
}

#[test]
fn nonzero_path_rule_is_recorded_but_never_silently_changed_by_old_lowering() {
    let shape = curved_path().fill(Color::WHITE);
    let commands = DrawContext::default()
        .record(&shape, ValidationLimits::default())
        .unwrap();
    assert!(matches!(
        commands[0],
        DrawCommand::Fill {
            rule: FillRule::NonZero,
            ..
        }
    ));
    let node = Node::from(shape).opacity(0.0);
    let list = node.compile_fixture(ValidationLimits::default()).unwrap();
    assert!(list.commands.iter().any(|command| matches!(
        command,
        DrawCommand::Fill {
            rule: FillRule::NonZero,
            ..
        }
    )));
    let rect = Rect::new(0.0, 0.0, 10.0, 10.0).fill(Color::WHITE);
    assert!(
        Node::from(rect)
            .compile_fixture(ValidationLimits::default())
            .is_ok()
    );
}

#[test]
fn repeated_path_verbs_share_the_global_work_budget_without_extra_depth() {
    let path = curved_path();
    let verbs = path.verbs().len();
    let leaf: Node = path.fill(Color::WHITE).fill_rule(FillRule::EvenOdd).into();
    let group = Node::group([leaf.clone(), leaf]).opacity(0.0);
    let exact = ValidationLimits {
        max_nodes: 1 + 2 * (2 + verbs),
        max_depth: 3,
    };
    assert!(group.compile_fixture(exact).is_ok());
    let error = group
        .compile_fixture(ValidationLimits {
            max_nodes: exact.max_nodes - 1,
            ..exact
        })
        .unwrap_err();
    assert_eq!(error.field, "max_nodes");
    assert!(error.path.contains(".verbs["));
    assert!(
        group
            .measure(ValidationLimits {
                max_nodes: 3,
                max_depth: 2
            })
            .is_ok()
    );
}

#[test]
fn custom_shapes_use_default_path_bounds_and_generic_styling_in_layout() {
    struct Triangle(Path2D);

    impl Shape for Triangle {
        fn path(&self) -> Path2D {
            self.0.clone()
        }
    }
    let mut path = Path2D::new();
    path.try_move_to(Point::new(5.0, 7.0)).unwrap();
    path.try_line_to(Point::new(25.0, 7.0)).unwrap();
    path.try_line_to(Point::new(5.0, 17.0)).unwrap();
    path.try_close().unwrap();
    let triangle = Triangle(path)
        .fill(Color::BLACK)
        .fill_rule(FillRule::EvenOdd)
        .fill(Color::WHITE);
    let row = Row::new()
        .gap(3.0)
        .children([
            Node::from(triangle),
            Node::from(Rect::new(0.0, 0.0, 10.0, 20.0).fill(Color::WHITE)),
        ])
        .pivot(Pivot::TopLeft)
        .at(Point::new(100.0, 80.0));
    assert_eq!(
        row.measure(ValidationLimits::default()).unwrap(),
        Rect::new(100.0, 80.0, 133.0, 100.0)
    );
    assert_eq!(
        row.compile_fixture(ValidationLimits::default())
            .unwrap()
            .commands
            .len(),
        2
    );
}

#[test]
fn empty_paths_and_ignored_path_budget_errors_keep_recording_contracts() {
    let context = DrawContext::default();
    let empty = Path2D::new()
        .fill(Color::WHITE)
        .fill_rule(FillRule::EvenOdd);
    assert_eq!(
        context
            .record(&empty, ValidationLimits::default())
            .unwrap()
            .len(),
        1
    );
    assert!(
        Node::from(empty)
            .compile_fixture(ValidationLimits::default())
            .is_ok()
    );

    struct Ignores(Path2D);

    impl Drawable for Ignores {
        fn bounds(&self, _: &DrawContext) -> Rect {
            self.0.bounding_box()
        }

        fn draw(&self, _: &DrawContext, canvas: &mut Canvas<'_>) -> Result<(), ValidationError> {
            let _ = canvas.fill(&self.0, Color::WHITE, FillRule::EvenOdd);
            let _ = canvas.fill_rect(Rect::default(), Color::WHITE);
            Ok(())
        }
    }
    let error = context
        .record(
            &Ignores(curved_path()),
            ValidationLimits {
                max_nodes: 3,
                max_depth: 2,
            },
        )
        .unwrap_err();
    assert_eq!(error.field, "max_nodes");
    assert!(error.path.contains(".verbs["));
}

#[test]
fn rectangles_record_their_own_bounds_without_geometry_or_color_narrowing() {
    let bounds = Rect::new(0.1, 0.2, 10.3, 20.4);
    let color = Color::from_rgb_float(1.25, -0.125, 0.333).with_alpha(0.123);
    let drawable = bounds.fill(color);
    let context = DrawContext::new(2.0).unwrap();
    assert_eq!(context.device_scale(), 2.0);
    assert_eq!(drawable.bounds(&context), bounds);
    assert_eq!(
        context
            .record(&drawable, ValidationLimits::default())
            .unwrap(),
        vec![DrawCommand::FillRect {
            bounds,
            transform: Transform::IDENTITY,
            paint: Paint::Solid(color)
        }]
    );
}

struct IgnoredErrors {
    bounds: Rect,
    commands: usize,
}

impl Drawable for IgnoredErrors {
    fn bounds(&self, _: &DrawContext) -> Rect {
        Rect::new(0.0, 0.0, 1.0, 1.0)
    }

    fn draw(&self, _: &DrawContext, canvas: &mut Canvas<'_>) -> Result<(), ValidationError> {
        for _ in 0..self.commands {
            let _ = canvas.fill_rect(self.bounds, Color::WHITE);
        }
        Ok(())
    }
}

#[test]
fn ignored_canvas_failures_cannot_bypass_shared_command_or_depth_limits() {
    let content = IgnoredErrors {
        bounds: Rect::new(0.0, 0.0, 1.0, 1.0),
        commands: 3,
    };
    let context = DrawContext::default();
    let exact = ValidationLimits {
        max_nodes: 4,
        max_depth: 2,
    };
    assert_eq!(context.record(&content, exact).unwrap().len(), 3);
    let error = context
        .record(
            &content,
            ValidationLimits {
                max_nodes: 3,
                ..exact
            },
        )
        .unwrap_err();
    assert_eq!(error.kind, ValidationErrorKind::StructuralLimit);
    assert_eq!(error.field, "max_nodes");
    assert_eq!(error.path, "drawable.commands[2]");
    assert_eq!(
        context
            .record(
                &content,
                ValidationLimits {
                    max_depth: 1,
                    ..exact
                }
            )
            .unwrap_err()
            .field,
        "max_depth"
    );
}

#[test]
fn invalid_commands_and_bounds_fail_even_when_drawing_discards_errors() {
    let context = DrawContext::default();
    for bounds in [
        Rect::new(0.0, 0.0, f64::NAN, 1.0),
        Rect::new(2.0, 0.0, 1.0, 1.0),
        Rect::new(-f64::MAX, 0.0, f64::MAX, 1.0),
    ] {
        let content = IgnoredErrors {
            bounds,
            commands: 2,
        };
        assert_eq!(
            context
                .record(&content, ValidationLimits::default())
                .unwrap_err()
                .field,
            "bounds"
        );
        assert_eq!(
            context
                .record(&bounds.fill(Color::WHITE), ValidationLimits::default())
                .unwrap_err()
                .field,
            "bounds"
        );
    }
    let point = Rect::new(1.0, 2.0, 1.0, 2.0).fill(Color::TRANSPARENT);
    assert_eq!(
        context
            .record(&point, ValidationLimits::default())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn drawing_context_rejects_invalid_scale_and_limits() {
    for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(DrawContext::new(scale).is_err());
    }
    let content = Rect::default().fill(Color::WHITE);
    assert_eq!(
        DrawContext::default()
            .record(
                &content,
                ValidationLimits {
                    max_nodes: 0,
                    max_depth: 2
                }
            )
            .unwrap_err()
            .field,
        "limits"
    );
}

#[test]
fn leaf_bounds_drive_layout_and_placement_without_declarations() {
    let first = Rect::new(5.0, 7.0, 25.0, 17.0).fill(Color::WHITE);
    let row = Row::new()
        .gap(3.0)
        .children([
            first.into(),
            Node::from(Rect::new(0.0, 0.0, 10.0, 20.0).fill(Color::BLACK)),
        ])
        .pivot(Pivot::TopLeft)
        .at(Point::new(100.0, 80.0));
    assert_eq!(
        row.measure(ValidationLimits::default()).unwrap(),
        Rect::new(100.0, 80.0, 133.0, 100.0)
    );
    let list = row.compile_fixture(ValidationLimits::default()).unwrap();
    let origins = list
        .commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::FillRect { transform, .. } => Some(transform.apply(Point::ORIGIN)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(origins, [Point::new(95.0, 73.0), Point::new(123.0, 80.0)]);
}

struct Counted {
    bounds_calls: AtomicUsize,
    draw_calls: AtomicUsize,
}

impl Drawable for Counted {
    fn bounds(&self, context: &DrawContext) -> Rect {
        self.bounds_calls.fetch_add(1, Ordering::SeqCst);
        Rect::new(0.0, 0.0, context.device_scale() * 10.0, 20.0)
    }

    fn draw(&self, context: &DrawContext, canvas: &mut Canvas<'_>) -> Result<(), ValidationError> {
        self.draw_calls.fetch_add(1, Ordering::SeqCst);
        canvas.fill_rect(
            Rect::new(0.0, 0.0, context.device_scale() * 10.0, 20.0),
            Color::WHITE,
        )
    }
}

#[test]
fn shared_custom_leaves_measure_without_drawing_and_receive_explicit_context() {
    let content = Arc::new(Counted {
        bounds_calls: AtomicUsize::new(0),
        draw_calls: AtomicUsize::new(0),
    });
    let node = Node::from(content.clone() as Arc<dyn Drawable>);
    let cloned = node.clone();
    assert_eq!(Arc::strong_count(&content), 2);
    let placed = cloned.at(Point::new(100.0, 80.0));
    assert_eq!(Arc::strong_count(&content), 3);
    assert_eq!(
        node.measure(ValidationLimits::default()).unwrap(),
        Rect::new(0.0, 0.0, 10.0, 20.0)
    );
    let context = DrawContext::new(2.0).unwrap();
    assert_eq!(
        placed
            .measure_with(&context, ValidationLimits::default())
            .unwrap(),
        Rect::new(90.0, 70.0, 110.0, 90.0)
    );
    assert_eq!(content.draw_calls.load(Ordering::SeqCst), 0);
    let list = placed
        .compile(
            crate::Size::new(640.0, 360.0),
            crate::PixelSize::new(1280, 720),
            ValidationLimits::default(),
        )
        .unwrap();
    let DrawCommand::FillRect {
        bounds, transform, ..
    } = list.commands[0]
    else {
        panic!("callback")
    };
    assert_eq!(bounds, Rect::new(0.0, 0.0, 20.0, 20.0));
    assert_eq!(transform.apply(Point::ORIGIN), Point::new(180.0, 140.0));
    assert_eq!(content.bounds_calls.load(Ordering::SeqCst), 3);
    assert_eq!(content.draw_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn commands_share_node_budgets_even_at_zero_opacity_or_reused_leaves() {
    let leaf: Node = Rect::new(0.0, 0.0, 10.0, 10.0).fill(Color::WHITE).into();
    let group = Node::group([leaf.clone(), leaf]).opacity(0.0);
    let exact = ValidationLimits {
        max_nodes: 5,
        max_depth: 3,
    };
    assert!(group.compile_fixture(exact).is_ok());
    assert_eq!(
        group
            .compile_fixture(ValidationLimits {
                max_nodes: 4,
                ..exact
            })
            .unwrap_err()
            .field,
        "max_nodes"
    );
    assert_eq!(
        group
            .compile_fixture(ValidationLimits {
                max_depth: 2,
                ..exact
            })
            .unwrap_err()
            .field,
        "max_depth"
    );
    assert!(
        group
            .measure(ValidationLimits {
                max_nodes: 3,
                max_depth: 2
            })
            .is_ok()
    );
}

#[test]
fn color_and_geometry_narrow_only_at_the_output_boundary() {
    let color = Color::from_rgb_float(1.25, -0.125, 0.333).with_alpha(0.123);
    let node: Node = Rect::new(0.0, 0.0, 10.0, 20.0).fill(color).into();
    let list = node.compile_fixture(ValidationLimits::default()).unwrap();
    assert!(
        matches!(list.commands[0], DrawCommand::FillRect { paint: Paint::Solid(recorded), .. } if recorded == color)
    );
    let huge: Node = Rect::new(1.0e40, 0.0, 2.0e40, 1.0)
        .fill(color)
        .named("huge");
    assert!(huge.measure(ValidationLimits::default()).is_ok());
    assert!(huge.compile_fixture(ValidationLimits::default()).is_ok());
}

#[test]
fn multi_command_opacity_isolates_once_and_invalid_content_still_fails() {
    let content = IgnoredErrors {
        bounds: Rect::new(0.0, 0.0, 1.0, 1.0),
        commands: 2,
    };
    let node: Node = content.opacity(0.5);
    let list = node.compile_fixture(ValidationLimits::default()).unwrap();
    assert!(matches!(
        list.commands[0],
        DrawCommand::PushLayer { opacity: 0.5, .. }
    ));
    assert_eq!(
        list.commands
            .iter()
            .filter(|command| matches!(command, DrawCommand::FillRect { .. }))
            .count(),
        2
    );
    assert_eq!(
        list.commands
            .iter()
            .filter(|command| matches!(command, DrawCommand::PushLayer { .. }))
            .count(),
        1
    );
    let bad = IgnoredErrors {
        bounds: Rect::new(0.0, 0.0, f64::NAN, 1.0),
        commands: 1,
    }
    .opacity(0.0)
    .named("bad-command");
    let error = bad
        .compile_fixture(ValidationLimits::default())
        .unwrap_err();
    assert!(error.path.contains("name=bad-command"));
    assert_eq!(error.field, "bounds");
    let invalid = Rect::new(1.0, 0.0, 0.0, 1.0)
        .fill(Color::WHITE)
        .named("bad-bounds");
    assert_eq!(
        invalid
            .measure(ValidationLimits::default())
            .unwrap_err()
            .field,
        "bounds"
    );
}

#[test]
fn callback_failures_discard_partial_commands_and_keep_named_node_diagnostics() {
    struct Fails {
        poison: bool,
    }

    impl Drawable for Fails {
        fn bounds(&self, _: &DrawContext) -> Rect {
            Rect::new(0.0, 0.0, 1.0, 1.0)
        }

        fn draw(&self, _: &DrawContext, canvas: &mut Canvas<'_>) -> Result<(), ValidationError> {
            canvas.fill_rect(Rect::new(0.0, 0.0, 1.0, 1.0), Color::WHITE)?;
            if self.poison {
                let _ = canvas.fill_rect(Rect::new(1.0, 0.0, 0.0, 1.0), Color::WHITE);
            }
            Err(ValidationError::invalid(
                "custom",
                "callback",
                "Custom drawing failed",
            ))
        }
    }
    for poison in [false, true] {
        let error = DrawContext::default()
            .record(&Fails { poison }, ValidationLimits::default())
            .unwrap_err();
        assert_eq!(error.field, if poison { "bounds" } else { "callback" });
        let node = Fails { poison }.named("broken").opacity(0.0);
        let error = node
            .compile_fixture(ValidationLimits::default())
            .unwrap_err();
        assert_eq!(error.field, if poison { "bounds" } else { "callback" });
        assert!(error.path.contains("name=broken"));
    }
}
