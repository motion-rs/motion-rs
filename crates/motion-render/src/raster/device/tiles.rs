//! Conservative tile storage and work admission before device coverage is selected.

use super::super::polygon::Edge;
use crate::prepared::PixelRect;
use kurbo::ParamCurve;
use motion_core::ValidationError;
use motion_core::budget::Budget;

/// Width and height of a coverage tile in pixels.
pub(crate) const COVERAGE_TILE_SIZE: u32 = 16;

/// Full-row area followed by eight subrow winding carries; mirrored in coverage.wgsl.
#[cfg(feature = "gpu")]
pub(crate) const COVERAGE_CARRY_STRIDE: u64 = 9;

/// Winding samples (8 x 8) one refined pixel evaluates against every edge of its tile.
const COVERAGE_REFINEMENT_SAMPLES: u64 = 64;

/// Pixels one edge reference can force into refinement inside a tile: the pixels it crosses
/// plus the pixels to its right in rows where it ends.
const REFINED_PIXELS_PER_EDGE: u64 = 2 * COVERAGE_TILE_SIZE as u64;

/// Bytes in the host/WGSL coverage fill descriptor.
#[cfg(feature = "gpu")]
pub(crate) const COVERAGE_FILL_BYTES: u64 = 48;

/// Bytes in the host/WGSL coverage edge descriptor.
#[cfg(feature = "gpu")]
pub(crate) const COVERAGE_EDGE_BYTES: u64 = 32;

/// Bytes in the host/WGSL coverage curve descriptor.
#[cfg(feature = "gpu")]
pub(crate) const COVERAGE_CURVE_BYTES: u64 = 48;

/// Bytes in the host/WGSL coverage uniform.
#[cfg(feature = "gpu")]
pub(crate) const COVERAGE_PARAMS_BYTES: u64 = 16;

/// Maximum conservatively admitted edge references in one tile.
const MAX_TILE_EDGES: u32 = 4096;

/// Maximum control hulls used to admit one curve, without generating its line segments.
const CURVE_ADMISSION_HULLS: u32 = 8;

/// Conservative scalar work for one cubic subsegment's control polygon.
const CURVE_HULL_WORK: u64 = 128;

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
    /// Area integration and row carries visit each edge reference at most `256 + 16` times,
    /// plus the count/scatter and prefix passes. Refinement evaluates every reference against
    /// each pixel it forces into the 8 x 8 winding test.
    fn work_for(indices: u64, tile_count: u64, bounds: PixelRect) -> u64 {
        let area = u64::from(COVERAGE_TILE_SIZE).pow(2) + 18;
        let refinement = REFINED_PIXELS_PER_EDGE * COVERAGE_REFINEMENT_SAMPLES;
        indices * (area + refinement) + tile_count * 32 + bounds.pixels()
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
        let mut indices = 0_u64;
        let mut crowded = false;
        for curve in curves {
            let parts = curve.segments.min(CURVE_ADMISSION_HULLS);
            budget.work(u64::from(parts) * CURVE_HULL_WORK, path)?;
            let points = curve
                .points
                .map(|point| kurbo::Point::new(point.x, point.y));
            let cubic = kurbo::CubicBez::new(points[0], points[1], points[2], points[3]);
            for part in 0..parts {
                let first = u64::from(part) * u64::from(curve.segments) / u64::from(parts);
                let last = u64::from(part + 1) * u64::from(curve.segments) / u64::from(parts);
                let hull = cubic.subsegment(
                    first as f64 / f64::from(curve.segments)
                        ..last as f64 / f64::from(curve.segments),
                );
                let points = [hull.p0, hull.p1, hull.p2, hull.p3];
                // Split on the GPU's segment boundaries. Each generated chord is in its
                // subcurve hull; the seam tile also covers f32 evaluation/parameter error.
                let axis = |x: bool, minimum: bool| {
                    points
                        .iter()
                        .map(|point| {
                            let value = if x {
                                point.x - f64::from(bounds.left)
                            } else {
                                point.y - f64::from(bounds.top)
                            };
                            f64::from(value as f32)
                        })
                        .fold(
                            if minimum {
                                f64::INFINITY
                            } else {
                                f64::NEG_INFINITY
                            },
                            |a, b| if minimum { a.min(b) } else { a.max(b) },
                        )
                };
                let tile = |value: f64, maximum: u32| {
                    (value / f64::from(COVERAGE_TILE_SIZE))
                        .floor()
                        .clamp(0.0, f64::from(maximum - 1)) as u32
                };
                let left = tile(axis(true, true), columns).saturating_sub(1);
                let right = (tile(axis(true, false), columns) + 1).min(columns - 1);
                let top = tile(axis(false, true), rows).saturating_sub(1);
                let bottom = (tile(axis(false, false), rows) + 1).min(rows - 1);
                budget.work(
                    u64::from(right - left + 1) * u64::from(bottom - top + 1),
                    path,
                )?;
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
        budget.release(tile_count * 4);
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

    #[cfg(feature = "gpu")]
    pub(crate) fn tile_count(self) -> u64 {
        u64::from(self.columns) * u64::from(self.rows)
    }
}
