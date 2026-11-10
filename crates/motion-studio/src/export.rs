//! Background export jobs started from the studio: export form state, output formats,
//! progress reporting and cancellation.

use super::StudioScene;
use crate::AudioOptions;
use eframe::egui::Context;
use motion_encode::ProgressSink;
use std::{
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
};

#[derive(Clone, Debug)]
pub(crate) enum JobState {
    Running(Option<JobProgress>),
    Finished(Result<(), Arc<motion_core::Error>>),
}

pub(crate) struct ExportJob {
    control: Cancellation,
    state: Arc<Mutex<JobState>>,
    pub label: String,
    pub path: PathBuf,
    pub started: std::time::Instant,
}

impl ExportJob {
    #[cfg(test)]
    pub fn start(
        scene: StudioScene,
        path: PathBuf,
        draft: ExportForm,
        audio: AudioOptions,
        context: Context,
    ) -> io::Result<Self> {
        Self::start_with_codec(
            scene,
            path,
            draft,
            audio,
            context,
            motion_encode::Codec::H264 {
                quality: draft.quality,
                preset: motion_encode::EncoderPreset::Medium,
            },
        )
    }

    pub fn start_with_codec(
        scene: StudioScene,
        path: PathBuf,
        draft: ExportForm,
        audio: AudioOptions,
        context: Context,
        codec: motion_encode::Codec,
    ) -> io::Result<Self> {
        codec
            .validate()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        if !scene.supports_video_export() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Scene is registered for preview only",
            ));
        }
        let control = Cancellation::default();
        let state = Arc::new(Mutex::new(JobState::Running(None)));
        let worker_control = control.clone();
        let worker_state = state.clone();
        let destination = path.clone();
        let label = scene.name.clone();
        thread::Builder::new()
            .name("motion-export".into())
            .spawn(move || {
                let label = scene.name.clone();
                let current_frame = std::sync::atomic::AtomicU64::new(u64::from(draft.range.0));
                let result = catch_unwind(AssertUnwindSafe(|| -> motion_core::Result<()> {
                    if worker_control.is_cancelled() {
                        return Err(motion_core::Error::Cancelled);
                    }
                    if !draft.png && draft.range.0 >= draft.range.1 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "Require In < Out",
                        )
                        .into());
                    }
                    let mut timeline = if let Some(timeline) = scene.definition.audio_timeline {
                        timeline
                    } else {
                        let rate = scene.composition.frame_rate();
                        let duration = motion_core::Time::from_frame(
                            i64::try_from(scene.composition.frame_count())
                                .map_err(|error| motion_core::Error::Parameters(Box::new(error)))?,
                            rate,
                        );
                        let build = scene.definition.build_scene;
                        let source = motion_core::authoring::Scene::with_duration(
                            move |context: &motion_core::SceneContext<'_>| build(context),
                            duration,
                        );
                        Arc::new(
                            motion_core::Timeline::try_new(scene.definition.design_size, rate)?
                                .try_track(|track| track.try_push(source))?
                                .build()?,
                        )
                    };
                    let silent = matches!(
                        codec,
                        motion_encode::Codec::Gif { .. } | motion_encode::Codec::PngSequence
                    );
                    if silent {
                        let visual = timeline.clone();
                        let duration = timeline.duration();
                        timeline = Arc::new(
                            motion_core::Timeline::try_new(timeline.size(), timeline.frame_rate())?
                                .try_track(|track| {
                                    track.try_push(motion_core::authoring::Scene::with_duration(
                                        move |context: &motion_core::SceneContext<'_>| {
                                            motion_core::authoring::Scene::render(
                                                visual.as_ref(),
                                                context,
                                            )
                                        },
                                        duration,
                                    ))
                                })?
                                .build()?,
                        );
                    }
                    if !draft.png && !silent {
                        let mut builder = timeline.to_builder();
                        for clip in audio.audio_clips() {
                            builder = builder.try_audio(clip.clone())?;
                        }
                        timeline = Arc::new(builder.build()?);
                    }
                    let mut options = motion_encode::ExportOptions::new(&destination);
                    options.render = scene.composition.studio_render_options(draft.backend);
                    options.render.quality = motion_core::RenderQuality::Final;
                    options.render.size = Some(draft.size);
                    options.render.motion_blur = if draft.motion_blur {
                        Some(motion_core::MotionBlur::default())
                    } else {
                        None
                    };
                    options.overwrite = draft.overwrite;
                    options.codec = codec;
                    if matches!(
                        codec,
                        motion_encode::Codec::ProRes4444
                            | motion_encode::Codec::PngSequence
                            | motion_encode::Codec::Gif { .. }
                            | motion_encode::Codec::Vp9 { alpha: true, .. }
                    ) {
                        options.render.background = None;
                    }
                    if draft.range.0 < draft.range.1 {
                        options.range = Some(
                            motion_core::Time::from_frame(
                                i64::from(draft.range.0),
                                timeline.frame_rate(),
                            )
                                ..motion_core::Time::from_frame(
                                    i64::from(draft.range.1),
                                    timeline.frame_rate(),
                                ),
                        );
                    }
                    let sink = StudioProgress {
                        control: &worker_control,
                        state: &worker_state,
                        context: &context,
                        current_frame: &current_frame,
                    };
                    if draft.png {
                        if sink.is_cancelled() {
                            return Err(motion_core::Error::Cancelled);
                        }
                        if !options.overwrite && destination.try_exists()? {
                            return Err(io::Error::new(
                                io::ErrorKind::AlreadyExists,
                                "PNG destination exists",
                            )
                            .into());
                        }
                        options.render.background = None;
                        let bitmap = motion_render::render_still(
                            &timeline,
                            motion_core::Time::from_frame(
                                i64::from(draft.frame),
                                timeline.frame_rate(),
                            ),
                            &options.render,
                        )?;
                        if sink.is_cancelled() {
                            return Err(motion_core::Error::Cancelled);
                        }
                        bitmap.save_png_with_overwrite(&destination, options.overwrite)?;
                    } else {
                        motion_encode::export::export(&timeline, &options, &sink)?;
                    }
                    Ok(())
                }))
                .unwrap_or_else(|payload| {
                    Err(motion_core::Error::scene_panic(
                        label,
                        current_frame.load(std::sync::atomic::Ordering::Relaxed),
                        payload,
                    ))
                })
                .map_err(|error| {
                    if error.kind() == motion_core::ErrorKind::Cancelled {
                        motion_core::Error::Cancelled
                    } else {
                        error
                    }
                })
                .map_err(Arc::new);
                *worker_state.lock().expect("export state") = JobState::Finished(result);
                context.request_repaint();
            })?;
        Ok(Self {
            control,
            state,
            label,
            path,
            started: std::time::Instant::now(),
        })
    }

    pub fn state(&self) -> JobState {
        self.state.lock().expect("export state").clone()
    }

    pub fn cancel(&self) {
        self.control.cancel();
    }

    pub fn cancelling(&self) -> bool {
        self.control.is_cancelled()
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ExportForm {
    pub size: motion_core::PixelSize,
    pub range: (u32, u32),
    pub backend: motion_render::BackendPreference,
    pub quality: u8,
    pub overwrite: bool,
    pub motion_blur: bool,
    pub png: bool,
    pub frame: u32,
}

pub(crate) struct QueuedExport {
    pub path: PathBuf,
    pub draft: ExportForm,
    pub audio: AudioOptions,
    pub codec: motion_encode::Codec,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ExportFormat {
    #[default]
    H264,
    H265,
    Vp9,
    ProRes,
    Sequence,
    Gif,
}

impl ExportFormat {
    pub const ALL: [Self; 6] = [
        Self::H264,
        Self::H265,
        Self::Vp9,
        Self::ProRes,
        Self::Sequence,
        Self::Gif,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::H265 => "H.265",
            Self::Vp9 => "VP9 + alpha",
            Self::ProRes => "ProRes 4444",
            Self::Sequence => "PNG sequence (silent)",
            Self::Gif => "GIF (silent)",
        }
    }

    pub fn extension(self) -> Option<&'static str> {
        match self {
            Self::H264 | Self::H265 => Some("mp4"),
            Self::Vp9 => Some("webm"),
            Self::ProRes => Some("mov"),
            Self::Sequence => None,
            Self::Gif => Some("gif"),
        }
    }

    pub fn codec(self, quality: u8, rate: motion_core::FrameRate) -> motion_encode::Codec {
        match self {
            Self::H264 => motion_encode::Codec::H264 {
                quality,
                preset: motion_encode::EncoderPreset::Medium,
            },
            Self::H265 => motion_encode::Codec::H265 { quality },
            Self::Vp9 => motion_encode::Codec::Vp9 {
                quality,
                alpha: true,
            },
            Self::ProRes => motion_encode::Codec::ProRes4444,
            Self::Sequence => motion_encode::Codec::PngSequence,
            Self::Gif => motion_encode::Codec::Gif {
                frame_rate: if u64::from(rate.numerator()) > 100 * u64::from(rate.denominator()) {
                    motion_core::FrameRate::from(30)
                } else {
                    rate
                },
                colors: 256,
            },
        }
    }
}

