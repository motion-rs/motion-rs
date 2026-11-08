//! Clock-free analytic oscillators, seeded noise, path motion and mappings.

use super::{Animation, Easing, Interpolate};
use crate::{Duration, Path2D, Point, Time, Vector2};
use std::sync::Arc;

/// Periodic waveform evaluated in turns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Waveform {
    /// Smooth sine starting at zero.
    #[default]
    Sine,
    /// Linear ramps starting at zero and rising to one.
    Triangle,
    /// Positive during the first half of each cycle, negative during the second.
    Square,
    /// Linear ramp from minus one to one with a discontinuity each cycle.
    Sawtooth,
}

/// Infinite stateless periodic scalar animation.
#[derive(Clone, Debug)]
pub struct Oscillator {
    waveform: Waveform,
    frequency: f64,
    amplitude: f64,
    phase: f64,
    offset: f64,
}

impl Oscillator {
    /// Creates a waveform with finite nonnegative frequency in hertz.
    pub fn new(waveform: Waveform, frequency_hertz: f64) -> Self {
        assert!(
            frequency_hertz.is_finite() && frequency_hertz >= 0.0,
            "Frequency must be finite and nonnegative"
        );
        Self {
            waveform,
            frequency: frequency_hertz,
            amplitude: 1.0,
            phase: 0.0,
            offset: 0.0,
        }
    }

    /// Sets finite peak amplitude.
    pub fn amplitude(mut self, amplitude: f64) -> Self {
        assert!(amplitude.is_finite());
        self.amplitude = amplitude;
        self
    }

    /// Sets a finite cycle offset in turns.
    pub fn phase(mut self, turns: f64) -> Self {
        assert!(turns.is_finite());
        self.phase = turns;
        self
    }

    /// Sets a finite output offset.
    pub fn offset(mut self, offset: f64) -> Self {
        assert!(offset.is_finite());
        self.offset = offset;
        self
    }
}

impl Animation<f64> for Oscillator {
    fn sample(&self, time: Time) -> f64 {
        let cycles = time.as_seconds() * self.frequency + self.phase;
        assert!(cycles.is_finite(), "Oscillator phase overflow");
        let turns = cycles - libm::floor(cycles);
        let value = match self.waveform {
            Waveform::Sine => libm::sin(turns * std::f64::consts::TAU),
            Waveform::Triangle if turns < 0.25 => turns * 4.0,
            Waveform::Triangle if turns < 0.75 => 2.0 - turns * 4.0,
            Waveform::Triangle => turns * 4.0 - 4.0,
            Waveform::Square => {
                if turns < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Waveform::Sawtooth => turns * 2.0 - 1.0,
        };
        let output = value * self.amplitude + self.offset;
        assert!(output.is_finite(), "Oscillator output overflow");
        output
    }
}

/// One-dimensional slice of [`crate::noise::value2_d`] along `y = 0`, with its derivative.
pub fn value_noise(position: f64, seed: u32) -> (f64, f64) {
    assert!(position.is_finite(), "Noise position must be finite");
    let (value, gradient) = crate::noise::value2_d([position, 0.0], seed);
    (value, gradient[0])
}

/// Infinite deterministic multi-octave noise animation in minus one to one.
#[derive(Clone, Debug)]
pub struct Noise {
    seed: u64,
    frequency: f64,
    octaves: u32,
    lacunarity: f64,
    gain: f64,
}

impl Noise {
    /// Creates seeded one-octave noise with one lattice cell per second.
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            frequency: 1.0,
            octaves: 1,
            lacunarity: 2.0,
            gain: 0.5,
        }
    }

    /// Sets finite nonnegative lattice frequency.
    pub fn frequency(mut self, hertz: f64) -> Self {
        assert!(hertz.is_finite() && hertz >= 0.0);
        self.frequency = hertz;
        self
    }

    /// Sets the bounded octave count, from one through 32.
    pub fn octaves(mut self, octaves: u32) -> Self {
        assert!((1..=32).contains(&octaves));
        self.octaves = octaves;
        self
    }

    /// Sets finite positive octave frequency multiplication.
    pub fn lacunarity(mut self, lacunarity: f64) -> Self {
        assert!(lacunarity.is_finite() && lacunarity > 0.0);
        self.lacunarity = lacunarity;
        self
    }

    /// Sets finite octave amplitude multiplication in zero to one.
    pub fn gain(mut self, gain: f64) -> Self {
        assert!(gain.is_finite() && (0.0..=1.0).contains(&gain));
        self.gain = gain;
        self
    }

    /// Creates independently seeded X/Y noise with the same spectral settings.
    pub fn vector(self) -> impl Animation<Vector2> {
        let mut vertical = self.clone();
        vertical.seed ^= 0x9e3779b97f4a7c15;
        move |time| Vector2::new(self.sample(time), vertical.sample(time))
    }
}

