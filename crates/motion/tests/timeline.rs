//! Integration tests for scenes, tracks and timeline validation.

mod scenes {

    mod validation {
        use motion::extend::*;

        use motion::render::*;

        #[cfg(test)]
        mod tests {
            use super::*;
            use motion::extend::DrawContext;
            use motion::render::Composition;
            use motion::{Color, Filter, MaskMode, Node, NodeModifiers, Rect, Shape, Size};

            fn rectangle() -> Node {
                Rect::new(0.0, 0.0, 12.0, 8.0).fill(Color::WHITE).into()
            }

            #[test]
            fn preflight_reports_nested_properties_and_original_causes() {
                let scene = Node::group([Node::group([
                    rectangle().filter(Filter::Blur { sigma: f64::NAN })
                ])]);
                let error = scene
                    .preflight_with(&DrawContext::default(), ValidationLimits::default())
                    .unwrap_err();
                assert_eq!(error.path, "node.children[0].children[0].filters[0]");
                assert_eq!(error.field, "filters");
                assert_eq!(error.kind, ValidationErrorKind::InvalidValue);
                let scene = rectangle().mask(rectangle().opacity(f64::NAN), MaskMode::Alpha);
                let error = scene
                    .preflight_with(&DrawContext::default(), ValidationLimits::default())
                    .unwrap_err();
                assert_eq!(error.path, "node.mask");
                assert_eq!(error.field, "opacity");
                let grade = Filter::ColorAdjust {
                    exposure: f64::NAN,
                    contrast: 1.0,
                    saturation: 1.0,
                    hue_degrees: 0.0,
                };
                let original = grade.validate("node.filters[0]").unwrap_err();
                let error = rectangle()
                    .filter(grade)
                    .preflight_with(&DrawContext::default(), ValidationLimits::default())
                    .unwrap_err();
                assert_eq!(error, original);
            }

            #[test]
            fn structure_budget_counts_masks_inactive_endpoints_and_temporal_samples() {
                let scene = Node::group([rectangle().opacity(0.0), rectangle()])
                    .mask(rectangle(), MaskMode::Alpha);
                let stats = scene
                    .preflight_with(&DrawContext::default(), ValidationLimits::default())
                    .unwrap();
                assert_eq!(stats.nodes, 4);
                assert_eq!(stats.max_depth, 2);
                let error = scene
                    .preflight_with(
                        &DrawContext::default(),
                        ValidationLimits {
                            max_nodes: 3,
                            max_depth: 64,
                        },
                    )
                    .unwrap_err();
                assert_eq!(error.kind, ValidationErrorKind::StructuralLimit);
                assert_eq!(error.field, "max_nodes");
                let scene = Node::weighted_sum([(rectangle(), 0.0), (rectangle(), 1.0)]).unwrap();
                assert_eq!(
                    scene
                        .preflight_with(&DrawContext::default(), ValidationLimits::default())
                        .unwrap()
                        .nodes,
                    5
                );
                assert_eq!(
                    scene
                        .preflight_with(
                            &DrawContext::default(),
                            ValidationLimits {
                                max_nodes: 4,
                                max_depth: 64
                            }
                        )
                        .unwrap_err()
                        .field,
                    "max_nodes"
                );
                let hidden =
                    Node::weighted_sum([(rectangle().opacity(f64::NAN), 0.0), (rectangle(), 1.0)])
                        .unwrap();
                assert_eq!(
                    hidden
                        .preflight_with(&DrawContext::default(), ValidationLimits::default())
                        .unwrap_err()
                        .field,
                    "opacity"
                );
            }

            #[test]
            fn depth_and_configured_limit_boundaries_are_checked_before_recursing() {
                let scene = Node::group([rectangle()]);
                assert!(
                    scene
                        .preflight_with(
                            &DrawContext::default(),
                            ValidationLimits {
                                max_nodes: 2,
                                max_depth: 2
                            }
                        )
                        .is_ok()
                );
                let error = scene
                    .preflight_with(
                        &DrawContext::default(),
                        ValidationLimits {
                            max_nodes: 2,
                            max_depth: 1,
                        },
                    )
                    .unwrap_err();
                assert_eq!(error.field, "max_depth");
                for limits in [
                    ValidationLimits {
                        max_nodes: 0,
                        max_depth: 1,
                    },
                    ValidationLimits {
                        max_nodes: 1,
                        max_depth: 129,
                    },
                ] {
                    assert_eq!(
                        scene
                            .preflight_with(&DrawContext::default(), limits)
                            .unwrap_err()
                            .field,
                        "limits"
                    );
                }
            }

            #[test]
            fn strict_constructors_do_not_normalize_or_silently_ignore_invalid_inputs() {
                assert_eq!(Size::new(-1.0, 2.0).width, -1.0);
                assert!(Rect::new(f64::NAN, 0.0, 1.0, 1.0).left.is_nan());
                assert!(
                    motion::PixelSize::new(16, 16)
                        .mapping(Size::new(-1.0, 2.0))
                        .is_err()
                );
                assert!(
                    Node::from(Rect::new(f64::NAN, 0.0, 1.0, 1.0).fill(Color::WHITE))
                        .preflight_with(&DrawContext::default(), ValidationLimits::default())
                        .is_err()
                );
                assert_eq!(
                    Composition::try_new(0, 16, motion::render::FrameClock::new(30, 150))
                        .err()
                        .unwrap()
                        .field,
                    "width"
                );
                for opacity in [-0.01, 1.01, f64::NAN] {
                    assert_eq!(
                        rectangle()
                            .opacity(opacity)
                            .preflight_with(&DrawContext::default(), ValidationLimits::default())
                            .unwrap_err()
                            .field,
                        "opacity"
                    );
                }
                assert_eq!(Rect::new(0.0, 0.0, -1.0, 1.0).width(), -1.0);
            }
        }
    }

    mod authoring {
        use motion::prelude::*;
        use motion::render::*;

        #[test]
        fn accumulation_is_linear_premultiplied_isolated_and_retains_empty_weights()
        -> std::result::Result<(), ValidationError> {
            let bounds = Rect::new(0.0, 0.0, 2.0, 2.0);
            let red = Node::from(bounds.fill(motion::Color::rgb(255, 0, 0).with_alpha(0.5)));
            let blue = Node::from(bounds.fill(motion::Color::rgb(0, 0, 255).with_alpha(0.5)));
            let sample = Node::accumulate([red.clone(), blue])?;
            for space in [motion::BlendSpace::LinearRgb, motion::BlendSpace::Srgb] {
                let frame = sample
                    .compile(
                        motion::Size::new(2.0, 2.0),
                        motion::PixelSize::new(2, 2),
                        ValidationLimits::default(),
                    )?
                    .prepare(
                        ValidationLimits::default(),
                        motion::render::RasterLimits::default(),
                    )?
                    .blend_space(space)
                    .render_linear(motion::render::RasterLimits::default())?;
                for (actual, expected) in frame
                    .linear_pixel(0, 0)
                    .unwrap()
                    .iter()
                    .zip([0.25, 0.0, 0.25, 0.5])
                {
                    assert!((actual - expected).abs() < 1e-6);
                }
            }
            let half = Node::accumulate([red, Node::empty()])?
                .compile(
                    motion::Size::new(2.0, 2.0),
                    motion::PixelSize::new(2, 2),
                    ValidationLimits::default(),
                )?
                .prepare(
                    ValidationLimits::default(),
                    motion::render::RasterLimits::default(),
                )?
                .render_linear(motion::render::RasterLimits::default())?;
            assert_eq!(half.linear_pixel(0, 0), Some([0.25, 0.0, 0.0, 0.25]));
            assert!(sample.measure(ValidationLimits::default()).is_ok());
            assert!(
                sample
                    .compile(
                        motion::Size::new(2.0, 2.0),
                        motion::PixelSize::new(2, 2),
                        ValidationLimits {
                            max_nodes: 1,
                            ..ValidationLimits::default()
                        }
                    )
                    .is_err()
            );
            let prepared = sample
                .compile(
                    motion::Size::new(2.0, 2.0),
                    motion::PixelSize::new(2, 2),
                    ValidationLimits::default(),
                )?
                .prepare(
                    ValidationLimits::default(),
                    motion::render::RasterLimits::default(),
                )?;
            assert!(
                prepared
                    .render_linear(motion::render::RasterLimits {
                        max_work: 1,
                        ..motion::render::RasterLimits::default()
                    })
                    .is_err()
            );
            assert_eq!(
                prepared
                    .render_linear(motion::render::RasterLimits::default())?
                    .linear_pixel(0, 0),
                Some([0.25, 0.0, 0.25, 0.5])
            );
            assert!(Node::accumulate([]).is_err());
            assert!(Node::accumulate(std::iter::repeat_n(Node::empty(), 65)).is_err());
            Ok(())
        }
    }

