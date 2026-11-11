//! Clock-free easing and value interpolation.

use std::{marker::PhantomData, sync::Arc};

use crate::{Duration, Point, Time, Transform, Vector2};

mod easing;
mod interval;
mod keyframes;
mod primitives;
mod spring;
#[cfg(test)]
mod tests;

pub use easing::Easing;
pub use interval::Interval;
pub use keyframes::Keyframes;
pub use primitives::{
    Extrapolate, FollowPath, Noise, NoiseBuilder, Oscillator, OscillatorBuilder, PathSample,
    Waveform, piecewise, remap, try_piecewise, try_remap, try_value_noise, value_noise,
};
pub use spring::{Spring, SpringBuilder};

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

/// A thread-safe, randomly addressable signal of values over signed local time.
///
/// A signal has no end: it may be sampled at any signed time. Sampling must not
/// depend on earlier calls. Signals with a known end implement
/// [`FiniteAnimation`], which owns the duration-aware combinators.
pub trait Signal<Value>: Send + Sync {
    /// Samples without advancing a clock or changing playback state.
    fn sample(&self, time: Time) -> Value;

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

    /// Samples in parallel; the result is finite only if both inputs are finite.
    fn zip<OtherValue, Other>(self, other: Other) -> Zip<Self, Other>
    where
        Self: Sized,
        Other: Signal<OtherValue>,
    {
        Zip {
            first: self,
            second: other,
        }
    }

    /// Shares a type-erased signal.
    fn boxed(self) -> Arc<dyn Signal<Value>>
    where
        Self: Sized + 'static,
    {
        Arc::new(self)
    }
}

/// A [`Signal`] with a nonnegative, stable duration.
///
/// Finite animations hold their endpoint values outside `[0, duration]`.
/// Sequencing combinators exist only here, so an unbounded signal cannot be
/// delayed, sequenced or repeated by construction.
pub trait FiniteAnimation<Value>: Signal<Value> {
    /// Returns the nonnegative duration.
    fn duration(&self) -> Duration;

    /// Delays the animation, holding its initial value before the offset.
    ///
    /// # Panics
    /// Panics on duration overflow; see [`Self::try_delay`].
    fn delay(self, offset: Duration) -> Delay<Self>
    where
        Self: Sized,
    {
        self.try_delay(offset)
            .unwrap_or_else(|error| panic!("{}", error.message))
    }

    /// Delays like [`Self::delay`], reporting duration overflow.
    fn try_delay(self, offset: Duration) -> Result<Delay<Self>, crate::ValidationError>
    where
        Self: Sized,
    {
        let duration = self
            .duration()
            .ticks()
            .checked_add(offset.ticks())
            .map(Duration::from_ticks)
            .filter(|duration| duration.checked_offset().is_some())
            .ok_or_else(|| {
                crate::ValidationError::invalid(
                    "animation",
                    "delay",
                    "animation delay overflows the tick range",
                )
            })?;
        Ok(Delay {
            source: self,
            offset,
            duration,
        })
    }

    /// Changes playback speed by a finite, strictly positive factor.
    /// Durations round to the nearest tick; a collapsed animation becomes
    /// instantaneous.
    ///
    /// # Panics
    /// Panics for an invalid factor or unrepresentable duration; see [`Self::try_speed`].
    fn speed(self, factor: f64) -> Speed<Self>
    where
        Self: Sized,
    {
        self.try_speed(factor)
            .unwrap_or_else(|error| panic!("{}", error.message))
    }

    /// Changes speed like [`Self::speed`], reporting an invalid factor or
    /// unrepresentable duration.
    fn try_speed(self, factor: f64) -> Result<Speed<Self>, crate::ValidationError>
    where
        Self: Sized,
    {
        if !(factor.is_finite() && factor > 0.0) {
            return Err(crate::ValidationError::invalid(
                "animation",
                "speed",
                "animation speed must be finite and positive",
            ));
        }
        let source_duration = self.duration();
        let duration = source_duration.try_div(factor)?;
        Ok(Speed {
            source: self,
            factor,
            source_duration,
            duration,
        })
    }

