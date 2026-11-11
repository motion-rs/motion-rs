//! Integration tests for animation values, easing and sampling through the facade.

mod values {

    mod animation {

        use motion::prelude::*;

        #[test]
        fn design_values_sample_through_local_ticks_tracks_and_arrays() -> motion::Result {
            use motion::{AngleLiteral, Point, TimeLiteral, Transform, Vector2};

            let cache = motion::render::SceneCache::new(0);
            let context = motion::SceneContext::new(
                500.milliseconds().after_origin(),
                motion::FrameRate::NTSC,
                motion::Size::new(64.0, 64.0),
                &cache,
            );
            let from = Point::new(16_777_217.125, -4.0);
            let to = from + Vector2::new(8.0, 12.0);
            assert_eq!(
                context.sample(&Tween::new(from, to, 1.seconds())),
                Point::new(16_777_221.125, 2.0)
            );
            let track = Keyframes::new((from, Vector2::new(0.0, 2.0))).to(
                1.seconds(),
                (to, Vector2::new(4.0, 6.0)),
                Easing::Linear,
            );
            let expected = (Point::new(16_777_221.125, 2.0), Vector2::new(2.0, 4.0));
            for milliseconds in [500, 0, 1_000, 500] {
                let value = track.sample(milliseconds.milliseconds());
                if milliseconds == 500 {
                    assert_eq!(value, expected);
                }
            }
            let transforms = context.sample(&Tween::new(
                [Transform::IDENTITY, Transform::rotate(170.degrees())],
                [
                    Transform::translate(10.0, 20.0),
                    Transform::rotate(-170.degrees()),
                ],
                1.seconds(),
            ));
            assert_eq!(transforms[0].apply(Point::ORIGIN), Point::new(5.0, 10.0));
            let rotated = transforms[1].apply(Point::new(1.0, 0.0));
            assert!((rotated.x + 1.0).abs() < 1e-12 && rotated.y.abs() < 1e-12);
            let empty: [Point; 0] = context.sample(&Tween::new([], [], 1.seconds()));
            assert_eq!(empty, []);
            Ok(())
        }
    }

    mod animation_typed {

        use motion::prelude::*;

        #[test]
        fn frames_sample_typed_values_using_sequence_local_ticks() -> motion::Result {
            let cache = motion::render::SceneCache::new(0);
            let rate = motion::FrameRate::from(60);
            let context = |index: i32| {
                motion::SceneContext::new(
                    Time::from_frame(i64::from(index), rate),
                    rate,
                    motion::Size::new(64.0, 64.0),
                    &cache,
                )
            };
            let local = context(61).shifted(Duration::from_frame(60, rate));
            let movement = Tween::new(0.0_f64, 1.0, 1.seconds());
            let expected = 1.0 / 60.0;
            assert_eq!(local.sample(&movement), expected);
            assert_ne!(local.sample(&movement), f64::from(expected as f32));
            let boxed = movement.boxed();
            assert_eq!(local.sample(boxed.as_ref()), expected);
            for index in [90, 60, 119, 90] {
                let frame = context(index);
                assert_eq!(frame.sample(&boxed), (f64::from(index) / 60.0).min(1.0));
            }
            Ok(())
        }
    }

    mod animation_typed_easing {

