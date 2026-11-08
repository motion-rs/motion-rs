//! Stateless lifetime curves, blackbody colors and position/velocity fields.

use super::*;
use motion_core::Easing;

/// Bounded nonnegative scalar keyframes with explicit normalized times and easing.
#[derive(Clone)]
pub struct ParticleCurve {
    /// Strictly increasing normalized times and finite values in 0..=65504.
    pub stops: Arc<[(f32, f32)]>,
    /// Easing between neighboring stops.
    pub easing: Easing,
}

impl ParticleCurve {
    /// Creates a two-stop linear curve; invalid values fail during evaluation.
    pub fn linear(start: f32, end: f32) -> Self {
        Self {
            stops: Arc::from([(0.0, start), (1.0, end)]),
            easing: Easing::Linear,
        }
    }

    /// Samples a validated curve, extending its endpoint values outside the stop span.
    pub fn sample(&self, progress: f32) -> Result<f32, ValidationError> {
        validate_times(self.stops.iter().map(|stop| stop.0), self.stops.len())?;
        if !progress.is_finite()
            || self
                .stops
                .iter()
                .any(|stop| !stop.1.is_finite() || !(0.0..=65504.0).contains(&stop.1))
        {
            return Err(invalid(
                "curve",
                "Curve progress/values must be finite and nonnegative",
            ));
        }
        let (first, second, amount) = interval(&self.stops, progress, &self.easing)?;
        let value = first.1 * (1.0 - amount) + second.1 * amount;
        if !value.is_finite() || !(0.0..=65504.0).contains(&value) {
            return Err(invalid("curve", "Easing produced an invalid scalar value"));
        }
        Ok(value)
    }
}

fn validate_times(times: impl Iterator<Item = f32>, count: usize) -> Result<(), ValidationError> {
    let mut previous = -1.0;
    if count == 0 || count > 256 {
        return Err(invalid("curve", "Require 1..=256 keyframes"));
    }
    for time in times {
        if !time.is_finite() || !(0.0..=1.0).contains(&time) || time <= previous {
            return Err(invalid(
                "curve",
                "Keyframe times must be finite, normalized and strictly increasing",
            ));
        }
        previous = time;
    }
    Ok(())
}

/// Bracketing stops and the eased interpolation amount.
type Interval<'a, Value> = (&'a (f32, Value), &'a (f32, Value), f32);

fn interval<'a, Value>(
    stops: &'a [(f32, Value)],
    progress: f32,
    easing: &Easing,
) -> Result<Interval<'a, Value>, ValidationError> {
    let upper = stops
        .partition_point(|stop| stop.0 <= progress)
        .min(stops.len() - 1);
    let lower = if progress >= stops[upper].0 {
        upper
    } else {
        upper.saturating_sub(1)
    };
    let amount = if lower == upper {
        0.0
    } else {
        (progress - stops[lower].0) / (stops[upper].0 - stops[lower].0)
    };
    Ok((
        &stops[lower],
        &stops[upper],
        easing.try_apply(f64::from(amount.clamp(0.0, 1.0)))? as f32,
    ))
}

/// Nonuniform premultiplied color keyframes, evaluated without accumulated state.
#[derive(Clone)]
pub struct ColorCurve {
    /// Normalized times and finite premultiplied colors.
    pub stops: Arc<[(f32, [f32; 4])]>,
    /// Interpolation easing.
    pub easing: Easing,
}

impl ColorCurve {
    /// Samples validated finite keyframes.
    pub fn sample(&self, progress: f32) -> Result<[f32; 4], ValidationError> {
        validate_times(self.stops.iter().map(|stop| stop.0), self.stops.len())?;
        if !progress.is_finite() {
            return Err(invalid("curve", "Nonfinite color progress"));
        }
        let (first, second, amount) = interval(&self.stops, progress, &self.easing)?;
        for stop in [first, second] {
            Particle {
                color: stop.1,
                ..Particle::default()
            }
            .validate()?;
        }
        let color = std::array::from_fn(|channel| {
            first.1[channel] * (1.0 - amount) + second.1[channel] * amount
        });
        Particle {
            color,
            ..Particle::default()
        }
        .validate()?;
        Ok(color)
    }
}

impl ParticleModifier for ColorCurve {
    fn apply(
        &self,
        particle: &mut Particle,
        _: f32,
        _: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        particle.color = self.sample(particle.age / particle.lifetime)?;
        particle.validate()
    }
}

