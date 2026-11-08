//! Signed integer-tick time and rational frame rates, independent of rendering.

use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

/// Flicks per second, exactly covering common video and audio sample rates.
pub const TICKS_PER_SECOND: i64 = 705_600_000;

/// A signed time point or duration, stored as integer flicks.
///
/// Comparisons and frame conversion use integers. Floating-point seconds are
/// intended for animation mathematics, not interval membership checks.
/// Arithmetic panics on overflow, independently of the build profile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time(i64);

/// A duration uses the same representation and arithmetic as a time point.
pub type Duration = Time;

impl Time {
    /// The time origin and an empty duration.
    pub const ZERO: Self = Self(0);

    /// Constructs time without losing integer tick precision.
    pub const fn from_ticks(ticks: i64) -> Self {
        Self(ticks)
    }

    /// Rounds seconds to the nearest tick; halfway cases round away from zero.
    ///
    /// # Panics
    /// Panics for nonfinite input or a rounded result outside the signed range.
    pub fn from_seconds(seconds: f64) -> Self {
        Self::rounded_ticks(seconds * TICKS_PER_SECOND as f64)
    }

    /// Returns the first tick at or after the exact rational frame boundary.
    ///
    /// Common integer and NTSC rates have exact tick boundaries. Other rates
    /// round toward positive infinity by less than one tick, including before
    /// zero. Compute each boundary directly; multiplying a rounded frame duration
    /// would accumulate drift. Frame indices round-trip through [`Self::to_frame`]
    /// when the rate does not exceed one frame per tick.
    ///
    /// # Panics
    /// Panics if the boundary cannot be represented as signed ticks.
    pub fn from_frame(frame: i64, rate: FrameRate) -> Self {
        Self::checked_from_frame(frame, rate).expect("frame time exceeds the signed tick range")
    }

    /// Converts a frame boundary with the same ceiling rule as from_frame.
    /// Returns None instead of panicking when signed ticks would overflow.
    pub fn checked_from_frame(frame: i64, rate: FrameRate) -> Option<Self> {
        let numerator =
            i128::from(frame) * i128::from(TICKS_PER_SECOND) * i128::from(rate.denominator);
        let denominator = i128::from(rate.numerator);
        let ticks =
            numerator.div_euclid(denominator) + i128::from(numerator.rem_euclid(denominator) != 0);
        i64::try_from(ticks).ok().map(Self)
    }

    /// Returns the containing frame, rounding toward negative infinity.
    ///
    /// # Panics
    /// Panics if the frame index is outside the signed 64-bit range.
    pub fn to_frame(self, rate: FrameRate) -> i64 {
        self.checked_to_frame(rate)
            .expect("frame index exceeds the signed 64-bit range")
    }

    /// Finds the containing frame using integer floor division.
    /// Returns None instead of panicking when the frame index would overflow.
    pub fn checked_to_frame(self, rate: FrameRate) -> Option<i64> {
        let numerator = i128::from(self.0) * i128::from(rate.numerator);
        let denominator = i128::from(TICKS_PER_SECOND) * i128::from(rate.denominator);
        i64::try_from(numerator.div_euclid(denominator)).ok()
    }

    /// Converts to floating-point seconds for animation mathematics.
    pub fn as_seconds(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }

    /// Returns the exact signed tick count.
    pub const fn ticks(self) -> i64 {
        self.0
    }

    /// Multiplies ticks exactly, returning None on signed overflow.
    pub fn checked_mul(self, factor: i64) -> Option<Self> {
        self.0.checked_mul(factor).map(Self)
    }

    fn rounded_ticks(ticks: f64) -> Self {
        let rounded = ticks.round();
        assert!(
            rounded.is_finite() && rounded >= i64::MIN as f64 && rounded < -(i64::MIN as f64),
            "time must be finite and fit the signed tick range"
        );
        Self(rounded as i64)
    }
}

impl Add for Time {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(self.0.checked_add(other.0).expect("time addition overflow"))
    }
}

impl Sub for Time {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self(
            self.0
                .checked_sub(other.0)
                .expect("time subtraction overflow"),
        )
    }
}

impl AddAssign for Time {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

impl SubAssign for Time {
    fn sub_assign(&mut self, other: Self) {
        *self = *self - other;
    }
}

impl Neg for Time {
    type Output = Self;

