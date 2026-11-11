//! Signed time displacements for places where moving earlier is meaningful.

use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

use super::{Duration, TICKS_PER_SECOND, try_rounded_ticks};

/// A signed span of time, stored as integer flicks.
///
/// Unlike [`Duration`], an offset may be negative. It is the result of
/// `Time - Time` and the type of clip offsets and similar shifts. Arithmetic
/// panics on overflow, independently of the build profile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimeOffset(i64);

impl TimeOffset {
    /// The empty offset.
    pub const ZERO: Self = Self(0);

    /// Constructs an offset without losing integer tick precision.
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

    /// Returns the exact signed tick count.
    pub const fn ticks(self) -> i64 {
        self.0
    }

    /// Converts to floating-point seconds for animation mathematics.
    pub fn as_seconds(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }

    /// Returns true when the offset moves toward earlier time.
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    /// Returns the length of the offset without its sign.
    pub const fn magnitude(self) -> Duration {
        Duration::from_ticks(self.0.unsigned_abs())
    }

    /// Returns the offset as a duration, or None when it is negative.
    pub const fn to_duration(self) -> Option<Duration> {
        if self.0 < 0 {
            None
        } else {
            Some(Duration::from_ticks(self.0 as u64))
        }
    }
}

impl Add for TimeOffset {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(
            self.0
                .checked_add(other.0)
                .expect("offset addition overflow"),
        )
    }
}

impl Sub for TimeOffset {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self(
            self.0
                .checked_sub(other.0)
                .expect("offset subtraction overflow"),
        )
    }
}

impl AddAssign for TimeOffset {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

impl SubAssign for TimeOffset {
    fn sub_assign(&mut self, other: Self) {
        *self = *self - other;
    }
}

impl Neg for TimeOffset {
    type Output = Self;

    fn neg(self) -> Self {
        Self(self.0.checked_neg().expect("offset negation overflow"))
    }
}
