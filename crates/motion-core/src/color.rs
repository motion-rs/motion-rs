//! Floating straight-alpha sRGB values and perceptual Oklab calculations.

pub mod math;

use crate::ValidationError;

/// A floating sRGB-encoded color with straight, unassociated alpha.
///
/// Public scalar inputs use f64; private channels use f32. Finite extended RGB
/// values are retained, including negative/out-of-gamut animation results.
/// Alpha is always in 0..=1. Invalid or unrepresentable constructor inputs panic.
/// Interpolation uses Oklab, not encoded RGB or premultiplied channels.
///
/// ```
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// const ACCENT: Color = Color::from_hex(0x4f8cff);
/// assert_eq!(ACCENT.to_rgba8(), [79, 140, 255, 255]);
/// let movement = Tween::new(Color::BLACK, Color::WHITE, 1.seconds());
/// assert_eq!(
///     movement.sample(500.milliseconds()).to_rgba8(),
///     [99, 99, 99, 255]
/// );
/// assert_eq!(
///     Color::BLACK.interpolate(&Color::WHITE, 0.5).to_rgba8(),
///     [99, 99, 99, 255]
/// );
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    channels: [f32; 4],
}

impl Color {
    /// Transparent black with zero alpha.
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);

    /// Opaque black.
    pub const BLACK: Self = Self::rgb(0, 0, 0);

    /// Opaque white.
    pub const WHITE: Self = Self::rgb(255, 255, 255);

    /// Creates an opaque color from 8-bit encoded sRGB channels.
    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self::rgba(red, green, blue, 255)
    }

    /// Creates a straight-alpha color from four 8-bit channels.
    pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self {
            channels: [
                red as f32 / 255.0,
                green as f32 / 255.0,
                blue as f32 / 255.0,
                alpha as f32 / 255.0,
            ],
        }
    }

    /// Reads the low 24 bits as 0xRRGGBB and supplies opaque alpha.
    pub const fn from_hex(value: u32) -> Self {
        Self::rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
    }

    /// Stores finite encoded RGB in f32, allowing extended values and rounding.
    /// Nonfinite values or magnitudes beyond f32::MAX panic; underflow may round to zero.
    pub fn from_rgb_float(red: f64, green: f64, blue: f64) -> Self {
        Self::from_channels([red, green, blue], 1.0)
    }

    /// Creates opaque HSL with finite degree hue and saturation/lightness in 0..=1.
    /// Hue wraps; invalid inputs are rejected rather than silently normalized.
    ///
    /// # Panics
    /// Panics for invalid inputs; see [`Self::try_hsl`].
    pub fn hsl(hue_degrees: f64, saturation: f64, lightness: f64) -> Self {
        Self::try_hsl(hue_degrees, saturation, lightness)
            .unwrap_or_else(|error| panic!("{}", error.message))
    }

    /// Creates opaque HSL like [`Self::hsl`], reporting invalid inputs.
    pub fn try_hsl(
        hue_degrees: f64,
        saturation: f64,
        lightness: f64,
    ) -> Result<Self, ValidationError> {
        if !(hue_degrees.is_finite()
            && (0.0..=1.0).contains(&saturation)
            && (0.0..=1.0).contains(&lightness))
        {
            return Err(ValidationError::invalid(
                "color",
                "hsl",
                "HSL requires finite hue and unit saturation/lightness",
            ));
        }
        Self::checked_channels(hsl_rgb(hue_degrees, saturation, lightness), 1.0)
    }

    /// Creates opaque OKLCH with lightness in 0..=1, nonnegative chroma and degree hue.
    /// Hue wraps; finite out-of-gamut RGB is retained.
    ///
    /// # Panics
    /// Panics for invalid or unrepresentable inputs; see [`Self::try_oklch`].
    pub fn oklch(lightness: f64, chroma: f64, hue_degrees: f64) -> Self {
        Self::try_oklch(lightness, chroma, hue_degrees)
            .unwrap_or_else(|error| panic!("{}", error.message))
    }

    /// Creates opaque OKLCH like [`Self::oklch`], reporting invalid or unrepresentable inputs.
    pub fn try_oklch(
        lightness: f64,
        chroma: f64,
        hue_degrees: f64,
    ) -> Result<Self, ValidationError> {
        if !((0.0..=1.0).contains(&lightness)
            && chroma.is_finite()
            && chroma >= 0.0
            && hue_degrees.is_finite())
        {
            return Err(ValidationError::invalid(
                "color",
                "oklch",
                "OKLCH requires unit lightness, finite nonnegative chroma and finite hue",
            ));
        }
        Self::checked_channels(oklch_rgb(lightness, chroma, hue_degrees), 1.0)
    }

    /// Adjusts Oklab lightness, clamping it to 0..=1 and retaining chromatic axes/alpha.
    /// Signed amounts are allowed.
    ///
    /// # Panics
    /// Panics for a nonfinite amount or unrepresentable RGB; see [`Self::try_lighten`].
    pub fn lighten(self, amount: f64) -> Self {
        self.try_lighten(amount)
            .unwrap_or_else(|error| panic!("{}", error.message))
    }

    /// Adjusts lightness like [`Self::lighten`], reporting invalid results.
    pub fn try_lighten(self, amount: f64) -> Result<Self, ValidationError> {
        if !amount.is_finite() {
            return Err(ValidationError::invalid(
                "color",
                "amount",
                "lightness adjustment must be finite",
            ));
        }
        if amount == 0.0 {
            return Ok(self);
        }
        let [red, green, blue, alpha] = self.to_rgba_float();
        let mut lab = srgb_to_oklab([red, green, blue]);
        lab[0] = (lab[0] + amount).clamp(0.0, 1.0);
        Self::checked_channels(oklab_to_srgb(lab), alpha)
    }

    /// Subtracts a signed amount from Oklab lightness, retaining chromatic axes/alpha.
    pub fn darken(self, amount: f64) -> Self {
        self.lighten(-amount)
    }

    /// Parses ASCII hex/named colors and numeric RGB/RGBA, HSL/HSLA or OKLCH.
    /// Numeric channels retain floating precision. HSL uses percentage saturation
    /// and lightness. Hue is bare degrees or `deg`; OKLCH percentages map L=100%
    /// to 1 and C=100% to 0.4. Alpha is a unit number or percentage.
    /// Commas or spaces are accepted for RGB/HSL; OKLCH uses spaces. Slash alpha
    /// belongs to space syntax. This is not a full CSS expression parser.
    /// Invalid/range errors and inputs over 4096 bytes return ValidationError;
    /// parsing does not call panicking constructors.
    ///
    /// ```
    /// use motion_core::Color;
    /// assert_eq!(
    ///     Color::parse("rgb(100% 0% 0% / 50%)")?.to_rgba8(),
    ///     [255, 0, 0, 128]
    /// );
    /// assert!(Color::parse("oklch(60% 0.1 30deg)").is_ok());
    /// assert!(Color::parse("rgb(NaN 0 0)").is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn parse(text: &str) -> crate::Result<Self> {
        Ok(text.parse()?)
    }

    /// Replaces straight alpha. Nonfinite values or alpha outside 0..=1 panic.
    pub fn with_alpha(self, alpha: f64) -> Self {
        let [red, green, blue, _] = self.to_rgba_float();
        Self::from_channels([red, green, blue], alpha)
    }

    /// Widens the stored encoded RGB and straight alpha without color conversion.
    pub fn to_rgba_float(self) -> [f64; 4] {
        self.channels.map(f64::from)
    }

    /// Clips to the encoded unit interval and rounds each channel to 8 bits.
    /// This explicitly discards extended gamut; it is not tone mapping.
    pub fn to_rgba8(self) -> [u8; 4] {
        self.to_rgba_float()
            .map(|value| libm::round(value.clamp(0.0, 1.0) * 255.0) as u8)
    }

    /// Decodes RGB and multiplies by alpha for a future linear-light renderer.
    /// Zero alpha returns transparent black without evaluating hidden RGB.
    pub fn to_linear_premultiplied(self) -> [f64; 4] {
        let [red, green, blue, alpha] = self.to_rgba_float();
        if alpha == 0.0 {
            return [0.0; 4];
        }
        [
            srgb_to_linear(red) * alpha,
            srgb_to_linear(green) * alpha,
            srgb_to_linear(blue) * alpha,
            alpha,
        ]
    }

    /// Interpolates/extrapolates straight Oklab coordinates and linear alpha.
    /// Finite progress may overshoot RGB; resulting alpha is clamped to 0..=1.
    /// Exact endpoints and equal values bypass conversion.
    ///
    /// # Panics
    /// Panics for nonfinite progress or unrepresentable RGB results; see
    /// [`Self::try_interpolate`].
    pub fn interpolate(&self, target: &Self, progress: f64) -> Self {
        self.try_interpolate(target, progress)
            .unwrap_or_else(|error| panic!("{}", error.message))
    }

    /// Interpolates like [`Self::interpolate`], reporting invalid progress or results.
    pub fn try_interpolate(&self, target: &Self, progress: f64) -> Result<Self, ValidationError> {
        if !progress.is_finite() {
            return Err(ValidationError::invalid(
                "color",
                "progress",
                "color progress must be finite",
            ));
        }
        if progress == 0.0 || self == target {
            return Ok(*self);
        }
        if progress == 1.0 {
            return Ok(*target);
        }
        let [red, green, blue, alpha] = self.to_rgba_float();
        let [target_red, target_green, target_blue, target_alpha] = target.to_rgba_float();
        let alpha = (alpha + (target_alpha - alpha) * progress).clamp(0.0, 1.0);
        if self.channels[..3] == target.channels[..3] {
            return Ok(self.with_alpha(alpha));
        }
        let from = srgb_to_oklab([red, green, blue]);
        let to = srgb_to_oklab([target_red, target_green, target_blue]);
        let lab = std::array::from_fn(|index| from[index] + (to[index] - from[index]) * progress);
        Self::checked_channels(oklab_to_srgb(lab), alpha)
    }

    fn from_channels(rgb: [f64; 3], alpha: f64) -> Self {
        Self::checked_channels(rgb, alpha).expect("Invalid floating color")
    }

    fn checked_channels(rgb: [f64; 3], alpha: f64) -> Result<Self, ValidationError> {
        for (field, value) in [("red", rgb[0]), ("green", rgb[1]), ("blue", rgb[2])] {
            if !value.is_finite() || value.abs() > f64::from(f32::MAX) {
                return Err(ValidationError::invalid(
                    "color",
                    field,
                    "Require a finite RGB component representable in f32",
                ));
            }
        }
        if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
            return Err(ValidationError::invalid(
                "color",
                "alpha",
                "Require finite alpha in 0..=1",
            ));
        }
        Ok(Self {
            channels: [rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, alpha as f32],
        })
    }
}