    fn neg(self) -> Self {
        Self(self.0.checked_neg().expect("time negation overflow"))
    }
}

impl Mul<f64> for Time {
    type Output = Self;

    /// Scales time and rounds to the nearest tick, away from zero at ties.
    fn mul(self, factor: f64) -> Self {
        match factor {
            1.0 => self,
            -1.0 => -self,
            _ => Self::rounded_ticks(self.0 as f64 * factor),
        }
    }
}

macro_rules! integer_time_mul {
    ($($integer:ty),+ $(,)?) => {$ (
        impl Mul<$integer> for Time {
            type Output = Time;
            fn mul(self, factor: $integer) -> Time {
                Time(i64::try_from(i128::from(self.0) * factor as i128).expect("time multiplication overflow"))
            }
        }
        impl Mul<Time> for $integer {
            type Output = Time;
            fn mul(self, time: Time) -> Time { time * self }
        }
    )+};
}

integer_time_mul!(i32, i64, u32, u64, usize);

impl Mul<Time> for f64 {
    type Output = Time;

    fn mul(self, time: Time) -> Time {
        time * self
    }
}

impl Div<f64> for Time {
    type Output = Self;

    /// Divides time; a nonfinite or zero divisor is rejected.
    fn div(self, divisor: f64) -> Self {
        assert!(
            divisor.is_finite() && divisor != 0.0,
            "time divisor must be finite and nonzero"
        );
        match divisor {
            1.0 => self,
            -1.0 => -self,
            _ => Self::rounded_ticks(self.0 as f64 / divisor),
        }
    }
}

impl Div for Time {
    type Output = f64;

    /// Returns a dimensionless ratio; a zero duration is rejected.
    fn div(self, duration: Self) -> f64 {
        assert!(duration.0 != 0, "time ratio requires a nonzero duration");
        self.0 as f64 / duration.0 as f64
    }
}

/// A positive rational number of frames per second, stored in reduced form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrameRate {
    numerator: u32,
    denominator: u32,
}

impl FrameRate {
    /// Film rate of exactly 24000/1001 frames per second.
    pub const NTSC_FILM: Self = Self::new(24_000, 1_001);

    /// NTSC rate of exactly 30000/1001 frames per second.
    pub const NTSC: Self = Self::new(30_000, 1_001);

    /// Double NTSC rate of exactly 60000/1001 frames per second.
    pub const NTSC_DOUBLE: Self = Self::new(60_000, 1_001);

    /// Constructs and reduces a rational frame rate.
    ///
    /// # Panics
    /// Panics if either component is zero.
    pub const fn new(numerator: u32, denominator: u32) -> Self {
        assert!(numerator != 0, "frame-rate numerator must be nonzero");
        assert!(denominator != 0, "frame-rate denominator must be nonzero");
        let mut common = numerator;
        let mut remainder = denominator;
        while remainder != 0 {
            let next = common % remainder;
            common = remainder;
            remainder = next;
        }
        Self {
            numerator: numerator / common,
            denominator: denominator / common,
        }
    }

    /// Returns the numerator of the reduced rate.
    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    /// Returns the denominator of the reduced rate.
    pub const fn denominator(self) -> u32 {
        self.denominator
    }

    /// Returns the first frame's duration, rounded up if not tick-exact.
    /// Use [`Time::from_frame`] for later boundaries to avoid accumulated rounding.
    pub fn frame_duration(self) -> Duration {
        Time::from_frame(1, self)
    }
}

impl From<u32> for FrameRate {
    fn from(frames_per_second: u32) -> Self {
        Self::new(frames_per_second, 1)
    }
}

/// Readable signed time literals, such as `2.seconds()` and `1.5.seconds()`.
///
/// Floating-point literals round to the nearest tick, away from zero at ties.
/// Nonfinite or out-of-range values panic. Integer literals use exact arithmetic
/// and panic on overflow, independently of the build profile.
pub trait TimeLiteral {
    /// Converts seconds to signed tick time.
    fn seconds(self) -> Time;

    /// Converts milliseconds to signed tick time.
    fn milliseconds(self) -> Time;
}

