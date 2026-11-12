//! Borrowed planning views over independently prepared CPU and GPU results.

use crate::gpu::raster::device::DeviceFill;
use crate::gpu::raster::prepare::GpuFrame;
use crate::prepared::{PreparedFrame, PreparedScene};

#[derive(Clone, Copy)]
pub(crate) struct FrameView<'a> {
    scene: &'a PreparedScene,
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
    type Target = PreparedScene;

    fn deref(&self) -> &Self::Target {
        self.scene
    }
}

impl AsRef<PreparedScene> for FrameView<'_> {
    fn as_ref(&self) -> &PreparedScene {
        self.scene
    }
}

pub(crate) trait FrameInput {
    fn gpu_view(&self) -> FrameView<'_>;
}

impl FrameInput for PreparedFrame {
    fn gpu_view(&self) -> FrameView<'_> {
        FrameView {
            scene: &self.scene,
            device_fills: &[],
            device_edges: &[],
            device_curves: &[],
            device_totals: Default::default(),
        }
    }
}

impl FrameInput for GpuFrame {
    fn gpu_view(&self) -> FrameView<'_> {
        FrameView {
            scene: &self.scene,
            device_fills: &self.geometry.device_fills,
            device_edges: &self.geometry.device_edges,
            device_curves: &self.geometry.device_curves,
            device_totals: self.geometry.device_totals,
        }
    }
}
