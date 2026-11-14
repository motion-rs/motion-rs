use super::*;
use crate::preview::PreviewKey;
use motion_core::Shape;
use motion_core::{Node, Size};
use motion_render::Composition;

#[test]
fn stats_copy_button_exports_current_sections_and_values() -> motion_core::Result<()> {
    let context = egui::Context::default();
    let creation = eframe::CreationContext::_new_kittest(context.clone());
    let scene = StudioScene::from_render(
        "Stats copy",
        Composition::new(
            16,
            8,
            motion_render::FrameClock::new(motion_core::fps!(30), 90),
        ),
        Size::new(16.0, 8.0),
        |_| Node::empty(),
    )
    .unwrap();
    let clock = StudioClock::new(&scene.composition)?;
    let mut app = StudioApp::with_persistence(&creation, scene, clock, false)?;
    app.inspection.inspector_tab = 3;
    for populated in [false, true] {
        if populated {
            let first = Instant::now();
            app.observations.presentation_times =
                [first, first + Duration::from_millis(100)].into();
            app.observations.dropped_frames = 7;
            app.observations.elapsed = Duration::from_millis(12);
            app.observations.cache_hits = 2;
            app.observations.preview_count = 3;
            app.cached_bytes = 1048576;
            let timings = motion_render::composition::FrameTimings {
                evaluation: Duration::from_millis(1),
                output: Duration::from_millis(5),
                fused_output: true,
                ..Default::default()
            };
            #[cfg(feature = "gpu")]
            let timings = motion_render::composition::FrameTimings {
                gpu: Some(motion_render::gpu::GpuTimings {
                    composition: motion_render::gpu::GpuTimestamp {
                        duration: Some(Duration::from_millis(2)),
                        ..Default::default()
                    },
                    instrumentation_bytes: 96,
                    ..Default::default()
                }),
                ..timings
            };
            app.observations.timings = Some(timings);
            app.observations.frame_stats = Some(motion_render::FrameStats {
                preparation: motion_render::PreparationStats {
                    peak_bytes: 2097152,
                    work: 42,
                    ..Default::default()
                },
                ..Default::default()
            });
        }
        let mut render = |events| {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(340.0, 1200.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| app.inspector(ui));
                },
            );
            output.textures_delta.clear();
            output
        };
        render(Vec::new());
        let output = render(Vec::new());
        assert!(output.platform_output.commands.is_empty());
        let separators = output
            .shapes
            .iter()
            .filter(|shape| matches!(&shape.shape, egui::Shape::LineSegment { .. }))
            .count();
        assert_eq!(
            separators,
            if populated {
                3 + usize::from(cfg!(feature = "gpu"))
            } else {
                2
            }
        );
        let copy_button = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == egui_phosphor::regular::COPY => {
                    Some(text.visual_bounding_rect())
                }
                _ => None,
            })
            .unwrap();
        let pointer = copy_button.center();
        assert!(copy_button.bottom() < 50.0);
        assert!(copy_button.left() > 300.0);
        render(vec![
            egui::Event::PointerMoved(pointer),
            egui::Event::PointerButton {
                pos: pointer,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        let output = render(vec![egui::Event::PointerButton {
            pos: pointer,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        let [egui::OutputCommand::CopyText(text)] = output.platform_output.commands.as_slice()
        else {
            panic!("Expected exactly one clipboard text command");
        };
        if populated {
            for expected in [
                "Playback\nRate: 10.0 / 30.00 fps\nSkipped frames: 7\n",
                "Preview worker\nTotal: 12.00 ms\nEvaluate: 1.00 ms\n",
                "CPU compose + output: 5.00 ms\n",
                "Native submit (CPU wall): 0.00 ms\n",
                "Cache\nHits: 2 / 3\nSize: 1.0 MiB\n",
                "Frame cost (work / peak)\nPrepare: 42 / 2.0 MiB\n",
                "Composite: 0 / 0.0 MiB\nOutput: 0 / 0.0 MiB\n",
            ] {
                assert!(text.contains(expected), "{text}");
            }
            #[cfg(feature = "gpu")]
            {
                assert!(text.contains("GPU timestamps\nComposition: 2.00 ms\n"));
                assert!(text.contains("Query buffers: 96 bytes\n"));
            }
            assert!(!text.contains("Output conversion:"));
        } else {
            assert_eq!(
                text,
                "Playback\nRate: -- / 30.00 fps\nSkipped frames: 0\n\
                 \nPreview worker\nTotal: 0.00 ms\n\
                 \nCache\nHits: 0 / 0\nSize: 0.0 MiB\n"
            );
        }
    }
    Ok(())
}

#[test]
fn loop_shortcut_preserves_transport_and_respects_keyboard_focus() -> motion_core::Result<()> {
    let context = egui::Context::default();
    let creation = eframe::CreationContext::_new_kittest(context.clone());
    let scene = StudioScene::from_render(
        "Loop shortcut",
        Composition::new(
            16,
            8,
            motion_render::FrameClock::new(motion_core::fps!(30), 90),
        ),
        Size::new(16.0, 8.0),
        |_| Node::empty(),
    )
    .unwrap();
    let clock = StudioClock::new(&scene.composition)?;
    let mut app = StudioApp::with_persistence(&creation, scene, clock, false)?;
    let loop_input = || egui::RawInput {
        events: [true, false]
            .map(|pressed| egui::Event::Key {
                key: egui::Key::L,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            })
            .to_vec(),
        ..Default::default()
    };
    app.playback.transport.frame = 42;
    app.exports.export_range = (20, 60);
    for playing in [false, true] {
        let anchor = playing.then_some((std::time::Instant::now(), 42));
        app.playback.transport.anchor = anchor;
        for looping in [true, false] {
            context
                .run_ui(loop_input(), |ui| app.shortcuts(ui.ctx()))
                .textures_delta
                .clear();
            assert_eq!(app.playback.transport.looping, looping);
            assert_eq!(app.playback.transport.frame, 42);
            assert_eq!(app.playback.transport.anchor, anchor);
            assert_eq!(app.exports.export_range, (20, 60));
        }
    }
    let anchor = app.playback.transport.anchor;
    let mut text = String::new();
    context
        .run_ui(loop_input(), |ui| {
            let context = &ui.ctx().clone();
            egui::CentralPanel::default().show(ui, |ui| {
                ui.text_edit_singleline(&mut text).request_focus();
                assert!(context.egui_wants_keyboard_input());
                assert!(context.input(|input| input.key_pressed(egui::Key::L)));
                app.shortcuts(context);
            });
        })
        .textures_delta
        .clear();
    assert!(!app.playback.transport.looping);
    assert_eq!(app.playback.transport.frame, 42);
    assert_eq!(app.playback.transport.anchor, anchor);
    Ok(())
}

#[test]
fn scene_status_and_sidebar_use_window_edges_without_preview_heading() -> motion_core::Result<()> {
    use eframe::App;
    for (width, height) in [(900.0, 620.0), (1240.0, 800.0), (1440.0, 900.0)] {
        let context = egui::Context::default();
        let creation = eframe::CreationContext::_new_kittest(context.clone());
        let scene = StudioScene::from_render(
            "scene",
            Composition::new(
                16,
                8,
                motion_render::FrameClock::new(motion_core::fps!(30), 1800),
            ),
            Size::new(16.0, 8.0),
            |_| Node::empty(),
        )
        .unwrap();
        let clock = StudioClock::new(&scene.composition)?;
        let mut app = StudioApp::with_persistence(&creation, scene, clock, false)?;
        app.observations.backend_status = "GPU (Apple M3)".into();
        let key = PreviewKey {
            frame: 1799,
            renderer: BackendKind::Cpu,
        };
        app.playback.transport.frame = key.frame;
        app.playback.last_interaction = Instant::now() - Duration::from_secs(1);
        app.preview_state.request(key, app.preview_settings);
        app.preview_state.pending = false;
        app.preview_state.displayed = Some(key);
        let mut frame = eframe::Frame::_new_kittest();
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, height),
                )),
                ..Default::default()
            },
            |ui| {
                app.ui(ui, &mut frame);
                assert!(app.preview_state.is_requested(key, app.preview_settings));
                assert!(
                    !app.preview_state.pending,
                    "Layout must not enqueue preview work"
                );
            },
        );
        output.textures_delta.clear();
        let text: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    Some(text)
                } else {
                    None
                }
            })
            .collect();
        for label in ["GPU (Apple M3)", "Frame 1799"] {
            let status = text
                .iter()
                .find(|text| text.galley.text() == label)
                .expect(label);
            assert!(
                status.visual_bounding_rect().top() >= height - 28.0,
                "{label} at {width}"
            );
        }
        let sidebar = text
            .iter()
            .find(|text| text.galley.text() == egui_phosphor::regular::SIDEBAR_SIMPLE)
            .expect("sidebar control");
        assert!(sidebar.visual_bounding_rect().center().x >= width - 32.0);
        assert!(sidebar.visual_bounding_rect().bottom() <= 40.0);
        assert!(!text.iter().any(|text| text.galley.text() == "scene"
            || text.galley.text() == egui_phosphor::regular::ARROW_CLOCKWISE));
    }
    Ok(())
}