    /// Runs two animations in sequence. The second owns their shared
    /// boundary. Duration overflow panics at construction.
    fn then<Next: FiniteAnimation<Value>>(self, next: Next) -> Then<Self, Next>
    where
        Self: Sized,
    {
        let first_duration = self.duration();
        let next_duration = next.duration();
        Then {
            first: self,
            next,
            first_duration,
            duration: Duration::from_ticks(
                first_duration
                    .ticks()
                    .checked_add(next_duration.ticks())
                    .filter(|ticks| i64::try_from(*ticks).is_ok())
                    .expect("animation duration overflow"),
            ),
        }
    }

    /// Reverses the animation.
    fn reverse(self) -> Reverse<Self>
    where
        Self: Sized,
    {
        let duration = self.duration();
        Reverse {
            source: self,
            duration,
        }
    }

    /// Plays the animation `count` times. Intermediate boundaries restart;
    /// the final boundary holds the target. Zero repeats hold the initial value.
    /// Duration overflow panics at construction.
    fn repeat(self, count: u32) -> Repeat<Self>
    where
        Self: Sized,
    {
        let source_duration = self.duration();
        let duration = cycle_duration(source_duration, count);
        Repeat {
            source: self,
            count,
            source_duration,
            duration,
        }
    }

    /// Plays the animation forward then backward, ending at its initial
    /// value. Duration overflow panics at construction.
    fn ping_pong(self) -> PingPong<Self>
    where
        Self: Sized,
    {
        let source_duration = self.duration();
        let duration = cycle_duration(source_duration, 2);
        PingPong {
            source: self,
            source_duration,
            duration,
        }
    }

    /// Shares a type-erased animation, retaining sampling, duration and combinators.
    fn boxed_finite(self) -> Arc<dyn FiniteAnimation<Value>>
    where
        Self: Sized + 'static,
    {
        Arc::new(self)
    }
}

impl<Value, Function> Signal<Value> for Function
where
    Function: Fn(Time) -> Value + Send + Sync,
{
    fn sample(&self, time: Time) -> Value {
        self(time)
    }
}

impl<Value> Signal<Value> for Arc<dyn Signal<Value> + '_> {
    fn sample(&self, time: Time) -> Value {
        self.as_ref().sample(time)
    }
}

impl<Value> Signal<Value> for Arc<dyn FiniteAnimation<Value> + '_> {
    fn sample(&self, time: Time) -> Value {
        self.as_ref().sample(time)
    }
}

