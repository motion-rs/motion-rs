//! Clock-free easing and value interpolation.

use std::{marker::PhantomData, sync::Arc};

use crate::{Duration, Point, Time, Transform, Vector2};

mod easing;
mod keyframes;
mod primitives;
mod spring;
#[cfg(test)]
mod tests;

pub use easing::Easing;
pub use keyframes::Keyframes;
pub use primitives::{
    Extrapolate, FollowPath, Noise, Oscillator, PathSample, Waveform, piecewise, remap, value_noise,
};
pub use spring::Spring;

const BEFORE_START: Time = Time::from_ticks(-1);

/// Values that can interpolate between endpoints. Progress may overshoot `0.0..=1.0`.
/// Implement this trait to sample custom values with [`Tween`] and [`Keyframes`].
/// Progress and geometry use double precision. f32 values retain their
/// own arithmetic; colors extrapolate Oklab and clamp only alpha.
pub trait Interpolate: Clone {
    /// Blends toward `target` with nominal progress zero at self and one at target.
    ///
    /// Custom implementations should be deterministic and handle overshoot
    /// intentionally. Built-in floats/pairs extrapolate; bounds preserve
    /// signed extrapolated dimensions. Colors extrapolate Oklab and clamp alpha.
    fn interpolate(&self, target: &Self, progress: f64) -> Self;
}

impl Interpolate for f32 {
    fn interpolate(&self, target: &Self, progress: f64) -> Self {
        let progress = progress as f32;
        self * (1.0 - progress) + target * progress
    }
}

impl Interpolate for f64 {
    fn interpolate(&self, target: &Self, progress: f64) -> Self {
        self * (1.0 - progress) + target * progress
    }
}

impl Interpolate for Point {
    fn interpolate(&self, target: &Self, progress: f64) -> Self {
        Point::interpolate(self, target, progress)
    }
}

impl Interpolate for Vector2 {
    fn interpolate(&self, target: &Self, progress: f64) -> Self {
        Vector2::interpolate(self, target, progress)
    }
}

impl Interpolate for Transform {
    fn interpolate(&self, target: &Self, progress: f64) -> Self {
        Transform::interpolate(self, target, progress)
    }
}

impl<Value: Interpolate, const COUNT: usize> Interpolate for [Value; COUNT] {
    fn interpolate(&self, target: &Self, progress: f64) -> Self {
        std::array::from_fn(|index| self[index].interpolate(&target[index], progress))
    }
}

impl Interpolate for crate::Color {
    fn interpolate(&self, target: &Self, progress: f64) -> Self {
        crate::Color::interpolate(self, target, progress)
    }
}

impl Interpolate for crate::Size {
    fn interpolate(&self, target: &Self, progress: f64) -> Self {
        Self::new(
            self.width.interpolate(&target.width, progress),
            self.height.interpolate(&target.height, progress),
        )
    }
}

impl Interpolate for crate::Rect {
    fn interpolate(&self, target: &Self, progress: f64) -> Self {
        Self::new(
            self.left.interpolate(&target.left, progress),
            self.top.interpolate(&target.top, progress),
            self.right.interpolate(&target.right, progress),
            self.bottom.interpolate(&target.bottom, progress),
        )
    }
}

impl<First: Interpolate, Second: Interpolate> Interpolate for (First, Second) {
    fn interpolate(&self, target: &Self, progress: f64) -> Self {
        (
            self.0.interpolate(&target.0, progress),
            self.1.interpolate(&target.1, progress),
        )
    }
}

/// A thread-safe, randomly addressable animation of values.
///
/// Finite implementations hold their endpoint values outside `[0, duration]`.
/// Infinite implementations may use negative time. Sampling must not depend on
/// earlier calls. A reported duration must be nonnegative and stable.
pub trait Animation<Value>: Send + Sync {
    /// Samples without advancing a clock or changing playback state.
    fn sample(&self, time: Time) -> Value;

    /// Returns the finite duration, or `None` for an unbounded signal.
    fn duration(&self) -> Option<Duration> {
        None
    }

    /// Transforms values without changing timing.
    fn map<Output, Function>(self, function: Function) -> Map<Self, Function, Value>
    where
        Self: Sized,
        Function: Fn(Value) -> Output + Send + Sync,
    {
        Map {
            source: self,
            function,
            input: PhantomData,
        }
    }

    /// Samples in parallel; the duration is infinite if either input is infinite.
    fn zip<OtherValue, Other>(self, other: Other) -> Zip<Self, Other>
    where
        Self: Sized,
        Other: Animation<OtherValue>,
    {
        Zip {
            first: self,
            second: other,
        }
    }

