//! Finite tick-addressed keyframe tracks with explicit interpolation and holds.

use super::{Easing, FiniteAnimation, Signal};
use crate::{Duration, Interpolate, Time, ValidationError};

#[derive(Clone, Debug)]
struct Keyframe<Value> {
    at: Time,
    value: Value,
    easing: Option<Easing>,
}

/// A nonempty track starting at zero, with absolute destinations and relative holds.
///
/// Exact timestamps return stored values. Intervals use the arriving point's
/// easing, while holds clone their value without invoking interpolation. Sampling
/// is stateless and uses integer ticks for interval selection before f64 progress.
///
/// This is the canonical [`crate::Keyframes`] type.
///
/// ```
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// let track = Keyframes::new(0.0_f64)
///     .to(1.seconds(), 10.0, Easing::QuadraticIn)
///     .hold(500.milliseconds())
///     .to(2.5.seconds(), 20.0, Easing::Linear);
/// assert_eq!(track.sample(500.milliseconds().after_origin()), 2.5);
/// assert_eq!(track.sample(1250.milliseconds().after_origin()), 10.0);
/// assert_eq!(track.sample(2.seconds().after_origin()), 15.0);
/// assert_eq!(track.duration(), 2.5.seconds());
/// ```
#[derive(Clone, Debug)]
pub struct Keyframes<Value> {
    points: Vec<Keyframe<Value>>,
}

impl<Value> Keyframes<Value> {
    /// Starts with a value at zero. Without further points the finite duration
    /// is zero and every sample returns this value.
    pub fn new(value: Value) -> Self {
        Self {
            points: vec![Keyframe {
                at: Time::ZERO,
                value,
                easing: None,
            }],
        }
    }

    /// Appends a destination at an absolute local time, not an added duration.
    /// The easing controls the segment arriving at this value.
    ///
    /// # Panics
    /// Panics unless `at` is strictly later than the previous point, including
    /// any appended hold. Equal and descending times are not sorted or converted
    /// into instantaneous jumps.
    pub fn to(self, at: Duration, value: Value, easing: Easing) -> Self {
        self.try_to(at, value, easing)
            .unwrap_or_else(|error| panic!("{}", error.message))
    }

    /// Appends a destination like [`Self::to`], reporting non-increasing times.
    pub fn try_to(
        mut self,
        at: Duration,
        value: Value,
        easing: Easing,
    ) -> Result<Self, ValidationError> {
        let at = at
            .checked_offset()
            .map(|offset| Time::ZERO + offset)
            .filter(|at| *at > self.last().at)
            .ok_or_else(|| {
                ValidationError::invalid(
                    "keyframes",
                    "at",
                    "keyframe times must be strictly increasing",
                )
            })?;
        self.points.push(Keyframe {
            at,
            value,
            easing: Some(easing),
        });
        Ok(self)
    }

    fn last(&self) -> &Keyframe<Value> {
        self.points
            .last()
            .expect("keyframes contain an initial value")
    }
}

impl<Value: Clone> Keyframes<Value> {
    /// Holds the last value for an additional duration, without interpolation.
    /// Zero is a no-op, leaving the last timestamp unchanged.
    ///
    /// # Panics
    /// Panics on signed tick overflow; see [`Self::try_hold`].
    pub fn hold(self, duration: Duration) -> Self {
        self.try_hold(duration)
            .unwrap_or_else(|error| panic!("{}", error.message))
    }

    /// Holds like [`Self::hold`], reporting tick overflow.
    pub fn try_hold(mut self, duration: Duration) -> Result<Self, ValidationError> {
        if duration == Duration::ZERO {
            return Ok(self);
        }
        let last = self.last();
        let at = last.at.checked_add(duration).ok_or_else(|| {
            ValidationError::invalid(
                "keyframes",
                "hold",
                "keyframe hold overflows the tick range",
            )
        })?;
        let point = Keyframe {
            at,
            value: last.value.clone(),
            easing: None,
        };
        self.points.push(point);
        Ok(self)
    }
}

