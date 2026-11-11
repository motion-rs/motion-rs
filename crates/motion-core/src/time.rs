//! Integer-tick time points, nonnegative durations, signed offsets and rational frame rates.

use std::ops::{Add, AddAssign, Div, Mul, Sub, SubAssign};

mod frame_rate;
mod offset;

pub use frame_rate::FrameRate;
pub use offset::TimeOffset;

/// Flicks per second, exactly covering common video and audio sample rates.
pub const TICKS_PER_SECOND: i64 = 705_600_000;

const TICKS_PER_SECOND_UNSIGNED: u64 = TICKS_PER_SECOND as u64;

/// A signed point on the timeline, stored as integer flicks since the origin.
///
/// Comparisons and frame conversion use integers. Floating-point seconds are
/// intended for animation mathematics, not interval membership checks.
/// Only meaningful operations exist: `Time - Time` is a signed [`TimeOffset`],
/// and `Time ± Duration` or `Time ± TimeOffset` is a [`Time`]. Arithmetic panics
/// on overflow, independently of the build profile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time(i64);

/// A nonnegative span of time, stored as integer flicks.
///
/// Negative spans cannot be constructed. Use [`TimeOffset`] where a signed
/// displacement is meaningful, such as shifting a clip earlier. Durations add,
/// scale and subtract (panicking below zero, see [`Duration::checked_sub`]);
/// they share the tick unit and overflow behavior of [`Time`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Duration(u64);

impl Time {
    /// The time origin.
    pub const ZERO: Self = Self(0);

    /// Constructs time without losing integer tick precision.
    pub const fn from_ticks(ticks: i64) -> Self {
        Self(ticks)
    }

    /// Rounds seconds to the nearest tick; halfway cases round away from zero.
    ///
    /// # Panics
    /// Panics for nonfinite input or a rounded result outside the signed range;
    /// see [`Self::try_from_seconds`].
    pub fn from_seconds(seconds: f64) -> Self {
        Self::try_from_seconds(seconds).unwrap_or_else(|error| panic!("{}", error.message))
    }

    /// Rounds seconds like [`Self::from_seconds`], reporting nonfinite or out-of-range input.
    pub fn try_from_seconds(seconds: f64) -> Result<Self, crate::ValidationError> {
        try_rounded_ticks(seconds * TICKS_PER_SECOND as f64).map(Self)
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
            i128::from(frame) * i128::from(TICKS_PER_SECOND) * i128::from(rate.denominator());
        let denominator = i128::from(rate.numerator());
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
        let numerator = i128::from(self.0) * i128::from(rate.numerator());
        let denominator = i128::from(TICKS_PER_SECOND) * i128::from(rate.denominator());
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

    /// Returns the signed displacement from the origin to this time point.
    pub const fn since_origin(self) -> TimeOffset {
        TimeOffset::from_ticks(self.0)
    }

    /// Returns the span since an earlier time point, or None when `earlier` is later.
    pub fn duration_since(self, earlier: Self) -> Option<Duration> {
        (self.0 >= earlier.0).then(|| Duration(self.0.abs_diff(earlier.0)))
    }

    /// Adds a duration, returning None on signed overflow.
    pub fn checked_add(self, span: Duration) -> Option<Self> {
        i64::try_from(span.0)
            .ok()
            .and_then(|ticks| self.0.checked_add(ticks))
            .map(Self)
    }

    /// Subtracts a duration, returning None on signed overflow.
    pub fn checked_sub(self, span: Duration) -> Option<Self> {
        i64::try_from(span.0)
            .ok()
            .and_then(|ticks| self.0.checked_sub(ticks))
            .map(Self)
    }
}

impl Add<Duration> for Time {
    type Output = Self;

    fn add(self, span: Duration) -> Self {
        self.checked_add(span).expect("time addition overflow")
    }
}

impl Sub<Duration> for Time {
    type Output = Self;

    fn sub(self, span: Duration) -> Self {
        self.checked_sub(span).expect("time subtraction overflow")
    }
}

impl Add<TimeOffset> for Time {
    type Output = Self;

    fn add(self, offset: TimeOffset) -> Self {
        Self(
            self.0
                .checked_add(offset.ticks())
                .expect("time addition overflow"),
        )
    }
}

impl Sub<TimeOffset> for Time {
    type Output = Self;