impl std::str::FromStr for Color {
    type Err = ValidationError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() > 4096 || !text.is_ascii() {
            return Err(parse_error(
                "Require at most 4096 bytes of ASCII color syntax",
            ));
        }
        let text = text.trim();
        if text.eq_ignore_ascii_case("transparent") {
            return Ok(Self::TRANSPARENT);
        }
        let Some((name, rest)) = text.split_once('(') else {
            let value = text
                .parse::<svgtypes::Color>()
                .map_err(|_| parse_error("Invalid named or hexadecimal color"))?;
            return Ok(Self::rgba(value.red, value.green, value.blue, value.alpha));
        };
        let body = rest
            .strip_suffix(')')
            .ok_or_else(|| parse_error("Missing closing parenthesis"))?;
        let (components, alpha, comma) = parse_components(body)?;
        let alpha = alpha
            .map(|value| scalar(value, 0.01))
            .transpose()?
            .unwrap_or(1.0);
        let rgb = match name.to_ascii_lowercase().as_str() {
            "rgb" | "rgba" => {
                let mut rgb = [0.0; 3];
                for (target, component) in rgb.iter_mut().zip(components) {
                    *target = match component.unit {
                        Unit::Number => component.value / 255.0,
                        Unit::Percent => component.value / 100.0,
                        Unit::Degrees => {
                            return Err(parse_error("RGB channels cannot have degree units"));
                        }
                    };
                }
                rgb
            }
            "hsl" | "hsla" => {
                if components[1].unit != Unit::Percent || components[2].unit != Unit::Percent {
                    return Err(parse_error(
                        "HSL saturation and lightness require percentages",
                    ));
                }
                let hue = hue(components[0])?;
                let saturation = scalar(components[1], 0.01)?;
                let lightness = scalar(components[2], 0.01)?;
                if !(0.0..=1.0).contains(&saturation) || !(0.0..=1.0).contains(&lightness) {
                    return Err(parse_error(
                        "HSL saturation and lightness must be in 0..=100%",
                    ));
                }
                hsl_rgb(hue, saturation, lightness)
            }
            "oklch" => {
                if comma {
                    return Err(parse_error("OKLCH requires space-separated components"));
                }
                let lightness = scalar(components[0], 0.01)?;
                let chroma = scalar(components[1], 0.004)?;
                if !(0.0..=1.0).contains(&lightness) || chroma < 0.0 {
                    return Err(parse_error(
                        "OKLCH requires unit lightness and nonnegative chroma",
                    ));
                }
                oklch_rgb(lightness, chroma, hue(components[2])?)
            }
            _ => return Err(parse_error("Unsupported color function")),
        };
        Self::checked_channels(rgb, alpha)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Unit {
    Number,
    Percent,
    Degrees,
}

#[derive(Clone, Copy)]
struct Component {
    value: f64,
    unit: Unit,
}

fn parse_error(message: &str) -> ValidationError {
    ValidationError::invalid("color", "text", message)
}

fn parse_component(text: &str) -> Result<Component, ValidationError> {
    let text = text.trim();
    let (number, unit) = if let Some(number) = text.strip_suffix('%') {
        (number, Unit::Percent)
    } else if let Some(number) = text.strip_suffix("deg") {
        (number, Unit::Degrees)
    } else {
        (text, Unit::Number)
    };
    let value = number
        .parse::<svgtypes::Number>()
        .map_err(|_| parse_error("Expected a complete finite color number"))?
        .0;
    Ok(Component { value, unit })
}

fn parse_components(
    body: &str,
) -> Result<([Component; 3], Option<Component>, bool), ValidationError> {
    if body.contains(',') {
        if body.contains('/') {
            return Err(parse_error("Cannot mix comma and slash separators"));
        }
        let mut fields = body.split(',');
        let components = three_components(&mut fields)?;
        let alpha = fields.next().map(parse_component).transpose()?;
        if fields.next().is_some() {
            return Err(parse_error("Unexpected trailing color data"));
        }
        return Ok((components, alpha, true));
    }
    let mut fields = body.split('/');
    let mut values = fields.next().unwrap_or_default().split_ascii_whitespace();
    let components = three_components(&mut values)?;
    if values.next().is_some() {
        return Err(parse_error("Expected slash before alpha"));
    }
    let alpha = fields.next().map(parse_component).transpose()?;
    if fields.next().is_some() {
        return Err(parse_error("Unexpected trailing color data"));
    }
    Ok((components, alpha, false))
}

fn three_components<'a>(
    fields: &mut impl Iterator<Item = &'a str>,
) -> Result<[Component; 3], ValidationError> {
    let mut components = [Component {
        value: 0.0,
        unit: Unit::Number,
    }; 3];
    for component in &mut components {
        *component = parse_component(
            fields
                .next()
                .ok_or_else(|| parse_error("Expected three color components"))?,
        )?;
    }
    Ok(components)
}

