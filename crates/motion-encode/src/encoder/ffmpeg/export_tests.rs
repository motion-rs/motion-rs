//! Unified export regressions for timing, audio and transactional completion.

use crate::*;
use motion_core::Result;
use motion_core::Shape;
#[cfg(feature = "ffmpeg")]
use motion_core::{
    AssetStatus, Assets, AudioClip, Color, MotionBlur, NodeModifiers, Rect, ValidationError,
};
use motion_core::{
    ErrorKind, FrameRate, Node, Scene, SceneContext, Size, Time, Timeline, TimelineBuilder,
};
#[cfg(feature = "ffmpeg")]
use motion_render::{BackendPreference, Composition, RenderContext};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
struct Progress {
    cancelled: AtomicBool,
    cancel_phase: Option<ExportPhase>,
    cancel_after: Option<u64>,
    phases: Mutex<Vec<ExportPhase>>,
    frames: Mutex<Vec<u64>>,
}

impl ProgressSink for Progress {
    fn phase(&self, phase: ExportPhase) {
        self.phases.lock().unwrap().push(phase);
        if self.cancel_phase == Some(phase) {
            self.cancelled.store(true, Ordering::Relaxed);
        }
    }

    fn frame_completed(&self, completed: u64, _: u64) {
        self.frames.lock().unwrap().push(completed);
        if self.cancel_after == Some(completed) {
            self.cancelled.store(true, Ordering::Relaxed);
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

fn builder(
    size: Size,
    rate: FrameRate,
    frames: u32,
    build: impl Fn(&SceneContext<'_>) -> Node + Send + Sync + 'static,
) -> TimelineBuilder {
    Timeline::new(size, rate)
        .track(|track| track.push(build.with_duration(Time::from_frame(i64::from(frames), rate))))
}

fn timeline(
    size: Size,
    rate: FrameRate,
    frames: u32,
    build: impl Fn(&SceneContext<'_>) -> Node + Send + Sync + 'static,
) -> Timeline {
    builder(size, rate, frames, build).build().unwrap()
}

#[test]
fn pre_cancelled_export_does_not_evaluate_or_create_output() -> Result {
    let directory = tempfile::tempdir()?;
    let progress = Progress::default();
    progress.cancelled.store(true, Ordering::Relaxed);
    let scene = timeline(Size::new(64.0, 64.0), FrameRate::from(30), 3, |_| {
        panic!("cancelled builder")
    });
    let error = export(
        &scene,
        &ExportOptions::new(directory.path().join("cancelled.mp4")).workers(1),
        &progress,
    )
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Cancelled);
    assert!(progress.phases.lock().unwrap().is_empty());
    assert!(progress.frames.lock().unwrap().is_empty());
    assert_eq!(directory.path().read_dir()?.count(), 0);
    Ok(())
}

#[test]
fn video_cancellation_and_no_clobber_precede_building() -> Result {
    let directory = tempfile::tempdir()?;
    let destination = directory.path().join("out.mp4");
    let scene = timeline(Size::new(8.0, 4.0), FrameRate::from(24), 12, |_| {
        panic!("rejected export builder")
    });
    let progress = Progress {
        cancel_phase: Some(ExportPhase::Preparing),
        ..Progress::default()
    };
    let error = export(
        &scene,
        &ExportOptions::new(&destination).workers(1),
        &progress,
    )
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Cancelled);
    assert_eq!(directory.path().read_dir()?.count(), 0);
    std::fs::write(&destination, b"existing")?;
    let error = export(
        &scene,
        &ExportOptions::new(&destination).workers(1),
        &NoProgress,
    )
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert_eq!(std::fs::read(destination)?, b"existing");
    Ok(())
}

#[cfg(feature = "ffmpeg")]
#[test]
fn audio_preparation_failure_preserves_destination_without_evaluating_scene() -> Result {
    let directory = tempfile::tempdir()?;
    let destination = directory.path().join("output.mp4");
    std::fs::write(&destination, b"existing")?;
    let source = directory.path().join("corrupt.wav");
    std::fs::write(&source, b"not an audio stream")?;
    let scene = builder(Size::new(64.0, 64.0), FrameRate::from(24), 24, |_| {
        panic!("audio preparation failure must precede scene evaluation")
    })
    .audio(AudioClip::file(&source))
    .build()?;
    assert!(
        export(
            &scene,
            &ExportOptions::new(&destination).workers(1).overwrite(true),
            &NoProgress,
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&destination)?, b"existing");
    assert_eq!(directory.path().read_dir()?.count(), 2);
    Ok(())
}

#[cfg(feature = "ffmpeg")]
#[test]
#[ignore = "requires MOTION_TEST_FONT and native FFmpeg/libx264; temporary output only"]
fn explicit_font_video_commit_cancel_and_failure() -> Result {
    use motion_content::{AssetsExt, FontFace, Text};
    use std::sync::Arc;

    let font = FontFace::from_bytes(std::fs::read(std::env::var("MOTION_TEST_FONT")?)?, 0)?;
    let text = Text::from_font("AV", &font, 16.0, Color::WHITE)?;
    let store = Assets::new(".")?;
    store.preload("title", Arc::new(font))?;
    let snapshot = Composition::new(64, 32, motion_render::FrameClock::new(30, 150))
        .with_options(motion_render::RenderOptions::default().background(Color::BLACK))?
        .render_frame(0, Size::new(64.0, 32.0), |_| Node::from(text.clone()))?;
    assert!(snapshot.pixels().iter().any(|pixel| *pixel != 0));
    assert!(snapshot.pixels().contains(&0));
    let directory = tempfile::tempdir()?;
    let destination = directory.path().join("text.mp4");
    let order = Arc::new(Mutex::new(Vec::new()));
    let observed = order.clone();
    let rate = FrameRate::from(24);
    let painted = text.clone();
    let scene = timeline(Size::new(64.0, 32.0), rate, 3, move |context| {
        assert_eq!(context.assets().font("title").status(), AssetStatus::Ready);
        observed.lock().unwrap().push(context.time());
        Node::from(painted.clone()).translate(context.seconds() * 24.0, 4.0)
    });
    let mut options = ExportOptions::new(&destination).workers(1);
    options.render.context = RenderContext::new().with_assets(store);
    let progress = Progress::default();
    assert_eq!(export(&scene, &options, &progress)?.frames, 3);
    assert_eq!(
        *order.lock().unwrap(),
        (0..3)
            .map(|index| Time::from_frame(index, rate))
            .collect::<Vec<_>>()
    );
    assert_eq!(*progress.frames.lock().unwrap(), [1, 2, 3]);
    assert_eq!(
        *progress.phases.lock().unwrap(),
        [
            ExportPhase::Preparing,
            ExportPhase::Rendering,
            ExportPhase::Finalizing,
            ExportPhase::Completed
        ]
    );
    let metadata = motion_ffmpeg::inspect::media(&destination)?;
    assert!(metadata.h264 && !metadata.aac);
    assert_eq!(
        (metadata.width, metadata.height, metadata.video_frames),
        (64, 32, 3)
    );
    assert_eq!(metadata.video_fps, 24.0);
    assert_eq!(motion_ffmpeg::inspect::video_frames(&destination, 3)?, 3);
    let original = std::fs::read(&destination)?;
    options.overwrite = true;
    for finalizing in [false, true] {
        let progress = Progress {
            cancel_phase: finalizing.then_some(ExportPhase::Finalizing),
            cancel_after: (!finalizing).then_some(1),
            ..Progress::default()
        };
        assert_eq!(
            export(&scene, &options, &progress).unwrap_err().kind(),
            ErrorKind::Cancelled
        );
        assert_eq!(std::fs::read(&destination)?, original);
        assert_eq!(directory.path().read_dir()?.count(), 1);
    }
    let failed = timeline(Size::new(64.0, 32.0), rate, 3, |_| {
        Node::empty().opacity(f64::NAN)
    });
    assert!(export(&failed, &options, &NoProgress).is_err());
    assert_eq!(std::fs::read(&destination)?, original);
    assert_eq!(directory.path().read_dir()?.count(), 1);
    Ok(())
}

#[cfg(feature = "ffmpeg")]
#[test]
#[ignore = "requires native FFmpeg/libx264; no fonts or window"]
fn cancelled_export_preserves_destination_and_removes_temporary_output() -> Result {
    let directory = tempfile::tempdir()?;
    let destination = directory.path().join("existing.mp4");
    std::fs::write(&destination, b"original video")?;
    let scene = timeline(Size::new(64.0, 64.0), FrameRate::from(30), 3, |_| {
        Node::empty()
    });
    for finalizing in [false, true] {
        let progress = Progress {
            cancel_phase: finalizing.then_some(ExportPhase::Finalizing),
            cancel_after: (!finalizing).then_some(1),
            ..Progress::default()
        };
        let error = export(
            &scene,
            &ExportOptions::new(&destination).workers(1).overwrite(true),
            &progress,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Cancelled);
        assert_eq!(std::fs::read(&destination)?, b"original video");
        assert_eq!(directory.path().read_dir()?.count(), 1);
    }
    Ok(())
}

#[cfg(feature = "ffmpeg")]
#[test]
#[ignore = "requires native FFmpeg/libx264; no fonts or window"]
fn global_shutter_video_encodes_sampled_frames_and_preserves_failed_destination() -> Result {
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    let directory = tempfile::tempdir()?;
    let path = directory.path().join("shutter.mp4");
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let rate = FrameRate::from(24);
    let scene = timeline(Size::new(8.0, 4.0), rate, 3, move |context| {
        observed.fetch_add(1, Ordering::Relaxed);
        assert!(context.time() >= Time::ZERO);
        Node::from(context.bounds().fill(Color::rgb(200, 50, 80)))
    });
    let mut options = ExportOptions::new(&path)
        .workers(1)
        .motion_blur(MotionBlur {
            shutter_angle: 180.0,
            samples: 4,
        });
    export(&scene, &options, &NoProgress)?;
    assert_eq!(calls.load(Ordering::Relaxed), 10);
    assert_eq!(motion_ffmpeg::inspect::video_frames(&path, 3)?, 3);
    let previous = std::fs::read(&path)?;
    let failed = timeline(Size::new(8.0, 4.0), rate, 3, move |context| {
        if context.time() > Time::from_frame(1, rate) {
            Node::empty().opacity(f64::NAN)
        } else {
            Node::from(context.bounds().fill(Color::WHITE))
        }
    });
    options.overwrite = true;
    assert!(export(&failed, &options, &NoProgress).is_err());
    assert_eq!(std::fs::read(&path)?, previous);
    assert_eq!(directory.path().read_dir()?.count(), 1);
    Ok(())
}

#[cfg(feature = "ffmpeg")]
fn fractional_video_acceptance(backend: BackendPreference) -> Result {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("tone.wav");
    crate::encoder::ffmpeg::native::tests::write_tone(&source, 440.0, 0.5)?;
    for (rate, expected_samples) in [
        (FrameRate::NTSC, 11_212_u64),
        (FrameRate::NTSC_FILM, 14_014),
        (FrameRate::NTSC_DOUBLE, 5606),
    ] {
        let destination = directory.path().join(format!("{}.mp4", rate.numerator()));
        let evaluated = std::sync::Arc::new(Mutex::new(Vec::new()));
        let observed = evaluated.clone();
        let scene = builder(Size::new(8.0, 4.0), rate, 7, move |context| {
            let index = context.time().to_frame(rate);
            assert_eq!(context.frame_rate(), rate);
            assert_eq!(context.time(), Time::from_frame(index, rate));
            observed.lock().unwrap().push(index);
            Rect::new(0.0, 0.0, 4.0, 4.0).fill(Color::WHITE).into()
        })
        .audio(AudioClip::file(&source).duration(Time::from_seconds(0.5)))
        .try_duration(Time::from_frame(7, rate))?
        .build()?;
        assert_eq!(
            export(
                &scene,
                &ExportOptions::new(&destination).workers(1).backend(backend),
                &NoProgress
            )?
            .frames,
            7
        );
        assert_eq!(*evaluated.lock().unwrap(), (0..7).collect::<Vec<i64>>());
        let metadata = motion_ffmpeg::inspect::media(&destination)?;
        assert!(metadata.h264 && metadata.aac);
        assert_eq!(
            metadata.video_fps,
            f64::from(rate.numerator()) / f64::from(rate.denominator())
        );
        assert_eq!(metadata.video_frames, 7);
        assert_eq!(motion_ffmpeg::inspect::video_frames(&destination, 7)?, 7);
        assert_eq!((metadata.sample_rate, metadata.channels), (48_000, 2));
        assert!(
            (metadata.audio_seconds - expected_samples as f64 / 48_000.0).abs() <= 1.0 / 48_000.0
        );
        let decoded =
            motion_ffmpeg::inspect::audio_samples(&destination, expected_samples as usize + 2048)?;
        assert!(
            (expected_samples as usize..=expected_samples as usize + 1024).contains(&decoded.len())
        );
        assert!(decoded.iter().flatten().all(|sample| sample.is_finite()));
        assert!(decoded.iter().flatten().any(|sample| sample.abs() > 0.01));
        assert!(!directory.path().read_dir()?.any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".motion-video-")
        }));
    }
    Ok(())
}