    fn sub(self, offset: TimeOffset) -> Self {
        Self(
            self.0
                .checked_sub(offset.ticks())
                .expect("time subtraction overflow"),
        )
    }
}

impl Sub for Time {
    type Output = TimeOffset;

    fn sub(self, other: Self) -> TimeOffset {
        TimeOffset::from_ticks(
            self.0
                .checked_sub(other.0)
                .expect("time subtraction overflow"),
        )
    }
}

impl AddAssign<Duration> for Time {
    fn add_assign(&mut self, span: Duration) {
        *self = *self + span;
    }
}

impl SubAssign<Duration> for Time {
    fn sub_assign(&mut self, span: Duration) {
        *self = *self - span;
    }
}

impl AddAssign<TimeOffset> for Time {
    fn add_assign(&mut self, offset: TimeOffset) {
        *self = *self + offset;
    }
}

impl SubAssign<TimeOffset> for Time {
    fn sub_assign(&mut self, offset: TimeOffset) {
        *self = *self - offset;
    }
}

impl Duration {
    /// The empty duration.
    pub const ZERO: Self = Self(0);

    /// Constructs a duration without losing integer tick precision.
    pub const fn from_ticks(ticks: u64) -> Self {
        Self(ticks)
    }

    /// Rounds seconds to the nearest tick; halfway cases round away from zero.
    ///
    /// # Panics
    /// Panics for negative or nonfinite input or a rounded result beyond the
    /// tick range; see [`Self::try_from_seconds`].
    pub fn from_seconds(seconds: f64) -> Self {
        Self::try_from_seconds(seconds).unwrap_or_else(|error| panic!("{}", error.message))
    }

    /// Rounds seconds like [`Self::from_seconds`], reporting negative, nonfinite or out-of-range input.
    pub fn try_from_seconds(seconds: f64) -> Result<Self, crate::ValidationError> {
        try_rounded_unsigned_ticks(seconds * TICKS_PER_SECOND as f64).map(Self)
    }

    /// Returns the duration from the origin to a frame boundary, with the same
    /// ceiling rule as [`Time::from_frame`].
    ///
    /// # Panics
    /// Panics if the boundary cannot be represented as ticks.
    pub fn from_frame(frame: u64, rate: FrameRate) -> Self {
        Self::checked_from_frame(frame, rate).expect("frame time exceeds the tick range")
    }

    /// Converts a frame boundary like [`Self::from_frame`], returning None on overflow.
    pub fn checked_from_frame(frame: u64, rate: FrameRate) -> Option<Self> {
        let numerator = u128::from(frame)
            * u128::from(TICKS_PER_SECOND_UNSIGNED)
            * u128::from(rate.denominator());
        let denominator = u128::from(rate.numerator());
        let ticks = numerator / denominator + u128::from(numerator % denominator != 0);
        u64::try_from(ticks).ok().map(Self)
    }

    /// Returns the frame containing this span's end, rounding down.
    ///
    /// # Panics
    /// Panics if the frame index is outside the unsigned 64-bit range.
    pub fn to_frame(self, rate: FrameRate) -> u64 {
        self.checked_to_frame(rate)
            .expect("frame index exceeds the unsigned 64-bit range")
    }

    /// Finds the frame count like [`Self::to_frame`], returning None on overflow.
    pub fn checked_to_frame(self, rate: FrameRate) -> Option<u64> {
        let numerator = u128::from(self.0) * u128::from(rate.numerator());
        let denominator = u128::from(TICKS_PER_SECOND_UNSIGNED) * u128::from(rate.denominator());
        u64::try_from(numerator / denominator).ok()
    }

    /// Converts to floating-point seconds for animation mathematics.
    pub fn as_seconds(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }

    /// Returns the exact tick count.
    pub const fn ticks(self) -> u64 {
        self.0
    }

    /// Returns the time point this far from the origin.
    ///
    /// # Panics
    /// Panics if the duration exceeds the signed tick range.
    pub fn after_origin(self) -> Time {
        Time(i64::try_from(self.0).expect("duration exceeds the signed tick range"))
    }

    /// Returns the signed displacement of this duration, or None beyond the signed tick range.
    pub fn checked_offset(self) -> Option<TimeOffset> {
        i64::try_from(self.0).ok().map(TimeOffset::from_ticks)
    }

