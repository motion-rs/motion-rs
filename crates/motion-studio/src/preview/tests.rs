use super::*;
use motion_content::{AssetsExt, FontFace, Image, Text};
use motion_core::Shape;

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires native GPU; validates preview backend switching without a window or fonts"]
fn preview_workers_honor_backend_settings_over_request_key() -> motion_core::Result<()> {
    let scene = StudioScene::from_render(
        "Backend selection",
        motion_render::Composition::new(
            16,
            8,
            motion_render::FrameClock::new(motion_core::fps!(30), 3),
        ),
        motion_core::Size::new(16.0, 8.0),
        |context| motion_core::Node::from(context.bounds().fill(motion_core::Color::WHITE)),
    )
    .unwrap();
    let (wake, updates) = std::sync::mpsc::channel();
    let context = Context::default();
    context.set_request_repaint_callback(move |_| {
        let _ = wake.send(());
    });
    let worker = PreviewCoordinator::with_slots(scene, context, 0, 1)?;
    let key = PreviewKey {
        frame: 0,
        renderer: BackendKind::Cpu,
    };
    for backend in [
        Some(motion_render::BackendPreference::gpu()),
        Some(motion_render::BackendPreference::Cpu),
        Some(motion_render::BackendPreference::gpu()),
        None,
    ] {
        worker.request_with_settings(
            key,
            PreviewSettings {
                backend,
                pixels: true,
                ..PreviewSettings::default()
            },
        );
        let reply = wait_for_preview(&worker, &updates, key);
        let image = reply.image?;
        assert_eq!(image.size, [16, 8]);
        assert!(
            image.pixels.iter().all(|pixel| *pixel == Color32::WHITE),
            "{:?}",
            image.pixels
        );
        let expected = if backend.is_some_and(|preference| preference.wants_gpu()) {
            "Gpu"
        } else {
            "Cpu"
        };
        assert!(reply.backend.starts_with(expected), "{}", reply.backend);
        let timings = reply.timings.unwrap();
        assert_eq!(timings.fused_output, expected == "Cpu");
    }
    Ok(())
}

#[test]
#[ignore = "requires native FFmpeg codecs; correctness only, no window or audio device"]
fn studio_all_codecs_export_and_retain_transactions() -> motion_core::Result<()> {
    let directory = tempfile::tempdir()?;
    let scene = StudioScene::from_render(
        "Codecs",
        motion_render::Composition::new(
            32,
            16,
            motion_render::FrameClock::new(motion_core::fps!(4), 2),
        ),
        motion_core::Size::new(32.0, 16.0),
        |context| {
            motion_core::Node::from(
                motion_core::Rect::new(0.0, 0.0, 16.0 + context.progress() * 16.0, 16.0)
                    .fill(motion_core::Color::WHITE),
            )
        },
    )
    .unwrap();
    let (wake, updates) = std::sync::mpsc::channel();
    let context = Context::default();
    context.set_request_repaint_callback(move |_| {
        let _ = wake.send(());
    });
    for format in crate::export::ExportFormat::ALL {
        let path = directory.path().join(format!(
            "{format:?}{}",
            format
                .extension()
                .map_or(String::new(), |extension| format!(".{extension}"))
        ));
        let draft = ExportForm {
            size: motion_core::PixelSize::new(32, 16),
            range: (0, 2),
            backend: motion_render::BackendPreference::Cpu,
            quality: 18,
            overwrite: false,
            motion_blur: false,
            png: false,
            frame: 0,
        };
        let codec = format.codec(18, motion_core::fps!(4));
        let job = ExportJob::start_with_codec(
            scene.clone(),
            path.clone(),
            draft,
            AudioOptions::default(),
            context.clone(),
            codec.clone(),
        )?;
        match wait_for_export(&job, &updates) {
            JobState::Finished(Ok(())) => {}
            JobState::Finished(Err(error)) => {
                return Err(motion_core::Error::asset(
                    "studio-codec",
                    format.label(),
                    error,
                ));
            }
            _ => panic!("unfinished export"),
        }
        assert!(path.exists());
        if format == crate::export::ExportFormat::Sequence {
            assert_eq!(std::fs::read_dir(&path)?.count(), 2);
        } else {
            assert!(!std::fs::read(&path)?.is_empty());
        }
        let before = if path.is_file() {
            Some(std::fs::read(&path)?)
        } else {
            None
        };
        let rejected = ExportJob::start_with_codec(
            scene.clone(),
            path.clone(),
            draft,
            AudioOptions::default(),
            context.clone(),
            codec,
        )?;
        assert!(matches!(
            wait_for_export(&rejected, &updates),
            JobState::Finished(Err(_))
        ));
        if let Some(before) = before {
            assert_eq!(std::fs::read(&path)?, before);
        }
    }
    Ok(())
}

fn write_tone(path: &std::path::Path, frequency: f64, seconds: f64) -> io::Result<()> {
    use std::io::Write;
    let count = (32_000.0 * seconds) as u32;
    let length = count * 2;
    let mut file = std::fs::File::create(path)?;
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + length).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16_u32.to_le_bytes())?;
    file.write_all(&1_u16.to_le_bytes())?;
    file.write_all(&1_u16.to_le_bytes())?;
    file.write_all(&32_000_u32.to_le_bytes())?;
    file.write_all(&64_000_u32.to_le_bytes())?;
    file.write_all(&2_u16.to_le_bytes())?;
    file.write_all(&16_u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&length.to_le_bytes())?;
    for index in 0..count {
        let sample = (4095.0
            * (std::f64::consts::TAU * frequency * f64::from(index) / 32_000.0).sin())
        .round() as i16;
        file.write_all(&sample.to_le_bytes())?;
    }
    Ok(())
}

fn start_export_job(
    scene: StudioScene,
    renderer: BackendKind,
    path: PathBuf,
    options: AudioOptions,
    context: Context,
) -> io::Result<ExportJob> {
    let draft = ExportForm {
        size: scene.composition.output_size(),
        range: (
            0,
            u32::try_from(scene.composition.frame_count()).map_err(io::Error::other)?,
        ),
        backend: if renderer == BackendKind::Gpu {
            motion_render::BackendPreference::gpu()
        } else {
            motion_render::BackendPreference::Cpu
        },
        quality: options.quality(),
        overwrite: options.overwrite_enabled(),
        motion_blur: false,
        png: false,
        frame: 0,
    };
    ExportJob::start(scene, path, draft, options, context)
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires native GPU and FFmpeg feature; no window, fonts or audio device"]
fn shared_texture_preview_and_automatic_export_loss_preserve_transactions()
-> motion_core::Result<()> {
    use motion_core::NodeModifiers;
    let context = crate::GpuContext::new()?;
    let scene = StudioScene::from_render(
        "Shared",
        motion_render::Composition::new(
            16,
            8,
            motion_render::FrameClock::new(motion_core::fps!(30), 3),
        )
        .with_options(motion_render::RenderOptions::default().gpu_context(context.clone()))?,
        motion_core::Size::new(16.0, 8.0),
        |_| {
            motion_core::Node::from(
                motion_core::Rect::new(0.0, 0.0, 8.0, 8.0).fill(motion_core::Color::WHITE),
            )
            .opacity(0.5)
        },
    )
    .unwrap();
    let (wake, updates) = std::sync::mpsc::channel();
    let egui = Context::default();
    egui.set_request_repaint_callback(move |_| {
        let _ = wake.send(());
    });
    let worker = PreviewCoordinator::new(scene, egui.clone())?;
    let key = PreviewKey {
        frame: 1,
        renderer: BackendKind::Gpu,
    };
    worker.request_with_settings(
        key,
        PreviewSettings {
            scale: 50,
            transparent: true,
            backend: Some(motion_render::BackendPreference::gpu()),
            ..Default::default()
        },
    );
    let reply = wait_for_preview(&worker, &updates, key);
    assert!(reply.texture.is_some());
    let image = reply.image?;
    assert_eq!(image.size, [8, 4]);
    assert!(image.pixels.iter().any(|pixel| pixel.a() == 0));
    assert!(image.pixels.iter().any(|pixel| pixel.a() == 128));
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("preserved.mp4");
    std::fs::write(&path, b"previous committed file")?;
    let loss = context.clone();
    let timeline = motion_core::Timeline::try_new(
        motion_core::Size::new(16.0, 8.0),
        motion_core::FrameRate::try_new(30, 1).unwrap(),
    )?
    .try_track(|track| {
        track.try_push(motion_core::authoring::Scene::with_duration(
            move |sample: &motion_core::SceneContext<'_>| {
                if sample.time().to_frame(sample.frame_rate()) == 1 {
                    loss.device().destroy();
                    let _ = loss.device().poll(wgpu::PollType::Wait {
                        submission_index: None,
                        timeout: Some(std::time::Duration::from_secs(60)),
                    });
                }
                motion_core::Node::from(sample.bounds().fill(motion_core::Color::WHITE))
            },
            motion_core::Duration::from_frame(3, motion_core::FrameRate::try_new(30, 1).unwrap()),
        ))
    })?
    .build()?;
    let mut options = motion_encode::ExportOptions::new(&path);
    options.overwrite = true;
    options.render.gpu_context = Some(context);
    options.render.backend = motion_render::BackendPreference::gpu();
    let error = motion_encode::export(&timeline, &options, &motion_encode::NoProgress)
        .expect_err("lost device must abort export");
    assert_eq!(error.kind(), motion_core::ErrorKind::DeviceLost);
    assert_eq!(std::fs::read(&path)?, b"previous committed file");
    assert_eq!(std::fs::read_dir(directory.path())?.count(), 1);
    Ok(())
}

