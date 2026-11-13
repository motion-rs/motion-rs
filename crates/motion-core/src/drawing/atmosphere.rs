//! Explicit seeded atmospheric paint recipes and shared envelope mathematics.

use crate::{
    Color, PixelSize, Point, Time, ValidationError,
    noise::{self, Fractal},
};

#[cfg(test)]
mod tests;

/// Maximum canvas axis in logical atmospheric pixels.
const MAX_CANVAS_AXIS: u32 = 16384;

/// Minimum normalized radius/extent whose squared shader envelope remains finite.
const MIN_SHADER_ENVELOPE: f64 = 1e-15;

/// Conservative scalar work units per Perlin fBm octave.
const OCTAVE_WORK: u64 = 192;

/// Conservative scalar work units for envelope, dust neighbors and color conversion.
const ENVELOPE_WORK: u64 = 256;

/// Spatial envelope applied to amplitude-normalized Perlin fractal noise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AtmosphereKind {
    /// Top-to-bottom density multipliers, each in `0..=8`.
    Fog {
        /// Density at the top and bottom of the canvas.
        gradient: [f64; 2],
    },
    /// Sparse soft discs with one possible mote per noise cell.
    Dust {
        /// Positive radius in canvas pixels, at most half the cell scale.
        radius: f64,
        /// Probability of a mote per cell in `0..=1`.
        coverage: f64,
    },
    /// Elliptical edge-light envelope.
    LightLeak {
        /// Normalized center coordinates in `-1..=2`.
        center: [f64; 2],
        /// Positive normalized extents, at most four.
        extent: [f64; 2],
    },
}

/// Editable unvalidated atmospheric parameters; paint construction validates all fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtmosphereParameters {
    /// Envelope and effect-specific parameters.
    pub kind: AtmosphereKind,
    /// Explicit noise and mote seed.
    pub seed: u32,
    /// Straight encoded-sRGB tint.
    pub color: Color,
    /// Alpha/density multiplier in `0..=1`.
    pub opacity: f64,
    /// Straight-linear RGB multiplier in `0..=64`.
    pub intensity: f64,
    /// Noise wavelength or dust-cell spacing in pixels, in `0.25..=1000000`.
    pub scale: f64,
    /// Pattern velocity in canvas pixels per second.
    pub drift: [f64; 2],
    /// Amplitude-normalized fractal parameters, with at most eight octaves.
    pub fractal: Fractal,
}

