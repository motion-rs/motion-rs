//! Borrowed planning views and opaque GPU preparation for worker-side planning.

use super::OutputPlan;
use crate::gpu::raster::device::{DeviceFill, DeviceGeometry};
use crate::prepared::{FrameRecords, PreparationStats, PreparedCpuFrame};
use motion_core::{PixelSize, RasterLimits, ValidationError};

/// Independently prepared GPU geometry and composition records.
///
/// Unlike [`crate::PreparedCpuFrame`], this is not CPU coverage and has no CPU rendering API.
/// Preparing and planning it never initializes a device or submits work.
#[derive(Debug)]
pub struct PreparedGpuFrame {
    pub(crate) records: FrameRecords,
    pub(crate) geometry: DeviceGeometry,
}

impl PreparedGpuFrame {
    /// Final output dimensions, excluding filter guard bands.
    pub fn size(&self) -> PixelSize {
        self.records.size()
    }

    /// Logical retained/peak storage and admitted preparation work.
    pub fn stats(&self) -> PreparationStats {
        self.records.stats()
    }

    /// Admitted geometry, tile capacity, geometry uploads and per-path CPU reasons.
    pub fn gpu_stats(&self) -> crate::GpuPreparationStats {
        self.geometry.stats(self.stats())
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

    pub(crate) fn quality(mut self, quality: motion_core::RenderQuality) -> Self {
        self.records.quality = quality;
        self
    }

    pub(crate) fn blend_space(mut self, space: motion_core::drawing::BlendSpace) -> Self {
        self.records.blend_space = space;
        self
    }

    pub(crate) fn has_device_coverage(&self) -> bool {
        !self.geometry.device_fills.is_empty()
    }
}

impl FrameInput for PreparedGpuFrame {
    fn gpu_view(&self) -> FrameView<'_> {
        FrameView {
            scene: &self.records,
            device_fills: &self.geometry.device_fills,
            device_edges: &self.geometry.device_edges,
            device_curves: &self.geometry.device_curves,
            device_totals: self.geometry.device_totals,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct FrameView<'a> {
    scene: &'a FrameRecords,
    pub(super) device_fills: &'a [DeviceFill],
    pub(super) device_edges: &'a [[f64; 5]],
    pub(super) device_curves: &'a [crate::gpu::raster::device::DeviceCurve],
    pub(super) device_totals: crate::gpu::raster::device::DeviceTotals,
}

impl FrameView<'_> {
    pub(super) fn has_device_coverage(&self) -> bool {
        !self.device_fills.is_empty()
    }
}

impl std::ops::Deref for FrameView<'_> {
    type Target = FrameRecords;

    fn deref(&self) -> &Self::Target {
        self.scene
    }
}

impl AsRef<FrameRecords> for FrameView<'_> {
    fn as_ref(&self) -> &FrameRecords {
        self.scene
    }
}

pub(crate) trait FrameInput {
    fn gpu_view(&self) -> FrameView<'_>;
}

impl FrameInput for PreparedCpuFrame {
    fn gpu_view(&self) -> FrameView<'_> {
        FrameView {
            scene: &self.0,
            device_fills: &[],
            device_edges: &[],
            device_curves: &[],
            device_totals: Default::default(),
        }
    }
}
