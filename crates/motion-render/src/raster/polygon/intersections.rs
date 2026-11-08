//! Row crossing computation for polygon edges, batched and reference variants.

use super::*;
use motion_core::ValidationError;

#[derive(Clone, Copy)]
struct Interval {
    index: usize,
    left: f64,
    right: f64,
}

pub(crate) fn row_cuts(
    edges: &[Edge],
    row: u32,
    right: u32,
    budget: &mut Budget,
    path: &str,
) -> Result<Vec<f64>, ValidationError> {
    row_cuts_batched(edges, row, right, budget, path, true)
}

fn row_cuts_batched(
    edges: &[Edge],
    row: u32,
    right: u32,
    budget: &mut Budget,
    path: &str,
    batched: bool,
) -> Result<Vec<f64>, ValidationError> {
    let top = f64::from(row);
    let bottom = top + 1.0;
    let mut cuts = Vec::new();
    budget.push(&mut cuts, top, path)?;
    budget.push(&mut cuts, bottom, path)?;
    let mut intervals = Vec::new();
    let admitted = batched && budget.work(edges.len() as u64, path).is_ok();
    for (index, edge) in edges.iter().enumerate() {
        if !admitted {
            budget.work(1, path)?;
        }
        for vertical in [edge.top.y, edge.bottom.y] {
            if vertical > top && vertical < bottom {
                budget.push(&mut cuts, vertical, path)?;
            }
        }
        if edge.top.y >= bottom || edge.bottom.y <= top {
            continue;
        }
        let start = top.max(edge.top.y);
        let end = bottom.min(edge.bottom.y);
        edge.check_projection(start, end, right, path)?;
        let first = edge.at(start);
        let last = edge.at(end);
        let magnitude = edge.top.x.abs().max(edge.bottom.x.abs());
        let padding = (magnitude * (16.0 * f64::EPSILON)).max(f64::from_bits(4));
        let (left, right) = if first.is_finite() && last.is_finite() {
            (
                (first.min(last) - padding).next_down(),
                (first.max(last) + padding).next_up(),
            )
        } else {
            (f64::NEG_INFINITY, f64::INFINITY)
        };
        budget.push(&mut intervals, Interval { index, left, right }, path)?;
    }
    #[cfg(test)]
    let vertex_cuts = cuts.len();
    sort_work(intervals.len(), budget, path)?;
    intervals.sort_unstable_by(|first, second| {
        first
            .left
            .total_cmp(&second.left)
            .then(first.index.cmp(&second.index))
    });
    let mut live: Vec<usize> = Vec::new();
    for (position, interval) in intervals.iter().enumerate() {
        budget.work(1 + live.len() as u64, path)?;
        let previous = live.len();
        live.retain(|index| intervals[*index].right >= interval.left);
        budget.release(((previous - live.len()) * size_of::<usize>()) as u64);
        let admitted = batched && budget.work(live.len() as u64, path).is_ok();
        for &other in &live {
            if !admitted {
                budget.work(1, path)?;
            }
            let first = interval.index.max(intervals[other].index);
            let second = interval.index.min(intervals[other].index);
            push_intersection(
                edges[first],
                edges[second],
                top,
                bottom,
                &mut cuts,
                budget,
                path,
            )?;
        }
        budget.push(&mut live, position, path)?;
    }
    budget.release(
        (intervals.len() * size_of::<Interval>() + live.len() * size_of::<usize>()) as u64,
    );
    #[cfg(test)]
    super::sweep::diagnostics::row(cuts.len() > vertex_cuts);
    Ok(cuts)
}

