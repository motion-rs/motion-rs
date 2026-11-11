//! Export pipeline acceptance and regression tests.

use super::*;
use motion_core::Shape;
use motion_core::{Color, FrameRate, Node, NodeModifiers, Rect, Scene, Size, Time, TimeLiteral};
use motion_render::PixelFormat;

#[cfg(feature = "ffmpeg")]
#[test]
#[ignore = "requires native libx265 and GPU; Main10 public export acceptance, no timings"]
fn native_main10_export_renders_display() -> motion_core::Result {
    use motion_core::{LinearGradient, Node, Point, Size, TimeLiteral};
    let directory = tempfile::tempdir()?;
    let timeline = motion_core::Timeline::new(Size::new(512.0, 16.0), 24)
        .track(|track| {
            track.scene(1.seconds(), |_| {
                Node::from(
                    motion_core::Rect::new(0.0, 0.0, 512.0, 16.0).fill(
                        LinearGradient::new(Point::ORIGIN, Point::new(1.0, 0.0))
                            .stop(0.0, motion_core::Color::BLACK)
                            .stop(1.0, motion_core::Color::WHITE),
                    ),
                )
            })
        })
        .build()?;
    let backends = [
        motion_render::BackendPreference::Cpu,
        #[cfg(feature = "gpu")]
        motion_render::BackendPreference::gpu(),
    ];
    for (index, backend) in backends.into_iter().enumerate() {
        let destination = directory.path().join(format!("main10-{index}.mp4"));
        let options = super::ExportOptions::new(&destination)
            .codec(super::Codec::H265 {
                quality: 0,
                preset: super::EncoderPreset::Medium,
                bit_depth: super::BitDepth::Ten,
                params: Some("lossless=1:log-level=error:pools=none:frame-threads=1".into()),
            })
            .backend(backend)
            .range(motion_core::Time::ZERO..motion_core::Time::from_frame(2, 24.into()))
            .render(|options| {
                options.display(motion_render::DisplayTransform {
                    tone_map: motion_render::ToneMap::AgX,
                    reference_white: Some(1.0),
                    transfer: motion_render::OutputTransfer::Gamma24,
                    ..motion_render::DisplayTransform::default()
                })
            });
        super::export(&timeline, &options, &super::NoProgress)?;
        let info = motion_ffmpeg::inspect::media(&destination)?;
        assert!(info.h265 && info.video_bit_depth == 10 && info.bt709_primaries_matrix_limited);
        assert_eq!(info.color_transfer, 1);
        let luma = motion_ffmpeg::inspect::video_luma10(&destination, 512 * 16 * 2)?;
        assert!(
            luma.into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                > 256
        );
    }
    Ok(())
}

#[test]
fn parallel_worker_selection_respects_frames_memory_and_backend() {
    let cpu = motion_render::BackendDecision {
        renderer: motion_render::Renderer::Cpu,
        reason: None,
    };
    let gpu = motion_render::BackendDecision {
        renderer: motion_render::Renderer::Gpu,
        reason: None,
    };
    let mut options = ExportOptions::new("unused")
        .size(motion_core::PixelSize::new(8, 4))
        .workers(8);
    options.render.limits.max_bytes = 1024 * 1024 * 1024;
    options.max_parallel_bytes = 3 * 2 * 1024;
    assert_eq!(render_workers(&options, 100, 1024, Some(&cpu)), 3);
    assert_eq!(render_workers(&options, 2, 1024, Some(&cpu)), 2);
    options.max_parallel_bytes = 1;
    assert_eq!(render_workers(&options, 100, 1024, Some(&cpu)), 1);
    options.max_parallel_bytes = u64::MAX;
    options.render.limits.max_bytes = u64::MAX;
    assert_eq!(render_workers(&options, 100, 1024, Some(&cpu)), 8);
    assert_eq!(render_workers(&options, 100, u64::MAX, Some(&cpu)), 1);
    options.render.backend = motion_render::BackendPreference::automatic();
    assert_eq!(render_workers(&options, 100, 1024, None), 1);
    assert_eq!(render_workers(&options, 100, 1024, Some(&cpu)), 8);
    assert_eq!(render_workers(&options, 100, 1024, Some(&gpu)), 1);
    let timeline = Timeline::new(Size::new(8.0, 4.0), 30)
        .track(|track| track.gap(1.seconds()))
        .build()
        .unwrap();
    for workers in [0, 65] {
        assert!(selected(&timeline, &options.clone().workers(workers)).is_err());
    }
}

