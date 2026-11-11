//! Unit tests for planar perspective projection.

use super::*;
use crate::{Rect, ValidationErrorKind};

const PIVOT: Point = Point::new(100.0, 100.0);

fn close(first: Point, second: Point) -> bool {
    (first.x - second.x).abs() < 1e-9 && (first.y - second.y).abs() < 1e-9
}

#[test]
fn zero_rotation_is_the_identity_and_pivot_is_fixed() {
    let flat = Perspective::new(PIVOT, 0.0, 0.0, 0.0, 500.0).unwrap();
    let point = Point::new(3.0, -40.0);
    assert!(close(flat.project(point).unwrap(), point));
    let tilted = Perspective::new(PIVOT, 0.7, -0.4, 0.2, 500.0).unwrap();
    assert!(close(tilted.project(PIVOT).unwrap(), PIVOT));
}

#[test]
fn nearer_edges_grow_and_farther_edges_shrink() {
    let tilt = Perspective::new(PIVOT, 0.0, 0.5, 0.0, 800.0).unwrap();
    let near = tilt.project(Point::new(0.0, 100.0)).unwrap();
    let far = tilt.project(Point::new(200.0, 100.0)).unwrap();
    assert!(100.0 - near.x > far.x - 100.0);
    let bottom = Perspective::new(PIVOT, 0.5, 0.0, 0.0, 800.0).unwrap();
    let down = bottom.project(Point::new(100.0, 200.0)).unwrap();
    let up = bottom.project(Point::new(100.0, 0.0)).unwrap();
    assert!(down.y - 100.0 > 100.0 - up.y);
}

#[test]
fn z_rotation_matches_a_plane_rotation_and_facing_follows_the_angle() {
    let spin =
        Perspective::new(Point::ORIGIN, 0.0, 0.0, std::f64::consts::FRAC_PI_2, 100.0).unwrap();
    assert!(close(
        spin.project(Point::new(1.0, 0.0)).unwrap(),
        Point::new(0.0, 1.0)
    ));
    assert!(spin.faces_camera());
    let back = Perspective::new(PIVOT, 0.0, 3.0, 0.0, 100.0).unwrap();
    assert!(!back.faces_camera());
}

#[test]
fn points_reaching_the_camera_plane_fail() {
    let edge_on = Perspective::new(PIVOT, 0.0, std::f64::consts::FRAC_PI_2, 0.0, 100.0).unwrap();
    let error = edge_on.project(Point::new(-200.0, 100.0)).unwrap_err();
    assert_eq!(error.kind, ValidationErrorKind::InvalidValue);
    assert!(Perspective::new(PIVOT, 0.0, 0.0, 0.0, 0.0).is_err());
    assert!(Perspective::new(PIVOT, f64::NAN, 0.0, 0.0, 10.0).is_err());
}

#[test]
fn paths_keep_structure_and_curves_stay_close_to_the_exact_projection() {
    let tilt = Perspective::new(PIVOT, 0.3, 0.4, 0.0, 600.0).unwrap();
    let square = Path2D::from_rect(Rect::new(0.0, 0.0, 200.0, 200.0)).unwrap();
    let projected = tilt.project_path(&square, 4).unwrap();
    assert_eq!(projected.verbs().len(), square.verbs().len());
    let curve = Path2D::parse_svg("M0 0C50 100 150 100 200 0").unwrap();
    let pieces = tilt.project_path(&curve, 16).unwrap();
    assert_eq!(pieces.verbs().len(), 17);
    let exact = tilt.project(Point::new(100.0, 75.0)).unwrap();
    let nearest = (0..=200)
        .map(|index| pieces.point_at(f64::from(index) / 200.0).0)
        .map(|point| libm::hypot(point.x - exact.x, point.y - exact.y))
        .fold(f64::MAX, f64::min);
    assert!(nearest < 0.5, "{nearest}");
    assert!(tilt.project_path(&curve, 0).is_err());
}
