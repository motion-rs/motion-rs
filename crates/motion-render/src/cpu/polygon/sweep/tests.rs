use super::*;
use crate::prepared::PaintRecord;
use motion_core::{Color, DisplayList, DrawCommand, Paint, PixelSize, ValidationLimits};

fn prepared(
    edges: &[Edge],
    rule: FillRule,
    optimized: bool,
    limits: RasterLimits,
) -> Result<PreparedCpuFrame, ValidationError> {
    let size = PixelSize::new(16, 9);
    let mut budget = Budget::new(size, limits)?;
    budget.bytes(
        size_of::<PreparedCpuFrame>() as u64 + std::mem::size_of_val(edges) as u64,
        "test",
    )?;
    let mut frame = FrameRecords {
        quality: motion_core::RenderQuality::Final,
        blend_space: motion_core::drawing::BlendSpace::LinearRgb,
        size,
        output_bounds: PixelRect {
            left: 0,
            top: 0,
            right: size.width,
            bottom: size.height,
        },
        filter_halo: 0,
        background: None,
        items: Vec::new(),
        strips: Vec::new(),
        spans: Vec::new(),
        alphas: Vec::new(),
        paints: Vec::new(),
        paint_sources: Vec::new(),
        clips: Vec::new(),
        stats: PreparationStats::default(),
    };
    if optimized {
        rasterize(
            &mut frame,
            edges,
            PixelRect {
                left: 0,
                top: 0,
                right: 16,
                bottom: 9,
            },
            rule,
            &mut budget,
            "test",
        )?;
    } else {
        for row in (0..9).step_by(4) {
            let height = (9 - row).min(4);
            let count = u64::from(16 * height);
            budget.work(count, "test")?;
            let mut areas = scratch::<f64>(count, &mut budget, "test")?;
            areas.resize(count as usize, 0.0);
            for vertical in row..row + height {
                integrate_row_reference(
                    edges,
                    Band {
                        row: vertical,
                        left: 0,
                        right: 16,
                    },
                    rule,
                    &mut areas[((vertical - row) * 16) as usize..],
                    &mut RasterScope::new(&mut budget, "reference"),
                )?;
            }
            frame.polygon_band(
                &mut areas,
                Band {
                    row,
                    left: 0,
                    right: 16,
                },
                height,
                &mut RasterScope::new(&mut budget, "reference"),
            )?;
            budget.release(count * size_of::<f64>() as u64);
        }
    }
    budget.release(std::mem::size_of_val(edges) as u64);
    budget.push(
        &mut frame.paints,
        PaintRecord {
            linear_premultiplied: [1.0; 4],
        },
        "test",
    )?;
    budget.push(&mut frame.paint_sources, None, "test")?;
    budget.push(
        &mut frame.items,
        PreparedItem::Fill {
            strips: 0..frame.strips.len(),
            spans: 0..frame.spans.len(),
            paint: 0,
        },
        "test",
    )?;
    frame.stats = PreparationStats {
        retained_bytes: budget.live_bytes(),
        peak_bytes: budget.peak_bytes(),
        work: budget.work_done(),
    };
    Ok(PreparedCpuFrame(frame))
}

fn contour(points: &[Point], edges: &mut Vec<Edge>, budget: &mut Budget) {
    for index in 0..points.len() {
        add_edge(
            edges,
            points[index],
            points[(index + 1) % points.len()],
            budget,
            "fixture",
        )
        .unwrap();
    }
}

