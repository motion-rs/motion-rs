//! Session-owned scratch vectors that polygon rasterization reuses between paths and bands.

use super::intersections::Interval;
use super::{Crossing, Edge};

/// Largest capacity, in bytes, one scratch vector may keep while idle.
pub(crate) const MAX_IDLE_SCRATCH_BYTES: usize = 1024 * 1024;

/// Buffers for the area accumulation and edge sweep of one path.
///
/// Contents are always cleared before use, so only capacity persists. Logical byte and work
/// admission is charged exactly as for fresh vectors.
#[derive(Default)]
pub(crate) struct PolygonScratch {
    pub(in crate::cpu) areas: Vec<f64>,
    pub(super) starts: Vec<usize>,
    pub(super) live: Vec<usize>,
    pub(super) active: Vec<Edge>,
    pub(super) row: RowScratch,
}

impl PolygonScratch {
    /// Drops every vector whose idle capacity exceeds [`MAX_IDLE_SCRATCH_BYTES`].
    pub(crate) fn trim(&mut self) {
        trim(&mut self.areas);
        trim(&mut self.starts);
        trim(&mut self.live);
        trim(&mut self.active);
        self.row.trim();
    }
}

/// Buffers for the crossings and trapezoid integration of one row.
#[derive(Default)]
pub(super) struct RowScratch {
    pub(super) cuts: Vec<f64>,
    pub(super) intervals: Vec<Interval>,
    pub(super) live: Vec<usize>,
    pub(super) crossings: Vec<Crossing>,
    pub(super) columns: ColumnScratch,
}

impl RowScratch {
    fn trim(&mut self) {
        trim(&mut self.cuts);
        trim(&mut self.intervals);
        trim(&mut self.live);
        trim(&mut self.crossings);
        trim(&mut self.columns.area);
        trim(&mut self.columns.cover);
    }
}

/// Per-column area and cover accumulators of one trapezoid.
#[derive(Default)]
pub(super) struct ColumnScratch {
    pub(super) area: Vec<f64>,
    pub(super) cover: Vec<f64>,
}

fn trim<Value>(values: &mut Vec<Value>) {
    values.clear();
    if values.capacity() * size_of::<Value>() > MAX_IDLE_SCRATCH_BYTES {
        *values = Vec::new();
    }
}
