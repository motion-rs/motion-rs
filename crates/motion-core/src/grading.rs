//! Immutable color grading data, without filesystem or renderer dependencies.

use crate::ValidationError;
use std::sync::Arc;

/// ASC slope/offset/power and Rec.709 saturation in straight linear RGB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cdl {
    /// Per-channel gain.
    pub slope: [f32; 3],
    /// Per-channel offset.
    pub offset: [f32; 3],
    /// Per-channel nonnegative power.
    pub power: [f32; 3],
    /// Nonnegative chroma multiplier.
    pub saturation: f32,
}

impl Default for Cdl {
    fn default() -> Self {
        Self {
            slope: [1.0; 3],
            offset: [0.0; 3],
            power: [1.0; 3],
            saturation: 1.0,
        }
    }
}

impl Cdl {
    /// Composes simple linear RGB temperature/tint gains in -1..=1.
    pub fn temperature_tint(
        mut self,
        temperature: f32,
        tint: f32,
    ) -> Result<Self, ValidationError> {
        if !(-1.0..=1.0).contains(&temperature) || !(-1.0..=1.0).contains(&tint) {
            return Err(ValidationError::invalid(
                "cdl",
                "white_balance",
                "Temperature and tint must be finite in -1..=1",
            ));
        }
        let gains = [
            libm::exp2f(temperature * 0.5 + tint * 0.25),
            libm::exp2f(-tint * 0.5),
            libm::exp2f(-temperature * 0.5 + tint * 0.25),
        ];
        self.slope = std::array::from_fn(|channel| self.slope[channel] * gains[channel]);
        self.validate()?;
        Ok(self)
    }

    /// Validates finite, nonnegative gain, power and saturation.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self
            .slope
            .iter()
            .chain(&self.offset)
            .chain(&self.power)
            .any(|value| !value.is_finite())
            || self
                .slope
                .iter()
                .chain(&self.power)
                .any(|value| *value < 0.0)
            || !self.saturation.is_finite()
            || self.saturation < 0.0
        {
            return Err(ValidationError::invalid(
                "cdl",
                "parameters",
                "Require finite CDL parameters and nonnegative gain, power and saturation",
            ));
        }
        Ok(())
    }

    /// Applies the no-clamp CDL convention, clipping negative power bases only.
    pub fn apply(self, rgb: [f32; 3]) -> [f32; 3] {
        if self == Self::default() {
            return rgb;
        }
        let rgb = std::array::from_fn(|channel| {
            libm::powf(
                (rgb[channel] * self.slope[channel] + self.offset[channel]).max(0.0),
                self.power[channel],
            )
        });
        if self.saturation == 1.0 {
            return rgb;
        }
        let luma = rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
        rgb.map(|value| luma + (value - luma) * self.saturation)
    }
}

/// Encoding expected on both sides of a look LUT.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum LutInput {
    /// Scene-linear values.
    #[default]
    Linear,
    /// Encoded sRGB values.
    SrgbEncoded,
    /// Normalized log2 stops between two strictly ordered limits.
    Log {
        /// Minimum stop.
        minimum: f32,
        /// Maximum stop.
        maximum: f32,
    },
}

impl LutInput {
    /// Validates the optional log domain.
    pub fn validate(self) -> Result<(), ValidationError> {
        if let Self::Log { minimum, maximum } = self
            && (!minimum.is_finite()
                || !maximum.is_finite()
                || minimum >= maximum
                || minimum < -64.0
                || maximum > 64.0)
        {
            return Err(ValidationError::invalid(
                "lut",
                "encoding",
                "Require ordered log limits within -64..=64 stops",
            ));
        }
        Ok(())
    }
}

/// Immutable per-channel linear shaper with 2..=65536 samples.
///
/// ```
/// use motion_core::{Cdl, ColorWheels, Lut1d};
/// let shaper = Lut1d::new([0.0; 3], [16.0; 3], vec![[0.0; 3], [0.8; 3], [1.0; 3]])?;
/// let look = ColorWheels::default().bake(2, 1.0, Cdl::default())?.with_shaper(shaper);
/// assert!((look.sample([8.0; 3])[0] - 0.8).abs() < 1e-6);
/// # Ok::<(), motion_core::ValidationError>(())
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Lut1d {
    domain_min: [f32; 3],
    domain_max: [f32; 3],
    data: Arc<[[f32; 3]]>,
}

