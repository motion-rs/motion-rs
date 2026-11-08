//! Unit tests for path construction, editing, bounds and stroke caching.

use super::*;
use crate::ValidationErrorKind;

#[test]
fn shared_fingerprints_and_verbs_keep_copy_on_write_atomicity() {
    let mut path = Path2D::from_rect(Rect::new(1.0, 2.0, 7.0, 8.0)).unwrap();
    let hash = path.fingerprint();
    let snapshot = path.clone();
    let verbs = path.shared_verbs();
    assert!(Arc::ptr_eq(&verbs, &snapshot.shared_verbs()));
    assert_eq!(snapshot.fingerprint(), hash);
    assert_eq!(
        path.axis_aligned_rectangle(),
        Some(Rect::new(1.0, 2.0, 7.0, 8.0))
    );
    assert!(path.try_line_to(Point::new(f64::NAN, 1.0)).is_err());
    assert_eq!(path.fingerprint(), hash);
    path.try_line_to(Point::new(3.0, 4.0)).unwrap();
    assert_ne!(path.fingerprint(), hash);
    assert_eq!(snapshot.fingerprint(), hash);
    assert_eq!(verbs.as_slice(), snapshot.verbs());
    assert!(!Arc::ptr_eq(&verbs, &path.shared_verbs()));
    assert!(path.axis_aligned_rectangle().is_none());
}

#[test]
fn dashes_preserve_distance_phase_contours_and_closed_seams() {
    let line = Path2D::parse_svg("M0 0H10 M20 0H30").unwrap();
    let dashed = line.dashed(&[2.0, 2.0], 0.0).unwrap();
    assert!((dashed.length() - 12.0).abs() < 1e-8);
    assert_eq!(
        dashed
            .verbs()
            .iter()
            .filter(|verb| matches!(verb, PathVerb::MoveTo(_)))
            .count(),
        6
    );
    assert_eq!(
        line.dashed(&[2.0, 2.0], -1.0).unwrap(),
        line.dashed(&[2.0, 2.0], 3.0).unwrap()
    );
    assert_eq!(line.dashed(&[2.0], 0.0).unwrap(), dashed);
    assert_eq!(line.dashed(&[], 3.0).unwrap(), line);
    let rectangle = Path2D::from_rect(Rect::new(1.0, 1.0, 7.0, 7.0)).unwrap();
    let dashed = rectangle.dashed(&[9.0, 2.0], 0.0).unwrap();
    assert_eq!(
        dashed
            .verbs()
            .iter()
            .filter(|verb| matches!(verb, PathVerb::MoveTo(_)))
            .count(),
        2
    );
    let mut circle = Path2D::new();
    circle
        .try_arc_to(
            Point::ORIGIN,
            Vector2::new(3.0, 3.0),
            0.0,
            std::f64::consts::TAU,
        )
        .unwrap();
    circle.try_close().unwrap();
    assert!(matches!(
        circle.dashed(&[100.0, 1.0], 0.0).unwrap().verbs().last(),
        Some(PathVerb::Close)
    ));
    for pattern in [
        vec![0.0, 1.0],
        vec![-1.0],
        vec![f64::NAN],
        vec![1.0; 257],
        vec![1e-100, 1e-100],
    ] {
        assert!(line.dashed(&pattern, 0.0).is_err());
    }
}

