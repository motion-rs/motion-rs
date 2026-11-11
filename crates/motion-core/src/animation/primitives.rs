//! Clock-free analytic oscillators, seeded noise, path motion and mappings.

use super::{Easing, FiniteAnimation, Interpolate, Signal};
use crate::{Duration, Path2D, Point, ValidationError, Vector2};
use std::sync::Arc;

/// Upper bound for oscillator frequency and phase so cycle counts stay finite.
const MAX_MAGNITUDE: f64 = 1.0e290;

/// Rounded-up bound on the seconds a signed tick count can express.
const MAX_SECONDS: f64 = 2.0e10;

/// Periodic waveform evaluated in turns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
    /// Starts a builder for a waveform with the given frequency in hertz.
    pub fn builder(waveform: Waveform, frequency_hertz: f64) -> OscillatorBuilder {
        OscillatorBuilder {
            waveform,
            frequency: frequency_hertz,
            amplitude: 1.0,
            phase: 0.0,
            offset: 0.0,
        }
    }
}

/// Unvalidated oscillator configuration; [`OscillatorBuilder::build`] reports invalid values.
#[derive(Clone, Debug)]
pub struct OscillatorBuilder {
    waveform: Waveform,
    frequency: f64,
    amplitude: f64,
    phase: f64,
    offset: f64,
}

impl OscillatorBuilder {
    /// Sets peak amplitude, which must be finite.
    pub fn amplitude(mut self, amplitude: f64) -> Self {
        self.amplitude = amplitude;
        self
    }

    /// Sets a cycle offset in turns, which must be finite and at most 1e290.
    pub fn phase(mut self, turns: f64) -> Self {
        self.phase = turns;
        self
    }

    /// Sets an output offset, which must be finite.
    pub fn offset(mut self, offset: f64) -> Self {
        self.offset = offset;
        self
    }

    /// Validates the configuration. Frequency must be finite, nonnegative and at
    /// most 1e290; the output range `amplitude * ±1 + offset` must
    /// be finite, so sampling can never overflow.
    pub fn build(self) -> Result<Oscillator, ValidationError> {
        let error = |field, message| ValidationError::invalid("oscillator", field, message);
        if !(self.frequency.is_finite() && (0.0..=MAX_MAGNITUDE).contains(&self.frequency)) {
            return Err(error(
                "frequency",
                "frequency must be finite, nonnegative and bounded",
            ));
        }
        if !(self.phase.is_finite() && self.phase.abs() <= MAX_MAGNITUDE) {
            return Err(error("phase", "phase must be finite and bounded"));
        }
        if !(self.amplitude.is_finite()
            && self.offset.is_finite()
            && (self.amplitude.abs() + self.offset.abs()).is_finite())
        {
            return Err(error(
                "amplitude",
                "amplitude and offset must be finite and keep the output finite",
            ));
        }
        Ok(Oscillator {
            waveform: self.waveform,
            frequency: self.frequency,
            amplitude: self.amplitude,
            phase: self.phase,
            offset: self.offset,
        })
    }
}

impl Signal<f64> for Oscillator {
    fn sample(&self, time: Duration) -> f64 {
        let cycles = time.as_seconds() * self.frequency + self.phase;
        debug_assert!(
            cycles.is_finite(),
            "bounded parameters keep the phase finite"
        );
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
        debug_assert!(
            output.is_finite(),
            "validated range keeps the output finite"
        );
        output
    }
}

/// One-dimensional slice of [`crate::noise::value2_d`] along `y = 0`, with its derivative.
///
/// # Panics
/// Panics for a nonfinite position; see [`try_value_noise`].
pub fn value_noise(position: f64, seed: u32) -> (f64, f64) {
    try_value_noise(position, seed).unwrap_or_else(|error| panic!("{}", error.message))
}

/// Evaluates [`value_noise`], reporting a nonfinite position.
pub fn try_value_noise(position: f64, seed: u32) -> Result<(f64, f64), ValidationError> {
    if !position.is_finite() {
        return Err(ValidationError::invalid(
            "noise",
            "position",
            "noise position must be finite",
        ));
    }
    let (value, gradient) = crate::noise::value2_d([position, 0.0], seed);
    Ok((value, gradient[0]))
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
        Self::builder(seed)
            .build()
            .unwrap_or_else(|error| unreachable!("default noise parameters are valid: {error}"))
    }

    /// Starts a builder with one octave and one lattice cell per second.
    pub fn builder(seed: u64) -> NoiseBuilder {
        NoiseBuilder {
            seed,
            frequency: 1.0,
            octaves: 1,
            lacunarity: 2.0,
            gain: 0.5,
        }
    }

    /// Creates independently seeded X/Y noise with the same spectral settings.
    pub fn vector(self) -> impl Signal<Vector2> {
        let mut vertical = self.clone();
        vertical.seed ^= 0x9e3779b97f4a7c15;
        move |time| Vector2::new(self.sample(time), vertical.sample(time))
    }
}