impl Lut1d {
    /// Freezes finite samples and strictly increasing per-channel input domains.
    pub fn new(
        domain_min: [f32; 3],
        domain_max: [f32; 3],
        data: Vec<[f32; 3]>,
    ) -> Result<Self, ValidationError> {
        if !(2..=65536).contains(&data.len())
            || (0..3).any(|channel| {
                !domain_min[channel].is_finite()
                    || !domain_max[channel].is_finite()
                    || domain_min[channel] >= domain_max[channel]
                    || !(domain_max[channel] - domain_min[channel]).is_finite()
            })
            || data.iter().flatten().any(|value| !value.is_finite())
        {
            return Err(ValidationError::invalid(
                "lut",
                "shaper",
                "Require 2..=65536 finite samples and ordered finite domains",
            ));
        }
        Ok(Self {
            domain_min,
            domain_max,
            data: data.into(),
        })
    }

    /// Minimum input domain.
    pub fn domain_min(&self) -> [f32; 3] {
        self.domain_min
    }

    /// Maximum input domain.
    pub fn domain_max(&self) -> [f32; 3] {
        self.domain_max
    }

    /// Immutable RGB rows, one sample per channel in each row.
    pub fn data(&self) -> &[[f32; 3]] {
        &self.data
    }

    /// Parses a standalone 1D `.cube`; combined files belong to `Lut3d::from_cube`.
    pub fn from_cube(text: &str) -> Result<Self, ValidationError> {
        let parsed = ParsedCube::parse(text)?;
        if parsed.size.is_some() {
            return Err(cube_error());
        }
        parsed.shaper.ok_or_else(cube_error)
    }

    /// Samples channels independently, clamping input but not output.
    pub fn sample(&self, rgb: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|channel| {
            let value = if rgb[channel].is_nan() {
                self.domain_min[channel]
            } else {
                rgb[channel]
            };
            let position = ((value - self.domain_min[channel])
                / (self.domain_max[channel] - self.domain_min[channel]))
                .clamp(0.0, 1.0)
                * (self.data.len() - 1) as f32;
            let index = (position as usize).min(self.data.len() - 2);
            let fraction = position - index as f32;
            self.data[index][channel] * (1.0 - fraction) + self.data[index + 1][channel] * fraction
        })
    }
}

/// Validated three-dimensional cube, with red varying fastest.
#[derive(Clone, Debug)]
pub struct Lut3d {
    size: u16,
    domain_min: [f32; 3],
    domain_max: [f32; 3],
    data: Arc<[[f32; 3]]>,
    shaper: Option<Lut1d>,

    fingerprint: u64,
}

impl PartialEq for Lut3d {
    fn eq(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint
            && self.size == other.size
            && self.domain_min == other.domain_min
            && self.domain_max == other.domain_max
            && self.data == other.data
            && self.shaper == other.shaper
    }
}

/// Smooth three-way linear-light grading around the 0.18 middle-gray pivot.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ColorWheels {
    /// Shadow RGB offsets, weighted down to zero at luminance 0.5.
    pub shadows: [f32; 3],
    /// Midtone RGB offsets, peaking at luminance 0.18.
    pub midtones: [f32; 3],
    /// Highlight RGB offsets, smoothly entering above luminance 0.5.
    pub highlights: [f32; 3],
}

impl ColorWheels {
    /// Evaluates finite, unclamped scene-linear RGB offsets.
    pub fn apply(self, rgb: [f32; 3]) -> [f32; 3] {
        let luma = (rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722).max(0.0);
        let smooth = |value: f32| {
            let value = value.clamp(0.0, 1.0);
            value * value * (3.0 - 2.0 * value)
        };
        let shadow = 1.0 - smooth(luma / 0.5);
        let highlight = smooth((luma - 0.5) / 0.5);
        let midtone = if luma <= 0.18 {
            smooth(luma / 0.18)
        } else {
            1.0 - smooth((luma - 0.18) / 0.82)
        };
        std::array::from_fn(|channel| {
            rgb[channel]
                + self.shadows[channel] * shadow
                + self.midtones[channel] * midtone
                + self.highlights[channel] * highlight
        })
    }