impl<Value> FiniteAnimation<Value> for Arc<dyn FiniteAnimation<Value> + '_> {
    fn duration(&self) -> Duration {
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

impl<Value: Clone + Send + Sync> Signal<Value> for Constant<Value> {
    fn sample(&self, _time: Time) -> Value {
        self.value.clone()
    }
}

/// A finite endpoint-to-endpoint interpolation with double-precision progress.
///
/// ```
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// let movement = Tween::new(Point::ORIGIN, Point::new(100.0, 40.0), 2.seconds());
/// assert_eq!(movement.sample(1.seconds().after_origin()), Point::new(50.0, 20.0));
/// assert_eq!(movement.sample(Time::from_seconds(-1.0)), Point::ORIGIN);
/// ```
#[derive(Clone, Debug)]
pub struct Tween<Value> {
    from: Value,
    to: Value,
    interval: Interval,
    easing: Easing,
}

impl<Value> Tween<Value> {
    /// Creates a linear tween.
    /// A zero duration holds `from` before zero and `to` at/after zero.
    pub fn new(from: Value, to: Value, duration: Duration) -> Self {
        Self {
            from,
            to,
            interval: Interval::new(Time::ZERO, duration),
            easing: Easing::Linear,
        }
    }

    /// Selects a curve.
    pub fn easing(mut self, easing: Easing) -> Self {
        self.easing = easing;
        self
    }
}

impl<Value: Interpolate + Send + Sync> Signal<Value> for Tween<Value> {
    fn sample(&self, time: Time) -> Value {
        let progress = self.interval.progress(time);
        if progress <= 0.0 {
            return self.from.clone();
        }
        if progress >= 1.0 {
            return self.to.clone();
        }
        self.from.interpolate(&self.to, self.easing.apply(progress))
    }
}

impl<Value: Interpolate + Send + Sync> FiniteAnimation<Value> for Tween<Value> {
    fn duration(&self) -> Duration {
        self.interval.duration()
    }
}

/// Value mapping produced by [`Signal::map`].
#[derive(Clone, Debug)]
pub struct Map<Source, Function, Input> {
    source: Source,
    function: Function,
    input: PhantomData<fn(Input)>,
}

impl<Input, Output, Source, Function> Signal<Output> for Map<Source, Function, Input>
where
    Source: Signal<Input>,
    Function: Fn(Input) -> Output + Send + Sync,
{
    fn sample(&self, time: Time) -> Output {
        (self.function)(self.source.sample(time))
    }
}

impl<Input, Output, Source, Function> FiniteAnimation<Output> for Map<Source, Function, Input>
where
    Source: FiniteAnimation<Input>,
    Function: Fn(Input) -> Output + Send + Sync,
{
    fn duration(&self) -> Duration {
        self.source.duration()
    }
}

/// Parallel sampling produced by [`Signal::zip`].
#[derive(Clone, Debug)]
pub struct Zip<First, Second> {
    first: First,
    second: Second,
}

impl<FirstValue, SecondValue, First, Second> Signal<(FirstValue, SecondValue)>
    for Zip<First, Second>
where
    First: Signal<FirstValue>,
    Second: Signal<SecondValue>,
{
    fn sample(&self, time: Time) -> (FirstValue, SecondValue) {
        (self.first.sample(time), self.second.sample(time))
    }
}

impl<FirstValue, SecondValue, First, Second> FiniteAnimation<(FirstValue, SecondValue)>
    for Zip<First, Second>
where
    First: FiniteAnimation<FirstValue>,
    Second: FiniteAnimation<SecondValue>,
{
    fn duration(&self) -> Duration {
        self.first.duration().max(self.second.duration())
    }
}

/// Combines durations for sampling, which addresses them as signed time points.
fn cycle_duration(duration: Duration, count: u32) -> Duration {
    duration
        .checked_mul(u64::from(count))
        .filter(|duration| duration.checked_offset().is_some())
        .expect("animation duration overflow")
}

/// Returns the time point at the end of a duration measured from the origin.
fn at(duration: Duration) -> Time {
    duration.after_origin()
}

/// A delayed animation produced by [`FiniteAnimation::delay`].
#[derive(Clone, Debug)]
pub struct Delay<Source> {
    source: Source,
    offset: Duration,
    duration: Duration,
}

impl<Value, Source: FiniteAnimation<Value>> Signal<Value> for Delay<Source> {
    fn sample(&self, time: Time) -> Value {
        let local = if time < at(self.offset) {
            BEFORE_START
        } else if time >= at(self.duration) {
            at(self.duration - self.offset)
        } else {
            time - self.offset
        };
        self.source.sample(local)
    }
}

impl<Value, Source: FiniteAnimation<Value>> FiniteAnimation<Value> for Delay<Source> {
    fn duration(&self) -> Duration {
        self.duration
    }
}

/// A time-scaled animation produced by [`FiniteAnimation::speed`].
#[derive(Clone, Debug)]
pub struct Speed<Source> {
    source: Source,
    factor: f64,
    source_duration: Duration,
    duration: Duration,
}

impl<Value, Source: FiniteAnimation<Value>> Signal<Value> for Speed<Source> {
    fn sample(&self, time: Time) -> Value {
        let local = if time < Time::ZERO {
            BEFORE_START
        } else if time >= at(self.duration) {
            at(self.source_duration)
        } else if self.factor == 1.0 {
            time
        } else {
            Time::from_ticks(
                (time.ticks() as f64 * self.factor)
                    .round()
                    .clamp(0.0, self.source_duration.ticks() as f64) as i64,
            )
        };
        self.source.sample(local)
    }
}

impl<Value, Source: FiniteAnimation<Value>> FiniteAnimation<Value> for Speed<Source> {
    fn duration(&self) -> Duration {
        self.duration
    }
}

/// A finite sequence produced by [`FiniteAnimation::then`].
#[derive(Clone, Debug)]
pub struct Then<First, Next> {
    first: First,
    next: Next,
    first_duration: Duration,
    duration: Duration,
}

impl<Value, First: FiniteAnimation<Value>, Next: FiniteAnimation<Value>> Signal<Value>
    for Then<First, Next>
{
    fn sample(&self, time: Time) -> Value {
        if time < Time::ZERO {
            self.first.sample(BEFORE_START)
        } else if time >= at(self.duration) {
            self.next.sample(at(self.duration - self.first_duration))
        } else if time < at(self.first_duration) {
            self.first.sample(time)
        } else {
            self.next.sample(time - self.first_duration)
        }
    }
}

impl<Value, First: FiniteAnimation<Value>, Next: FiniteAnimation<Value>> FiniteAnimation<Value>
    for Then<First, Next>
{
    fn duration(&self) -> Duration {
        self.duration
    }
}

/// A reversed finite animation produced by [`FiniteAnimation::reverse`].
#[derive(Clone, Debug)]
pub struct Reverse<Source> {
    source: Source,
    duration: Duration,
}

impl<Value, Source: FiniteAnimation<Value>> Signal<Value> for Reverse<Source> {
    fn sample(&self, time: Time) -> Value {
        let local = if time < Time::ZERO {
            at(self.duration)
        } else if time >= at(self.duration) {
            BEFORE_START
        } else {
            at(self.duration) - time.since_origin()
        };
        self.source.sample(local)
    }
}

impl<Value, Source: FiniteAnimation<Value>> FiniteAnimation<Value> for Reverse<Source> {
    fn duration(&self) -> Duration {
        self.duration
    }
}

/// A finite repetition produced by [`FiniteAnimation::repeat`].
#[derive(Clone, Debug)]
pub struct Repeat<Source> {
    source: Source,
    count: u32,
    source_duration: Duration,
    duration: Duration,
}

impl<Value, Source: FiniteAnimation<Value>> Signal<Value> for Repeat<Source> {
    fn sample(&self, time: Time) -> Value {
        let local = if self.count == 0 || time < Time::ZERO {
            BEFORE_START
        } else if time >= at(self.duration) {
            at(self.source_duration)
        } else {
            Time::from_ticks(time.ticks() % at(self.source_duration).ticks())
        };
        self.source.sample(local)
    }
}

impl<Value, Source: FiniteAnimation<Value>> FiniteAnimation<Value> for Repeat<Source> {
    fn duration(&self) -> Duration {
        self.duration
    }
}

/// A finite forward/backward cycle produced by [`FiniteAnimation::ping_pong`].
#[derive(Clone, Debug)]
pub struct PingPong<Source> {
    source: Source,
    source_duration: Duration,
    duration: Duration,
}

impl<Value, Source: FiniteAnimation<Value>> Signal<Value> for PingPong<Source> {
    fn sample(&self, time: Time) -> Value {
        let local = if time < Time::ZERO || time >= at(self.duration) {
            BEFORE_START
        } else if time <= at(self.source_duration) {
            time
        } else {
            at(self.duration) - time.since_origin()
        };
        self.source.sample(local)
    }
}

impl<Value, Source: FiniteAnimation<Value>> FiniteAnimation<Value> for PingPong<Source> {
    fn duration(&self) -> Duration {
        self.duration
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
