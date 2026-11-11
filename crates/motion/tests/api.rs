//! Integration tests for the canonical public API surface of the facade.

mod canonical_api_tests {

    #[cfg(feature = "particles")]
    use motion::particles::Particle;
    use motion::prelude::*;
    #[cfg(all(feature = "text", feature = "particles"))]
    use motion::render::FrameRenderer;
    use motion::render::{Composition, ValidationLimits};

    #[test]
    fn tuple_groups_support_sixteen_items_empty_groups_and_snapshot_push() -> motion::Result {
        let sixteen = Node::group((
            Circle::new(Point::new(2.0, 2.0), 1.0)?.fill(Color::WHITE),
            Node::empty(),
            Node::empty(),
            Node::empty(),
            Node::empty(),
            Node::empty(),
            Node::empty(),
            Node::empty(),
            Node::empty(),
            Node::empty(),
            Node::empty(),
            Node::empty(),
            Node::empty(),
            Node::empty(),
            Node::empty(),
            Rect::new(4.0, 1.0, 6.0, 3.0).fill(Color::WHITE),
        ));
        assert_eq!(
            sixteen.measure(ValidationLimits::default())?,
            Rect::new(1.0, 1.0, 6.0, 3.0)
        );
        let empty = Node::group(());
        let mut growing = empty.clone();
        growing.push(sixteen);
        assert_eq!(
            empty.measure(ValidationLimits::default())?,
            Rect::new(0.0, 0.0, 0.0, 0.0)
        );
        assert_eq!(
            group![].measure(ValidationLimits::default())?,
            Rect::new(0.0, 0.0, 0.0, 0.0)
        );
        assert_eq!(
            growing.measure(ValidationLimits::default())?,
            Rect::new(1.0, 1.0, 6.0, 3.0)
        );
        Ok(())
    }

    #[test]
    fn typed_arc_audio_sources_forward_reading_and_natural_duration() -> motion::Result {
        use motion::extend::AudioSource;

        struct Samples;

        impl AudioSource for Samples {
            fn sample_rate(&self) -> u32 {
                4
            }

            fn channels(&self) -> u16 {
                1
            }

            fn duration(&self) -> Option<Duration> {
                Some(1.seconds())
            }

            fn read(&self, start_frame: u64, output: &mut [f32]) -> usize {
                let count = 4_usize
                    .saturating_sub(usize::try_from(start_frame).unwrap_or(usize::MAX))
                    .min(output.len());
                output[..count].fill(0.25);
                count
            }
        }
        let source = std::sync::Arc::new(Samples);
        for clip in [AudioClip::source(source.clone()), AudioClip::from(source)] {
            let parameters = clip.parameters();
            assert_eq!(parameters.source.sample_rate(), 4);
            assert_eq!(parameters.source.channels(), 1);
            assert_eq!(parameters.source.duration(), Some(1.seconds()));
            let mut output = [0.0; 4];
            assert_eq!(parameters.source.try_read(2, &mut output)?, 2);
            assert_eq!(output, [0.25, 0.25, 0.0, 0.0]);
        }
        Ok(())
    }

    #[test]
    fn author_prelude_supports_result_tuples_memo_math_and_track_scene() -> motion::Result {
        let timeline = Timeline::new(Size::new(8.0, 4.0), 30)
            .track(|track| {
                track.scene(1_u32.seconds(), |context| -> motion::Result<_> {
                    let title = context.memo(key::F64(2.0), || {
                        Rect::new(0.0, 0.0, 2.0, 2.0).fill(Color::WHITE)
                    });
                    let progress = context.ease_in(Time::ZERO, 1.seconds(), Easing::Linear);
                    Ok((
                        title,
                        Circle::new(Point::new(4.0, 2.0), math::lerp(&0.5, &1.0, progress))?
                            .fill(Color::BLACK),
                    ))
                })
            })
            .build()?;
        assert_eq!(timeline.duration(), 1.seconds());
        render_still(&timeline, Time::ZERO, &RenderOptions::default())?;
        Ok(())
    }

    #[test]
    fn core_time_reexports_keep_type_identity_and_scene_sampling() -> Result<(), Error> {
        let rate: FrameRate = motion_core::FrameRate::NTSC;
        let time: Time = motion_core::Time::from_frame(1, rate);
        let duration: motion_core::Duration = 1.seconds();
        let movement = Tween::new(0.0, 1.0, duration);
        let output = Composition::new(2, 1, motion::render::FrameClock::new(rate, 2))
            .render_frame(1, Size::new(2.0, 1.0), |context| {
                let sampled: motion_core::Time = context.time();
                assert_eq!(sampled, time);
                assert_eq!(context.frame_rate(), rate);
                assert_eq!(context.sample(&movement), time.since_origin() / duration);
                Node::empty()
            })?;
        assert_eq!(
            (output.size().width as usize, output.size().height as usize),
            (2, 1)
        );
        assert_eq!(
            std::any::TypeId::of::<Time>(),
            std::any::TypeId::of::<motion_core::Time>()
        );
        assert_eq!(
            std::any::TypeId::of::<FrameRate>(),
            std::any::TypeId::of::<motion_core::FrameRate>()
        );
        Ok(())
    }

    #[test]
    #[cfg(all(feature = "text", feature = "particles"))]
    fn canonical_root_and_prelude_bind_contracts() -> Result<(), Error> {
        fn scene_contract(_: &dyn Scene) {}

        fn animation_contract(value: &dyn Signal<f64>) -> f64 {
            value.sample(500.milliseconds())
        }
        let callback = |context: &SceneContext<'_>| {
            Node::from(context.bounds().fill(Color::rgb(20, 150, 125)))
        };
        scene_contract(&callback);
        assert_eq!(animation_contract(&Tween::new(0.0, 1.0, 1.seconds())), 0.5);
        let bounds = Rect::new(0.125, 0.25, 8.5, 4.75);
        let _coordinate: f64 = bounds.left;
        let color = Color::from_rgb_float(1.25, 0.5, 0.25);
        assert!(color.to_rgba_float()[0] > 1.0);
        let timeline = Timeline::try_new(Size::new(8.0, 4.0), FrameRate::NTSC)?
            .try_track(|track| track.try_push(callback.with_duration(1.seconds())))?
            .build()?;
        let _: FrameRenderer = Composition::new(8, 4, motion::render::FrameClock::new(30, 150))
            .frame_renderer(timeline.size())?;
        let _span = TextSpan::new("canonical").size(14.0);
        assert_eq!(std::mem::size_of::<Particle>(), 72);
        assert!(Node::empty().measure(ValidationLimits::default()).is_ok());
        Ok(())
    }
}