#[test]
fn parallel_export_matches_serial_pixels_ntsc_ranges_and_progress() -> Result<()> {
    use std::sync::Mutex;
    let rate = FrameRate::NTSC;
    let evaluations = std::sync::Arc::new(Mutex::new(Vec::new()));
    let observed = evaluations.clone();
    let timeline = Timeline::try_new(Size::new(8.0, 4.0), rate)?
        .try_track(|track| {
            track.try_push(
                (move |context: &motion_core::SceneContext<'_>| {
                    observed
                        .lock()
                        .unwrap()
                        .push((context.time().to_frame(rate), std::thread::current().id()));
                    Node::from(
                        Rect::new(0.25 + context.seconds(), 0.5, 6.75, 3.5)
                            .fill(Color::rgb(190, 40, 125)),
                    )
                    .opacity(0.3 + context.seconds())
                })
                .with_duration(motion_core::Duration::from_frame(12, rate)),
            )
        })?
        .build()?;

    struct Sink<'a> {
        frames: &'a Mutex<Vec<FrameBuffer>>,
        format: PixelFormat,
    }

    impl Encoder for Sink<'_> {
        fn input_format(&self) -> PixelFormat {
            self.format
        }

        fn open(
            &mut self,
            _: &ExportOptions,
            actual_rate: FrameRate,
            _: Option<&crate::MixedAudio>,
        ) -> Result<()> {
            assert_eq!(actual_rate, FrameRate::NTSC);
            Ok(())
        }

        fn write(&mut self, index: u64, frame: &FrameBuffer) -> Result<()> {
            let mut frames = self.frames.lock().unwrap();
            assert_eq!(index, frames.len() as u64);
            frames.push(frame.clone());
            Ok(())
        }

        fn finish(self: Box<Self>) -> Result<ExportReport> {
            Ok(ExportReport {
                path: "unused".into(),
                frames: self.frames.lock().unwrap().len() as u64,
                range: Time::from_frame(2, FrameRate::NTSC)..Time::from_frame(10, FrameRate::NTSC),
                elapsed_seconds: 0.0,
                bytes: 0,
                fonts: Vec::new(),
            })
        }
    }

    struct Progress(Mutex<Vec<u64>>, std::thread::ThreadId);

    impl ProgressSink for Progress {
        fn frame_completed(&self, completed: u64, total: u64) {
            assert_eq!(std::thread::current().id(), self.1);
            assert_eq!(total, 8);
            self.0.lock().unwrap().push(completed);
        }
    }
    #[cfg(feature = "gpu")]
    let backends = [motion_render::BackendPreference::Cpu];
    #[cfg(not(feature = "gpu"))]
    let backends = [
        motion_render::BackendPreference::Cpu,
        motion_render::BackendPreference::automatic(),
    ];
    for format in [
        PixelFormat::SrgbRgba8 {
            premultiplied: false,
        },
        PixelFormat::SrgbRgba8 {
            premultiplied: true,
        },
        PixelFormat::LinearRgba16Float,
        PixelFormat::Yuv420 { bit_depth: 8 },
        PixelFormat::Yuv420 { bit_depth: 10 },
    ] {
        let mut reference = Vec::new();
        for backend in backends {
            for workers in [1, 2, 4, 16] {
                evaluations.lock().unwrap().clear();
                let frames = Mutex::new(Vec::new());
                let progress = Progress(Mutex::new(Vec::new()), std::thread::current().id());
                let mut options = ExportOptions::new("unused-parallel-custom-output")
                    .range(Time::from_frame(2, rate)..Time::from_frame(10, rate))
                    .workers(workers);
                options.render.backend = backend;
                options.render.limits.max_bytes = 1024 * 1024;
                if matches!(
                    format,
                    PixelFormat::SrgbRgba8 { .. } | PixelFormat::LinearRgba16Float
                ) {
                    options.render.background = None;
                }
                let report = export_with_encoder(
                    &timeline,
                    &options,
                    &progress,
                    Box::new(Sink {
                        frames: &frames,
                        format,
                    }),
                )?;
                assert_eq!(report.frames, 8);
                let frames = frames.into_inner().unwrap();
                if reference.is_empty() {
                    reference = frames;
                } else {
                    assert_eq!(frames, reference);
                }
                assert_eq!(*progress.0.lock().unwrap(), (1..=8).collect::<Vec<_>>());
                let evaluations = evaluations.lock().unwrap();
                assert_eq!(
                    evaluations.iter().filter(|(frame, _)| *frame == 2).count(),
                    1
                );
                if workers > 1 {
                    let threads = evaluations
                        .iter()
                        .filter(|(frame, _)| *frame > 2)
                        .map(|(_, thread)| *thread)
                        .collect::<std::collections::HashSet<_>>();
                    assert!(threads.len() > 1, "{backend:?} must use multiple workers");
                    assert!(!threads.contains(&progress.1));
                }
            }
        }
    }
    let calls = std::sync::Arc::new(std::array::from_fn::<_, 12, _>(|_| {
        std::sync::atomic::AtomicUsize::new(0)
    }));
    let observed = calls.clone();
    let changing = Timeline::new(motion_core::Size::new(64.0, 64.0), rate)
        .track(|track| {
            track.push(
                (move |context: &motion_core::SceneContext<'_>| {
                    let index = context.time().to_frame(rate) as usize;
                    observed[index].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if index < 4 {
                        Node::empty()
                    } else {
                        Node::from(Rect::new(0.25, 0.5, 62.0, 62.0).fill(Color::WHITE))
                            .filter(motion_core::Filter::Blur { sigma: 8.0 })
                            .opacity(0.5)
                    }
                })
                .with_duration(motion_core::Duration::from_frame(12, rate)),
            )
        })
        .build()?;
    let format = PixelFormat::SrgbRgba8 {
        premultiplied: false,
    };
    let mut options = ExportOptions::new("unused-later-large-frame")
        .range(Time::from_frame(2, rate)..Time::from_frame(10, rate))
        .workers(4);
    #[cfg(not(feature = "gpu"))]
    {
        options.render.backend = motion_render::BackendPreference::automatic();
    }
    let mut reference_renderer =
        composition(&changing, &options.render)?.frame_renderer(changing.size())?;
    render_frame(
        &mut reference_renderer,
        &changing,
        &options,
        rate,
        format,
        2,
    )?;
    let peak = reference_renderer.last_stats().unwrap().peak_bytes();
    options.max_parallel_bytes = peak * 2 * 4;
    assert_eq!(
        render_workers(&options, 7, peak, reference_renderer.backend_decision()),
        4
    );
    let reference = (2..10)
        .map(|index| {
            render_frame(
                &mut reference_renderer,
                &changing,
                &options,
                rate,
                format,
                index,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let heavy_peak = reference_renderer.last_stats().unwrap().peak_bytes();
    assert!(
        heavy_peak > peak * 2,
        "retry fixture must exceed the per-slot budget: first {peak}, heavy {heavy_peak}"
    );
    for count in calls.iter() {
        count.store(0, std::sync::atomic::Ordering::Relaxed);
    }
    let frames = Mutex::new(Vec::new());
    let progress = Progress(Mutex::new(Vec::new()), std::thread::current().id());
    export_with_encoder(
        &changing,
        &options,
        &progress,
        Box::new(Sink {
            frames: &frames,
            format,
        }),
    )?;
    assert_eq!(frames.into_inner().unwrap(), reference);
    assert_eq!(calls[2].load(std::sync::atomic::Ordering::Relaxed), 1);
    assert!(
        calls[4].load(std::sync::atomic::Ordering::Relaxed) >= 2,
        "first peak {peak}, heavy peak {heavy_peak}, heavy calls {}",
        calls[4].load(std::sync::atomic::Ordering::Relaxed)
    );
    assert_eq!(*progress.0.lock().unwrap(), (1..=8).collect::<Vec<_>>());
    Ok(())
}

#[test]
fn encoder_capabilities_validate_without_opening_resources() {
    let capabilities = EncoderCapabilities {
        supports_audio: false,
        max_size: Some(motion_core::PixelSize::new(16, 8)),
        dimension_multiple: 2,
    };
    assert!(
        capabilities
            .validate_size(motion_core::PixelSize::new(16, 8))
            .is_ok()
    );
    assert!(
        capabilities
            .validate_size(motion_core::PixelSize::new(15, 8))
            .is_err()
    );
    assert!(
        capabilities
            .validate_size(motion_core::PixelSize::new(18, 8))
            .is_err()
    );
    assert!(capabilities.validate_audio(false).is_ok());
    assert!(capabilities.validate_audio(true).is_err());
    assert!(
        EncoderCapabilities::default()
            .validate_size(motion_core::PixelSize::new(3, 5))
            .is_ok()
    );
    assert!(EncoderCapabilities::default().validate_audio(true).is_ok());
}

#[test]
fn still_and_custom_encoder_share_transparent_pixels_absolute_ticks_and_cancellation() -> Result<()>
{
    let timeline = Timeline::try_new(Size::new(4.0, 2.0), 10)?
        .try_track(|track| {
            track.try_push(
                (|context: &motion_core::SceneContext<'_>| {
                    Node::from(Rect::new(0.0, 0.0, 4.0, 2.0).fill(Color::rgb(255, 0, 0)))
                        .opacity(context.seconds())
                })
                .with_duration(1.seconds()),
            )
        })?
        .build()?;
    let bitmap = render_still(
        &timeline,
        250.milliseconds().after_origin(),
        &RenderOptions::default(),
    )?;
    assert_eq!(&bitmap.bytes()[..4], &[255, 0, 0, 64]);

    struct Sink;

    impl Encoder for Sink {
        fn input_format(&self) -> PixelFormat {
            PixelFormat::SrgbRgba8 {
                premultiplied: false,
            }
        }

        fn open(
            &mut self,
            options: &ExportOptions,
            _: FrameRate,
            _: Option<&crate::MixedAudio>,
        ) -> Result<()> {
            assert_eq!(
                options.range,
                Some(200.milliseconds().after_origin()..500.milliseconds().after_origin())
            );
            Ok(())
        }

        fn write(&mut self, index: u64, frame: &FrameBuffer) -> Result<()> {
            let alpha = frame.planes()[0].bytes()[3];
            assert_eq!(alpha, [51, 77, 102][index as usize]);
            Ok(())
        }

        fn finish(self: Box<Self>) -> Result<ExportReport> {
            Ok(ExportReport {
                path: "custom".into(),
                frames: 3,
                range: 200.milliseconds().after_origin()..500.milliseconds().after_origin(),
                elapsed_seconds: 0.0,
                bytes: 0,
                fonts: Vec::new(),
            })
        }
    }
    let mut options = ExportOptions::new("unused-custom-output");
    options.render.background = None;
    options.range = Some(150.milliseconds().after_origin()..450.milliseconds().after_origin());
    assert_eq!(
        export_with_encoder(&timeline, &options, &NoProgress, Box::new(Sink))?.frames,
        3
    );

    struct Cancel;

    impl ProgressSink for Cancel {
        fn is_cancelled(&self) -> bool {
            true
        }
    }
    assert_eq!(
        export(&timeline, &options, &Cancel).unwrap_err().kind(),
        motion_core::ErrorKind::Cancelled
    );
    Ok(())
}

#[cfg(all(unix, feature = "ffmpeg"))]
#[test]
#[ignore = "requires native FFmpeg with libx264; tests umask in isolated subprocesses"]
fn output_permissions_follow_umask_for_stills_sequences_gif_and_video() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    const CHILD: &str = "MOTION_TEST_OUTPUT_PERMISSIONS_CHILD";
    let Ok(mask) = std::env::var(CHILD) else {
        for mask in ["0022", "0077"] {
            let output = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg("umask \"$1\"; exec \"$2\" --exact export::tests::output_permissions_follow_umask_for_stills_sequences_gif_and_video --ignored --nocapture")
                .arg("motion-permissions")
                .arg(mask)
                .arg(std::env::current_exe()?)
                .env(CHILD, mask)
                .output()?;
            assert!(
                output.status.success(),
                "umask {mask}: stdout={} stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return Ok(());
    };
    let mask = u32::from_str_radix(&mask, 8).unwrap();
    let file_mode = 0o666 & !mask;
    let directory_mode = 0o777 & !mask;
    let directory = tempfile::tempdir()?;
    let timeline = Timeline::new(Size::new(8.0, 4.0), 1)
        .track(|track| {
            track.push(motion_core::Scene::with_duration(
                |_: &motion_core::SceneContext<'_>| Node::empty(),
                1.seconds(),
            ))
        })
        .build()?;
    let bitmap = render_still(&timeline, Time::ZERO, &RenderOptions::default())?;
    for overwrite in [false, true] {
        let still = directory.path().join("still.png");
        bitmap.save_png_with_overwrite(&still, overwrite)?;
        assert_eq!(
            std::fs::metadata(&still)?.permissions().mode() & 0o777,
            file_mode
        );
        for (name, codec) in [
            ("video.mp4", Codec::default()),
            (
                "animation.gif",
                Codec::Gif {
                    frame_rate: FrameRate::from(1),
                    colors: 16,
                },
            ),
            ("frames", Codec::PngSequence),
        ] {
            let path = directory.path().join(name);
            export(
                &timeline,
                &ExportOptions::new(&path)
                    .codec(codec.clone())
                    .overwrite(overwrite),
                &NoProgress,
            )?;
            if codec == Codec::PngSequence {
                assert_eq!(
                    std::fs::metadata(&path)?.permissions().mode() & 0o777,
                    directory_mode
                );
                assert_eq!(
                    std::fs::metadata(path.join("frame-000000.png"))?
                        .permissions()
                        .mode()
                        & 0o777,
                    file_mode
                );
            } else {
                assert_eq!(
                    std::fs::metadata(&path)?.permissions().mode() & 0o777,
                    file_mode
                );
            }
        }
    }
    Ok(())
}

#[cfg(feature = "ffmpeg")]
#[test]
#[ignore = "requires native libx264; compares serial and parallel committed files"]
fn parallel_native_exports_match_serial_files_and_preserve_destinations() -> Result<()> {
    use std::sync::atomic::{AtomicBool, Ordering};
    let directory = tempfile::tempdir()?;
    let rate = FrameRate::NTSC;
    let timeline = Timeline::try_new(Size::new(32.0, 16.0), rate)?
        .try_track(|track| {
            track.try_push(
                (|context: &motion_core::SceneContext<'_>| {
                    Node::from(
                        Rect::new(0.25 + context.seconds() * 8.0, 0.5, 24.0, 12.0)
                            .fill(Color::rgb(190, 40, 125)),
                    )
                    .opacity(0.3 + context.seconds() * 0.5)
                })
                .with_duration(motion_core::Duration::from_frame(24, rate)),
            )
        })?
        .build()?;
    let path = directory.path().join("serial.mp4");
    let mut options = ExportOptions::new(&path)
        .workers(1)
        .range(Time::from_frame(2, rate)..Time::from_frame(20, rate));
    options.render.limits.max_bytes = 1024 * 1024;
    export(&timeline, &options, &NoProgress)?;
    let reference = std::fs::read(&path)?;
    #[cfg(not(feature = "gpu"))]
    {
        options.render.backend = motion_render::BackendPreference::automatic();
    }
    for workers in [2, 4, 16] {
        options.path = directory.path().join(format!("workers-{workers}.mp4"));
        options.workers = workers;
        let report = export(&timeline, &options, &NoProgress)?;
        assert_eq!(report.frames, 18);
        assert_eq!(
            std::fs::read(&options.path)?,
            reference,
            "workers={workers}"
        );
    }

    struct Cancel(AtomicBool);

    impl ProgressSink for Cancel {
        fn frame_completed(&self, completed: u64, _: u64) {
            if completed == 2 {
                self.0.store(true, Ordering::Release);
            }
        }

        fn is_cancelled(&self) -> bool {
            self.0.load(Ordering::Acquire)
        }
    }
    options.overwrite = true;
    assert_eq!(
        export(&timeline, &options, &Cancel(AtomicBool::new(false)))
            .unwrap_err()
            .kind(),
        motion_core::ErrorKind::Cancelled
    );
    assert_eq!(std::fs::read(&options.path)?, reference);
    for panic_frame in [2, 3] {
        let failing = Timeline::try_new(Size::new(32.0, 16.0), rate)?
            .try_track(|track| {
                track.try_push(
                    (move |context: &motion_core::SceneContext<'_>| {
                        assert!(
                            context.time() != Time::from_frame(panic_frame, rate),
                            "scene panic"
                        );
                        Node::empty()
                    })
                    .with_duration(motion_core::Duration::from_frame(24, rate)),
                )
            })?
            .build()?;
        let error = export(&failing, &options, &NoProgress).unwrap_err();
        assert_eq!(error.kind(), motion_core::ErrorKind::ScenePanic);
        assert_eq!(error.frame(), Some(panic_frame as u64));
        assert_eq!(std::fs::read(&options.path)?, reference);
    }
    assert_eq!(directory.path().read_dir()?.count(), 4);
    Ok(())
}

