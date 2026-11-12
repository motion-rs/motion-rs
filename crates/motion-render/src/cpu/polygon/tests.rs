use super::*;
use motion_core::Path2D;
use motion_core::{
    Color, DisplayList, DrawCommand, Paint, PixelSize, Rect, ValidationError, ValidationErrorKind,
    ValidationLimits,
};
use std::sync::Arc;

fn contour(path: &mut Path2D, points: &[(f64, f64)]) {
    path.try_move_to(Point::new(points[0].0, points[0].1))
        .unwrap();
    for &(horizontal, vertical) in &points[1..] {
        path.try_line_to(Point::new(horizontal, vertical)).unwrap();
    }
}

fn source(path: Path2D, rule: FillRule) -> DisplayList {
    DisplayList {
        output_size: PixelSize::new(1, 1),
        background: None,
        commands: vec![DrawCommand::Fill {
            path: Arc::new(path),
            transform: Transform::IDENTITY,
            paint: Paint::Solid(Color::WHITE),
            rule,
        }],
    }
}

fn ir_bytes(list: &DisplayList) -> u64 {
    ir_stats(list, RasterLimits::default())
        .unwrap()
        .retained_bytes
}

fn ir_stats(
    list: &DisplayList,
    limits: RasterLimits,
) -> Result<crate::prepared::PreparationStats, ValidationError> {
    let source = list.clone().validate(ValidationLimits::default()).unwrap();
    crate::raster::frame_ir::FrameIr::build(
        &source,
        ValidationLimits::default(),
        limits,
        1.0 / 256.0,
    )
    .map(|ir| ir.stats())
}

fn alpha(path: Path2D, rule: FillRule) -> u8 {
    let frame = source(path, rule)
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    crate::raster::tests::coverage(&frame)[0]
}

#[test]
fn band_quantization_handles_kind_changes_and_partial_tails_in_place() {
    let limits = RasterLimits::default();
    for width in 1..=33 {
        for height in 1..=4 {
            let mut list = source(Path2D::new(), FillRule::NonZero);
            list.output_size = PixelSize::new(width, height);
            let mut frame = list.prepare(ValidationLimits::default(), limits).unwrap();
            let mut areas = (0..height)
                .flat_map(|row| {
                    (0..width).map(move |column| match (column / 4 + row) % 4 {
                        0 => 0.0,
                        1 => 1.0,
                        2 => [0.0, 0.5 / 255.0, 0.5, 1.0][column as usize % 4],
                        _ => [-0.25, 1.25, f64::NAN, f64::INFINITY][column as usize % 4],
                    })
                })
                .collect::<Vec<_>>();
            let expected = areas
                .iter()
                .copied()
                .map(coverage_to_byte)
                .collect::<Vec<_>>();
            let mut budget = Budget::new(frame.size, limits).unwrap();
            frame
                .polygon_band(
                    &mut areas,
                    Band {
                        row: 0,
                        left: 0,
                        right: width,
                    },
                    height,
                    &mut RasterScope::new(&mut budget, "test.band"),
                )
                .unwrap();
            assert_eq!(crate::raster::tests::coverage(&frame), expected);
            assert_eq!(
                areas,
                expected.iter().copied().map(f64::from).collect::<Vec<_>>()
            );
            assert!(
                frame
                    .spans
                    .iter()
                    .all(|span| span.start_x % 4 == 0 && span.end_x % 4 == 0)
            );
        }
    }
}

#[test]
fn triangle_area_and_duplicate_contours_distinguish_winding_rules() {
    let triangle = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)];
    let mut path = Path2D::new();
    contour(&mut path, &triangle);
    assert_eq!(alpha(path.clone(), FillRule::NonZero), 128);
    assert_eq!(alpha(path.clone(), FillRule::EvenOdd), 128);
    contour(&mut path, &triangle);
    assert_eq!(alpha(path.clone(), FillRule::NonZero), 128);
    assert_eq!(alpha(path, FillRule::EvenOdd), 0);
}

#[test]
fn reversed_contours_cancel_and_bow_tie_splits_at_crossing() {
    let mut path = Path2D::new();
    contour(&mut path, &[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)]);
    contour(&mut path, &[(0.0, 1.0), (1.0, 0.0), (0.0, 0.0)]);
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        assert_eq!(alpha(path.clone(), rule), 0);
        let mut bow = Path2D::new();
        contour(&mut bow, &[(0.0, 0.0), (1.0, 1.0), (0.0, 1.0), (1.0, 0.0)]);
        assert_eq!(alpha(bow, rule), 128);
    }
}

