//! Borrowed planning views over independently prepared CPU and GPU results.

use crate::prepared::{DeviceFill, PreparedFrame, PreparedScene};
use crate::raster::gpu_prepare::GpuFrame;

#[derive(Clone, Copy)]
pub(crate) struct FrameView<'a> {
    scene: &'a PreparedScene,
    pub(super) device_fills: &'a [DeviceFill],
    pub(super) device_edges: &'a [[f64; 5]],
    pub(super) device_totals: [u64; 2],
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
            device_totals: [0; 2],
        }
    }
}

impl FrameInput for GpuFrame {
    fn gpu_view(&self) -> FrameView<'_> {
        FrameView {
            scene: &self.scene,
            device_fills: &self.geometry.device_fills,
            device_edges: &self.geometry.device_edges,
            device_totals: self.geometry.device_totals,
        }
    }
}