fn scalar(component: Component, percent_scale: f64) -> Result<f64, ValidationError> {
    match component.unit {
        Unit::Number => Ok(component.value),
        Unit::Percent => Ok(component.value * percent_scale),
        Unit::Degrees => Err(parse_error("Unexpected degree unit")),
    }
}

fn hue(component: Component) -> Result<f64, ValidationError> {
    if component.unit == Unit::Percent {
        return Err(parse_error("Hue must be in degrees"));
    }
    Ok(component.value)
}

fn wrapped_degrees(hue: f64) -> f64 {
    let wrapped = libm::fmod(hue, 360.0);
    if wrapped < 0.0 {
        wrapped + 360.0
    } else {
        wrapped
    }
}

fn hsl_rgb(hue: f64, saturation: f64, lightness: f64) -> [f64; 3] {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = wrapped_degrees(hue) / 60.0;
    let secondary = chroma * (1.0 - (libm::fmod(sector, 2.0) - 1.0).abs());
    let rgb = match sector as u8 {
        0 => [chroma, secondary, 0.0],
        1 => [secondary, chroma, 0.0],
        2 => [0.0, chroma, secondary],
        3 => [0.0, secondary, chroma],
        4 => [secondary, 0.0, chroma],
        _ => [chroma, 0.0, secondary],
    };
    rgb.map(|channel| channel + lightness - chroma * 0.5)
}