impl AtmosphereParameters {
    /// Rejects nonfinite, unsupported or out-of-range recipe parameters.
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.fractal.validate()?;
        let bounded = |value: f64, min, max| value.is_finite() && (min..=max).contains(&value);
        let valid_kind = match self.kind {
            AtmosphereKind::Fog { gradient } => {
                gradient.into_iter().all(|value| bounded(value, 0.0, 8.0))
            }
            AtmosphereKind::Dust { radius, coverage } => {
                radius.is_finite()
                    && radius > 0.0
                    && radius <= self.scale * 0.5
                    && bounded(coverage, 0.0, 1.0)
            }
            AtmosphereKind::LightLeak { center, extent } => {
                center.into_iter().all(|value| bounded(value, -1.0, 2.0))
                    && extent
                        .into_iter()
                        .all(|value| value.is_finite() && value > 0.0 && value <= 4.0)
            }
        };
        if !valid_kind
            || !bounded(self.opacity, 0.0, 1.0)
            || !bounded(self.intensity, 0.0, 64.0)
            || !bounded(self.scale, 0.25, 1e6)
            || self.drift.into_iter().any(|value| !value.is_finite())
            || self.color.to_linear_premultiplied().iter().any(|value| {
                !(*value * self.intensity).is_finite()
                    || !((*value * self.intensity) as f32).is_finite()
            })
        {
            return Err(ValidationError::invalid(
                "atmosphere",
                "parameters",
                "Require finite bounded envelope, color, opacity, intensity, scale and drift",
            ));
        }
        Ok(())
    }

    /// Evaluates straight linear RGBA at an explicit canvas pixel coordinate.
    ///
    /// The caller supplies canvas dimensions and advection phase; image generators
    /// and independent paint backends use these same mathematical definitions.
    pub fn sample(
        &self,
        pixel: [f64; 2],
        canvas: PixelSize,
        phase: [f64; 2],
    ) -> Result<[f64; 4], ValidationError> {
        self.validate()?;
        if canvas.width == 0
            || canvas.height == 0
            || pixel
                .into_iter()
                .chain(phase)
                .any(|value| !value.is_finite())
        {
            return Err(ValidationError::invalid(
                "atmosphere",
                "point",
                "Atmosphere coordinates must be finite with a positive canvas",
            ));
        }
        let point = [
            (pixel[0] + phase[0]) / self.scale,
            (pixel[1] + phase[1]) / self.scale,
        ];
        let cloud = (0.5
            + 0.5
                * noise::fbm(
                    |point| noise::perlin2(point, self.seed),
                    point,
                    self.fractal,
                )?)
        .clamp(0.0, 1.0);
        let envelope = match self.kind {
            AtmosphereKind::Fog { gradient } => {
                let vertical = pixel[1] / f64::from(canvas.height);
                let density = gradient[0] * (1.0 - vertical) + gradient[1] * vertical;
                1.0 - libm::exp(-self.opacity * density * (0.35 + cloud))
            }
            AtmosphereKind::Dust { radius, coverage } => {
                self.opacity
                    * dust(point, self.seed, radius / self.scale, coverage)
                    * (0.7 + 0.3 * cloud)
            }
            AtmosphereKind::LightLeak { center, extent } => {
                let horizontal = (pixel[0] / f64::from(canvas.width) - center[0]) / extent[0];
                let vertical = (pixel[1] / f64::from(canvas.height) - center[1]) / extent[1];
                self.opacity
                    * libm::exp(-2.0 * (horizontal * horizontal + vertical * vertical))
                    * (0.6 + 0.4 * cloud)
            }
        };
        let tint = self.color.to_linear_premultiplied();
        let alpha = tint[3];
        let rgb = [tint[0], tint[1], tint[2]].map(|value| {
            if alpha > 0.0 {
                value / alpha * self.intensity
            } else {
                0.0
            }
        });
        Ok([rgb[0], rgb[1], rgb[2], (envelope * alpha).clamp(0.0, 1.0)])
    }
}

/// Immutable, admitted atmospheric paint with explicit time and canvas coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtmospherePaint {
    parameters: AtmosphereParameters,
    canvas: PixelSize,
    time: Time,
}

impl AtmospherePaint {
    /// Validates a recipe and its full canvas at explicit time without allocating pixels.
    pub fn new(
        parameters: AtmosphereParameters,
        canvas: PixelSize,
        time: Time,
    ) -> Result<Self, ValidationError> {
        let paint = Self {
            parameters,
            canvas,
            time,
        };
        paint.validate()?;
        Ok(paint)
    }

    /// Validated immutable recipe.
    pub fn parameters(&self) -> &AtmosphereParameters {
        &self.parameters
    }

    /// Canvas used for pixel wavelengths and normalized envelopes.
    pub fn canvas(&self) -> PixelSize {
        self.canvas
    }

    /// Explicit sample time; no playback history is retained.
    pub fn time(&self) -> Time {
        self.time
    }

    /// Advection offset in canvas pixels.
    pub fn phase(&self) -> [f64; 2] {
        self.parameters
            .drift
            .map(|velocity| -velocity * self.time.as_seconds())
    }

