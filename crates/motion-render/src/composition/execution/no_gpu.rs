//! Seam methods of builds without the `gpu` feature; mirrors `gpu.rs` so shared execution code
//! needs no feature branches. A GPU request always fails and the CPU path never reaches these.

use super::{FrameExecution, PreparedOutput};
use crate::RasterLimits;
use motion_core::{Result, ValidatedDisplayList, ValidationError, ValidationLimits};

/// Without the gpu feature no plan can exist.
pub(super) type GpuPlan = Option<std::convert::Infallible>;

impl FrameExecution<'_> {
    pub(super) fn init_gpu(&mut self) -> Result<()> {
        Err(motion_core::Error::GpuUnavailable(Box::new(
            std::io::Error::other("Native GPU requires feature gpu"),
        )))
    }

    pub(in crate::composition) fn gpu_reset(&mut self) {}

    pub(super) fn prepare_gpu(
        &mut self,
        _: &ValidatedDisplayList,
        _: ValidationLimits,
        _: f64,
    ) -> Result<PreparedOutput> {
        Err(ValidationError::unsupported(
            "prepared",
            "backend",
            "GPU preparation requires the gpu feature",
        )
        .into())
    }

    pub(super) fn plan_gpu(&mut self, _: &PreparedOutput, _: RasterLimits) -> Result<GpuPlan> {
        Ok(None)
    }

    pub(super) fn render_gpu(
        &mut self,
        _: &PreparedOutput,
        _: GpuPlan,
        _: RasterLimits,
    ) -> Result<crate::linear::LinearFrame> {
        Err(motion_core::Error::GpuUnavailable(Box::new(
            std::io::Error::other("Native GPU rendering requires the gpu feature"),
        )))
    }

    pub(super) fn render_packed_output(
        &mut self,
        _: u32,
        _: &PreparedOutput,
        _: RasterLimits,
        _: crate::PixelFormat,
    ) -> Result<Option<crate::FrameBuffer>> {
        Ok(None)
    }
}