#[test]
fn arc_lengths_sampling_and_exact_curve_trims_have_independent_references() {
    let quadratic = Path2D::parse_svg("M0 0Q1 1 2 0").unwrap();
    assert!((quadratic.length() - libm::sqrt(2.0) - libm::asinh(1.0)).abs() < 1e-10);
    assert!(quadratic.point_at(0.5).0.distance(Point::new(1.0, 0.5)) < 1e-9);
    let mut path = Path2D::new();
    path.try_move_to(Point::ORIGIN).unwrap();
    path.try_cubic_to(
        Point::new(0.0, 10.0),
        Point::new(10.0, 10.0),
        Point::new(10.0, 0.0),
    )
    .unwrap();
    assert!((path.length() - 20.0).abs() < 1e-10);
    let (position, tangent) = path.point_at(0.5);
    assert!(position.distance(Point::new(5.0, 7.5)) < 1e-9);
    assert!((tangent - Vector2::new(1.0, 0.0)).length() < 1e-9);
    let trimmed = path.trim(0.25, 0.75);
    assert!(matches!(trimmed.verbs()[1], PathVerb::CubicTo(..)));
    assert!((trimmed.length() - 10.0).abs() < 1e-8);
    assert!(trimmed.point_at(0.0).0.distance(path.point_at(0.25).0) < 1e-8);
    assert!(trimmed.point_at(1.0).0.distance(path.point_at(0.75).0) < 1e-8);
    assert_eq!(path.trim(0.0, 1.0), path);
    let retained = path.clone();
    path.try_line_to(Point::new(20.0, 0.0)).unwrap();
    assert!((path.length() - 30.0).abs() < 1e-9);
    assert!((retained.length() - 20.0).abs() < 1e-9);
    let mut circle = Path2D::new();
    circle
        .try_arc_to(
            Point::ORIGIN,
            Vector2::new(3.0, 3.0),
            0.0,
            std::f64::consts::TAU,
        )
        .unwrap();
    assert!((circle.length() - 6.0 * std::f64::consts::PI).abs() < 1e-6);
    assert_eq!(circle.data.current, Some(Point::new(3.0, 0.0)));
}

#[test]
fn gaussian_lengths_and_partial_curves_match_dense_independent_references() {
    let mut random = crate::Random::from_key("path-metrics-reference");
    for _ in 0..24 {
        let controls = std::array::from_fn::<_, 4, _>(|_| {
            Point::new(random.range(-10.0..10.0), random.range(-10.0..10.0))
        });
        let mut path = Path2D::new();
        path.try_move_to(controls[0]).unwrap();
        path.try_cubic_to(controls[1], controls[2], controls[3])
            .unwrap();
        let mut previous = controls[0];
        let mut reference = 0.0;
        for index in 1..=8192 {
            let progress = index as f64 / 8192.0;
            let remaining = 1.0 - progress;
            let position = Point::new(
                controls[0].x * remaining * remaining * remaining
                    + 3.0 * controls[1].x * remaining * remaining * progress
                    + 3.0 * controls[2].x * remaining * progress * progress
                    + controls[3].x * progress * progress * progress,
                controls[0].y * remaining * remaining * remaining
                    + 3.0 * controls[1].y * remaining * remaining * progress
                    + 3.0 * controls[2].y * remaining * progress * progress
                    + controls[3].y * progress * progress * progress,
            );
            reference += position.distance(previous);
            previous = position;
        }
        let length = path.length();
        assert!((length - reference).abs() <= length * 1e-6);
        assert!((path.trim(0.17, 0.83).length() - length * 0.66).abs() <= length * 1e-8);
    }
}

#[test]
fn metric_degeneracies_cusps_and_contours_remain_explicit() {
    let tiny = f64::from_bits(2);
    let mut tiny_line = Path2D::new();
    tiny_line.try_move_to(Point::ORIGIN).unwrap();
    tiny_line.try_line_to(Point::new(tiny, 0.0)).unwrap();
    assert_eq!(tiny_line.length(), tiny);
    assert_eq!(
        tiny_line.point_at(0.5),
        (Point::new(f64::from_bits(1), 0.0), Vector2::new(1.0, 0.0))
    );
    assert_eq!(tiny_line.trim(0.0, 0.5).length(), f64::from_bits(1));
    let mut tiny_curve = Path2D::new();
    tiny_curve.try_move_to(Point::ORIGIN).unwrap();
    tiny_curve
        .try_quadratic_to(Point::new(tiny, 0.0), Point::ORIGIN)
        .unwrap();
    assert!(tiny_curve.try_length().is_err());
    assert_eq!(Path2D::new().point_at(0.5), (Point::ORIGIN, Vector2::ZERO));
    let mut cusp = Path2D::new();
    cusp.try_move_to(Point::ORIGIN).unwrap();
    cusp.try_cubic_to(Point::new(10.0, 0.0), Point::new(-10.0, 0.0), Point::ORIGIN)
        .unwrap();
    assert!((cusp.length() - 20.0 / libm::sqrt(3.0)).abs() < 1e-7);
    assert!(cusp.point_at(0.25).0.x.is_finite());
    let mut contours = Path2D::from_rect(Rect::new(0.0, 0.0, 10.0, 10.0)).unwrap();
    contours.try_move_to(Point::new(100.0, 0.0)).unwrap();
    contours.try_line_to(Point::new(110.0, 0.0)).unwrap();
    assert!((contours.length() - 50.0).abs() < 1e-10);
    let trimmed = contours.trim(0.2, 0.9);
    assert_eq!(
        trimmed
            .verbs()
            .iter()
            .filter(|verb| matches!(verb, PathVerb::MoveTo(_)))
            .count(),
        2
    );
    assert!((trimmed.length() - 35.0).abs() < 1e-8);
    assert!(contours.trim(0.8, 0.2).verbs().is_empty());
    assert!(contours.try_trim(f64::NAN, 1.0).is_err());
    assert!(contours.try_point_at(f64::INFINITY).is_err());
    assert!(
        Path2D::parse_svg("M0 0H1e300 M0 0H1")
            .unwrap()
            .try_length()
            .is_err()
    );
    assert!(Path2D::parse_svg("M0 0A1e309 1 0 0 1 0 0").is_err());
}