    /// Delays a finite animation, holding its initial value before the offset.
    /// Infinite signals shift their signed time instead. Negative offsets and
    /// finite duration overflow panic at construction.
    fn delay(self, offset: Duration) -> Delay<Self>
    where
        Self: Sized,
    {
        assert!(offset >= Time::ZERO, "animation delay must be nonnegative");
        let duration = checked_duration(self.duration()).map(|duration| duration + offset);
        Delay {
            source: self,
            offset,
            duration,
        }
    }

    /// Changes playback speed by a finite, strictly positive factor.
    /// Durations round to the nearest tick; a collapsed finite animation becomes
    /// instantaneous. Invalid factors and unrepresentable durations panic.
    fn speed(self, factor: f64) -> Speed<Self>
    where
        Self: Sized,
    {
        assert!(
            factor.is_finite() && factor > 0.0,
            "animation speed must be finite and positive"
        );
        let source_duration = checked_duration(self.duration());
        let duration = source_duration.map(|duration| duration / factor);
        Speed {
            source: self,
            factor,
            source_duration,
            duration,
        }
    }

    /// Runs two finite animations in sequence. The second owns their shared
    /// boundary. Infinite sources or duration overflow panic at construction.
    fn then<Next: Animation<Value>>(self, next: Next) -> Then<Self, Next>
    where
        Self: Sized,
    {
        let first_duration = finite_duration(self.duration(), "then");
        let next_duration = finite_duration(next.duration(), "then");
        Then {
            first: self,
            next,
            first_duration,
            duration: first_duration + next_duration,
        }
    }

    /// Reverses a finite animation. Infinite sources panic at construction.
    fn reverse(self) -> Reverse<Self>
    where
        Self: Sized,
    {
        let duration = finite_duration(self.duration(), "reverse");
        Reverse {
            source: self,
            duration,
        }
    }

    /// Plays a finite animation `count` times. Intermediate boundaries restart;
    /// the final boundary holds the target. Zero repeats hold the initial value.
    /// Infinite sources or duration overflow panic at construction.
    fn repeat(self, count: u32) -> Repeat<Self>
    where
        Self: Sized,
    {
        let source_duration = finite_duration(self.duration(), "repeat");
        let duration = cycle_duration(source_duration, count);
        Repeat {
            source: self,
            count,
            source_duration,
            duration,
        }
    }

    /// Plays a finite animation forward then backward, ending at its initial
    /// value. Infinite sources or duration overflow panic at construction.
    fn ping_pong(self) -> PingPong<Self>
    where
        Self: Sized,
    {
        let source_duration = finite_duration(self.duration(), "ping_pong");
        let duration = cycle_duration(source_duration, 2);
        PingPong {
            source: self,
            source_duration,
            duration,
        }
    }

    /// Shares a type-erased animation, retaining sampling and combinators.
    fn boxed(self) -> Arc<dyn Animation<Value>>
    where
        Self: Sized + 'static,
    {
        Arc::new(self)
    }
}

impl<Value, Function> Animation<Value> for Function
where
    Function: Fn(Time) -> Value + Send + Sync,
{
    fn sample(&self, time: Time) -> Value {
        self(time)
    }
}

impl<Value> Animation<Value> for Arc<dyn Animation<Value> + '_> {
    fn sample(&self, time: Time) -> Value {
        self.as_ref().sample(time)
    }

    fn duration(&self) -> Option<Duration> {
        self.as_ref().duration()
    }
}

/// An unbounded constant signal.
#[derive(Clone, Debug)]
pub struct Constant<Value> {
    value: Value,
}

impl<Value> Constant<Value> {
    /// Stores the value returned at every signed time.
    pub const fn new(value: Value) -> Self {
        Self { value }
    }
}

impl<Value: Clone + Send + Sync> Animation<Value> for Constant<Value> {
    fn sample(&self, _time: Time) -> Value {
        self.value.clone()
    }
}

/// A finite endpoint-to-endpoint interpolation with double-precision progress.
///
/// ```
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// let movement = Tween::new(Point::ORIGIN, Point::new(100.0, 40.0), 2.seconds());
/// assert_eq!(movement.sample(1.seconds()), Point::new(50.0, 20.0));
/// assert_eq!(movement.sample((-1).seconds()), Point::ORIGIN);
/// ```
#[derive(Clone, Debug)]
pub struct Tween<Value> {
    from: Value,
    to: Value,
    duration: Duration,
    easing: Easing,
}

impl<Value> Tween<Value> {
    /// Creates a linear tween. A negative duration panics at construction.
    /// A zero duration holds `from` before zero and `to` at/after zero.
    pub fn new(from: Value, to: Value, duration: Duration) -> Self {
        Self::try_new(from, to, duration).unwrap_or_else(|error| panic!("{}", error.message))
    }