impl Animation<f64> for Noise {
    fn sample(&self, time: Time) -> f64 {
        let mut position = time.as_seconds() * self.frequency;
        let mut amplitude = 1.0;
        let mut value = 0.0;
        let mut weight = 0.0;
        for octave in 0..self.octaves {
            let seed = (self.seed ^ (self.seed >> 32)) as u32;
            value += value_noise(position, seed.wrapping_add(octave.wrapping_mul(0x9e3779b9))).0
                * amplitude;
            weight += amplitude;
            position *= self.lacunarity;
            amplitude *= self.gain;
        }
        value / weight
    }
}

/// One arc-length path sample in local geometry coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathSample {
    /// Position along the path.
    pub position: Point,
    /// Normalized local tangent, or zero for degenerate geometry.
    pub tangent: Vector2,
    /// Tangent angle in radians, zero for degenerate geometry.
    pub angle: f64,
}

/// Finite path motion sampled by normalized arc length.
#[derive(Clone, Debug)]
pub struct FollowPath {
    path: Arc<Path2D>,
    duration: Duration,
    easing: Easing,
}

impl FollowPath {
    /// Creates path motion with a nonnegative duration; zero jumps to the end.
    pub fn new(path: impl Into<Arc<Path2D>>, duration: Duration) -> Self {
        assert!(duration >= Time::ZERO);
        Self {
            path: path.into(),
            duration,
            easing: Easing::Linear,
        }
    }

    /// Sets the normalized distance easing.
    pub fn easing(mut self, easing: Easing) -> Self {
        easing.try_validate().expect("Valid path easing");
        self.easing = easing;
        self
    }
}

impl Animation<PathSample> for FollowPath {
    fn sample(&self, time: Time) -> PathSample {
        let progress = if self.duration == Time::ZERO {
            if time < Time::ZERO { 0.0 } else { 1.0 }
        } else {
            (time / self.duration).clamp(0.0, 1.0)
        };
        let (position, tangent) = self
            .path
            .point_at(self.easing.apply(progress).clamp(0.0, 1.0));
        PathSample {
            position,
            tangent,
            angle: libm::atan2(tangent.y, tangent.x),
        }
    }

    fn duration(&self) -> Option<Duration> {
        Some(self.duration)
    }
}

/// Behavior outside a mapping's input interval.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Extrapolate {
    /// Keep the nearest endpoint.
    #[default]
    Clamp,
    /// Continue the endpoint segment linearly.
    Extend,
    /// Repeat the half-open input interval.
    Wrap,
}

fn progress(value: f64, from: [f64; 2], extrapolate: Extrapolate) -> f64 {
    assert!(
        value.is_finite() && from.iter().all(|value| value.is_finite()) && from[0] != from[1],
        "Mapping inputs must be finite with distinct endpoints"
    );
    let scale = value
        .abs()
        .max(from[0].abs())
        .max(from[1].abs())
        .max(f64::MIN_POSITIVE);
    let span = from[1] - from[0];
    let distance = value - from[0];
    let amount = if span.is_finite() && distance.is_finite() {
        distance / span
    } else {
        (value / scale - from[0] / scale) / (from[1] / scale - from[0] / scale)
    };
    match extrapolate {
        Extrapolate::Clamp => amount.clamp(0.0, 1.0),
        Extrapolate::Extend => amount,
        Extrapolate::Wrap => amount - libm::floor(amount),
    }
}

/// Maps one finite scalar interval; reversed endpoints are supported.
pub fn remap(value: f64, from: [f64; 2], to: [f64; 2], extrapolate: Extrapolate) -> f64 {
    assert!(to.iter().all(|value| value.is_finite()));
    let output = to[0].interpolate(&to[1], progress(value, from, extrapolate));
    assert!(output.is_finite(), "Mapping output overflow");
    output
}

