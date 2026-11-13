//! Conservative tile storage and work admission before device coverage is selected.

use crate::prepared::PixelRect;
use kurbo::{ParamCurve, ParamCurveDeriv};
use motion_core::ValidationError;
use motion_core::budget::Budget;
use motion_core::geometry::flatten::Edge;

/// Width and height of a coverage tile in pixels.
pub(crate) const COVERAGE_TILE_SIZE: u32 = 16;

/// Signed row height followed by four sample-row winding carries; mirrored in coverage.wgsl.
pub(crate) const COVERAGE_CARRY_STRIDE: u64 = 5;

/// Winding samples (4 x 4) one refined pixel evaluates against every edge of its tile.
const COVERAGE_REFINEMENT_SAMPLES: u64 = 16;

/// Every pixel can require refinement, including pixels affected by carried windings.
const REFINED_PIXELS_PER_TILE: u64 = (COVERAGE_TILE_SIZE as u64).pow(2);

/// Bytes in the host/WGSL coverage fill descriptor.
pub(crate) const COVERAGE_FILL_BYTES: u64 = 48;

/// Bytes in the host/WGSL coverage edge descriptor.
pub(crate) const COVERAGE_EDGE_BYTES: u64 = 32;

/// Bytes in the host/WGSL coverage curve descriptor.
pub(crate) const COVERAGE_CURVE_BYTES: u64 = 48;

/// Bytes in the host/WGSL coverage uniform.
pub(crate) const COVERAGE_PARAMS_BYTES: u64 = 16;

/// Maximum conservatively admitted edge references in one tile. Every pixel scans this list
/// once and a refined pixel once more per sample row, so the cap bounds the worst per-pixel
/// shader loop at 5 x 2^12 edge tests.
const MAX_TILE_EDGES: u32 = 4096;

/// Maximum control hulls used to admit one curve, without generating its line segments.
const CURVE_ADMISSION_HULLS: u32 = 8;

/// Pixels the whole control polygon must clear tile seams by to admit all hulls at once.
const SINGLE_TILE_MARGIN: f64 = 1.0 / 256.0;

/// Conservative scalar work for one cubic subsegment's control polygon.
const CURVE_HULL_WORK: u64 = 128;

/// Additional scan and bounded fill-lookup work per tile.
const SEGMENTED_PREFIX_WORK_PER_TILE: u64 = 64;

/// One full workgroup scans block sums even for a single-tile fill.
const SEGMENTED_PREFIX_WORK_PER_FILL: u64 = 512;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TileLayout {
    pub(crate) columns: u32,
    pub(crate) rows: u32,
    pub(crate) indices: u32,
    pub(crate) work: u64,
}

impl TileLayout {
    /// Work units for binning, row carries, area integration and refinement.
    ///
    /// Area integration visits each edge reference at most 256 times; row carries evaluate
    /// its height and four sample windings on each of 16 rows. Refinement charges all 16
    /// winding samples at every pixel, including pixels affected by earlier tiles.
    fn work_for(indices: u64, tile_count: u64, bounds: PixelRect) -> u64 {
        let area = u64::from(COVERAGE_TILE_SIZE).pow(2)
            + u64::from(COVERAGE_TILE_SIZE) * COVERAGE_CARRY_STRIDE
            + 2;
        let refinement = REFINED_PIXELS_PER_TILE * COVERAGE_REFINEMENT_SAMPLES;
        indices * (area + refinement)
            + tile_count * (32 + SEGMENTED_PREFIX_WORK_PER_TILE)
            + SEGMENTED_PREFIX_WORK_PER_FILL
            + bounds.pixels()
    }