/// Unvalidated noise configuration; [`NoiseBuilder::build`] reports invalid values.
#[derive(Clone, Debug)]
pub struct NoiseBuilder {
    seed: u64,
    frequency: f64,
    octaves: u32,
    lacunarity: f64,
    gain: f64,
}

impl NoiseBuilder {
    /// Sets lattice frequency, which must be finite and nonnegative.
    pub fn frequency(mut self, hertz: f64) -> Self {
        self.frequency = hertz;
        self
    }

    /// Sets the octave count, from one through 32.
    pub fn octaves(mut self, octaves: u32) -> Self {
        self.octaves = octaves;
        self
    }

    /// Sets octave frequency multiplication, which must be finite and positive.
    pub fn lacunarity(mut self, lacunarity: f64) -> Self {
        self.lacunarity = lacunarity;
        self
    }

    /// Sets octave amplitude multiplication in zero to one.
    pub fn gain(mut self, gain: f64) -> Self {
        self.gain = gain;
        self
    }

    /// Validates every setting at once. Beyond the individual ranges, the
    /// highest octave position over the full time range must stay finite, so
    /// sampling can never overflow.
    pub fn build(self) -> Result<Noise, ValidationError> {
        let error = |field, message| ValidationError::invalid("noise", field, message);
        if !(self.frequency.is_finite() && self.frequency >= 0.0) {
            return Err(error(
                "frequency",
                "frequency must be finite and nonnegative",
            ));
        }
        if !(1..=32).contains(&self.octaves) {
            return Err(error("octaves", "octaves must be from 1 through 32"));
        }
        if !(self.lacunarity.is_finite() && self.lacunarity > 0.0) {
            return Err(error(
                "lacunarity",
                "lacunarity must be finite and positive",
            ));
        }
        if !(self.gain.is_finite() && (0.0..=1.0).contains(&self.gain)) {
            return Err(error("gain", "gain must be finite and in 0..=1"));
        }
        let reach = MAX_SECONDS * self.frequency * self.lacunarity.powi(self.octaves as i32 - 1);
        if !reach.is_finite() {
            return Err(error(
                "frequency",
                "frequency and lacunarity must keep octave positions finite",
            ));
        }
        Ok(Noise {
            seed: self.seed,
            frequency: self.frequency,
            octaves: self.octaves,
            lacunarity: self.lacunarity,
            gain: self.gain,
        })
    }
}

