//! GPU export planning and submission tests.

use super::*;
use motion_core::Scene;
use motion_core::Shape;
use motion_core::{Color, FrameRate, Node, NodeModifiers, Rect, Size, Time};
use motion_render::PixelFormat;
use std::sync::{Arc, Mutex};

#[test]
fn packed_gpu_output_supports_both_yuv_bit_depths() {
    for bit_depth in [8, 10] {
        assert!(motion_render::gpu::Gpu::supports_packed_output(
            motion_core::PixelSize::new(32, 16),
            PixelFormat::Yuv420 { bit_depth },
        ));
    }
    assert!(!motion_render::gpu::Gpu::supports_packed_output(
        motion_core::PixelSize::new(32, 16),
        PixelFormat::Yuv420 { bit_depth: 12 },
    ));
}

#[test]
#[ignore = "requires native GPU; ten-bit GPU export must agree with the CPU export within one code value"]
fn ten_bit_gpu_export_matches_cpu_export_within_one_code_value() -> Result<()> {
    struct Sink(Arc<Mutex<Vec<FrameBuffer>>>);

    impl Encoder for Sink {
        fn input_format(&self) -> PixelFormat {
            PixelFormat::Yuv420 { bit_depth: 10 }
        }

        fn open(
            &mut self,
            _: &ExportOptions,
            _: FrameRate,
            _: Option<&crate::MixedAudio>,
        ) -> Result<()> {
            Ok(())
        }

        fn write(&mut self, _: u64, frame: &FrameBuffer) -> Result<()> {
            self.0.lock().unwrap().push(frame.clone());
            Ok(())
        }

        fn finish(self: Box<Self>) -> Result<ExportReport> {
            Ok(ExportReport {
                path: "unused".into(),
                fonts: Vec::new(),
                frames: self.0.lock().unwrap().len() as u64,
                range: Time::ZERO..Time::from_frame(6, FrameRate::NTSC),
                elapsed_seconds: 0.0,
                bytes: 0,
            })
        }
    }

    let rate = FrameRate::NTSC;
    let timeline = Timeline::new(Size::new(32.0, 16.0), rate)
        .track(|track| {
            track.push(
                (move |context: &motion_core::SceneContext<'_>| {
                    Node::from(
                        Rect::new(0.25 + context.seconds() * 8.0, 0.5, 28.0, 12.0)
                            .fill(Color::rgb(190, 40, 125)),
                    )
                    .opacity(0.5)
                })
                .with_duration(motion_core::Duration::from_frame(6, rate)),
            )
        })
        .build()?;
    let context = motion_render::GpuContext::new()?;
    let mut outputs = Vec::new();
    for backend in [
        motion_render::BackendPreference::Cpu,
        motion_render::BackendPreference::gpu(),
    ] {
        let mut options = ExportOptions::new("unused-ten-bit-gpu-export")
            .backend(backend)
            .workers(2);
        options.render.limits.max_bytes = 16 * 1024 * 1024;
        options.render.gpu_context = Some(context.clone());
        let written = Arc::new(Mutex::new(Vec::new()));
        let sink = written.clone();
        let options = options.encoder(move || Box::new(Sink(sink.clone())));
        export(&timeline, &options, &NoProgress)?;
        outputs.push(std::mem::take(&mut *written.lock().unwrap()));
    }
    let (cpu, gpu) = (&outputs[0], &outputs[1]);
    assert_eq!(cpu.len(), 6);
    assert_eq!(gpu.len(), 6);
    let samples = |plane: &motion_render::PixelPlane| {
        plane
            .bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect::<Vec<_>>()
    };
    let mut high_codes = false;
    for (cpu, gpu) in cpu.iter().zip(gpu) {
        assert_eq!(cpu.format(), PixelFormat::Yuv420 { bit_depth: 10 });
        assert_eq!(gpu.format(), cpu.format());
        for (cpu, gpu) in cpu.planes().iter().zip(gpu.planes()) {
            let (cpu, gpu) = (samples(cpu), samples(gpu));
            assert_eq!(cpu.len(), gpu.len());
            for (cpu, gpu) in cpu.into_iter().zip(gpu) {
                assert!(cpu.abs_diff(gpu) <= 1, "{cpu} != {gpu}");
                high_codes |= cpu > 255;
            }
        }
    }
    assert!(
        high_codes,
        "ten-bit samples must use codes above 8-bit range"
    );
    Ok(())
}

