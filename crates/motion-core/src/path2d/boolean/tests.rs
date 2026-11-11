//! Unit tests for path boolean operations.

use super::*;
use crate::{Rect, ValidationErrorKind, Vector2};

fn square(left: f64, top: f64, right: f64, bottom: f64) -> Path2D {
    Path2D::from_rect(Rect::new(left, top, right, bottom)).unwrap()
}

fn run(first: &Path2D, second: &Path2D, op: PathBooleanOp) -> Path2D {
    first
        .boolean(second, op, PathBooleanOptions::default())
        .unwrap()
}

fn moves(path: &Path2D) -> usize {
    path.verbs()
        .iter()
        .filter(|verb| matches!(verb, PathVerb::MoveTo(_)))
        .count()
}

// Net signed area of all contours: holes wind opposite to their parents.
fn area(path: &Path2D) -> f64 {
    let mut total = 0.0;
    let mut ring: Vec<Point> = Vec::new();
    for verb in path.verbs() {
        match *verb {
            PathVerb::MoveTo(point) | PathVerb::LineTo(point) => ring.push(point),
            PathVerb::Close => {
                for (index, start) in ring.iter().enumerate() {
                    let end = ring[(index + 1) % ring.len()];
                    total += (start.x * end.y - end.x * start.y) * 0.5;
                }
                ring.clear();
            }
            _ => panic!("boolean results are line-only"),
        }
    }
    total.abs()
}

#[test]
fn overlapping_squares_cover_the_expected_areas() {
    let first = square(0.0, 0.0, 10.0, 10.0);
    let second = square(5.0, 5.0, 15.0, 15.0);
    let expected = [
        (PathBooleanOp::Union, 175.0),
        (PathBooleanOp::Intersection, 25.0),
        (PathBooleanOp::Difference, 75.0),
        (PathBooleanOp::Xor, 150.0),
    ];
    for (op, value) in expected {
        let result = run(&first, &second, op);
        assert!((area(&result) - value).abs() < 1e-9, "{op:?}");
    }
    let overlap = run(&first, &second, PathBooleanOp::Intersection);
    assert_eq!(overlap.bounding_box(), Rect::new(5.0, 5.0, 10.0, 10.0));
}

#[test]
fn difference_with_an_inner_square_leaves_a_hole() {
    let outer = square(0.0, 0.0, 10.0, 10.0);
    let inner = square(3.0, 3.0, 6.0, 6.0);
    let ring = run(&outer, &inner, PathBooleanOp::Difference);
    assert_eq!(moves(&ring), 2);
    assert!((area(&ring) - 91.0).abs() < 1e-9);
    let union = run(&outer, &inner, PathBooleanOp::Union);
    assert!((area(&union) - 100.0).abs() < 1e-9);
}

#[test]
fn shared_and_collinear_edges_merge_without_slivers() {
    let first = square(0.0, 0.0, 10.0, 10.0);
    let second = square(10.0, 0.0, 20.0, 10.0);
    let union = run(&first, &second, PathBooleanOp::Union);
    assert_eq!(union.bounding_box(), Rect::new(0.0, 0.0, 20.0, 10.0));
    assert_eq!(union.verbs().len(), 5);
    assert!(
        run(&first, &second, PathBooleanOp::Intersection)
            .verbs()
            .is_empty()
    );
    assert!(run(&first, &first, PathBooleanOp::Xor).verbs().is_empty());
}

#[test]
fn curves_flatten_and_results_replay_exactly() {
    let circle = Path2D::builder()
        .arc_to(
            Point::new(10.0, 10.0),
            Vector2::new(8.0, 8.0),
            0.0,
            std::f64::consts::TAU,
        )
        .close()
        .build()
        .unwrap();
    let cover = square(10.0, 0.0, 30.0, 20.0);
    let half = run(&circle, &cover, PathBooleanOp::Intersection);
    let expected = std::f64::consts::PI * 64.0 * 0.5;
    assert!((area(&half) - expected).abs() < 1.0);
    assert_eq!(half, run(&circle, &cover, PathBooleanOp::Intersection));
}

#[test]
fn fill_rule_decides_whether_nested_same_direction_contours_are_holes() {
    let nested = Path2D::parse_svg("M0 0H10V10H0Z M3 3H7V7H3Z").unwrap();
    let far = square(100.0, 100.0, 101.0, 101.0);
    let even_odd = PathBooleanOptions {
        fill_rule: FillRule::EvenOdd,
        ..PathBooleanOptions::default()
    };
    let holed = nested
        .boolean(&far, PathBooleanOp::Difference, even_odd)
        .unwrap();
    assert!((area(&holed) - 84.0).abs() < 1e-9);
    let solid = run(&nested, &far, PathBooleanOp::Difference);
    assert!((area(&solid) - 100.0).abs() < 1e-9);
}

#[test]
fn crossing_self_intersection_is_split() {
    let bowtie = Path2D::parse_svg("M0 0L10 10H0L10 0Z").unwrap();
    let far = square(100.0, 100.0, 101.0, 101.0);
    let result = run(&bowtie, &far, PathBooleanOp::Difference);
    assert_eq!(moves(&result), 2);
}

#[test]
fn invalid_options_and_limits_are_rejected() {
    let first = square(0.0, 0.0, 1.0, 1.0);
    let bad = PathBooleanOptions {
        tolerance: 0.0,
        ..PathBooleanOptions::default()
    };
    assert!(first.boolean(&first, PathBooleanOp::Union, bad).is_err());
    let starved = PathBooleanOptions {
        limits: RasterLimits {
            max_work: 4,
            ..RasterLimits::default()
        },
        ..PathBooleanOptions::default()
    };
    let error = first
        .boolean(&first, PathBooleanOp::Union, starved)
        .unwrap_err();
    assert_eq!(error.kind, ValidationErrorKind::StructuralLimit);
}