#[test]
fn arcs_preserve_quadrants_snapshots_and_atomic_failures() {
    let mut path = Path2D::new();
    path.try_arc_to(
        Point::new(10.0, 20.0),
        Vector2::new(5.0, 3.0),
        0.0,
        std::f64::consts::FRAC_PI_2,
    )
    .unwrap();
    assert_eq!(path.verbs().len(), 5);
    let bounds = path.bounding_box();
    for (actual, expected) in [
        (bounds.left, 10.0),
        (bounds.top, 20.0),
        (bounds.right, 15.0),
        (bounds.bottom, 23.0),
    ] {
        assert!((actual - expected).abs() < 1e-12);
    }
    let retained = path.clone();
    for sweep in [f64::NAN, f64::INFINITY, f64::MAX] {
        assert!(
            path.try_arc_to(Point::ORIGIN, Vector2::new(1.0, 1.0), 0.0, sweep)
                .is_err()
        );
        assert_eq!(path, retained);
    }
    path.try_arc_to(
        Point::new(10.0, 20.0),
        Vector2::new(5.0, 3.0),
        std::f64::consts::FRAC_PI_2,
        -std::f64::consts::FRAC_PI_2,
    )
    .unwrap();
    assert_eq!(retained.verbs().len(), 5);
    assert_eq!(path.data.current, Some(Point::new(15.0, 20.0)));
}

#[test]
fn svg_relative_lines_normalize_to_absolute_verbs() {
    let parsed = Path2D::parse_svg("M2 3h10v20H2z m1 2 3 4").unwrap();
    let mut expected = Path2D::from_rect(Rect::new(2.0, 3.0, 12.0, 23.0)).unwrap();
    expected.try_move_to(Point::new(3.0, 5.0)).unwrap();
    expected.try_line_to(Point::new(6.0, 9.0)).unwrap();
    assert_eq!(parsed, expected);
    assert_eq!(parsed.bounding_box(), expected.bounding_box());
}

