//! Rational frame rates with nonzero components, reduced to lowest terms.

use std::num::NonZeroU32;

use super::Duration;

/// Builds a [`FrameRate`] of whole frames per second from an integer constant.
///
/// A zero value fails compilation. Use [`FrameRate::try_new`] for runtime values.
///
/// ```
/// use motion_core::{FrameRate, fps};
/// assert_eq!(fps!(30), FrameRate::try_new(60, 2).unwrap());
/// ```
#[macro_export]
macro_rules! fps {
    ($frames:expr) => {
        $crate::FrameRate::per_second(
            const {
                match ::std::num::NonZeroU32::new($frames) {
                    Some(frames) => frames,
                    None => panic!("frames per second must be nonzero"),
                }
            },
        )
    };
}

/// A positive rational number of frames per second, stored in reduced form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrameRate {
    numerator: NonZeroU32,
    denominator: NonZeroU32,
}

impl FrameRate {
    /// Film rate of exactly 24000/1001 frames per second.
    pub const NTSC_FILM: Self = Self::new(non_zero(24_000), non_zero(1_001));

    /// NTSC rate of exactly 30000/1001 frames per second.
    pub const NTSC: Self = Self::new(non_zero(30_000), non_zero(1_001));

    /// Double NTSC rate of exactly 60000/1001 frames per second.
    pub const NTSC_DOUBLE: Self = Self::new(non_zero(60_000), non_zero(1_001));

    /// Constructs and reduces a rational frame rate from runtime components.
    ///
    /// # Errors
    /// Returns an error when either component is zero.
    pub fn try_new(numerator: u32, denominator: u32) -> Result<Self, crate::ValidationError> {
        match (NonZeroU32::new(numerator), NonZeroU32::new(denominator)) {
            (Some(numerator), Some(denominator)) => Ok(Self::new(numerator, denominator)),
            _ => Err(crate::ValidationError::invalid(
                "frame_rate",
                "ratio",
                "frame-rate numerator and denominator must be nonzero",
            )),
        }
    }

    /// Constructs and reduces a rational frame rate.
    pub const fn new(numerator: NonZeroU32, denominator: NonZeroU32) -> Self {
        let mut common = numerator.get();
        let mut remainder = denominator.get();
        while remainder != 0 {
            let next = common % remainder;
            common = remainder;
            remainder = next;
        }
        Self {
            numerator: non_zero(numerator.get() / common),
            denominator: non_zero(denominator.get() / common),
        }
    }

    /// Constructs a rate of whole frames per second; see also [`fps!`](crate::fps).
    pub const fn per_second(frames: NonZeroU32) -> Self {
        Self {
            numerator: frames,
            denominator: NonZeroU32::MIN,
        }
    }

    /// Returns the numerator of the reduced rate.
    pub const fn numerator(self) -> u32 {
        self.numerator.get()
    }

    /// Returns the denominator of the reduced rate.
    pub const fn denominator(self) -> u32 {
        self.denominator.get()
    }

    /// Returns the first frame's duration, rounded up if not tick-exact.
    /// Use [`Time::from_frame`](super::Time::from_frame) for later boundaries to avoid accumulated rounding.
    pub fn frame_duration(self) -> Duration {
        Duration::from_frame(1, self)
    }
}

impl From<NonZeroU32> for FrameRate {
    fn from(frames_per_second: NonZeroU32) -> Self {
        Self::per_second(frames_per_second)
    }
}

/// Wraps a component that is nonzero by construction; zero fails constant evaluation.
const fn non_zero(value: u32) -> NonZeroU32 {
    match NonZeroU32::new(value) {
        Some(value) => value,
        None => panic!("frame-rate component must be nonzero"),
    }
}