#[cfg(feature = "ffmpeg")]
#[test]
#[ignore = "requires native FFmpeg with libx264/AAC; no fonts or window"]
fn fractional_video_and_audio_keep_exact_rate_and_cleanup() -> Result {
    fractional_video_acceptance(BackendPreference::Cpu)
}

#[cfg(all(feature = "ffmpeg", feature = "gpu"))]
#[test]
#[ignore = "requires native GPU and FFmpeg with libx264/AAC; no fonts or window"]
fn gpu_fractional_video_and_audio_keep_exact_rate_and_cleanup() -> Result {
    fractional_video_acceptance(BackendPreference::gpu())
}

#[cfg(feature = "ffmpeg")]
#[test]
#[ignore = "requires native FFmpeg audio decoding; temporary output only"]
fn audio_source_cannot_be_replaced_by_destination_even_with_overwrite() -> Result {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("source.wav");
    crate::encoder::ffmpeg::native::tests::write_tone(&source, 440.0, 0.25)?;
    let original = std::fs::read(&source)?;
    let scene = builder(Size::new(64.0, 64.0), FrameRate::from(24), 24, |_| {
        panic!("audio source destination builder")
    })
    .audio(AudioClip::file(&source))
    .build()?;
    let error = export(
        &scene,
        &ExportOptions::new(&source).workers(1).overwrite(true),
        &NoProgress,
    )
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<ValidationError>().unwrap().field,
        "path"
    );
    assert_eq!(std::fs::read(source)?, original);
    assert_eq!(directory.path().read_dir()?.count(), 1);
    Ok(())
}

#[cfg(not(feature = "ffmpeg"))]
#[test]
fn video_missing_feature_does_not_evaluate_or_create_output() -> Result {
    let directory = tempfile::tempdir()?;
    let scene = timeline(Size::new(8.0, 4.0), FrameRate::from(24), 12, |_| {
        panic!("disabled encoder builder")
    });
    let error = export(
        &scene,
        &ExportOptions::new(directory.path().join("out.mp4")).workers(1),
        &NoProgress,
    )
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::Unsupported
    );
    assert_eq!(directory.path().read_dir()?.count(), 0);
    Ok(())
}
