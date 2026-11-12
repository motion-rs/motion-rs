//! Backend-specific session preparation results without implicit CPU materialization.

#[cfg(feature = "gpu")]
use crate::gpu::raster::prepare::GpuFrame;
use crate::prepared::{PreparedCpuFrame, FrameRecords};

pub(in crate::composition) enum PreparedOutput {
    Cpu(PreparedCpuFrame),
    #[cfg(feature = "gpu")]
    Gpu(GpuFrame),
}

impl PreparedOutput {
    /// Reference-path frame; GPU preparation is never converted to CPU coverage.
    pub(super) fn into_cpu(self) -> motion_core::Result<Self> {
        match self {
            Self::Cpu(_) => Ok(self),
            #[cfg(feature = "gpu")]
            Self::Gpu(_) => Err(motion_core::ValidationError::unsupported(
                "prepared",
                "backend",
                "GPU preparation cannot be rendered by the CPU backend",
            )
            .into()),
        }
    }
}

impl std::ops::Deref for PreparedOutput {
    type Target = FrameRecords;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Cpu(frame) => &frame.0,
            #[cfg(feature = "gpu")]
            Self::Gpu(frame) => &frame.scene,
        }
    }
}

#[cfg(feature = "gpu")]
impl crate::gpu::frame::FrameInput for PreparedOutput {
    fn gpu_view(&self) -> crate::gpu::frame::FrameView<'_> {
        match self {
            Self::Cpu(frame) => frame.gpu_view(),
            Self::Gpu(frame) => frame.gpu_view(),
        }
    }
}