#[test]
fn overlapping_nonzero_contours_match_reference_with_fewer_integrals() {
    let mut edges = Vec::new();
    let mut building = Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
    for index in 0..24 {
        let offset = f64::from(index) / 32.0;
        contour(
            &[
                Point::new(1.25 + offset, 0.5),
                Point::new(12.75 + offset, 0.75),
                Point::new(13.25 - offset, 7.5),
                Point::new(1.0 + offset, 8.25),
            ],
            &mut edges,
            &mut building,
        );
    }
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let actual = prepared(&edges, rule, true, RasterLimits::default()).unwrap();
        let expected = prepared(&edges, rule, false, RasterLimits::default()).unwrap();
        assert_eq!(
            actual
                .compose_linear(RasterLimits::default())
                .unwrap()
                .pixels(),
            expected
                .compose_linear(RasterLimits::default())
                .unwrap()
                .pixels()
        );
        if rule == FillRule::NonZero {
            let mut merged = Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
            let mut adjacent = Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
            for row in 0..9 {
                let mut merged_areas = [0.0; 16];
                let mut adjacent_areas = [0.0; 16];
                integrate_row(
                    &edges,
                    Band {
                        row,
                        left: 0,
                        right: 16,
                    },
                    rule,
                    &mut merged_areas,
                    &mut RasterScope::new(&mut merged, "merged"),
                )
                .unwrap();
                integrate_row_adjacent(
                    &edges,
                    Band {
                        row,
                        left: 0,
                        right: 16,
                    },
                    rule,
                    &mut adjacent_areas,
                    &mut RasterScope::new(&mut adjacent, "adjacent"),
                )
                .unwrap();
                let quantize = crate::raster::coverage_to_byte;
                assert_eq!(merged_areas.map(quantize), adjacent_areas.map(quantize));
            }
            assert!(merged.work_done() < adjacent.work_done());
        }
    }
}

#[test]
fn sparse_bands_match_all_edge_integrals_with_less_admitted_work() {
    let mut edges = Vec::new();
    let mut building = Budget::new(PixelSize::new(8, 1024), RasterLimits::default()).unwrap();
    for index in (0..128).rev() {
        let top = f64::from(index * 8) + 0.25;
        add_edge(
            &mut edges,
            Point::new(0.25, top),
            Point::new(1.75, top + 0.5),
            &mut building,
            "test",
        )
        .unwrap();
        add_edge(
            &mut edges,
            Point::new(3.75, top + 0.5),
            Point::new(2.25, top),
            &mut building,
            "test",
        )
        .unwrap();
    }
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let mut optimized = Budget::new(PixelSize::new(8, 1024), RasterLimits::default()).unwrap();
        let mut reference = Budget::new(PixelSize::new(8, 1024), RasterLimits::default()).unwrap();
        let mut sweep = EdgeSweep::new(&edges, &mut optimized, "test").unwrap();
        for row in (0..1024).step_by(4) {
            let mut active = Vec::new();
            sweep
                .band(row, 4, &mut optimized, "test", &mut active)
                .unwrap();
            for vertical in row..row + 4 {
                let mut expected = [0.0; 8];
                let mut actual = [0.0; 8];
                integrate_row_reference(
                    &edges,
                    Band {
                        row: vertical,
                        left: 0,
                        right: 8,
                    },
                    rule,
                    &mut expected,
                    &mut RasterScope::new(&mut reference, "reference"),
                )
                .unwrap();
                integrate_row(
                    &active,
                    Band {
                        row: vertical,
                        left: 0,
                        right: 8,
                    },
                    rule,
                    &mut actual,
                    &mut RasterScope::new(&mut optimized, "optimized"),
                )
                .unwrap();
                assert_eq!(actual, expected);
            }
            optimized.release((active.len() * size_of::<Edge>()) as u64);
        }
        sweep.finish(&mut optimized);
        assert_eq!(optimized.live_bytes(), 0);
        assert!(
            optimized.work_done() < reference.work_done() / 4,
            "{} vs {}",
            optimized.work_done(),
            reference.work_done()
        );
    }
}

