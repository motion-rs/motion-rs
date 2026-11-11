//! Conversion of animated document values into sampled engine animations.

use super::Compiler;
use crate::value::{
    Animated, ColorValue, EasingSpec, ExprSpec, KeyframesSpec, MotionSpec, NoiseSpec,
    OscillateSpec, TweenSpec,
};
use motion_core::{
    Color, Duration, Easing, FiniteAnimation, Interpolate, Keyframes, Noise, Oscillator, Signal,
    Tween,
};
use std::sync::Arc;

/// Largest accepted magnitude for oscillator and noise parameters.
const MAX_SIGNAL_MAGNITUDE: f64 = 1e12;

/// Largest accepted oscillator or noise frequency and lacunarity.
const MAX_FREQUENCY: f64 = 1e6;

/// A compiled property: a constant or a shared clock-free sampler.
pub(crate) enum Prop<Engine> {
    Fixed(Engine),
    Varying(Arc<dyn Signal<Engine>>),
}

impl<Engine: Clone> Prop<Engine> {
    pub(crate) fn at(&self, time: Duration) -> Engine {
        match self {
            Self::Fixed(value) => value.clone(),
            Self::Varying(animation) => animation.sample(time),
        }
    }

    pub(crate) fn varying(&self) -> bool {
        matches!(self, Self::Varying(_))
    }
}

/// Flattens an optional compiled property: absent stays `Some(None)`, failed becomes `None`.
pub(crate) trait Lift<T> {
    fn lift(self) -> Option<Option<T>>;
}

impl<T> Lift<T> for Option<Option<T>> {
    fn lift(self) -> Option<Option<T>> {
        match self {
            None => Some(None),
            Some(inner) => inner.map(Some),
        }
    }
}

/// A document value type with an interpolable engine representation.
pub(crate) trait Value: Clone {
    type Engine: Interpolate + Send + Sync + 'static;

    fn engine(&self) -> Result<Self::Engine, &'static str>;

    /// Number of numeric components, or zero for values that are not numeric.
    const ARITY: usize = 0;

    /// Combines one numeric signal per component into a value animation.
    fn from_signals(_: &[Arc<dyn Signal<f64>>]) -> Option<Arc<dyn Signal<Self::Engine>>> {
        None
    }
}

impl Value for f64 {
    type Engine = f64;

    fn engine(&self) -> Result<f64, &'static str> {
        if self.is_finite() {
            Ok(*self)
        } else {
            Err("Value must be finite")
        }
    }

    const ARITY: usize = 1;

    fn from_signals(signals: &[Arc<dyn Signal<f64>>]) -> Option<Arc<dyn Signal<f64>>> {
        signals.first().cloned()
    }
}

impl<const COUNT: usize> Value for [f64; COUNT] {
    type Engine = Self;

    const ARITY: usize = COUNT;

    fn from_signals(signals: &[Arc<dyn Signal<f64>>]) -> Option<Arc<dyn Signal<Self>>> {
        let signals: Vec<_> = signals.to_vec();
        (signals.len() == COUNT).then(|| {
            Arc::new(move |time: Duration| std::array::from_fn(|index| signals[index].sample(time)))
                as Arc<dyn Signal<Self>>
        })
    }

    fn engine(&self) -> Result<Self, &'static str> {
        if self.iter().all(|value| value.is_finite()) {
            Ok(*self)
        } else {
            Err("Every component must be finite")
        }
    }
}

impl Value for ColorValue {
    type Engine = Color;

    fn engine(&self) -> Result<Color, &'static str> {
        Ok(self.color())
    }
}