impl Signal<f64> for Noise {
    fn sample(&self, time: Duration) -> f64 {
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
        assert!(duration >= Duration::ZERO);
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

impl Signal<PathSample> for FollowPath {
    fn sample(&self, time: Duration) -> PathSample {
        let progress = if self.duration == Duration::ZERO {
            if time < Duration::ZERO { 0.0 } else { 1.0 }
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
}

impl FiniteAnimation<PathSample> for FollowPath {
    fn duration(&self) -> Duration {
        self.duration
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

fn mapping_error(field: &'static str, message: &'static str) -> ValidationError {
    ValidationError::invalid("mapping", field, message)
}

fn progress(value: f64, from: [f64; 2], extrapolate: Extrapolate) -> Result<f64, ValidationError> {
    if !(value.is_finite() && from.iter().all(|value| value.is_finite()) && from[0] != from[1]) {
        return Err(mapping_error(
            "from",
            "mapping inputs must be finite with distinct endpoints",
        ));
    }
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
    Ok(match extrapolate {
        Extrapolate::Clamp => amount.clamp(0.0, 1.0),
        Extrapolate::Extend => amount,
        Extrapolate::Wrap => amount - libm::floor(amount),
    })
}

/// Maps one finite scalar interval; reversed endpoints are supported.
///
/// # Panics
/// Panics for invalid inputs or an overflowing output; see [`try_remap`].
pub fn remap(value: f64, from: [f64; 2], to: [f64; 2], extrapolate: Extrapolate) -> f64 {
    try_remap(value, from, to, extrapolate).unwrap_or_else(|error| panic!("{}", error.message))
}

/// Maps like [`remap`], reporting nonfinite inputs, equal input endpoints and overflow.
pub fn try_remap(
    value: f64,
    from: [f64; 2],
    to: [f64; 2],
    extrapolate: Extrapolate,
) -> Result<f64, ValidationError> {
    if !to.iter().all(|value| value.is_finite()) {
        return Err(mapping_error("to", "mapping outputs must be finite"));
    }
    let output = to[0].interpolate(&to[1], progress(value, from, extrapolate)?);
    if !output.is_finite() {
        return Err(mapping_error("output", "mapping output overflows"));
    }
    Ok(output)
}

/// Interpolates matching samples with finite strictly increasing input knots.
///
/// # Panics
/// Panics for invalid knots or an invalid mapping; see [`try_piecewise`].
pub fn piecewise<Value: Interpolate>(
    value: f64,
    inputs: &[f64],
    outputs: &[Value],
    easing: Easing,
    extrapolate: Extrapolate,
) -> Value {
    try_piecewise(value, inputs, outputs, easing, extrapolate)
        .unwrap_or_else(|error| panic!("{}", error.message))
}

/// Interpolates like [`piecewise`], reporting invalid knots, easing and mapping errors.
pub fn try_piecewise<Value: Interpolate>(
    value: f64,
    inputs: &[f64],
    outputs: &[Value],
    easing: Easing,
    extrapolate: Extrapolate,
) -> Result<Value, ValidationError> {
    if inputs.len() < 2 || inputs.len() != outputs.len() {
        return Err(mapping_error(
            "inputs",
            "piecewise needs at least two knots and one output per knot",
        ));
    }
    if !(inputs.iter().all(|value| value.is_finite())
        && inputs.windows(2).all(|pair| pair[0] < pair[1]))
    {
        return Err(mapping_error(
            "inputs",
            "piecewise knots must be finite and strictly increasing",
        ));
    }
    let mapped = try_remap(
        value,
        [inputs[0], inputs[inputs.len() - 1]],
        [inputs[0], inputs[inputs.len() - 1]],
        extrapolate,
    )?;
    let index = inputs
        .partition_point(|input| *input <= mapped)
        .saturating_sub(1)
        .min(inputs.len() - 2);
    let amount = progress(
        mapped,
        [inputs[index], inputs[index + 1]],
        Extrapolate::Extend,
    )?;
    let amount = if !(0.0..=1.0).contains(&amount) && extrapolate == Extrapolate::Extend {
        amount
    } else {
        easing.try_apply(amount)?
    };
    Ok(outputs[index].interpolate(&outputs[index + 1], amount))
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
            let value = Oscillator::builder(waveform, 2.0)
                .amplitude(3.0)
                .offset(7.0)
                .build()
                .unwrap();
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
            Oscillator::builder(Waveform::Triangle, 1.0)
                .build()
                .unwrap()
                .sample(250.milliseconds()),
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
            assert!(
                (-1.0..=1.0).contains(&Noise::builder(17).octaves(5).build().unwrap().sample(time))
            );
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
        let path = crate::Line::new(Point::ORIGIN, Point::new(10.0, 0.0))
            .unwrap()
            .path()
            .unwrap();
        let motion = FollowPath::new(path, 2.seconds());
        assert_eq!(motion.duration(), 2.seconds());
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
            try_piecewise(
                0.0,
                &[0.0, 0.0],
                &[0.0, 1.0],
                Easing::Linear,
                Extrapolate::Clamp
            )
            .is_err()
        );
        assert!(try_remap(0.0, [1.0, 1.0], [0.0, 1.0], Extrapolate::Clamp).is_err());
        assert!(
            try_remap(
                f64::MAX,
                [0.0, 1.0],
                [-f64::MAX, f64::MAX],
                Extrapolate::Extend
            )
            .is_err()
        );
    }

    #[test]
    fn builders_report_invalid_and_overflowing_parameters() {
        let oscillator = |frequency| Oscillator::builder(Waveform::Sine, frequency);
        assert!(oscillator(f64::NAN).build().is_err());
        assert!(oscillator(-1.0).build().is_err());
        assert!(oscillator(f64::MAX).build().is_err());
        assert!(oscillator(1.0).phase(f64::INFINITY).build().is_err());
        assert!(
            oscillator(1.0)
                .amplitude(f64::MAX)
                .offset(f64::MAX)
                .build()
                .is_err()
        );
        let extreme = oscillator(1.0e290).build().unwrap();
        assert!(extreme.sample(Duration::from_ticks(i64::MAX)).is_finite());
        let noise = Noise::builder(1);
        assert!(noise.clone().frequency(-1.0).build().is_err());
        assert!(noise.clone().octaves(0).build().is_err());
        assert!(noise.clone().octaves(33).build().is_err());
        assert!(noise.clone().lacunarity(0.0).build().is_err());
        assert!(noise.clone().gain(1.5).build().is_err());
        assert!(noise.clone().frequency(1.0e300).build().is_err());
        assert!(noise.octaves(32).lacunarity(1.0e10).build().is_err());
        assert!(try_value_noise(f64::NAN, 0).is_err());
    }
}