impl<Value: Interpolate + Send + Sync> Signal<Value> for Keyframes<Value> {
    fn sample(&self, time: Time) -> Value {
        let upper = self.points.partition_point(|point| point.at <= time);
        if upper == 0 {
            return self.points[0].value.clone();
        }
        let left = &self.points[upper - 1];
        if upper == self.points.len() || time == left.at {
            return left.value.clone();
        }
        let right = &self.points[upper];
        let Some(easing) = &right.easing else {
            return left.value.clone();
        };
        let duration = right.at - left.at;
        let progress = easing.apply((time - left.at).ticks() as f64 / duration.ticks() as f64);
        left.value.interpolate(&right.value, progress)
    }
}

impl<Value: Interpolate + Send + Sync> FiniteAnimation<Value> for Keyframes<Value> {
    fn duration(&self) -> Duration {
        self.last().at.since_origin().magnitude()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Point, TimeLiteral, TimeOffset, Tween};

    #[test]
    fn absolute_destinations_relative_holds_and_arriving_easing_are_seekable() {
        let track = Keyframes::new(0.0_f64)
            .to(1.seconds(), 10.0, Easing::QuadraticIn)
            .hold(500.milliseconds())
            .to(2.5.seconds(), 20.0, Easing::Linear);
        assert_eq!(track.duration(), 2.5.seconds());
        for (milliseconds, expected) in [
            (2_000, 15.0),
            (0, 0.0),
            (500, 2.5),
            (1_000, 10.0),
            (1_250, 10.0),
            (1_500, 10.0),
            (2_500, 20.0),
            (500, 2.5),
        ] {
            assert_eq!(
                track.sample(milliseconds.milliseconds().after_origin()),
                expected
            );
        }
        assert_eq!(track.sample(Time::from_ticks(i64::MIN)), 0.0);
        assert_eq!(track.sample(Time::from_ticks(i64::MAX)), 20.0);
    }

    #[test]
    fn progress_and_adjacent_ticks_survive_large_absolute_timestamps() {
        let track =
            Keyframes::new(0.0_f64).to(Duration::from_ticks(10_000_000_000), 1.0, Easing::Linear);
        let progress = track.sample(Time::from_ticks(5_000_000_001));
        assert_eq!(progress, 0.500_000_000_1);
        assert_ne!(progress, f64::from(progress as f32));
        let start = Duration::from_ticks(1_u64 << 53);
        let end = start + Duration::from_ticks(3);
        let track = Keyframes::new(0.0_f64)
            .hold(start)
            .to(end, 3.0, Easing::Linear)
            .hold(Duration::from_ticks(i64::MAX as u64) - end);
        assert_eq!(track.duration(), Duration::from_ticks(i64::MAX as u64));
        assert_eq!(track.sample(start.after_origin()), 0.0);
        assert_eq!(
            track.sample((start + Duration::from_ticks(1)).after_origin()),
            1.0
        );
        assert_eq!(
            track.sample((start + Duration::from_ticks(2)).after_origin()),
            2.0
        );
        assert_eq!(track.sample(end.after_origin()), 3.0);
        assert_eq!(track.sample(Time::from_ticks(i64::MAX)), 3.0);
        let last_tick = Keyframes::new(2.0_f64)
            .hold(Duration::from_ticks(i64::MAX as u64 - 1))
            .to(Duration::from_ticks(i64::MAX as u64), 7.0, Easing::Linear);
        assert_eq!(last_tick.sample(Time::from_ticks(i64::MAX - 1)), 2.0);
        assert_eq!(last_tick.sample(Time::from_ticks(i64::MAX)), 7.0);
    }

    #[test]
    fn holds_and_exact_points_never_call_value_interpolation() {
        #[derive(Clone, Debug, PartialEq)]
        struct ExactOnly(u32);

        impl Interpolate for ExactOnly {
            fn interpolate(&self, _target: &Self, _progress: f64) -> Self {
                panic!("hold or exact timestamp must return the stored value");
            }
        }
        let track = Keyframes::new(ExactOnly(1))
            .to(Duration::from_ticks(10), ExactOnly(2), Easing::QuadraticOut)
            .hold(Duration::from_ticks(5));
        for (ticks, expected) in [
            (i64::MIN, 1),
            (0, 1),
            (10, 2),
            (12, 2),
            (15, 2),
            (i64::MAX, 2),
        ] {
            assert_eq!(track.sample(Time::from_ticks(ticks)), ExactOnly(expected));
        }
        let constant = Keyframes::new(ExactOnly(3));
        assert_eq!(constant.duration(), Duration::ZERO);
        assert_eq!(constant.sample(Time::ZERO), ExactOnly(3));
        assert_eq!(constant.sample(Time::from_ticks(i64::MAX)), ExactOnly(3));
    }