#[test]
fn scaled_transparent_preview_is_rational_and_cached_without_devices() -> motion_core::Result<()> {
    use motion_core::NodeModifiers;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let scene = StudioScene::from_render(
        "Fractional",
        motion_render::Composition::new(
            16,
            8,
            motion_render::FrameClock::new(
                motion_core::FrameRate::try_new(30_000, 1001).unwrap(),
                6,
            ),
        ),
        motion_core::Size::new(16.0, 8.0),
        move |context| {
            count.fetch_add(1, Ordering::SeqCst);
            assert_eq!(
                context.time(),
                motion_core::Time::from_frame(2, context.frame_rate())
            );
            motion_core::Node::from(
                motion_core::Rect::new(0.0, 0.0, 8.0, 8.0).fill(motion_core::Color::WHITE),
            )
            .opacity(0.5)
        },
    )
    .unwrap();
    let (wake, updates) = std::sync::mpsc::channel();
    let context = Context::default();
    context.set_request_repaint_callback(move |_| {
        let _ = wake.send(());
    });
    let worker = PreviewCoordinator::new(scene, context)?;
    let key = PreviewKey {
        frame: 2,
        renderer: BackendKind::Cpu,
    };
    let settings = PreviewSettings {
        scale: 50,
        transparent: true,
        backend: Some(motion_render::BackendPreference::Cpu),
        ..Default::default()
    };
    for visit in 0..2 {
        worker.request_with_settings(key, settings);
        let reply = wait_for_preview(&worker, &updates, key);
        assert_eq!(reply.cache_hit, visit == 1);
        assert!(reply.cached_frames.contains(&2));
        assert!(worker.thumbnails().is_empty());
        assert_eq!(reply.stats.is_some(), visit == 0);
        assert_eq!(reply.timings.is_some(), visit == 0);
        let image = reply.image?;
        assert_eq!(image.size, [8, 4]);
        assert!(image.pixels.iter().any(|pixel| pixel.a() == 0));
        assert!(image.pixels.iter().any(|pixel| pixel.a() == 128));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    Ok(())
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires native GPU; validates automatic admission fallback, no window or fonts"]
fn automatic_preview_rejects_gpu_plan_without_rejecting_cpu_frame() -> motion_core::Result<()> {
    let context = crate::GpuContext::new()?;
    let limits = motion_core::RasterLimits {
        max_bytes: 512 * 1024,
        ..motion_core::RasterLimits::default()
    };
    let scene = StudioScene::from_render(
        "GPU admission",
        motion_render::Composition::new(
            64,
            64,
            motion_render::FrameClock::new(motion_core::fps!(30), 150),
        )
        .with_options(
            motion_render::RenderOptions::default()
                .gpu_context(context)
                .backend(motion_render::BackendPreference::gpu())
                .limits(limits),
        )?,
        motion_core::Size::new(64.0, 64.0),
        |sample| {
            motion_core::Node::group((0..48).map(|index| {
                motion_core::Node::from(
                    sample
                        .bounds()
                        .fill(motion_core::Color::rgb(index * 4, 120, 180).with_alpha(0.2)),
                )
            }))
        },
    )
    .unwrap();
    let settings = PreviewSettings {
        backend: Some(motion_render::BackendPreference::gpu()),
        ..PreviewSettings::default()
    };
    let mut automatic = PreviewRenderer::new(&scene, motion_render::BackendPreference::Cpu)?;
    let image = automatic.preview(&scene, 0, settings)?;
    assert!(image.texture.is_none());
    assert!(image.backend.starts_with("Cpu"), "{}", image.backend);
    assert!(
        image.backend.contains("gpu.") && image.backend.contains("max_bytes"),
        "{}",
        image.backend
    );
    let cpu_scene = StudioScene {
        composition: scene
            .composition
            .clone()
            .configure(|options| options.backend(motion_render::BackendPreference::Cpu)),
        ..scene.clone()
    };
    let expected = PreviewRenderer::new(&cpu_scene, motion_render::BackendPreference::Cpu)?
        .preview(&cpu_scene, 0, settings)?;
    assert_eq!(image.image.pixels, expected.image.pixels);
    assert_eq!(
        automatic.preview(&scene, 0, settings)?.image.pixels,
        expected.image.pixels
    );
    let mut inherited = PreviewRenderer::new(&scene, motion_render::BackendPreference::Cpu)?;
    assert_eq!(
        inherited.renderer.requested_backend(),
        motion_render::BackendPreference::gpu()
    );
    assert_eq!(
        inherited
            .preview(&scene, 0, PreviewSettings::default())?
            .image
            .pixels,
        expected.image.pixels
    );
    let timeline = scene.timeline.clone();
    let changing = StudioScene::from_render(
        "changing",
        scene
            .composition
            .clone()
            .with_clock(motion_render::FrameClock::new(
                scene.composition.frame_rate(),
                2,
            )),
        scene.timeline.size(),
        move |sample| {
            if sample.time() == motion_core::Time::ZERO {
                motion_core::Node::empty()
            } else {
                timeline.render(sample)
            }
        },
    )
    .unwrap();
    let mut selected = PreviewRenderer::new(&changing, motion_render::BackendPreference::Cpu)?;
    let first = selected.preview(&changing, 0, settings)?;
    assert!(first.texture.is_some(), "{}", first.backend);
    let error = selected.preview(&changing, 1, settings).err().unwrap();
    assert_eq!(
        error
            .downcast_ref::<motion_core::ValidationError>()
            .unwrap()
            .field,
        "max_bytes"
    );
    assert_eq!(selected.renderer.backend_kind(), BackendKind::Gpu);
    let explicit = StudioScene {
        composition: scene
            .composition
            .clone()
            .configure(|options| options.backend(motion_render::BackendPreference::gpu())),
        ..scene
    };
    let error = PreviewRenderer::new(&explicit, motion_render::BackendPreference::gpu())?
        .preview(
            &explicit,
            0,
            PreviewSettings {
                backend: Some(motion_render::BackendPreference::gpu()),
                ..settings
            },
        )
        .err()
        .unwrap();
    assert_eq!(
        error
            .downcast_ref::<motion_core::ValidationError>()
            .unwrap()
            .field,
        "max_bytes"
    );
    Ok(())
}

#[test]
fn progressive_preview_policy_keeps_presentation_out_of_render_identity() {
    let settings = PreviewSettings {
        scale: 75,
        ..PreviewSettings::default()
    };
    let fresh = settings.for_interaction(false, false, std::time::Duration::from_millis(149));
    assert_eq!(fresh.quality, motion_core::RenderQuality::Draft);
    assert_eq!(fresh.scale, 75);
    let settled = settings.for_interaction(false, false, std::time::Duration::from_millis(150));
    assert_eq!(settled.quality, motion_core::RenderQuality::Final);
    assert!(settled.transparent);
    let scrub = settings.for_interaction(false, true, std::time::Duration::from_secs(1));
    assert_eq!(
        (scrub.quality, scrub.scale),
        (motion_core::RenderQuality::Draft, 25)
    );
    assert_eq!(
        settings
            .for_interaction(true, false, std::time::Duration::from_secs(1))
            .quality,
        motion_core::RenderQuality::Draft
    );
    assert_eq!(
        settled,
        PreviewSettings {
            transparent: true,
            ..settings
        }
        .for_interaction(false, false, std::time::Duration::from_secs(1))
    );
}

#[test]
fn preview_cache_budget_parsing_and_shared_pixel_charge() {
    assert_eq!(preview_cache_bytes(None).unwrap(), 256 * 1024 * 1024);
    assert_eq!(preview_cache_bytes(Some("0")).unwrap(), 0);
    assert_eq!(
        preview_cache_bytes(Some("1024")).unwrap(),
        1024 * 1024 * 1024
    );
    for value in ["-1", "1025", "abc", ""] {
        assert_eq!(
            preview_cache_bytes(Some(value)).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
    let image = PreviewImage {
        backend: String::new(),
        image: Arc::new(ColorImage::filled([8, 4], Color32::WHITE)),
        nodes: Arc::from([]),
        stats: None,
        timings: None,
        #[cfg(feature = "gpu")]
        texture: None,
    };
    assert!(image.retained_bytes() > 128);
    assert!(Arc::ptr_eq(&image.image, &image.clone().image));
}

#[test]
fn preview_cache_retains_long_cycles_and_evicts_by_bytes() -> motion_core::Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let bytes = PreviewImage {
        backend: String::new(),
        image: Arc::new(ColorImage::filled([8, 4], Color32::WHITE)),
        nodes: Arc::from([]),
        stats: None,
        timings: None,
        #[cfg(feature = "gpu")]
        texture: None,
    }
    .retained_bytes()
        + 64;
    for (bytes, expected_calls) in [(0, 96), (bytes, 96), (48 * bytes, 48)] {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let scene = StudioScene::from_render(
            "cache",
            motion_render::Composition::new(
                8,
                4,
                motion_render::FrameClock::new(motion_core::fps!(30), 48),
            ),
            motion_core::Size::new(8.0, 4.0),
            move |context| {
                observed.fetch_add(1, Ordering::SeqCst);
                motion_core::Node::from(context.bounds().fill(motion_core::Color::WHITE))
            },
        )
        .unwrap();
        let (wake, updates) = std::sync::mpsc::channel();
        let context = Context::default();
        context.set_request_repaint_callback(move |_| {
            let _ = wake.send(());
        });
        let worker = PreviewCoordinator::with_cache_bytes(scene, context, bytes)?;
        let mut first = None;
        for cycle in 0..2 {
            for frame in 0..48 {
                let key = PreviewKey {
                    frame,
                    renderer: BackendKind::Cpu,
                };
                worker.request_with_settings(key, PreviewSettings::default());
                let image = wait_for_preview(&worker, &updates, key).image?;
                if frame == 0 {
                    if cycle == 0 {
                        first = Some(image);
                    } else if expected_calls == 48 {
                        assert!(Arc::ptr_eq(first.as_ref().unwrap(), &image));
                    }
                }
            }
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            expected_calls,
            "budget {bytes}"
        );
    }
    Ok(())
}

#[test]
fn unified_png_is_transactional_and_range_errors_do_not_evaluate() -> motion_core::Result<()> {
    let directory = tempfile::tempdir()?;
    let scene = StudioScene::from_render(
        "PNG",
        motion_render::Composition::new(
            8,
            4,
            motion_render::FrameClock::new(motion_core::fps!(30), 3),
        ),
        motion_core::Size::new(8.0, 4.0),
        |_| {
            motion_core::Node::from(
                motion_core::Rect::new(0.0, 0.0, 4.0, 4.0).fill(motion_core::Color::WHITE),
            )
        },
    )
    .unwrap();
    let (wake, updates) = std::sync::mpsc::channel();
    let context = Context::default();
    context.set_request_repaint_callback(move |_| {
        let _ = wake.send(());
    });
    let mut draft = ExportForm {
        size: motion_core::PixelSize::new(4, 2),
        range: (0, 3),
        backend: motion_render::BackendPreference::Cpu,
        quality: 18,
        overwrite: false,
        motion_blur: false,
        png: true,
        frame: 1,
    };
    let path = directory.path().join("output.png");
    let job = ExportJob::start(
        scene.clone(),
        path.clone(),
        draft,
        AudioOptions::default(),
        context.clone(),
    )?;
    assert!(matches!(
        wait_for_export(&job, &updates),
        JobState::Finished(Ok(()))
    ));
    let image = image::open(&path)
        .map_err(|error| motion_core::Error::Other(Box::new(error)))?
        .to_rgba8();
    assert_eq!(image.dimensions(), (4, 2));
    assert!(image.pixels().any(|pixel| pixel.0[3] == 0));
    draft.png = false;
    draft.range = (2, 1);
    let invalid = StudioScene::from_render(
        "invalid",
        scene.composition.clone(),
        motion_core::Size::new(8.0, 4.0),
        |_| panic!("invalid range must precede callback"),
    )
    .unwrap();
    let job = ExportJob::start(
        invalid,
        directory.path().join("bad.mp4"),
        draft,
        AudioOptions::default(),
        context,
    )?;
    let JobState::Finished(Err(error)) = wait_for_export(&job, &updates) else {
        panic!("range must fail");
    };
    assert_ne!(error.kind(), motion_core::ErrorKind::ScenePanic);
    assert!(!directory.path().join("bad.mp4").exists());
    Ok(())
}

#[test]
fn playback_prefetch_is_bounded_cached_and_never_published() -> motion_core::Result<()> {
    let (built, builds) = std::sync::mpsc::channel();
    let scene = StudioScene::from_render(
        "prefetch",
        motion_render::Composition::new(
            8,
            4,
            motion_render::FrameClock::new(motion_core::fps!(30), 20),
        ),
        motion_core::Size::new(8.0, 4.0),
        move |context| {
            built
                .send(context.time().to_frame(context.frame_rate()))
                .unwrap();
            motion_core::Node::from(context.bounds().fill(motion_core::Color::WHITE))
        },
    )
    .unwrap();
    let (wake, updates) = std::sync::mpsc::channel();
    let context = Context::default();
    context.set_request_repaint_callback(move |_| {
        let _ = wake.send(());
    });
    let worker = PreviewCoordinator::with_slots(scene, context, 4096, 2)?;
    let key = PreviewKey {
        frame: 0,
        renderer: BackendKind::Cpu,
    };
    worker.request_playback(key, PreviewSettings::default());
    wait_for_preview(&worker, &updates, key).image?;
    let mut prefetched: Vec<_> = (0..=4)
        .map(|_| {
            builds
                .recv_timeout(std::time::Duration::from_secs(30))
                .unwrap()
        })
        .collect();
    prefetched.sort_unstable();
    assert_eq!(prefetched, [0, 1, 2, 3, 4]);
    let next = PreviewKey { frame: 4, ..key };
    worker.request_with_settings(next, PreviewSettings::default());
    wait_for_preview(&worker, &updates, next).image?;
    assert!(builds.try_recv().is_err());
    assert!(worker.take_reply().is_none());
    worker.stop_prefetch();
    Ok(())
}

#[test]
fn playback_seek_preempts_inflight_prefetch_without_stale_reply() -> motion_core::Result<()> {
    let (entered, waiting) = std::sync::mpsc::channel();
    let (release, gate) = std::sync::mpsc::channel();
    let gate = Mutex::new(gate);
    let observed = Arc::new(Mutex::new(Vec::new()));
    let builds = observed.clone();
    let scene = StudioScene::from_render(
        "seek",
        motion_render::Composition::new(
            8,
            4,
            motion_render::FrameClock::new(motion_core::fps!(30), 30),
        ),
        motion_core::Size::new(8.0, 4.0),
        move |context| {
            let frame = context.time().to_frame(context.frame_rate());
            builds.lock().unwrap().push(frame);
            if frame == 1 {
                entered.send(()).unwrap();
                gate.lock().unwrap().recv().unwrap();
            }
            motion_core::Node::from(context.bounds().fill(motion_core::Color::WHITE))
        },
    )
    .unwrap();
    let (wake, updates) = std::sync::mpsc::channel();
    let context = Context::default();
    context.set_request_repaint_callback(move |_| {
        let _ = wake.send(());
    });
    let worker = PreviewCoordinator::with_slots(scene, context, 4096, 2)?;
    let key = PreviewKey {
        frame: 0,
        renderer: BackendKind::Cpu,
    };
    worker.request_playback(key, PreviewSettings::default());
    wait_for_preview(&worker, &updates, key).image?;
    waiting
        .recv_timeout(std::time::Duration::from_secs(30))
        .unwrap();
    worker.stop_prefetch();
    let seek = PreviewKey { frame: 20, ..key };
    worker.request_with_settings(seek, PreviewSettings::default());
    wait_for_preview(&worker, &updates, seek).image?;
    let frames = observed.lock().unwrap().clone();
    assert!(frames.contains(&0) && frames.contains(&1) && frames.contains(&20));
    assert!(frames.iter().all(|frame| *frame <= 4 || *frame == 20));
    assert_eq!(frames.iter().filter(|frame| **frame == 20).count(), 1);
    release.send(()).unwrap();
    assert!(worker.take_reply().is_none());
    Ok(())
}

#[test]
fn preview_missing_asset_keeps_magenta_frame_and_typed_diagnostic() -> motion_core::Result {
    let directory = tempfile::tempdir()?;
    let scene = StudioScene::from_render(
        "Resource failure",
        motion_render::Composition::new(
            8,
            8,
            motion_render::FrameClock::new(motion_core::fps!(30), 150),
        )
        .with_options(
            motion_render::RenderOptions::default()
                .resources(motion_core::Assets::new(directory.path())?),
        )?,
        motion_core::Size::new(8.0, 8.0),
        |_| motion_core::Node::from(Image::open("missing.png").size(8.0, 8.0)),
    )
    .unwrap();
    let (wake, updates) = std::sync::mpsc::channel();
    let context = Context::default();
    context.set_request_repaint_callback(move |_| {
        let _result = wake.send(());
    });
    let worker = PreviewCoordinator::new(scene, context)?;
    let key = PreviewKey {
        frame: 2,
        renderer: BackendKind::Cpu,
    };
    worker.request(key);
    let reply = wait_for_preview(&worker, &updates, key);
    assert!(
        reply
            .image?
            .pixels
            .iter()
            .all(|pixel| *pixel == Color32::from_rgb(255, 0, 255))
    );
    assert_eq!(reply.diagnostics.len(), 1);
    assert_eq!(reply.diagnostics[0].kind(), motion_core::ErrorKind::Asset);
    assert_eq!(reply.diagnostics[0].frame(), Some(2));
    assert!(reply.diagnostics[0].to_string().contains("missing.png"));
    Ok(())
}

#[test]
fn export_no_clobber_uses_branch_before_evaluation() -> motion_core::Result {
    let directory = tempfile::tempdir()?;
    let destination = directory.path().join("existing.mp4");
    std::fs::write(&destination, b"committed destination")?;
    let scene = StudioScene::from_render(
        "export",
        motion_render::Composition::new(
            8,
            4,
            motion_render::FrameClock::new(motion_core::fps!(30), 150),
        ),
        motion_core::Size::new(8.0, 4.0),
        |_| panic!("No-clobber must precede scene evaluation"),
    )
    .unwrap();
    let (wake, updates) = std::sync::mpsc::channel();
    let context = Context::default();
    context.set_request_repaint_callback(move |_| {
        let _result = wake.send(());
    });
    let job = start_export_job(
        scene,
        BackendKind::Cpu,
        destination.clone(),
        AudioOptions::default(),
        context,
    )?;
    let JobState::Finished(Err(error)) = wait_for_export(&job, &updates) else {
        panic!("Existing destination must be rejected");
    };
    assert_ne!(error.kind(), motion_core::ErrorKind::ScenePanic);
    assert!(
        error.to_string().to_ascii_lowercase().contains("exist"),
        "{error}"
    );
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(std::fs::read(&destination)?, b"committed destination");
    assert_eq!(std::fs::read_dir(directory.path())?.count(), 1);
    Ok(())
}

#[test]
#[ignore = "requires native FFmpeg/libx264/AAC; no audio device, fonts or window"]
fn timeline_worker_keeps_preview_silent_and_exports_its_mix() -> motion_core::Result {
    use motion_core::{AudioSource, TimeLiteral};
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    struct Tone(Arc<std::sync::atomic::AtomicUsize>);

    impl AudioSource for Tone {
        fn sample_rate(&self) -> u32 {
            48_000
        }

        fn channels(&self) -> u16 {
            1
        }

        fn duration(&self) -> Option<motion_core::Duration> {
            Some(50.milliseconds())
        }

        fn read(&self, start: u64, output: &mut [f32]) -> usize {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            output.fill(0.0);
            let count = 2400u64.saturating_sub(start).min(output.len() as u64) as usize;
            for (index, value) in output[..count].iter_mut().enumerate() {
                *value = (libm::sin(
                    std::f64::consts::TAU * 440.0 * (start + index as u64) as f64 / 48_000.0,
                ) * 0.25) as f32;
            }
            count
        }
    }
    let timeline =
        motion_core::Timeline::try_new(motion_core::Size::new(8.0, 4.0), motion_core::fps!(30))?
            .try_audio(motion_core::AudioClip::source(Tone(calls.clone())))?
            .try_duration(100.milliseconds())?
            .build()?;
    let scene = StudioScene::new(
        "sound",
        motion_render::Composition::new(
            8,
            4,
            motion_render::FrameClock::new(motion_core::fps!(30), 150),
        ),
        timeline,
    )?;
    let mut preview = PreviewRenderer::new(&scene, motion_render::BackendPreference::Cpu)?;
    preview.render_frame(&scene, 0)?;
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 0);
    let directory = tempfile::tempdir()?;
    let destination = directory.path().join("timeline.mp4");
    let (wake, updates) = std::sync::mpsc::channel();
    let context = Context::default();
    context.set_request_repaint_callback(move |_| {
        let _result = wake.send(());
    });
    let job = start_export_job(
        scene.clone(),
        BackendKind::Cpu,
        destination.clone(),
        AudioOptions::default(),
        context.clone(),
    )?;
    assert!(matches!(
        wait_for_export(&job, &updates),
        JobState::Finished(Ok(()))
    ));
    assert!(calls.load(std::sync::atomic::Ordering::Relaxed) > 0);
    let metadata = motion_ffmpeg::inspect::media(&destination)?;
    assert!(metadata.aac);
    assert_eq!(metadata.sample_rate, 48_000);
    assert_eq!(metadata.channels, 2);
    assert_eq!(motion_ffmpeg::inspect::video_frames(&destination, 3)?, 3);
    assert!(
        motion_ffmpeg::inspect::audio_samples(&destination, 8192)?
            .iter()
            .any(|sample| sample[0].abs() > 0.01)
    );
    calls.store(0, std::sync::atomic::Ordering::Relaxed);
    let refused = start_export_job(
        scene,
        BackendKind::Cpu,
        destination.clone(),
        AudioOptions::default(),
        context,
    )?;
    assert!(matches!(
        wait_for_export(&refused, &updates),
        JobState::Finished(Err(_))
    ));
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(directory.path().read_dir()?.count(), 1);
    println!("Studio timeline audio: preview_reads=0 AAC/stereo/3frames/no-clobber/cleanup=pass");
    Ok(())
}

#[test]
#[cfg(not(feature = "gpu"))]
fn export_missing_gpu_fails_before_evaluation_or_files() -> motion_core::Result {
    let directory = tempfile::tempdir()?;
    let destination = directory.path().join("native.mp4");
    let scene = StudioScene::from_render(
        "Native GPU unavailable",
        motion_render::Composition::new(
            8,
            4,
            motion_render::FrameClock::new(motion_core::fps!(30), 150),
        ),
        motion_core::Size::new(8.0, 4.0),
        |_| panic!("Missing GPU feature must precede scene evaluation"),
    )
    .unwrap();
    let (wake, updates) = std::sync::mpsc::channel();
    let context = Context::default();
    context.set_request_repaint_callback(move |_| {
        let _result = wake.send(());
    });
    let job = start_export_job(
        scene,
        BackendKind::Gpu,
        destination.clone(),
        AudioOptions::default(),
        context,
    )?;
    let JobState::Finished(Err(error)) = wait_for_export(&job, &updates) else {
        panic!("Missing GPU feature must reject export");
    };
    assert!(error.to_string().contains("gpu feature"), "{error}");
    assert!(!destination.exists());
    assert_eq!(std::fs::read_dir(directory.path())?.count(), 0);
    Ok(())
}

#[test]
fn preview_session_uses_linear_cpu_without_fonts() -> motion_core::Result {
    use motion_core::{Color, Node, NodeModifiers, Rect, Size};
    use motion_render::Composition;
    let scene = StudioScene::from_render(
        "geometry",
        Composition::new(
            8,
            4,
            motion_render::FrameClock::new(motion_core::fps!(30), 150),
        )
        .with_options(motion_render::RenderOptions::default().background(Color::BLACK))?,
        Size::new(8.0, 4.0),
        |_| Node::from(Rect::new(0.0, 0.0, 8.0, 4.0).fill(Color::WHITE)).opacity(0.5),
    )
    .unwrap();
    let mut session = PreviewRenderer::new(&scene, motion_render::BackendPreference::Cpu)?;
    let snapshot = session.render_frame(&scene, 0)?;
    assert_eq!(
        (
            snapshot.size().width as usize,
            snapshot.size().height as usize
        ),
        (8, 4)
    );
    assert_eq!(snapshot.packed_rgb()[0], 0xbbbbbb);
    let denied = StudioScene {
        composition: scene.composition.clone().configure(|options| {
            options.limits(motion_core::RasterLimits {
                max_bytes: 1,
                ..motion_core::RasterLimits::default()
            })
        }),
        ..scene.clone()
    };
    let mut restricted = PreviewRenderer::new(&denied, motion_render::BackendPreference::Cpu)?;
    let error = restricted
        .render_frame(&denied, 0)
        .expect_err("preview must inherit scene budget");
    assert_eq!(
        error
            .downcast_ref::<motion_core::ValidationError>()
            .unwrap()
            .field,
        "max_bytes"
    );
    assert_eq!(
        scene.composition.raster_limits().max_bytes,
        motion_core::RasterLimits::default().max_bytes
    );
    assert_eq!(
        session.render_frame(&scene, 0)?.packed_rgb(),
        snapshot.packed_rgb()
    );
    Ok(())
}

#[test]
fn preview_honors_global_shutter_and_replays_without_fonts() -> motion_core::Result {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = calls.clone();
    let composition = motion_render::Composition::new(
        8,
        4,
        motion_render::FrameClock::new(motion_core::fps!(24), 3),
    )
    .with_options(
        motion_render::RenderOptions::default()
            .background(motion_core::Color::BLACK)
            .motion_blur(motion_core::MotionBlur {
                shutter_angle: 180.0,
                samples: 4,
            }),
    )?;
    let scene = StudioScene::from_render(
        "shutter",
        composition,
        motion_core::Size::new(8.0, 4.0),
        move |context| {
            observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            motion_core::Node::from(
                context
                    .bounds()
                    .fill(motion_core::Color::WHITE.with_alpha(0.5)),
            )
        },
    )
    .unwrap();
    let mut renderer = PreviewRenderer::new(&scene, motion_render::BackendPreference::Cpu)?;
    let first = renderer.render_frame(&scene, 1)?;
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 4);
    assert_eq!(first.packed_rgb()[0], 0xbbbbbb);
    assert_eq!(
        first.packed_rgb(),
        renderer.render_frame(&scene, 1)?.packed_rgb()
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 8);
    Ok(())
}

pub(super) fn wait_for_preview(
    worker: &PreviewCoordinator,
    updates: &std::sync::mpsc::Receiver<()>,
    expected: PreviewKey,
) -> PreviewReply {
    let started = std::time::Instant::now();
    loop {
        if let Some(reply) = worker.take_reply() {
            assert_eq!(reply.key, expected);
            return reply;
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(30),
            "Preview worker timed out"
        );
        let _update = updates.recv_timeout(std::time::Duration::from_millis(20));
    }
}

fn worker_scene(builds: Arc<std::sync::atomic::AtomicUsize>) -> StudioScene {
    use motion_core::{Color, Node, NodeModifiers, Rect, Size};
    use motion_render::Composition;
    StudioScene::from_render(
        "worker",
        Composition::new(
            8,
            4,
            motion_render::FrameClock::new(motion_core::fps!(30), 8),
        )
        .with_options(motion_render::RenderOptions::default().background(Color::BLACK))
        .unwrap(),
        Size::new(8.0, 4.0),
        move |context| {
            let paint = context.cache("paint", || {
                builds.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Color::WHITE
            });
            let index = context.time().to_frame(context.frame_rate());
            assert_ne!(index, 3, "intentional preview panic");
            let left = if index == 2 { f64::NAN } else { index as f64 };
            Node::from(Rect::new(left, 0.0, left + 2.0, 4.0).fill(*paint)).opacity(0.5)
        },
    )
    .unwrap()
}

#[test]
fn preview_worker_seeks_and_recovers_with_session_cache() -> motion_core::Result {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let builds = Arc::new(AtomicUsize::new(0));
    let scene = worker_scene(builds.clone());
    let (wake, updates) = std::sync::mpsc::channel();
    let context = Context::default();
    context.set_request_repaint_callback(move |_| {
        let _result = wake.send(());
    });
    let worker = PreviewCoordinator::new(scene, context)?;
    let mut images = Vec::new();
    for frame in [4, 0, 6, 4] {
        worker.request(key(frame));
        images.push(
            wait_for_preview(&worker, &updates, key(frame))
                .image
                .map_err(io::Error::other)?,
        );
    }
    assert_eq!(images[0].size, [8, 4]);
    assert!(images[0].pixels == images[3].pixels);
    assert!(images[0].pixels != images[1].pixels);
    assert!(
        images[0]
            .pixels
            .iter()
            .any(|pixel| *pixel != Color32::BLACK)
    );
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    worker.request(key(2));
    let marked = wait_for_preview(&worker, &updates, key(2))
        .image
        .map_err(io::Error::other)?;
    assert!(
        marked
            .pixels
            .iter()
            .any(|pixel| pixel.r() > 100 && pixel.g() < pixel.r())
    );
    for frame in [8, 3] {
        worker.request(key(frame));
        let error = wait_for_preview(&worker, &updates, key(frame))
            .image
            .expect_err("invalid request must fail");
        if frame == 3 {
            assert_eq!(error.kind(), motion_core::ErrorKind::ScenePanic);
            assert_eq!(error.frame(), Some(3));
            assert!(error.to_string().contains("intentional preview panic"));
            assert!(error.to_string().contains("worker"));
        }
        worker.request(key(4));
        let recovered = wait_for_preview(&worker, &updates, key(4))
            .image
            .map_err(io::Error::other)?;
        assert!(recovered.pixels == images[0].pixels);
        assert_eq!(
            builds.load(Ordering::SeqCst),
            if frame == 3 { 2 } else { 1 }
        );
    }
    assert_eq!(builds.load(Ordering::SeqCst), 2);
    Ok(())
}

#[cfg(not(feature = "gpu"))]
#[test]
fn preview_worker_rejects_gpu_without_fallback_and_recovers() -> motion_core::Result {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let builds = Arc::new(AtomicUsize::new(0));
    let scene = worker_scene(builds.clone());
    let (wake, updates) = std::sync::mpsc::channel();
    let context = Context::default();
    context.set_request_repaint_callback(move |_| {
        let _result = wake.send(());
    });
    let worker = PreviewCoordinator::new(scene, context)?;
    let gpu = PreviewKey {
        renderer: BackendKind::Gpu,
        ..key(4)
    };
    worker.request(gpu);
    assert!(wait_for_preview(&worker, &updates, gpu).image.is_err());
    assert_eq!(builds.load(Ordering::SeqCst), 0);
    worker.request(key(4));
    assert!(wait_for_preview(&worker, &updates, key(4)).image.is_ok());
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    Ok(())
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires a native GPU; no window or video export"]
fn preview_worker_gpu_switching_matches_cpu() -> motion_core::Result {
    let scene = worker_scene(Arc::new(std::sync::atomic::AtomicUsize::new(0)));
    let mut reference = PreviewRenderer::new(&scene, motion_render::BackendPreference::Cpu)?;
    let supports_timestamps = crate::GpuContext::new()?.supports_timestamps();
    let (wake, updates) = std::sync::mpsc::channel();
    let context = Context::default();
    context.set_request_repaint_callback(move |_| {
        let _result = wake.send(());
    });
    let worker = PreviewCoordinator::new(scene.clone(), context)?;
    for (renderer, frame) in [
        (BackendKind::Gpu, 4),
        (BackendKind::Gpu, 0),
        (BackendKind::Cpu, 6),
        (BackendKind::Gpu, 6),
        (BackendKind::Gpu, 4),
    ] {
        let request = PreviewKey {
            renderer,
            ..key(frame)
        };
        worker.request(request);
        let reply = wait_for_preview(&worker, &updates, request);
        if let Some(timings) = reply.timings {
            assert_eq!(
                timings.gpu.is_some(),
                renderer == BackendKind::Gpu && supports_timestamps
            );
        }
        let actual = reply.image.map_err(io::Error::other)?;
        let expected = color_image(reference.render_frame(&scene, frame)?);
        assert_eq!(actual.size, expected.size);
        assert!(
            actual.pixels == expected.pixels,
            "worker differs at {request:?}"
        );
    }
    Ok(())
}

fn wait_for_export(job: &ExportJob, updates: &std::sync::mpsc::Receiver<()>) -> JobState {
    let started = std::time::Instant::now();
    loop {
        let state = job.state();
        if matches!(state, JobState::Finished(_)) {
            return state;
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(30),
            "Export worker timed out"
        );
        let _update = updates.recv_timeout(std::time::Duration::from_millis(20));
    }
}

fn worker_export_acceptance(renderer: BackendKind) -> motion_core::Result {
    use crate::audio::settings::{AudioForm, ClipSettings};
    use motion_core::{Assets, Color, Node, NodeModifiers, Rect, SceneContext, Size};
    use motion_render::{Composition, RenderContext};
    let font = std::env::var_os("MOTION_TEST_FONT").expect("explicit font path");
    let font = FontFace::from_bytes(std::fs::read(font)?, 0)?;
    let assets = Assets::new(".")?;
    assets.preload("label", Arc::new(font))?;
    let composition = Composition::new(
        96,
        48,
        motion_render::FrameClock::new(motion_core::fps!(24), 3),
    )
    .with_options(
        motion_render::RenderOptions::default()
            .background(Color::BLACK)
            .context(RenderContext::new().with_assets(assets))
            .limits(motion_core::RasterLimits {
                max_bytes: 64 * 1024 * 1024,
                ..motion_core::RasterLimits::default()
            }),
    )?;

    fn build(context: &SceneContext<'_>) -> Node {
        let font = context.assets().font("label").get().unwrap();
        Node::group([
            Node::from(Text::from_font("Studio native", &font, 14.0, Color::WHITE).unwrap())
                .translate(2.0 + context.seconds() * 24.0, 3.0)
                .opacity(0.75),
            Node::from(
                Rect::new(4.0, 32.0, 28.0 + context.seconds() * 48.0, 40.0)
                    .fill(Color::from_hex(0x147d84)),
            ),
        ])
    }
    let scene = StudioScene::from_render(
        "acceptance",
        composition.clone(),
        Size::new(96.0, 48.0),
        build,
    )
    .unwrap();
    let (wake, updates) = std::sync::mpsc::channel();
    let context = Context::default();
    context.set_request_repaint_callback(move |_| {
        let _result = wake.send(());
    });
    let preview = PreviewCoordinator::new(scene.clone(), context.clone())?;
    let mut reference = PreviewRenderer::new(&scene, motion_render::BackendPreference::Cpu)?;
    let mut replay = None;
    let mut maximum = 0;
    for frame in [2, 0, 1, 2] {
        let request = PreviewKey { frame, renderer };
        preview.request(request);
        let actual = wait_for_preview(&preview, &updates, request)
            .image
            .map_err(io::Error::other)?;
        let expected = color_image(reference.render_frame(&scene, frame)?);
        assert_eq!(actual.size, expected.size);
        for (actual, expected) in actual.pixels.iter().zip(&expected.pixels) {
            for (actual, expected) in actual.to_array().into_iter().zip(expected.to_array()) {
                maximum = maximum.max(actual.abs_diff(expected));
                assert!(actual.abs_diff(expected) <= 1);
            }
        }
        assert!(actual.pixels.iter().any(|pixel| *pixel != Color32::BLACK));
        if frame == 2 {
            if let Some(previous) = &replay {
                assert_eq!(previous, &actual.pixels);
            } else {
                replay = Some(actual.pixels.clone());
            }
        }
    }
    drop(preview);
    let directory = tempfile::tempdir()?;
    let audio_directory = tempfile::tempdir()?;
    let mut audio = AudioForm::default();
    if renderer == BackendKind::Cpu {
        let path = audio_directory.path().join("tone.wav");
        write_tone(&path, 440.0, 0.125)?;
        audio.enabled = true;
        audio.clips.push(ClipSettings::new(path, 0.125));
    }
    let destination = directory.path().join("native.mp4");
    let job = start_export_job(
        scene.clone(),
        renderer,
        destination.clone(),
        audio.apply(AudioOptions::default())?,
        context.clone(),
    )?;
    assert_eq!(job.label, "acceptance");
    assert_eq!(job.path, destination);
    assert!(matches!(
        wait_for_export(&job, &updates),
        JobState::Finished(Ok(()))
    ));
    let media = motion_ffmpeg::inspect::media(&destination)?;
    assert_eq!((media.width, media.height, media.video_frames), (96, 48, 3));
    assert_eq!(media.video_fps, 24.0);
    assert!(media.h264);
    assert_eq!(media.aac, renderer == BackendKind::Cpu);
    assert_eq!(motion_ffmpeg::inspect::video_frames(&destination, 4)?, 3);
    if media.aac {
        assert_eq!((media.sample_rate, media.channels), (48000, 2));
        let samples = motion_ffmpeg::inspect::audio_samples(&destination, 7168)?;
        assert!((6000..=7024).contains(&samples.len()));
        assert!(samples.iter().flatten().all(|sample| sample.is_finite()));
    }
    let committed = std::fs::read(&destination)?;
    let restricted = StudioScene {
        composition: composition.clone().configure(|options| {
            options.limits(motion_core::RasterLimits {
                max_work: 1,
                ..composition.raster_limits()
            })
        }),
        ..scene.clone()
    };
    let denied = start_export_job(
        restricted,
        renderer,
        destination.clone(),
        AudioOptions::default().overwrite(true),
        context.clone(),
    )?;
    let JobState::Finished(Err(error)) = wait_for_export(&denied, &updates) else {
        panic!("export must inherit the restrictive work budget");
    };
    assert!(error.to_string().contains("max_work"), "{error}");
    assert_eq!(std::fs::read(&destination)?, committed);
    assert_eq!(std::fs::read_dir(directory.path())?.count(), 1);
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let worker_gate = gate.clone();
    let (reached, waiting) = std::sync::mpsc::channel();
    let cancellable = StudioScene::from_render(
        "cancel",
        composition.clone(),
        Size::new(96.0, 48.0),
        move |context| {
            if context.time() == context.frames(1).after_origin() {
                let _result = reached.send(());
                let (released, timeout) = worker_gate
                    .1
                    .wait_timeout_while(
                        worker_gate.0.lock().unwrap(),
                        std::time::Duration::from_secs(30),
                        |released| !*released,
                    )
                    .unwrap();
                assert!(
                    *released && !timeout.timed_out(),
                    "cancellation gate timed out"
                );
            }
            build(context)
        },
    )
    .unwrap();
    let cancelled = start_export_job(
        cancellable,
        renderer,
        destination.clone(),
        AudioOptions::default().overwrite(true),
        context.clone(),
    )?;
    let reached = waiting.recv_timeout(std::time::Duration::from_secs(30));
    let progress = cancelled.state();
    cancelled.cancel();
    *gate.0.lock().unwrap() = true;
    gate.1.notify_one();
    reached.map_err(io::Error::other)?;
    assert!(matches!(progress, JobState::Running(_)));
    assert!(cancelled.cancelling());
    assert!(matches!(
        wait_for_export(&cancelled, &updates),
        JobState::Finished(Err(error)) if error.kind() == motion_core::ErrorKind::Cancelled
    ));
    assert_eq!(std::fs::read(&destination)?, committed);
    for panic in [false, true] {
        let failing = StudioScene::from_render(
            "failure",
            composition.clone(),
            Size::new(96.0, 48.0),
            move |context| {
                if context.time() == context.frames(1).after_origin() {
                    assert!(!panic, "Expected export panic");
                    return Node::empty().opacity(f64::NAN);
                }
                build(context)
            },
        )
        .unwrap();
        let failed = start_export_job(
            failing,
            renderer,
            destination.clone(),
            AudioOptions::default().overwrite(true),
            context.clone(),
        )?;
        let JobState::Finished(Err(error)) = wait_for_export(&failed, &updates) else {
            panic!("Invalid scene must fail");
        };
        assert_eq!(
            error.kind() == motion_core::ErrorKind::ScenePanic,
            panic,
            "{error}"
        );
        assert_eq!(error.frame(), Some(1));
        if panic {
            assert!(error.to_string().contains("`export`"));
            assert!(error.to_string().contains("Expected export panic"));
        }
        assert_eq!(std::fs::read(&destination)?, committed);
        assert_eq!(std::fs::read_dir(directory.path())?.count(), 1);
    }
    let retry = start_export_job(
        scene,
        renderer,
        destination.clone(),
        audio.apply(AudioOptions::default().overwrite(true))?,
        context,
    )?;
    assert!(matches!(
        wait_for_export(&retry, &updates),
        JobState::Finished(Ok(()))
    ));
    assert_eq!(motion_ffmpeg::inspect::video_frames(&destination, 4)?, 3);
    assert_eq!(std::fs::read_dir(directory.path())?.count(), 1);
    eprintln!(
        "_studio_worker renderer={renderer:?} preview_requests=4 max_rgba_difference={maximum} replay=pass h264=pass decoded_frames=3 aac={} scene_budget=pass cancel_during_prepare_frame=1 failure=pass panic=pass retry=pass cleanup=pass",
        media.aac
    );
    Ok(())
}

#[test]
#[ignore = "requires MOTION_TEST_FONT and native FFmpeg/libx264/AAC"]
fn cpu_preview_export_cancel_failure_and_recovery() -> motion_core::Result {
    worker_export_acceptance(BackendKind::Cpu)
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires MOTION_TEST_FONT, native GPU and FFmpeg/libx264"]
fn gpu_preview_export_cancel_failure_and_recovery() -> motion_core::Result {
    worker_export_acceptance(BackendKind::Gpu)
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires a native GPU, fonts and native FFmpeg with libx264"]
fn gpu_preview_seek_export_and_cancellation_preserve_worker_contracts() -> motion_core::Result {
    use motion_core::{Color, Node, Rect, RoundedRect, SceneContext, Shape, Size};
    use motion_render::Composition;
    use std::time::{Duration, Instant};

    let directory = tempfile::Builder::new()
        .prefix("gpu-worker-validation-")
        .tempdir_in(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"))?
        .keep();
    eprintln!("GPU_WORKER_ARTIFACTS={}", directory.display());
    let composition = Composition::new(
        96,
        64,
        motion_render::FrameClock::new(motion_core::fps!(12), 12),
    )
    .with_options(motion_render::RenderOptions::default().background(Color::BLACK))?;
    let build = |context: &SceneContext<'_>| {
        let left = 8.0 + context.time().to_frame(context.frame_rate()) as f64 * 4.0;
        Node::from(
            RoundedRect::new(Rect::new(left, 12.0, left + 24.0, 44.0), 6.0)
                .unwrap()
                .fill(Color::from_hex(0x118677)),
        )
    };
    let size = Size::new(96.0, 64.0);
    let scene = StudioScene::from_render("GPU worker acceptance", composition.clone(), size, build)
        .unwrap();
    let mut software = composition.frame_renderer(size)?;
    let mut gpu = composition
        .clone()
        .configure(|options| options.backend(motion_render::BackendPreference::gpu()))
        .frame_renderer(size)?;
    gpu.render_frame(0, build)?;
    let adapter = gpu.gpu_info().expect("initialized GPU").clone();
    #[cfg(target_os = "macos")]
    assert_eq!(adapter.backend, motion_render::GpuBackend::Metal);
    let (wake, updates) = std::sync::mpsc::channel();
    let context = Context::default();
    context.set_request_repaint_callback(move |_| {
        let _result = wake.send(());
    });
    let preview = PreviewCoordinator::new(scene.clone(), context.clone())?;
    let mut preview_checks = Vec::new();
    for (renderer, frame) in [
        (BackendKind::Gpu, 0),
        (BackendKind::Gpu, 9),
        (BackendKind::Cpu, 3),
        (BackendKind::Gpu, 11),
        (BackendKind::Gpu, 0),
    ] {
        let key = PreviewKey { frame, renderer };
        preview.request(key);
        let started = Instant::now();
        let reply = loop {
            if let Some(reply) = preview.take_reply() {
                break reply;
            }
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "Preview worker timed out"
            );
            let _update = updates.recv_timeout(Duration::from_millis(20));
        };
        assert_eq!(reply.key, key);
        let image = reply.image.map_err(io::Error::other)?;
        let reference = match renderer {
            BackendKind::Cpu => software.render_frame(frame, build)?,
            BackendKind::Gpu => gpu.render_frame(frame, build)?,
        };
        let reference = color_image(reference);
        assert_eq!(image.size, [96, 64]);
        assert!(
            image.pixels == reference.pixels,
            "Preview differs at {key:?}"
        );
        assert!(image.pixels.iter().any(|pixel| *pixel != Color32::BLACK));
        preview_checks
            .push(serde_json::json!({"renderer": format!("{renderer:?}"), "frame": frame, "reference": "exact"}));
    }
    drop(preview);
    let destination = directory.join("gpu.mp4");
    let export = start_export_job(
        scene,
        BackendKind::Gpu,
        destination.clone(),
        AudioOptions::default(),
        context.clone(),
    )?;
    assert!(matches!(
        wait_for_export(&export, &updates),
        JobState::Finished(Ok(()))
    ));
    let metadata = motion_ffmpeg::inspect::media(&destination)?;
    assert!(metadata.h264 && !metadata.aac);
    assert_eq!((metadata.width, metadata.height), (96, 64));
    assert_eq!(metadata.video_fps, 12.0);
    assert_eq!(metadata.video_frames, 12);
    assert_eq!(motion_ffmpeg::inspect::video_frames(&destination, 12)?, 12);
    let previous = std::fs::read(&destination)?;
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let worker_gate = gate.clone();
    let (reached, waiting) = std::sync::mpsc::channel();
    let cancellable =
        StudioScene::from_render("GPU cancellation", composition, size, move |frame| {
            if frame.time().to_frame(frame.frame_rate()) == 2 {
                let _result = reached.send(());
                let (released, timeout) = worker_gate
                    .1
                    .wait_timeout_while(
                        worker_gate.0.lock().unwrap(),
                        Duration::from_secs(30),
                        |released| !*released,
                    )
                    .unwrap();
                assert!(
                    *released && !timeout.timed_out(),
                    "Cancellation gate timed out"
                );
            }
            build(frame)
        })
        .unwrap();
    let cancelled = start_export_job(
        cancellable,
        BackendKind::Gpu,
        destination.clone(),
        AudioOptions::default().overwrite(true),
        context,
    )?;
    let reached = waiting.recv_timeout(Duration::from_secs(30));
    let running = matches!(cancelled.state(), JobState::Running(_));
    cancelled.cancel();
    *gate.0.lock().unwrap() = true;
    gate.1.notify_one();
    reached.map_err(io::Error::other)?;
    assert!(
        running,
        "Cancellation must occur while GPU frame preparation is gated"
    );
    assert!(cancelled.cancelling());
    assert!(matches!(
        wait_for_export(&cancelled, &updates),
        JobState::Finished(Err(error)) if error.kind() == motion_core::ErrorKind::Cancelled
    ));
    assert_eq!(std::fs::read(&destination)?, previous);
    assert!(!directory.read_dir()?.any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".motion-video-")
    }));
    let report = serde_json::json!({
        "backend": format!("{:?}", adapter.backend), "adapter": adapter.name,
        "device_type": adapter.device_type, "preview": preview_checks,
        "encoder": "_ffmpeg", "video": {"width": metadata.width, "height": metadata.height,
        "fps": metadata.video_fps, "decoded_frames": 12},
        "cancelled_during_preparation_frame": 2, "existing_video_preserved": true,
        "partial_outputs_remaining": false, "_gui_interaction": "not_tested",
    });
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_vec_pretty(&report)
            .map_err(|error| motion_core::Error::Other(Box::new(error)))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "requires native FFmpeg with libx264/AAC and system fonts"]
