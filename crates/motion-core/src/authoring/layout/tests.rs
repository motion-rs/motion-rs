//! Unit tests for layout stacks, alignment and measurement.

use super::*;
use crate::ValidationErrorKind;
use crate::{
    Color, DrawCommand, NodeModifiers, Pivot, PixelSize, Point, Rect, Size, ValidationLimits,
};

fn item(width: f64, height: f64) -> Node {
    Node::from(Rect::new(0.0, 0.0, width, height).fill(Color::WHITE))
}

#[test]
fn real_baselines_align_in_measurement_and_compilation() -> crate::Result {
    struct TextTile(f64, f64, f64);

    impl crate::Drawable for TextTile {
        fn bounds(&self, _: &crate::DrawContext) -> Rect {
            Rect::new(0.0, 0.0, self.0, self.1)
        }

        fn baseline(&self, _: &crate::DrawContext) -> Option<f64> {
            Some(self.2)
        }

        fn draw(
            &self,
            context: &crate::DrawContext,
            canvas: &mut crate::Canvas<'_>,
        ) -> Result<(), ValidationError> {
            canvas.fill(
                &self.bounds(context),
                crate::Color::WHITE,
                crate::FillRule::NonZero,
            )
        }
    }
    let row = Row::new().alignment(Alignment::Baseline).children([
        Node::from(TextTile(10.0, 10.0, 8.0)).translate(100.0, 50.0),
        Node::from(TextTile(5.0, 5.0, 3.0)).scale(2.0),
    ]);
    assert_eq!(
        row.measure(ValidationLimits::default())?,
        Rect::new(0.0, 0.0, 20.0, 12.0)
    );
    let list = row.compile(
        crate::Size::new(32.0, 32.0),
        crate::PixelSize::new(32, 32),
        ValidationLimits::default(),
    )?;
    let baselines: Vec<_> = list
        .commands
        .iter()
        .zip([8.0, 3.0])
        .map(|(command, baseline)| {
            let crate::DrawCommand::FillRect { transform, .. } = command else {
                panic!("rectangle");
            };
            transform.apply(Point::new(0.0, baseline)).y
        })
        .collect();
    assert_eq!(baselines, [8.0, 8.0]);
    for node in [
        Node::from(TextTile(1.0, 1.0, f64::NAN)),
        Node::from(TextTile(1.0, 1.0, 0.5)).rotate(0.2),
    ] {
        assert!(
            Row::new()
                .alignment(Alignment::Baseline)
                .children([node])
                .measure(ValidationLimits::default())
                .is_err()
        );
    }
    Ok(())
}

fn origins(node: &Node) -> Vec<(f64, f64)> {
    node.compile(
        Size::new(640.0, 360.0),
        PixelSize::new(640, 360),
        ValidationLimits::default(),
    )
    .unwrap()
    .commands
    .iter()
    .filter_map(|command| match command {
        DrawCommand::FillRect { transform, .. } => {
            let point = transform.apply(Point::ORIGIN);
            Some((point.x, point.y))
        }
        _ => None,
    })
    .collect()
}

#[test]
fn rows_align_transformed_bounds_with_finite_gaps() {
    for (alignment, first_y) in [
        (Alignment::Start, 0.0),
        (Alignment::Center, 10.0),
        (Alignment::End, 20.0),
    ] {
        let row = Row::new()
            .gap(5.0)
            .alignment(alignment)
            .children([item(20.0, 20.0), item(5.0, 40.0)]);
        assert_eq!(origins(&row), vec![(0.0, first_y), (25.0, 0.0)]);
        assert_eq!(
            row.measure(ValidationLimits::default()).unwrap(),
            Rect::new(0.0, 0.0, 30.0, 40.0)
        );
    }
    let shifted = Row::new().children([item(10.0, 20.0).translate(-8.0, 17.0)]);
    assert_eq!(origins(&shifted), vec![(0.0, 0.0)]);
}

#[test]
fn columns_skip_empty_nodes_but_keep_declared_degenerate_geometry() {
    for (alignment, first_x) in [
        (Alignment::Start, 0.0),
        (Alignment::Center, 10.0),
        (Alignment::End, 20.0),
    ] {
        let column = Column::new().gap(3.0).alignment(alignment).children([
            Some(item(20.0, 10.0)),
            None,
            Some(item(40.0, 5.0)),
        ]);
        assert_eq!(origins(&column), vec![(first_x, 0.0), (0.0, 13.0)]);
        assert_eq!(
            column.measure(ValidationLimits::default()).unwrap(),
            Rect::new(0.0, 0.0, 40.0, 18.0)
        );
    }
    let point = Column::new()
        .gap(4.0)
        .children([item(0.0, 0.0), item(2.0, 3.0)]);
    assert_eq!(
        point.measure(ValidationLimits::default()).unwrap(),
        Rect::new(0.0, 0.0, 2.0, 7.0)
    );
    let empty = Row::new().children(Vec::<Node>::new());
    assert_eq!(
        empty.measure(ValidationLimits::default()).unwrap(),
        Rect::default()
    );
    let overlap = Row::new()
        .gap(-10.0)
        .children([item(20.0, 10.0), item(5.0, 10.0)]);
    assert_eq!(origins(&overlap), vec![(0.0, 0.0), (10.0, 0.0)]);
    assert_eq!(
        overlap.measure(ValidationLimits::default()).unwrap().right,
        20.0
    );
}