#[test]
fn svg_smooth_controls_reflect_only_the_matching_preceding_curve() {
    let expected = [
        PathVerb::MoveTo(Point::new(1.0, 1.0)),
        PathVerb::CubicTo(
            Point::new(2.0, 3.0),
            Point::new(3.0, 3.0),
            Point::new(4.0, 1.0),
        ),
        PathVerb::CubicTo(
            Point::new(5.0, -1.0),
            Point::new(6.0, -1.0),
            Point::new(7.0, 1.0),
        ),
        PathVerb::QuadraticTo(Point::new(8.0, 3.0), Point::new(9.0, 1.0)),
        PathVerb::QuadraticTo(Point::new(10.0, -1.0), Point::new(11.0, 1.0)),
        PathVerb::LineTo(Point::new(12.0, 1.0)),
        PathVerb::CubicTo(
            Point::new(12.0, 1.0),
            Point::new(13.0, 2.0),
            Point::new(14.0, 1.0),
        ),
        PathVerb::QuadraticTo(Point::new(14.0, 1.0), Point::new(16.0, 1.0)),
        PathVerb::MoveTo(Point::new(17.0, 2.0)),
        PathVerb::QuadraticTo(Point::new(17.0, 2.0), Point::new(18.0, 3.0)),
        PathVerb::Close,
        PathVerb::QuadraticTo(Point::new(17.0, 2.0), Point::new(18.0, 3.0)),
    ];
    for data in [
        "M1 1 c1 2 2 2 3 0 s2 -2 3 0 q1 2 2 0 t2 0 l1 0 s1 1 2 0 t2 0 m1 1t1 1z t1 1",
        "M1 1 C2 3 3 3 4 1 S6 -1 7 1 Q8 3 9 1 T11 1 L12 1 S13 2 14 1 T16 1 M17 2 T18 3 Z T18 3",
    ] {
        assert_eq!(Path2D::parse_svg(data).unwrap().verbs(), &expected);
    }
}

#[test]
fn svg_compact_numbers_empty_input_and_shared_triangle_preserve_geometry() {
    let parsed = Path2D::parse_svg("m1e1,-.5 2.5.5h-2.5v+2.5l-1e1-2.5z").unwrap();
    assert_eq!(
        parsed.verbs(),
        &[
            PathVerb::MoveTo(Point::new(10.0, -0.5)),
            PathVerb::LineTo(Point::new(12.5, 0.0)),
            PathVerb::LineTo(Point::new(10.0, 0.0)),
            PathVerb::LineTo(Point::new(10.0, 2.5)),
            PathVerb::LineTo(Point::ORIGIN),
            PathVerb::Close,
        ]
    );
    for data in ["", " ", "\t\r\n"] {
        assert_eq!(Path2D::parse_svg(data).unwrap(), Path2D::new());
    }
    let triangle = Path2D::parse_svg("M470 80l100 170H410z").unwrap();
    let mut manual = Path2D::new();
    manual.try_move_to(Point::new(470.0, 80.0)).unwrap();
    manual.try_line_to(Point::new(570.0, 250.0)).unwrap();
    manual.try_line_to(Point::new(410.0, 250.0)).unwrap();
    manual.try_close().unwrap();
    assert_eq!(triangle, manual);
}

#[test]
fn svg_rejects_bad_syntax_and_accepts_arcs_without_partial_recovery() {
    for data in [
        "L0 0",
        "Z",
        "M",
        "M0",
        "M0 0 L1",
        "M0 0 X1 1",
        "M0 0z 1 1",
        "M0 0 L1 1 garbage",
        "M0 0 LNaN 1",
        "M0 0 LInfinity 0",
        "M0 0 L1e309 0",
        "<svg><path d='M0 0'/></svg>",
        "M0 0\0",
        "M0 0\u{2003}",
    ] {
        let error = Path2D::parse_svg(data).unwrap_err();
        assert_eq!(error.kind, ValidationErrorKind::InvalidValue, "{data:?}");
        assert_eq!(error.field, "data");
        assert!(error.path.starts_with("path2d.svg.commands["));
    }
    let error = Path2D::parse_svg("M0 0 L1 1 Q2").unwrap_err();
    assert_eq!(error.path, "path2d.svg.commands[2]");
    for data in ["M0 0 A2 3 45 0 1 4 5", "M0 0 a0 0 0 0 0 0 0"] {
        assert!(Path2D::parse_svg(data).is_ok());
    }
    let arc = Path2D::parse_svg("M10 0 A10 10 0 0 1 0 10 S-5 20 -10 10").unwrap();
    assert!(
        matches!(arc.verbs().last(), Some(PathVerb::CubicTo(first, _, _)) if *first == Point::new(0.0, 10.0))
    );
    let quarter = Path2D::parse_svg("M10 0 a10 10 0 0 1 -10 10").unwrap();
    assert!((quarter.length() - 5.0 * std::f64::consts::PI).abs() < 1e-6);
    assert_eq!(quarter.data.current, Some(Point::new(0.0, 10.0)));
    assert!(Path2D::parse_svg("M0 0 A1 1 0 2 0 1 1").is_err());
    assert!(Path2D::parse_svg("M0 0L1 1").is_ok());
}

