use super::*;

#[test]
fn playhead_mapping_clamps_edges_and_short_timelines() {
    let area = egui::Rect::from_min_size(egui::pos2(72.0, 0.0), egui::vec2(100.0, 24.0));
    assert_eq!(frame_at(area, egui::pos2(-1.0, 0.0), 30), 0);
    assert_eq!(frame_at(area, egui::pos2(122.0, 0.0), 30), 15);
    assert_eq!(frame_at(area, egui::pos2(200.0, 0.0), 30), 29);
    assert_eq!(frame_at(area, egui::pos2(200.0, 0.0), 1), 0);
}

#[test]
fn range_handles_snap_to_frame_boundaries_inside_the_ruler() {
    let area = egui::Rect::from_min_size(egui::pos2(72.0, 0.0), egui::vec2(100.0, 24.0));
    assert_eq!(boundary_at(area, egui::pos2(0.0, 0.0), 30), 0);
    assert_eq!(boundary_at(area, egui::pos2(172.0, 0.0), 30), 30);
    assert_eq!(boundary_at(area, egui::pos2(500.0, 0.0), 30), 30);
    assert_eq!(boundary_at(area, egui::pos2(122.0, 0.0), 30), 15);
}

#[test]
fn cached_frames_merge_into_runs_for_painting_and_lookup() {
    assert_eq!(merge_runs(&[]), Vec::<(u32, u32)>::new());
    assert_eq!(merge_runs(&[1, 2, 3, 7, 9, 10]), [(1, 4), (7, 8), (9, 11)]);
    let context = egui::Context::default();
    let scene = StudioScene::new(
        "runs",
        motion_render::Composition::new(
            16,
            8,
            motion_render::FrameClock::new(motion_core::fps!(30), 30),
        ),
        motion_core::Size::new(16.0, 8.0),
        |_| motion_core::Node::empty(),
    );
    let mut view = TimelineView::new(
        &scene,
        &context,
        crate::playback::StudioClock::new(&scene.composition).unwrap(),
    );
    view.set_cache(&[10, 3, 4, 3, 5]);
    assert_eq!(view.cache_runs, [(3, 6), (10, 11)]);
    for (frame, cached) in [
        (2, false),
        (3, true),
        (5, true),
        (6, false),
        (10, true),
        (11, false),
    ] {
        assert_eq!(view.cache_contains(frame), cached, "frame {frame}");
    }
}

#[test]
fn ruler_steps_stay_readable_from_short_clips_to_long_films() {
    assert_eq!(tick_step(90, 30, 800.0), 10);
    assert_eq!(tick_step(1800, 30, 800.0), 300);
    for (total, width) in [(1, 800.0), (30, 40.0), (1_000_000, 300.0)] {
        let step = tick_step(total, 30, width);
        assert!(step >= 1);
    }
    assert_eq!(tick_label(90, 30, 30), "0:03");
    assert_eq!(tick_label(45, 30, 5), "45");
    assert_eq!(timecode(75.25), "01:15.250");
}