fn oklch_rgb(lightness: f64, chroma: f64, hue: f64) -> [f64; 3] {
    let angle = wrapped_degrees(hue) * (std::f64::consts::PI / 180.0);
    oklab_to_srgb([
        lightness,
        chroma * libm::cos(angle),
        chroma * libm::sin(angle),
    ])
}

/// Decodes a signed extended-sRGB channel without clipping.
pub fn srgb_to_linear(value: f64) -> f64 {
    let magnitude = value.abs();
    if magnitude <= 0.04045 {
        value / 12.92
    } else {
        libm::copysign(libm::pow((magnitude + 0.055) / 1.055, 2.4), value)
    }
}

/// Encodes a signed linear-sRGB channel without clipping.
pub fn linear_to_srgb(value: f64) -> f64 {
    let magnitude = value.abs();
    if magnitude <= 0.0031308 {
        value * 12.92
    } else {
        libm::copysign(1.055 * libm::pow(magnitude, 1.0 / 2.4) - 0.055, value)
    }
}

/// Converts straight encoded-sRGB channels into Oklab coordinates.
pub fn srgb_to_oklab(rgb: [f64; 3]) -> [f64; 3] {
    let [red, green, blue] = rgb.map(srgb_to_linear);
    let long = libm::cbrt(0.4122214708 * red + 0.5363325363 * green + 0.0514459929 * blue);
    let medium = libm::cbrt(0.2119034982 * red + 0.6806995451 * green + 0.1073969566 * blue);
    let short = libm::cbrt(0.0883024619 * red + 0.2817188376 * green + 0.6299787005 * blue);
    [
        0.2104542553 * long + 0.7936177850 * medium - 0.0040720468 * short,
        1.9779984951 * long - 2.4285922050 * medium + 0.4505937099 * short,
        0.0259040371 * long + 0.7827717662 * medium - 0.8086757660 * short,
    ]
}