fn audio_drafts_round_trip_and_mix_through_the_export_worker() -> motion_core::Result {
    use crate::audio::settings::{AudioForm, ClipSettings};

    let directory = tempfile::Builder::new()
        .prefix("studio-audio-validation-")
        .tempdir_in(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"))?
        .keep();
    eprintln!("Studio audio validation artifacts: {}", directory.display());
    let voice = directory.join("voice with spaces.wav");
    let music = directory.join("music.wav");
    for (path, frequency) in [(&voice, 880), (&music, 220)] {
        write_tone(path, f64::from(frequency), 2.0)?;
    }
    let mut narration = ClipSettings::new(voice.clone(), 1.25);
    narration.source_in = 0.25;
    narration.start = 0.5;
    narration.volume = 0.6;
    narration.fade_in = 0.1;
    narration.fade_out = 0.1;
    let mut background = ClipSettings::new(music, 2.0);
    background.volume = 0.12;
    background.fade_in = 0.25;
    background.fade_out = 0.25;
    let original = AudioForm {
        enabled: true,
        clips: vec![narration, background],
    };
    let configuration = directory.join("audio.motion.json");
    original.save(&configuration)?;
    let mut restored = AudioForm::default();
    restored.load(&configuration)?;
    let options = restored.apply(AudioOptions::default())?;
    let destination = directory.join("mixed.mp4");
    let (wake, updates) = std::sync::mpsc::channel();
    let context = Context::default();
    context.set_request_repaint_callback(move |_| {
        let _result = wake.send(());
    });
    let scene = StudioScene::from_render(
        "Audio acceptance",
        motion_render::Composition::new(
            64,
            64,
            motion_render::FrameClock::new(motion_core::fps!(24), 48),
        ),
        motion_core::Size::new(64.0, 64.0),
        |_| motion_core::Node::empty(),
    )
    .unwrap();
    let job = start_export_job(
        scene.clone(),
        BackendKind::Cpu,
        destination.clone(),
        options,
        context.clone(),
    )?;
    restored.enabled = false;
    restored.clips.clear();
    assert!(matches!(
        wait_for_export(&job, &updates),
        JobState::Finished(Ok(()))
    ));
    let metadata = motion_ffmpeg::inspect::media(&destination)?;
    assert!(metadata.h264 && metadata.aac);
    assert_eq!((metadata.width, metadata.height), (64, 64));
    assert_eq!(metadata.video_fps, 24.0);
    assert_eq!(metadata.video_frames, 48);
    assert_eq!(motion_ffmpeg::inspect::video_frames(&destination, 48)?, 48);
    assert_eq!((metadata.sample_rate, metadata.channels), (48_000, 2));
    assert!((metadata.audio_seconds - 2.0).abs() < 1.0 / 48_000.0);
    let decoded = motion_ffmpeg::inspect::audio_samples(&destination, 97_024)?;
    assert!(decoded.iter().flatten().all(|sample| sample.is_finite()));
    let samples: Vec<_> = decoded.iter().map(|pair| pair[0]).collect();
    assert!((96000..=97024).contains(&samples.len()));
    assert!(samples.iter().all(|sample| sample.is_finite()));
    let amplitude = |frequency: f64, start: f64, end: f64| {
        let begin = (start * 48000.0) as usize;
        let finish = (end * 48000.0) as usize;
        let mut sine = 0.0;
        let mut cosine = 0.0;
        for (offset, sample) in samples[begin..finish].iter().enumerate() {
            let angle = std::f64::consts::TAU * frequency * (begin + offset) as f64 / 48000.0;
            sine += f64::from(*sample) * angle.sin();
            cosine += f64::from(*sample) * angle.cos();
        }
        2.0 * sine.hypot(cosine) / (finish - begin) as f64
    };
    let music_alone = amplitude(220.0, 0.25, 0.45);
    let music_overlap = amplitude(220.0, 0.8, 1.0);
    let voice_overlap = amplitude(880.0, 0.8, 1.0);
    let music_in = amplitude(220.0, 0.025, 0.075);
    let music_out = amplitude(220.0, 1.925, 1.975);
    let voice_in = amplitude(880.0, 0.51, 0.55);
    let voice_out = amplitude(880.0, 1.45, 1.49);
    assert!((0.01..0.02).contains(&music_alone));
    assert!((0.04..0.1).contains(&voice_overlap));
    assert!((0.8..1.2).contains(&(music_overlap / music_alone)));
    assert!(amplitude(880.0, 0.25, 0.45) < 0.002);
    assert!(amplitude(880.0, 1.6, 1.8) < 0.002);
    assert!(music_in < music_alone * 0.5 && music_out < music_alone * 0.5);
    assert!(voice_in < voice_overlap * 0.65 && voice_out < voice_overlap * 0.65);
    restored.load(&configuration)?;
    let options = restored.apply(AudioOptions::default().overwrite(true))?;
    std::fs::rename(&voice, directory.join("voice moved.wav"))?;
    let previous = std::fs::read(&destination)?;
    let missing = start_export_job(
        scene,
        BackendKind::Cpu,
        destination.clone(),
        options,
        context,
    )?;
    let missing_error = match wait_for_export(&missing, &updates) {
        JobState::Finished(Err(error)) => error,
        state => panic!("Missing source unexpectedly succeeded: {state:?}"),
    };
    assert_eq!(std::fs::read(&destination)?, previous);
    assert!(!directory.read_dir()?.any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".motion-video-")
    }));
    let report = serde_json::json!({
        "encoder_backend": "_ffmpeg",
        "video_frames": metadata.video_frames, "video_fps": metadata.video_fps,
        "video_width": metadata.width, "video_height": metadata.height,
        "audio_sample_rate": metadata.sample_rate, "audio_channels": metadata.channels,
        "audio_seconds": metadata.audio_seconds,
        "pcm_frames": samples.len(), "music_amplitude": music_alone,
        "music_overlap_ratio": music_overlap / music_alone,
        "voice_amplitude": voice_overlap, "music_fade_in_ratio": music_in / music_alone,
        "music_fade_out_ratio": music_out / music_alone,
        "voice_fade_in_ratio": voice_in / voice_overlap,
        "voice_fade_out_ratio": voice_out / voice_overlap,
        "missing_source_error": missing_error.to_string(), "existing_video_preserved": true,
        "_ui_verified": false,
    });
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_vec_pretty(&report)
            .map_err(|error| motion_core::Error::Other(Box::new(error)))?,
    )?;
    eprintln!(
        "{}",
        serde_json::to_string_pretty(&report)
            .map_err(|error| motion_core::Error::Other(Box::new(error)))?
    );
    Ok(())
}

