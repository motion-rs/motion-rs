//! Native GPU half of frame execution: device startup, planning, preparation, dispatch and
//! same-device texture targets. `no_gpu.rs` mirrors the seam methods for builds without the
//! `gpu` feature so the shared execution code carries no feature branches.

use super::{FrameExecution, PreparedOutput};
use crate::composition::FrameStats;
use crate::{RasterLimits, Renderer, WorkStats};
use motion_core::{
    Color, Node, Result, SceneContext, Shape, Time, ValidatedDisplayList, ValidationLimits,
};

/// Plan computed before device startup so unsupported precision fails before any device exists.
pub(super) type GpuPlan = Option<crate::gpu::Plan>;

impl FrameExecution<'_> {
    /// Shared context after startup selection; None means the CPU renderer.
    pub fn selected_gpu_context(&mut self) -> Result<Option<crate::GpuContext>> {
        let renderer = self.select_backend()?;
        Ok(if renderer == Renderer::Gpu {
            self.gpu.as_ref().map(|gpu| gpu.context().clone())
        } else {
            None
        })
    }

    /// Renders directly into a compatible same-device texture without reading
    /// frame pixels to CPU. The normal resource/clock/shutter contracts apply.
    pub fn render_to_texture(
        &mut self,
        index: u32,
        background: Option<Color>,
        texture: &wgpu::Texture,
        format: crate::PixelFormat,
        build: impl FnMut(&SceneContext<'_>) -> Node,
    ) -> Result<()> {
        self.submit_to_texture(index, background, texture, format, build)?
            .finish()
    }

    pub(in crate::composition) fn submit_to_texture(
        &mut self,
        index: u32,
        background: Option<Color>,
        texture: &wgpu::Texture,
        format: crate::PixelFormat,
        mut build: impl FnMut(&SceneContext<'_>) -> Node,
    ) -> Result<crate::gpu::PendingTextureFrame> {
        let end = self.duration;
        let shutter = self.composition.shutter();
        let sampling = self.composition.options.shutter_sampling;
        let output = self.composition.output_size();
        let (prepared, limits) = self.prepare_frame(
            index,
            self.time,
            if shutter.is_some() { None } else { background },
            |context| {
                if let Some(shutter) = shutter {
                    sampling
                        .capture(shutter, context, output, |sample| {
                            let backdrop =
                                background.map(|color| Node::from(sample.bounds().fill(color)));
                            let source = if sample.time() < Time::ZERO || sample.time() >= end {
                                Node::empty()
                            } else {
                                build(sample)
                            };
                            Node::group(backdrop.into_iter().chain(std::iter::once(source)))
                        })
                        .unwrap_or_else(motion_core::authoring::transition::diagnostic)
                } else {
                    build(context)
                }
            },
        )?;
        let started = self.timings.map(|_| std::time::Instant::now());
        let plan = crate::gpu::Plan::new(&prepared, limits)?
            .with_display(&self.composition.options.display, u64::from(index))?;
        if let Some(started) = started {
            self.timings.as_mut().expect("enabled timings").plan += started.elapsed();
        }
        self.selected_gpu_context()?.ok_or_else(|| {
            motion_core::Error::GpuUnavailable(Box::new(std::io::Error::other(
                "Texture targets require an available GPU backend",
            )))
        })?;
        let result = self
            .gpu
            .as_mut()
            .expect("selected native GPU")
            .submit_texture(&prepared, plan, texture, format)
            .map_err(|error| error.at_frame(u64::from(index)));
        self.record_gpu_timings();
        result
    }

    pub(super) fn init_gpu(&mut self) -> Result<()> {
        let allow_software = self.composition.options.backend.allow_software();
        if self.gpu.is_none() {
            self.gpu = Some(crate::gpu::Gpu::new()?);
        }
        let context = self.gpu.as_ref().expect("GPU initialized").context();
        context.validate_policy(allow_software)?;
        context.self_check()?;
        let enabled = self.timings.is_some();
        self.gpu
            .as_mut()
            .expect("GPU initialized")
            .enable_timings(enabled);
        Ok(())
    }

    pub(in crate::composition) fn gpu_reset(&mut self) {
        self.gpu = None;
    }

    fn record_gpu_timings(&mut self) {
        if let Some(timings) = self.session.timings.as_mut()
            && let Some(gpu) = self.session.gpu.as_ref()
        {
            let (encode, readback) = gpu.host_timings();
            timings.encode += encode;
            timings.readback += readback;
            timings.gpu = gpu.timings();
        }
    }

    pub(super) fn prepare_gpu(
        &mut self,
        list: &ValidatedDisplayList,
        validation: ValidationLimits,
        tolerance: f64,
    ) -> Result<PreparedOutput> {
        let frame = crate::gpu::raster::prepare::prepare_validated_device(
            list,
            validation,
            self.limits,
            tolerance,
            Some(&mut self.preparation_cache),
        )?
        .quality(self.composition.options.quality)
        .blend_space(self.composition.options.blend_space);
        self.gpu_preparation = Some(frame.geometry.stats(frame.stats()));
        Ok(PreparedOutput::Gpu(frame))
    }

    /// Plans a GPU request ahead of backend selection; other renderers have no plan.
    pub(super) fn plan_gpu(
        &mut self,
        prepared: &PreparedOutput,
        limits: RasterLimits,
    ) -> Result<GpuPlan> {
        if self.renderer() != Renderer::Gpu {
            return Ok(None);
        }
        let started = self.timings.map(|_| std::time::Instant::now());
        let plan = crate::gpu::Plan::new(prepared, limits)?;
        if let Some(started) = started {
            self.timings.as_mut().expect("enabled timings").plan += started.elapsed();
        }
        Ok(Some(plan))
    }

    pub(super) fn render_gpu(
        &mut self,
        prepared: &PreparedOutput,
        plan: GpuPlan,
        limits: RasterLimits,
    ) -> Result<crate::linear::LinearFrame> {
        let plan = match plan {
            Some(plan) => plan,
            None => crate::gpu::Plan::new(prepared, limits)?,
        };
        let linear = self
            .gpu
            .as_mut()
            .expect("selected native GPU")
            .render(prepared, plan)?;
        self.record_gpu_timings();
        Ok(linear)
    }

    /// Renders a device-packed output frame when the GPU can emit `format` directly.
    pub(super) fn render_packed_output(
        &mut self,
        index: u32,
        prepared: &PreparedOutput,
        limits: RasterLimits,
        format: crate::PixelFormat,
    ) -> Result<Option<crate::FrameBuffer>> {
        let packed = format == (crate::PixelFormat::Yuv420 { bit_depth: 10 })
            || self.device_packing
                && format
                    == (crate::PixelFormat::SrgbRgba8 {
                        premultiplied: false,
                    });
        if self.renderer() == Renderer::Cpu
            || !packed
            || !crate::gpu::Gpu::supports_packed_output(prepared.size(), format)
        {
            return Ok(None);
        }
        let display = self.composition.options.display.clone();
        let started = self.timings.map(|_| std::time::Instant::now());
        let plan = crate::gpu::Plan::new_packed(prepared, limits, format)
            .and_then(|plan| plan.with_display(&display, u64::from(index)))
            .map_err(|error| motion_core::Error::from(error).at_frame(u64::from(index)))?;
        if let Some(started) = started {
            self.timings.as_mut().expect("enabled timings").plan += started.elapsed();
        }
        if self.select_backend()? != Renderer::Gpu {
            return Ok(None);
        }
        let stats = plan.packed_stats(prepared.size(), format)?;
        let output_work =
            crate::FrameBuffer::display_conversion_work(prepared.size(), format, Some(&display));
        let frame = self
            .gpu
            .as_mut()
            .expect("selected native GPU")
            .render_output(prepared, plan, format)
            .map_err(|error| error.at_frame(u64::from(index)))?;
        self.record_gpu_timings();
        self.last_stats = Some(FrameStats {
            preparation: prepared.stats(),
            linear: WorkStats {
                peak_bytes: stats.peak_bytes,
                work: stats.work - output_work,
            },
            output: WorkStats {
                peak_bytes: stats.peak_bytes,
                work: output_work,
            },
        });
        Ok(Some(frame))
    }
}