    /// Bakes an original look into an immutable CPU/GPU cube in an explicit domain.
    pub fn bake(self, size: u16, maximum: f32, cdl: Cdl) -> Result<Lut3d, ValidationError> {
        cdl.validate()?;
        if !(2..=64).contains(&size)
            || !maximum.is_finite()
            || maximum <= 0.0
            || self
                .shadows
                .iter()
                .chain(&self.midtones)
                .chain(&self.highlights)
                .any(|value| !value.is_finite())
        {
            return Err(ValidationError::invalid(
                "color_wheels",
                "parameters",
                "Require finite wheels, positive domain and size 2..=64",
            ));
        }
        let mut data = Vec::with_capacity(usize::from(size).pow(3));
        for blue in 0..size {
            for green in 0..size {
                for red in 0..size {
                    data.push(
                        self.apply(
                            cdl.apply(
                                [red, green, blue]
                                    .map(|value| f32::from(value) / f32::from(size - 1) * maximum),
                            ),
                        ),
                    );
                }
            }
        }
        Lut3d::new(size, [0.0; 3], [maximum; 3], data)
    }
}

impl Lut3d {
    /// Freezes a finite 2..=64 cube with a strictly increasing per-channel domain.
    pub fn new(
        size: u16,
        domain_min: [f32; 3],
        domain_max: [f32; 3],
        data: Vec<[f32; 3]>,
    ) -> Result<Self, ValidationError> {
        if !(2..=64).contains(&size)
            || data.len() != usize::from(size).pow(3)
            || (0..3).any(|channel| {
                !domain_min[channel].is_finite()
                    || !domain_max[channel].is_finite()
                    || domain_min[channel] >= domain_max[channel]
                    || !(domain_max[channel] - domain_min[channel]).is_finite()
            })
            || data.iter().flatten().any(|value| !value.is_finite())
        {
            return Err(ValidationError::invalid(
                "lut",
                "cube",
                "Require a finite 2..=64 cube and ordered finite domain",
            ));
        }
        let mut fingerprint = 0xcbf29ce484222325u64;
        for value in domain_min
            .iter()
            .chain(&domain_max)
            .chain(data.iter().flatten())
        {
            for byte in value.to_bits().to_le_bytes() {
                fingerprint = (fingerprint ^ u64::from(byte)).wrapping_mul(0x100000001b3);
            }
        }
        Ok(Self {
            size,
            domain_min,
            domain_max,
            data: data.into(),
            shaper: None,
            fingerprint,
        })
    }

    /// Parses a bounded 3D `.cube`, optionally preceded by a 1D shaper.
    ///
    /// In combined files DOMAIN_MIN/MAX describe the shaper input. The cube
    /// defaults to 0..1 unless LUT_3D_INPUT_RANGE specifies another range.
    pub fn from_cube(text: &str) -> Result<Self, ValidationError> {
        let parsed = ParsedCube::parse(text)?;
        let mut lut = Self::new(
            parsed.size.ok_or_else(cube_error)?,
            parsed.min,
            parsed.max,
            parsed.data,
        )?;
        lut.shaper = parsed.shaper;
        Ok(lut)
    }

    /// Cube edge length.
    pub fn size(&self) -> u16 {
        self.size
    }

    /// Minimum input domain.
    pub fn domain_min(&self) -> [f32; 3] {
        self.domain_min
    }

    /// Maximum input domain.
    pub fn domain_max(&self) -> [f32; 3] {
        self.domain_max
    }

    /// Red-fastest immutable samples.
    pub fn data(&self) -> &[[f32; 3]] {
        &self.data
    }

    /// Attaches an input shaper; its output is interpreted in the cube domain.
    pub fn with_shaper(mut self, shaper: Lut1d) -> Self {
        self.shaper = Some(shaper);
        self
    }

    /// Optional per-channel input shaper, evaluated before tetrahedral lookup.
    pub fn shaper(&self) -> Option<&Lut1d> {
        self.shaper.as_ref()
    }

    /// Encodes linear input, samples a look and decodes its output back to linear.
    pub fn apply(&self, rgb: [f32; 3], input: LutInput) -> [f32; 3] {
        let encoded = rgb.map(|value| match input {
            LutInput::Linear => value,
            LutInput::SrgbEncoded => crate::linear_to_srgb(value.max(0.0)),
            LutInput::Log { minimum, maximum } => {
                (libm::log2f(value.max(libm::exp2f(minimum))) - minimum) / (maximum - minimum)
            }
        });
        self.sample(encoded).map(|value| match input {
            LutInput::Linear => value,
            LutInput::SrgbEncoded => crate::srgb_to_linear(value.max(0.0)),
            LutInput::Log { minimum, maximum } => {
                libm::exp2f(value * (maximum - minimum) + minimum)
            }
        })
    }