impl ColorOverLifetime {
    /// Converts equally spaced stops to an explicit time/easing curve.
    pub fn curve(self, easing: Easing) -> ColorCurve {
        let denominator = self.stops.len().saturating_sub(1).max(1) as f32;
        ColorCurve {
            stops: self
                .stops
                .iter()
                .enumerate()
                .map(|(index, color)| (index as f32 / denominator, *color))
                .collect(),
            easing,
        }
    }
}

/// RGB energy multiplier; alpha is unchanged and HDR energy is not clipped.
#[derive(Clone)]
pub struct IntensityOverLifetime {
    /// Multiplier over normalized lifetime.
    pub curve: ParticleCurve,
}

impl IntensityOverLifetime {
    /// Creates an RGB energy multiplier curve.
    pub fn new(curve: ParticleCurve) -> Self {
        Self { curve }
    }
}

impl ParticleModifier for IntensityOverLifetime {
    fn apply(
        &self,
        particle: &mut Particle,
        _: f32,
        _: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        let intensity = self.curve.sample(particle.age / particle.lifetime)?;
        for channel in &mut particle.color[..3] {
            *channel *= intensity;
        }
        particle.validate()
    }

    fn resets_color(&self) -> bool {
        true
    }
}

/// Deterministic blackbody approximation, 1000..=40000 Kelvin, decoded to linear RGB.
/// As a modifier it replaces the particle's RGB with the premultiplied blackbody color,
/// discarding any earlier hue; alpha is unchanged.
#[derive(Clone)]
pub struct Temperature {
    /// Kelvin over normalized lifetime.
    pub curve: ParticleCurve,
}

impl Temperature {
    /// Creates a color-temperature curve.
    pub fn new(curve: ParticleCurve) -> Self {
        Self { curve }
    }

    /// Evaluates a bounded color temperature, with no platform color-management dependency.
    pub fn rgb(kelvin: f32) -> Result<[f32; 3], ValidationError> {
        if !kelvin.is_finite() || !(1000.0..=40000.0).contains(&kelvin) {
            return Err(invalid("temperature", "Require 1000..=40000 Kelvin"));
        }
        let temperature = f64::from(kelvin) / 100.0;
        let red = if temperature <= 66.0 {
            255.0
        } else {
            329.698727446 * libm::pow(temperature - 60.0, -0.1332047592)
        };
        let green = if temperature <= 66.0 {
            99.4708025861 * libm::log(temperature) - 161.1195681661
        } else {
            288.1221695283 * libm::pow(temperature - 60.0, -0.0755148492)
        };
        let blue = if temperature >= 66.0 {
            255.0
        } else if temperature <= 19.0 {
            0.0
        } else {
            138.5177312231 * libm::log(temperature - 10.0) - 305.0447927307
        };
        Ok([red, green, blue]
            .map(|value| motion_core::srgb_to_linear((value / 255.0).clamp(0.0, 1.0) as f32)))
    }
}

impl ParticleModifier for Temperature {
    fn apply(
        &self,
        particle: &mut Particle,
        _: f32,
        _: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        let rgb = Self::rgb(self.curve.sample(particle.age / particle.lifetime)?)?;
        for (channel, value) in rgb.into_iter().enumerate() {
            particle.color[channel] = value * particle.color[3];
        }
        particle.validate()
    }
}

/// Scalar coordinate used by a color field.
#[derive(Clone, Copy, Debug)]
pub enum FieldCoordinate {
    /// Horizontal position.
    X,
    /// Vertical position.
    Y,
    /// Velocity magnitude.
    Speed,
}

/// Color sampled from position or speed, with an explicit normalization interval.
#[derive(Clone)]
pub struct ColorField {
    /// Source coordinate.
    pub coordinate: FieldCoordinate,
    /// Strictly increasing finite normalization interval.
    pub range: [f32; 2],
    /// Output color curve.
    pub colors: ColorCurve,
}

impl ParticleModifier for ColorField {
    fn apply(
        &self,
        particle: &mut Particle,
        _: f32,
        _: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        if self.range.iter().any(|value| !value.is_finite()) || self.range[0] >= self.range[1] {
            return Err(invalid("field", "Invalid coordinate range"));
        }
        let value = match self.coordinate {
            FieldCoordinate::X => particle.position[0],
            FieldCoordinate::Y => particle.position[1],
            FieldCoordinate::Speed => libm::hypotf(particle.velocity[0], particle.velocity[1]),
        };
        particle.color = self
            .colors
            .sample(((value - self.range[0]) / (self.range[1] - self.range[0])).clamp(0.0, 1.0))?;
        particle.validate()
    }
}