fn key(frame: u32) -> PreviewKey {
    PreviewKey {
        frame,
        renderer: BackendKind::Cpu,
    }
}

#[test]
fn preview_mailbox_coalesces_requests_and_rejects_stale_results() {
    let mut mailbox = PreviewRequestQueue::default();
    mailbox.request(key(1));
    let first = mailbox.pending.take().unwrap();
    mailbox.request(key(2));
    mailbox.request(key(90));
    assert!(!mailbox.publish(first, Err(io::Error::other("stale").into())));
    let newest = mailbox.pending.take().unwrap();
    assert_eq!(newest.key.frame, 90);
    assert!(mailbox.publish(
        newest,
        Ok(Arc::new(ColorImage::filled([1, 1], Color32::WHITE)))
    ));
    assert_eq!(mailbox.reply.take().unwrap().key.frame, 90);
    mailbox.stopping = true;
    assert!(!mailbox.publish(newest, Err(io::Error::other("stopped").into())));
}

#[test]
fn preview_pixels_keep_rgb_channels_and_opaque_alpha() {
    let snapshot = motion_render::Composition::new(
        2,
        1,
        motion_render::FrameClock::new(motion_core::fps!(30), 150),
    )
    .with_options(
        motion_render::RenderOptions::default().background(motion_core::Color::rgb(17, 134, 119)),
    )
    .unwrap()
    .render_frame(0, motion_core::Size::new(2.0, 1.0), |_| {
        motion_core::Node::empty()
    })
    .unwrap();
    let image = color_image(snapshot);
    assert_eq!(image.size, [2, 1]);
    assert_eq!(image.pixels, vec![Color32::from_rgb(17, 134, 119); 2]);
}

fn color_image(snapshot: motion_render::FrameBuffer) -> ColorImage {
    ColorImage::new(
        [
            snapshot.size().width as usize,
            snapshot.size().height as usize,
        ],
        snapshot
            .packed_rgb()
            .iter()
            .map(|pixel| Color32::from_rgb((pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8))
            .collect(),
    )
}
