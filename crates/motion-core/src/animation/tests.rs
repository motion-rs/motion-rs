//! Unit tests for animation combinators, timing adapters and sampling.

use super::*;
use crate::{Point, TimeLiteral};

#[test]
fn delay_and_speed_preserve_boundaries_and_collapsed_durations() {
    let tween = Tween::new(0.0_f64, 10.0, Duration::from_ticks(10));
    let delayed = tween.clone().delay(Duration::from_ticks(5));
    assert_eq!(delayed.duration(), Duration::from_ticks(15));
    for (ticks, expected) in [
        (i64::MIN, 0.0),
        (5, 0.0),
        (10, 5.0),
        (15, 10.0),
        (i64::MAX, 10.0),
    ] {
        assert_eq!(delayed.sample(Duration::from_ticks(ticks)), expected);
    }
    let faster = tween.clone().speed(2.0);
    assert_eq!(faster.duration(), Duration::from_ticks(5));
    assert_eq!(faster.sample(Duration::from_ticks(2)), 4.0);
    assert_eq!(faster.sample(Duration::from_ticks(5)), 10.0);
    assert_eq!(faster.sample(Duration::from_ticks(i64::MIN)), 0.0);
    let collapsed = tween.speed(100.0);
    assert_eq!(collapsed.duration(), Duration::ZERO);
    assert_eq!(collapsed.sample(BEFORE_START), 0.0);
    assert_eq!(collapsed.sample(Duration::ZERO), 10.0);
    let maximum = Tween::new(0.0_f64, 1.0, Duration::from_ticks(i64::MAX)).speed(2.0);
    let near_end = maximum.duration() - Duration::from_ticks(1);
    let value = maximum.sample(near_end);
    assert!(value.is_finite() && (0.0..=1.0).contains(&value));
}

#[test]
fn finite_combinators_use_exact_integer_boundaries_and_arbitrary_order() {
    let tween = Tween::new(0.0_f64, 10.0, Duration::from_ticks(10));
    let sequence = tween
        .clone()
        .then(Tween::new(100.0_f64, 110.0, Duration::from_ticks(5)));
    assert_eq!(sequence.duration(), Duration::from_ticks(15));
    for (ticks, expected) in [
        (15, 110.0),
        (0, 0.0),
        (10, 100.0),
        (12, 104.0),
        (5, 5.0),
        (i64::MIN, 0.0),
        (i64::MAX, 110.0),
    ] {
        assert_eq!(sequence.sample(Duration::from_ticks(ticks)), expected);
    }
    let reversed = tween.clone().reverse();
    assert_eq!(reversed.duration(), Duration::from_ticks(10));
    for (ticks, expected) in [
        (10, 0.0),
        (0, 10.0),
        (5, 5.0),
        (i64::MIN, 10.0),
        (i64::MAX, 0.0),
    ] {
        assert_eq!(reversed.sample(Duration::from_ticks(ticks)), expected);
    }
    let repeated = tween.clone().repeat(3);
    assert_eq!(repeated.duration(), Duration::from_ticks(30));
    for (ticks, expected) in [
        (30, 10.0),
        (0, 0.0),
        (10, 0.0),
        (20, 0.0),
        (29, 9.0),
        (i64::MIN, 0.0),
        (i64::MAX, 10.0),
    ] {
        assert_eq!(repeated.sample(Duration::from_ticks(ticks)), expected);
    }
    let round_trip = tween.clone().ping_pong();
    assert_eq!(round_trip.duration(), Duration::from_ticks(20));
    for (ticks, expected) in [
        (20, 0.0),
        (0, 0.0),
        (10, 10.0),
        (15, 5.0),
        (i64::MIN, 0.0),
        (i64::MAX, 0.0),
    ] {
        assert_eq!(round_trip.sample(Duration::from_ticks(ticks)), expected);
    }
    assert_eq!(tween.repeat(0).sample(Duration::from_ticks(i64::MAX)), 0.0);
    let composed = round_trip
        .repeat(2)
        .speed(2.0)
        .delay(Duration::from_ticks(5));
    assert_eq!(composed.duration(), Duration::from_ticks(25));
    assert_eq!(composed.sample(Duration::from_ticks(10)), 10.0);
    assert_eq!(composed.sample(Duration::from_ticks(15)), 0.0);
}

