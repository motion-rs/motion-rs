//! Opaque GPU preparation for worker-side planning and ordered export submission.

use super::{
    OutputPlan,
    frame::{FrameInput, FrameView},
};
use crate::gpu::raster::prepare::GpuFrame;
use crate::prepared::PreparationStats;
use motion_core::{PixelSize, RasterLimits, ValidationError};

/// Independently prepared GPU geometry and composition records.
///
/// Unlike [`crate::PreparedCpuFrame`], this is not CPU coverage and has no CPU rendering API.
/// Preparing and planning it never initializes a device or submits work.
pub struct PreparedGpuFrame {
    pub(super) frame: GpuFrame,
}

impl PreparedGpuFrame {
    pub(crate) fn new(frame: GpuFrame) -> Self {
        Self { frame }
    }

    /// Final output dimensions, excluding filter guard bands.
    pub fn size(&self) -> PixelSize {
        self.frame.size()
    }

    /// Logical retained/peak storage and admitted preparation work.
    pub fn stats(&self) -> PreparationStats {
        self.frame.stats()
    }

    /// Admitted geometry, tile capacity, geometry uploads and per-path CPU reasons.
    pub fn gpu_stats(&self) -> crate::GpuPreparationStats {
        self.frame.geometry.stats(self.frame.stats())
    }

    /// Plans packed GPU output under the remaining frame allowance.
    pub fn plan(
        &self,
        limits: RasterLimits,
        format: crate::PixelFormat,
    ) -> Result<OutputPlan, ValidationError> {
        self.plan_with_planning_limit(limits, limits.max_bytes, format)
    }

    /// Bounds planning separately from the eventual submission allowance.
    pub fn plan_with_planning_limit(
        &self,
        limits: RasterLimits,
        planning_bytes: u64,
        format: crate::PixelFormat,
    ) -> Result<OutputPlan, ValidationError> {
        OutputPlan::from_gpu(self, limits, planning_bytes, format)
    }
}

impl FrameInput for PreparedGpuFrame {
    fn gpu_view(&self) -> FrameView<'_> {
        self.frame.gpu_view()
    }
}