    /// Validates parameters and every octave's full-canvas lattice support, even when invisible.
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.parameters.validate()?;
        let supported_envelope = match self.parameters.kind {
            AtmosphereKind::Fog { .. } => true,
            AtmosphereKind::Dust { radius, .. } => {
                radius / self.parameters.scale >= MIN_SHADER_ENVELOPE
            }
            AtmosphereKind::LightLeak { extent, .. } => {
                extent.into_iter().all(|value| value >= MIN_SHADER_ENVELOPE)
            }
        };
        if !supported_envelope {
            return Err(ValidationError::invalid(
                "atmosphere",
                "envelope",
                "Atmosphere envelope is too small for finite shader arithmetic",
            ));
        }
        if self.canvas.width == 0
            || self.canvas.height == 0
            || self.canvas.width > MAX_CANVAS_AXIS
            || self.canvas.height > MAX_CANVAS_AXIS
        {
            return Err(ValidationError::invalid(
                "atmosphere",
                "canvas",
                "Canvas axes must be in 1..=16384",
            ));
        }
        let phase = self.phase();
        for horizontal in [0.0, f64::from(self.canvas.width)] {
            for vertical in [0.0, f64::from(self.canvas.height)] {
                let point = [
                    (horizontal + phase[0]) / self.parameters.scale,
                    (vertical + phase[1]) / self.parameters.scale,
                ];
                noise::fbm(|_| 0.0, point, self.parameters.fractal)?;
                let mut narrowed = [
                    (horizontal as f32 + phase[0] as f32) / self.parameters.scale as f32,
                    (vertical as f32 + phase[1] as f32) / self.parameters.scale as f32,
                ];
                for _ in 0..self.parameters.fractal.octaves {
                    if narrowed.iter().any(|value| {
                        !value.is_finite() || *value < i32::MIN as f32 || *value >= i32::MAX as f32
                    }) {
                        return Err(ValidationError::invalid(
                            "atmosphere",
                            "point",
                            "Atmosphere exceeds GPU lattice support",
                        ));
                    }
                    narrowed =
                        narrowed.map(|value| value * self.parameters.fractal.lacunarity as f32);
                }
            }
        }
        Ok(())
    }

    /// Evaluates normalized object coordinates into linear-premultiplied working pixels.
    pub fn sample(&self, point: Point) -> Result<[f32; 4], ValidationError> {
        let pixel = [
            point.x * f64::from(self.canvas.width),
            point.y * f64::from(self.canvas.height),
        ];
        let value = self.parameters.sample(pixel, self.canvas, self.phase())?;
        Ok([
            value[0] * value[3],
            value[1] * value[3],
            value[2] * value[3],
            value[3],
        ]
        .map(|value| value as f32))
    }

    /// Conservative scalar work charge per evaluated pixel.
    pub fn work_per_pixel(&self) -> u64 {
        u64::from(self.parameters.fractal.octaves) * OCTAVE_WORK + ENVELOPE_WORK
    }
}

impl From<AtmospherePaint> for super::Paint {
    fn from(value: AtmospherePaint) -> Self {
        Self::Atmosphere(value)
    }
}

fn dust(point: [f64; 2], seed: u32, radius: f64, coverage: f64) -> f64 {
    let cell = point.map(|value| value.floor() as i32);
    let unit = |value: u32| f64::from(value) / f64::from(u32::MAX);
    let mut result: f64 = 0.0;
    for dy in -1i32..=1 {
        for dx in -1i32..=1 {
            let horizontal = cell[0].wrapping_add(dx);
            let vertical = cell[1].wrapping_add(dy);
            let hash = noise::hash2(horizontal, vertical, seed);
            if unit(hash) >= coverage {
                continue;
            }
            let center = [
                f64::from(horizontal) + unit(noise::hash32(hash ^ 0xa511_e9b3)),
                f64::from(vertical) + unit(noise::hash32(hash ^ 0x63d8_3595)),
            ];
            let radius = radius * (0.6 + 0.4 * unit(noise::hash32(hash ^ 0x9e37_79b9)));
            let distance = (point[0] - center[0]).powi(2) + (point[1] - center[1]).powi(2);
            result = result.max((1.0 - distance / (radius * radius)).max(0.0).powi(2));
        }
    }
    result
}