#[test]
fn instantaneous_sources_keep_distinct_before_and_after_values() {
    let instant = Tween::new(2.0_f64, 7.0, Duration::ZERO);
    let delayed = instant.clone().delay(Duration::from_ticks(5));
    assert_eq!(delayed.sample(Duration::from_ticks(4)), 2.0);
    assert_eq!(delayed.sample(Duration::from_ticks(5)), 7.0);
    let repeated = instant.clone().repeat(3);
    assert_eq!(repeated.sample(BEFORE_START), 2.0);
    assert_eq!(repeated.sample(Duration::ZERO), 7.0);
    assert_eq!(instant.clone().repeat(0).sample(Duration::ZERO), 2.0);
    assert_eq!(instant.clone().speed(2.0).sample(BEFORE_START), 2.0);
    assert_eq!(instant.clone().reverse().sample(BEFORE_START), 7.0);
    assert_eq!(instant.clone().reverse().sample(Duration::ZERO), 2.0);
    assert_eq!(instant.clone().ping_pong().sample(Duration::ZERO), 2.0);
    let sequence = instant.then(Tween::new(11.0_f64, 13.0, Duration::from_ticks(2)));
    assert_eq!(sequence.sample(BEFORE_START), 2.0);
    assert_eq!(sequence.sample(Duration::ZERO), 11.0);
}

#[test]
fn invalid_compositions_and_duration_overflows_fail_at_construction() {
    fn tween() -> Tween<f64> {
        Tween::new(0.0_f64, 1.0, 1.seconds())
    }
    for factor in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(std::panic::catch_unwind(|| tween().speed(factor)).is_err());
    }
    assert!(std::panic::catch_unwind(|| Tween::new(0.0_f64, 1.0, BEFORE_START)).is_err());
    assert!(std::panic::catch_unwind(|| tween().delay(BEFORE_START)).is_err());

    fn maximum() -> Tween<f64> {
        Tween::new(0.0_f64, 1.0, Duration::from_ticks(i64::MAX))
    }
    assert_eq!(
        maximum().repeat(1).duration(),
        Duration::from_ticks(i64::MAX)
    );
    assert_eq!(
        maximum().speed(1.0).duration(),
        Duration::from_ticks(i64::MAX)
    );
    assert!(std::panic::catch_unwind(|| maximum().repeat(2)).is_err());
    assert!(std::panic::catch_unwind(|| maximum().ping_pong()).is_err());
    assert!(std::panic::catch_unwind(|| maximum().delay(Duration::from_ticks(1))).is_err());
    assert!(std::panic::catch_unwind(|| maximum().then(tween())).is_err());
    assert!(std::panic::catch_unwind(|| maximum().speed(0.5)).is_err());

    struct Invalid;

    impl Signal<f64> for Invalid {
        fn sample(&self, _time: Duration) -> f64 {
            0.0
        }
    }

    impl FiniteAnimation<f64> for Invalid {
        fn duration(&self) -> Duration {
            BEFORE_START
        }
    }
    assert!(std::panic::catch_unwind(|| Invalid.reverse()).is_err());
}

#[test]
fn tween_preserves_tick_progress_signed_endpoints_and_overshoot() {
    let tween = Tween::new(0.0_f64, 1.0, Duration::from_ticks(10_000_000_000));
    let progress = 0.500_000_000_1;
    assert_eq!(tween.sample(Duration::from_ticks(5_000_000_001)), progress);
    assert_ne!(progress, f64::from(progress as f32));
    for (ticks, expected) in [
        (i64::MIN, 0.0),
        (0, 0.0),
        (10_000_000_000, 1.0),
        (i64::MAX, 1.0),
    ] {
        assert_eq!(tween.sample(Duration::from_ticks(ticks)), expected);
    }
    let instant = Tween::new(2.0_f64, 7.0, Duration::ZERO);
    assert_eq!(instant.sample(Duration::from_ticks(-1)), 2.0);
    assert_eq!(instant.sample(Duration::ZERO), 7.0);
    let overshooting = Tween::new(0.0_f64, 1.0, 1.seconds()).easing(Easing::CubicBezier {
        x1: 0.3,
        y1: 1.8,
        x2: 0.7,
        y2: 1.8,
    });
    assert!(overshooting.sample(500.milliseconds()) > 1.0);
    assert_eq!(overshooting.sample(1.seconds()), 1.0);
}

#[test]
fn map_zip_closures_and_type_erasure_preserve_timing_and_thread_safety() {
    let tween = Tween::new(0.0_f64, 8.0, 2.seconds());
    let mapped = tween
        .clone()
        .map(|value| Point::new(value, value * 2.0))
        .boxed_finite();
    let zipped = mapped.clone().zip(Tween::new(0.0_f64, 3.0, 3.seconds()));
    assert_eq!(zipped.duration(), 3.seconds());
    assert_eq!(zipped.sample(1.seconds()), (Point::new(4.0, 8.0), 1.0));
    assert_eq!(zipped.sample(4.seconds()), (Point::new(8.0, 16.0), 3.0));
    let signal = (|time: Duration| time.ticks())
        .map(|ticks| ticks.to_string())
        .boxed();
    assert_eq!(signal.sample(Duration::from_ticks(-17)), "-17");
    assert_eq!(
        std::thread::spawn(move || mapped.sample(1.seconds()))
            .join()
            .unwrap(),
        Point::new(4.0, 8.0)
    );
}