#[test]
fn dense_parallel_contours_keep_pixels_with_less_candidate_work() {
    let mut edges = Vec::new();
    let mut building = Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
    for index in (0..512).rev() {
        let left = f64::from(index) / 32.0 + 1.0 / 128.0;
        contour(
            &[
                Point::new(left, -1.0),
                Point::new(left + 1.0 / 128.0, -1.0),
                Point::new(left + 1.0 / 128.0, 10.0),
                Point::new(left, 10.0),
            ],
            &mut edges,
            &mut building,
        );
    }
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let actual = prepared(&edges, rule, true, RasterLimits::default()).unwrap();
        let expected = prepared(&edges, rule, false, RasterLimits::default()).unwrap();
        assert_eq!(actual.strips, expected.strips);
        assert_eq!(actual.spans, expected.spans);
        assert_eq!(actual.alphas, expected.alphas);
        assert_eq!(
            actual.compose_linear(RasterLimits::default()).unwrap(),
            expected.compose_linear(RasterLimits::default()).unwrap()
        );
        assert!(
            actual.stats.work < expected.stats.work * 3 / 4,
            "{} vs {}",
            actual.stats.work,
            expected.stats.work
        );
    }
}

#[test]
fn generated_contours_match_all_edge_prepared_storage_and_cpu_pixels() {
    let mut fixtures = vec![Vec::new()];
    let mut building = Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
    for case in 0..64 {
        let mut edges = Vec::new();
        let points: Vec<_> = (0..3 + case % 9)
            .map(|vertex| {
                Point::new(
                    f64::from((case * 17 + vertex * 37 + vertex * vertex * 3) % 97) / 8.0 - 2.0,
                    f64::from((case * 13 + vertex * 23 + vertex * vertex * 7) % 89) / 8.0 - 0.5,
                )
            })
            .collect();
        contour(&points, &mut edges, &mut building);
        if case % 3 == 0 {
            contour(&points, &mut edges, &mut building);
        }
        if case % 5 == 0 {
            let reversed: Vec<_> = points.iter().rev().copied().collect();
            contour(&reversed, &mut edges, &mut building);
        }
        fixtures.push(edges);
    }
    let mut curves = Vec::new();
    for offset in [0.0, 3.75, 7.5] {
        let curve = CubicBez::new(
            (1.0, offset),
            (15.0, offset + 3.0),
            (-3.0, offset + 3.0),
            (12.0, offset),
        );
        flatten_curve(&mut curves, curve, 1.0 / 256.0, &mut building, "fixture").unwrap();
        add_edge(
            &mut curves,
            Point::new(12.0, offset),
            Point::new(1.0, offset),
            &mut building,
            "fixture",
        )
        .unwrap();
    }
    fixtures.push(curves);
    for edges in fixtures {
        for rule in [FillRule::NonZero, FillRule::EvenOdd] {
            let actual = prepared(&edges, rule, true, RasterLimits::default()).unwrap();
            let expected = prepared(&edges, rule, false, RasterLimits::default()).unwrap();
            assert_eq!(actual.items, expected.items);
            assert_eq!(actual.strips, expected.strips);
            assert_eq!(actual.spans, expected.spans);
            assert_eq!(actual.alphas, expected.alphas);
            assert_eq!(actual.stats.retained_bytes, expected.stats.retained_bytes);
            assert_eq!(
                actual.compose_linear(RasterLimits::default()).unwrap(),
                expected.compose_linear(RasterLimits::default()).unwrap()
            );
        }
    }
}