impl TimeLiteral for i64 {
    fn seconds(self) -> Time {
        Time(
            self.checked_mul(TICKS_PER_SECOND)
                .expect("seconds overflow"),
        )
    }

    fn milliseconds(self) -> Time {
        Time(
            self.checked_mul(TICKS_PER_SECOND / 1_000)
                .expect("milliseconds overflow"),
        )
    }
}

impl TimeLiteral for i32 {
    fn seconds(self) -> Time {
        i64::from(self).seconds()
    }

    fn milliseconds(self) -> Time {
        i64::from(self).milliseconds()
    }
}

macro_rules! unsigned_time_literal {
    ($($integer:ty),+ $(,)?) => {$ (
        impl TimeLiteral for $integer {
            fn seconds(self) -> Time { i64::try_from(self).expect("seconds overflow").seconds() }
            fn milliseconds(self) -> Time { i64::try_from(self).expect("milliseconds overflow").milliseconds() }
        }
    )+};
}

unsigned_time_literal!(u32, u64, usize);

impl TimeLiteral for f64 {
    fn seconds(self) -> Time {
        Time::from_seconds(self)
    }

    fn milliseconds(self) -> Time {
        Time::rounded_ticks(self * (TICKS_PER_SECOND / 1_000) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsigned_literals_and_integer_multipliers_preserve_large_tick_precision() {
        assert_eq!(2_u32.seconds(), 2_i64.seconds());
        assert_eq!(3_usize.milliseconds(), 3_i64.milliseconds());
        assert_eq!(4_u64.seconds(), 4_i64.seconds());
        let large = Time::from_ticks((1_i64 << 53) + 1);
        assert_eq!((large * 3_i64).ticks(), ((1_i64 << 53) + 1) * 3);
        assert_eq!((3_u32 * large).ticks(), (large * 3_i64).ticks());
        assert_eq!((Time::from_ticks(-2) * 4_u64).ticks(), -8);
        assert_eq!(Time::from_ticks(i64::MAX).checked_mul(2), None);
        assert!(std::panic::catch_unwind(|| Time::from_ticks(i64::MAX) * 2_u32).is_err());
        assert!(std::panic::catch_unwind(|| u64::MAX.seconds()).is_err());
    }

    #[test]
    fn standard_video_and_audio_intervals_are_exact() {
        for numerator in [24, 25, 30, 48, 50, 60, 90, 100, 120] {
            let rate = FrameRate::from(numerator);
            assert_eq!(
                rate.frame_duration().ticks() * i64::from(numerator),
                TICKS_PER_SECOND
            );
        }
        for rate in [
            FrameRate::NTSC_FILM,
            FrameRate::NTSC,
            FrameRate::NTSC_DOUBLE,
        ] {
            assert_eq!(
                i128::from(rate.frame_duration().ticks()) * i128::from(rate.numerator()),
                i128::from(TICKS_PER_SECOND) * i128::from(rate.denominator())
            );
        }
        for sample_rate in [44_100, 48_000, 96_000] {
            assert_eq!(TICKS_PER_SECOND % sample_rate, 0);
        }
        assert_eq!(FrameRate::new(60_000, 2_002), FrameRate::NTSC);
        assert_eq!(FrameRate::new(120, 2), FrameRate::from(60));
    }

    #[test]
    fn signed_frame_boundaries_round_trip_without_drift() {
        for rate in [
            FrameRate::from(24),
            FrameRate::from(60),
            FrameRate::NTSC,
            FrameRate::NTSC_FILM,
            FrameRate::new(11, 1),
            FrameRate::new(123_457, 1_001),
        ] {
            for frame in [-1_000_000, -101, -1, 0, 1, 101, 1_000_000] {
                let time = Time::from_frame(frame, rate);
                assert_eq!(time.to_frame(rate), frame);
                assert_eq!((time - Time::from_ticks(1)).to_frame(rate), frame - 1);
            }
        }
        let rate = FrameRate::from(11);
        assert_eq!(Time::from_frame(11, rate), 1.seconds());
        assert_ne!(rate.frame_duration() * 11.0, 1.seconds());
        assert_eq!(Time::from_ticks(-1).to_frame(FrameRate::from(30)), -1);
        let tiny_time = Time::from_frame(i64::MAX, FrameRate::new(u32::MAX, 1));
        assert!(tiny_time.ticks() > 0);
    }

    #[test]
    fn checked_frame_conversions_preserve_rounding_and_reject_overflow() {
        for rate in [
            FrameRate::NTSC,
            FrameRate::new(11, 1),
            FrameRate::new(123_457, 1_001),
        ] {
            for frame in [-1_000_000, -1, 0, 1, 1_000_000] {
                let time = Time::checked_from_frame(frame, rate).unwrap();
                assert_eq!(time, Time::from_frame(frame, rate));
                assert_eq!(time.checked_to_frame(rate), Some(frame));
                assert_eq!(
                    (time - Time::from_ticks(1)).checked_to_frame(rate),
                    Some(frame - 1)
                );
            }
        }
        assert_eq!(Time::checked_from_frame(i64::MAX, FrameRate::from(1)), None);
        assert_eq!(Time::checked_from_frame(i64::MIN, FrameRate::from(1)), None);
        assert_eq!(
            Time::from_ticks(i64::MAX).checked_to_frame(FrameRate::new(u32::MAX, 1)),
            None
        );
        assert_eq!(
            Time::from_ticks(i64::MIN).checked_to_frame(FrameRate::new(u32::MAX, 1)),
            None
        );
    }

    #[test]
    fn literals_and_arithmetic_share_signed_tick_time() {
        let duration: Duration = 1.5.seconds();
        assert_eq!(duration, 1.seconds() + 500.milliseconds());
        assert_eq!((-250).milliseconds(), -250.milliseconds());
        assert_eq!(Time::from_seconds(1.0 / 1024.0).ticks(), 689_063);
        assert_eq!(Time::from_seconds(-1.0 / 1024.0).ticks(), -689_063);
        assert_eq!((1.0 / 128.0).milliseconds().ticks(), 5_513);
        assert_eq!(2.0 * duration, 3.seconds());
        assert_eq!(duration / 2.0, 750.milliseconds());
        assert_eq!(duration / 500.milliseconds(), 3.0);
        assert_eq!(duration.as_seconds(), 1.5);
        let mut time = Time::ZERO;
        time += duration;
        time -= 2.seconds();
        assert_eq!(time, -500.milliseconds());
        for ticks in [i64::MIN, i64::MAX] {
            assert_eq!(Time::from_ticks(ticks) * 1.0, Time::from_ticks(ticks));
            assert_eq!(Time::from_ticks(ticks) / 1.0, Time::from_ticks(ticks));
        }
    }

    #[test]
    fn invalid_inputs_and_overflow_do_not_saturate_or_wrap() {
        for seconds in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX] {
            assert!(std::panic::catch_unwind(|| Time::from_seconds(seconds)).is_err());
        }
        for components in [(0, 1), (1, 0), (0, 0)] {
            assert!(
                std::panic::catch_unwind(|| FrameRate::new(components.0, components.1)).is_err()
            );
        }
        let maximum = Time::from_ticks(i64::MAX);
        let minimum = Time::from_ticks(i64::MIN);
        assert!(std::panic::catch_unwind(|| maximum + Time::from_ticks(1)).is_err());
        assert!(std::panic::catch_unwind(|| minimum - Time::from_ticks(1)).is_err());
        assert!(std::panic::catch_unwind(|| -minimum).is_err());
        assert!(std::panic::catch_unwind(|| maximum * 2.0).is_err());
        assert!(std::panic::catch_unwind(|| 1.seconds() * f64::NAN).is_err());
        assert!(std::panic::catch_unwind(|| 1.seconds() / 0.0).is_err());
        assert!(std::panic::catch_unwind(|| Time::ZERO / Time::ZERO).is_err());
        assert!(std::panic::catch_unwind(|| i64::MAX.seconds()).is_err());
        assert!(std::panic::catch_unwind(|| i64::MAX.milliseconds()).is_err());
        assert!(
            std::panic::catch_unwind(|| Time::from_frame(i64::MAX, FrameRate::from(1))).is_err()
        );
        assert!(
            std::panic::catch_unwind(|| maximum.to_frame(FrameRate::new(u32::MAX, 1))).is_err()
        );
    }
}
