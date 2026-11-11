use super::*;
use motion_core::Shape;

fn wait_thumbnail(
    worker: &PreviewCoordinator,
    updates: &std::sync::mpsc::Receiver<()>,
    frame: u32,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !worker.thumbnails().iter().any(|entry| entry.0 == frame) {
        assert!(Instant::now() < deadline, "thumbnail timed out");
        let _ = updates.recv_timeout(Duration::from_millis(20));
    }
}

#[test]
fn seek_discards_inflight_thumbnail_before_new_thumbnail_completes() -> motion_core::Result<()> {
    let (entered, waiting) = std::sync::mpsc::channel();
    let (release, gate) = std::sync::mpsc::channel();
    let gate = Mutex::new(gate);
    let scene = StudioScene::preview_only(
        "thumbnail seek",
        motion_render::Composition::new(
            16,
            8,
            motion_render::FrameClock::new(motion_core::fps!(30), 12),
        ),
        motion_core::Size::new(16.0, 8.0),
        move |context| {
            if context.time().to_frame(context.frame_rate()) == 6 {
                entered.send(()).unwrap();
                gate.lock().unwrap().recv().unwrap();
            }
            motion_core::Node::from(context.bounds().fill(motion_core::Color::WHITE))
        },
    );
    let (wake, updates) = std::sync::mpsc::channel();
    let context = Context::default();
    context.set_request_repaint_callback(move |_| {
        let _ = wake.send(());
    });
    let worker = PreviewCoordinator::with_slots(scene, context, 65536, 2)?;
    let key = PreviewKey {
        frame: 3,
        renderer: Renderer::Cpu,
    };
    worker.request_with_settings(key, PreviewSettings::default());
    super::super::tests::wait_for_preview(&worker, &updates, key).image?;
    worker.request_thumbnail(PreviewKey { frame: 6, ..key });
    waiting.recv_timeout(Duration::from_secs(30)).unwrap();
    let seek = PreviewKey { frame: 9, ..key };
    worker.request_with_settings(seek, PreviewSettings::default());
    super::super::tests::wait_for_preview(&worker, &updates, seek).image?;
    worker.request_thumbnail(PreviewKey { frame: 7, ..key });
    release.send(()).unwrap();
    wait_thumbnail(&worker, &updates, 7);
    assert!(!worker.thumbnails().iter().any(|entry| entry.0 == 6));
    assert!(worker.take_reply().is_none());
    Ok(())
}

#[test]
fn resource_failure_thumbnail_is_not_retained_as_healthy_artwork() -> motion_core::Result<()> {
    let directory = tempfile::tempdir()?;
    let (entered, waiting) = std::sync::mpsc::channel();
    let scene = StudioScene::preview_only(
        "thumbnail resource",
        motion_render::Composition::new(
            16,
            8,
            motion_render::FrameClock::new(motion_core::fps!(30), 12),
        )
        .with_options(
            motion_render::RenderOptions::default()
                .resources(motion_core::Assets::new(directory.path())?),
        )?,
        motion_core::Size::new(16.0, 8.0),
        move |context| {
            let frame = context.time().to_frame(context.frame_rate());
            entered.send(frame).unwrap();
            if frame == 6 {
                motion_core::Node::from(motion_content::Image::open("missing.png").size(16.0, 8.0))
            } else {
                motion_core::Node::from(context.bounds().fill(motion_core::Color::WHITE))
            }
        },
    );
    let (wake, updates) = std::sync::mpsc::channel();
    let context = Context::default();
    context.set_request_repaint_callback(move |_| {
        let _ = wake.send(());
    });
    let worker = PreviewCoordinator::with_slots(scene, context, 65536, 2)?;
    let key = PreviewKey {
        frame: 3,
        renderer: Renderer::Cpu,
    };
    worker.request_with_settings(key, PreviewSettings::default());
    super::super::tests::wait_for_preview(&worker, &updates, key).image?;
    assert_eq!(waiting.recv_timeout(Duration::from_secs(30)).unwrap(), 3);
    worker.request_thumbnail(PreviewKey { frame: 6, ..key });
    assert_eq!(waiting.recv_timeout(Duration::from_secs(30)).unwrap(), 6);
    worker.request_thumbnail(PreviewKey { frame: 7, ..key });
    wait_thumbnail(&worker, &updates, 7);
    assert!(!worker.thumbnails().iter().any(|entry| entry.0 == 6));
    assert!(worker.take_reply().is_none());
    Ok(())
}

