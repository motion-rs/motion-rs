//! One-frame render entry points and output conversion; scene preparation and backend
//! dispatch live in the `preparation` and `backend` submodules.
//!
//! CPU and GPU backends use only explicitly supplied fonts; nothing is discovered implicitly.

use super::*;
use crate::{OutputStats, RasterLimits, RenderContext, Renderer, Snapshot};
use motion_core::{Color, Node, Result, SceneContext, Shape, Time, ValidationError};

mod backend;
mod preparation;

use preparation::remaining;

enum RenderedFrame {
    Linear(crate::LinearFrame),
    #[cfg(feature = "gpu")]
    Packed(crate::FrameBuffer),
}

pub(super) struct FrameExecution<'a> {
    pub(super) composition: Composition,
    pub(super) duration: Time,
    pub(super) time: Option<Time>,
    pub(super) limits: RasterLimits,
    pub(super) session: &'a mut RenderSession,
}

impl std::ops::Deref for FrameExecution<'_> {
    type Target = RenderSession;

    fn deref(&self) -> &RenderSession {
        self.session
    }
}

impl std::ops::DerefMut for FrameExecution<'_> {
    fn deref_mut(&mut self) -> &mut RenderSession {
        self.session
    }
}

impl FrameExecution<'_> {
    /// Shared registry and structural policy; builders receive its immutable
    /// assets through SceneContext. fonts are explicit preloaded handles.
    pub fn context(&self) -> &RenderContext {
        &self.composition.options.context
    }

    /// Evaluates once at the requested frame's signed-tick time, then consumes
    /// Node -> DisplayList -> PreparedFrame -> linear-light composition -> sRGB RGBA8 -> Snapshot.
    /// GPU execution packs RGBA8 before readback; CPU execution converts its linear frame.
    ///
    /// Composition dimensions/timing/index and configured raster limits validate
    /// before evaluation. Runtime validation errors carry the requested frame.
    /// Panics in user code propagate; failures return no partial public output.
    /// SDR output uses the existing clipping/Bayer-dither contract, not HDR tone
    /// mapping or encoded-space compositing. Source builders must be pure.
    pub fn render_frame(
        &mut self,
        index: u32,
        build: impl FnOnce(&SceneContext<'_>) -> Node,
    ) -> Result<Snapshot> {
        if self.composition.shutter().is_some() {
            self.last_stats = None;
            self.asset_diagnostics.clear();
            return Err(motion_core::Error::from(ValidationError::unsupported(
                "_frame",
                "motion_blur",
                "FnOnce entry cannot resample; use render_frame_sampled",
            ))
            .at_frame(u64::from(index)));
        }
        self.render_frame_once(index, build)
    }

    /// Captures the whole frame with a centered shutter, including its background.
    /// Global out-of-range samples contain background only; callbacks retain full
    /// clip duration and are never given out-of-range global frame times.
    pub fn render_frame_sampled(
        &mut self,
        index: u32,
        mut build: impl FnMut(&SceneContext<'_>) -> Node,
    ) -> Result<Snapshot> {
        let Some(configuration) = self.composition.shutter() else {
            return self.render_frame_once(index, build);
        };
        let color = self.composition.snapshot_background();
        let color = color.with_alpha(1.0);
        let sampling = self.composition.options.shutter_sampling;
        let output = self.composition.output_size();
        self.render_frame_once(index, |context| {
            sampling
                .capture(configuration, context, output, |sample| {
                    let background = Node::from(sample.bounds().fill(color));
                    if sample.time() < Time::ZERO
                        || sample
                            .clip_duration()
                            .is_some_and(|duration| sample.time() >= duration.after_origin())
                    {
                        background
                    } else {
                        Node::group([background, Node::group([build(sample)])])
                    }
                })
                .unwrap_or_else(motion_core::authoring::transition::diagnostic)
        })
    }

    pub(super) fn render_frame_once(
        &mut self,
        index: u32,
        build: impl FnOnce(&SceneContext<'_>) -> Node,
    ) -> Result<Snapshot> {
        (|| -> Result<Snapshot> {
            let background = self.composition.snapshot_background();
            let (prepared, limits) =
                self.prepare_frame(index, None, Some(background.with_alpha(1.0)), build)?;
            let rendered = self.render_prepared_output(index, prepared, limits, true)?;
            let mut stats = self.last_stats.take().expect("successful linear stage");
            #[cfg(feature = "gpu")]
            if let RenderedFrame::Packed(frame) = rendered {
                let started = self.timings.map(|_| std::time::Instant::now());
                let (snapshot, snapshot_stats) = crate::Rgba8Frame::snapshot_from_bytes(
                    frame.size(),
                    frame.planes()[0].bytes(),
                    frame.retained_bytes(),
                    remaining(self.limits, stats.work())?,
                )?;
                if let Some(started) = started {
                    self.timings.as_mut().expect("enabled timings").snapshot += started.elapsed();
                }
                stats.snapshot = snapshot_stats;
                self.last_stats = Some(stats);
                return Ok(snapshot);
            }
            #[cfg(feature = "gpu")]
            let linear = match rendered {
                RenderedFrame::Linear(linear) => linear,
                RenderedFrame::Packed(_) => unreachable!("packed output consumed above"),
            };
            #[cfg(not(feature = "gpu"))]
            let RenderedFrame::Linear(linear) = rendered;
            let started = self.timings.map(|_| std::time::Instant::now());
            let spent = stats.preparation.work + stats.linear.work;
            let rgba = remaining(self.limits, spent).and_then(|limits| {
                crate::FrameBuffer::from_linear_display(
                    &linear,
                    crate::PixelFormat::SrgbRgba8 {
                        premultiplied: false,
                    },
                    limits,
                    &self.composition.options.display,
                    u64::from(index),
                )
            });
            linear.recycle(&mut self.cpu_pool);
            let rgba = rgba?;
            if let Some(started) = started {
                self.timings.as_mut().expect("enabled timings").output += started.elapsed();
            }
            stats.output = OutputStats {
                work: crate::FrameBuffer::display_conversion_work(
                    rgba.size(),
                    rgba.format(),
                    Some(&self.composition.options.display),
                ),
                peak_bytes: rgba.retained_bytes()
                    + u64::from(rgba.size().width) * u64::from(rgba.size().height) * 16,
            };
            let started = self.timings.map(|_| std::time::Instant::now());
            let (snapshot, snapshot_stats) = crate::Rgba8Frame::snapshot_from_bytes(
                rgba.size(),
                rgba.planes()[0].bytes(),
                rgba.retained_bytes(),
                remaining(self.limits, spent + stats.output.work)?,
            )?;
            if let Some(started) = started {
                self.timings.as_mut().expect("enabled timings").snapshot += started.elapsed();
            }
            stats.snapshot = snapshot_stats;
            self.last_stats = Some(stats);
            Ok(snapshot)
        })()
        .map_err(|error| error.at_frame(u64::from(index)))
    }

    /// Renders a repeatable callback at an exact in-range tick into a chosen memory
    /// format. None background preserves alpha; shutter capture includes that background.
    pub fn render_output_at(
        &mut self,
        time: Time,
        background: Option<Color>,
        format: crate::PixelFormat,
        build: impl FnMut(&SceneContext<'_>) -> Node,
    ) -> Result<crate::FrameBuffer> {
        let rate = self.composition.clock.rate;
        let index = u32::try_from(time.to_frame(rate))
            .map_err(|error| motion_core::Error::Parameters(Box::new(error)))?;
        let (prepared, limits) = self.prepare_output_at(time, background, build)?;
        self.finish_prepared_output(index, prepared, limits, format)
    }

    pub(super) fn prepare_output_at(
        &mut self,
        time: Time,
        background: Option<Color>,
        mut build: impl FnMut(&SceneContext<'_>) -> Node,
    ) -> Result<(crate::PreparedFrame, RasterLimits)> {
        let rate = self.composition.clock.rate;
        let end = self.duration;
        if time < Time::ZERO || time >= end {
            self.last_stats = None;
            self.asset_diagnostics.clear();
            return Err(ValidationError::invalid(
                "_frame",
                "time",
                "Output time must lie within the composition",
            )
            .into());
        }
        let index = u32::try_from(time.to_frame(rate))
            .map_err(|error| motion_core::Error::Parameters(Box::new(error)))?;
        let shutter = self.composition.shutter();
        let sampling = self.composition.options.shutter_sampling;
        let output = self.composition.output_size();
        self.prepare_frame(index, Some(time), background, |context| {
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
                        Node::group(
                            backdrop
                                .into_iter()
                                .chain(std::iter::once(Node::group([source]))),
                        )
                    })
                    .unwrap_or_else(motion_core::authoring::transition::diagnostic)
            } else {
                build(context)
            }
        })
    }

    fn finish_prepared_output(
        &mut self,
        index: u32,
        prepared: crate::PreparedFrame,
        limits: RasterLimits,
        format: crate::PixelFormat,
    ) -> Result<crate::FrameBuffer> {
        self.output_pool.trim(limits.max_bytes / 8);
        let started = self.timings.map(|_| std::time::Instant::now());
        let cpu = self.backend.as_ref().map_or_else(
            || self.composition.options.backend == crate::BackendPreference::Cpu,
            |decision| decision.renderer == Renderer::Cpu,
        );
        let display = self.composition.options.display.clone();
        if cpu && (!prepared.items().is_empty() || !display.is_identity()) {
            self.select_backend()?;
            let output_pool = self.output_pool.clone();
            let (frame, linear, output) = crate::FrameBuffer::from_prepared_display(
                &prepared,
                format,
                limits,
                &mut self.cpu_pool,
                &output_pool,
                crate::display::DisplaySettings {
                    display: (!display.is_identity()).then_some(&display),
                    frame: u64::from(index),
                    origin: [0; 2],
                },
            )
            .map_err(|error| motion_core::Error::from(error).at_frame(u64::from(index)))?;
            self.last_stats = Some(FrameStats {
                preparation: prepared.stats(),
                linear,
                output,
                snapshot: OutputStats::default(),
            });
            if let Some(started) = started {
                self.timings.as_mut().expect("enabled timings").output += started.elapsed();
                self.timings.as_mut().expect("enabled timings").fused_output = true;
            }
            return Ok(frame);
        }
        if prepared.items().is_empty() && cpu {
            self.select_backend()?;
            let pixels = u64::from(prepared.size().width) * u64::from(prepared.size().height);
            let limits = remaining(limits, pixels)
                .map_err(|error| motion_core::Error::from(error).at_frame(u64::from(index)))?;
            let mut conversion_limits = limits;
            conversion_limits.max_bytes = conversion_limits
                .max_bytes
                .checked_sub(prepared.stats().retained_bytes)
                .filter(|value| *value > 0)
                .ok_or_else(|| {
                    motion_core::Error::from(ValidationError::limit(
                        "linear.prepared",
                        "max_bytes",
                        "Prepared frame exhausts output allowance",
                    ))
                    .at_frame(u64::from(index))
                })?;
            let (frame, peak) = crate::FrameBuffer::from_constant(
                prepared.size(),
                prepared
                    .background()
                    .map_or([0.0; 4], |paint| paint.linear_premultiplied),
                format,
                conversion_limits,
            )
            .map_err(|error| motion_core::Error::from(error).at_frame(u64::from(index)))?;
            self.last_stats = Some(FrameStats {
                preparation: prepared.stats(),
                linear: crate::LinearRenderStats {
                    peak_bytes: prepared.stats().retained_bytes + peak,
                    work: pixels,
                },
                output: OutputStats {
                    peak_bytes: peak,
                    work: crate::FrameBuffer::conversion_work(frame.size(), format),
                },
                snapshot: OutputStats::default(),
            });
            if let Some(started) = started {
                self.timings.as_mut().expect("enabled timings").output += started.elapsed();
                self.timings.as_mut().expect("enabled timings").fused_output = true;
            }
            return Ok(frame);
        }
        #[cfg(feature = "gpu")]
        if !cpu
            && format == (crate::PixelFormat::Yuv420 { bit_depth: 10 })
            && crate::gpu::Gpu::supports_packed_output(prepared.size(), format)
        {
            let started = self.timings.map(|_| std::time::Instant::now());
            let plan = match crate::gpu::Plan::new_packed(&prepared, limits, format) {
                Ok(plan) => Some(plan.with_display(&display, u64::from(index))?),
                Err(error)
                    if self.backend.is_none()
                        && matches!(
                            self.composition.options.backend,
                            crate::BackendPreference::Automatic { .. }
                        ) =>
                {
                    self.backend = Some(crate::BackendDecision {
                        renderer: Renderer::Cpu,
                        reason: Some(error.to_string()),
                    });
                    None
                }
                Err(error) => {
                    return Err(motion_core::Error::from(error).at_frame(u64::from(index)));
                }
            };
            if let Some(started) = started {
                self.timings.as_mut().expect("enabled timings").plan += started.elapsed();
            }
            if let Some(plan) = plan
                && self.select_backend()? == Renderer::Gpu
            {
                let stats = plan.packed_stats(prepared.size(), format)?;
                let output_work = crate::FrameBuffer::display_conversion_work(
                    prepared.size(),
                    format,
                    Some(&display),
                );
                let frame = self
                    .gpu
                    .as_mut()
                    .expect("selected native GPU")
                    .render_output(&prepared, plan, format)
                    .map_err(|error| error.at_frame(u64::from(index)))?;
                self.record_gpu_timings();
                self.last_stats = Some(FrameStats {
                    preparation: prepared.stats(),
                    linear: crate::LinearRenderStats {
                        peak_bytes: stats.peak_bytes,
                        work: stats.work - output_work,
                    },
                    output: OutputStats {
                        peak_bytes: stats.peak_bytes,
                        work: output_work,
                    },
                    snapshot: OutputStats::default(),
                });
                return Ok(frame);
            }
        }
        let linear = self.render_prepared(index, prepared, limits)?;
        let started = self.timings.map(|_| std::time::Instant::now());
        let mut stats = self.last_stats.take().expect("successful linear stage");
        let linear_bytes = linear.pixels().len() as u64 * 16;
        let frame = remaining(self.limits, stats.preparation.work + stats.linear.work)
            .and_then(|limits| {
                crate::FrameBuffer::from_linear_display(
                    &linear,
                    format,
                    limits,
                    &display,
                    u64::from(index),
                )
            })
            .map_err(|error| motion_core::Error::from(error).at_frame(u64::from(index)));
        linear.recycle(&mut self.cpu_pool);
        let frame = frame?;
        stats.output = OutputStats {
            peak_bytes: linear_bytes
                + std::mem::size_of::<crate::LinearFrame>() as u64
                + frame.retained_bytes(),
            work: crate::FrameBuffer::display_conversion_work(frame.size(), format, Some(&display)),
        };
        self.last_stats = Some(stats);
        if let Some(started) = started {
            self.timings.as_mut().expect("enabled timings").output += started.elapsed();
        }
        Ok(frame)
    }
}

#[cfg(test)]
mod scene_output_tests {
    use super::*;
    use motion_core::Scene;

    #[test]
    fn opt_in_timings_preserve_output_work_and_reset_attempts() {
        let mut renderer = Composition::new(8, 8, crate::FrameClock::new(30, 150))
            .frame_renderer(Size::new(8.0, 8.0))
            .unwrap();
        let build = |_: &SceneContext<'_>| {
            Node::from(motion_core::Rect::new(0.0, 0.0, 4.0, 8.0).fill(Color::WHITE))
        };
        let format = crate::PixelFormat::SrgbRgba8 {
            premultiplied: false,
        };
        let cold = renderer
            .render_output_at(Time::ZERO, None, format, build)
            .unwrap();
        let work = renderer.last_stats().unwrap().linear.work;
        assert!(renderer.timings().is_none());
        renderer.enable_timings(true);
        let observed = renderer
            .render_output_at(Time::ZERO, None, format, build)
            .unwrap();
        assert_eq!(cold, observed);
        assert_eq!(renderer.last_stats().unwrap().linear.work, work);
        assert!(renderer.timings().is_some());
        assert!(renderer.timings().unwrap().fused_output);
        #[cfg(feature = "gpu")]
        assert!(renderer.timings().unwrap().gpu.is_none());
        renderer.enable_timings(false);
        assert!(renderer.timings().is_none());
    }

    #[test]
    #[cfg(feature = "gpu")]
    #[ignore = "requires native GPU; verifies frame-stage observations without latency assertions"]
    fn native_frame_stage_timings_preserve_output_and_logical_stats() -> motion_core::Result {
        let mut renderer = Composition::new(8, 8, crate::FrameClock::new(30, 2))
            .with_options(crate::RenderOptions::default().backend(Renderer::Gpu.into()))?
            .frame_renderer(Size::new(8.0, 8.0))?;
        let build = |_: &SceneContext<'_>| {
            Node::from(motion_core::Rect::new(0.0, 0.0, 4.0, 8.0).fill(Color::WHITE))
        };
        let expected = renderer.render_frame(0, build)?;
        let stats = *renderer.last_stats().unwrap();
        renderer.enable_timings(true);
        let observed = renderer.render_frame(0, build)?;
        assert_eq!(
            (observed.width(), observed.height()),
            (expected.width(), expected.height())
        );
        assert_eq!(observed.pixels(), expected.pixels());
        assert_eq!(*renderer.last_stats().unwrap(), stats);
        let supported = renderer
            .selected_gpu_context()?
            .unwrap()
            .supports_timestamps();
        assert_eq!(renderer.timings().unwrap().gpu.is_some(), supported);
        assert!(!renderer.timings().unwrap().fused_output);
        renderer.enable_timings(false);
        assert!(renderer.timings().is_none());
        assert_eq!(renderer.render_frame(0, build)?.pixels(), expected.pixels());
        Ok(())
    }

    #[test]
    fn memoized_hidden_scene_failures_retain_original_errors_on_later_frames() {
        use motion_core::NodeModifiers;
        let composition = Composition::new(8, 8, crate::FrameClock::new(30, 4))
            .with_options(crate::RenderOptions::default().allow_missing_assets(true))
            .unwrap();
        let mut renderer = composition.frame_renderer(Size::new(8.0, 8.0)).unwrap();
        let builds = std::cell::Cell::new(0);
        for index in [0, 1, 0] {
            let error = renderer
                .render_frame(index, |context| {
                    context
                        .memo("failed-node", || {
                            builds.set(builds.get() + 1);
                            motion_core::scene(|_| -> motion_core::Result<Node> {
                                Err(std::io::Error::other("memoized scene failure").into())
                            })
                            .render(context)
                        })
                        .opacity(0.0)
                })
                .err()
                .unwrap();
            assert_eq!(error.kind(), motion_core::ErrorKind::Io);
            assert_eq!(error.frame(), Some(u64::from(index)));
            assert_eq!(error.build_errors().len(), 1);
            assert!(
                error.build_errors()[0]
                    .downcast_ref::<std::io::Error>()
                    .is_some()
            );
        }
        assert_eq!(builds.get(), 1);
    }

    #[test]
    fn recoverable_scene_errors_retain_frames_and_do_not_poison_session_retries() {
        let composition = Composition::new(8, 8, crate::FrameClock::new(30, 4))
            .with_options(crate::RenderOptions::default().allow_missing_assets(true))
            .unwrap();
        let mut renderer = composition.frame_renderer(Size::new(8.0, 8.0)).unwrap();
        let failed = motion_core::scene(|_| -> motion_core::Result<Node> {
            Err(std::io::Error::other("scene I/O failure").into())
        });
        let error = renderer
            .render_frame(2, |context| failed.render(context))
            .err()
            .unwrap();
        assert_eq!(error.kind(), motion_core::ErrorKind::Io);
        assert_eq!(error.frame(), Some(2));
        assert!(
            error.build_errors()[0]
                .downcast_ref::<std::io::Error>()
                .is_some()
        );
        assert!(renderer.last_stats().is_none());
        let success = motion_core::scene(|context| context.bounds().fill(Color::WHITE));
        renderer
            .render_frame(3, |context| success.render(context))
            .unwrap();
        assert!(renderer.last_stats().is_some());
    }
}