impl Compiler<'_> {
    /// Compiles a property, recording diagnostics at `path`.
    pub(crate) fn animated<T: Value>(
        &mut self,
        value: &Animated<T>,
        path: &str,
    ) -> Option<Prop<T::Engine>> {
        match value {
            Animated::Fixed(value) => self.value(value, path).map(Prop::Fixed),
            Animated::Motion(motion) => self.motion(motion, path).map(Prop::Varying),
        }
    }

    /// Compiles an optional property, substituting `default` when omitted.
    pub(crate) fn animated_or<T: Value>(
        &mut self,
        value: Option<&Animated<T>>,
        default: T::Engine,
        path: &str,
    ) -> Option<Prop<T::Engine>> {
        match value {
            Some(value) => self.animated(value, path),
            None => Some(Prop::Fixed(default)),
        }
    }

    pub(crate) fn easing(&mut self, easing: EasingSpec, path: &str) -> Option<Easing> {
        let easing = Easing::from(easing);
        match easing.try_validate() {
            Ok(()) => Some(easing),
            Err(error) => {
                self.error(path, "easing", error.message);
                None
            }
        }
    }

    /// Accepts a finite value in an inclusive range.
    pub(crate) fn bounded(
        &mut self,
        value: f64,
        minimum: f64,
        maximum: f64,
        path: &str,
        field: &str,
    ) -> Option<f64> {
        if value.is_finite() && (minimum..=maximum).contains(&value) {
            Some(value)
        } else {
            self.error(
                path,
                field,
                format!("Value must be finite and in {minimum}..={maximum}"),
            );
            None
        }
    }

    fn value<T: Value>(&mut self, value: &T, path: &str) -> Option<T::Engine> {
        value
            .engine()
            .map_err(|message| self.error(path, "value", message))
            .ok()
    }

    fn motion<T: Value>(
        &mut self,
        motion: &MotionSpec<T>,
        path: &str,
    ) -> Option<Arc<dyn Signal<T::Engine>>> {
        match motion {
            MotionSpec::Tween(tween) => self.tween(tween, &format!("{path}.tween")),
            MotionSpec::Keyframes(keyframes) => {
                self.keyframes(keyframes, &format!("{path}.keyframes"))
            }
            MotionSpec::Oscillate(oscillate) => {
                let path = format!("{path}.oscillate");
                let signal = self.oscillate(oscillate, &path)?;
                self.scalar::<T>(signal, &path)
            }
            MotionSpec::Noise(noise) => {
                let path = format!("{path}.noise");
                let signal = self.noise(noise, &path)?;
                self.scalar::<T>(signal, &path)
            }
            MotionSpec::Expr(spec) => self.expression_motion::<T>(spec, &format!("{path}.expr")),
        }
    }

    fn expression_motion<T: Value>(
        &mut self,
        spec: &ExprSpec,
        path: &str,
    ) -> Option<Arc<dyn Signal<T::Engine>>> {
        let sources = spec.sources();
        if T::ARITY == 0 {
            self.error(path, "expr", "Expressions apply only to numeric properties");
            return None;
        }
        if sources.len() != T::ARITY {
            self.error(
                path,
                "expr",
                format!(
                    "Expected {} expression(s), found {}",
                    T::ARITY,
                    sources.len()
                ),
            );
            return None;
        }
        let signals = sources
            .iter()
            .enumerate()
            .map(|(index, source)| {
                let path = if T::ARITY == 1 {
                    path.to_owned()
                } else {
                    format!("{path}[{index}]")
                };
                self.expression(source, &path)
            })
            .collect::<Vec<_>>();
        let signals = signals.into_iter().collect::<Option<Vec<_>>>()?;
        T::from_signals(&signals)
    }

    fn scalar<T: Value>(
        &mut self,
        signal: Arc<dyn Signal<f64>>,
        path: &str,
    ) -> Option<Arc<dyn Signal<T::Engine>>> {
        let adapted = (T::ARITY == 1)
            .then(|| T::from_signals(&[signal]))
            .flatten();
        if adapted.is_none() {
            self.error(path, "motion", "Signals apply only to numeric properties");
        }
        adapted
    }

    fn tween<T: Value>(
        &mut self,
        tween: &TweenSpec<T>,
        path: &str,
    ) -> Option<Arc<dyn Signal<T::Engine>>> {
        let from = self.value(&tween.from, &format!("{path}.from"));
        let to = self.value(&tween.to, &format!("{path}.to"));
        let duration = self.seconds(tween.duration, path, "duration");
        let easing = self.easing(tween.easing, path);
        let animation = Tween::try_new(from?, to?, duration?)
            .map_err(|error| self.error(path, "duration", error.message))
            .ok()?
            .easing(easing?);
        self.timing(
            Arc::new(animation),
            Timing {
                delay: tween.delay,
                repeat: tween.repeat,
                ping_pong: tween.ping_pong,
            },
            path,
        )
    }

    fn keyframes<T: Value>(
        &mut self,
        keyframes: &KeyframesSpec<T>,
        path: &str,
    ) -> Option<Arc<dyn Signal<T::Engine>>> {
        let Some(first) = keyframes.keys.first() else {
            self.error(path, "keys", "Keyframes require at least one key");
            return None;
        };
        let start = self.seconds(first.time, &format!("{path}.keys[0]"), "time");
        let value = self.value(&first.value, &format!("{path}.keys[0].value"));
        let mut previous = start?;
        let mut track = Keyframes::new(value?).hold(previous);
        for (index, key) in keyframes.keys.iter().enumerate().skip(1) {
            let key_path = format!("{path}.keys[{index}]");
            let time = self.seconds(key.time, &key_path, "time");
            let value = self.value(&key.value, &format!("{key_path}.value"));
            let easing = self.easing(key.easing, &key_path);
            let time = time?;
            if time <= previous {
                self.error(&key_path, "time", "Keyframe times must strictly increase");
                return None;
            }
            previous = time;
            track = track.to(time, value?, easing?);
        }
        self.timing(
            Arc::new(track),
            Timing {
                delay: keyframes.delay,
                repeat: keyframes.repeat,
                ping_pong: keyframes.ping_pong,
            },
            path,
        )
    }

    /// Applies mirror, then repeat, then delay, bounding the resulting duration.
    fn timing<Engine: Send + Sync + 'static>(
        &mut self,
        animation: Arc<dyn FiniteAnimation<Engine>>,
        timing: Timing,
        path: &str,
    ) -> Option<Arc<dyn Signal<Engine>>> {
        let mut animation = animation;
        let mut length = animation.duration().as_seconds();
        if timing.ping_pong {
            length *= 2.0;
            animation = animation.ping_pong().boxed_finite();
        }
        if let Some(count) = timing.repeat {
            length *= f64::from(count);
            if length > super::MAX_SECONDS {
                self.error(path, "repeat", "Repeated duration exceeds the time limit");
                return None;
            }
            animation = animation.repeat(count).boxed_finite();
        }
        if let Some(delay) = timing.delay {
            animation = animation
                .delay(self.seconds(delay, path, "delay")?)
                .boxed_finite();
        }
        Some(animation)
    }

    fn oscillate(&mut self, oscillate: &OscillateSpec, path: &str) -> Option<Arc<dyn Signal<f64>>> {
        let frequency = self.bounded(oscillate.frequency, 0.0, MAX_FREQUENCY, path, "frequency");
        let amplitude = self.signal(oscillate.amplitude.unwrap_or(1.0), path, "amplitude");
        let phase = self.signal(oscillate.phase.unwrap_or(0.0), path, "phase");
        let offset = self.signal(oscillate.offset.unwrap_or(0.0), path, "offset");
        let oscillator = Oscillator::builder(oscillate.waveform, frequency?)
            .amplitude(amplitude?)
            .phase(phase?)
            .offset(offset?)
            .build();
        match oscillator {
            Ok(oscillator) => Some(Arc::new(oscillator)),
            Err(error) => {
                self.error(path, "oscillate", &error.message);
                None
            }
        }
    }

    fn noise(&mut self, noise: &NoiseSpec, path: &str) -> Option<Arc<dyn Signal<f64>>> {
        let frequency = self.bounded(
            noise.frequency.unwrap_or(1.0),
            0.0,
            MAX_FREQUENCY,
            path,
            "frequency",
        );
        let octaves = self.bounded(
            f64::from(noise.octaves.unwrap_or(1)),
            1.0,
            32.0,
            path,
            "octaves",
        );
        let lacunarity = self.bounded(
            noise.lacunarity.unwrap_or(2.0),
            f64::MIN_POSITIVE,
            MAX_FREQUENCY,
            path,
            "lacunarity",
        );
        let gain = self.bounded(noise.gain.unwrap_or(0.5), 0.0, 1.0, path, "gain");
        let amplitude = self.signal(noise.amplitude.unwrap_or(1.0), path, "amplitude");
        let offset = self.signal(noise.offset.unwrap_or(0.0), path, "offset");
        let (amplitude, offset) = (amplitude?, offset?);
        let source = Noise::builder(noise.seed)
            .frequency(frequency?)
            .octaves(octaves? as u32)
            .lacunarity(lacunarity?)
            .gain(gain?)
            .build();
        match source {
            Ok(source) => Some(Arc::new(
                source.map(move |value| offset + amplitude * value),
            )),
            Err(error) => {
                self.error(path, "noise", &error.message);
                None
            }
        }
    }

    fn signal(&mut self, value: f64, path: &str, field: &str) -> Option<f64> {
        self.bounded(
            value,
            -MAX_SIGNAL_MAGNITUDE,
            MAX_SIGNAL_MAGNITUDE,
            path,
            field,
        )
    }
}

/// Shared play-count, mirror and offset modifiers.
struct Timing {
    delay: Option<f64>,
    repeat: Option<u32>,
    ping_pong: bool,
}