    #[test]
    fn invalid_order_and_hold_overflow_fail_during_building() {
        assert!(
            std::panic::catch_unwind(|| {
                Keyframes::new(0.0_f64).to(Duration::ZERO, 1.0, Easing::Linear)
            })
            .is_err()
        );

        fn track() -> Keyframes<f64> {
            Keyframes::new(0.0_f64).hold(Duration::from_ticks(10))
        }
        for ticks in [9, 10] {
            assert!(
                std::panic::catch_unwind(|| {
                    track().to(Duration::from_ticks(ticks), 1.0, Easing::Linear)
                })
                .is_err()
            );
        }
        let unchanged = track().hold(Duration::ZERO);
        assert_eq!(unchanged.points.len(), 2);
        assert_eq!(unchanged.duration(), Duration::from_ticks(10));
        assert!(
            std::panic::catch_unwind(|| {
                Keyframes::new(0.0_f64)
                    .hold(Duration::from_ticks(i64::MAX as u64))
                    .hold(Duration::from_ticks(1))
            })
            .is_err()
        );
    }

    #[test]
    fn tracks_use_existing_combinators_dynamic_dispatch_and_local_frames() -> crate::Result {
        let track = crate::Keyframes::new(0.0_f64)
            .to(1.seconds(), 10.0, Easing::Linear)
            .hold(500.milliseconds())
            .to(2.seconds(), 20.0, Easing::Linear);
        let cache = crate::SceneCache::new(0);
        let local = crate::SceneContext::new(
            1500.milliseconds().after_origin(),
            crate::fps!(60),
            crate::Size::new(64.0, 64.0),
            &cache,
        )
        .shifted(TimeOffset::from_seconds(1.0));
        assert_eq!(local.sample(&track), 5.0);
        let boxed = track
            .clone()
            .map(|value| Point::new(value, value * 2.0))
            .boxed();
        assert_eq!(local.sample(boxed.as_ref()), Point::new(5.0, 10.0));
        assert_eq!(
            track
                .clone()
                .reverse()
                .sample(500.milliseconds().after_origin()),
            10.0
        );
        let repeated = track.clone().repeat(2).delay(250.milliseconds());
        assert_eq!(repeated.duration(), 4.25.seconds());
        assert_eq!(repeated.sample(2.25.seconds().after_origin()), 0.0);
        assert_eq!(repeated.sample(4.25.seconds().after_origin()), 20.0);
        let joined = track.then(Tween::new(20.0_f64, 40.0, 1.seconds()));
        assert_eq!(joined.duration(), 3.seconds());
        assert_eq!(joined.sample(2.5.seconds().after_origin()), 30.0);
        assert_eq!(
            std::thread::spawn(move || boxed.sample(1.25.seconds().after_origin()))
                .join()
                .unwrap(),
            Point::new(10.0, 20.0)
        );
        Ok(())
    }

    #[test]
    fn moving_segments_match_double_precision_tweens_including_overshoot() {
        for easing in [
            Easing::Linear,
            Easing::QuadraticIn,
            Easing::QuadraticOut,
            Easing::QuadraticInOut,
            Easing::CubicBezier {
                x1: 0.3,
                y1: 1.8,
                x2: 0.7,
                y2: 1.8,
            },
        ] {
            let track = Keyframes::new(10.0_f64).hold(300.milliseconds()).to(
                2.3.seconds(),
                30.0,
                easing.clone(),
            );
            let tween = Tween::new(10.0_f64, 30.0, 2.seconds()).easing(easing);
            for milliseconds in [1_000, 0, 1, 500, 1_999, 2_000] {
                let time = milliseconds.milliseconds();
                assert_eq!(
                    track.sample((300.milliseconds() + time).after_origin()),
                    tween.sample(time.after_origin())
                );
            }
        }
    }
}