    mod authoring_sampling {
        use motion::prelude::*;
        use motion::render::*;
        use motion::sample_offsets;

        #[test]
        fn temporal_midpoints_advance_both_clocks_and_average_premultiplied_samples()
        -> motion::Result {
            assert_eq!(
                sample_offsets(Duration::from_ticks(8), 4)?,
                [-3, -1, 1, 3].map(Duration::from_ticks)
            );
            let cache = SceneCache::new(0);
            let context = SceneContext::new(
                5.seconds().after_origin(),
                FrameRate::NTSC,
                Size::new(2.0, 1.0),
                &cache,
            )
            .shifted(5.seconds())
            .with_clip_duration(1.seconds());
            let mut observed = Vec::new();
            let node = context.try_motion_blur(Duration::from_ticks(8), 4, |sample| {
                observed.push((sample.time(), sample.global_time(), sample.clip_duration()));
                Node::from(
                    Rect::new(0.0, 0.0, 2.0, 1.0).fill(if sample.time() < Time::ZERO {
                        Color::rgb(255, 0, 0).with_alpha(0.5)
                    } else {
                        Color::rgb(0, 0, 255).with_alpha(0.5)
                    }),
                )
            })?;
            for (index, offset) in [-3, -1, 1, 3].into_iter().enumerate() {
                assert_eq!(
                    observed[index],
                    (
                        Time::from_ticks(offset),
                        (5.seconds() + Duration::from_ticks(offset)).after_origin(),
                        Some(1.seconds())
                    )
                );
            }
            let frame = node
                .compile(
                    context.size(),
                    PixelSize::new(2, 1),
                    ValidationLimits::default(),
                )?
                .prepare(ValidationLimits::default(), RasterLimits::default())?
                .render_linear(RasterLimits::default())?;
            assert_eq!(frame.linear_pixel(0, 0), Some([0.25, 0.0, 0.25, 0.5]));
            assert_eq!(
                MotionBlur {
                    shutter_angle: 180.0,
                    samples: 4
                }
                .shutter(FrameRate::NTSC)?,
                Duration::from_frame(1, FrameRate::NTSC) / 2.0
            );
            assert!(
                MotionBlur {
                    shutter_angle: 1e-8,
                    samples: 4
                }
                .shutter(FrameRate::new(1, u32::MAX))?
                .ticks()
                    > 80_000_000
            );
            let invalid_context = SceneContext::new(
                Time::from_ticks(i64::MAX),
                FrameRate::from(30),
                context.size(),
                &cache,
            );
            let mut calls = 0;
            assert!(
                invalid_context
                    .try_motion_blur(Duration::from_ticks(8), 4, |_| {
                        calls += 1;
                        Node::empty()
                    })
                    .is_err()
            );
            assert_eq!(calls, 0);
            assert!(sample_offsets(Duration::from_ticks(-1), 4).is_err());
            assert!(sample_offsets(Duration::ZERO, 65).is_err());
            let moving = context
                .try_motion_blur(Duration::from_ticks(8), 4, |sample| {
                    let column = ((sample.time().ticks() + 3) / 2) as f64;
                    Node::from(
                        Rect::new(column, 0.0, column + 1.0, 1.0)
                            .fill(Color::rgb(255, 0, 0).with_alpha(0.5)),
                    )
                })?
                .compile(
                    Size::new(4.0, 1.0),
                    PixelSize::new(4, 1),
                    ValidationLimits::default(),
                )?
                .prepare(ValidationLimits::default(), RasterLimits::default())?
                .render_linear(RasterLimits::default())?;
            for column in 0..4 {
                assert_eq!(
                    moving.linear_pixel(column, 0),
                    Some([0.125, 0.0, 0.0, 0.125])
                );
            }
            Ok(())
        }
    }

    mod timeline_rendering {
        use motion::extend::*;
        use motion::prelude::*;
        use motion::render::*;

        #[derive(Clone)]
        struct CurveFade(Easing);

        impl Transition for CurveFade {
            fn compose(
                &self,
                progress: f64,
                outgoing: Node,
                incoming: Node,
                context: &SceneContext<'_>,
            ) -> Node {
                CrossFade.compose(progress, outgoing, incoming, context)
            }

            fn easing(&self) -> Easing {
                self.0.clone()
            }
        }

        fn artwork(index: usize, time: Duration, duration: Duration, remap: bool) -> Node {
            let progress = (time / duration).clamp(0.0, 1.0);
            let progress = if remap {
                match index {
                    0 => progress * 0.5,
                    1 => 1.0 - progress,
                    3 => 0.75,
                    _ => (progress * 2.0).min(1.0),
                }
            } else {
                progress
            };
            let colors = [
                Color::from_hex(0x118677),
                Color::from_hex(0xd45d49),
                Color::from_hex(0x3458b3),
                Color::from_hex(0xe4b55c),
            ];
            Node::group([
                Node::from(
                    motion::Rect::new(1.0, 1.0, 63.0, 47.0).fill(colors[index].with_alpha(0.7)),
                ),
                Node::from(
                    motion::Rect::new(4.0 + progress * 40.0, 20.0, 10.0 + progress * 40.0, 28.0)
                        .fill(Color::WHITE),
                ),
            ])
        }

        fn compare(renderer: Renderer) -> motion::Result {
            let rate = FrameRate::from(30);
            let duration = Duration::from_frame(8, rate);
            let overlap = Duration::from_frame(4, rate);
            let size = Size::new(64.0, 48.0);
            for (curve, remap) in [
                (Easing::Linear, false),
                (Easing::QuadraticInOut, false),
                (
                    Easing::CubicBezier {
                        x1: 0.3,
                        y1: 1.8,
                        x2: 0.7,
                        y2: 1.8,
                    },
                    false,
                ),
                (Easing::QuadraticInOut, true),
            ] {
                let timeline = Timeline::try_new(size, rate)?
                    .try_track(|mut track| {
                        for index in 0..4 {
                            if index > 0 {
                                track = track.try_transition(CurveFade(curve.clone()), overlap)?;
                            }
                            track = track.try_push(
                                (move |context: &SceneContext<'_>| {
                                    artwork(index, context.elapsed(), duration, remap)
                                })
                                .with_duration(duration),
                            )?;
                        }
                        Ok(track)
                    })?
                    .build()?;
                assert_eq!(timeline.frame_count(), 20);
                let mut session = Composition::new(64, 48, motion::render::FrameClock::new(30, 24))
                    .with_options(
                        motion::render::RenderOptions::default().backend(renderer.into()),
                    )?
                    .frame_renderer(size)?;
                let mut replay = None;
                let mut held = None;
                for index in [5, 0, 7, 8, 11, 4, 12, 15, 16, 19, 20, 23, 5] {
                    let actual = session.render_frame(index, |context| timeline.render(context))?;
                    let expected = session.render_frame(index, |context| {
                        let active = (0..4)
                            .filter(|clip| {
                                let start = Time::from_frame(clip * 4, rate);
                                context.time() >= start && context.time() < start + duration
                            })
                            .collect::<Vec<_>>();
                        match active.as_slice() {
                            [] => Node::empty(),
                            [clip] => artwork(
                                *clip as usize,
                                context.time() - Time::from_frame(clip * 4, rate),
                                duration,
                                remap,
                            ),
                            [outgoing, incoming] => {
                                let incoming_time =
                                    context.time() - Time::from_frame(incoming * 4, rate);
                                let progress = curve.apply(incoming_time / overlap).clamp(0.0, 1.0);
                                CrossFade.compose(
                                    progress,
                                    artwork(
                                        *outgoing as usize,
                                        context.time() - Time::from_frame(outgoing * 4, rate),
                                        duration,
                                        remap,
                                    ),
                                    artwork(*incoming as usize, incoming_time, duration, remap),
                                    context,
                                )
                            }
                            _ => panic!("triple overlap"),
                        }
                    })?;
                    assert_eq!(
                        actual.packed_rgb(),
                        expected.packed_rgb(),
                        "manual timeline {renderer:?}/{index}"
                    );
                    assert_eq!(actual.packed_rgb()[0], 0xffffff);
                    if index >= 20 {
                        assert!(actual.packed_rgb().iter().all(|pixel| *pixel == 0xffffff));
                    }
                    if index == 5 {
                        if let Some(first) = &replay {
                            assert_eq!(actual.packed_rgb(), *first);
                        }
                        replay = Some(actual.packed_rgb());
                    }
                    if remap && index == 16 {
                        held = Some(actual.packed_rgb());
                    }
                    if remap && index == 19 {
                        assert_eq!(actual.packed_rgb(), *held.as_ref().unwrap());
                    }
                }
            }
            Ok(())
        }

