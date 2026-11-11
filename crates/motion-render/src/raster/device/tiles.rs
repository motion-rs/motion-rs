//! Conservative tile storage and work admission before device coverage is selected.

use super::super::polygon::Edge;
use crate::prepared::PixelRect;
use motion_core::ValidationError;
use motion_core::budget::Budget;

/// Width and height of a coverage tile in pixels.
pub(crate) const COVERAGE_TILE_SIZE: u32 = 16;

/// Bytes in the host/WGSL coverage fill descriptor.
#[cfg(feature = "gpu")]
pub(crate) const COVERAGE_FILL_BYTES: u64 = 48;

/// Bytes in the host/WGSL coverage edge descriptor.
#[cfg(feature = "gpu")]
pub(crate) const COVERAGE_EDGE_BYTES: u64 = 32;

/// Bytes in the host/WGSL coverage uniform.
#[cfg(feature = "gpu")]
pub(crate) const COVERAGE_PARAMS_BYTES: u64 = 16;

/// Maximum conservatively admitted edge references in one tile.
const MAX_TILE_EDGES: u32 = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TileLayout {
    pub(crate) columns: u32,
    pub(crate) rows: u32,
    pub(crate) indices: u32,
    pub(crate) work: u64,
}

impl TileLayout {
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
            // Raster area plus row carries visit each reference at most 256 + 16 times;
            // count/scatter and prefix passes are charged as well.
            work: indices * (u64::from(COVERAGE_TILE_SIZE).pow(2) + 18)
                + tile_count * 32
                + bounds.pixels(),
        }))
    }

    #[cfg(feature = "gpu")]
    pub(crate) fn tile_count(self) -> u64 {
        u64::from(self.columns) * u64::from(self.rows)
    }
}