    /// Returns the signed displacement of this duration, for shifting time later.
    ///
    /// # Panics
    /// Panics if the duration exceeds the signed tick range; see [`Self::checked_offset`].
    pub fn offset(self) -> TimeOffset {
        self.checked_offset()
            .expect("duration exceeds the signed tick range")
    }

    /// Subtracts exactly, returning None when `other` is longer.
    pub fn checked_sub(self, other: Self) -> Option<Self> {
        self.0.checked_sub(other.0).map(Self)
    }

    /// Subtracts, stopping at [`Self::ZERO`] when `other` is longer.
    pub fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    /// Multiplies ticks exactly, returning None on overflow.
    pub fn checked_mul(self, factor: u64) -> Option<Self> {
        self.0.checked_mul(factor).map(Self)
    }

    /// Scales like the `* f64` operator, reporting a negative or nonfinite factor and overflow.
    pub fn try_mul(self, factor: f64) -> Result<Self, crate::ValidationError> {
        if factor == 1.0 {
            return Ok(self);
        }
        if !factor.is_finite() || factor < 0.0 {
            return Err(time_error(
                "factor",
                "duration factor must be finite and nonnegative",
            ));
        }
        try_rounded_unsigned_ticks(self.0 as f64 * factor).map(Self)
    }

    /// Divides like the `/ f64` operator, reporting a negative, nonfinite or zero divisor and overflow.
    pub fn try_div(self, divisor: f64) -> Result<Self, crate::ValidationError> {
        if !divisor.is_finite() || divisor <= 0.0 {
            return Err(time_error(
                "divisor",
                "duration divisor must be finite and positive",
            ));
        }
        if divisor == 1.0 {
            return Ok(self);
        }
        try_rounded_unsigned_ticks(self.0 as f64 / divisor).map(Self)
    }

    /// Returns the dimensionless ratio, reporting a zero divisor.
    pub fn try_ratio(self, divisor: Self) -> Result<f64, crate::ValidationError> {
        if divisor.0 == 0 {
            return Err(time_error(
                "divisor",
                "duration ratio requires a nonzero divisor",
            ));
        }
        Ok(self.0 as f64 / divisor.0 as f64)
    }
}

impl Add for Duration {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(
            self.0
                .checked_add(other.0)
                .expect("duration addition overflow"),
        )
    }
}

impl Sub for Duration {
    type Output = Self;

    /// Subtracts exactly.
    ///
    /// # Panics
    /// Panics when `other` is longer; see [`Duration::checked_sub`].
    fn sub(self, other: Self) -> Self {
        self.checked_sub(other)
            .expect("duration subtraction underflow")
    }
}

impl AddAssign for Duration {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

impl SubAssign for Duration {
    fn sub_assign(&mut self, other: Self) {
        *self = *self - other;
    }
}

impl std::iter::Sum for Duration {
    fn sum<Spans: Iterator<Item = Self>>(spans: Spans) -> Self {
        spans.fold(Self::ZERO, |total, span| total + span)
    }
}

impl Mul<f64> for Duration {
    type Output = Self;

    /// Scales the duration and rounds to the nearest tick, away from zero at ties.
    ///
    /// # Panics
    /// Panics for a negative or nonfinite factor or overflow; see [`Duration::try_mul`].
    fn mul(self, factor: f64) -> Self {
        self.try_mul(factor)
            .unwrap_or_else(|error| panic!("{}", error.message))
    }
}

macro_rules! integer_duration_mul {
    ($($integer:ty),+ $(,)?) => {$ (
        impl Mul<$integer> for Duration {
            type Output = Duration;

            /// Scales exactly.
            ///
            /// # Panics
            /// Panics for a negative factor or on overflow; see [`Duration::checked_mul`].
            fn mul(self, factor: $integer) -> Duration {
                Duration(u64::try_from(u128::from(self.0) * u128::try_from(factor).expect("duration multiplication overflow")).expect("duration multiplication overflow"))
            }
        }
        impl Mul<Duration> for $integer {
            type Output = Duration;
            fn mul(self, span: Duration) -> Duration { span * self }
        }
    )+};
}

integer_duration_mul!(i32, i64, u32, u64, usize);

impl Mul<Duration> for f64 {
    type Output = Duration;

    fn mul(self, span: Duration) -> Duration {
        span * self
    }
}

impl Div<f64> for Duration {
    type Output = Self;