/// Converts Oklab coordinates into straight extended encoded-sRGB channels.
pub fn oklab_to_srgb(lab: [f64; 3]) -> [f64; 3] {
    let [lightness, horizontal, vertical] = lab;
    let long = 0.9999999984505201 * lightness
        + 0.3963377921737678 * horizontal
        + 0.21580375806075877 * vertical;
    let medium = 1.0000000088817607 * lightness
        - 0.10556134232365634 * horizontal
        - 0.0638541747717059 * vertical;
    let short = 1.0000000546724108 * lightness
        - 0.08948418209496575 * horizontal
        - 1.2914855378640917 * vertical;
    let long = long * long * long;
    let medium = medium * medium * medium;
    let short = short * short * short;
    [
        4.076741661347994 * long - 3.3077115904081937 * medium + 0.2309699287294278 * short,
        -1.2684380040921763 * long + 2.609757400663372 * medium - 0.34131939631021957 * short,
        -0.004196086541837074 * long - 0.7034186144594495 * medium + 1.7076147009309446 * short,
    ]
    .map(linear_to_srgb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Color;

    #[test]
    fn signed_transfer_functions_preserve_breakpoints_extended_values_and_round_trips() {
        for value in [0.0, 0.0031308, 0.04045, 0.18, 0.5, 1.0, 4.0, -0.5, -4.0] {
            assert!((linear_to_srgb(srgb_to_linear(value)) - value).abs() < 4e-8);
            assert_eq!(srgb_to_linear(-value), -srgb_to_linear(value));
            assert_eq!(linear_to_srgb(-value), -linear_to_srgb(value));
        }
        assert_eq!(srgb_to_linear(0.04045), 0.04045 / 12.92);
        assert_eq!(linear_to_srgb(0.0031308), 0.0031308 * 12.92);
        assert!(srgb_to_linear(2.0) > 1.0);
        assert!(srgb_to_linear(-0.5) < -0.1);
    }

    #[test]
    fn oklab_primary_references_and_perceptual_neutral_midpoint_match() {
        for (rgb, expected) in [
            ([1.0, 0.0, 0.0], [0.6279553606, 0.2248630611, 0.1258462985]),
            ([0.0, 1.0, 0.0], [0.8664396115, -0.2338875742, 0.1794984799]),
            (
                [0.0, 0.0, 1.0],
                [0.4520137184, -0.0324569842, -0.3115281477],
            ),
        ] {
            for (actual, expected) in srgb_to_oklab(rgb).into_iter().zip(expected) {
                assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
            }
        }
        for channel in oklab_to_srgb(srgb_to_oklab([1.0; 3]).map(|channel| channel * 0.5)) {
            assert!((channel - 0.3885728590463344).abs() < 1e-12);
            assert!((channel - 0.5).abs() > 0.1);
        }
    }

    #[test]
    fn oklab_round_trips_finite_in_gamut_and_extended_srgb_samples() {
        for rgb in [
            [0.0; 3],
            [1.0; 3],
            [0.1, 0.7, 0.3],
            [1.0, 0.0, 0.0],
            [-0.2, 1.3, 3.0],
            [-4.0, 0.5, 0.1],
            [0.999999, 0.003, 1e-9],
        ] {
            for (actual, expected) in oklab_to_srgb(srgb_to_oklab(rgb)).into_iter().zip(rgb) {
                assert!(actual.is_finite());
                assert!(
                    (actual - expected).abs() < 3e-6 * expected.abs().max(1.0),
                    "{actual} != {expected}"
                );
            }
        }
    }

    #[test]
    fn constructors_keep_float_storage_and_round_trip_all_byte_values() {
        const ACCENT: Color = Color::from_hex(0xff4f8cff);
        assert_eq!(ACCENT.to_rgba8(), [79, 140, 255, 255]);
        for value in 0..=255 {
            let channels = [value, 255 - value, value, value];
            assert_eq!(
                Color::rgba(channels[0], channels[1], channels[2], channels[3]).to_rgba8(),
                channels
            );
        }
        let stored = Color::from_rgb_float(0.123456789, 0.5, 2.0).to_rgba_float();
        assert_eq!(stored[0], f64::from(0.123_456_79_f32));
        assert!((stored[0] * 255.0 - libm::round(stored[0] * 255.0)).abs() > 0.1);
        assert_eq!(stored[2], 2.0);
        assert_eq!(Color::TRANSPARENT.to_rgba8(), [0; 4]);
    }

    #[test]
    fn perceptual_interpolation_preserves_endpoints_hidden_rgb_and_overshoot() {
        let midpoint = Color::BLACK.interpolate(&Color::WHITE, 0.5);
        assert_eq!(midpoint.to_rgba8(), [99, 99, 99, 255]);
        let from = Color::rgb(255, 0, 0).with_alpha(0.25);
        let to = Color::rgb(0, 0, 255).with_alpha(0.75);
        assert_eq!(from.interpolate(&to, 0.0), from);
        assert_eq!(from.interpolate(&to, 1.0), to);
        assert_eq!(from.interpolate(&from, 0.37), from);
        assert_eq!(from.interpolate(&to, 0.5).to_rgba_float()[3], 0.5);
        assert!(Color::BLACK.interpolate(&Color::WHITE, 1.5).to_rgba_float()[0] > 1.0);
        assert!(
            Color::BLACK
                .interpolate(&Color::WHITE, -0.5)
                .to_rgba_float()[0]
                < 0.0
        );
        let hidden = Color::WHITE.with_alpha(0.0).interpolate(&Color::BLACK, 0.5);
        assert_eq!(hidden.to_rgba8(), [99, 99, 99, 128]);
        assert_eq!(
            Color::TRANSPARENT
                .interpolate(&Color::WHITE, 1.5)
                .to_rgba_float()[3],
            1.0
        );
        assert_eq!(
            Color::BLACK
                .interpolate(&Color::WHITE, 1e20)
                .to_rgba_float()[3],
            1.0
        );
        let translucent = Color::rgb(79, 140, 255).with_alpha(0.25);
        assert_eq!(
            translucent.interpolate(&translucent.with_alpha(0.75), 1e20),
            translucent.with_alpha(1.0)
        );
    }

    #[test]
    fn linear_premultiplication_and_explicit_quantization_have_separate_boundaries() {
        let value = Color::from_rgb_float(0.5, 0.5, 0.5).with_alpha(0.25);
        let linear = value.to_linear_premultiplied();
        for channel in &linear[..3] {
            assert!((*channel - 0.21404114048223255 * 0.25).abs() < 1e-14);
        }
        assert_eq!(linear[3], 0.25);
        assert_eq!(
            Color::from_rgb_float(f64::from(f32::MAX), -2.0, 4.0)
                .with_alpha(0.0)
                .to_linear_premultiplied(),
            [0.0; 4]
        );
        let extended = Color::from_rgb_float(-1.0, 2.0, 0.5).with_alpha(0.5);
        assert_eq!(extended.to_rgba8(), [0, 255, 128, 128]);
    }

    #[test]
    fn invalid_float_channels_alpha_and_progress_fail_without_poisoning_values() {
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX] {
            assert!(std::panic::catch_unwind(|| Color::from_rgb_float(invalid, 0.0, 0.0)).is_err());
        }
        for invalid in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
            assert!(std::panic::catch_unwind(|| Color::BLACK.with_alpha(invalid)).is_err());
        }
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(
                std::panic::catch_unwind(|| Color::BLACK.interpolate(&Color::BLACK, invalid))
                    .is_err()
            );
        }
    }

    #[test]
    fn hsl_oklch_and_lightness_adjustments_match_their_coordinate_contracts() {
        for hue in [0.0, 360.0, -360.0] {
            assert_eq!(Color::hsl(hue, 1.0, 0.5).to_rgba8(), [255, 0, 0, 255]);
        }
        assert_eq!(Color::hsl(-120.0, 1.0, 0.5).to_rgba8(), [0, 0, 255, 255]);
        assert_eq!(
            Color::oklch(0.6279553606, 0.2576833077, 29.23388519).to_rgba8(),
            [255, 0, 0, 255]
        );
        assert!(
            Color::hsl(1e300, 1.0, 0.5)
                .to_rgba_float()
                .iter()
                .all(|value| value.is_finite())
        );
        let value = Color::rgb(79, 140, 255).with_alpha(0.5);
        assert_eq!(value.lighten(0.0), value);
        let before = srgb_to_oklab(value.to_rgba_float()[..3].try_into().unwrap());
        let after = value.lighten(0.1);
        let lab = srgb_to_oklab(after.to_rgba_float()[..3].try_into().unwrap());
        assert!((lab[0] - before[0] - 0.1).abs() < 2e-7);
        assert!((lab[1] - before[1]).abs() < 2e-7 && (lab[2] - before[2]).abs() < 2e-7);
        assert_eq!(after.to_rgba_float()[3], 0.5);
        assert_eq!(value.darken(0.1), value.lighten(-0.1));
        assert!(std::panic::catch_unwind(|| Color::hsl(0.0, 2.0, 0.5)).is_err());
        assert!(std::panic::catch_unwind(|| Color::oklch(0.5, -0.1, 0.0)).is_err());
        assert!(std::panic::catch_unwind(|| value.lighten(f64::NAN)).is_err());
    }

    #[test]
    fn parsing_reuses_svg_syntax_and_retains_fractional_function_channels() -> crate::Result {
        for (text, expected) in [
            ("#f00", [255, 0, 0, 255]),
            ("#f008", [255, 0, 0, 136]),
            ("#4f8cff", [79, 140, 255, 255]),
            ("#4f8cff80", [79, 140, 255, 128]),
            (" red ", [255, 0, 0, 255]),
            ("transparent", [0, 0, 0, 0]),
            ("rgb(255, 0, 0)", [255, 0, 0, 255]),
            ("rgba(255, 0, 0, .5)", [255, 0, 0, 128]),
            ("RGB(100% 0% 0% / 50%)", [255, 0, 0, 128]),
            ("hsl(120deg 100% 50% / 50%)", [0, 255, 0, 128]),
            (
                "oklch(62.79553606% 0.2576833077 29.23388519 / .5)",
                [255, 0, 0, 128],
            ),
        ] {
            assert_eq!(Color::parse(text)?.to_rgba8(), expected, "{text}");
        }
        assert_eq!("#4f8cff".parse::<Color>()?, Color::from_hex(0x4f8cff));
        assert_eq!(
            Color::parse("rgb(.5 0 0)")?.to_rgba_float()[0],
            f64::from((0.5 / 255.0) as f32)
        );
        assert_eq!(Color::parse("rgb(200% -20% 0)")?.to_rgba_float()[0], 2.0);
        assert_eq!(
            Color::parse("oklch(50% 25% 30deg)")?,
            Color::oklch(0.5, 0.1, 30.0)
        );
        Ok(())
    }

    #[test]
    fn malformed_color_text_returns_structured_errors_without_panicking() {
        for text in [
            "",
            "#12xz89",
            "#12345",
            "unknown",
            "rgb()",
            "rgb(1 2)",
            "rgb(1 2 3 4)",
            "rgb(1,,2,3)",
            "rgb(1 2,3)",
            "rgb(1-2 3)",
            "rgb(1 2 3) trailing",
            "rgb(NaN 0 0)",
            "rgb(1e309 0 0)",
            "rgb(1e100 0 0)",
            "rgb(0 0 0 / 2)",
            "rgb(1deg 0 0)",
            "oklch(.5,-.1,30)",
            "oklch(.5 -.1 30)",
            "oklch(2 .1 30)",
            "oklch(.5 .1 30%)",
            "oklch(.5 1e308 30)",
            "hsl(0 .5 .5)",
            "hsl(0 200% 50%)",
            "var(--fill)",
            "r\u{e9}d",
        ] {
            let result = std::panic::catch_unwind(|| Color::parse(text));
            let error = result
                .unwrap_or_else(|_| panic!("parser panicked for {text}"))
                .unwrap_err();
            let diagnostic = error.downcast_ref::<ValidationError>().unwrap();
            assert_eq!(diagnostic.path, "color");
        }
        assert!(Color::parse(&"0".repeat(4097)).is_err());
    }
}