#[test]
fn measured_gpu_schedule_ignores_configuration_ceiling_and_bounds_all_slots() {
    let mut options = ExportOptions::new("unused").workers(8);
    options.render.limits.max_bytes = 1024 * 1024 * 1024;
    let (workers, slots, preparation_bytes, queue_bytes) =
        schedule(&options, 100, 8 * 1024 * 1024, 64 * 1024 * 1024);
    assert_eq!(workers, 8);
    assert_eq!(slots, 3);
    assert_eq!(
        preparation_bytes * workers as u64 + queue_bytes,
        options.max_parallel_bytes
    );
    assert!(preparation_bytes >= 2 * 8 * 1024 * 1024);
    assert!(queue_bytes >= slots as u64 * 64 * 1024 * 1024);
    assert_eq!(
        schedule(&options, 2, 8 * 1024 * 1024, 64 * 1024 * 1024).0,
        2
    );
    options.max_parallel_bytes = 1;
    assert_eq!(schedule(&options, 100, 8, 64).0, 0);
    options.max_parallel_bytes = u64::MAX;
    assert_eq!(schedule(&options, 100, u64::MAX, u64::MAX).0, 0);
}

#[test]
#[ignore = "requires native GPU; an Automatic request that selects the GPU must use the pipeline"]
fn automatic_backend_selecting_the_gpu_uses_the_pipeline() -> Result<()> {
    struct Discard;

    impl Encoder for Discard {
        fn input_format(&self) -> PixelFormat {
            PixelFormat::Yuv420 { bit_depth: 8 }
        }

        fn open(
            &mut self,
            _: &ExportOptions,
            _: FrameRate,
            _: Option<&crate::MixedAudio>,
        ) -> Result<()> {
            Ok(())
        }

        fn write(&mut self, _: u64, _: &FrameBuffer) -> Result<()> {
            Ok(())
        }

        fn finish(self: Box<Self>) -> Result<ExportReport> {
            Ok(ExportReport {
                path: "unused".into(),
                fonts: Vec::new(),
                frames: 12,
                range: Time::ZERO..Time::from_frame(12, FrameRate::NTSC),
                elapsed_seconds: 0.0,
                bytes: 0,
            })
        }
    }
    let threads = Arc::new(Mutex::new(std::collections::HashSet::new()));
    let observed = threads.clone();
    let rate = FrameRate::NTSC;
    let timeline = Timeline::new(Size::new(32.0, 16.0), rate)
        .track(|track| {
            track.push(
                (move |context: &motion_core::SceneContext<'_>| {
                    observed.lock().unwrap().insert(std::thread::current().id());
                    Node::from(
                        Rect::new(0.25 + context.seconds() * 4.0, 0.5, 28.0, 12.0)
                            .fill(Color::rgb(190, 40, 125)),
                    )
                })
                .with_duration(motion_core::Duration::from_frame(12, rate)),
            )
        })
        .build()?;
    let mut options = ExportOptions::new("unused-automatic-output")
        .backend(motion_render::BackendPreference::automatic())
        .workers(2);
    options.render.gpu_context = Some(motion_render::GpuContext::new()?);
    export(
        &timeline,
        &options.encoder(|| Box::new(Discard)),
        &NoProgress,
    )?;
    // The caller plus two preparation workers; the serial loop would use one thread.
    assert_eq!(threads.lock().unwrap().len(), 3);
    Ok(())
}

