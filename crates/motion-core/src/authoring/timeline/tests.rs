//! Unit tests for timeline scheduling, frame alignment and track composition.

use super::*;
use crate::{PixelSize, SceneCache, TimeLiteral, ValidationLimits};

fn size() -> Size {
    Size::new(8.0, 4.0)
}

fn scene(duration: Duration) -> impl Scene {
    (|_: &SceneContext<'_>| Node::empty()).with_duration(duration)
}

#[test]
fn fluent_builders_retain_all_errors_and_valid_chains_need_no_question_marks() -> crate::Result {
    let error = Timeline::new(size(), 30)
        .track(|track| {
            track
                .push((|_: &SceneContext<'_>| Node::empty()).with_duration(1.seconds()))
                .gap((-1).seconds())
                .place(
                    (-1).seconds().after_origin(),
                    (|_: &SceneContext<'_>| Node::empty()).with_duration(1.seconds()),
                )
        })
        .duration((-1).seconds())
        .build()
        .unwrap_err();
    assert_eq!(error.errors().len(), 3);
    assert!(error.to_string().contains("track[0]"));
    let valid = Timeline::new(size(), 30)
        .track(|track| {
            track.push((|_: &SceneContext<'_>| Node::empty()).with_duration(1.seconds()))
        })
        .build()?;
    assert_eq!(valid.frame_count(), 30);
    assert!(Timeline::new(Size::new(f64::NAN, 4.0), 30).build().is_err());
    Ok(())
}

#[test]
fn track_boundaries_are_half_open_and_sorted() -> crate::Result {
    let timeline = Timeline::try_new(size(), 10)?
        .try_track(|track| {
            track
                .try_place(2.seconds().after_origin(), scene(1.seconds()))?
                .try_place(Time::ZERO, scene(1.seconds()))?
                .try_push(scene(1.seconds()))
        })?
        .build()?;
    assert_eq!(timeline.frame_count(), 40);
    assert_eq!(
        timeline.tracks()[0]
            .clips()
            .iter()
            .map(Clip::id)
            .collect::<Vec<_>>(),
        [1, 0, 2]
    );
    let track = &timeline.tracks()[0];
    assert_eq!(track.active_clip(Time::ZERO).unwrap().id(), 1);
    assert!(track.active_clip(1.seconds().after_origin()).is_none());
    assert_eq!(
        track.active_clip(3.seconds().after_origin()).unwrap().id(),
        2
    );
    assert!(track.active_clip(4.seconds().after_origin()).is_none());
    let cache = SceneCache::new(0);
    let context = SceneContext::new(Time::ZERO, FrameRate::from(10), size(), &cache);
    timeline
        .render(&context)
        .compile(size(), PixelSize::new(8, 4), ValidationLimits::default())?;
    Ok(())
}

#[test]
fn nested_push_and_embed_preserve_global_ticks_and_child_rate_without_retiming() -> crate::Result {
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let calls = observed.clone();
    let child = Timeline::try_new(Size::new(8.0, 4.0), 24)?
        .try_track(|track| {
            track.try_push(
                (move |context: &SceneContext<'_>| {
                    calls.lock().unwrap().push((
                        context.time(),
                        context.global_time(),
                        context.frame_rate(),
                        context.size(),
                        context.clip_duration(),
                    ));
                    Node::from(context.bounds().fill(Color::WHITE))
                })
                .with_duration(2.seconds()),
            )
        })?
        .build()?;
    let parent = Timeline::try_new(Size::new(12.0, 12.0), FrameRate::NTSC)?
        .try_track(|track| track.try_gap(1.seconds())?.try_nest(child.clone()))?
        .build()?;
    let cache = SceneCache::new(0);
    let context = SceneContext::new(
        9.seconds().after_origin(),
        FrameRate::NTSC,
        parent.size(),
        &cache,
    )
    .shifted(7.seconds());
    let start = parent.tracks()[0].clips()[0].start();
    let pushed = parent.render(&context).compile(
        parent.size(),
        PixelSize::new(12, 12),
        ValidationLimits::default(),
    )?;
    let embedded = context.embed(&child.visual(), start).compile(
        parent.size(),
        PixelSize::new(12, 12),
        ValidationLimits::default(),
    )?;
    assert_eq!(pushed.commands, embedded.commands);
    assert_eq!(
        observed.lock().unwrap().as_slice(),
        &[(
            (2.seconds() - start.since_origin()).after_origin(),
            9.seconds().after_origin(),
            FrameRate::from(24),
            child.size(),
            Some(2.seconds())
        ); 2]
    );
    let at_end = SceneContext::new(
        start + child.duration(),
        FrameRate::NTSC,
        parent.size(),
        &cache,
    );
    assert!(
        at_end
            .embed(&child.visual(), start)
            .compile(
                parent.size(),
                PixelSize::new(12, 12),
                ValidationLimits::default()
            )?
            .commands
            .is_empty()
    );
    Ok(())
}

#[test]
fn timeline_rejects_overlap_missing_duration_and_collapsed_intervals() -> crate::Result {
    let builder =
        Timeline::try_new(size(), 10)?.try_track(|track| track.try_push(scene(1.seconds())))?;
    let error = builder
        .clone()
        .try_track(|track| {
            track
                .try_push(scene(1.seconds()))?
                .try_place(500.milliseconds().after_origin(), scene(1.seconds()))
        })
        .unwrap_err();
    assert_eq!(error.kind(), crate::ErrorKind::Timeline);
    assert!(error.to_string().contains("track[1]") && error.to_string().contains("overlaps"));
    assert_eq!(builder.build()?.tracks.len(), 1);
    assert!(
        Timeline::try_new(size(), 10)?
            .try_track(|track| track.try_push(|_: &SceneContext<'_>| Node::empty()))
            .unwrap_err()
            .to_string()
            .contains("with_duration")
    );
    assert!(
        Timeline::try_new(size(), 10)?
            .try_track(|track| track.try_push(scene(1.milliseconds())))
            .is_err()
    );
    assert!(Timeline::try_new(size(), FrameRate::new(u32::MAX, 1)).is_err());
    Ok(())
}

#[test]
fn timeline_snaps_independent_boundaries_and_retains_warnings() -> crate::Result {
    let timeline = Timeline::try_new(size(), 10)?
        .try_track(|track| {
            track.try_place(50.milliseconds().after_origin(), scene(100.milliseconds()))
        })?
        .build()?;
    let clip = &timeline.tracks()[0].clips()[0];
    assert_eq!(clip.start(), 100.milliseconds().after_origin());
    assert_eq!(clip.end(), 200.milliseconds().after_origin());
    assert!(
        timeline
            .diagnostics()
            .boundary_adjustments()
            .any(|adjustment| adjustment.edge == BoundaryEdge::Start
                && adjustment.requested == 50.milliseconds().after_origin())
    );
    assert_eq!(timeline.frame_count(), 2);
    Ok(())
}

#[test]
fn alignment_uses_tick_distances_ntsc_and_raw_cursors_without_drift() -> crate::Result {
    for rate in [
        FrameRate::from(60),
        FrameRate::NTSC_FILM,
        FrameRate::NTSC,
        FrameRate::NTSC_DOUBLE,
        FrameRate::new(61, 1),
        FrameRate::new(1000, 3),
        FrameRate::new(TICKS_PER_SECOND as u32, 1),
    ] {
        for index in [0, 1, 2, 59, 1000, 100_000] {
            let first = Time::from_frame(index, rate);
            let second = Time::from_frame(index + 1, rate);
            assert_eq!(align(first, rate)?, (first, index));
            let middle = Time::from_ticks(first.ticks() + (second.ticks() - first.ticks()) / 2);
            let expected = if middle.ticks() - first.ticks() < second.ticks() - middle.ticks() {
                (first, index)
            } else {
                (second, index + 1)
            };
            assert_eq!(align(middle, rate)?, expected);
        }
    }
    let timeline = Timeline::try_new(size(), 60)?
        .try_track(|mut track| {
            for _ in 0..1000 {
                track = track.try_push(scene(167.milliseconds()))?;
            }
            assert_eq!(track.cursor(), 167.seconds().after_origin());
            Ok(track)
        })?
        .build()?;
    assert_eq!(timeline.frame_count(), 10_020);
    for clips in timeline.tracks[0].clips.windows(2) {
        assert_eq!(clips[0].end(), clips[1].start());
    }
    let ntsc = Timeline::try_new(size(), FrameRate::NTSC)?
        .try_track(|track| track.try_push(scene(1.seconds())))?;
    assert_eq!(
        ntsc.clone().build()?.duration(),
        Duration::from_frame(30, FrameRate::NTSC)
    );
    let adjusted = ntsc.try_duration(1500.milliseconds())?;
    assert_eq!(adjusted.clone().build()?.frame_count(), 45);
    let durations = |timeline: &Timeline| {
        timeline
            .diagnostics()
            .boundary_adjustments()
            .filter(|adjustment| adjustment.edge == BoundaryEdge::Duration)
            .count()
    };
    assert_eq!(durations(&adjusted.clone().build()?), 1);
    assert_eq!(durations(&adjusted.try_duration(2.seconds())?.build()?), 1);
    Ok(())
}

#[test]
fn construction_errors_limits_and_cow_snapshots_are_bounded() -> crate::Result {
    for bad in [Time::from_ticks(-1), Time::from_ticks(i64::MIN)] {
        assert!(
            Timeline::try_new(size(), 30)?
                .try_track(|track| track.try_place(bad, scene(1.seconds())))
                .is_err()
        );
        assert!(
            Timeline::try_new(size(), 30)?
                .try_track(|track| track.try_gap(bad.since_origin()))
                .is_err()
        );
        assert!(
            Timeline::try_new(size(), 30)?
                .try_duration(bad.since_origin())
                .is_err()
        );
    }
    assert!(
        Timeline::try_new(size(), 30)?
            .try_track(|track| track.try_push(scene(Duration::ZERO)))
            .is_err()
    );
    assert!(
        Timeline::try_new(size(), 30)?
            .try_track(|track| track.try_place(Time::from_ticks(i64::MAX), scene(1.seconds())))
            .is_err()
    );
    assert!(
        Timeline::try_new(size(), 30)?
            .try_track(|track| track
                .try_gap(Duration::from_ticks(i64::MAX))?
                .try_gap(1.seconds()))
            .is_err()
    );
    assert!(Timeline::try_new(Size::new(f64::NAN, 4.0), 30).is_err());
    let mut original = Timeline::try_new(size(), 30)?;
    for _ in 0..MAX_TRACKS {
        original = original.try_track(Ok)?;
    }
    let snapshot = original.clone();
    assert!(original.try_track(Ok).is_err());
    assert_eq!(snapshot.build()?.tracks.len(), MAX_TRACKS);
    let full = Timeline::try_new(size(), 10)?.try_track(|mut track| {
        for _ in 0..MAX_CLIPS {
            track = track.try_push(scene(100.milliseconds()))?;
        }
        Ok(track)
    })?;
    assert!(
        full.clone()
            .try_track(|track| track.try_push(scene(1.seconds())))
            .is_err()
    );
    assert_eq!(full.build()?.tracks()[0].clips().len(), MAX_CLIPS);
    let mut foreign = None;
    let captured = Timeline::try_new(size(), 24)?.try_track(|track| {
        foreign = Some(track);
        Err(invalid("capture only"))
    });
    assert!(captured.is_err());
    assert!(
        Timeline::try_new(size(), 30)?
            .try_track(|_| Ok(foreign.unwrap()))
            .is_err()
    );
    Ok(())
}

#[test]
fn soundtrack_exposes_declared_audio_and_master() -> crate::Result {
    let master = crate::Master {
        gain_db: -3.0,
        ..Default::default()
    };
    let timeline = Timeline::new(size(), 30)
        .audio(crate::AudioClip::file("a.wav").duration(1.seconds()))
        .audio_bus(crate::AudioBus::new("music"))
        .master(master)
        .build()?;
    let soundtrack = timeline.soundtrack();
    assert_eq!(soundtrack.clips().len(), 1);
    assert_eq!(soundtrack.buses().len(), 1);
    assert_eq!(soundtrack.master(), master);
    Ok(())
}

#[test]
fn explicit_duration_shorter_than_content_warns_and_audio_extends_inferred_length() -> crate::Result
{
    let timeline = Timeline::new(size(), 10)
        .track(|track| track.push(scene(2.seconds())))
        .duration(1.seconds())
        .build()?;
    assert_eq!(timeline.duration(), 1.seconds());
    assert_eq!(
        timeline.diagnostics().warnings(),
        [TimelineWarning::Truncated {
            duration: 1.seconds(),
            content: 2.seconds(),
        }]
    );
    let with_audio = Timeline::new(size(), 10)
        .track(|track| track.push(scene(1.seconds())))
        .audio(
            crate::AudioClip::file("a.wav")
                .start(500.milliseconds().after_origin())
                .duration(1.seconds()),
        )
        .build()?;
    assert_eq!(with_audio.duration(), 1500.milliseconds());
    assert!(with_audio.diagnostics().is_empty());
    Ok(())
}

#[test]
fn clips_take_their_duration_at_placement_and_can_start_inside_the_scene() -> crate::Result {
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let calls = observed.clone();
    let probe = move |context: &SceneContext<'_>| {
        calls.lock().unwrap().push(context.time());
        Node::empty()
    };
    let timeline = Timeline::try_new(size(), 10)?
        .try_track(|track| {
            track
                .try_push_for(1.seconds(), probe.clone())?
                .try_push_trimmed(500.milliseconds(), 1.seconds(), probe.clone())
        })?
        .build()?;
    assert_eq!(timeline.duration(), 2.seconds());
    let clips = timeline.tracks()[0].clips();
    assert_eq!(clips[0].offset(), Duration::ZERO);
    assert_eq!(clips[1].offset(), 500.milliseconds());
    let cache = SceneCache::new(0);
    for time in [200.milliseconds(), 1200.milliseconds()] {
        let context = SceneContext::new(time.after_origin(), FrameRate::from(10), size(), &cache);
        timeline.render(&context);
    }
    assert_eq!(
        observed.lock().unwrap().as_slice(),
        [
            200.milliseconds().after_origin(),
            700.milliseconds().after_origin()
        ]
    );
    assert!(
        Timeline::try_new(size(), 10)?
            .try_track(|track| track.try_push_trimmed((-1).seconds(), 1.seconds(), probe.clone()))
            .is_err()
    );
    assert!(probe.clone().try_with_duration((-1).seconds()).is_err());
    Ok(())
}