        #[test]
        fn curves_integrate_with_tweens_tracks_frames_and_parameter_validation() -> motion::Result {
            use motion::{Easing, Keyframes, Signal, TimeLiteral, Tween};
            let movement = Tween::new(0.0_f64, 100.0, 2.seconds()).easing(Easing::CubicOut);
            assert_eq!(movement.sample(1.seconds()), 87.5);
            let track = Keyframes::new(0.0_f64)
                .to(1.seconds(), 100.0, Easing::QuadraticIn)
                .hold(500.milliseconds())
                .to(2.5.seconds(), 200.0, Easing::CubicOut);
            let cache = motion::render::SceneCache::new(0);
            let frame = motion::SceneContext::new(
                500.milliseconds().after_origin(),
                motion::FrameRate::from(60),
                motion::Size::new(64.0, 64.0),
                &cache,
            );
            assert_eq!(frame.sample(&track), 25.0);
            assert_eq!(track.sample(1250.milliseconds()), 100.0);
            assert_eq!(track.sample(2.seconds()), 187.5);
            let control = 0.500_000_000_1;
            let curve = Easing::CubicBezier {
                x1: 0.0,
                y1: control,
                x2: 1.0,
                y2: control,
            };
            let tween = Tween::new(0.0_f64, 1.0, 1.seconds()).easing(curve.clone());
            let track = Keyframes::new(0.0_f64).to(1.seconds(), 1.0, curve);
            assert_eq!(tween.sample(500.milliseconds()), 0.75 * control + 0.125);
            assert_eq!(
                track.sample(500.milliseconds()),
                tween.sample(500.milliseconds())
            );
            assert!(
                std::panic::catch_unwind(
                    || Tween::new(0.0_f64, 1.0, motion::Duration::ZERO).easing(Easing::Steps(0))
                )
                .is_err()
            );
            assert!(
                std::panic::catch_unwind(|| Keyframes::new(0.0_f64).to(
                    1.seconds(),
                    1.0,
                    Easing::Steps(0)
                ))
                .is_err()
            );
            assert!(
                std::panic::catch_unwind(
                    || Tween::new(0.0_f64, 1.0, 1.seconds()).easing(Easing::Steps(0))
                )
                .is_err()
            );
            Ok(())
        }
    }

    mod animation_typed_spring {

        use motion::prelude::*;

        fn close(actual: f64, expected: f64) {
            assert!(actual.is_finite() && expected.is_finite());
            assert!((actual - expected).abs() <= 2e-12 * expected.abs().max(1e-12));
        }

        #[test]
        fn generic_springs_share_frame_and_dynamic_sampling_contracts() -> motion::Result {
            let scalar = Spring::default().damping(20.0);
            let movement = Spring::new(Point::ORIGIN, Point::new(100.0, 40.0)).damping(20.0);
            let cache = motion::render::SceneCache::new(0);
            let frame = motion::SceneContext::new(
                1250.milliseconds().after_origin(),
                motion::FrameRate::from(60),
                motion::Size::new(64.0, 64.0),
                &cache,
            )
            .shifted(1.seconds());
            let progress = frame.sample(&scalar);
            let point = frame.sample(&movement);
            close(point.x, progress * 100.0);
            close(point.y, progress * 40.0);
            let end = movement.settle_duration();
            assert_eq!(movement.sample(end), Point::new(100.0, 40.0));
            let transforms =
                Spring::new([Transform::IDENTITY], [Transform::translate(100.0, 40.0)])
                    .damping(20.0);
            close(frame.sample(&transforms)[0].apply(Point::ORIGIN).x, point.x);
            let boxed = movement.map(|point| point.x).boxed();
            close(frame.sample(boxed.as_ref()), point.x);
            close(
                std::thread::spawn(move || boxed.sample(250.milliseconds()))
                    .join()
                    .unwrap(),
                point.x,
            );
            Ok(())
        }
    }

    mod geometry {

        use motion::prelude::*;