/// Interpolates matching samples with finite strictly increasing input knots.
pub fn piecewise<Value: Interpolate>(
    value: f64,
    inputs: &[f64],
    outputs: &[Value],
    easing: Easing,
    extrapolate: Extrapolate,
) -> Value {
    assert!(inputs.len() >= 2 && inputs.len() == outputs.len());
    assert!(
        inputs.iter().all(|value| value.is_finite())
            && inputs.windows(2).all(|pair| pair[0] < pair[1])
    );
    let mapped = remap(
        value,
        [inputs[0], inputs[inputs.len() - 1]],
        [inputs[0], inputs[inputs.len() - 1]],
        extrapolate,
    );
    let index = inputs
        .partition_point(|input| *input <= mapped)
        .saturating_sub(1)
        .min(inputs.len() - 2);
    let amount = progress(
        mapped,
        [inputs[index], inputs[index + 1]],
        Extrapolate::Extend,
    );
    let amount = if !(0.0..=1.0).contains(&amount) && extrapolate == Extrapolate::Extend {
        amount
    } else {
        easing.apply(amount)
    };
    outputs[index].interpolate(&outputs[index + 1], amount)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Shape, TimeLiteral};

    #[test]
    fn oscillators_are_periodic_seek_independent_and_infinite() {
        for waveform in [
            Waveform::Sine,
            Waveform::Triangle,
            Waveform::Square,
            Waveform::Sawtooth,
        ] {
            let value = Oscillator::new(waveform, 2.0).amplitude(3.0).offset(7.0);
            assert_eq!(value.duration(), None);
            assert_eq!(
                value.sample(125.milliseconds()),
                value.sample(625.milliseconds())
            );
            assert_eq!(
                value.sample((-375).milliseconds()),
                value.sample(125.milliseconds())
            );
        }
        assert_eq!(
            Oscillator::new(Waveform::Triangle, 1.0).sample(250.milliseconds()),
            1.0
        );
    }

    #[test]
    fn noise_and_vector_replay_share_the_particle_noise_primitive() {
        let noise = Noise::new(17);
        for time in [
            500.milliseconds(),
            (-200).milliseconds(),
            3.seconds(),
            500.milliseconds(),
        ] {
            assert_eq!(noise.sample(time), value_noise(time.as_seconds(), 17).0);
            assert!((-1.0..=1.0).contains(&noise.clone().octaves(5).sample(time)));
        }
        let vector = noise.vector();
        assert_eq!(
            vector.sample(500.milliseconds()),
            vector.sample(500.milliseconds())
        );
        assert_ne!(
            vector.sample(500.milliseconds()).x,
            vector.sample(500.milliseconds()).y
        );
    }

    #[test]
    fn path_motion_uses_arc_length_and_has_exact_endpoints() {
        let path = crate::Line::new(Point::ORIGIN, Point::new(10.0, 0.0)).path();
        let motion = FollowPath::new(path, 2.seconds());
        assert_eq!(motion.duration(), Some(2.seconds()));
        assert_eq!(motion.sample(1.seconds()).position, Point::new(5.0, 0.0));
        assert_eq!(motion.sample((-1).seconds()).position, Point::ORIGIN);
        assert_eq!(motion.sample(3.seconds()).position, Point::new(10.0, 0.0));
        assert_eq!(motion.sample(1.seconds()).angle, 0.0);
    }

    #[test]
    fn mappings_cover_clamp_wrap_extend_reversed_ranges_and_invalid_knots() {
        assert_eq!(
            remap(3.0, [0.0, 2.0], [0.0, 10.0], Extrapolate::Extend),
            15.0
        );
        assert_eq!(remap(-1.0, [0.0, 2.0], [0.0, 10.0], Extrapolate::Wrap), 5.0);
        assert_eq!(remap(3.0, [2.0, 0.0], [0.0, 10.0], Extrapolate::Clamp), 0.0);
        assert_eq!(
            piecewise(
                1.5,
                &[0.0, 1.0, 2.0],
                &[0.0, 10.0, 30.0],
                Easing::Linear,
                Extrapolate::Clamp
            ),
            20.0
        );
        assert_eq!(
            piecewise(
                3.0,
                &[0.0, 1.0, 2.0],
                &[0.0, 10.0, 30.0],
                Easing::Linear,
                Extrapolate::Extend
            ),
            50.0
        );
        assert!(
            std::panic::catch_unwind(|| piecewise(
                0.0,
                &[0.0, 0.0],
                &[0.0, 1.0],
                Easing::Linear,
                Extrapolate::Clamp
            ))
            .is_err()
        );
    }
}