        #[test]
        fn timelines_match_independent_clip_selection_easing_remapping_and_replay() -> motion::Result
        {
            compare(Renderer::Cpu)
        }

        #[cfg(feature = "gpu")]
        #[test]
        #[ignore = "requires native GPU; no fonts, window or encoder"]
        fn gpu_timelines_match_independent_selection_and_replay() -> motion::Result {
            compare(Renderer::Gpu)
        }
    }

    mod authoring_timeline {
        use motion::extend::*;
        use motion::prelude::*;
        use motion::render::*;

        fn size() -> Size {
            Size::new(8.0, 4.0)
        }

        fn scene(duration: Duration) -> impl Scene {
            (|_: &SceneContext<'_>| Node::empty()).with_duration(duration)
        }

        use std::sync::Arc;

        #[test]
        fn timeline_audio_inference_nested_offsets_and_crops_do_not_leak() -> motion::Result {
            use motion::extend::AudioSource;
            let clip = || {
                motion::AudioClip::source(
                    motion::encode::PcmBuffer::new(4, 2, vec![1.0; 16]).unwrap(),
                )
                .fade_out(1.seconds())
            };
            let child = Timeline::try_new(size(), 4)?.try_audio(clip())?.build()?;
            let parent = Timeline::try_new(size(), 4)?
                .try_track(|track| {
                    track
                        .try_gap(500.milliseconds())?
                        .try_nest_for(1.seconds(), child)?
                        .try_gap(1.seconds())
                })?
                .build()?;
            let mixed =
                motion::encode::MixedAudio::new(&parent, motion::encode::MixOptions::new(4))?;
            let mut values = [0.0; 20];
            mixed.read(0, &mut values);
            assert_eq!(&values[..4], &[0.0; 4]);
            assert_eq!(&values[4..12], &[1.0; 8]);
            assert_eq!(&values[12..], &[0.0; 8]);
            assert_eq!(parent.duration(), 2500.milliseconds());
            let short = Timeline::try_new(size(), 30)?
                .try_audio(motion::AudioClip::source(motion::encode::PcmBuffer::new(
                    1000,
                    1,
                    vec![1.0; 10],
                )?))?
                .build()?;
            assert_eq!(short.frame_count(), 1);
            let _mix =
                motion::encode::MixedAudio::new(&parent, motion::encode::MixOptions::new(48_000))?;
            Ok(())
        }

        #[test]
        fn transition_schedule_keeps_pair_clocks_and_rejects_pending_and_triple_overlaps()
        -> motion::Result {
            let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
            let make = |id, color| {
                let calls = calls.clone();
                (move |context: &SceneContext<'_>| {
                    calls.lock().unwrap().push((
                        id,
                        context.time(),
                        context.global_time(),
                        context.clip_duration(),
                    ));
                    Node::from(context.bounds().fill(color))
                })
                .with_duration(2.seconds())
            };
            let timeline = Timeline::try_new(size(), 10)?
                .try_track(|track| {
                    track
                        .try_push(make(0, Color::rgb(255, 0, 0)))?
                        .try_transition(motion::authoring::transition::CrossFade, 1.seconds())?
                        .try_push(make(1, Color::rgb(0, 0, 255)))
                })?
                .build()?;
            assert_eq!(timeline.frame_count(), 30);
            let cache = SceneCache::new(0);
            let context = SceneContext::new(
                1500.milliseconds().after_origin(),
                FrameRate::from(10),
                size(),
                &cache,
            );
            let frame = timeline
                .render(&context)
                .compile(size(), PixelSize::new(8, 4), ValidationLimits::default())?
                .prepare(ValidationLimits::default(), RasterLimits::default())?
                .render_linear(RasterLimits::default())?;
            assert_eq!(frame.linear_pixel(1, 1), Some([0.5, 0.0, 0.5, 1.0]));
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                &[
                    (
                        0,
                        1500.milliseconds().after_origin(),
                        1500.milliseconds().after_origin(),
                        Some(2.seconds())
                    ),
                    (
                        1,
                        500.milliseconds().after_origin(),
                        1500.milliseconds().after_origin(),
                        Some(2.seconds())
                    )
                ]
            );
            assert!(
                Timeline::try_new(size(), 10)?
                    .try_track(|track| track
                        .try_transition(motion::authoring::transition::CrossFade, 1.seconds()))
                    .is_err()
            );
            assert!(
                Timeline::try_new(size(), 10)?
                    .try_track(|track| track
                        .try_push(scene(2.seconds()))?
                        .try_transition(motion::authoring::transition::CrossFade, 1.seconds()))
                    .is_err()
            );
            assert!(
                Timeline::try_new(size(), 10)?
                    .try_track(|track| track
                        .try_push(scene(2.seconds()))?
                        .try_transition(
                            motion::authoring::transition::CrossFade,
                            1500.milliseconds()
                        )?
                        .try_push(scene(2.seconds()))?
                        .try_transition(motion::authoring::transition::CrossFade, 1.seconds()))
                    .is_err()
            );
            assert!(
                Timeline::try_new(size(), 10)?
                    .try_track(|track| track
                        .try_push(scene(2.seconds()))?
                        .try_transition(motion::authoring::transition::CrossFade, 1.seconds())?
                        .try_push(scene(500.milliseconds())))
                    .is_err()
            );
            Ok(())
        }

        #[test]
        fn custom_transition_receives_overlap_local_time_and_easing() -> motion::Result {
            struct Eased;

            impl motion::authoring::transition::Transition for Eased {
                fn easing(&self) -> motion::Easing {
                    motion::Easing::QuadraticIn
                }

                fn compose(
                    &self,
                    progress: f64,
                    outgoing: Node,
                    incoming: Node,
                    context: &SceneContext<'_>,
                ) -> Node {
                    assert_eq!(progress, 0.25);
                    assert_eq!(context.time(), 500.milliseconds().after_origin());
                    assert_eq!(context.global_time(), 1500.milliseconds().after_origin());
                    assert_eq!(context.clip_duration(), Some(1.seconds()));
                    motion::authoring::transition::Transition::compose(
                        &motion::authoring::transition::CrossFade,
                        progress,
                        outgoing,
                        incoming,
                        context,
                    )
                }
            }
            let timeline = Timeline::try_new(size(), 10)?
                .try_track(|track| {
                    track
                        .try_push(
                            (|context: &SceneContext<'_>| {
                                Node::from(context.bounds().fill(Color::rgb(255, 0, 0)))
                            })
                            .with_duration(2.seconds()),
                        )?
                        .try_transition(Eased, 1.seconds())?
                        .try_push(
                            (|context: &SceneContext<'_>| {
                                Node::from(context.bounds().fill(Color::rgb(0, 0, 255)))
                            })
                            .with_duration(2.seconds()),
                        )
                })?
                .build()?;
            let cache = SceneCache::new(0);
            let context = SceneContext::new(
                1500.milliseconds().after_origin(),
                FrameRate::from(10),
                size(),
                &cache,
            );
            let frame = timeline
                .render(&context)
                .compile(size(), PixelSize::new(8, 4), ValidationLimits::default())?
                .prepare(ValidationLimits::default(), RasterLimits::default())?
                .render_linear(RasterLimits::default())?;
            assert_eq!(frame.linear_pixel(1, 1), Some([0.75, 0.0, 0.25, 1.0]));
            Ok(())
        }

        #[test]
        fn nested_timeline_contain_is_centered_and_clips_content_before_mapping() -> motion::Result
        {
            let child = Timeline::try_new(Size::new(8.0, 4.0), 24)?
                .try_track(|track| {
                    track.try_push(
                        (|_: &SceneContext<'_>| {
                            Node::from(motion::Rect::new(-8.0, -4.0, 16.0, 8.0).fill(Color::WHITE))
                        })
                        .with_duration(1.seconds()),
                    )
                })?
                .build()?;
            let cache = SceneCache::new(0);
            for (parent_size, expected) in [
                (
                    Size::new(12.0, 12.0),
                    motion::Rect::new(0.0, 3.0, 12.0, 9.0),
                ),
                (Size::new(4.0, 12.0), motion::Rect::new(0.0, 5.0, 4.0, 7.0)),
            ] {
                let context =
                    SceneContext::new(Time::ZERO, FrameRate::from(30), parent_size, &cache);
                let pixels = PixelSize::new(parent_size.width as u32, parent_size.height as u32);
                let frame = context
                    .embed(&child.visual(), Time::ZERO)
                    .compile(parent_size, pixels, ValidationLimits::default())?
                    .prepare(ValidationLimits::default(), RasterLimits::default())?
                    .render_linear(RasterLimits::default())?;
                for row in 0..pixels.height {
                    for column in 0..pixels.width {
                        let inside = f64::from(column) + 0.5 >= expected.left
                            && f64::from(column) + 0.5 < expected.right
                            && f64::from(row) + 0.5 >= expected.top
                            && f64::from(row) + 0.5 < expected.bottom;
                        assert_eq!(
                            frame.linear_pixel(column, row),
                            Some(if inside { [1.0; 4] } else { [0.0; 4] })
                        );
                    }
                }
            }
            Ok(())
        }

        #[test]
        fn nested_equal_viewports_clip_after_placement_and_three_levels_compose_once()
        -> motion::Result {
            let cache = SceneCache::new(0);
            let child = Timeline::try_new(Size::new(8.0, 4.0), 30)?
                .try_track(|track| {
                    track.try_push(
                        (|_: &SceneContext<'_>| {
                            Node::from(motion::Rect::new(-8.0, -4.0, 16.0, 8.0).fill(Color::WHITE))
                        })
                        .with_duration(1.seconds()),
                    )
                })?
                .build()?;
            let context = SceneContext::new(Time::ZERO, FrameRate::from(30), child.size(), &cache);
            let shifted = context
                .embed(&child.visual(), Time::ZERO)
                .translate(4.0, 0.0)
                .compile(
                    child.size(),
                    PixelSize::new(8, 4),
                    ValidationLimits::default(),
                )?
                .prepare(ValidationLimits::default(), RasterLimits::default())?
                .render_linear(RasterLimits::default())?;
            for column in 0..8 {
                assert_eq!(
                    shifted.linear_pixel(column, 1),
                    Some(if column >= 4 { [1.0; 4] } else { [0.0; 4] })
                );
            }
            let inner = Timeline::try_new(Size::new(8.0, 8.0), 24)?
                .background(Color::WHITE)
                .try_duration(1.seconds())?
                .build()?;
            let middle = Timeline::try_new(Size::new(16.0, 8.0), 30)?
                .try_track(|track| track.try_nest(inner))?
                .build()?;
            let outer = Timeline::try_new(Size::new(8.0, 12.0), 60)?
                .try_track(|track| track.try_nest(middle))?
                .build()?;
            let context = SceneContext::new(Time::ZERO, FrameRate::from(60), outer.size(), &cache);
            let frame = outer
                .render(&context)
                .compile(
                    outer.size(),
                    PixelSize::new(8, 12),
                    ValidationLimits::default(),
                )?
                .prepare(ValidationLimits::default(), RasterLimits::default())?
                .render_linear(RasterLimits::default())?;
            for row in 0..12 {
                for column in 0..8 {
                    assert_eq!(
                        frame.linear_pixel(column, row),
                        Some(if (2..6).contains(&column) && (4..8).contains(&row) {
                            [1.0; 4]
                        } else {
                            [0.0; 4]
                        })
                    );
                }
            }
            let tiny = Timeline::try_new(Size::new(1e-300, 1e-300), 30)?
                .try_duration(1.seconds())?
                .build()?;
            let huge_context = SceneContext::new(
                Time::ZERO,
                FrameRate::from(30),
                Size::new(1e300, 1e300),
                &cache,
            );
            assert!(
                tiny.render(&huge_context)
                    .compile(
                        Size::new(1.0, 1.0),
                        PixelSize::new(1, 1),
                        ValidationLimits::default()
                    )
                    .is_err()
            );
            Ok(())
        }

        #[test]
        fn multitrack_clocks_resources_gaps_paint_order_and_overrides_are_exact() -> motion::Result
        {
            let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
            let fonts = motion::FontLibrary::new();
            let identity = fonts.snapshot_id();
            let resources = motion::Assets::new(".")?.with_fonts(fonts);
            let make = |id, color: Color, z_index| {
                let calls = calls.clone();
                (move |context: &SceneContext<'_>| {
                    assert_eq!(context.size(), size());
                    assert_eq!(context.frame_rate(), FrameRate::from(10));
                    assert_eq!(context.assets().fonts().unwrap().snapshot_id(), identity);
                    calls.lock().unwrap().push((
                        id,
                        context.time(),
                        context.global_time(),
                        context.clip_duration(),
                    ));
                    Node::from(size().to_rect().fill(color)).z_index(z_index)
                })
                .with_duration(1.seconds())
            };
            let timeline = Timeline::try_new(size(), 10)?
                .background(Color::BLACK)
                .try_track(|track| {
                    track
                        .try_push(make(0, Color::rgb(255, 0, 0), i32::MAX))?
                        .try_push(make(1, Color::rgb(0, 255, 0), 0))?
                        .try_gap(500.milliseconds())
                })?
                .try_track(|track| {
                    track.try_place(
                        500.milliseconds().after_origin(),
                        make(2, Color::rgb(0, 0, 255).with_alpha(0.5), i32::MIN),
                    )
                })?
                .build()?;
            assert_eq!(timeline.duration(), 2500.milliseconds());
            let cache = SceneCache::new(0);
            for (time, color, ids) in [
                (Time::from_ticks(-1), [0.0; 4], vec![]),
                (Time::ZERO, [1.0, 0.0, 0.0, 1.0], vec![0]),
                (
                    500.milliseconds().after_origin(),
                    [0.5, 0.0, 0.5, 1.0],
                    vec![0, 2],
                ),
                (1.seconds().after_origin(), [0.0, 0.5, 0.5, 1.0], vec![1, 2]),
                (
                    1500.milliseconds().after_origin(),
                    [0.0, 1.0, 0.0, 1.0],
                    vec![1],
                ),
                (2.seconds().after_origin(), [0.0, 0.0, 0.0, 1.0], vec![]),
                (2500.milliseconds().after_origin(), [0.0; 4], vec![]),
            ] {
                calls.lock().unwrap().clear();
                let context = SceneContext::new(
                    time + 5.seconds(),
                    FrameRate::from(24),
                    Size::new(16.0, 8.0),
                    &cache,
                )
                .shifted(5.seconds())
                .with_clip_duration(99.seconds())
                .with_assets(&resources);
                let frame = timeline
                    .render(&context)
                    .compile(size(), PixelSize::new(8, 4), ValidationLimits::default())?
                    .prepare(ValidationLimits::default(), RasterLimits::default())?
                    .render_linear(RasterLimits::default())?;
                assert_eq!(frame.linear_pixel(2, 1), Some(color));
                let recorded = calls.lock().unwrap();
                assert_eq!(recorded.iter().map(|call| call.0).collect::<Vec<_>>(), ids);
                for (id, local, global, duration) in recorded.iter() {
                    let start = [
                        Time::ZERO,
                        1.seconds().after_origin(),
                        500.milliseconds().after_origin(),
                    ][*id];
                    assert_eq!(*local, (time - start).after_origin());
                    assert_eq!(*global, time + 5.seconds());
                    assert_eq!(*duration, Some(1.seconds()));
                }
            }
            assert_eq!(
                timeline
                    .to_builder()
                    .try_duration(1.seconds())?
                    .build()?
                    .frame_count(),
                10
            );
            assert_eq!(
                timeline
                    .to_builder()
                    .try_duration(3.seconds())?
                    .build()?
                    .frame_count(),
                30
            );
            let empty = timeline
                .to_builder()
                .try_duration(Duration::ZERO)?
                .build()?;
            let context = SceneContext::new(Time::ZERO, FrameRate::from(10), size(), &cache);
            assert!(
                empty
                    .render(&context)
                    .compile(size(), PixelSize::new(8, 4), ValidationLimits::default())?
                    .commands
                    .is_empty()
            );
            assert_eq!(timeline.frame_count(), 25);
            Ok(())
        }

        #[test]
        fn timeline_rendering_is_replayable_concurrent_and_clipped_by_explicit_duration()
        -> motion::Result {
            let timeline = Timeline::try_new(size(), 30)?
                .background(Color::BLACK)
                .try_track(|track| {
                    track.try_push(
                        (|context: &SceneContext<'_>| {
                            Node::from(
                                motion::Rect::new(0.0, 0.0, 2.0 + context.progress() * 4.0, 4.0)
                                    .fill(Color::WHITE),
                            )
                        })
                        .with_duration(1.seconds()),
                    )
                })?
                .try_duration(500.milliseconds())?
                .build()?;
            let composition = motion::render::Composition::new(
                8,
                4,
                motion::render::FrameClock::new(timeline.frame_rate(), timeline.frame_count()),
            );
            let build = |context: &SceneContext<'_>| timeline.render(context);
            let late = composition.render_frame(14, size(), build)?;
            let early = composition.render_frame(0, size(), build)?;
            assert_ne!(late.packed_rgb(), early.packed_rgb());
            assert_eq!(
                late.packed_rgb(),
                composition.render_frame(14, size(), build)?.packed_rgb()
            );
            std::thread::scope(|scope| {
                let tasks = (0..3)
                    .map(|_| scope.spawn(|| composition.render_frame(14, size(), build).unwrap()))
                    .collect::<Vec<_>>();
                for task in tasks {
                    assert_eq!(task.join().unwrap().packed_rgb(), late.packed_rgb());
                }
            });
            let cache = SceneCache::new(0);
            let at_end = SceneContext::new(
                500.milliseconds().after_origin(),
                FrameRate::from(30),
                size(),
                &cache,
            );
            assert!(
                timeline
                    .render(&at_end)
                    .compile(size(), PixelSize::new(8, 4), ValidationLimits::default())?
                    .commands
                    .is_empty()
            );
            Ok(())
        }

        #[test]
        #[cfg(feature = "gpu")]
        #[ignore = "requires native GPU; no fonts, window or encoder"]
        fn multitrack_gpu_matches_cpu_at_ntsc_and_clip_boundaries() -> motion::Result {
            let design = Size::new(32.0, 16.0);
            let validation = ValidationLimits::default();
            let limits = RasterLimits::default();
            let mut gpu = motion::render::gpu::Gpu::new()?;
            let mut maximum = 0.0f32;
            let mut bytes = 0u8;
            let mut requests = 0;
            let cache = SceneCache::new(0);
            for rate in [FrameRate::from(30), FrameRate::NTSC] {
                let timeline = Timeline::try_new(design, rate)?
                    .background(Color::rgb(10, 20, 30))
                    .try_track(|track| {
                        track
                            .try_push(
                                (move |context: &SceneContext<'_>| {
                                    Node::from(
                                        motion::Rect::new(
                                            0.25 + 4.0 * context.progress(),
                                            1.5,
                                            25.75,
                                            14.25,
                                        )
                                        .fill(Color::rgb(230, 40, 60)),
                                    )
                                    .z_index(100)
                                })
                                .with_duration(1.seconds()),
                            )?
                            .try_gap(500.milliseconds())?
                            .try_push(
                                (|_: &SceneContext<'_>| {
                                    Node::from(
                                        motion::Rect::new(2.5, 0.5, 30.25, 12.75)
                                            .fill(Color::rgb(20, 210, 80)),
                                    )
                                })
                                .with_duration(1.seconds()),
                            )
                    })?
                    .try_track(|track| {
                        track.try_place(
                            500.milliseconds().after_origin(),
                            (|context: &SceneContext<'_>| {
                                Node::from(
                                    motion::Rect::new(8.5, 2.25, 29.5, 15.0)
                                        .fill(Color::rgb(20, 70, 235)),
                                )
                                .opacity(0.25 + context.progress() * 0.5)
                            })
                            .with_duration(1500.milliseconds()),
                        )
                    })?
                    .build()?;
                for index in [0, 14, 15, 29, 30, 44, 45, 59, 60, 74, 75, 14] {
                    let context =
                        SceneContext::new(Time::from_frame(index, rate), rate, design, &cache);
                    let prepared = timeline
                        .render(&context)
                        .compile(design, PixelSize::new(32, 16), validation)?
                        .prepare(validation, limits)?;
                    let expected = prepared.render_linear(limits)?;
                    let actual = gpu.render_linear(
                        &prepared,
                        motion::render::gpu::Plan::new(&prepared, limits)?,
                    )?;
                    for (left, right) in actual.linear_pixels().zip(expected.linear_pixels()) {
                        for (left, right) in left.iter().zip(right) {
                            maximum = maximum.max((left - right).abs());
                            assert!((left - right).abs() <= 3e-6);
                        }
                    }
                    let expected = expected.to_format(
                        motion::render::PixelFormat::SrgbRgba8 {
                            premultiplied: false,
                        },
                        limits,
                    )?;
                    let actual = actual.to_format(
                        motion::render::PixelFormat::SrgbRgba8 {
                            premultiplied: false,
                        },
                        limits,
                    )?;
                    for (left, right) in actual.bytes().iter().zip(expected.bytes()) {
                        bytes = bytes.max(left.abs_diff(*right));
                        assert!(left.abs_diff(*right) <= 1);
                    }
                    let replay = gpu
                        .render_linear(
                            &prepared,
                            motion::render::gpu::Plan::new(&prepared, limits)?,
                        )?
                        .to_format(
                            motion::render::PixelFormat::SrgbRgba8 {
                                premultiplied: false,
                            },
                            limits,
                        )?;
                    assert_eq!(replay, actual);
                    requests += 1;
                }
            }
            println!(
                "timeline GPU: adapter={} backend={:?} requests={requests} max_linear={maximum:e} max_rgba={bytes} replay=exact",
                gpu.info().name,
                gpu.info().backend
            );
            Ok(())
        }

        #[test]
        #[cfg(feature = "gpu")]
        #[ignore = "requires native GPU and explicit MOTION_TEST_FONT; no font discovery"]
        fn nested_camera_baseline_layout_matches_cpu_and_keeps_hud_in_parent_space()
        -> motion::Result {
            let mut fonts = motion::FontLibrary::new();
            fonts
                .load_file(std::env::var_os("MOTION_TEST_FONT").expect("explicit font fixture"))?;
            let identity = fonts.snapshot_id();
            let resources = motion::Assets::new(".")?.with_fonts(fonts.clone());
            let scene_fonts = fonts.clone();
            let child_size = Size::new(128.0, 64.0);
            let child = Timeline::try_new(child_size, 24)?
                .try_track(|track| {
                    track.try_push(
                        (move |context: &SceneContext<'_>| {
                            assert_eq!(context.size(), child_size);
                            assert_eq!(context.frame_rate(), FrameRate::from(24));
                            assert_eq!(context.assets().fonts().unwrap().snapshot_id(), identity);
                            let progress = context.progress();
                            let world = Node::group([
                                Node::from(
                                    motion::Rect::new(-64.0, -64.0, 192.0, 128.0)
                                        .fill(Color::rgb(40, 100, 130)),
                                ),
                                Node::from(
                                    motion::Rect::new(70.0, 20.0, 95.0, 50.0)
                                        .fill(Color::rgb(240, 100, 60)),
                                ),
                            ]);
                            let world = motion::try_camera(
                                context,
                                world,
                                motion::Camera {
                                    center: motion::Point::new(64.0 + progress * 12.0, 32.0),
                                    zoom: 1.0 + progress * 0.2,
                                    rotation: progress * 0.15,
                                },
                            )
                            .unwrap();
                            let animated = motion::Text::new("A")
                                .size(18.0)
                                .fonts(&scene_fonts)
                                .try_animate_parts_at(context, Duration::ZERO, |part| {
                                    let amount = part.sample(&motion::Tween::new(
                                        0.0,
                                        1.0,
                                        200.milliseconds(),
                                    ));
                                    motion::PartStyle::new()
                                        .scale(0.8 + amount * 0.2)
                                        .offset(0.0, 2.0 * (1.0 - amount))
                                        .opacity(amount)
                                })
                                .unwrap();
                            let labels = motion::Row::new()
                                .alignment(motion::Alignment::Baseline)
                                .gap(4.0)
                                .children([
                                    animated,
                                    Node::from(
                                        motion::Text::new("B").size(12.0).fonts(&scene_fonts),
                                    ),
                                ])
                                .translate(8.0, 8.0);
                            Node::group([
                                world,
                                labels,
                                Node::from(
                                    motion::Rect::new(2.0, 2.0, 6.0, 6.0)
                                        .fill(Color::rgb(0, 255, 0)),
                                ),
                            ])
                        })
                        .with_duration(1500.milliseconds()),
                    )
                })?
                .build()?;
            let validation = ValidationLimits::default();
            let limits = RasterLimits::default();
            let cache = SceneCache::new(0);
            let mut gpu = motion::render::gpu::Gpu::new()?;
            let mut maximum = 0.0f32;
            let mut bytes = 0u8;
            let mut requests = 0;
            for design in [
                Size::new(128.0, 96.0),
                Size::new(96.0, 96.0),
                Size::new(64.0, 128.0),
            ] {
                let parent = Timeline::try_new(design, FrameRate::NTSC)?
                    .try_track(|track| track.try_gap(500.milliseconds())?.try_nest(child.clone()))?
                    .build()?;
                let start = parent.tracks()[0].clips()[0].start();
                let scale =
                    (design.width / child_size.width).min(design.height / child_size.height);
                let offset = (design.height - child_size.height * scale) * 0.5;
                let mut seen = std::collections::BTreeMap::new();
                for index in [0, 14, 15, 29, 30, 44, 59, 60, 30] {
                    let time = Time::from_frame(index, parent.frame_rate());
                    let context = SceneContext::new(time, parent.frame_rate(), design, &cache)
                        .with_assets(&resources);
                    let frame = parent
                        .render(&context)
                        .compile_with_fonts(
                            design,
                            PixelSize::new(design.width as u32, design.height as u32),
                            validation,
                            &fonts,
                        )?
                        .prepare(validation, limits)?;
                    let expected = frame.render_linear(limits)?;
                    assert_eq!(expected.linear_pixel(0, 0), Some([0.0; 4]));
                    if time >= start && time < start + child.duration() {
                        assert_eq!(
                            expected
                                .linear_pixel((4.0 * scale) as u32, (offset + 4.0 * scale) as u32),
                            Some([0.0, 1.0, 0.0, 1.0])
                        );
                        assert!(expected.linear_pixels().any(|pixel| pixel[3] > 0.9));
                    } else {
                        assert!(expected.linear_pixels().all(|pixel| pixel[3] == 0.0));
                    }
                    let actual =
                        gpu.render_linear(&frame, motion::render::gpu::Plan::new(&frame, limits)?)?;
                    for (left, right) in actual.linear_pixels().zip(expected.linear_pixels()) {
                        for (left, right) in left.iter().zip(right) {
                            maximum = maximum.max((left - right).abs());
                            assert!((left - right).abs() <= 3e-6);
                        }
                    }
                    let expected = expected.to_format(
                        motion::render::PixelFormat::SrgbRgba8 {
                            premultiplied: false,
                        },
                        limits,
                    )?;
                    let actual = actual.to_format(
                        motion::render::PixelFormat::SrgbRgba8 {
                            premultiplied: false,
                        },
                        limits,
                    )?;
                    if let Some(previous) = seen.insert(index, expected.clone()) {
                        assert_eq!(previous, expected);
                    }
                    for (left, right) in actual.bytes().iter().zip(expected.bytes()) {
                        bytes = bytes.max(left.abs_diff(*right));
                        assert!(left.abs_diff(*right) <= 1);
                    }
                    assert_eq!(
                        gpu.render_linear(&frame, motion::render::gpu::Plan::new(&frame, limits)?)?
                            .to_format(
                                motion::render::PixelFormat::SrgbRgba8 {
                                    premultiplied: false
                                },
                                limits
                            )?,
                        actual
                    );
                    requests += 1;
                }
            }
            println!(
                "nested camera/baseline GPU: adapter={} backend={:?} requests={requests} max_linear={maximum:e} max_rgba={bytes} replay/HUD/letterbox/resources=pass",
                gpu.info().name,
                gpu.info().backend
            );
            Ok(())
        }
    }

    #[cfg(feature = "gpu")]
    mod authoring_transition {
        use motion::extend::*;
        use motion::prelude::*;

        use motion::render::{RasterLimits, SceneCache, ValidationLimits};
        use motion::{FrameRate, PixelSize, Size, Time};

        #[test]
        #[cfg(feature = "gpu")]
        #[ignore = "requires native GPU; no fonts, window or encoder"]
        fn transition_and_temporal_captures_match_gpu_in_both_blend_spaces() -> motion::Result {
            let cache = SceneCache::new(0);
            let context =
                SceneContext::new(Time::ZERO, FrameRate::NTSC, Size::new(8.0, 6.0), &cache);
            let outgoing = Node::group([
                Node::from(
                    context
                        .bounds()
                        .fill(Color::rgb(230, 30, 50).with_alpha(0.45)),
                ),
                Node::from(
                    Rect::new(0.25, 0.5, 3.5, 5.25).fill(Color::rgb(20, 200, 80).with_alpha(0.8)),
                ),
            ]);
            let incoming = Node::group([
                Node::from(
                    context
                        .bounds()
                        .fill(Color::rgb(30, 60, 230).with_alpha(0.6)),
                ),
                Node::from(Rect::new(4.5, 0.25, 7.5, 4.5).fill(Color::WHITE.with_alpha(0.3))),
            ]);
            let mut effects: Vec<Box<dyn Transition>> = vec![
                Box::new(CrossFade),
                Box::new(Wipe::default()),
                Box::new(Wipe {
                    angle: 0.73,
                    feather: 2.0,
                }),
                Box::new(Wipe {
                    angle: std::f64::consts::FRAC_PI_2,
                    feather: 0.0,
                }),
                Box::new(Iris::default()),
                Box::new(Iris {
                    shape: IrisShape::Rectangle,
                }),
                Box::new(ZoomBlur),
            ];
            for direction in [
                Direction::Left,
                Direction::Right,
                Direction::Up,
                Direction::Down,
            ] {
                effects.push(Box::new(Slide { direction }));
                effects.push(Box::new(Push { direction }));
            }
            let limits = RasterLimits::default();
            let validation = ValidationLimits::default();
            let mut gpu = motion::render::gpu::Gpu::new()?;
            let mut maximum = 0.0f32;
            let mut bytes = 0u8;
            let mut cases = 0;
            for space in [motion::BlendSpace::LinearRgb, motion::BlendSpace::Srgb] {
                let mut nodes = Vec::new();
                for effect in &effects {
                    for progress in [0.0, 0.25, 0.5, 0.75, 1.0, 0.5] {
                        nodes.push(effect.compose(
                            progress,
                            outgoing.clone(),
                            incoming.clone(),
                            &context,
                        ));
                    }
                }
                nodes.push(
                    context.try_motion_blur(Duration::from_ticks(8), 4, |sample| {
                        let column = ((sample.time().ticks() + 3) / 2) as f64;
                        Node::from(
                            Rect::new(column, 0.0, column + 1.0, 1.0)
                                .fill(Color::rgb(255, 0, 0).with_alpha(0.5)),
                        )
                    })?,
                );
                for (index, node) in nodes.into_iter().enumerate() {
                    let frame = node
                        .compile(context.size(), PixelSize::new(8, 6), validation)?
                        .prepare(validation, limits)?
                        .blend_space(space);
                    let expected = frame.render_linear(limits)?;
                    let actual =
                        gpu.render_linear(&frame, motion::render::gpu::Plan::new(&frame, limits)?)?;
                    for (left, right) in actual.linear_pixels().zip(expected.linear_pixels()) {
                        for (left, right) in left.iter().zip(right) {
                            maximum = maximum.max((left - right).abs());
                            assert!(
                                (left - right).abs() <= 4e-6,
                                "space={space:?} case={index}: {left} vs {right}"
                            );
                        }
                    }
                    let expected = expected.to_format(
                        motion::render::PixelFormat::SrgbRgba8 {
                            premultiplied: false,
                        },
                        limits,
                    )?;
                    let actual = actual.to_format(
                        motion::render::PixelFormat::SrgbRgba8 {
                            premultiplied: false,
                        },
                        limits,
                    )?;
                    for (left, right) in actual.bytes().iter().zip(expected.bytes()) {
                        bytes = bytes.max(left.abs_diff(*right));
                        assert!(left.abs_diff(*right) <= 1, "space={space:?} case={index}");
                    }
                    assert_eq!(
                        gpu.render_linear(&frame, motion::render::gpu::Plan::new(&frame, limits)?)?
                            .to_format(
                                motion::render::PixelFormat::SrgbRgba8 {
                                    premultiplied: false
                                },
                                limits
                            )?,
                        actual
                    );
                    cases += 1;
                }
            }
            println!(
                "transitions/shutter GPU: adapter={} backend={:?} cases={cases} max_linear={maximum:e} max_rgba={bytes} replay=exact",
                gpu.info().name,
                gpu.info().backend
            );
            Ok(())
        }
    }

    mod camera {
        use motion::prelude::*;
        use motion::render::*;
        use motion::try_camera;

        #[test]
        fn camera_centers_inverts_and_renders_against_independent_rectangles() -> motion::Result {
            let cache = motion::render::SceneCache::new(0);
            let context = motion::SceneContext::new(
                motion::Time::ZERO,
                motion::FrameRate::from(30),
                motion::Size::new(8.0, 8.0),
                &cache,
            );
            for (rotation, bounds) in [
                (0.0, motion::Rect::new(2.0, 3.0, 6.0, 5.0)),
                (
                    std::f64::consts::FRAC_PI_2,
                    motion::Rect::new(3.0, 2.0, 5.0, 6.0),
                ),
            ] {
                let view = Camera {
                    center: motion::Point::new(10.0, 5.0),
                    zoom: 2.0,
                    rotation,
                };
                assert_eq!(
                    view.world_to_screen(&context, view.center)?,
                    context.center()
                );
                for point in [
                    motion::Point::ORIGIN,
                    view.center,
                    motion::Point::new(-100.0, 40.0),
                ] {
                    let result =
                        view.screen_to_world(&context, view.world_to_screen(&context, point)?)?;
                    assert!(
                        (result.x - point.x).abs() < 1e-12 && (result.y - point.y).abs() < 1e-12
                    );
                }
                let node = try_camera(
                    &context,
                    motion::Rect::new(9.0, 4.5, 11.0, 5.5).fill(motion::Color::WHITE),
                    view,
                )?;
                let frame = node
                    .compile(
                        context.size(),
                        motion::PixelSize::new(8, 8),
                        motion::render::ValidationLimits::default(),
                    )?
                    .prepare(
                        motion::render::ValidationLimits::default(),
                        motion::render::RasterLimits::default(),
                    )?
                    .render_linear(motion::render::RasterLimits::default())?;
                for row in 0..8 {
                    for column in 0..8 {
                        let inside = f64::from(column) >= bounds.left
                            && f64::from(column) < bounds.right
                            && f64::from(row) >= bounds.top
                            && f64::from(row) < bounds.bottom;
                        assert_eq!(
                            frame.linear_pixel(column, row),
                            Some(if inside { [1.0; 4] } else { [0.0; 4] })
                        );
                    }
                }
            }
            Ok(())
        }

        #[test]
        #[ignore = "requires native GPU when enabled; no fonts or window"]
        fn camera_pixels_keep_viewport_hud_and_seek_replay() -> motion::Result {
            use motion::render::{Composition, SceneCache};
            use motion::{
                Color, Node, NodeModifiers, Point, Rect, Renderer, SceneContext, Size, Time,
            };

            #[cfg(feature = "gpu")]
            let backends = [Renderer::Cpu, Renderer::Gpu];
            #[cfg(not(feature = "gpu"))]
            let backends = [Renderer::Cpu];
            let mut software = std::collections::BTreeMap::<u32, Vec<u32>>::new();
            let marker_color = Color::from_hex(0xd45d49);
            for backend in backends {
                let composition = Composition::new(64, 64, motion::render::FrameClock::new(30, 10))
                    .with_options(
                        motion::render::RenderOptions::default()
                            .backend(backend.into())
                            .background(Color::WHITE),
                    )?;
                let mut renderer = composition.frame_renderer(Size::new(64.0, 64.0))?;
                let mut references = std::collections::BTreeMap::<u32, Vec<u32>>::new();
                for index in [5, 0, 2, 1, 9, 5, 2] {
                    let position = Point::new(f64::from(index) * 3.0, f64::from(index) * -2.0);
                    let angle = [0.0_f64, 90.0, -90.0][index as usize % 3];
                    let camera = Camera {
                        center: position,
                        zoom: 1.0 + f64::from(index % 2),
                        rotation: angle.to_radians(),
                    };
                    let cache = SceneCache::new(0);
                    let context = SceneContext::new(
                        Time::from_frame(i64::from(index), motion::FrameRate::from(30)),
                        motion::FrameRate::from(30),
                        Size::new(48.0, 36.0),
                        &cache,
                    );
                    let world = Node::group([
                        Node::from(
                            Rect::new(
                                position.x - 512.0,
                                position.y - 512.0,
                                position.x + 512.0,
                                position.y + 512.0,
                            )
                            .fill(Color::from_hex(0x2d66e5)),
                        ),
                        Node::from(
                            Rect::new(
                                position.x - 2.0,
                                position.y - 2.0,
                                position.x + 2.0,
                                position.y + 2.0,
                            )
                            .fill(Color::from_hex(0x118677)),
                        ),
                        Node::from(
                            Rect::new(
                                position.x + 7.0,
                                position.y - 1.0,
                                position.x + 9.0,
                                position.y + 1.0,
                            )
                            .fill(marker_color),
                        ),
                    ]);
                    let scene = Node::group([
                        Node::group([
                            motion::try_camera(&context, world, camera)?.translate(8.0, 12.0)
                        ])
                        .clip_with(
                            Rect::new(8.0, 12.0, 56.0, 48.0),
                            motion::ClipMode::Geometric,
                        ),
                        Node::from(Rect::new(1.0, 1.0, 4.0, 4.0).fill(Color::BLACK)),
                    ]);
                    let snapshot = renderer.render_frame(index, |_| scene)?;
                    let pixels = snapshot.packed_rgb();
                    assert_eq!(pixels[30 * 64 + 32], 0x118677);
                    assert_eq!(pixels[13 * 64 + 9], 0x2d66e5);
                    let marker = camera
                        .world_to_screen(&context, Point::new(position.x + 8.0, position.y))?
                        + motion::Vector2::new(8.0, 12.0);
                    assert_eq!(
                        pixels[marker.y.round() as usize * 64 + marker.x.round() as usize],
                        0xd45d49
                    );
                    for row in 0..64 {
                        for column in 0..64 {
                            if !(8..56).contains(&column) || !(12..48).contains(&row) {
                                let expected = if (1..4).contains(&column) && (1..4).contains(&row)
                                {
                                    Color::BLACK
                                } else {
                                    Color::WHITE
                                };
                                assert_eq!(
                                    pixels[row * 64 + column],
                                    if expected == Color::BLACK {
                                        0
                                    } else {
                                        0xffffff
                                    },
                                    "outside viewport at {column},{row}"
                                );
                            }
                        }
                    }
                    if let Some(reference) = references.get(&index) {
                        assert!(pixels == *reference, "camera replay at {index}");
                    }
                    references.insert(index, pixels.to_vec());
                    if backend == Renderer::Cpu {
                        software.insert(index, pixels.to_vec());
                    } else {
                        let reference = &software[&index];
                        let difference = pixels
                            .iter()
                            .zip(reference)
                            .flat_map(|(actual, expected)| {
                                [0, 8, 16].map(|shift| {
                                    (((actual >> shift) & 255) as i32
                                        - ((expected >> shift) & 255) as i32)
                                        .abs()
                                })
                            })
                            .max()
                            .unwrap_or(0);
                        assert!(difference <= 3, "camera backend difference: {difference}");
                    }
                }
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod transition_contracts {
        use motion::extend::*;
        use motion::prelude::*;

        use motion::render::{RasterLimits, SceneCache, ValidationLimits};
        use motion::{FrameRate, PixelSize, Size, Time};

        fn pixel(node: Node, space: motion::BlendSpace) -> motion::render::FrameBuffer {
            node.compile(
                Size::new(4.0, 4.0),
                PixelSize::new(4, 4),
                ValidationLimits::default(),
            )
            .unwrap()
            .prepare(ValidationLimits::default(), RasterLimits::default())
            .unwrap()
            .blend_space(space)
            .render_linear(RasterLimits::default())
            .unwrap()
        }

        #[test]
        fn transitions_preserve_transparent_endpoints_and_selected_sources() {
            let cache = SceneCache::new(0);
            let context =
                SceneContext::new(Time::ZERO, FrameRate::from(30), Size::new(4.0, 4.0), &cache);
            let outgoing = Node::from(context.bounds().fill(Color::rgb(255, 0, 0).with_alpha(0.5)));
            let incoming = Node::from(context.bounds().fill(Color::rgb(0, 0, 255).with_alpha(0.5)));
            let effects: Vec<Box<dyn Transition>> = vec![
                Box::new(CrossFade),
                Box::new(Slide {
                    direction: Direction::Left,
                }),
                Box::new(Push {
                    direction: Direction::Left,
                }),
                Box::new(Wipe::default()),
                Box::new(Iris::default()),
                Box::new(ZoomBlur),
            ];
            for effect in &effects {
                for (progress, expected) in
                    [(0.0, [0.5, 0.0, 0.0, 0.5]), (1.0, [0.0, 0.0, 0.5, 0.5])]
                {
                    assert_eq!(
                        pixel(
                            effect.compose(progress, outgoing.clone(), incoming.clone(), &context),
                            motion::BlendSpace::LinearRgb
                        )
                        .linear_pixel(1, 1),
                        Some(expected)
                    );
                }
            }
            for space in [motion::BlendSpace::LinearRgb, motion::BlendSpace::Srgb] {
                let frame = pixel(
                    CrossFade.compose(0.5, outgoing.clone(), incoming.clone(), &context),
                    space,
                );
                for (value, reference) in frame
                    .linear_pixel(1, 1)
                    .unwrap()
                    .iter()
                    .zip([0.25, 0.0, 0.25, 0.5])
                {
                    assert!((value - reference).abs() < 1e-6);
                }
            }
            for effect in [
                Box::new(Slide {
                    direction: Direction::Left,
                }) as Box<dyn Transition>,
                Box::new(Push {
                    direction: Direction::Left,
                }),
                Box::new(Wipe::default()),
            ] {
                let frame = pixel(
                    effect.compose(0.5, outgoing.clone(), incoming.clone(), &context),
                    motion::BlendSpace::LinearRgb,
                );
                let left = if frame.linear_pixel(0, 1).unwrap()[2] > 0.0 {
                    0
                } else {
                    3
                };
                assert_eq!(frame.linear_pixel(left, 1), Some([0.0, 0.0, 0.5, 0.5]));
            }
            assert!(
                CrossFade
                    .compose(f64::NAN, outgoing.clone(), incoming.clone(), &context)
                    .compile(
                        context.size(),
                        PixelSize::new(4, 4),
                        ValidationLimits::default()
                    )
                    .is_err()
            );
            assert!(
                Wipe {
                    angle: 0.0,
                    feather: -1.0
                }
                .compose(0.0, outgoing, incoming, &context)
                .compile(
                    context.size(),
                    PixelSize::new(4, 4),
                    ValidationLimits::default()
                )
                .is_err()
            );
        }

        #[test]
        fn directions_feather_iris_and_radial_zoom_follow_independent_pixel_references() {
            let cache = SceneCache::new(0);
            let context =
                SceneContext::new(Time::ZERO, FrameRate::from(30), Size::new(4.0, 4.0), &cache);
            let outgoing = Node::from(context.bounds().fill(Color::rgb(255, 0, 0).with_alpha(0.5)));
            let incoming = Node::from(context.bounds().fill(Color::rgb(0, 0, 255).with_alpha(0.5)));
            for direction in [
                Direction::Left,
                Direction::Right,
                Direction::Up,
                Direction::Down,
            ] {
                for effect in [
                    Box::new(Slide { direction }) as Box<dyn Transition>,
                    Box::new(Push { direction }),
                ] {
                    let frame = pixel(
                        effect.compose(0.5, outgoing.clone(), incoming.clone(), &context),
                        motion::BlendSpace::LinearRgb,
                    );
                    for row in 0..4 {
                        for column in 0..4 {
                            let blue = match direction {
                                Direction::Left => column >= 2,
                                Direction::Right => column < 2,
                                Direction::Up => row >= 2,
                                Direction::Down => row < 2,
                            };
                            assert_eq!(
                                frame.linear_pixel(column, row),
                                Some(if blue {
                                    [0.0, 0.0, 0.5, 0.5]
                                } else {
                                    [0.5, 0.0, 0.0, 0.5]
                                })
                            );
                        }
                    }
                }
            }
            let feather = pixel(
                Wipe {
                    angle: 0.0,
                    feather: 2.0,
                }
                .compose(0.5, outgoing.clone(), incoming.clone(), &context),
                motion::BlendSpace::LinearRgb,
            );
            for (column, blue) in [1.0, 0.75, 0.25, 0.0].into_iter().enumerate() {
                assert_eq!(
                    feather.linear_pixel(column as u32, 1),
                    Some([0.5 * (1.0 - blue), 0.0, 0.5 * blue, 0.5])
                );
            }
            let iris = pixel(
                Iris {
                    shape: IrisShape::Rectangle,
                }
                .compose(0.5, outgoing.clone(), incoming.clone(), &context),
                motion::BlendSpace::LinearRgb,
            );
            for row in 0..4 {
                for column in 0..4 {
                    assert_eq!(
                        iris.linear_pixel(column, row),
                        Some(if (1..3).contains(&column) && (1..3).contains(&row) {
                            [0.0, 0.0, 0.5, 0.5]
                        } else {
                            [0.5, 0.0, 0.0, 0.5]
                        })
                    );
                }
            }
            let circle = pixel(
                Iris::default().compose(0.5, outgoing.clone(), incoming.clone(), &context),
                motion::BlendSpace::LinearRgb,
            );
            assert_eq!(circle.linear_pixel(2, 2), Some([0.0, 0.0, 0.5, 0.5]));
            assert_eq!(circle.linear_pixel(0, 0), Some([0.5, 0.0, 0.0, 0.5]));
            let zoom = pixel(
                ZoomBlur.compose(0.5, outgoing, incoming, &context),
                motion::BlendSpace::LinearRgb,
            );
            assert!((zoom.linear_pixel(1, 1).unwrap()[2] - 0.25).abs() < 1e-6);
            assert!(zoom.linear_pixel(0, 0).unwrap()[2] < 0.25);
        }
    }
}
