//! A half-open time interval that turns a time point into clamped progress.

use crate::{Duration, Easing, Time};

/// A half-open interval `[start, start + duration)` on a timeline.
///
/// This is the single definition of "time range to progress" shared by
/// `SceneContext::progress_in`, `SceneContext::ease_in` and [`crate::Tween`].
/// Comparisons use integer ticks; a zero duration jumps from zero to one at `start`.
///
/// ```
/// use motion_core::prelude::*;
/// use motion_core::Interval;
/// let interval = Interval::new(Time::ZERO + 1.seconds(), 2.seconds());
/// assert_eq!(interval.progress(Time::ZERO), 0.0);
/// assert_eq!(interval.progress(Time::ZERO + 2.seconds()), 0.5);
/// assert_eq!(interval.progress(Time::ZERO + 5.seconds()), 1.0);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    start: Time,
    duration: Duration,
}

impl Interval {
    /// Creates an interval starting at `start` and lasting `duration`.
    pub const fn new(start: Time, duration: Duration) -> Self {
        Self { start, duration }
    }

    /// Returns the first time inside the interval.
    pub const fn start(&self) -> Time {
        self.start
    }

    /// Returns the nonnegative length of the interval.
    pub const fn duration(&self) -> Duration {
        self.duration
    }

    /// Reports whether `time` lies in `[start, start + duration)`.
    ///
    /// Wide integer differences allow queries across opposite signed tick extremes.
    pub fn contains(&self, time: Time) -> bool {
        let elapsed = self.elapsed(time);
        elapsed >= 0 && elapsed < i128::from(self.duration.ticks())
    }

    /// Returns progress clamped to `0.0..=1.0`: zero before the start, one at
    /// or after the end.
    pub fn progress(&self, time: Time) -> f64 {
        let elapsed = self.elapsed(time);
        if elapsed < 0 {
            0.0
        } else if elapsed >= i128::from(self.duration.ticks()) {
            1.0
        } else {
            elapsed as f64 / self.duration.ticks() as f64
        }
    }

    /// Returns [`Self::progress`] shaped by an easing curve.
    pub fn ease(&self, time: Time, easing: Easing) -> f64 {
        easing.apply(self.progress(time))
    }

    fn elapsed(&self, time: Time) -> i128 {
        i128::from(time.ticks()) - i128::from(self.start.ticks())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TimeLiteral;

    #[test]
    fn progress_clamps_and_zero_duration_jumps() {
        let interval = Interval::new(Time::ZERO + 1.seconds(), 2.seconds());
        assert!(!interval.contains(Time::ZERO));
        assert!(interval.contains(Time::ZERO + 1.seconds()));
        assert!(!interval.contains(Time::ZERO + 3.seconds()));
        assert_eq!(interval.progress(Time::ZERO + 1.seconds()), 0.0);
        assert_eq!(interval.progress(Time::ZERO + 3.seconds()), 1.0);

        let jump = Interval::new(Time::ZERO, Duration::ZERO);
        assert_eq!(jump.progress(Time::from_ticks(-1)), 0.0);
        assert_eq!(jump.progress(Time::ZERO), 1.0);
        assert!(!jump.contains(Time::ZERO));
    }

    #[test]
    fn extreme_ticks_do_not_overflow() {
        let interval = Interval::new(Time::from_ticks(i64::MIN), Duration::ZERO);
        assert_eq!(interval.progress(Time::from_ticks(i64::MAX)), 1.0);
    }
}
