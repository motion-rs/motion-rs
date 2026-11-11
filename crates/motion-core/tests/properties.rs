//! Property tests for time arithmetic and animation sampling.

use motion_core::{Duration, FiniteAnimation, FrameRate, Signal, Time, Tween};
use proptest::prelude::*;

#[derive(Clone, Copy)]
struct Infinite;

impl Signal<f64> for Infinite {
    fn sample(&self, time: Duration) -> f64 {
        time.as_seconds()
    }
}

proptest! {
    #![proptest_config({
        let mut config = ProptestConfig::default();
        if std::env::var_os("PROPTEST_CASES").is_none() {
            config.cases = 128;
        }
        config
    })]

    #[test]
    fn signed_frame_boundaries_round_trip(
        frame in -1_000_000_i64..=1_000_000,
        numerator in 1_u32..=240_000,
        denominator in 1_u32..=10_000,
    ) {
        let rate = FrameRate::new(numerator, denominator);
        let boundary = Time::from_frame(frame, rate);
        prop_assert_eq!(boundary.to_frame(rate), frame);
        prop_assert_eq!(Time::from_ticks(boundary.ticks() - 1).to_frame(rate), frame - 1);
        prop_assert!(Time::from_frame(frame + 1, rate) > boundary);
    }

    #[test]
    fn finite_then_and_zip_obey_duration_and_boundary_rules(
        first_ticks in 0_i64..=1_000_000,
        second_ticks in 0_i64..=1_000_000,
        sample_ticks in -1_000_000_i64..=3_000_000,
    ) {
        let first = Tween::new(0.0_f64, 1.0, Duration::from_ticks(first_ticks));
        let second = Tween::new(2.0_f64, 3.0, Duration::from_ticks(second_ticks));
        let sample = Duration::from_ticks(sample_ticks);
        let zipped = first.clone().zip(second.clone());
        prop_assert_eq!(zipped.duration(), Duration::from_ticks(first_ticks.max(second_ticks)));
        prop_assert_eq!(zipped.sample(sample), (first.sample(sample), second.sample(sample)));
        prop_assert_eq!(
            first.clone().zip(Infinite).sample(sample),
            (first.sample(sample), sample.as_seconds())
        );
        let chained = first.then(second.clone());
        prop_assert_eq!(chained.duration(), Duration::from_ticks(first_ticks + second_ticks));
        prop_assert_eq!(chained.sample(Duration::from_ticks(first_ticks)), second.sample(Duration::ZERO));
        prop_assert_eq!(chained.sample(Duration::from_ticks(first_ticks + second_ticks)), 3.0);
    }
}