#[cfg(feature = "ffmpeg")]
#[test]
#[ignore = "requires libx264/AAC; no devices, fonts or media executables"]
fn encoder_streams_yuv_crops_fractional_audio_and_preserves_destinations() -> Result<()> {
    use motion_core::AudioSource as _;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("range.mp4");
    let rate = FrameRate::NTSC;
    let duration = Time::from_frame(7, rate);
    let pcm = crate::PcmBuffer::new(
        48_000,
        1,
        (0..11212)
            .map(|index| 0.1 * libm::sinf(index as f32 * std::f32::consts::TAU * 1000.0 / 48000.0))
            .collect(),
    )?;
    let timeline = Timeline::try_new(Size::new(8.0, 4.0), rate)?
        .try_track(|track| {
            track.try_push(
                (|_: &motion_core::SceneContext<'_>| {
                    Node::from(Rect::new(0.0, 0.0, 8.0, 4.0).fill(Color::rgb(255, 0, 0)))
                        .opacity(0.5)
                })
                .with_duration(duration.since_origin()),
            )
        })?
        .try_audio(motion_core::AudioClip::source(pcm))?
        .build()?;
    let mut options = ExportOptions::new(&path);
    options.range = Some(Time::from_frame(1, rate)..Time::from_frame(4, rate));
    let report = export(&timeline, &options, &NoProgress)?;
    assert_eq!(report.frames, 3);
    assert!(report.bytes > 0);
    let media = motion_ffmpeg::inspect::media(&path)?;
    assert!(media.h264 && media.aac && media.bt709_primaries_matrix_limited);
    assert_eq!(media.color_transfer, 13);
    assert!(
        (media.video_fps - f64::from(rate.numerator()) / f64::from(rate.denominator())).abs()
            < 1e-10
    );
    assert_eq!(motion_ffmpeg::inspect::video_frames(&path, 3)?, 3);
    let decoded = crate::PcmBuffer::open(&path)?;
    assert_eq!(
        decoded.duration(),
        Some(motion_core::Duration::from_ticks(
            4805 * motion_core::TICKS_PER_SECOND / 48000
        ))
    );
    let mut samples = vec![0.0; 4805 * 2];
    assert_eq!(decoded.read(0, &mut samples), 4805);
    assert!(samples.iter().any(|sample| sample.abs() > 0.01));
    let committed = std::fs::read(&path)?;
    assert!(export(&timeline, &options, &NoProgress).is_err());

    struct Cancel(Arc<AtomicBool>);

    impl ProgressSink for Cancel {
        fn frame_completed(&self, _: u64, _: u64) {
            self.0.store(true, Ordering::SeqCst);
        }

        fn is_cancelled(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
    }
    options.overwrite = true;
    assert_eq!(
        export(
            &timeline,
            &options,
            &Cancel(Arc::new(AtomicBool::new(false)))
        )
        .unwrap_err()
        .kind(),
        motion_core::ErrorKind::Cancelled
    );
    options.render.background = None;
    let error = export(&timeline, &options, &NoProgress).unwrap_err();
    assert_eq!(error.frame(), Some(1));
    assert_eq!(std::fs::read(&path)?, committed);
    assert_eq!(std::fs::read_dir(directory.path())?.count(), 1);
    Ok(())
}