    /// Tetrahedral interpolation; out-of-domain input clamps to the boundary.
    pub fn sample(&self, rgb: [f32; 3]) -> [f32; 3] {
        let rgb = self
            .shaper
            .as_ref()
            .map_or(rgb, |shaper| shaper.sample(rgb));
        let rgb: [f32; 3] = std::array::from_fn(|channel| {
            if rgb[channel].is_nan() {
                self.domain_min[channel]
            } else {
                rgb[channel]
            }
        });
        let edge = usize::from(self.size);
        let position: [f32; 3] = std::array::from_fn(|channel| {
            ((rgb[channel] - self.domain_min[channel])
                / (self.domain_max[channel] - self.domain_min[channel]))
                .clamp(0.0, 1.0)
                * (edge - 1) as f32
        });
        let mut cell = position.map(|value| (value as usize).min(edge - 2));
        let fraction: [f32; 3] =
            std::array::from_fn(|channel| position[channel] - cell[channel] as f32);
        let mut order = [0, 1, 2];
        order.sort_by(|first, second| fraction[*second].total_cmp(&fraction[*first]));
        let at = |cell: [usize; 3]| self.data[cell[0] + edge * (cell[1] + edge * cell[2])];
        let start = at(cell);
        cell[order[0]] += 1;
        let first = at(cell);
        cell[order[1]] += 1;
        let second = at(cell);
        cell[order[2]] += 1;
        let end = at(cell);
        std::array::from_fn(|channel| {
            start[channel]
                + fraction[order[0]] * (first[channel] - start[channel])
                + fraction[order[1]] * (second[channel] - first[channel])
                + fraction[order[2]] * (end[channel] - second[channel])
        })
    }
}

fn cube_error() -> ValidationError {
    ValidationError::invalid(
        "lut",
        "cube",
        "Malformed, ambiguous or unsupported .cube data",
    )
}

struct ParsedCube {
    size: Option<u16>,
    min: [f32; 3],
    max: [f32; 3],
    data: Vec<[f32; 3]>,
    shaper: Option<Lut1d>,
}