#[test]
fn insertion_order_controls_layout_while_z_order_controls_painting() {
    let row = Row::new()
        .gap(5.0)
        .children([item(10.0, 10.0).z_index(2), item(20.0, 20.0).z_index(-1)]);
    assert_eq!(origins(&row), vec![(15.0, 0.0), (0.0, 0.0)]);
    let placed = row
        .pivot(Pivot::TopLeft)
        .at(Point::new(50.0, 60.0))
        .scale(2.0)
        .opacity(0.0);
    assert_eq!(
        placed.measure(ValidationLimits::default()).unwrap(),
        Rect::new(50.0, 60.0, 120.0, 100.0)
    );
    let scene = placed
        .compile(
            Size::new(640.0, 360.0),
            PixelSize::new(640, 360),
            ValidationLimits::default(),
        )
        .unwrap();
    assert!(matches!(
        scene.commands[0],
        DrawCommand::PushLayer { opacity: 0.0, .. }
    ));
}

#[test]
fn child_rotation_and_reflection_supply_arranged_aabb_sizes() {
    let rotated = item(10.0, 20.0)
        .pivot(Pivot::TopLeft)
        .rotate(std::f64::consts::FRAC_PI_2);
    let reflected = item(5.0, 30.0).pivot(Pivot::TopLeft).scale_xy(-2.0, 0.5);
    let row = Row::new()
        .gap(3.0)
        .alignment(Alignment::Center)
        .children([rotated, reflected]);
    assert_eq!(
        row.measure(ValidationLimits::default()).unwrap(),
        Rect::new(0.0, 0.0, 33.0, 15.0)
    );
    assert_eq!(origins(&row), vec![(20.0, 2.5), (33.0, 0.0)]);
    let equal_z = Row::new().children([item(10.0, 10.0).z_index(1), item(20.0, 20.0).z_index(1)]);
    assert_eq!(origins(&equal_z), vec![(0.0, 0.0), (10.0, 0.0)]);
}

#[test]
fn unsupported_and_unrepresentable_layouts_fail_without_silent_fallbacks() {
    let limits = ValidationLimits::default();
    for gap in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            Row::new()
                .gap(gap)
                .children([item(1.0, 1.0)])
                .measure(limits)
                .unwrap_err()
                .field,
            "gap"
        );
    }
    for node in [
        Row::new()
            .alignment(Alignment::Baseline)
            .children([item(1.0, 1.0)]),
        Column::new()
            .alignment(Alignment::Baseline)
            .children([item(1.0, 1.0)]),
    ] {
        assert_eq!(
            node.measure(limits).unwrap_err().kind,
            ValidationErrorKind::Unsupported
        );
        assert!(
            node.opacity(0.0)
                .compile(Size::new(8.0, 8.0), PixelSize::new(8, 8), limits)
                .is_err()
        );
    }
    let wide = Node::from(Rect::new(-f64::MAX, 0.0, f64::MAX, 1.0).fill(Color::WHITE));
    assert_eq!(
        Row::new()
            .children([wide])
            .measure(limits)
            .unwrap_err()
            .field,
        "bounds"
    );
    let overflow =
        Row::new()
            .gap(f64::MAX)
            .children([item(1.0, 1.0), item(1.0, 1.0), item(1.0, 1.0)]);
    assert!(overflow.measure(limits).is_err());
}

#[test]
fn nested_layout_uses_one_structure_budget_and_preserves_input_snapshots() {
    let child = item(10.0, 10.0).translate(3.0, 7.0);
    let nested = Column::new().gap(2.0).children([
        Row::new()
            .gap(1.0)
            .children([child.clone(), item(20.0, 5.0)]),
        Row::new().children([item(4.0, 8.0)]),
    ]);
    assert_eq!(
        nested.measure(ValidationLimits::default()).unwrap(),
        Rect::new(0.0, 0.0, 31.0, 20.0)
    );
    assert_eq!(
        child.measure(ValidationLimits::default()).unwrap(),
        Rect::new(3.0, 7.0, 13.0, 17.0)
    );
    let exact = ValidationLimits {
        max_nodes: 6,
        max_depth: 3,
    };
    assert!(nested.measure(exact).is_ok());
    let draw_limits = ValidationLimits {
        max_nodes: 9,
        max_depth: 4,
    };
    assert!(
        nested
            .compile(Size::new(64.0, 64.0), PixelSize::new(64, 64), draw_limits)
            .is_ok()
    );
    assert!(
        nested
            .compile(
                Size::new(64.0, 64.0),
                PixelSize::new(64, 64),
                ValidationLimits {
                    max_nodes: 8,
                    ..draw_limits
                }
            )
            .is_err()
    );
    assert_eq!(
        nested
            .measure(ValidationLimits {
                max_nodes: 5,
                ..exact
            })
            .unwrap_err()
            .field,
        "max_nodes"
    );
    assert_eq!(
        nested
            .measure(ValidationLimits {
                max_depth: 2,
                ..exact
            })
            .unwrap_err()
            .field,
        "max_depth"
    );
}