#[test]
#[ignore = "requires native GPU; validates real preparation/packed submission/ordered encoder pipeline"]
fn gpu_export_pipeline_matches_linear_reference_and_cancels_transactionally() -> Result<()> {
    let threads = Arc::new(Mutex::new(std::collections::HashSet::new()));
    let observed = threads.clone();
    let rate = FrameRate::NTSC;
    let timeline = Timeline::new(Size::new(32.0, 16.0), rate)
        .track(|track| {
            track.push(
                (move |context: &motion_core::SceneContext<'_>| {
                    observed.lock().unwrap().insert(std::thread::current().id());
                    Node::from(
                        Rect::new(0.25 + context.seconds() * 4.0, 0.5, 28.0, 12.0)
                            .fill(Color::rgb(190, 40, 125)),
                    )
                    .opacity(0.5)
                })
                .with_duration(motion_core::Duration::from_frame(12, rate)),
            )
        })
        .build()?;
    let mut options = ExportOptions::new("unused-gpu-pipeline-output")
        .backend(motion_render::BackendPreference::gpu())
        .workers(2);
    options.render.limits.max_bytes = 16 * 1024 * 1024;
    let format = PixelFormat::Yuv420 { bit_depth: 8 };
    let context = motion_render::GpuContext::new()?;
    let mut gpu = motion_render::gpu::Gpu::from_context(context.clone());
    let mut expected = Vec::new();
    let mut preparation = motion_render::Composition::for_timeline(&timeline, &options.render)?
        .frame_renderer(timeline.size())?;
    for index in 0..12 {
        let (frame, limits) = preparation.prepare_output_at(
            Time::from_frame(index, rate),
            options.render.background,
            |context| timeline.render(context),
        )?;
        expected.push(
            gpu.render_linear(&frame, limits)?
                .to_format(format, options.render.limits)?,
        );
    }
    threads.lock().unwrap().clear();
    let written = Arc::new(Mutex::new(Vec::new()));

    struct Sink(Arc<Mutex<Vec<FrameBuffer>>>);

    impl Encoder for Sink {
        fn input_format(&self) -> PixelFormat {
            PixelFormat::Yuv420 { bit_depth: 8 }
        }

        fn open(
            &mut self,
            _: &ExportOptions,
            _: FrameRate,
            _: Option<&crate::MixedAudio>,
        ) -> Result<()> {
            Ok(())
        }

        fn write(&mut self, index: u64, frame: &FrameBuffer) -> Result<()> {
            let mut written = self.0.lock().unwrap();
            assert_eq!(index, written.len() as u64);
            written.push(frame.clone());
            Ok(())
        }

        fn finish(self: Box<Self>) -> Result<ExportReport> {
            Ok(ExportReport {
                path: "unused".into(),
                fonts: Vec::new(),
                frames: self.0.lock().unwrap().len() as u64,
                range: Time::ZERO..Time::from_frame(12, FrameRate::NTSC),
                elapsed_seconds: 0.0,
                bytes: 0,
            })
        }
    }
    options.render.gpu_context = Some(context);
    let sink = written.clone();
    let capturing = options
        .clone()
        .encoder(move || Box::new(Sink(sink.clone())));
    let report = export(&timeline, &capturing, &NoProgress)?;
    assert_eq!(report.frames, 12);
    assert_eq!(threads.lock().unwrap().len(), 3);
    let actual = written.lock().unwrap();
    for (actual, expected) in actual.iter().zip(&expected) {
        for (a, b) in actual.planes().iter().zip(expected.planes()) {
            assert!(
                a.bytes()
                    .iter()
                    .zip(b.bytes())
                    .all(|(a, b)| a.abs_diff(*b) <= 1)
            );
        }
    }
    drop(actual);

    let calls = Arc::new(std::array::from_fn::<_, 12, _>(|_| {
        std::sync::atomic::AtomicUsize::new(0)
    }));
    let observed = calls.clone();
    let changing = Timeline::new(Size::new(64.0, 64.0), rate)
        .track(|track| {
            track.push(
                (move |context: &motion_core::SceneContext<'_>| {
                    let index = context.time().to_frame(rate) as usize;
                    observed[index].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if index < 2 {
                        Node::empty()
                    } else {
                        Node::from(Rect::new(0.25, 0.5, 62.0, 62.0).fill(Color::WHITE))
                            .opacity(0.5)
                            .filter(motion_core::Filter::Blur { sigma: 9.0 })
                    }
                })
                .with_duration(motion_core::Duration::from_frame(12, rate)),
            )
        })
        .build()?;
    let mut changing_options = options.clone();
    changing_options.path = "unused-heavier-gpu-pipeline".into();
    let context = changing_options.render.gpu_context.clone().unwrap();
    let mut oracle = motion_render::Composition::for_timeline(&changing, &changing_options.render)?
        .frame_renderer(changing.size())?;
    let mut serial_queue =
        motion_render::gpu::FrameQueue::new(context, changing_options.render.limits.max_bytes)?;
    let mut expected = Vec::new();
    let mut first_peak = 0;
    let mut heavy_preparation_peak = 0;
    let mut heavy_planning_peak = 0;
    let mut preparation_peak = 0;
    for index in 0..12 {
        let (frame, limits) = oracle.prepare_output_at(
            Time::from_frame(index, rate),
            changing_options.render.background,
            |context| changing.render(context),
        )?;
        let plan = OutputPlan::new(&frame, limits, format)?;
        if index == 0 {
            first_peak = plan.peak_bytes()?;
            preparation_peak = frame.stats().peak_bytes.max(plan.planning_peak_bytes());
        }
        if index == 2 {
            heavy_preparation_peak = frame.stats().peak_bytes;
            heavy_planning_peak = plan.planning_peak_bytes();
        }
        serial_queue.submit_output(&frame, plan)?;
        expected.push(serial_queue.finish_next_output()?.unwrap());
    }
    changing_options.max_parallel_bytes = first_peak * 3 + preparation_peak * 4;
    let (workers, slots, preparation_bytes, _) =
        schedule(&changing_options, 11, preparation_peak, first_peak);
    assert_eq!((workers, slots), (2, 3));
    assert!(heavy_preparation_peak <= preparation_bytes);
    assert!(heavy_planning_peak > preparation_bytes);
    for count in calls.iter() {
        count.store(0, std::sync::atomic::Ordering::Relaxed);
    }
    let written = Arc::new(Mutex::new(Vec::new()));
    let sink = written.clone();
    let changing_options = changing_options.encoder(move || Box::new(Sink(sink.clone())));
    export(&changing, &changing_options, &NoProgress)?;
    assert_eq!(*written.lock().unwrap(), expected);
    assert_eq!(calls[0].load(std::sync::atomic::Ordering::Relaxed), 1);
    assert!(calls[2].load(std::sync::atomic::Ordering::Relaxed) >= 2);

    use std::sync::atomic::{AtomicBool, Ordering};

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
    let cancelled_frames = Arc::new(Mutex::new(Vec::new()));
    let sink = cancelled_frames.clone();
    let capturing = options
        .clone()
        .encoder(move || Box::new(Sink(sink.clone())));
    let error = export(&timeline, &capturing, &Cancel(AtomicBool::new(false))).unwrap_err();
    assert_eq!(error.kind(), motion_core::ErrorKind::Cancelled);
    assert_eq!(cancelled_frames.lock().unwrap().len(), 2);
    let directory = tempfile::tempdir()?;
    options.path = directory.path().join("pipeline.mp4");
    export(&timeline, &options, &NoProgress)?;
    let committed = std::fs::read(&options.path)?;
    options.overwrite = true;
    assert_eq!(
        export(&timeline, &options, &Cancel(AtomicBool::new(false)))
            .unwrap_err()
            .kind(),
        motion_core::ErrorKind::Cancelled
    );
    assert_eq!(std::fs::read(&options.path)?, committed);
    let failing = Timeline::new(Size::new(32.0, 16.0), rate)
        .track(|track| {
            track.push(
                (move |context: &motion_core::SceneContext<'_>| {
                    assert!(
                        context.time() != Time::from_frame(3, rate),
                        "pipeline scene panic"
                    );
                    Node::empty()
                })
                .with_duration(motion_core::Duration::from_frame(12, rate)),
            )
        })
        .build()?;
    let error = export(&failing, &options, &NoProgress).unwrap_err();
    assert_eq!(error.kind(), motion_core::ErrorKind::ScenePanic);
    assert_eq!(error.frame(), Some(3));
    assert_eq!(std::fs::read(&options.path)?, committed);
    assert_eq!(directory.path().read_dir()?.count(), 1);
    Ok(())
}