impl ParsedCube {
    fn parse(text: &str) -> Result<Self, ValidationError> {
        if text.len() > 8 * 1024 * 1024 {
            return Err(cube_error());
        }
        let mut size = None;
        let mut shaper_size = None;
        let mut domain_min = None;
        let mut domain_max = None;
        let mut input1 = None;
        let mut input3 = None;
        let mut title = false;
        let mut data = Vec::new();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            let words: Vec<_> = line.split_whitespace().collect();
            let number = |index: usize| -> Result<f32, ValidationError> {
                let value: f32 = words
                    .get(index)
                    .ok_or_else(cube_error)?
                    .parse()
                    .map_err(|_| cube_error())?;
                if !value.is_finite() {
                    return Err(cube_error());
                }
                Ok(value)
            };
            match words[0] {
                "TITLE"
                    if !title
                        && data.is_empty()
                        && line[5..].trim().starts_with('"')
                        && line.ends_with('"') =>
                {
                    title = true;
                }
                "LUT_3D_SIZE" if size.is_none() && data.is_empty() && words.len() == 2 => {
                    let count: u16 = words[1].parse().map_err(|_| cube_error())?;
                    if !(2..=64).contains(&count) {
                        return Err(cube_error());
                    }
                    size = Some(count);
                }
                "LUT_1D_SIZE" if shaper_size.is_none() && data.is_empty() && words.len() == 2 => {
                    let count: usize = words[1].parse().map_err(|_| cube_error())?;
                    if !(2..=65536).contains(&count) {
                        return Err(cube_error());
                    }
                    shaper_size = Some(count);
                }
                "DOMAIN_MIN" if domain_min.is_none() && data.is_empty() && words.len() == 4 => {
                    domain_min = Some([number(1)?, number(2)?, number(3)?]);
                }
                "DOMAIN_MAX" if domain_max.is_none() && data.is_empty() && words.len() == 4 => {
                    domain_max = Some([number(1)?, number(2)?, number(3)?]);
                }
                "LUT_1D_INPUT_RANGE" if input1.is_none() && data.is_empty() && words.len() == 3 => {
                    input1 = Some((number(1)?, number(2)?));
                }
                "LUT_3D_INPUT_RANGE" if input3.is_none() && data.is_empty() && words.len() == 3 => {
                    input3 = Some((number(1)?, number(2)?));
                }
                _ => {
                    let count = shaper_size.unwrap_or(0)
                        + size.map_or(0, |value| usize::from(value).pow(3));
                    if words.len() != 3 || data.len() >= count {
                        return Err(cube_error());
                    }
                    if data.is_empty() {
                        data.try_reserve_exact(count).map_err(|_| cube_error())?;
                    }
                    data.push([number(0)?, number(1)?, number(2)?]);
                }
            }
        }
        let has_domain = domain_min.is_some() || domain_max.is_some();
        if (input1.is_some() && (shaper_size.is_none() || has_domain))
            || (input3.is_some() && (size.is_none() || (shaper_size.is_none() && has_domain)))
            || (size.is_none() && shaper_size.is_none())
            || data.len()
                != shaper_size.unwrap_or(0) + size.map_or(0, |value| usize::from(value).pow(3))
        {
            return Err(cube_error());
        }
        let min = domain_min.unwrap_or([0.0; 3]);
        let max = domain_max.unwrap_or([1.0; 3]);
        let shaper = if let Some(count) = shaper_size {
            let (min, max) = input1.map_or((min, max), |(min, max)| ([min; 3], [max; 3]));
            Some(Lut1d::new(min, max, data.drain(..count).collect())?)
        } else {
            None
        };
        let (min, max) = input3.map_or(
            if shaper.is_some() {
                ([0.0; 3], [1.0; 3])
            } else {
                (min, max)
            },
            |(min, max)| ([min; 3], [max; 3]),
        );
        Ok(Self {
            size,
            min,
            max,
            data,
            shaper,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shaper_precedes_cube_and_preserves_unbounded_output() {
        let shaper = Lut1d::new(
            [0.0; 3],
            [4.0; 3],
            vec![[0.0; 3], [0.2, 0.4, 0.6], [1.0; 3]],
        )
        .unwrap();
        assert_eq!(shaper.sample([2.0; 3]), [0.2, 0.4, 0.6]);
        assert_eq!(
            shaper.sample([f32::NAN, f32::INFINITY, -1.0]),
            [0.0, 1.0, 0.0]
        );
        let cube = ColorWheels::default().bake(2, 1.0, Cdl::default()).unwrap();
        let shaped = cube.clone().with_shaper(shaper);
        assert_ne!(cube, shaped);
        for (actual, expected) in shaped.sample([2.0; 3]).into_iter().zip([0.2, 0.4, 0.6]) {
            assert!((actual - expected).abs() < 1e-6);
        }
        let hdr = Lut1d::new([0.0; 3], [1.0; 3], vec![[-2.0; 3], [8.0; 3]]).unwrap();
        assert_eq!(hdr.sample([0.5; 3]), [3.0; 3]);
        assert!(Lut1d::new([0.0; 3], [0.0; 3], vec![[0.0; 3]; 2]).is_err());
        assert!(Lut1d::new([0.0; 3], [1.0; 3], vec![[0.0; 3]]).is_err());
    }

    #[test]
    fn standalone_and_combined_cube_shapers_parse() {
        let header = "LUT_1D_SIZE 3\nLUT_1D_INPUT_RANGE 0 4\n";
        let rows = "0 0 0\n0.2 0.4 0.6\n1 1 1\n";
        let standalone = Lut1d::from_cube(&format!("{header}{rows}")).unwrap();
        assert_eq!(standalone.sample([2.0; 3]), [0.2, 0.4, 0.6]);
        let cube = "0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        let combined = format!("{header}LUT_3D_SIZE 2\nLUT_3D_INPUT_RANGE 0 1\n{rows}{cube}");
        let lut = Lut3d::from_cube(&combined).unwrap();
        assert_eq!(lut.shaper(), Some(&standalone));
        for (actual, expected) in lut.sample([2.0; 3]).into_iter().zip([0.2, 0.4, 0.6]) {
            assert!((actual - expected).abs() < 1e-6);
        }
        assert!(Lut1d::from_cube(&combined).is_err());
        for text in [
            "LUT_1D_SIZE 65537",
            "LUT_1D_SIZE 2\n0 0 0",
            "LUT_1D_SIZE 2\nLUT_1D_SIZE 2",
            "LUT_1D_SIZE 2\nLUT_1D_INPUT_RANGE 1 0\n0 0 0\n1 1 1",
            "LUT_1D_SIZE 2\nLUT_1D_INPUT_RANGE 0 1\nDOMAIN_MIN 0 0 0\n0 0 0\n1 1 1",
            "LUT_1D_SIZE 2\n0 0 0\nNaN 1 1",
            "LUT_1D_SIZE 2\n0 0 0\n1 1 1\n2 2 2",
        ] {
            assert!(Lut1d::from_cube(text).is_err(), "{text}");
        }
    }

    #[test]
    fn shaper_runs_between_input_encoding_and_output_decoding() {
        let shaper = Lut1d::new(
            [0.0; 3],
            [1.0; 3],
            vec![[0.0; 3], [0.7, 0.2, 0.4], [1.0; 3]],
        )
        .unwrap();
        let lut = ColorWheels::default()
            .bake(2, 1.0, Cdl::default())
            .unwrap()
            .with_shaper(shaper.clone());
        let rgb = [0.01, 0.18, 0.8];
        for input in [
            LutInput::Linear,
            LutInput::SrgbEncoded,
            LutInput::Log {
                minimum: -8.0,
                maximum: 4.0,
            },
        ] {
            let encoded = rgb.map(|value| match input {
                LutInput::Linear => value,
                LutInput::SrgbEncoded => crate::linear_to_srgb(value),
                LutInput::Log { minimum, maximum } => {
                    (libm::log2f(value) - minimum) / (maximum - minimum)
                }
            });
            let expected = shaper.sample(encoded).map(|value| match input {
                LutInput::Linear => value,
                LutInput::SrgbEncoded => crate::srgb_to_linear(value),
                LutInput::Log { minimum, maximum } => {
                    libm::exp2f(value * (maximum - minimum) + minimum)
                }
            });
            for (actual, expected) in lut.apply(rgb, input).into_iter().zip(expected) {
                assert!((actual - expected).abs() <= 1e-5 * expected.abs().max(1.0));
            }
        }
    }

    #[test]
    fn identity_cube_and_tetrahedral_boundaries() {
        let lut = ColorWheels::default().bake(4, 1.0, Cdl::default()).unwrap();
        for rgb in [
            [0.0; 3],
            [1.0; 3],
            [0.13, 0.83, 0.42],
            [0.8, 0.2, 0.2],
            [0.5; 3],
        ] {
            let actual = lut.sample(rgb);
            for channel in 0..3 {
                assert!((actual[channel] - rgb[channel]).abs() < 1e-5);
            }
        }
        assert_eq!(lut.sample([-1.0, 2.0, 0.0]), [0.0, 1.0, 0.0]);
        assert_eq!(
            lut,
            ColorWheels::default().bake(4, 1.0, Cdl::default()).unwrap()
        );
    }

    #[test]
    fn cube_parser_rejects_missing_cube_duplicates_nan_and_truncation() {
        for text in [
            "LUT_1D_SIZE 2",
            "LUT_3D_SIZE 1",
            "LUT_3D_SIZE 2\n0 0 0",
            "LUT_3D_SIZE 2\nLUT_3D_SIZE 2",
            "LUT_3D_SIZE 2\nNaN 0 0",
        ] {
            assert!(Lut3d::from_cube(text).is_err());
        }
        let data = "TITLE \"identity\"\nLUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        assert_eq!(
            Lut3d::from_cube(data).unwrap().sample([0.2, 0.7, 0.4]),
            [0.2, 0.7, 0.4]
        );
    }

    #[test]
    fn cdl_white_balance_and_wheels_validate() {
        assert_eq!(Cdl::default().apply([-0.1, 0.18, 4.0]), [-0.1, 0.18, 4.0]);
        assert_eq!(
            Cdl::default().temperature_tint(0.0, 0.0).unwrap(),
            Cdl::default()
        );
        assert!(Cdl::default().temperature_tint(f32::NAN, 0.0).is_err());
        assert!(
            ColorWheels {
                shadows: [f32::NAN; 3],
                ..ColorWheels::default()
            }
            .bake(2, 1.0, Cdl::default())
            .is_err()
        );
    }
}