#[test]
fn holes_overlaps_shared_edges_and_close_resume_use_pointwise_winding() {
    let square = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    let inner = [(0.25, 0.25), (0.75, 0.25), (0.75, 0.75), (0.25, 0.75)];
    for reversed in [false, true] {
        let mut path = Path2D::new();
        contour(&mut path, &square);
        let mut points = inner;
        if reversed {
            points.reverse();
        }
        contour(&mut path, &points);
        assert_eq!(
            alpha(path.clone(), FillRule::NonZero),
            if reversed { 191 } else { 255 }
        );
        assert_eq!(alpha(path, FillRule::EvenOdd), 191);
    }
    let mut path = Path2D::new();
    contour(
        &mut path,
        &[(0.0, 0.0), (0.75, 0.0), (0.75, 1.0), (0.0, 1.0)],
    );
    contour(
        &mut path,
        &[(0.25, 0.0), (1.0, 0.0), (1.0, 1.0), (0.25, 1.0)],
    );
    assert_eq!(alpha(path.clone(), FillRule::NonZero), 255);
    assert_eq!(alpha(path, FillRule::EvenOdd), 128);
    let mut path = Path2D::new();
    contour(&mut path, &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)]);
    path.try_close().unwrap();
    path.try_close().unwrap();
    path.try_line_to(Point::new(1.0, 1.0)).unwrap();
    path.try_line_to(Point::new(0.0, 1.0)).unwrap();
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        assert_eq!(alpha(path.clone(), rule), 255);
    }
}