    pub(super) fn admit_curves(
        curves: &[super::DeviceCurve],
        bounds: PixelRect,
        budget: &mut Budget,
        path: &str,
    ) -> Result<Option<Self>, ValidationError> {
        let columns = bounds.width().div_ceil(COVERAGE_TILE_SIZE);
        let rows = bounds.height().div_ceil(COVERAGE_TILE_SIZE);
        let tile_count = u64::from(columns) * u64::from(rows);
        budget.bytes(tile_count * 4, path)?;
        budget.work(tile_count + curves.len() as u64, path)?;
        let mut counts = Vec::<u32>::new();
        counts.try_reserve_exact(tile_count as usize).map_err(|_| {
            ValidationError::limit(path, "max_bytes", "Could not reserve curve tile admission")
        })?;
        counts.resize(tile_count as usize, 0);
        // Per-row difference counts: a hull adds its segments at its first column and removes
        // them past its last. Tile sums never exceed `indices`, so once that fits in u32 the
        // wrapping prefix sums below are exact.
        let mut indices = 0_u64;
        let tile = |value: f64, maximum: u32| {
            (value / f64::from(COVERAGE_TILE_SIZE))
                .floor()
                .clamp(0.0, f64::from(maximum - 1)) as u32
        };
        let mut add = |left: u32, right: u32, top: u32, bottom: u32, added: u32| {
            for row in top..=bottom {
                let base = (row * columns) as usize;
                counts[base + left as usize] = counts[base + left as usize].wrapping_add(added);
                if right + 1 < columns {
                    let next = base + right as usize + 1;
                    counts[next] = counts[next].wrapping_sub(added);
                }
            }
        };
        for curve in curves {
            let parts = curve.segments.min(CURVE_ADMISSION_HULLS);
            budget.work(u64::from(parts) * CURVE_HULL_WORK, path)?;
            // Subcurve hulls lie in the control polygon up to rounding far below the margin,
            // and f32 rounding, floor and clamp are monotone: if the widened polygon stays in
            // one tile on both axes, every hull admits the same tiles.
            let single = |axis: fn(&motion_core::Point) -> f64, origin: u32, maximum: u32| {
                let (low, high) = curve
                    .points
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), point| {
                        (low.min(axis(point)), high.max(axis(point)))
                    });
                let origin = f64::from(origin);
                let low = tile(
                    f64::from((low - origin - SINGLE_TILE_MARGIN) as f32),
                    maximum,
                );
                let high = tile(
                    f64::from((high - origin + SINGLE_TILE_MARGIN) as f32),
                    maximum,
                );
                (low == high).then_some(low)
            };
            let finite = curve
                .points
                .iter()
                .all(|point| point.x.is_finite() && point.y.is_finite());
            if let (true, Some(column), Some(row)) = (
                finite,
                single(|point| point.x, bounds.left, columns),
                single(|point| point.y, bounds.top, rows),
            ) {
                let (left, right) = (column.saturating_sub(1), (column + 1).min(columns - 1));
                let (top, bottom) = (row.saturating_sub(1), (row + 1).min(rows - 1));
                let spans = u64::from(right - left + 1) * u64::from(bottom - top + 1);
                budget.work(u64::from(parts) * spans, path)?;
                add(left, right, top, bottom, curve.segments);
                indices += spans * u64::from(curve.segments);
                continue;
            }
            let points = curve
                .points
                .map(|point| kurbo::Point::new(point.x, point.y));
            let cubic = kurbo::CubicBez::new(points[0], points[1], points[2], points[3]);
            let derivative = cubic.deriv();
            let segments = f64::from(curve.segments);
            // Shared hull endpoints are evaluated once, with the operations of
            // `kurbo::CubicBez::subsegment`.
            let sample = |index: u64| {
                let time = index as f64 / segments;
                (time, cubic.eval(time), derivative.eval(time).to_vec2())
            };
            let mut first = 0_u64;
            let mut start = sample(first);
            for part in 0..parts {
                let last = u64::from(part + 1) * u64::from(curve.segments) / u64::from(parts);
                let end = sample(last);
                let scale = (end.0 - start.0) * (1.0 / 3.0);
                let hull = [
                    start.1,
                    start.1 + scale * start.2,
                    end.1 - scale * end.2,
                    end.1,
                ];
                // Split on the GPU's segment boundaries. Each generated chord is in its
                // subcurve hull; the seam tile also covers f32 evaluation/parameter error.
                let [mut x_min, mut y_min] = [f64::INFINITY; 2];
                let [mut x_max, mut y_max] = [f64::NEG_INFINITY; 2];
                for point in hull {
                    let x = f64::from((point.x - f64::from(bounds.left)) as f32);
                    let y = f64::from((point.y - f64::from(bounds.top)) as f32);
                    (x_min, x_max) = (x_min.min(x), x_max.max(x));
                    (y_min, y_max) = (y_min.min(y), y_max.max(y));
                }
                let left = tile(x_min, columns).saturating_sub(1);
                let right = (tile(x_max, columns) + 1).min(columns - 1);
                let top = tile(y_min, rows).saturating_sub(1);
                let bottom = (tile(y_max, rows) + 1).min(rows - 1);
                let spans = u64::from(right - left + 1) * u64::from(bottom - top + 1);
                budget.work(spans, path)?;
                add(left, right, top, bottom, (last - first) as u32);
                indices += spans * (last - first);
                (first, start) = (last, end);
            }
        }
        let crowded = indices > u64::from(u32::MAX)
            || counts.chunks_exact(columns as usize).any(|row| {
                let mut count = 0_u32;
                row.iter().any(|delta| {
                    count = count.wrapping_add(*delta);
                    count > MAX_TILE_EDGES
                })
            });
        budget.release(tile_count * 4);
        if crowded {
            return Ok(None);
        }
        Ok(Some(Self {
            columns,
            rows,
            indices: indices as u32,
            work: Self::work_for(indices, tile_count, bounds),
        }))
    }

    pub(super) fn admit(
        edges: &[Edge],
        bounds: PixelRect,
        budget: &mut Budget,
        path: &str,
    ) -> Result<Option<Self>, ValidationError> {
        let columns = bounds.width().div_ceil(COVERAGE_TILE_SIZE);
        let rows = bounds.height().div_ceil(COVERAGE_TILE_SIZE);
        let tile_count = u64::from(columns) * u64::from(rows);
        let bytes = tile_count * 4;
        budget.bytes(bytes, path)?;
        budget.work(tile_count + edges.len() as u64, path)?;
        let mut counts = Vec::<u32>::new();
        counts.try_reserve_exact(tile_count as usize).map_err(|_| {
            ValidationError::limit(
                path,
                "max_bytes",
                "Could not reserve coverage tile admission",
            )
        })?;
        counts.resize(tile_count as usize, 0);
        let mut indices = 0_u64;
        let mut crowded = false;
        for edge in edges {
            // Admit from the actual f32 endpoints sent to WGSL, not the original f64 path.
            let x0 = f64::from((edge.top.x - f64::from(bounds.left)) as f32);
            let y0 = f64::from((edge.top.y - f64::from(bounds.top)) as f32);
            let x1 = f64::from((edge.bottom.x - f64::from(bounds.left)) as f32);
            let y1 = f64::from((edge.bottom.y - f64::from(bounds.top)) as f32);
            if y1 <= y0 {
                continue;
            }
            let first_row = (y0.max(0.0) / f64::from(COVERAGE_TILE_SIZE)).floor() as u32;
            let last_row = (y1.min(f64::from(bounds.height())).max(0.0)
                / f64::from(COVERAGE_TILE_SIZE))
            .ceil() as u32;
            for row in first_row..last_row.min(rows) {
                let top = f64::from(row * COVERAGE_TILE_SIZE).max(y0);
                let bottom = f64::from((row + 1) * COVERAGE_TILE_SIZE).min(y1);
                let xa = x0 + (x1 - x0) * ((top - y0) / (y1 - y0));
                let xb = x0 + (x1 - x0) * ((bottom - y0) / (y1 - y0));
                // One extra tile on either side covers f32 interpolation/fusion at tile seams.
                // The 8192-pixel coordinate guard bounds that error well below one tile.
                let column = |x: f64| {
                    (x / f64::from(COVERAGE_TILE_SIZE))
                        .floor()
                        .clamp(0.0, f64::from(columns - 1)) as u32
                };
                let first = column(xa.min(xb)).saturating_sub(1);
                let last = (column(xa.max(xb)) + 1).min(columns - 1);
                budget.work(u64::from(last - first + 1), path)?;
                for column in first..=last {
                    let count = &mut counts[(row * columns + column) as usize];
                    *count += 1;
                    crowded |= *count > MAX_TILE_EDGES;
                    indices += 1;
                }
            }
        }
        budget.release(bytes);
        if crowded || indices > u64::from(u32::MAX) {
            return Ok(None);
        }
        Ok(Some(Self {
            columns,
            rows,
            indices: indices as u32,
            work: Self::work_for(indices, tile_count, bounds),
        }))
    }

    pub(crate) fn tile_count(self) -> u64 {
        u64::from(self.columns) * u64::from(self.rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::raster::device::DeviceCurve;
    use motion_core::{PixelSize, Point, RasterLimits};

    /// Hull-by-hull admission with `kurbo::CubicBez::subsegment` and per-tile counting.
    fn reference(curves: &[DeviceCurve], bounds: PixelRect) -> (Option<TileLayout>, u64) {
        let columns = bounds.width().div_ceil(COVERAGE_TILE_SIZE);
        let rows = bounds.height().div_ceil(COVERAGE_TILE_SIZE);
        let tile_count = u64::from(columns) * u64::from(rows);
        let mut work = tile_count + curves.len() as u64;
        let mut counts = vec![0_u32; tile_count as usize];
        let (mut indices, mut crowded) = (0_u64, false);
        let tile = |value: f64, maximum: u32| {
            (value / f64::from(COVERAGE_TILE_SIZE))
                .floor()
                .clamp(0.0, f64::from(maximum - 1)) as u32
        };
        for curve in curves {
            let parts = curve.segments.min(CURVE_ADMISSION_HULLS);
            work += u64::from(parts) * CURVE_HULL_WORK;
            let [p0, p1, p2, p3] = curve
                .points
                .map(|point| kurbo::Point::new(point.x, point.y));
            let cubic = kurbo::CubicBez::new(p0, p1, p2, p3);
            for part in 0..parts {
                let first = u64::from(part) * u64::from(curve.segments) / u64::from(parts);
                let last = u64::from(part + 1) * u64::from(curve.segments) / u64::from(parts);
                let segments = f64::from(curve.segments);
                let hull = cubic.subsegment(first as f64 / segments..last as f64 / segments);
                let points = [hull.p0, hull.p1, hull.p2, hull.p3];
                let x = points.map(|point| f64::from((point.x - f64::from(bounds.left)) as f32));
                let y = points.map(|point| f64::from((point.y - f64::from(bounds.top)) as f32));
                let low = |values: [f64; 4]| values.into_iter().fold(f64::INFINITY, f64::min);
                let high = |values: [f64; 4]| values.into_iter().fold(f64::NEG_INFINITY, f64::max);
                let left = tile(low(x), columns).saturating_sub(1);
                let right = (tile(high(x), columns) + 1).min(columns - 1);
                let top = tile(low(y), rows).saturating_sub(1);
                let bottom = (tile(high(y), rows) + 1).min(rows - 1);
                work += u64::from(right - left + 1) * u64::from(bottom - top + 1);
                for row in top..=bottom {
                    for column in left..=right {
                        let count = &mut counts[(row * columns + column) as usize];
                        *count = count.saturating_add((last - first) as u32);
                        crowded |= *count > MAX_TILE_EDGES;
                        indices += last - first;
                    }
                }
            }
        }
        let layout = (!crowded && indices <= u64::from(u32::MAX)).then(|| TileLayout {
            columns,
            rows,
            indices: indices as u32,
            work: TileLayout::work_for(indices, tile_count, bounds),
        });
        (layout, work)
    }

    #[test]
    fn curve_admission_matches_hull_by_hull_reference() -> motion_core::Result {
        let bounds = PixelRect {
            left: 3,
            top: 5,
            right: 163,
            bottom: 125,
        };
        // Deterministic control points on, just inside and just outside tile seams,
        // plus interior and offscreen points.
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            state >> 33
        };
        let mut curves = Vec::new();
        for index in 0..400 {
            let mut coordinate = |origin: u32, extent: u32| {
                let seams = u64::from(extent / COVERAGE_TILE_SIZE + 2);
                let seam = (next() % seams) as f64 * 16.0 - 8.0;
                let offsets = [0.0, 1e-9, -1e-9, 1.0 / 512.0, -1.0 / 512.0, 0.3, 7.9];
                let offset = offsets[next() as usize % offsets.len()];
                let jitter = if next() % 3 == 0 {
                    (next() % 1000) as f64 / 100.0
                } else {
                    0.0
                };
                f64::from(origin) + seam + offset + jitter
            };
            let anchor = Point::new(coordinate(bounds.left, 160), coordinate(bounds.top, 120));
            let spread = if index % 2 == 0 { 0.004 } else { 20.0 };
            let mut step = || ((next() % 2001) as f64 / 1000.0 - 1.0) * spread;
            let mut corner = || Point::new(anchor.x + step(), anchor.y + step());
            curves.push(DeviceCurve {
                points: [anchor, corner(), corner(), corner()],
                segments: [0, 1, 3, 8, 13, 64][index % 6],
                tolerance: 0.25,
            });
        }
        let size = PixelSize::new(bounds.right, bounds.bottom);
        for length in [1, 7, curves.len()] {
            let mut budget = Budget::new(size, RasterLimits::default())?;
            let layout =
                TileLayout::admit_curves(&curves[..length], bounds, &mut budget, "curves")?;
            assert_eq!(
                (layout, budget.work_done()),
                reference(&curves[..length], bounds)
            );
        }
        // Enough stacked single-tile curves to exceed the tile edge cap.
        let crowded = (0..=MAX_TILE_EDGES / 64)
            .map(|_| DeviceCurve {
                points: curves[0].points,
                segments: 64,
                tolerance: 0.25,
            })
            .collect::<Vec<_>>();
        let mut budget = Budget::new(size, RasterLimits::default())?;
        let layout = TileLayout::admit_curves(&crowded, bounds, &mut budget, "curves")?;
        assert_eq!(layout, None);
        assert_eq!((layout, budget.work_done()), reference(&crowded, bounds));
        Ok(())
    }
}