#[test]
fn half_open_band_membership_and_source_order_survive_expiration() {
    let mut edges = Vec::new();
    let mut building = Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
    for (top, bottom) in [
        (8.0, 9.0),
        (4.0, 8.0),
        (-2.0, 0.0),
        (0.0, 4.0),
        (3.999999, 4.000001),
        (-1.0e100, 1.0e100),
    ] {
        add_edge(
            &mut edges,
            Point::new(0.0, top),
            Point::new(1.0, bottom),
            &mut building,
            "fixture",
        )
        .unwrap();
    }
    let mut budget = Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
    let mut sweep = EdgeSweep::new(&edges, &mut budget, "test").unwrap();
    for (row, height) in [(0, 4), (4, 4), (8, 1)] {
        let mut actual = Vec::new();
        sweep
            .band(row, height, &mut budget, "test", &mut actual)
            .unwrap();
        let expected: Vec<_> = edges
            .iter()
            .filter(|edge| edge.top.y < f64::from(row + height) && edge.bottom.y > f64::from(row))
            .collect();
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            assert_eq!(actual.top, expected.top);
            assert_eq!(actual.bottom, expected.bottom);
            assert_eq!(actual.winding, expected.winding);
        }
        budget.release((actual.len() * size_of::<Edge>()) as u64);
    }
    sweep.finish(&mut budget);
    assert_eq!(budget.live_bytes(), 0);
}

#[test]
fn huge_empty_gaps_skip_without_allocating_height_sized_storage() {
    use motion_core::Path2D;
    use std::sync::Arc;
    let mut geometry = Path2D::new();
    for (top, bottom) in [
        (0.25, 1.25),
        (f64::from(u32::MAX) - 1.75, f64::from(u32::MAX) - 0.25),
    ] {
        geometry.try_move_to(Point::new(0.25, top)).unwrap();
        geometry.try_line_to(Point::new(0.75, top)).unwrap();
        geometry.try_line_to(Point::new(0.75, bottom)).unwrap();
        geometry.try_line_to(Point::new(0.25, bottom)).unwrap();
        geometry.try_close().unwrap();
    }
    let source = DisplayList {
        output_size: PixelSize::new(1, u32::MAX),
        background: None,
        commands: vec![DrawCommand::Fill {
            path: Arc::new(geometry),
            transform: Transform::IDENTITY,
            paint: Paint::Solid(Color::WHITE),
            rule: FillRule::NonZero,
        }],
    };
    let limits = RasterLimits {
        max_pixels: u64::MAX,
        max_bytes: 8192,
        max_work: 2000,
    };
    let frame = source.prepare(ValidationLimits::default(), limits).unwrap();
    assert_eq!(frame.strips.len(), 2);
    assert_eq!(frame.strips[0].y, 0);
    assert_eq!(frame.strips[1].y, u32::MAX - 3);
    assert_eq!(frame.strips[1].height, 3);
    assert_eq!(frame.alphas.len(), 7);
    assert!(frame.stats.peak_bytes < limits.max_bytes);
    assert!(frame.stats.work < limits.max_work);
}

#[test]
fn active_indices_and_band_copies_obey_exact_limits_and_release() {
    let mut edges = Vec::new();
    let mut building = Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
    contour(
        &[
            Point::new(0.25, 0.5),
            Point::new(15.75, 0.5),
            Point::new(0.25, 8.5),
        ],
        &mut edges,
        &mut building,
    );
    let frame = prepared(&edges, FillRule::NonZero, true, RasterLimits::default()).unwrap();
    let exact = RasterLimits {
        max_pixels: 144,
        max_bytes: frame.stats.peak_bytes,
        max_work: frame.stats.work,
    };
    assert_eq!(
        prepared(&edges, FillRule::NonZero, true, exact).unwrap(),
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
        assert_eq!(
            prepared(&edges, FillRule::NonZero, true, limits)
                .unwrap_err()
                .field,
            field
        );
        assert_eq!(
            prepared(&edges, FillRule::NonZero, true, exact).unwrap(),
            frame
        );
    }
    let mut budget = Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
    let mut sweep = EdgeSweep::new(&edges, &mut budget, "test").unwrap();
    for (row, height) in [(0, 4), (4, 4), (8, 1)] {
        let mut active = Vec::new();
        sweep
            .band(row, height, &mut budget, "test", &mut active)
            .unwrap();
        budget.release((active.len() * size_of::<Edge>()) as u64);
    }
    sweep.finish(&mut budget);
    assert_eq!(budget.live_bytes(), 0);
}
