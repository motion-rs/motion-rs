//! GPU preparation totals and explicit per-path CPU coverage reasons.

use super::PreparationStats;

/// Counts of paths prepared with CPU coverage inside a GPU preparation attempt.
///
/// These do not change the selected backend. Rectangles and clip coverage are not
/// fallback paths; they keep their existing analytic or CPU implementations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuFallbackCounts {
    /// Local device coordinates exceed the supported f32 precision range.
    pub coordinate_range: u64,
    /// Conservative tile edge density exceeds the per-tile capacity.
    pub tile_capacity: u64,
    /// Conservative per-fill or per-frame device raster work exceeds its ceiling.
    pub work_limit: u64,
    /// Device fill count or bounding-box pixel count exceeds the frame ceiling.
    pub frame_limit: u64,
    /// Decal paints require the existing CPU coverage path.
    pub unsupported_paint: u64,
    /// Device coverage is disabled, including encoded-sRGB blend-space preparation.
    pub coverage_disabled: u64,
}

impl GpuFallbackCounts {
    /// Number of paths taking one explicitly classified CPU coverage route.
    pub fn total(self) -> u64 {
        self.coordinate_range
            + self.tile_capacity
            + self.work_limit
            + self.frame_limit
            + self.unsupported_paint
            + self.coverage_disabled
    }
}

/// Device-independent observations from a completed GPU preparation attempt.
///
/// Counts describe admitted work, not GPU timings, actual driver allocations or
/// cross-frame residency. The attempt may precede an Automatic startup CPU decision.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuPreparationStats {
    /// Logical host preparation storage and work, including CPU fallback coverage.
    pub preparation: PreparationStats,
    /// Flattened segments retained for GPU coverage.
    pub segment_count: u64,
    /// Admitted 16-by-16 coverage tiles, including fully interior tiles.
    pub tile_count: u64,
    /// Conservative edge-index capacity; actual GPU bin populations can be smaller.
    pub tile_index_capacity: u64,
    /// Bytes of fill/edge descriptors and the coverage uniform uploaded for binning.
    ///
    /// Excludes CPU coverage, paints, textures, output parameters and zeroed scratch.
    pub geometry_upload_bytes: u64,
    /// CPU coverage path counts grouped by their preparation-time reason.
    pub fallback_paths: GpuFallbackCounts,
}
