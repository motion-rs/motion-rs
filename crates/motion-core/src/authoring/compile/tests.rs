//! Unit tests for compiling authoring nodes into validated display lists.

use super::*;
use crate::ValidationErrorKind;
use crate::{
    Color, Drawable, FillRule, NodeModifiers, Paint, Path2D, Pivot, Point, Rect, Row, Shape,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn compile(node: &Node) -> Result<ValidatedDisplayList, ValidationError> {
    node.compile(
        Size::new(100.0, 100.0),
        PixelSize::new(100, 100),
        ValidationLimits::default(),
    )
}

fn tile(width: f64, height: f64) -> Node {
    Rect::new(0.0, 0.0, width, height).fill(Color::WHITE).into()
}

#[test]
fn nested_placement_composes_f64_matrices_and_root_contain_mapping() {
    let child = tile(10.0, 20.0)
        .pivot(Pivot::TopLeft)
        .scale_xy(2.0, 3.0)
        .rotate(std::f64::consts::FRAC_PI_2)
        .at(Point::new(0.1, 0.2));
    let node = Node::group([child])
        .pivot(Pivot::Point(Point::ORIGIN))
        .scale_xy(1.5, 0.5)
        .rotate(std::f64::consts::FRAC_PI_4)
        .translate(30.125, 40.25);
    let list = node
        .compile(
            Size::new(100.0, 50.0),
            PixelSize::new(200, 200),
            ValidationLimits::default(),
        )
        .unwrap();
    let [DrawCommand::FillRect { transform, .. }] = list.commands.as_slice() else {
        panic!("expected one global fill")
    };
    let child = Transform::scale_non_uniform(2.0, 3.0)
        .then(Transform::rotate(std::f64::consts::FRAC_PI_2))
        .then(Transform::translate(0.1, 0.2));
    let parent = Transform::scale_non_uniform(1.5, 0.5)
        .then(Transform::rotate(std::f64::consts::FRAC_PI_4))
        .then(Transform::translate(30.125, 40.25));
    let root = Transform::scale_non_uniform(2.0, 2.0).then(Transform::translate(0.0, 50.0));
    let expected = root.apply(parent.apply(child.apply(Point::new(2.0, 3.0))));
    let actual = transform.apply(Point::new(2.0, 3.0));
    assert!((actual.x - expected.x).abs() < 1.0e-12);
    assert!((actual.y - expected.y).abs() < 1.0e-12);
    assert_eq!(list.output_size, PixelSize::new(200, 200));
    assert_eq!(list.background, None);
    assert!(list.preflight(ValidationLimits::default()).is_ok());
}

#[test]
fn layout_keeps_insertion_slots_and_stable_paint_order() {
    let row = Row::new()
        .gap(3.0)
        .alignment(crate::Alignment::Center)
        .children([
            tile(10.0, 20.0).translate(500.0, -70.0).z_index(2),
            Node::empty(),
            tile(5.0, 10.0).z_index(-1),
            tile(7.0, 6.0).z_index(-1),
        ]);
    let list = compile(&row).unwrap();
    let points: Vec<_> = list
        .commands
        .iter()
        .map(|command| match command {
            DrawCommand::FillRect { transform, .. } => transform.apply(Point::ORIGIN),
            _ => panic!("expected fills"),
        })
        .collect();
    assert_eq!(
        points,
        vec![Point::new(13.0, 5.0), Point::new(21.0, 7.0), Point::ORIGIN]
    );
}

#[test]
fn group_opacity_isolates_once_and_preserves_extreme_alpha() {
    let group = Node::group([tile(10.0, 10.0), tile(10.0, 10.0).translate(5.0, 0.0)]).opacity(0.5);
    let list = compile(&group).unwrap();
    assert!(
        matches!(list.commands[0], DrawCommand::PushLayer { opacity: 0.5, bounds, .. } if bounds == Rect::new(0.0, 0.0, 15.0, 10.0))
    );
    assert_eq!(list.commands.len(), 4);
    assert!(matches!(list.commands[3], DrawCommand::PopLayer));
    let near_one = 1.0 - 2.0_f64.powi(-30);
    let tiny = f64::from_bits(1);
    let node = Node::group([tile(1.0, 1.0).opacity(tiny)]).opacity(near_one);
    let list = compile(&node).unwrap();
    let values: Vec<_> = list
        .commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::PushLayer { opacity, .. } => Some(*opacity),
            _ => None,
        })
        .collect();
    assert_eq!(values, vec![near_one, tiny]);
    assert!(node.measure(ValidationLimits::default()).is_ok());
}

