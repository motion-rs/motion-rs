use std::num::NonZeroU32;

use super::*;
use crate::fps;

fn nonzero(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap()
}

#[test]
fn unsigned_literals_and_integer_multipliers_preserve_large_tick_precision() {
    assert_eq!(2_u32.seconds(), 2_i64.seconds());
    assert_eq!(3_usize.milliseconds(), 3_i64.milliseconds());
    assert_eq!(4_u64.seconds(), 4_i64.seconds());
    let large = Duration::from_ticks((1 << 53) + 1);
    assert_eq!((large * 3_i64).ticks(), ((1 << 53) + 1) * 3);
    assert_eq!((3_u32 * large).ticks(), (large * 3_i64).ticks());
    assert_eq!((Duration::from_ticks(2) * 4_u64).ticks(), 8);
    assert_eq!(Duration::from_ticks(u64::MAX).checked_mul(2), None);
    assert!(std::panic::catch_unwind(|| Duration::from_ticks(u64::MAX) * 2_u32).is_err());
    assert!(std::panic::catch_unwind(|| u64::MAX.seconds()).is_err());
}

#[test]
fn standard_video_and_audio_intervals_are_exact() {
    for numerator in [24, 25, 30, 48, 50, 60, 90, 100, 120] {
        let rate = FrameRate::per_second(nonzero(numerator));
        assert_eq!(
            rate.frame_duration().ticks() * u64::from(numerator),
            TICKS_PER_SECOND as u64
        );
    }
    for rate in [
        FrameRate::NTSC_FILM,
        FrameRate::NTSC,
        FrameRate::NTSC_DOUBLE,
    ] {
        assert_eq!(
            u128::from(rate.frame_duration().ticks()) * u128::from(rate.numerator()),
            u128::from(TICKS_PER_SECOND as u64) * u128::from(rate.denominator())
        );
    }
    for sample_rate in [44_100, 48_000, 96_000] {
        assert_eq!(TICKS_PER_SECOND % sample_rate, 0);
    }
    assert_eq!(FrameRate::try_new(60_000, 2_002).unwrap(), FrameRate::NTSC);
    assert_eq!(
        FrameRate::new(nonzero(120), nonzero(2)),
        FrameRate::from(nonzero(60))
    );
    assert_eq!(fps!(60), FrameRate::from(nonzero(60)));
}

#[test]
fn frame_rate_construction_rejects_zero_components() {
    for (numerator, denominator) in [(0, 1), (1, 0), (0, 0)] {
        assert!(FrameRate::try_new(numerator, denominator).is_err());
    }
}

#[test]
fn signed_frame_boundaries_round_trip_without_drift() {
    for rate in [
        fps!(24),
        fps!(60),
        FrameRate::NTSC,
        FrameRate::NTSC_FILM,
        FrameRate::try_new(11, 1).unwrap(),
        FrameRate::try_new(123_457, 1_001).unwrap(),
    ] {
        for frame in [-1_000_000, -101, -1, 0, 1, 101, 1_000_000] {
            let time = Time::from_frame(frame, rate);
            assert_eq!(time.to_frame(rate), frame);
            assert_eq!((time - Duration::from_ticks(1)).to_frame(rate), frame - 1);
        }
        for frame in [0_u64, 1, 101, 1_000_000] {
            let span = Duration::from_frame(frame, rate);
            assert_eq!(span.to_frame(rate), frame);
            assert_eq!(span.after_origin(), Time::from_frame(frame as i64, rate));
        }
    }
    let rate = fps!(11);
    assert_eq!(Time::from_frame(11, rate), Time::ZERO + 1.seconds());
    assert_ne!(rate.frame_duration() * 11.0, 1.seconds());
    assert_eq!(Time::from_ticks(-1).to_frame(fps!(30)), -1);
    let tiny_time = Time::from_frame(i64::MAX, FrameRate::try_new(u32::MAX, 1).unwrap());
    assert!(tiny_time.ticks() > 0);
}

#[test]
fn checked_frame_conversions_preserve_rounding_and_reject_overflow() {
    let huge = FrameRate::try_new(u32::MAX, 1).unwrap();
    for rate in [
        FrameRate::NTSC,
        FrameRate::try_new(11, 1).unwrap(),
        FrameRate::try_new(123_457, 1_001).unwrap(),
    ] {
        for frame in [-1_000_000, -1, 0, 1, 1_000_000] {
            let time = Time::checked_from_frame(frame, rate).unwrap();
            assert_eq!(time, Time::from_frame(frame, rate));
            assert_eq!(time.checked_to_frame(rate), Some(frame));
            assert_eq!(
                (time - TimeOffset::from_ticks(1)).checked_to_frame(rate),
                Some(frame - 1)
            );
        }
    }
    assert_eq!(Time::checked_from_frame(i64::MAX, fps!(1)), None);
    assert_eq!(Time::checked_from_frame(i64::MIN, fps!(1)), None);
    assert_eq!(Duration::checked_from_frame(u64::MAX, fps!(1)), None);
    assert_eq!(Time::from_ticks(i64::MAX).checked_to_frame(huge), None);
    assert_eq!(Time::from_ticks(i64::MIN).checked_to_frame(huge), None);
    assert_eq!(Duration::from_ticks(u64::MAX).checked_to_frame(huge), None);
}