#[test]
fn shifted_triangles_match_independent_integrated_half_plane_area() {
    for origin_x in [-0.25, 0.0, 0.25, 3.5] {
        for origin_y in [-0.5, 0.0, 0.25, 3.5] {
            for width in [0.25, 1.0, 2.0, 4.0, 8.0] {
                for height in [0.5, 1.0, 2.0, 4.0] {
                    let mut path = Path2D::new();
                    contour(
                        &mut path,
                        &[
                            (origin_x, origin_y),
                            (origin_x + width, origin_y),
                            (origin_x, origin_y + height),
                        ],
                    );
                    let mut list = source(path, FillRule::NonZero);
                    list.output_size = PixelSize::new(16, 9);
                    let frame = list
                        .prepare(ValidationLimits::default(), RasterLimits::default())
                        .unwrap();
                    let actual = crate::raster::tests::coverage(&frame);
                    for row in 0..9 {
                        for column in 0..16 {
                            let left = f64::from(column).max(origin_x);
                            let top = f64::from(row).max(origin_y);
                            let right = (f64::from(column) + 1.0).min(origin_x + width);
                            let bottom = (f64::from(row) + 1.0).min(origin_y + height);
                            let primitive = |horizontal: f64, vertical: f64| {
                                (1.0 - (horizontal - origin_x) / width
                                    - (vertical - origin_y) / height)
                                    .max(0.0)
                                    .powi(2)
                            };
                            let area = if left < right && top < bottom {
                                width
                                    * height
                                    * 0.5
                                    * (primitive(left, top)
                                        - primitive(right, top)
                                        - primitive(left, bottom)
                                        + primitive(right, bottom))
                            } else {
                                0.0
                            };
                            assert_eq!(
                                actual[(row * 16 + column) as usize],
                                (area * 255.0).round() as u8,
                                "triangle {origin_x},{origin_y},{width},{height}; pixel {column},{row}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn oblique_rectangle_reflection_singular_and_offscreen_contours() {
    let transform = Transform::from_coefficients([1.0, 1.0, -1.0, 1.0, 1.0, 0.0]);
    let mut list = source(Path2D::new(), FillRule::NonZero);
    list.output_size = PixelSize::new(2, 2);
    list.commands = vec![DrawCommand::FillRect {
        bounds: Rect::new(0.0, 0.0, 1.0, 1.0),
        transform,
        paint: Paint::Solid(Color::WHITE),
    }];
    let diamond = list
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    assert_eq!(crate::raster::tests::coverage(&diamond), vec![128; 4]);
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        list.commands = vec![DrawCommand::Fill {
            path: Arc::new(Path2D::from_rect(Rect::new(0.0, 0.0, 1.0, 1.0)).unwrap()),
            transform: transform.then(Transform::from_coefficients([
                -1.0, 0.0, 0.0, 1.0, 2.0, 0.0,
            ])),
            paint: Paint::Solid(Color::WHITE),
            rule,
        }];
        let frame = list
            .prepare(ValidationLimits::default(), RasterLimits::default())
            .unwrap();
        assert_eq!(crate::raster::tests::coverage(&frame), vec![128; 4]);
    }
    list.commands = vec![DrawCommand::FillRect {
        bounds: Rect::new(0.0, 0.0, 1.0, 1.0),
        transform: Transform::from_coefficients([1.0, 1.0, 1.0, 1.0, 0.0, 0.0]),
        paint: Paint::Solid(Color::WHITE),
    }];
    let collapsed = list
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    assert!(collapsed.strips.is_empty() && collapsed.spans.is_empty());
    let huge = Path2D::from_rect(Rect::new(-1.0e200, -1.0e200, 1.0e200, 1.0e200)).unwrap();
    assert_eq!(alpha(huge, FillRule::EvenOdd), 255);
    let outside = Path2D::from_rect(Rect::new(1.0e100, 1.0e100, 2.0e100, 2.0e100)).unwrap();
    assert_eq!(alpha(outside, FillRule::NonZero), 0);
    let mut uncertain = Path2D::new();
    contour(
        &mut uncertain,
        &[
            (-1.0e200, -1.0e200),
            (1.0e200, 1.0e200),
            (-1.0e200, 1.0e200),
        ],
    );
    let error = source(uncertain, FillRule::NonZero)
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap_err();
    assert_eq!(
        (error.kind, error.field.as_str()),
        (ValidationErrorKind::InvalidValue, "geometry")
    );
}

#[test]
fn temporary_storage_peak_work_limits_and_sibling_release_are_exact() {
    let mut path = Path2D::new();
    contour(&mut path, &[(0.25, 0.5), (15.75, 0.5), (0.25, 8.5)]);
    let mut list = source(path, FillRule::NonZero);
    list.output_size = PixelSize::new(16, 9);
    let frame = list
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    assert!(frame.stats.peak_bytes > frame.stats.retained_bytes);
    let exact = RasterLimits {
        max_pixels: 144,
        max_bytes: frame.stats.peak_bytes,
        max_work: frame.stats.work,
    };
    assert_eq!(
        list.prepare(ValidationLimits::default(), exact).unwrap(),
        frame
    );
    for (limits, field) in [
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
        let error = list
            .prepare(ValidationLimits::default(), limits)
            .unwrap_err();
        assert_eq!(
            (error.kind, error.field.as_str()),
            (ValidationErrorKind::StructuralLimit, field)
        );
        assert_eq!(
            list.prepare(ValidationLimits::default(), exact).unwrap(),
            frame
        );
    }
    list.commands.push(list.commands[0].clone());
    let twice = list
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    let mut single = list.clone();
    single.commands.pop();
    assert_eq!(
        twice.stats.peak_bytes - frame.stats.peak_bytes,
        frame.stats.retained_bytes - size_of::<PreparedCpuFrame>() as u64 + ir_bytes(&list)
            - ir_bytes(&single)
    );
    assert_eq!(
        twice.stats.retained_bytes - frame.stats.retained_bytes,
        frame.stats.retained_bytes - size_of::<PreparedCpuFrame>() as u64
    );
    let single_ir = ir_stats(&single, RasterLimits::default()).unwrap();
    let repeated_ir = ir_stats(&list, RasterLimits::default()).unwrap();
    let first_work = frame.stats.work - single_ir.work;
    let first_peak = repeated_ir.retained_bytes + frame.stats.peak_bytes - single_ir.retained_bytes;
    assert_eq!(twice.stats.work, repeated_ir.work + 2 * first_work);
    for max_work in 1..twice.stats.work {
        let limits = RasterLimits {
            max_work,
            ..RasterLimits::default()
        };
        let error = list
            .prepare(ValidationLimits::default(), limits)
            .unwrap_err();
        assert_eq!(error.field, "max_work");
        match ir_stats(&list, limits) {
            Err(normalization) => assert_eq!(error, normalization),
            Ok(ir) => {
                let index = usize::from(max_work - ir.work >= first_work);
                assert_eq!(error.path, format!("display_list.commands[{index}]"));
            }
        }
    }
    for max_bytes in size_of::<PreparedCpuFrame>() as u64..twice.stats.peak_bytes {
        let limits = RasterLimits {
            max_bytes,
            ..RasterLimits::default()
        };
        let error = list
            .prepare(ValidationLimits::default(), limits)
            .unwrap_err();
        assert_eq!(error.field, "max_bytes");
        match ir_stats(&list, limits) {
            Err(normalization) => assert_eq!(error, normalization),
            Ok(ir) => {
                let header = ir.retained_bytes + size_of::<PreparedCpuFrame>() as u64;
                let index = usize::from(max_bytes >= first_peak);
                let before_command = header
                    + index as u64
                        * (frame.stats.retained_bytes - size_of::<PreparedCpuFrame>() as u64)
                    + crate::raster::frame_ir::COMMAND_PATH_BYTES;
                let expected = if max_bytes < header {
                    "prepared".to_owned()
                } else if max_bytes < before_command {
                    "prepared.commands".to_owned()
                } else {
                    format!("display_list.commands[{index}]")
                };
                assert_eq!(error.path, expected);
            }
        }
    }
}

#[test]
fn rectangle_paths_match_fast_coverage_and_linear_pixels_under_both_rules() {
    for bounds in crate::raster::tests::rectangle_cases() {
        let mut list = source(Path2D::new(), FillRule::NonZero);
        list.output_size = PixelSize::new(16, 9);
        list.commands = vec![DrawCommand::FillRect {
            bounds,
            transform: Transform::IDENTITY,
            paint: Paint::Solid(Color::WHITE),
        }];
        let fast = list
            .prepare(ValidationLimits::default(), RasterLimits::default())
            .unwrap();
        let expected = crate::raster::tests::coverage(&fast);
        let pixels = fast.compose_linear(RasterLimits::default()).unwrap();
        for rule in [FillRule::NonZero, FillRule::EvenOdd] {
            list.commands = vec![DrawCommand::Fill {
                path: Arc::new(Path2D::from_rect(bounds).unwrap()),
                transform: Transform::IDENTITY,
                paint: Paint::Solid(Color::WHITE),
                rule,
            }];
            let frame = list
                .prepare(ValidationLimits::default(), RasterLimits::default())
                .unwrap();
            assert_eq!(
                crate::raster::tests::coverage(&frame),
                expected,
                "{bounds:?}, {rule:?}"
            );
            assert_eq!(
                frame
                    .compose_linear(RasterLimits::default())
                    .unwrap()
                    .pixels(),
                pixels.pixels()
            );
        }
    }
    let list = source(Path2D::new(), FillRule::NonZero);
    let frame = list
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    assert!(frame.strips.is_empty() && frame.spans.is_empty());
}

#[test]
fn rectangle_fast_path_matches_independent_sweep_and_preserves_fallback() {
    let size = PixelSize::new(16, 9);
    let blank = DisplayList {
        output_size: size,
        background: None,
        commands: vec![],
    }
    .prepare(ValidationLimits::default(), RasterLimits::default())
    .unwrap();
    let render = |geometry: &Path2D, transform, rule, fast, limits| {
        let mut frame = blank.clone();
        let mut budget = Budget::new(size, limits)?;
        budget.bytes(size_of::<PreparedCpuFrame>() as u64, "test")?;
        if fast {
            frame.polygon_path(geometry, transform, rule, 0.25, &mut budget, "test")?;
        } else {
            frame.polygon(
                geometry.verbs().iter().copied(),
                transform,
                rule,
                0.25,
                &mut budget,
                "test",
            )?;
        }
        Ok::<_, ValidationError>((frame, budget))
    };
    let transforms = [
        Transform::IDENTITY,
        Transform::from_coefficients([-1.0, 0.0, 0.0, 1.0, 16.0, 0.0]),
        Transform::from_coefficients([0.0, 1.0, -1.0, 0.0, 16.0, 0.0]),
        Transform::from_coefficients([1.0, 0.25, 0.0, 1.0, 0.0, 0.0]),
        Transform::from_coefficients([0.0, 0.0, 0.0, 1.0, 2.0, 0.0]),
        Transform::from_coefficients([1.0, 0.0, 0.0, 1.0, 0.25, 0.0]),
    ];
    let mut saved = 0;
    for bounds in crate::raster::tests::rectangle_cases() {
        let geometry = Path2D::from_rect(bounds).unwrap();
        for transform in transforms {
            for rule in [FillRule::NonZero, FillRule::EvenOdd] {
                let (actual, budget) =
                    render(&geometry, transform, rule, true, RasterLimits::default()).unwrap();
                let (reference, scalar) =
                    render(&geometry, transform, rule, false, RasterLimits::default()).unwrap();
                assert_eq!(
                    crate::raster::tests::coverage(&actual),
                    crate::raster::tests::coverage(&reference),
                    "{bounds:?} {transform:?} {rule:?}"
                );
                saved += usize::from(budget.work_done() < scalar.work_done());
                if budget.work_done() < scalar.work_done() && saved == 1 {
                    let exact = RasterLimits {
                        max_work: budget.work_done(),
                        max_bytes: budget.peak_bytes(),
                        ..RasterLimits::default()
                    };
                    assert_eq!(
                        render(&geometry, transform, rule, true, exact).unwrap().0,
                        actual
                    );
                    for limits in [
                        RasterLimits {
                            max_work: exact.max_work - 1,
                            ..exact
                        },
                        RasterLimits {
                            max_bytes: exact.max_bytes - 1,
                            ..exact
                        },
                    ] {
                        let first = render(&geometry, transform, rule, true, limits)
                            .err()
                            .unwrap();
                        let again = render(&geometry, transform, rule, true, limits)
                            .err()
                            .unwrap();
                        assert_eq!(first, again);
                        assert_eq!(first.path, "test");
                        assert_eq!(
                            render(&geometry, transform, rule, true, exact).unwrap().0,
                            actual
                        );
                    }
                }
            }
        }
    }
    assert!(saved > 0);
    for text in [
        "M1 1L12 1L12 8L1 8Z M4 3L8 3L8 6L4 6Z",
        "M1 1L12 8L1 8L12 1Z",
        "M1 1C2 4 8 7 12 1L12 8L1 8Z",
    ] {
        let geometry = Path2D::parse_svg(text).unwrap();
        let (actual, budget) = render(
            &geometry,
            Transform::IDENTITY,
            FillRule::EvenOdd,
            true,
            RasterLimits::default(),
        )
        .unwrap();
        let (reference, scalar) = render(
            &geometry,
            Transform::IDENTITY,
            FillRule::EvenOdd,
            false,
            RasterLimits::default(),
        )
        .unwrap();
        assert_eq!(actual, reference);
        assert_eq!(
            (budget.work_done(), budget.peak_bytes()),
            (scalar.work_done(), scalar.peak_bytes())
        );
    }
}

#[test]
#[ignore = "Explicit 1080p diagnostic corpus, not timing or a general-scene percentage"]
fn intersection_free_band_diagnostics() {
    let cases = [
        ("pixel-rectangle", "M10 10L290 10L290 150L10 150Z"),
        ("svg-path", "M10 80C60-20 240 180 290 80L240 150H60Z"),
        (
            "curved-hole",
            "M40 180Q150 20 300 160C410 30 560 100 580 260L60 300Z M220 190C220 150 280 150 280 190C280 230 220 230 220 190Z",
        ),
        ("bow-tie", "M10 10L290 150L10 150L290 10Z"),
        ("row-boundary-crossing", "M10 10L290 150L10 150L290 10Z"),
        (
            "row-internal-crossing",
            "M10 10.25L290 150.75L10 150.75L290 10.25Z",
        ),
    ];
    for (name, text) in cases {
        let geometry = Path2D::parse_svg(text).unwrap();
        let totals = sweep::diagnostics::measure(|| {
            let mut list = source(geometry, FillRule::EvenOdd);
            list.output_size = PixelSize::new(1920, 1080);
            if let DrawCommand::Fill { transform, .. } = &mut list.commands[0] {
                *transform = Transform::from_coefficients([
                    3.0,
                    0.0,
                    0.0,
                    3.0,
                    0.0,
                    if name == "row-boundary-crossing" {
                        1.0
                    } else {
                        0.0
                    },
                ]);
            }
            list.prepare(ValidationLimits::default(), RasterLimits::default())
                .unwrap();
        });
        println!(
            "intersection-diagnostic name={name} bands={} crossing_bands={} rows={} crossing_rows={}",
            totals.bands, totals.crossing_bands, totals.rows, totals.crossing_rows
        );
        if name == "pixel-rectangle" {
            assert_eq!(totals.bands, 0);
        }
        if name == "row-boundary-crossing" {
            assert!(totals.crossing_bands > 0);
            assert_eq!(totals.crossing_rows, 0);
        }
        if name == "row-internal-crossing" {
            assert!(totals.crossing_bands > 0);
        }
    }
}

#[test]
fn compilation_composes_shared_path_coverage_and_accepts_curves() {
    use motion_core::{Node, NodeModifiers, Shape, Size};
    let mut path = Path2D::new();
    contour(&mut path, &[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)]);
    let node = Node::group([
        Node::from(path.clone().fill(Color::WHITE)),
        Node::from(path.fill(Color::WHITE)),
    ])
    .opacity(0.5);
    let list = node
        .compile(
            Size::new(1.0, 1.0),
            PixelSize::new(1, 1),
            ValidationLimits::default(),
        )
        .unwrap();
    let frame = list
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    assert!(matches!(
        frame.items.as_slice(),
        [
            PreparedItem::PushLayer { .. },
            PreparedItem::Fill { paint: 0, .. },
            PreparedItem::Fill { paint: 1, .. },
            PreparedItem::PopLayer
        ]
    ));
    let coverage = 128.0f32 / 255.0;
    let expected = (coverage + coverage * (1.0 - coverage)) * 0.5;
    assert_eq!(
        frame
            .compose_linear(RasterLimits::default())
            .unwrap()
            .pixel(0, 0),
        Some([expected; 4])
    );
    for cubic in [false, true] {
        for offset in [0.0, 100.0] {
            let mut path = Path2D::new();
            path.try_move_to(Point::new(offset, offset)).unwrap();
            if cubic {
                path.try_cubic_to(
                    Point::new(offset, offset + 1.0),
                    Point::new(offset + 1.0, offset + 1.0),
                    Point::new(offset + 1.0, offset),
                )
                .unwrap();
            } else {
                path.try_quadratic_to(
                    Point::new(offset, offset + 1.0),
                    Point::new(offset + 1.0, offset),
                )
                .unwrap();
            }
            for rule in [FillRule::NonZero, FillRule::EvenOdd] {
                let mut list = source(path.clone(), rule);
                list.commands.insert(
                    0,
                    DrawCommand::PushLayer {
                        opacity: 0.0,
                        bounds: Rect::new(0.0, 0.0, 1.0, 1.0),
                        blend_mode: motion_core::drawing::BlendMode::SourceOver,
                        clip: None,
                    },
                );
                list.commands.push(DrawCommand::PopLayer);
                let frame = list
                    .prepare(ValidationLimits::default(), RasterLimits::default())
                    .unwrap();
                assert_eq!(frame.items.len(), 3);
                assert_eq!(
                    frame
                        .compose_linear(RasterLimits::default())
                        .unwrap()
                        .pixel(0, 0),
                    Some([0.0; 4])
                );
            }
        }
    }
}

#[test]
fn quadratic_and_cubic_caps_match_independent_exact_area() {
    for cubic in [false, true] {
        let mut path = Path2D::new();
        path.try_move_to(Point::ORIGIN).unwrap();
        if cubic {
            path.try_cubic_to(
                Point::new(0.0, 1.0),
                Point::new(1.0, 1.0),
                Point::new(1.0, 0.0),
            )
            .unwrap();
        } else {
            path.try_quadratic_to(Point::new(0.5, 2.0), Point::new(1.0, 0.0))
                .unwrap();
        }
        for rule in [FillRule::NonZero, FillRule::EvenOdd] {
            let actual = alpha(path.clone(), rule);
            let exact: u8 = if cubic { 153 } else { 170 };
            assert_eq!(actual, if cubic { 152 } else { 169 });
            assert!(actual.abs_diff(exact) <= 1);
        }
    }
}

fn quadratic_cap(origin_x: f64, origin_y: f64, width: f64, height: f64) -> Path2D {
    let mut path = Path2D::new();
    path.try_move_to(Point::new(origin_x, origin_y)).unwrap();
    path.try_quadratic_to(
        Point::new(origin_x + width * 0.5, origin_y + height * 2.0),
        Point::new(origin_x + width, origin_y),
    )
    .unwrap();
    path
}

#[test]
fn transformed_curve_chords_bound_sampled_geometry_and_keep_reversals() {
    let curve = CubicBez::new((0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, 0.0));
    for transform in [
        Transform::IDENTITY,
        Transform::scale_non_uniform(128.0, 0.25),
        Transform::scale_non_uniform(0.25, 128.0),
        Transform::from_coefficients([8.0, 3.0, -2.0, 4.0, 7.0, 1.0]),
        Transform::from_coefficients([-16.0, 0.0, 0.0, 2.0, 16.0, 0.25]),
    ] {
        let map = |point: kurbo::Point| {
            let point = transform.apply(Point::new(point.x, point.y));
            (point.x, point.y)
        };
        let curve = CubicBez::new(map(curve.p0), map(curve.p1), map(curve.p2), map(curve.p3));
        for tolerance in [1.0 / 64.0, 1.0 / 1024.0] {
            let mut budget = Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
            let mut edges = Vec::new();
            flatten_curve(&mut edges, curve, tolerance, &mut budget, "test.curve").unwrap();
            assert_eq!(
                budget.live_bytes(),
                (edges.len() * size_of::<Edge>()) as u64
            );
            for sample in 0..=1024 {
                let point = curve.eval(f64::from(sample) / 1024.0);
                let distance = edges
                    .iter()
                    .map(|edge| {
                        let delta = edge.bottom - edge.top;
                        let offset = Point::new(point.x, point.y) - edge.top;
                        let progress = ((offset.x * delta.x + offset.y * delta.y)
                            / (delta.x * delta.x + delta.y * delta.y))
                            .clamp(0.0, 1.0);
                        libm::hypot(
                            point.x - (edge.top.x + delta.x * progress),
                            point.y - (edge.top.y + delta.y * progress),
                        )
                    })
                    .fold(f64::INFINITY, f64::min);
                assert!(
                    distance <= tolerance,
                    "{transform:?}: distance {distance} > {tolerance}"
                );
            }
        }
    }
    let curve = CubicBez::new((0.0, 0.0), (1.0, 1.0), (-1.0, -1.0), (0.0, 0.0));
    let mut budget = Budget::new(PixelSize::new(1, 1), RasterLimits::default()).unwrap();
    let mut edges = Vec::new();
    flatten_curve(&mut edges, curve, 1.0 / 256.0, &mut budget, "test.cusp").unwrap();
    assert!(edges.len() > 2);
    assert!(
        edges
            .iter()
            .any(|edge| edge.top.x > 0.1 || edge.bottom.x > 0.1)
    );
    assert!(
        edges
            .iter()
            .any(|edge| edge.top.x < -0.1 || edge.bottom.x < -0.1)
    );
}

#[test]
fn transformed_and_pretransformed_curves_have_identical_coverage() {
    let path = quadratic_cap(0.0, 0.0, 1.0, 1.0);
    for transform in [
        Transform::scale_non_uniform(12.0, 2.0).then(Transform::translate(0.25, 0.5)),
        Transform::scale_non_uniform(2.0, 7.0),
        Transform::from_coefficients([8.0, 2.0, -1.0, 3.0, 2.0, 0.5]),
        Transform::from_coefficients([-12.0, 0.0, 0.0, 4.0, 14.0, 0.25]),
        Transform::scale(0.0),
        Transform::scale(0.0).then(Transform::translate(1.0e100, 1.0e100)),
    ] {
        for rule in [FillRule::NonZero, FillRule::EvenOdd] {
            let mut original = source(path.clone(), rule);
            original.output_size = PixelSize::new(16, 9);
            if let DrawCommand::Fill {
                transform: matrix, ..
            } = &mut original.commands[0]
            {
                *matrix = transform;
            }
            let mut mapped = source(path.transformed(&transform).unwrap(), rule);
            mapped.output_size = original.output_size;
            let first = original
                .prepare(ValidationLimits::default(), RasterLimits::default())
                .unwrap();
            let second = mapped
                .prepare(ValidationLimits::default(), RasterLimits::default())
                .unwrap();
            assert_eq!(
                crate::raster::tests::coverage(&first),
                crate::raster::tests::coverage(&second)
            );
            assert_eq!(
                first
                    .compose_linear(RasterLimits::default())
                    .unwrap()
                    .pixels(),
                second
                    .compose_linear(RasterLimits::default())
                    .unwrap()
                    .pixels()
            );
        }
    }
}

#[test]
fn quadratic_pixels_follow_independent_clipped_parabola_integrals() {
    for origin_x in [-0.25, 0.0, 0.25] {
        for origin_y in [0.0, 0.25] {
            for width in [0.25, 1.0, 3.5, 8.0] {
                for height in [0.25, 1.0, 4.0, 8.0] {
                    let path = quadratic_cap(origin_x, origin_y, width, height);
                    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
                        let mut list = source(path.clone(), rule);
                        list.output_size = PixelSize::new(16, 9);
                        for tolerance in [1.0 / 256.0, 1.0 / 4096.0] {
                            let frame = list
                                .prepare_with_tolerance(
                                    ValidationLimits::default(),
                                    RasterLimits::default(),
                                    tolerance,
                                )
                                .unwrap();
                            let actual = crate::raster::tests::coverage(&frame);
                            for row in 0..9 {
                                for column in 0..16 {
                                    let left =
                                        ((f64::from(column) - origin_x) / width).clamp(0.0, 1.0);
                                    let right = ((f64::from(column + 1) - origin_x) / width)
                                        .clamp(0.0, 1.0);
                                    let integral = |threshold: f64| {
                                        if threshold >= height {
                                            return 0.0;
                                        }
                                        let root = if threshold <= 0.0 {
                                            0.0
                                        } else {
                                            (1.0 - libm::sqrt(1.0 - threshold / height)) * 0.5
                                        };
                                        let start = left.max(root);
                                        let end = right.min(1.0 - root);
                                        let primitive = |progress: f64| {
                                            width
                                                * (height
                                                    * (2.0 * progress.powi(2)
                                                        - 4.0 / 3.0 * progress.powi(3))
                                                    - threshold * progress)
                                        };
                                        if start < end {
                                            primitive(end) - primitive(start)
                                        } else {
                                            0.0
                                        }
                                    };
                                    let top = f64::from(row) - origin_y;
                                    let bottom = top + 1.0;
                                    let baseline = ((-top).max(0.0) - (-bottom).max(0.0))
                                        * width
                                        * (right - left);
                                    let area = (integral(top) - integral(bottom) - baseline)
                                        .clamp(0.0, 1.0);
                                    let expected = (area * 255.0).round() as u8;
                                    let actual = actual[(row * 16 + column) as usize];
                                    assert!(
                                        actual.abs_diff(expected) <= 1,
                                        "{origin_x},{origin_y},{width},{height}; pixel {column},{row}; tolerance {tolerance}; {actual} != {expected}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn curve_loops_self_intersections_duplicates_and_holes_keep_winding() {
    let mut loop_path = Path2D::new();
    loop_path.try_move_to(Point::new(0.5, 0.0)).unwrap();
    loop_path
        .try_cubic_to(
            Point::new(1.5, 4.0 / 3.0),
            Point::new(-0.5, 4.0 / 3.0),
            Point::new(0.5, 0.0),
        )
        .unwrap();
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let actual = alpha(loop_path.clone(), rule);
        assert_eq!(actual, 101);
        assert!(actual.abs_diff(102) <= 1);
    }
    let mut crossing = Path2D::new();
    crossing.try_move_to(Point::new(0.25, 0.0)).unwrap();
    crossing
        .try_cubic_to(
            Point::new(1.25, 1.0),
            Point::new(-0.75, 1.0),
            Point::new(0.75, 0.0),
        )
        .unwrap();
    let root = (13.0 - libm::sqrt(117.0)) / 26.0;
    let signed_lobe = -1.5 * root + 12.75 * root.powi(2) - 34.5 * root.powi(3)
        + 39.0 * root.powi(4)
        - 15.6 * root.powi(5);
    let exact = ((0.075 - 2.0 * signed_lobe) * 255.0).round() as u8;
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        assert!(alpha(crossing.clone(), rule).abs_diff(exact) <= 1);
    }
    let mut duplicate = quadratic_cap(0.0, 0.0, 1.0, 1.0);
    duplicate.try_close().unwrap();
    duplicate.try_move_to(Point::ORIGIN).unwrap();
    duplicate
        .try_quadratic_to(Point::new(0.5, 2.0), Point::new(1.0, 0.0))
        .unwrap();
    assert_eq!(
        alpha(duplicate.clone(), FillRule::NonZero),
        alpha(quadratic_cap(0.0, 0.0, 1.0, 1.0), FillRule::NonZero)
    );
    assert_eq!(alpha(duplicate, FillRule::EvenOdd), 0);
    for reversed in [false, true] {
        let mut hole = quadratic_cap(0.0, 0.0, 1.0, 1.0);
        let start = if reversed { 0.75 } else { 0.25 };
        let end = if reversed { 0.25 } else { 0.75 };
        hole.try_move_to(Point::new(start, 0.0)).unwrap();
        hole.try_quadratic_to(Point::new(0.5, 1.0), Point::new(end, 0.0))
            .unwrap();
        assert!(
            alpha(hole.clone(), FillRule::NonZero).abs_diff(if reversed { 128 } else { 170 }) <= 1
        );
        assert!(alpha(hole, FillRule::EvenOdd).abs_diff(128) <= 1);
    }
}

#[test]
fn curve_tolerance_scratch_budgets_and_hidden_failures_are_explicit() {
    let list = source(quadratic_cap(0.0, 0.0, 1.0, 1.0), FillRule::NonZero);
    let frame = list
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    assert_eq!(
        frame,
        list.prepare_with_tolerance(
            ValidationLimits::default(),
            RasterLimits::default(),
            1.0 / 256.0
        )
        .unwrap()
    );
    assert!(frame.stats.peak_bytes > frame.stats.retained_bytes);
    let exact = RasterLimits {
        max_pixels: 1,
        max_bytes: frame.stats.peak_bytes,
        max_work: frame.stats.work,
    };
    assert_eq!(
        list.prepare(ValidationLimits::default(), exact).unwrap(),
        frame
    );
    for (limits, field) in [
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
        let error = list
            .prepare(ValidationLimits::default(), limits)
            .unwrap_err();
        assert_eq!(
            (error.kind, error.field.as_str()),
            (ValidationErrorKind::StructuralLimit, field)
        );
        assert_eq!(
            list.prepare(ValidationLimits::default(), exact).unwrap(),
            frame
        );
    }
    for (limits, field) in [
        (
            RasterLimits {
                max_work: 1,
                ..exact
            },
            "max_work",
        ),
        (
            RasterLimits {
                max_bytes: size_of::<PreparedCpuFrame>() as u64,
                ..exact
            },
            "max_bytes",
        ),
    ] {
        assert_eq!(
            list.prepare(ValidationLimits::default(), limits)
                .unwrap_err()
                .field,
            field
        );
    }
    let repeated_list = DisplayList {
        output_size: list.output_size,
        background: list.background,
        commands: vec![list.commands[0].clone(), list.commands[0].clone()],
    };
    let repeated = repeated_list
        .prepare(ValidationLimits::default(), RasterLimits::default())
        .unwrap();
    assert_eq!(
        repeated.stats.peak_bytes - frame.stats.peak_bytes,
        frame.stats.retained_bytes - size_of::<PreparedCpuFrame>() as u64 + ir_bytes(&repeated_list)
            - ir_bytes(&list)
    );
    for tolerance in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let error = list
            .prepare_with_tolerance(
                ValidationLimits::default(),
                RasterLimits::default(),
                tolerance,
            )
            .unwrap_err();
        assert_eq!(
            (error.kind, error.field.as_str()),
            (ValidationErrorKind::InvalidValue, "tolerance")
        );
    }
    let coarse = list
        .prepare_with_tolerance(ValidationLimits::default(), RasterLimits::default(), 0.25)
        .unwrap();
    assert!(coarse.stats.work < frame.stats.work);
    assert!(
        crate::raster::tests::coverage(&coarse)[0].abs_diff(170)
            > crate::raster::tests::coverage(&frame)[0].abs_diff(170)
    );
    for offset in [0.0, 100.0] {
        let mut hidden = source(quadratic_cap(offset, offset, 1.0, 1.0), FillRule::EvenOdd);
        hidden.commands.insert(
            0,
            DrawCommand::PushLayer {
                opacity: 0.0,
                bounds: Rect::default(),
                blend_mode: motion_core::drawing::BlendMode::SourceOver,
                clip: None,
            },
        );
        hidden.commands.push(DrawCommand::PopLayer);
        let error = hidden
            .prepare_with_tolerance(
                ValidationLimits::default(),
                RasterLimits::default(),
                1.0e-20,
            )
            .unwrap_err();
        assert_eq!(
            (error.kind, error.field.as_str(), error.path.as_str()),
            (
                ValidationErrorKind::InvalidValue,
                "geometry",
                "display_list.commands[1]"
            )
        );
        hidden.commands[0] = DrawCommand::PopLayer;
        assert_eq!(
            hidden
                .prepare_with_tolerance(
                    ValidationLimits::default(),
                    RasterLimits::default(),
                    f64::NAN
                )
                .unwrap_err()
                .field,
            "layers"
        );
    }
}