    /// Creates a tween from dynamic input without panicking for negative duration.
    pub fn try_new(
        from: Value,
        to: Value,
        duration: Duration,
    ) -> Result<Self, crate::ValidationError> {
        if duration < Time::ZERO {
            return Err(crate::ValidationError::invalid(
                "tween",
                "duration",
                "tween duration must be nonnegative",
            ));
        }
        Ok(Self {
            from,
            to,
            duration,
            easing: Easing::Linear,
        })
    }

    /// Selects a f64 curve.
    /// Invalid curve parameters panic during construction.
    pub fn easing(mut self, easing: Easing) -> Self {
        self.easing = easing.validated();
        self
    }

    /// Selects a curve after recoverable validation of its built-in parameters.
    pub fn try_easing(mut self, easing: Easing) -> Result<Self, crate::ValidationError> {
        easing.try_validate()?;
        self.easing = easing;
        Ok(self)
    }
}

impl<Value: Interpolate + Send + Sync> Animation<Value> for Tween<Value> {
    fn sample(&self, time: Time) -> Value {
        if time < Time::ZERO {
            return self.from.clone();
        }
        if time >= self.duration {
            return self.to.clone();
        }
        if time == Time::ZERO {
            return self.from.clone();
        }
        let progress = self.easing.apply(time / self.duration);
        self.from.interpolate(&self.to, progress)
    }

    fn duration(&self) -> Option<Duration> {
        Some(self.duration)
    }
}

/// Value mapping produced by [`Animation::map`].
#[derive(Clone, Debug)]
pub struct Map<Source, Function, Input> {
    source: Source,
    function: Function,
    input: PhantomData<fn(Input)>,
}

impl<Input, Output, Source, Function> Animation<Output> for Map<Source, Function, Input>
where
    Source: Animation<Input>,
    Function: Fn(Input) -> Output + Send + Sync,
{
    fn sample(&self, time: Time) -> Output {
        (self.function)(self.source.sample(time))
    }

    fn duration(&self) -> Option<Duration> {
        self.source.duration()
    }
}

/// Parallel sampling produced by [`Animation::zip`].
#[derive(Clone, Debug)]
pub struct Zip<First, Second> {
    first: First,
    second: Second,
}

impl<FirstValue, SecondValue, First, Second> Animation<(FirstValue, SecondValue)>
    for Zip<First, Second>
where
    First: Animation<FirstValue>,
    Second: Animation<SecondValue>,
{
    fn sample(&self, time: Time) -> (FirstValue, SecondValue) {
        (self.first.sample(time), self.second.sample(time))
    }

    fn duration(&self) -> Option<Duration> {
        Some(self.first.duration()?.max(self.second.duration()?))
    }
}

fn checked_duration(duration: Option<Duration>) -> Option<Duration> {
    if let Some(duration) = duration {
        assert!(
            duration >= Time::ZERO,
            "animation duration must be nonnegative"
        );
    }
    duration
}

fn finite_duration(duration: Option<Duration>, operation: &str) -> Duration {
    checked_duration(duration).unwrap_or_else(|| panic!("{operation} requires a finite animation"))
}

fn cycle_duration(duration: Duration, count: u32) -> Duration {
    Time::from_ticks(
        duration
            .ticks()
            .checked_mul(i64::from(count))
            .expect("animation duration overflow"),
    )
}

/// A delayed animation produced by [`Animation::delay`].
#[derive(Clone, Debug)]
pub struct Delay<Source> {
    source: Source,
    offset: Duration,
    duration: Option<Duration>,
}

impl<Value, Source: Animation<Value>> Animation<Value> for Delay<Source> {
    fn sample(&self, time: Time) -> Value {
        let local = match self.duration {
            Some(_) if time < self.offset => BEFORE_START,
            Some(duration) if time >= duration => duration - self.offset,
            _ => time - self.offset,
        };
        self.source.sample(local)
    }

    fn duration(&self) -> Option<Duration> {
        self.duration
    }
}

/// A time-scaled animation produced by [`Animation::speed`].
#[derive(Clone, Debug)]
pub struct Speed<Source> {
    source: Source,
    factor: f64,
    source_duration: Option<Duration>,
    duration: Option<Duration>,
}

impl<Value, Source: Animation<Value>> Animation<Value> for Speed<Source> {
    fn sample(&self, time: Time) -> Value {
        let local = match (self.duration, self.source_duration) {
            (Some(_), _) if time < Time::ZERO => BEFORE_START,
            (Some(duration), Some(source_duration)) if time >= duration => source_duration,
            (Some(_), Some(_)) if self.factor == 1.0 => time,
            (Some(_), Some(source_duration)) => Time::from_ticks(
                (time.ticks() as f64 * self.factor)
                    .round()
                    .clamp(0.0, source_duration.ticks() as f64) as i64,
            ),
            _ => time * self.factor,
        };
        self.source.sample(local)
    }