#[test]
fn literals_and_arithmetic_share_tick_time() {
    let duration: Duration = 1.5.seconds();
    assert_eq!(duration, 1.seconds() + 500.milliseconds());
    assert_eq!(Duration::from_seconds(1.0 / 1024.0).ticks(), 689_063);
    assert_eq!(TimeOffset::from_seconds(-1.0 / 1024.0).ticks(), -689_063);
    assert_eq!((1.0 / 128.0).milliseconds().ticks(), 5_513);
    assert_eq!(2.0 * duration, 3.seconds());
    assert_eq!(duration / 2.0, 750.milliseconds());
    assert_eq!(duration / 500.milliseconds(), 3.0);
    assert_eq!(duration.as_seconds(), 1.5);
    let mut time = Time::ZERO;
    time += duration;
    time -= 2.seconds();
    assert_eq!(time, Time::ZERO - 500.milliseconds());
    assert_eq!(time - Time::ZERO, TimeOffset::from_seconds(-0.5));
    assert_eq!(time.since_origin().magnitude(), 500.milliseconds());
    assert_eq!(Time::ZERO + time.since_origin(), time);
    assert_eq!(
        (Time::ZERO + 2.seconds()).duration_since(Time::ZERO + 500.milliseconds()),
        Some(1500.milliseconds())
    );
    assert_eq!(time.duration_since(Time::ZERO), None);
    assert_eq!(
        Time::from_ticks(i64::MAX).duration_since(Time::from_ticks(i64::MIN)),
        Some(Duration::from_ticks(u64::MAX))
    );
    for ticks in [0, 1, u64::MAX] {
        assert_eq!(
            Duration::from_ticks(ticks) * 1.0,
            Duration::from_ticks(ticks)
        );
        assert_eq!(
            Duration::from_ticks(ticks) / 1.0,
            Duration::from_ticks(ticks)
        );
    }
}

#[test]
fn durations_cannot_be_negative() {
    assert!(Duration::try_from_seconds(-1.0).is_err());
    assert!(Duration::try_from_seconds(-0.0).is_ok());
    assert_eq!(1.seconds().checked_sub(2.seconds()), None);
    assert_eq!(1.seconds().saturating_sub(2.seconds()), Duration::ZERO);
    assert_eq!(2.seconds().checked_sub(1.seconds()), Some(1.seconds()));
    assert!(1.seconds().try_mul(-1.0).is_err());
    assert!(1.seconds().try_div(-1.0).is_err());
    assert!(TimeOffset::from_ticks(-1).to_duration().is_none());
    assert_eq!(
        TimeOffset::from_ticks(i64::MIN).magnitude().ticks(),
        1 << 63
    );
    assert!(std::panic::catch_unwind(|| 1.seconds() - 2.seconds()).is_err());
    assert!(std::panic::catch_unwind(|| (-1).seconds()).is_err());
    assert!(std::panic::catch_unwind(|| (-1.0).seconds()).is_err());
    assert!(std::panic::catch_unwind(|| 1.seconds() * -1).is_err());
    assert!(std::panic::catch_unwind(|| 1.seconds() * -1.0).is_err());
}

#[test]
fn invalid_inputs_and_overflow_do_not_saturate_or_wrap() {
    for seconds in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX] {
        assert!(std::panic::catch_unwind(|| Duration::from_seconds(seconds)).is_err());
        assert!(std::panic::catch_unwind(|| Time::from_seconds(seconds)).is_err());
    }
    let maximum = Duration::from_ticks(u64::MAX);
    assert!(std::panic::catch_unwind(|| maximum + Duration::from_ticks(1)).is_err());
    assert!(
        std::panic::catch_unwind(|| Time::from_ticks(i64::MAX) + Duration::from_ticks(1)).is_err()
    );
    assert!(std::panic::catch_unwind(|| Time::from_ticks(0) + maximum).is_err());
    assert!(std::panic::catch_unwind(|| maximum.after_origin()).is_err());
    assert!(std::panic::catch_unwind(|| Time::from_ticks(i64::MIN) - Time::from_ticks(1)).is_err());
    assert!(std::panic::catch_unwind(|| -TimeOffset::from_ticks(i64::MIN)).is_err());
    assert!(std::panic::catch_unwind(|| maximum * 2.0).is_err());
    assert!(std::panic::catch_unwind(|| 1.seconds() * f64::NAN).is_err());
    assert!(std::panic::catch_unwind(|| 1.seconds() / 0.0).is_err());
    assert!(std::panic::catch_unwind(|| Duration::ZERO / Duration::ZERO).is_err());
    assert!(std::panic::catch_unwind(|| i64::MAX.seconds()).is_err());
    assert!(std::panic::catch_unwind(|| i64::MAX.milliseconds()).is_err());
    assert!(std::panic::catch_unwind(|| Time::from_frame(i64::MAX, fps!(1))).is_err());
    assert!(
        std::panic::catch_unwind(|| maximum.to_frame(FrameRate::try_new(u32::MAX, 1).unwrap()))
            .is_err()
    );
}
