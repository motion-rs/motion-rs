//! Backend-specific session preparation results without implicit CPU materialization.

use crate::prepared::{PreparedFrame, PreparedScene};
#[cfg(feature = "gpu")]
use crate::raster::gpu_prepare::GpuFrame;

pub(in crate::composition) enum PreparedOutput {
    Cpu(PreparedFrame),
    #[cfg(feature = "gpu")]
    Gpu {
        frame: GpuFrame,
        source: motion_core::ValidatedDisplayList,
        tolerance: f64,
    },
}

impl std::ops::Deref for PreparedOutput {
    type Target = PreparedScene;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Cpu(frame) => &frame.scene,
            #[cfg(feature = "gpu")]
            Self::Gpu { frame, .. } => &frame.scene,
        }
    }
}

#[cfg(feature = "gpu")]
impl crate::gpu::frame::FrameInput for PreparedOutput {
    fn gpu_view(&self) -> crate::gpu::frame::FrameView<'_> {
        match self {
            Self::Cpu(frame) => frame.gpu_view(),
            Self::Gpu { frame, .. } => frame.gpu_view(),
        }
    }
}