#[test]
fn svg_numeric_validation_keeps_range_and_checks_derived_coordinates() {
    for (data, field) in [
        ("M1e308 0 l1e308 0", "coordinates"),
        ("M-1e308 0 L1e308 0", "bounds"),
        ("M0 0 C0 0 0 0 1e308 0 S0 0 0 0", "coordinates"),
    ] {
        let error = Path2D::parse_svg(data).unwrap_err();
        assert_eq!(error.kind, ValidationErrorKind::InvalidValue);
        assert_eq!(error.field, field);
    }
    let huge = Path2D::parse_svg("M1e308 0 C1e308 0 1e308 0 1e308 0 S1e308 0 1e308 0").unwrap();
    let PathVerb::CubicTo(first, second, end) = huge.verbs()[2] else {
        panic!("expected cubic")
    };
    assert!(first.x.is_finite());
    assert_eq!((first, second), (end, end));
    let tiny = Path2D::parse_svg("M0 0 Q1e-100 0 1 1").unwrap();
    let PathVerb::QuadraticTo(control, _) = tiny.verbs()[1] else {
        panic!("expected quadratic")
    };
    assert!(control.x > 0.0 && control.x < f64::from(f32::MIN_POSITIVE));
    assert_eq!(control.x, 1e-100);
    assert!(tiny.bounding_box().is_finite());
    let wide = Path2D::parse_svg("M0 0 Q1e40 0 1 1").unwrap();
    let PathVerb::QuadraticTo(control, _) = wide.verbs()[1] else {
        panic!("expected quadratic")
    };
    assert!(control.x > f64::from(f32::MAX));
}

#[test]
fn svg_byte_and_command_caps_are_exact_and_close_runs_are_iterative() {
    let mut padded = String::from("M0 0");
    padded.push_str(&" ".repeat(MAX_SVG_PATH_BYTES - padded.len()));
    assert_eq!(Path2D::parse_svg(&padded).unwrap().verbs().len(), 1);
    padded.push(' ');
    let error = Path2D::parse_svg(&padded).unwrap_err();
    assert_eq!(
        (error.kind, error.field.as_str()),
        (ValidationErrorKind::StructuralLimit, "svg_bytes")
    );
    let mut repeated = String::from("M0 0");
    repeated.push_str(&"z".repeat(MAX_PATH_VERBS - 1));
    let retained = Path2D::parse_svg(&repeated).unwrap();
    assert_eq!(retained.verbs().len(), MAX_PATH_VERBS);
    repeated.push('z');
    let error = Path2D::parse_svg(&repeated).unwrap_err();
    assert_eq!(
        (error.kind, error.field.as_str()),
        (ValidationErrorKind::StructuralLimit, "commands")
    );
    assert_eq!(retained.verbs().len(), MAX_PATH_VERBS);
    assert!(Path2D::parse_svg("M0 0z").is_ok());
}

#[test]
fn curve_bounds_use_extrema_instead_of_control_hulls() {
    let mut quad = Path2D::new();
    quad.try_move_to(Point::ORIGIN).unwrap();
    quad.try_quadratic_to(Point::new(50.0, 100.0), Point::new(100.0, 0.0))
        .unwrap();
    assert_eq!(quad.bounding_box(), Rect::new(0.0, 0.0, 100.0, 50.0));
    let mut cubic = Path2D::new();
    cubic.try_move_to(Point::ORIGIN).unwrap();
    cubic
        .try_cubic_to(
            Point::new(0.0, 100.0),
            Point::new(100.0, 100.0),
            Point::new(100.0, 0.0),
        )
        .unwrap();
    assert_eq!(cubic.bounding_box(), Rect::new(0.0, 0.0, 100.0, 75.0));
}

#[test]
fn closed_rectangles_have_exact_verbs_and_checked_source_bounds() {
    let bounds = Rect::new(2.0, 3.0, 12.0, 23.0);
    let path = Path2D::from_rect(bounds).unwrap();
    assert_eq!(path.verbs().len(), 5);
    assert_eq!(path.bounding_box(), bounds);
    assert_eq!(path.verbs().last(), Some(&PathVerb::Close));
    assert!(Path2D::from_rect(Rect::new(1.0, 0.0, 0.0, 1.0)).is_err());
}