#[test]
fn invalid_leaves_are_explicit_errors_even_when_hidden_or_degenerate() {
    for node in [
        Node::from(Rect::new(0.0, 0.0, f64::NAN, 1.0).fill(Color::WHITE)),
        Node::from(Rect::new(0.0, 0.0, 0.0, f64::INFINITY).fill(Color::WHITE)),
    ] {
        let error =
            compile(&node.opacity(0.0).translate(1000.0, 1000.0).named("hidden")).unwrap_err();
        assert_eq!(error.kind, ValidationErrorKind::InvalidValue);
        assert_eq!(error.field, "bounds");
        assert!(error.path.contains("name=hidden"));
    }
}

#[test]
fn paths_keep_both_rules_extended_paint_and_wide_coordinates() {
    let color = Color::from_rgb_float(1.25, -0.125, 0.333);
    let mut path = Path2D::new();
    path.try_move_to(Point::ORIGIN).unwrap();
    path.try_quadratic_to(Point::new(1.0e-100, 1.0), Point::new(2.0, 2.0))
        .unwrap();
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let node: Node = path.clone().fill(color).fill_rule(rule).into();
        let list = compile(&node).unwrap();
        let [
            DrawCommand::Fill {
                path: recorded,
                paint,
                rule: recorded_rule,
                ..
            },
        ] = list.commands.as_slice()
        else {
            panic!("expected path")
        };
        assert_eq!(recorded.verbs(), path.verbs());
        assert_eq!(*paint, Paint::Solid(color));
        assert_eq!(*recorded_rule, rule);
        assert!(node.measure(ValidationLimits::default()).is_ok());
    }
    let huge: Node = Rect::new(1.0e40, 0.0, 2.0e40, 1.0).fill(color).into();
    assert!(
        huge.compile(
            Size::new(2.0e40, 1.0),
            PixelSize::new(100, 100),
            ValidationLimits::default()
        )
        .is_ok()
    );
    assert!(huge.measure(ValidationLimits::default()).is_ok());
}

#[test]
fn callbacks_receive_actual_device_scale_exactly_once_per_occurrence() {
    struct Counted {
        bounds: AtomicUsize,
        draw: AtomicUsize,
    }

    impl Drawable for Counted {
        fn bounds(&self, context: &DrawContext) -> Rect {
            self.bounds.fetch_add(1, Ordering::SeqCst);
            assert_eq!(context.device_scale(), 2.0);
            Rect::new(0.0, 0.0, 10.0, 20.0)
        }

        fn draw(
            &self,
            context: &DrawContext,
            canvas: &mut Canvas<'_>,
        ) -> Result<(), ValidationError> {
            self.draw.fetch_add(1, Ordering::SeqCst);
            assert_eq!(context.device_scale(), 2.0);
            canvas.fill_rect(Rect::new(0.0, 0.0, 10.0, 20.0), Color::WHITE)
        }
    }
    let content = Arc::new(Counted {
        bounds: AtomicUsize::new(0),
        draw: AtomicUsize::new(0),
    });
    let node = Node::from(content.clone() as Arc<dyn Drawable>);
    node.compile(
        Size::new(100.0, 50.0),
        PixelSize::new(200, 200),
        ValidationLimits::default(),
    )
    .unwrap();
    assert_eq!(content.bounds.load(Ordering::SeqCst), 1);
    assert_eq!(content.draw.load(Ordering::SeqCst), 1);
    Node::group([node.clone(), node])
        .compile(
            Size::new(100.0, 50.0),
            PixelSize::new(200, 200),
            ValidationLimits::default(),
        )
        .unwrap();
    assert_eq!(content.bounds.load(Ordering::SeqCst), 3);
    assert_eq!(content.draw.load(Ordering::SeqCst), 3);
}

