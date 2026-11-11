//! GPU preparation totals.

use super::PreparationStats;

/// Device-independent observations from a completed GPU preparation attempt.
///
/// Counts describe admitted work, not GPU timings, actual driver allocations or
/// cross-frame residency.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuPreparationStats {
    /// Logical host preparation storage and work, including rectangle and clip coverage.
    pub preparation: PreparationStats,
    /// Admitted edge slots, including Wang-sized edges generated on the GPU.
    pub segment_count: u64,
    /// Admitted 16-by-16 coverage tiles, including fully interior tiles.
    pub tile_count: u64,
    /// Conservative edge-index capacity; actual GPU bin populations can be smaller.
    pub tile_index_capacity: u64,
    /// Bytes of fill, line and curve descriptors plus the coverage uniform uploaded for binning.
    ///
    /// Excludes CPU coverage, paints, textures, output parameters and zeroed scratch.
    pub geometry_upload_bytes: u64,
    /// Analytic batches retaining CPU-evaluated primitives for an explicit precision reason.
    pub analytic_cpu_fallbacks: u64,
}