#[test]
fn snapshots_share_storage_and_mutations_invalidate_only_the_new_cache() {
    let mut path = Path2D::new();
    path.try_move_to(Point::new(10.0, 20.0)).unwrap();
    path.try_line_to(Point::new(30.0, 40.0)).unwrap();
    let before = path.bounding_box();
    let old = path.clone();
    assert!(Arc::ptr_eq(&path.data, &old.data));
    path.try_close().unwrap();
    path.try_line_to(Point::new(50.0, 60.0)).unwrap();
    assert!(!Arc::ptr_eq(&path.data, &old.data));
    assert_eq!(old.bounding_box(), before);
    assert_eq!(path.bounding_box(), Rect::new(10.0, 20.0, 50.0, 60.0));
}

#[test]
fn invalid_mutations_are_atomic_and_verb_storage_is_bounded() {
    let mut path = Path2D::new();
    assert!(path.try_close().is_err());
    assert!(path.try_line_to(Point::ORIGIN).is_err());
    assert!(path.try_move_to(Point::new(f64::NAN, 1.0)).is_err());
    assert!(path.verbs().is_empty());
    path.try_move_to(Point::new(-f64::MAX, 0.0)).unwrap();
    assert!(path.try_line_to(Point::new(f64::MAX, 0.0)).is_err());
    assert_eq!(path.verbs().len(), 1);
    let mut path = Path2D::new();
    path.try_move_to(Point::ORIGIN).unwrap();
    assert!(
        path.try_quadratic_to(Point::new(f64::INFINITY, 0.0), Point::ORIGIN)
            .is_err()
    );
    assert!(
        path.try_cubic_to(Point::ORIGIN, Point::new(0.0, f64::NAN), Point::ORIGIN)
            .is_err()
    );
    assert_eq!(path.verbs().len(), 1);
    for _ in 1..MAX_PATH_VERBS {
        path.try_line_to(Point::ORIGIN).unwrap();
    }
    assert_eq!(
        path.try_close().unwrap_err().kind,
        ValidationErrorKind::StructuralLimit
    );
    assert_eq!(path.verbs().len(), MAX_PATH_VERBS);
}

#[test]
fn affine_mapping_preserves_curves_and_original_geometry() {
    let mut path = Path2D::new();
    path.try_move_to(Point::ORIGIN).unwrap();
    path.try_quadratic_to(Point::new(5.0, 20.0), Point::new(10.0, 0.0))
        .unwrap();
    let mapped = path
        .transformed(&Transform::scale_non_uniform(2.0, -1.0).then(Transform::translate(3.0, 4.0)))
        .unwrap();
    assert_eq!(mapped.bounding_box(), Rect::new(3.0, -6.0, 23.0, 4.0));
    assert!(matches!(mapped.verbs()[1], PathVerb::QuadraticTo(_, _)));
    assert!(
        path.transformed(&Transform::scale_non_uniform(f64::INFINITY, 1.0))
            .is_err()
    );
    assert!(
        path.transformed(&Transform::scale_non_uniform(f64::MAX, 1.0))
            .is_err()
    );
    assert_eq!(path.bounding_box(), Rect::new(0.0, 0.0, 10.0, 10.0));
}

#[test]
fn empty_point_and_extreme_curves_keep_finite_bounds() {
    assert_eq!(Path2D::new().bounding_box(), Rect::default());
    let mut point = Path2D::new();
    point.try_move_to(Point::new(9.0, -7.0)).unwrap();
    assert_eq!(point.bounding_box(), Rect::new(9.0, -7.0, 9.0, -7.0));
    let mut huge = Path2D::new();
    huge.try_move_to(Point::ORIGIN).unwrap();
    huge.try_quadratic_to(Point::new(0.0, 1.0e300), Point::new(1.0e300, 0.0))
        .unwrap();
    assert_eq!(huge.bounding_box(), Rect::new(0.0, 0.0, 1.0e300, 5.0e299));
}