    fn duration(&self) -> Option<Duration> {
        self.duration
    }
}

/// A finite sequence produced by [`Animation::then`].
#[derive(Clone, Debug)]
pub struct Then<First, Next> {
    first: First,
    next: Next,
    first_duration: Duration,
    duration: Duration,
}

impl<Value, First: Animation<Value>, Next: Animation<Value>> Animation<Value>
    for Then<First, Next>
{
    fn sample(&self, time: Time) -> Value {
        if time < Time::ZERO {
            self.first.sample(BEFORE_START)
        } else if time >= self.duration {
            self.next.sample(self.duration - self.first_duration)
        } else if time < self.first_duration {
            self.first.sample(time)
        } else {
            self.next.sample(time - self.first_duration)
        }
    }

    fn duration(&self) -> Option<Duration> {
        Some(self.duration)
    }
}

/// A reversed finite animation produced by [`Animation::reverse`].
#[derive(Clone, Debug)]
pub struct Reverse<Source> {
    source: Source,
    duration: Duration,
}

impl<Value, Source: Animation<Value>> Animation<Value> for Reverse<Source> {
    fn sample(&self, time: Time) -> Value {
        let local = if time < Time::ZERO {
            self.duration
        } else if time >= self.duration {
            BEFORE_START
        } else {
            self.duration - time
        };
        self.source.sample(local)
    }

    fn duration(&self) -> Option<Duration> {
        Some(self.duration)
    }
}

/// A finite repetition produced by [`Animation::repeat`].
#[derive(Clone, Debug)]
pub struct Repeat<Source> {
    source: Source,
    count: u32,
    source_duration: Duration,
    duration: Duration,
}

impl<Value, Source: Animation<Value>> Animation<Value> for Repeat<Source> {
    fn sample(&self, time: Time) -> Value {
        let local = if self.count == 0 || time < Time::ZERO {
            BEFORE_START
        } else if time >= self.duration {
            self.source_duration
        } else {
            Time::from_ticks(time.ticks() % self.source_duration.ticks())
        };
        self.source.sample(local)
    }

    fn duration(&self) -> Option<Duration> {
        Some(self.duration)
    }
}

/// A finite forward/backward cycle produced by [`Animation::ping_pong`].
#[derive(Clone, Debug)]
pub struct PingPong<Source> {
    source: Source,
    source_duration: Duration,
    duration: Duration,
}

impl<Value, Source: Animation<Value>> Animation<Value> for PingPong<Source> {
    fn sample(&self, time: Time) -> Value {
        let local = if time < Time::ZERO || time >= self.duration {
            BEFORE_START
        } else if time <= self.source_duration {
            time
        } else {
            self.duration - time
        };
        self.source.sample(local)
    }

    fn duration(&self) -> Option<Duration> {
        Some(self.duration)
    }
}

#[cfg(test)]
mod interpolation_tests {
    use super::*;

    #[test]
    fn interpolation_preserves_double_precision_progress() {
        #[derive(Clone)]
        struct CustomValue(f64);

        impl Interpolate for CustomValue {
            fn interpolate(&self, _target: &Self, progress: f64) -> Self {
                Self(progress)
            }
        }

        fn blend<Value: Interpolate>(from: Value, to: Value, progress: f64) -> Value {
            from.interpolate(&to, progress)
        }

        let progress = 0.500_000_000_1;
        assert_ne!(progress, f64::from(progress as f32));
        assert_eq!(blend(0.0_f64, 1.0, progress), progress);
        assert_eq!(
            blend(CustomValue(0.0), CustomValue(1.0), progress).0,
            progress
        );
        let point = blend(Point::ORIGIN, Point::new(1.0, 2.0), progress);
        assert_eq!(point.x, progress);
        assert_eq!(point.y, 2.0 * progress);
        assert_eq!(
            blend(Vector2::ZERO, Vector2::new(1.0, 0.0), progress).x,
            progress
        );
        let transform = blend(
            Transform::IDENTITY,
            Transform::translate(1.0, 0.0),
            progress,
        );
        assert_eq!(transform.apply(Point::ORIGIN).x, progress);
        let array = blend([0.0_f64; 2], [1.0_f64; 2], progress);
        assert_eq!(array, [progress; 2]);
        assert_eq!(
            blend((0.0_f64, 0.0_f64), (1.0, 2.0), progress),
            (progress, 2.0 * progress)
        );
    }
}