struct StudioProgress<'a> {
    control: &'a Cancellation,
    state: &'a Mutex<JobState>,
    context: &'a Context,
    current_frame: &'a std::sync::atomic::AtomicU64,
}

impl motion_encode::ProgressSink for StudioProgress<'_> {
    fn phase(&self, phase: motion_encode::ExportPhase) {
        if phase == motion_encode::ExportPhase::Finalizing {
            let mut state = self.state.lock().expect("export progress");
            if let JobState::Running(Some(progress)) = &mut *state {
                progress.phase = motion_encode::ExportPhase::Finalizing;
            }
            self.context.request_repaint();
        }
    }

    fn is_cancelled(&self) -> bool {
        self.control.is_cancelled()
    }

    fn frame_completed(&self, completed: u64, total: u64) {
        self.current_frame
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *self.state.lock().expect("export progress") = JobState::Running(Some(JobProgress {
            completed_frames: completed,
            total_frames: total,
            phase: motion_encode::ExportPhase::Rendering,
        }));
        self.context.request_repaint();
    }
}

impl Drop for ExportJob {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Cancellation(Arc<std::sync::atomic::AtomicBool>);

impl Cancellation {
    pub(crate) fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct JobProgress {
    pub(crate) completed_frames: u64,
    pub(crate) total_frames: u64,
    pub(crate) phase: motion_encode::ExportPhase,
}
