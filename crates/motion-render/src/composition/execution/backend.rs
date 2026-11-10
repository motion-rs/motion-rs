//! Backend selection and dispatch of prepared frames to the CPU rasterizer or native GPU,
//! including same-device texture targets and GPU timing collection.

use super::{FrameExecution, RenderedFrame};
use crate::composition::FrameStats;
use crate::{OutputStats, RasterLimits, Renderer};
use motion_core::Result;
#[cfg(feature = "gpu")]
use motion_core::{Color, Node, SceneContext, Time};

impl FrameExecution<'_> {
    /// Shared context after startup selection; None means the fixed CPU decision.
    #[cfg(feature = "gpu")]
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
    #[cfg(feature = "gpu")]
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

    #[cfg(feature = "gpu")]
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
        #[cfg(feature = "gpu")]
        let started = self.timings.map(|_| std::time::Instant::now());
        #[cfg(feature = "gpu")]
        let plan = crate::gpu::Plan::new(&prepared, limits)?
            .with_display(&self.composition.options.display, u64::from(index))?;
        #[cfg(feature = "gpu")]
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

    pub(in crate::composition) fn select_backend(&mut self) -> Result<crate::Renderer> {
        if let Some(decision) = &self.backend {
            return Ok(decision.renderer);
        }
        let preference = self.composition.options.backend;
        let allow_software = preference.allow_software();
        if preference == crate::BackendPreference::Cpu {
            self.backend = Some(crate::BackendDecision {
                renderer: Renderer::Cpu,
                reason: None,
            });
            return Ok(Renderer::Cpu);
        }
        // Without the gpu feature the body is a single expression and needs no closure for `?`.
        #[cfg_attr(not(feature = "gpu"), allow(clippy::redundant_closure_call))]
        let result = (|| -> Result<()> {
            #[cfg(feature = "gpu")]
            {
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
            #[cfg(not(feature = "gpu"))]
            {
                let _ = allow_software;
                Err(motion_core::Error::GpuUnavailable(Box::new(
                    std::io::Error::other("Native GPU requires feature gpu"),
                )))
            }
        })();
        match result {
            Ok(()) => {
                self.backend = Some(crate::BackendDecision {
                    renderer: Renderer::Gpu,
                    reason: None,
                });
                Ok(Renderer::Gpu)
            }
            Err(error) if matches!(preference, crate::BackendPreference::Automatic { .. }) => {
                self.gpu_reset();
                self.backend = Some(crate::BackendDecision {
                    renderer: Renderer::Cpu,
                    reason: Some(error.to_string()),
                });
                Ok(Renderer::Cpu)
            }
            Err(error) => Err(error),
        }
    }

    pub(in crate::composition) fn gpu_reset(&mut self) {
        #[cfg(feature = "gpu")]
        {
            self.gpu = None;
        }
    }

    #[cfg(feature = "gpu")]
    pub(super) fn record_gpu_timings(&mut self) {
        if let Some(timings) = self.session.timings.as_mut()
            && let Some(gpu) = self.session.gpu.as_ref()
        {
            let (encode, readback) = gpu.host_timings();
            timings.encode += encode;
            timings.readback += readback;
            timings.gpu = gpu.timings();
        }
    }

    pub(super) fn render_prepared(
        &mut self,
        index: u32,
        prepared: crate::PreparedFrame,
        limits: RasterLimits,
    ) -> Result<crate::LinearFrame> {
        match self.render_prepared_output(index, prepared, limits, false)? {
            RenderedFrame::Linear(linear) => Ok(linear),
            #[cfg(feature = "gpu")]
            RenderedFrame::Packed(_) => unreachable!("linear output requested"),
        }
    }

    pub(super) fn render_prepared_output(
        &mut self,
        index: u32,
        prepared: crate::PreparedFrame,
        limits: RasterLimits,
        packed: bool,
    ) -> Result<RenderedFrame> {
        #[cfg(not(feature = "gpu"))]
        let _ = packed;
        let result = (|| -> Result<_> {
            let preparation = prepared.stats();
            #[cfg(feature = "gpu")]
            let plan = {
                let preference = self.composition.options.backend;
                let requested = self
                    .backend
                    .as_ref()
                    .map_or(preference.wants_gpu(), |decision| {
                        decision.renderer == Renderer::Gpu
                    });
                let started = self
                    .timings
                    .filter(|_| requested)
                    .map(|_| std::time::Instant::now());
                let plan = if requested {
                    let planned = if packed {
                        crate::gpu::Plan::new_packed(
                            &prepared,
                            limits,
                            crate::PixelFormat::SrgbRgba8 {
                                premultiplied: false,
                            },
                        )
                    } else {
                        crate::gpu::Plan::new(&prepared, limits)
                    };
                    match planned {
                        Ok(plan) => Some(plan),
                        Err(error)
                            if self.backend.is_none()
                                && matches!(
                                    preference,
                                    crate::BackendPreference::Automatic { .. }
                                ) =>
                        {
                            self.backend = Some(crate::BackendDecision {
                                renderer: Renderer::Cpu,
                                reason: Some(error.to_string()),
                            });
                            None
                        }
                        Err(error) => return Err(error.into()),
                    }
                } else {
                    None
                };
                if let Some(started) = started {
                    self.timings.as_mut().expect("enabled timings").plan += started.elapsed();
                }
                plan
            };
            let renderer = self.select_backend()?;
            let rendered = match renderer {
                Renderer::Cpu => {
                    let started = self.timings.map(|_| std::time::Instant::now());
                    let linear = prepared.render_linear_pooled(limits, &mut self.cpu_pool)?;
                    if let Some(started) = started {
                        self.timings.as_mut().expect("enabled timings").composition +=
                            started.elapsed();
                    }
                    RenderedFrame::Linear(linear)
                }
                Renderer::Gpu => {
                    #[cfg(feature = "gpu")]
                    {
                        let plan = if let Some(plan) = plan {
                            plan
                        } else if packed {
                            crate::gpu::Plan::new_packed(
                                &prepared,
                                limits,
                                crate::PixelFormat::SrgbRgba8 {
                                    premultiplied: false,
                                },
                            )?
                        } else {
                            crate::gpu::Plan::new(&prepared, limits)?
                        };
                        let plan = if packed {
                            plan.with_display(&self.composition.options.display, u64::from(index))?
                        } else {
                            plan
                        };
                        let gpu = self.gpu.as_mut().expect("selected native GPU");
                        if packed {
                            let format = crate::PixelFormat::SrgbRgba8 {
                                premultiplied: false,
                            };
                            let packed_stats = plan.packed_stats(prepared.size(), format)?;
                            let output_work =
                                crate::FrameBuffer::conversion_work(prepared.size(), format);
                            let frame = gpu.render_output(&prepared, plan, format)?;
                            self.record_gpu_timings();
                            return Ok((
                                RenderedFrame::Packed(frame),
                                FrameStats {
                                    preparation,
                                    linear: crate::LinearRenderStats {
                                        peak_bytes: packed_stats.peak_bytes,
                                        work: packed_stats.work - output_work,
                                    },
                                    output: OutputStats {
                                        peak_bytes: packed_stats.peak_bytes,
                                        work: output_work,
                                    },
                                    snapshot: OutputStats::default(),
                                },
                            ));
                        }
                        let linear = gpu.render(&prepared, plan)?;
                        self.record_gpu_timings();
                        RenderedFrame::Linear(linear)
                    }
                    #[cfg(not(feature = "gpu"))]
                    {
                        return Err(motion_core::Error::GpuUnavailable(Box::new(
                            std::io::Error::other("Native GPU rendering requires the gpu feature"),
                        )));
                    }
                }
            };
            let linear_stats = match &rendered {
                RenderedFrame::Linear(linear) => linear.stats(),
                #[cfg(feature = "gpu")]
                RenderedFrame::Packed(_) => unreachable!("packed output returned above"),
            };
            Ok((
                rendered,
                FrameStats {
                    preparation,
                    linear: linear_stats,
                    output: OutputStats::default(),
                    snapshot: OutputStats::default(),
                },
            ))
        })()
        .map_err(|error| error.at_frame(u64::from(index)))?;
        self.last_stats = Some(result.1);
        Ok(result.0)
    }
}