        #[test]
        fn authoring_scalars_have_f64_signatures_and_retain_precision() {
            use motion::{Color, Node, NodeModifiers, Rect, Signal, Size, Tween};

            let _: fn(f64, f64) -> Point = Point::new;
            let _: fn(f64, f64) -> Vector2 = Vector2::new;
            let _: fn(f64, f64) -> Size = Size::new;
            let _: fn(f64, f64, f64, f64) -> Rect = Rect::new;
            let _: fn(f64) -> Transform = Transform::rotate;
            let _: fn(f64, f64, f64) -> Color = Color::from_rgb_float;
            let _: fn(Color, f64) -> Color = Color::with_alpha;
            let scalar = f64::from_bits(0.5_f64.to_bits() + 1);
            assert_ne!(scalar, f64::from(scalar as f32));
            assert_eq!(Point::new(scalar, 0.0).x.to_bits(), scalar.to_bits());
            assert_eq!(Vector2::new(scalar, 0.0).x.to_bits(), scalar.to_bits());
            assert_eq!(Size::new(scalar, 1.0).width.to_bits(), scalar.to_bits());
            assert_eq!(
                Rect::new(0.0, 0.0, scalar, 1.0).width().to_bits(),
                scalar.to_bits()
            );
            assert_eq!(
                Transform::translate(scalar, 0.0)
                    .apply(Point::ORIGIN)
                    .x
                    .to_bits(),
                scalar.to_bits()
            );
            let animation = Tween::new(scalar, 1.0_f64, Duration::from_ticks(3));
            assert_eq!(animation.sample(Duration::ZERO).to_bits(), scalar.to_bits());
            let _node = Node::empty()
                .translate(scalar, -scalar)
                .rotate(scalar)
                .scale(scalar)
                .opacity(scalar);
        }
    }

    mod geometry_bounds {

        use motion::prelude::*;

        #[test]
        fn bounds_sample_through_local_ticks_tracks_springs_and_arrays() -> motion::Result {
            let from = Rect::new(10.0, 20.0, 40.0, 30.0);
            let to = Rect::new(20.0, 30.0, 80.0, 50.0);
            let cache = motion::render::SceneCache::new(0);
            let context = motion::SceneContext::new(
                1250.milliseconds().after_origin(),
                motion::FrameRate::from(60),
                Size::new(64.0, 64.0),
                &cache,
            )
            .shifted(1.seconds());
            let movement = Tween::new([from], [to], 1.seconds());
            let sampled = context.sample(&movement)[0];
            assert_eq!(sampled, Rect::new(12.5, 22.5, 50.0, 35.0));
            assert_eq!(sampled.size(), Size::new(37.5, 12.5));
            let track = Keyframes::new(from)
                .to(1.seconds(), to, Easing::Linear)
                .hold(1.seconds());
            assert_eq!(context.sample(&track), sampled);
            assert_eq!(track.sample(1500.milliseconds()), to);
            let spring = Spring::new(Size::new(10.0, 20.0), Size::default());
            let overshoot = spring.sample(500.milliseconds());
            assert!(overshoot.width < 0.0 && overshoot.height < 0.0);
            assert_eq!(overshoot.to_rect().size(), overshoot);
            assert_eq!(spring.sample(spring.settle_duration()), Size::default());
            Ok(())
        }
    }

    mod color {

        use motion::prelude::*;

        #[test]
        fn colors_share_frame_track_array_spring_and_thread_sampling_protocols() -> motion::Result {
            let cache = motion::render::SceneCache::new(0);
            let frame = motion::SceneContext::new(
                1250.milliseconds().after_origin(),
                motion::FrameRate::from(60),
                motion::Size::new(64.0, 64.0),
                &cache,
            )
            .shifted(1.seconds());
            let movement = Tween::new([Color::BLACK], [Color::WHITE], 1.seconds());
            let sampled = frame.sample(&movement)[0];
            assert_eq!(sampled, Color::BLACK.interpolate(&Color::WHITE, 0.25));
            let track = Keyframes::new(Color::BLACK)
                .to(1.seconds(), Color::WHITE, Easing::Linear)
                .hold(1.seconds());
            assert_eq!(frame.sample(&track), sampled);
            assert_eq!(track.sample(1500.milliseconds()), Color::WHITE);
            let spring = Spring::new(Color::BLACK, Color::WHITE);
            assert!(spring.sample(500.milliseconds()).to_rgba_float()[0] > 1.0);
            assert_eq!(spring.sample(Duration::ZERO), Color::BLACK);
            assert_eq!(spring.sample(spring.settle_duration()), Color::WHITE);
            let boxed = movement.map(|colors| colors[0]).boxed();
            assert_eq!(
                std::thread::spawn(move || boxed.sample(250.milliseconds()))
                    .join()
                    .unwrap(),
                sampled
            );
            Ok(())
        }
    }
}
