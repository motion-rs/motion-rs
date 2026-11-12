//! Backend selection and dispatch of prepared frames to the CPU rasterizer or native GPU.

use super::{FrameExecution, PreparedOutput};
use crate::composition::FrameStats;
use crate::{BackendKind, RasterLimits, WorkStats};
use motion_core::Result;

impl FrameExecution<'_> {
    pub(in crate::composition) fn select_backend(&mut self) -> Result<crate::BackendKind> {
        let renderer = self.backend_kind();
        if renderer == BackendKind::Cpu || self.backend_ready {
            return Ok(renderer);
        }
        match self.init_gpu() {
            Ok(()) => {
                self.backend_ready = true;
                Ok(BackendKind::Gpu)
            }
            Err(error) => {
                self.gpu_reset();
                Err(error)
            }
        }
    }

    pub(super) fn render_prepared(
        &mut self,
        index: u32,
        prepared: PreparedOutput,
        limits: RasterLimits,
    ) -> Result<crate::linear::LinearFrame> {
        let result = (|| -> Result<_> {
            let plan = self.plan_gpu(&prepared, limits)?;
            let (linear, preparation) = match self.select_backend()? {
                BackendKind::Cpu => {
                    let prepared = prepared.into_cpu()?;
                    let started = self.timings.map(|_| std::time::Instant::now());
                    let linear = prepared.render_linear_pooled(limits, &mut self.cpu_pool)?;
                    if let Some(started) = started {
                        self.timings.as_mut().expect("enabled timings").composition +=
                            started.elapsed();
                    }
                    (linear, prepared.stats())
                }
                BackendKind::Gpu => (self.render_gpu(&prepared, plan, limits)?, prepared.stats()),
            };
            let stats = FrameStats {
                preparation,
                linear: linear.stats(),
                output: WorkStats::default(),
            };
            Ok((linear, stats))
        })()
        .map_err(|error| error.at_frame(u64::from(index)))?;
        self.last_stats = Some(result.1);
        Ok(result.0)
    }
}
