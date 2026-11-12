//! One-frame render entry points and output conversion; scene preparation and backend
//! dispatch live in `preparation` and backend methods on [`FrameExecution`].
//!
//! CPU and GPU backends use only explicitly supplied fonts; nothing is discovered implicitly.

use super::*;
#[cfg(feature = "gpu")]
use crate::gpu::PreparedGpuFrame;
use crate::prepared::{FrameRecords, PreparedCpuFrame};
use crate::{BackendKind, RasterLimits, RenderContext, WorkStats};
use motion_core::{Color, Node, Result, SceneContext, Shape, Time, ValidationError};

#[cfg(feature = "gpu")]
mod gpu;
#[cfg(not(feature = "gpu"))]
#[path = "execution/no_gpu.rs"]
mod gpu;
mod preparation;

use preparation::remaining;

pub(in crate::composition) enum PreparedOutput {
    Cpu(PreparedCpuFrame),
    #[cfg(feature = "gpu")]
    Gpu(PreparedGpuFrame),
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

pub(super) struct FrameExecution<'a> {
    pub(super) composition: Composition,
    pub(super) duration: Time,
    pub(super) time: Option<Time>,
    pub(super) limits: RasterLimits,
    pub(super) device_packing: bool,
    pub(super) preparation_backend: Option<BackendKind>,
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
    ) -> Result<(PreparedOutput, RasterLimits)> {
        let rate = self.composition.clock.rate;
        let end = self.duration;
        if time < Time::ZERO || time >= end {
            self.last_stats = None;
            self.gpu_preparation = None;
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
        prepared: PreparedOutput,
        limits: RasterLimits,
        format: crate::PixelFormat,
    ) -> Result<crate::FrameBuffer> {
        self.output_pool.trim(limits.max_bytes / 8);
        let started = self.timings.map(|_| std::time::Instant::now());
        let cpu = self.backend_kind() == BackendKind::Cpu;
        let display = self.composition.options.display.clone();
        if cpu && (!prepared.items.is_empty() || !display.is_identity()) {
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
            });
            if let Some(started) = started {
                self.timings.as_mut().expect("enabled timings").output += started.elapsed();
                self.timings.as_mut().expect("enabled timings").fused_output = true;
            }
            return Ok(frame);
        }
        if prepared.items.is_empty() && cpu {
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
                linear: crate::WorkStats {
                    peak_bytes: prepared.stats().retained_bytes + peak,
                    work: pixels,
                },
                output: WorkStats {
                    peak_bytes: peak,
                    work: crate::FrameBuffer::conversion_work(frame.size(), format),
                },
            });
            if let Some(started) = started {
                self.timings.as_mut().expect("enabled timings").output += started.elapsed();
                self.timings.as_mut().expect("enabled timings").fused_output = true;
            }
            return Ok(frame);
        }
        if let Some(frame) = self.render_packed_output(index, &prepared, limits, format)? {
            return Ok(frame);
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
        self.cpu_pool.put(linear.into_pixels());
        let frame = frame?;
        stats.output = WorkStats {
            peak_bytes: linear_bytes
                + std::mem::size_of::<crate::linear::LinearFrame>() as u64
                + frame.retained_bytes(),
            work: crate::FrameBuffer::display_conversion_work(frame.size(), format, Some(&display)),
        };
        self.last_stats = Some(stats);
        if let Some(started) = started {
            self.timings.as_mut().expect("enabled timings").output += started.elapsed();
        }
        Ok(frame)
    }

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

#[cfg(test)]
mod scene_output_tests {
    use super::*;
    use motion_core::Scene;

    #[test]
    fn opt_in_timings_preserve_output_work_and_reset_attempts() {
        let mut renderer =
            Composition::new(8, 8, crate::FrameClock::new(motion_core::fps!(30), 150))
                .with_options(
                    crate::RenderOptions::default().backend(crate::BackendPreference::Cpu),
                )
                .unwrap()
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
        let mut renderer = Composition::new(8, 8, crate::FrameClock::new(motion_core::fps!(30), 2))
            .with_options(crate::RenderOptions::default().backend(crate::BackendPreference::gpu()))?
            .frame_renderer(Size::new(8.0, 8.0))?;
        let build = |_: &SceneContext<'_>| {
            Node::from(motion_core::Rect::new(0.0, 0.0, 4.0, 8.0).fill(Color::WHITE))
        };
        let expected = renderer.render_frame(0, build)?;
        let stats = *renderer.last_stats().unwrap();
        renderer.enable_timings(true);
        let observed = renderer.render_frame(0, build)?;
        assert_eq!(
            (
                observed.size().width as usize,
                observed.size().height as usize
            ),
            (
                expected.size().width as usize,
                expected.size().height as usize
            )
        );
        assert_eq!(observed.packed_rgb(), expected.packed_rgb());
        assert_eq!(*renderer.last_stats().unwrap(), stats);
        let supported = renderer
            .selected_gpu_context()?
            .unwrap()
            .supports_timestamps();
        assert_eq!(renderer.timings().unwrap().gpu.is_some(), supported);
        assert!(!renderer.timings().unwrap().fused_output);
        renderer.enable_timings(false);
        assert!(renderer.timings().is_none());
        assert_eq!(
            renderer.render_frame(0, build)?.packed_rgb(),
            expected.packed_rgb()
        );
        Ok(())
    }

    #[test]
    fn memoized_hidden_scene_failures_retain_original_errors_on_later_frames() {
        use motion_core::NodeModifiers;
        let composition = Composition::new(8, 8, crate::FrameClock::new(motion_core::fps!(30), 4))
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
            assert!(error.downcast_ref::<std::io::Error>().is_some());
        }
        assert_eq!(builds.get(), 1);
    }

    #[test]
    fn recoverable_scene_errors_retain_frames_and_do_not_poison_session_retries() {
        let composition = Composition::new(8, 8, crate::FrameClock::new(motion_core::fps!(30), 4))
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
        assert!(error.downcast_ref::<std::io::Error>().is_some());
        assert!(renderer.last_stats().is_none());
        let success = motion_core::scene(|context| context.bounds().fill(Color::WHITE));
        renderer
            .render_frame(3, |context| success.render(context))
            .unwrap();
        assert!(renderer.last_stats().is_some());
    }
}