#[test]
fn timeline_boundary_warnings_are_visible_without_rendering_or_devices() -> motion_core::Result<()>
{
    let context = egui::Context::default();
    let creation = eframe::CreationContext::_new_kittest(context);
    let timeline = motion_core::Timeline::new(Size::new(16.0, 8.0), motion_core::fps!(30))
        .track(|track| {
            track.scene(motion_core::Duration::from_seconds(0.05), |_| {
                Node::from(
                    motion_core::Rect::new(0.0, 0.0, 8.0, 8.0).fill(motion_core::Color::WHITE),
                )
            })
        })
        .build()?;
    assert!(!timeline.diagnostics().is_empty());
    let scene = StudioScene::new(
        "Alignment",
        Composition::new(
            16,
            8,
            motion_render::FrameClock::new(motion_core::fps!(30), 150),
        ),
        timeline,
    )?;
    let clock = StudioClock::new(&scene.composition)?;
    let app = StudioApp::with_persistence(&creation, scene, clock, false)?;
    assert!(!app.observations.diagnostics.is_empty());
    assert!(
        app.observations
            .diagnostics
            .iter()
            .all(|(frame, message)| *frame < app.clock.frames
                && message.contains("Frame-boundary alignment"))
    );
    Ok(())
}

#[test]
fn single_scene_workbench_layout_and_queue_are_bounded_without_devices() -> motion_core::Result<()>
{
    use eframe::App;
    for (width, height) in [(900.0, 620.0), (1240.0, 800.0), (1440.0, 900.0)] {
        let context = egui::Context::default();
        let creation = eframe::CreationContext::_new_kittest(context.clone());
        let scene = StudioScene::from_render(
            "Single scene with a long descriptive title",
            Composition::new(
                16,
                8,
                motion_render::FrameClock::new(motion_core::fps!(30), 90),
            ),
            Size::new(16.0, 8.0),
            |_| {
                Node::from(
                    motion_core::Rect::new(0.0, 0.0, 8.0, 8.0).fill(motion_core::Color::WHITE),
                )
            },
        )
        .unwrap();
        let clock = StudioClock::new(&scene.composition)?;
        let mut app = StudioApp::with_persistence(&creation, scene, clock, false)?;
        app.exports.export_destination = "fixture.mp4".into();
        app.start_export(&context);
        assert_eq!(app.exports.export_queue.len(), 1);
        app.exports.export_size = motion_core::PixelSize::new(32, 16);
        app.start_export(&context);
        assert_eq!(
            app.exports.export_queue[0].draft.size,
            motion_core::PixelSize::new(16, 8)
        );
        assert_eq!(
            app.exports.export_queue[1].draft.size,
            motion_core::PixelSize::new(32, 16)
        );
        for _ in 2..16 {
            app.start_export(&context);
        }
        assert_eq!(app.exports.export_queue.len(), 16);
        app.start_export(&context);
        assert_eq!(app.exports.export_queue.len(), 16);
        assert!(matches!(app.exports.export_message, Some((true, _))));
        app.exports.export_queue.clear();
        let first = Instant::now();
        app.observations
            .presentation_times
            .extend([first, first + std::time::Duration::from_millis(100)]);
        app.playback.preview_muted = true;
        app.toggle_playback();
        assert!(app.observations.presentation_times.is_empty());
        app.pause();
        app.observations
            .presentation_times
            .extend([first, first + std::time::Duration::from_millis(100)]);
        let cue = "\u{754c}".repeat(90);
        app.apply_session(&serde_json::json!({"version":1,"scene":app.scene.name,"frame":11,"range":[3,31],"scale":25,"transparent":false,"cues":[[8,cue],[91,"outside"],[9,"x".repeat(129)]]}));
        assert_eq!(app.playback.transport.frame, 11);
        assert_eq!(app.exports.export_range, (3, 31));
        assert_eq!(app.preview_settings.scale, 25);
        assert!(!app.preview_settings.transparent);
        assert_eq!(app.inspection.viewer.background, 3);
        assert_eq!(app.inspection.timeline_view.cues, vec![(8, cue)]);
        app.apply_session(&serde_json::json!({"version":1,"scene":"another","frame":89,"range":[0,90],"cues":[],"transparent":false}));
        assert_eq!(app.playback.transport.frame, 11);
        assert_eq!(app.exports.export_range, (3, 31));
        assert_eq!(app.inspection.timeline_view.cues.len(), 1);
        app.close_when_idle = true;
        let mut frame = eframe::Frame::_new_kittest();
        for tab in 0..4 {
            app.inspection.inspector_tab = tab;
            app.inspection.output_visible = Some(true);
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, height),
                    )),
                    ..Default::default()
                },
                |ui| {
                    app.close_when_idle = false;
                    app.ui(ui, &mut frame);
                    app.pause();
                },
            );
            output.textures_delta.clear();
            assert!(!output.shapes.is_empty());
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape {
                    let visible = text.visual_bounding_rect().intersect(shape.clip_rect);
                    if visible.is_positive() {
                        assert!(
                            visible.left() >= -1.0
                                && visible.top() >= -1.0
                                && visible.right() <= width + 1.0
                                && visible.bottom() <= height + 1.0,
                            "tab {tab}, width {width}: {visible:?}"
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
fn studio_session_round_trips_atomically_and_rejects_bad_or_large_files() -> io::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join(".motion/studio.json");
    let value = serde_json::json!({"version":1,"scene":"fractional","frame":27,"scale":50,"transparent":true});
    write_session(&path, &value)?;
    assert_eq!(read_session(&path), Some(value.clone()));
    let oversized = serde_json::json!({"version":1,"scene":"a".repeat(65_537)});
    assert!(write_session(&path, &oversized).is_err());
    assert_eq!(read_session(&path), Some(value));
    std::fs::write(&path, b"invalid json")?;
    assert!(read_session(&path).is_none());
    std::fs::write(&path, b"{\"version\":2}")?;
    assert!(read_session(&path).is_none());
    std::fs::write(&path, vec![b' '; 65_537])?;
    assert!(read_session(&path).is_none());
    Ok(())
}

#[test]
fn rational_transport_uses_wall_time_without_integer_fps_rounding() -> motion_core::Result<()> {
    let now = Instant::now();
    let rate = motion_core::FrameRate::try_new(30_000, 1001).unwrap();
    let mut transport = Transport {
        anchor: Some((now, 0)),
        ..Default::default()
    };
    transport.advance_rate(now + Duration::from_secs(1001), rate, 40_000);
    assert_eq!(transport.frame, 30_000);
    transport.advance_rate(now + Duration::from_secs(2002), rate, 40_000);
    assert_eq!(transport.frame, 39_999);
    assert!(transport.anchor.is_none());
    Ok(())
}

#[test]
fn invalid_design_dimensions_fail_at_registration_before_evaluation() {
    for size in [Size::new(0.0, 8.0), Size::new(f64::NAN, 8.0)] {
        let error = StudioScene::from_render(
            "Invalid geometry",
            Composition::new(
                16,
                8,
                motion_render::FrameClock::new(motion_core::fps!(30), 150),
            ),
            size,
            |_| panic!("invalid configuration must not evaluate"),
        )
        .err()
        .expect("invalid design size must be rejected");
        assert_eq!(error.kind(), motion_core::ErrorKind::Timeline);
    }
}

#[test]
fn playback_uses_elapsed_time_stops_or_loops_and_seeking_pauses() {
    let now = Instant::now();
    let mut transport = Transport::default();
    transport.toggle(now, 90);
    transport.advance_rate(now + Duration::from_millis(500), motion_core::fps!(30), 90);
    assert_eq!(transport.frame, 15);
    transport.advance_rate(now + Duration::from_millis(1500), motion_core::fps!(30), 90);
    assert_eq!(transport.frame, 45);
    transport.advance_rate(now + Duration::from_secs(5), motion_core::fps!(30), 90);
    assert_eq!(transport.frame, 89);
    assert!(transport.anchor.is_none());
    transport.looping = true;
    transport.toggle(now, 90);
    transport.advance_rate(now + Duration::from_secs(4), motion_core::fps!(30), 90);
    assert_eq!(transport.frame, 30);
    transport.seek(900, 90);
    assert_eq!(transport.frame, 89);
    assert!(transport.anchor.is_none());
}

#[test]
fn preview_keeps_aspect_ratio_in_wide_and_tall_panels() {
    for size in [egui::vec2(800.0, 300.0), egui::vec2(300.0, 800.0)] {
        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        let fitted = crate::viewer::Viewer::default().bounds(area, egui::vec2(640.0, 360.0));
        assert!((fitted.width() / fitted.height() - 16.0 / 9.0).abs() < 0.001);
        assert!(area.contains_rect(fitted));
    }
}

#[test]
fn play_holds_the_clock_until_its_first_frame_is_answered() -> motion_core::Result<()> {
    let context = egui::Context::default();
    let creation = eframe::CreationContext::_new_kittest(context);
    let scene = StudioScene::from_render(
        "Play start",
        Composition::new(
            16,
            8,
            motion_render::FrameClock::new(motion_core::fps!(30), 90),
        ),
        Size::new(16.0, 8.0),
        |_| Node::empty(),
    )
    .unwrap();
    let clock = StudioClock::new(&scene.composition)?;
    let mut app = StudioApp::with_persistence(&creation, scene, clock, false)?;
    app.playback.preview_muted = true;
    app.playback.transport.frame = 89;
    app.toggle_playback();
    // The last frame wraps to the start; the clock stays still while that frame renders.
    assert_eq!(app.playback.transport.frame, 0);
    assert!(app.playback.transport.anchor.is_none());
    assert!(app.playback.transport.playing());
    let key = preview::PreviewKey {
        frame: 0,
        renderer: BackendKind::Cpu,
    };
    app.preview_state
        .request(key, preview::PreviewSettings::default());
    assert!(!app.ready_to_start());
    app.preview_state.pending = false;
    assert!(app.ready_to_start());
    app.start_clock();
    assert!(app.playback.transport.starting.is_none());
    assert_eq!(
        app.playback.transport.anchor.map(|anchor| anchor.1),
        Some(0)
    );

    // Pausing before the first frame cancels the start; a stuck first frame times out.
    app.pause();
    app.toggle_playback();
    app.pause();
    assert!(!app.playback.transport.playing());
    app.toggle_playback();
    app.preview_state.pending = true;
    app.playback.transport.starting = Some(Instant::now() - Duration::from_secs(3));
    assert!(app.ready_to_start());
    app.start_clock();
    assert!(app.playback.transport.anchor.is_some());
    Ok(())
}

fn reloadable(path: &std::path::Path) -> StudioScene {
    let source = path.to_owned();
    let load = move || -> motion_core::Result<StudioScene> {
        let text = std::fs::read_to_string(&source)?;
        let frames: u64 = text.trim().parse().map_err(|_| {
            motion_core::ValidationError::invalid(
                "scene",
                "frames",
                format!("Not a frame count: {text}"),
            )
        })?;
        Ok(StudioScene::from_render(
            "Reloadable",
            Composition::new(
                16,
                8,
                motion_render::FrameClock::new(motion_core::fps!(30), frames),
            ),
            Size::new(16.0, 8.0),
            |_| Node::empty(),
        )
        .unwrap())
    };
    load().unwrap().reloadable([path.to_owned()], load)
}

fn rewrite(path: &std::path::Path, text: &str, seconds_ahead: u64) {
    std::fs::write(path, text).unwrap();
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(seconds_ahead))
        .unwrap();
}

fn settle(app: &mut StudioApp, context: &egui::Context, done: impl Fn(&StudioApp) -> bool) {
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    while !done(app) {
        assert!(
            Instant::now() < deadline,
            "reload did not finish: {:?}",
            app.reload_status
        );
        context
            .run_ui(Default::default(), |ui| app.poll_reload(ui.ctx()))
            .textures_delta
            .clear();
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
}

#[test]
fn changed_files_reload_the_scene_and_keep_the_playhead() -> motion_core::Result<()> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("scene.txt");
    std::fs::write(&path, "30").unwrap();
    let context = egui::Context::default();
    let creation = eframe::CreationContext::_new_kittest(context.clone());
    let scene = reloadable(&path);
    let clock = StudioClock::new(&scene.composition)?;
    let mut app = StudioApp::with_persistence(&creation, scene, clock, false)?;
    app.playback.transport.frame = 20;
    app.inspection.timeline_view.cues = vec![(5, "early".into()), (25, "late".into())];
    assert_eq!(app.clock.frame_count(), 30);

    rewrite(&path, "60", 5);
    settle(&mut app, &context, |app| {
        app.reload_status
            .as_ref()
            .is_some_and(|(_, message)| message == "Reloaded")
    });
    assert_eq!(app.clock.frame_count(), 60);
    assert_eq!(app.playback.transport.frame, 20);
    assert_eq!(app.inspection.timeline_view.cues.len(), 2);
    assert_eq!(app.exports.export_range, (0, 60));
    assert!(
        app.scene.reload.is_some(),
        "the swapped scene stays reloadable"
    );

    rewrite(&path, "10", 10);
    settle(&mut app, &context, |app| app.clock.frame_count() == 10);
    assert_eq!(
        app.playback.transport.frame, 9,
        "the playhead clamps to the new end"
    );
    assert_eq!(app.inspection.timeline_view.cues, vec![(5, "early".into())]);
    assert_eq!(app.exports.export_range, (0, 10));
    Ok(())
}

#[test]
fn failed_reloads_keep_the_previous_scene_and_report_the_error() -> motion_core::Result<()> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("scene.txt");
    std::fs::write(&path, "30").unwrap();
    let context = egui::Context::default();
    let creation = eframe::CreationContext::_new_kittest(context.clone());
    let scene = reloadable(&path);
    let clock = StudioClock::new(&scene.composition)?;
    let mut app = StudioApp::with_persistence(&creation, scene, clock, false)?;

    rewrite(&path, "not a number", 5);
    settle(&mut app, &context, |app| {
        app.reload_status.as_ref().is_some_and(|(error, _)| *error)
    });
    let (_, message) = app.reload_status.clone().unwrap();
    assert!(
        message.contains("previous version") && message.contains("Not a frame count"),
        "{message}"
    );
    assert_eq!(app.clock.frame_count(), 30);
    assert!(
        app.observations
            .diagnostics
            .iter()
            .any(|(_, text)| text.contains("Not a frame count"))
    );

    rewrite(&path, "45", 10);
    settle(&mut app, &context, |app| app.clock.frame_count() == 45);
    assert_eq!(app.reload_status, Some((false, "Reloaded".into())));
    Ok(())
}