    /// Divides the duration.
    ///
    /// # Panics
    /// Panics for a negative, nonfinite or zero divisor; see [`Duration::try_div`].
    fn div(self, divisor: f64) -> Self {
        self.try_div(divisor)
            .unwrap_or_else(|error| panic!("{}", error.message))
    }
}

impl Div for Duration {
    type Output = f64;

    /// Returns a dimensionless ratio.
    ///
    /// # Panics
    /// Panics for a zero divisor; see [`Duration::try_ratio`].
    fn div(self, divisor: Self) -> f64 {
        self.try_ratio(divisor)
            .unwrap_or_else(|error| panic!("{}", error.message))
    }
}

/// Readable nonnegative time literals, such as `2.seconds()` and `1.5.seconds()`.
///
/// Floating-point literals round to the nearest tick, away from zero at ties.
/// Negative, nonfinite or out-of-range values panic; the literals are source
/// constants, and runtime values use [`Duration::try_from_seconds`]. Integer
/// literals use exact arithmetic and panic on overflow, independently of the
/// build profile.
pub trait TimeLiteral {
    /// Converts seconds to a tick duration.
    fn seconds(self) -> Duration;

    /// Converts milliseconds to a tick duration.
    fn milliseconds(self) -> Duration;
}

impl TimeLiteral for i64 {
    fn seconds(self) -> Duration {
        Duration(
            u64::try_from(self)
                .ok()
                .and_then(|seconds| seconds.checked_mul(TICKS_PER_SECOND_UNSIGNED))
                .expect("seconds overflow"),
        )
    }

    fn milliseconds(self) -> Duration {
        Duration(
            u64::try_from(self)
                .ok()
                .and_then(|milliseconds| {
                    milliseconds.checked_mul(TICKS_PER_SECOND_UNSIGNED / 1_000)
                })
                .expect("milliseconds overflow"),
        )
    }
}

impl TimeLiteral for i32 {
    fn seconds(self) -> Duration {
        i64::from(self).seconds()
    }

    fn milliseconds(self) -> Duration {
        i64::from(self).milliseconds()
    }
}

impl TimeLiteral for u64 {
    fn seconds(self) -> Duration {
        Duration(
            self.checked_mul(TICKS_PER_SECOND_UNSIGNED)
                .expect("seconds overflow"),
        )
    }

    fn milliseconds(self) -> Duration {
        Duration(
            self.checked_mul(TICKS_PER_SECOND_UNSIGNED / 1_000)
                .expect("milliseconds overflow"),
        )
    }
}

macro_rules! unsigned_time_literal {
    ($($integer:ty),+ $(,)?) => {$ (
        impl TimeLiteral for $integer {
            fn seconds(self) -> Duration { u64::try_from(self).expect("seconds overflow").seconds() }
            fn milliseconds(self) -> Duration { u64::try_from(self).expect("milliseconds overflow").milliseconds() }
        }
    )+};
}

unsigned_time_literal!(u32, usize);

impl TimeLiteral for f64 {
    fn seconds(self) -> Duration {
        Duration::from_seconds(self)
    }

    fn milliseconds(self) -> Duration {
        Duration(
            try_rounded_unsigned_ticks(self * (TICKS_PER_SECOND / 1_000) as f64)
                .unwrap_or_else(|error| panic!("{}", error.message)),
        )
    }
}

fn try_rounded_ticks(ticks: f64) -> Result<i64, crate::ValidationError> {
    let rounded = ticks.round();
    if rounded.is_finite() && rounded >= i64::MIN as f64 && rounded < -(i64::MIN as f64) {
        Ok(rounded as i64)
    } else {
        Err(time_error(
            "seconds",
            "time must be finite and fit the signed tick range",
        ))
    }
}

fn try_rounded_unsigned_ticks(ticks: f64) -> Result<u64, crate::ValidationError> {
    let rounded = ticks.round();
    if rounded.is_finite() && rounded >= 0.0 && rounded < u64::MAX as f64 {
        Ok(rounded as u64)
    } else {
        Err(time_error(
            "seconds",
            "duration must be finite, nonnegative and fit the tick range",
        ))
    }
}

fn time_error(field: &'static str, message: &'static str) -> crate::ValidationError {
    crate::ValidationError::invalid("time", field, message)
}

#[cfg(test)]
mod tests;
