//! Conservative tile storage and boundary-integration work admission.

use super::admission::{self, Envelope};
use crate::prepared::PixelRect;
use kurbo::ParamCurve;
use motion_core::budget::Budget;
use motion_core::geometry::flatten::Edge;
use motion_core::{Point, ValidationError};

/// Width and height of a coverage tile in pixels.
pub(crate) const COVERAGE_TILE_SIZE: u32 = 16;

/// Integrated boundary areas per tile row, mirrored in coverage.wgsl.
pub(crate) const COVERAGE_BOUNDARY_STRIDE: u64 = 16;

/// Bytes in the host/WGSL coverage fill descriptor.
pub(crate) const COVERAGE_FILL_BYTES: u64 = 48;

/// Bytes in the host/WGSL coverage edge descriptor.
pub(crate) const COVERAGE_EDGE_BYTES: u64 = 32;

/// Bytes in the host/WGSL coverage curve descriptor.
pub(crate) const COVERAGE_CURVE_BYTES: u64 = 48;

/// Bytes in the host/WGSL coverage uniform.
pub(crate) const COVERAGE_PARAMS_BYTES: u64 = 16;

/// Maximum conservatively admitted edge references in one tile.
pub(super) const MAX_TILE_EDGES: u32 = 4096;

/// Scalar work admitted for one subcurve control hull.
const CURVE_HULL_WORK: u64 = 128;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TileLayout {
    pub(crate) columns: u32,
    pub(crate) rows: u32,
    pub(crate) indices: u32,
    pub(crate) work: u64,
}

impl TileLayout {
    pub(super) fn admit_curves(
        curves: &[super::DeviceCurve],
        bounds: PixelRect,
        budget: &mut Budget,
        path: &str,
    ) -> Result<Option<Self>, ValidationError> {
        let count = curves
            .iter()
            .map(|curve| u64::from(curve.segments))
            .sum::<u64>();
        let bytes = count * size_of::<Envelope>() as u64;
        budget.bytes(bytes, path)?;
        budget.work(count * CURVE_HULL_WORK, path)?;
        let mut envelopes = Vec::new();
        envelopes.try_reserve_exact(count as usize).map_err(|_| {
            ValidationError::limit(
                path,
                "max_bytes",
                "Could not reserve curve boundary envelopes",
            )
        })?;
        for curve in curves {
            let points = curve
                .points
                .map(|point| kurbo::Point::new(point.x, point.y));
            let cubic = kurbo::CubicBez::new(points[0], points[1], points[2], points[3]);
            for segment in 0..curve.segments {
                let hull = cubic.subsegment(
                    f64::from(segment) / f64::from(curve.segments)
                        ..f64::from(segment + 1) / f64::from(curve.segments),
                );
                envelopes.push(Envelope::new(
                    [hull.p0, hull.p1, hull.p2, hull.p3].map(|p| Point::new(p.x, p.y)),
                    false,
                    bounds,
                ));
            }
        }
        let result = admission::admit(&envelopes, bounds, budget, path);
        budget.release(bytes);
        result
    }

    pub(super) fn admit(
        edges: &[Edge],
        bounds: PixelRect,
        budget: &mut Budget,
        path: &str,
    ) -> Result<Option<Self>, ValidationError> {
        let bytes = edges.len() as u64 * size_of::<Envelope>() as u64;
        budget.bytes(bytes, path)?;
        budget.work(edges.len() as u64, path)?;
        let mut envelopes = Vec::new();
        envelopes.try_reserve_exact(edges.len()).map_err(|_| {
            ValidationError::limit(
                path,
                "max_bytes",
                "Could not reserve line boundary envelopes",
            )
        })?;
        envelopes.extend(edges.iter().map(|edge| {
            Envelope::new([edge.top, edge.top, edge.bottom, edge.bottom], true, bounds)
        }));
        let result = admission::admit(&envelopes, bounds, budget, path);
        budget.release(bytes);
        result
    }

    pub(crate) fn tile_count(self) -> u64 {
        u64::from(self.columns) * u64::from(self.rows)
    }
}