#[test]
fn scenes_without_a_reload_source_are_never_watched() -> motion_core::Result<()> {
    let context = egui::Context::default();
    let creation = eframe::CreationContext::_new_kittest(context.clone());
    let scene = StudioScene::from_render(
        "Static",
        Composition::new(
            16,
            8,
            motion_render::FrameClock::new(motion_core::fps!(30), 3),
        ),
        Size::new(16.0, 8.0),
        |_| Node::empty(),
    )
    .unwrap();
    let clock = StudioClock::new(&scene.composition)?;
    let mut app = StudioApp::with_persistence(&creation, scene, clock, false)?;
    context
        .run_ui(Default::default(), |ui| app.poll_reload(ui.ctx()))
        .textures_delta
        .clear();
    assert!(app.reload.is_none() && app.reload_status.is_none());
    Ok(())
}

#[test]
fn replacement_scenes_update_the_watched_dependencies() -> motion_core::Result<()> {
    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("original.txt");
    let dependency = directory.path().join("dependency.txt");
    std::fs::write(&original, "30").unwrap();
    std::fs::write(&dependency, "45").unwrap();
    let context = egui::Context::default();
    let creation = eframe::CreationContext::_new_kittest(context.clone());
    let scene = reloadable(&original);
    let clock = StudioClock::new(&scene.composition)?;
    let mut app = StudioApp::with_persistence(&creation, scene, clock, false)?;
    app.replace_scene(&context, reloadable(&dependency))?;
    assert_eq!(
        app.scene.reload.as_ref().unwrap().watch,
        vec![dependency.clone()]
    );
    rewrite(&dependency, "60", 5);
    settle(&mut app, &context, |app| app.clock.frame_count() == 60);
    Ok(())
}
