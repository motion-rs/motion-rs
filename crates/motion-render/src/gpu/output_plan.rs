//! Opaque packed-output plans that let callers plan off the submission thread.

use super::*;

/// Device-independent plan for one packed output frame, ready for [`FrameQueue::submit_output`].
/// Constructing it initializes no device and submits no work, so workers may plan ahead of
/// the thread that owns the queue.
pub struct OutputPlan {
    pub(super) plan: Plan,
    pub(super) format: crate::PixelFormat,
    size: PixelSize,
}

impl OutputPlan {
    pub(crate) fn from_gpu(
        frame: &PreparedGpuFrame,
        limits: RasterLimits,
        planning_bytes: u64,
        format: crate::PixelFormat,
    ) -> Result<Self, ValidationError> {
        Ok(Self {
            plan: Plan::with_planning_limit(frame, limits, planning_bytes, Some(format))?,
            format,
            size: frame.size(),
        })
    }

    /// Plans `frame` for a supported packed `format` without floating-point readback storage.
    /// Unsupported precision or exhausted logical budgets fail before device access.
    pub fn new(
        frame: &PreparedFrame,
        limits: RasterLimits,
        format: crate::PixelFormat,
    ) -> Result<Self, ValidationError> {
        Self::with_planning_limit(frame, limits, limits.max_bytes, format)
    }

    /// Bounds planning separately from the submission allowance in `limits`.
    pub fn with_planning_limit(
        frame: &PreparedFrame,
        limits: RasterLimits,
        planning_bytes: u64,
        format: crate::PixelFormat,
    ) -> Result<Self, ValidationError> {
        Ok(Self {
            plan: Plan::with_planning_limit(frame, limits, planning_bytes, Some(format))?,
            format,
            size: frame.size(),
        })
    }

    /// Adds output-only display processing for the frame at `frame_index`.
    pub fn with_display(
        mut self,
        display: &crate::DisplayTransform,
        frame_index: u64,
    ) -> Result<Self, ValidationError> {
        self.plan = self.plan.with_display(display, frame_index)?;
        Ok(self)
    }

    /// Logical peak bytes reserved by the queue while this frame is in flight.
    pub fn peak_bytes(&self) -> Result<u64, ValidationError> {
        Ok(self.plan.packed_stats(self.size, self.format)?.peak_bytes)
    }

    /// Logical peak bytes of planning itself, which excludes packed output resources.
    pub fn planning_peak_bytes(&self) -> u64 {
        self.plan.stats.peak_bytes
    }
}