#[test]
fn source_structure_and_generated_layer_commands_have_independent_limits() {
    let node = Node::group([tile(1.0, 1.0).opacity(0.5)]).opacity(0.5);
    let exact = ValidationLimits {
        max_nodes: 5,
        max_depth: 3,
    };
    assert!(
        node.compile(Size::new(100.0, 100.0), PixelSize::new(100, 100), exact)
            .is_ok()
    );
    let error = node
        .compile(
            Size::new(100.0, 100.0),
            PixelSize::new(100, 100),
            ValidationLimits {
                max_nodes: 4,
                ..exact
            },
        )
        .unwrap_err();
    assert_eq!(error.field, "max_nodes");
    assert!(error.path.ends_with(".pop_layer"));
    let empty = Node::group([Node::empty()]).opacity(0.0);
    assert_eq!(
        empty
            .compile(
                Size::new(1.0, 1.0),
                PixelSize::new(1, 1),
                ValidationLimits {
                    max_nodes: 1,
                    max_depth: 2
                }
            )
            .unwrap_err()
            .field,
        "max_nodes"
    );
    assert!(compile(&empty).unwrap().commands.is_empty());
}

#[test]
fn shared_paths_charge_source_verbs_per_occurrence_before_output() {
    let path = Path2D::from_rect(Rect::new(0.0, 0.0, 1.0, 1.0)).unwrap();
    let child: Node = path.fill(Color::WHITE).into();
    let node = Node::group([child.clone(), child]).opacity(0.0);
    let exact = ValidationLimits {
        max_nodes: 15,
        max_depth: 3,
    };
    let list = node
        .compile(Size::new(100.0, 100.0), PixelSize::new(100, 100), exact)
        .unwrap();
    assert_eq!(list.preflight(exact).unwrap().nodes, 14);
    let error = node
        .compile(
            Size::new(100.0, 100.0),
            PixelSize::new(100, 100),
            ValidationLimits {
                max_nodes: 14,
                ..exact
            },
        )
        .unwrap_err();
    assert_eq!(error.field, "max_nodes");
    assert!(error.path.ends_with(".children[1].commands[0].verbs[4]"));
}

#[test]
fn transparent_commands_still_validate_actual_global_geometry_and_repeats() {
    struct BadBounds;

    impl Drawable for BadBounds {
        fn bounds(&self, _: &DrawContext) -> Rect {
            Rect::new(0.0, 0.0, 1.0, 1.0)
        }

        fn draw(&self, _: &DrawContext, canvas: &mut Canvas<'_>) -> Result<(), ValidationError> {
            canvas.fill_rect(Rect::new(0.0, 0.0, f64::MAX, 1.0), Color::WHITE)
        }
    }
    let error = compile(&BadBounds.scale(2.0).opacity(0.0).named("overflow")).unwrap_err();
    assert_eq!(error.field, "transform");
    assert!(error.path.contains("name=overflow"));
    let mut path = Path2D::new();
    path.try_move_to(Point::ORIGIN).unwrap();
    path.try_quadratic_to(Point::new(1.0e308, 1.0), Point::new(0.0, 2.0))
        .unwrap();
    let node: Node = path.fill(Color::WHITE).opacity(0.0).named("control");
    let error = node
        .compile(
            Size::new(100.0, 100.0),
            PixelSize::new(200, 200),
            ValidationLimits::default(),
        )
        .unwrap_err();
    assert_eq!(error.field, "coordinates");
    assert!(error.path.contains("name=control"));
    assert!(error.path.ends_with(".commands[0].verbs[1]"));
    let matrix = tile(1.0, 1.0).pivot(Pivot::TopLeft).scale_xy(1.0e308, 1.0);
    let error = matrix
        .compile(
            Size::new(100.0, 100.0),
            PixelSize::new(200, 200),
            ValidationLimits::default(),
        )
        .unwrap_err();
    assert_eq!(error.field, "transform");
    let node = Row::new()
        .gap(0.25)
        .children([tile(1.0, 2.0), tile(3.0, 4.0)])
        .rotate(0.125)
        .opacity(0.5);
    let first = compile(&node).unwrap();
    for size in [
        PixelSize::new(200, 100),
        PixelSize::new(100, 200),
        PixelSize::new(100, 100),
    ] {
        node.compile(Size::new(100.0, 100.0), size, ValidationLimits::default())
            .unwrap();
        assert_eq!(compile(&node).unwrap(), first);
    }
}