#[test]
fn idle_prefetch_is_serial_and_thumbnails_never_replace_main_reply() -> motion_core::Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (built, builds) = std::sync::mpsc::channel();
    let observed = active.clone();
    let maximum = peak.clone();
    let scene = StudioScene::preview_only(
        "idle",
        motion_render::Composition::new(
            16,
            8,
            motion_render::FrameClock::new(motion_core::fps!(30), 12),
        ),
        motion_core::Size::new(16.0, 8.0),
        move |context| {
            maximum.fetch_max(
                observed.fetch_add(1, Ordering::SeqCst) + 1,
                Ordering::SeqCst,
            );
            built
                .send(context.time().to_frame(context.frame_rate()))
                .unwrap();
            let node = motion_core::Node::from(context.bounds().fill(motion_core::Color::WHITE));
            observed.fetch_sub(1, Ordering::SeqCst);
            node
        },
    );
    let (wake, updates) = std::sync::mpsc::channel();
    let context = Context::default();
    context.set_request_repaint_callback(move |_| {
        let _ = wake.send(());
    });
    let worker = PreviewCoordinator::with_slots(scene, context, 65536, 3)?;
    let key = PreviewKey {
        frame: 3,
        renderer: Renderer::Cpu,
    };
    worker.request_idle(key, PreviewSettings::default());
    super::super::tests::wait_for_preview(&worker, &updates, key).image?;
    let mut frames = (0..5)
        .map(|_| builds.recv_timeout(Duration::from_secs(30)).unwrap())
        .collect::<Vec<_>>();
    frames.sort_unstable();
    assert_eq!(frames, [1, 2, 3, 4, 5]);
    assert_eq!(peak.load(Ordering::SeqCst), 1);
    worker.request_thumbnail(PreviewKey { frame: 6, ..key });
    let deadline = Instant::now() + Duration::from_secs(30);
    while !worker.thumbnails().iter().any(|entry| entry.0 == 6) {
        assert!(Instant::now() < deadline, "thumbnail timed out");
        let _ = updates.recv_timeout(Duration::from_millis(20));
    }
    assert!(worker.take_reply().is_none());
    worker.stop_prefetch();
    worker.request_with_settings(PreviewKey { frame: 5, ..key }, PreviewSettings::default());
    super::super::tests::wait_for_preview(&worker, &updates, PreviewKey { frame: 5, ..key })
        .image?;
    assert_eq!(builds.try_recv().unwrap(), 6);
    assert!(builds.try_recv().is_err());
    Ok(())
}

#[test]
fn thumbnail_cache_is_independent_small_and_entry_bounded() {
    let mut cache = ThumbnailCache::default();
    let image = ColorImage::filled([320, 180], eframe::egui::Color32::WHITE);
    for frame in 0..140 {
        cache.insert(
            PreviewKey {
                frame,
                renderer: Renderer::Cpu,
            },
            &image,
        );
    }
    assert_eq!(cache.entries.len(), 128);
    assert_eq!(cache.entries.front().unwrap().0.frame, 12);
    assert_eq!(cache.entries.back().unwrap().1.size, [160, 90]);
    assert!(cache.bytes <= 16 * 1024 * 1024);
    assert!(cache.entries.iter().all(|entry| {
        entry
            .1
            .pixels
            .iter()
            .all(|pixel| *pixel == eframe::egui::Color32::WHITE)
    }));
}

#[test]
fn latency_is_tracked_per_settings_and_forgotten_on_request() {
    let draft = PreviewSettings {
        quality: motion_core::RenderQuality::Draft,
        ..PreviewSettings::default()
    };
    let final_quality = PreviewSettings::default();
    let mut latency = Latencies::default();
    assert_eq!(latency.get(draft), Duration::ZERO);
    latency.observe(final_quality, Duration::from_millis(400));
    latency.observe(draft, Duration::from_millis(100));
    latency.observe(draft, Duration::from_millis(140));
    assert_eq!(latency.get(draft), Duration::from_millis(110));
    assert_eq!(latency.get(final_quality), Duration::from_millis(400));
    latency.forget(draft);
    assert_eq!(latency.get(draft), Duration::ZERO);
    assert_eq!(latency.get(final_quality), Duration::from_millis(400));
    for scale in 1..=8 {
        latency.observe(PreviewSettings { scale, ..draft }, Duration::from_millis(1));
    }
    assert_eq!(latency.0.len(), 8);
    assert_eq!(latency.get(final_quality), Duration::ZERO);
}

#[test]
fn rational_deadline_lead_is_bounded_and_uses_observed_latency() {
    assert_eq!(lead_frames(Duration::ZERO, motion_core::FrameRate::NTSC), 0);
    assert_eq!(
        lead_frames(Duration::from_millis(50), motion_core::FrameRate::NTSC),
        2
    );
    assert_eq!(
        lead_frames(Duration::from_secs(5), motion_core::FrameRate::NTSC),
        150
    );
}