fn push_intersection(
    first_edge: Edge,
    second_edge: Edge,
    top: f64,
    bottom: f64,
    cuts: &mut Vec<f64>,
    budget: &mut Budget,
    path: &str,
) -> Result<(), ValidationError> {
    let start = top.max(first_edge.top.y).max(second_edge.top.y);
    let end = bottom.min(first_edge.bottom.y).min(second_edge.bottom.y);
    if start >= end {
        return Ok(());
    }
    let first = first_edge.at(start) - second_edge.at(start);
    let last = first_edge.at(end) - second_edge.at(end);
    if (first < 0.0 && last > 0.0) || (first > 0.0 && last < 0.0) {
        let scale = first.abs().max(last.abs());
        let fraction = (first.abs() / scale) / (first.abs() / scale + last.abs() / scale);
        let crossing = start + (end - start) * fraction;
        if crossing > start && crossing < end {
            budget.push(cuts, crossing, path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn row_cuts_reference(
    edges: &[Edge],
    row: u32,
    right: u32,
    budget: &mut Budget,
    path: &str,
) -> Result<Vec<f64>, ValidationError> {
    let top = f64::from(row);
    let bottom = top + 1.0;
    let mut cuts = Vec::new();
    budget.push(&mut cuts, top, path)?;
    budget.push(&mut cuts, bottom, path)?;
    for (index, edge) in edges.iter().enumerate() {
        budget.work(1, path)?;
        for vertical in [edge.top.y, edge.bottom.y] {
            if vertical > top && vertical < bottom {
                budget.push(&mut cuts, vertical, path)?;
            }
        }
        if edge.top.y >= bottom || edge.bottom.y <= top {
            continue;
        }
        edge.check_projection(top.max(edge.top.y), bottom.min(edge.bottom.y), right, path)?;
        for other in &edges[..index] {
            budget.work(1, path)?;
            push_intersection(*edge, *other, top, bottom, &mut cuts, budget, path)?;
        }
    }
    Ok(cuts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::{PixelSize, ValidationErrorKind};

    #[test]
    fn batched_events_preserve_work_storage_and_first_failure() {
        let mut building = Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
        let mut edges = Vec::new();
        for index in 0..8 {
            add_edge(
                &mut edges,
                Point::new(f64::from(index), 0.0),
                Point::new(12.0 - f64::from(index), 2.0),
                &mut building,
                "fixture",
            )
            .unwrap();
        }
        let run = |limits, batched| {
            let mut budget = Budget::new(PixelSize::new(16, 9), limits).unwrap();
            let result = row_cuts_batched(&edges, 0, 16, &mut budget, "events", batched);
            (result, budget)
        };
        let (_, full) = run(RasterLimits::default(), false);
        for (max_work, max_bytes) in (1..=full.work_done() + 1)
            .map(|work| (work, u64::MAX))
            .chain((1..=full.peak_bytes() + 1).map(|bytes| (u64::MAX, bytes)))
        {
            let limits = RasterLimits {
                max_work,
                max_bytes,
                ..RasterLimits::default()
            };
            let (actual, actual_budget) = run(limits, true);
            let (expected, expected_budget) = run(limits, false);
            match (actual, expected) {
                (Ok(actual), Ok(expected)) => {
                    assert_eq!(normalized(actual), normalized(expected));
                    assert_eq!(
                        (
                            actual_budget.work_done(),
                            actual_budget.live_bytes(),
                            actual_budget.peak_bytes()
                        ),
                        (
                            expected_budget.work_done(),
                            expected_budget.live_bytes(),
                            expected_budget.peak_bytes()
                        )
                    );
                }
                (Err(actual), Err(expected)) => assert_eq!(
                    (actual.kind, actual.field, actual.path),
                    (expected.kind, expected.field, expected.path)
                ),
                other => panic!("Admission mismatch: {other:?}"),
            }
        }
    }

    fn normalized(mut cuts: Vec<f64>) -> Vec<u64> {
        cuts.sort_unstable_by(f64::total_cmp);
        cuts.into_iter().map(f64::to_bits).collect()
    }

    #[test]
    fn generated_row_events_match_all_pairs_bit_for_bit() {
        let mut building = Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
        for case in 0..96 {
            let mut edges = Vec::new();
            for index in 0..3 + case % 15 {
                let first = Point::new(
                    f64::from((case * 31 + index * 13) % 137) / 8.0 - 4.0,
                    f64::from((case * 17 + index * 23) % 97) / 8.0 - 1.0,
                );
                let second = Point::new(
                    f64::from((case * 19 + index * 47) % 131) / 8.0 - 4.0,
                    f64::from((case * 29 + index * 37) % 89) / 8.0 - 0.5,
                );
                add_edge(&mut edges, first, second, &mut building, "fixture").unwrap();
                if index % 4 == 0 {
                    add_edge(&mut edges, second, first, &mut building, "fixture").unwrap();
                }
            }
            for row in 0..9 {
                let mut optimized =
                    Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
                let mut reference =
                    Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
                let actual = row_cuts(&edges, row, 16, &mut optimized, "test").unwrap();
                let expected = row_cuts_reference(&edges, row, 16, &mut reference, "test").unwrap();
                assert_eq!(
                    optimized.live_bytes(),
                    (actual.len() * size_of::<f64>()) as u64
                );
                assert_eq!(
                    normalized(actual),
                    normalized(expected),
                    "case {case}, row {row}"
                );
            }
        }
    }

    #[test]
    fn outward_bounds_keep_ulp_close_crossings_and_source_pair_orientation() {
        let smallest = f64::from_bits(1);
        for base in [
            -f64::MAX,
            -1.0e20,
            -1.0,
            -f64::MIN_POSITIVE,
            -1.0e-310,
            -smallest,
            0.0,
            smallest,
            1.0e-310,
            f64::MIN_POSITIVE,
            1.0,
            1.0e20,
            f64::MAX.next_down(),
        ] {
            let mut edges = Vec::new();
            let mut building = Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
            add_edge(
                &mut edges,
                Point::new(base, 0.0),
                Point::new(base.next_up(), 1.0),
                &mut building,
                "fixture",
            )
            .unwrap();
            add_edge(
                &mut edges,
                Point::new(base.next_up(), 0.0),
                Point::new(base, 1.0),
                &mut building,
                "fixture",
            )
            .unwrap();
            for reversed in [false, true] {
                if reversed {
                    edges.reverse();
                }
                let mut optimized =
                    Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
                let mut reference =
                    Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
                let actual = row_cuts(&edges, 0, 16, &mut optimized, "test").unwrap();
                let expected = row_cuts_reference(&edges, 0, 16, &mut reference, "test").unwrap();
                assert!(actual.contains(&0.5), "base {base}, reversed {reversed}");
                assert_eq!(normalized(actual), normalized(expected));
            }
        }
    }

    #[test]
    fn candidate_scratch_limits_recover_and_preserve_projection_errors() {
        let mut edges = Vec::new();
        let mut building = Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
        for index in 0..12 {
            add_edge(
                &mut edges,
                Point::new(f64::from(index) * 0.25, -0.25),
                Point::new(4.0 - f64::from(index) * 0.2, 1.25),
                &mut building,
                "fixture",
            )
            .unwrap();
        }
        let mut budget = Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
        let expected = row_cuts(&edges, 0, 16, &mut budget, "test").unwrap();
        assert_eq!(
            budget.live_bytes(),
            (expected.len() * size_of::<f64>()) as u64
        );
        let exact = RasterLimits {
            max_pixels: 144,
            max_bytes: budget.peak_bytes(),
            max_work: budget.work_done(),
        };
        let mut exact_budget = Budget::new(PixelSize::new(16, 9), exact).unwrap();
        assert_eq!(
            row_cuts(&edges, 0, 16, &mut exact_budget, "test").unwrap(),
            expected
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
            let mut limited = Budget::new(PixelSize::new(16, 9), limits).unwrap();
            let error = row_cuts(&edges, 0, 16, &mut limited, "test").unwrap_err();
            assert_eq!(
                (error.kind, error.field.as_str()),
                (ValidationErrorKind::StructuralLimit, field)
            );
            let mut recovered = Budget::new(PixelSize::new(16, 9), exact).unwrap();
            assert_eq!(
                row_cuts(&edges, 0, 16, &mut recovered, "test").unwrap(),
                expected
            );
        }
        let bad = [Edge {
            top: Point::new(-1.0e200, -1.0e200),
            bottom: Point::new(1.0e200, 1.0e200),
            winding: 1,
        }];
        let mut optimized = Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
        let mut reference = Budget::new(PixelSize::new(16, 9), RasterLimits::default()).unwrap();
        let actual = row_cuts(&bad, 0, 16, &mut optimized, "test.bad").unwrap_err();
        let expected = row_cuts_reference(&bad, 0, 16, &mut reference, "test.bad").unwrap_err();
        assert_eq!(
            (actual.kind, actual.field, actual.path),
            (expected.kind, expected.field, expected.path)
        );
    }
}
