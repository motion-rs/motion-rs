//! Subtree filters, clips and masks.

use crate::content::{CircleSpec, RectSpec};
use crate::element::Element;
use crate::value::Animated;
use motion_core::{ClipMode, Color, MaskMode, Rect};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// An ordered subtree filter, written as `{"blur": {"sigma": 4}}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum FilterSpec {
    /// Gaussian blur.
    Blur {
        /// Nonnegative standard deviation in design units.
        sigma: Animated<f64>,
    },
    /// Offset blurred silhouette behind the subtree.
    DropShadow {
        /// Shadow offset `[x, y]`.
        offset: Animated<[f64; 2]>,
        /// Nonnegative blur.
        sigma: Animated<f64>,
        /// Shadow color.
        color: Animated<Color>,
    },
    /// Additive blurred highlights above a luminance threshold.
    Glow {
        /// Luminance threshold.
        threshold: Animated<f64>,
        /// Nonnegative blur.
        sigma: Animated<f64>,
        /// Nonnegative strength.
        intensity: Animated<f64>,
    },
    /// Multi-scale highlight bloom.
    Bloom(BloomSpec),
    /// Exposure, contrast, saturation and hue adjustment.
    ColorAdjust(ColorAdjustSpec),
    /// ASC CDL slope/offset/power with saturation.
    Cdl(CdlSpec),
    /// Red-orange film highlight halation.
    Halation {
        /// Nonnegative strength.
        strength: Animated<f64>,
        /// Positive radius in design units.
        radius: f64,
    },
    /// Independent horizontal and vertical Gaussian blur.
    BlurXy {
        /// Horizontal standard deviation in design units.
        sigma_x: Animated<f64>,
        /// Vertical standard deviation in design units.
        sigma_y: Animated<f64>,
    },
    /// Seeded scene-linear monochrome noise preserving alpha.
    Grain {
        /// Explicit full-width deterministic seed.
        seed: u64,
        /// Nonnegative noise amplitude.
        strength: Animated<f64>,
    },
    /// Bounded optical effects on the completed subtree.
    Optics(OpticsSpec),
}

impl FilterSpec {
    /// Creates a blur filter.
    pub fn blur(sigma: impl Into<Animated<f64>>) -> Self {
        Self::Blur {
            sigma: sigma.into(),
        }
    }
}

/// Multi-scale bloom; omitted values use the cinematic look's settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BloomSpec {
    /// Linear luminance threshold; 0.9 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    /// Soft threshold knee; 0.25 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knee: Option<f64>,
    /// Base radius in design units; 4 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    /// Pyramid levels; 5 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<u8>,
    /// Per-level weight falloff; 0.5 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub falloff: Option<f64>,
    /// Strength; 0.15 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intensity: Option<Animated<f64>>,
    /// Bloom tint; white by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<Color>,
}

/// Color adjustment; omitted values are neutral.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ColorAdjustSpec {
    /// Exposure in stops; zero by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exposure: Option<Animated<f64>>,
    /// Contrast multiplier; one by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contrast: Option<Animated<f64>>,
    /// Saturation multiplier; one by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saturation: Option<Animated<f64>>,
    /// Hue rotation in degrees; zero by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hue: Option<Animated<f64>>,
}

/// ASC CDL grade; omitted values are neutral.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CdlSpec {
    /// Per-channel gain; ones by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slope: Option<[f64; 3]>,
    /// Per-channel offset; zeros by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<[f64; 3]>,
    /// Per-channel nonnegative power; ones by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power: Option<[f64; 3]>,
    /// Nonnegative chroma multiplier; one by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saturation: Option<f64>,
}

/// Optical effects lowered to the core lens and relighting implementations.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum OpticsSpec {
    /// Mirrored highlights and a radial halo.
    LensFlare {
        /// Optical center in local design units.
        center: Animated<[f64; 2]>,
        /// Ghost count in `1..=8`.
        ghosts: u8,
        /// Nonnegative highlight cutoff.
        threshold: Animated<f64>,
        /// Nonnegative added energy.
        intensity: Animated<f64>,
        /// Ghost tint.
        tint: Color,
    },
    /// Radial distortion and chromatic dispersion.
    Lens {
        /// Radial distortion coefficient in `-0.3..=1`.
        distortion: Animated<f64>,
        /// Chromatic coefficient in `-0.1..=0.1`.
        chromatic: Animated<f64>,
        /// Shrinks sampling coordinates to protect corners.
        auto_fit: bool,
        /// Optical center; effect bounds are used when omitted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        center: Option<Animated<[f64; 2]>>,
    },
    /// Physical cosine-fourth falloff.
    LensVignette {
        /// Full diagonal view angle in degrees, between zero and 170.
        angle_of_view: f64,
        /// Effect strength in `0..=1`.
        strength: Animated<f64>,
        /// Elliptical at zero, circular at one.
        roundness: f64,
    },
    /// Bounded aperture blur.
    Bokeh {
        /// Nonnegative blur radius in design units.
        radius: Animated<f64>,
        /// Aperture footprint.
        shape: ApertureSpec,
        /// Nonnegative highlight weighting.
        highlight_boost: f64,
    },
    /// Decaying samples toward a local light position.
    RadialBlur {
        /// Light position in design units.
        center: Animated<[f64; 2]>,
        /// Ray length fraction in `0..=1`.
        strength: Animated<f64>,
        /// Tap count in `1..=64`.
        samples: u8,
        /// Relative decay in `0..=1`.
        decay: f64,
    },
    /// Horizontal highlight streaks.
    Anamorphic {
        /// Nonnegative luminance cutoff.
        threshold: f64,
        /// Horizontal standard deviation in design units.
        length: Animated<f64>,
        /// Horizontal-to-vertical ratio in `1..=64`.
        stretch: f64,
        /// Streak tint.
        tint: Color,
        /// Nonnegative added energy.
        intensity: f64,
    },
    /// Sobel height normals and diffuse/specular lighting.
    Relight {
        /// Height source.
        height_from: HeightSourceSpec,
        /// Finite nonzero direction toward the light.
        light_dir: [Animated<f64>; 3],
        /// Light color.
        color: Color,
        /// Nonnegative ambient multiplier.
        ambient: f64,
        /// Nonnegative specular multiplier.
        specular: f64,
    },
}

/// Bokeh aperture geometry, with clockwise rotation authored in degrees.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ApertureSpec {
    /// Circular aperture.
    Disc,
    /// Regular aperture polygon.
    Polygon {
        /// Blade count in `3..=16`.
        sides: u8,
        /// Clockwise rotation in degrees.
        rotation: Animated<f64>,
    },
}

/// Channel used to derive a relighting height field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum HeightSourceSpec {
    /// Straight-linear Rec.709 luminance.
    Luma,
    /// Source alpha.
    Alpha,
}

/// A clip applied to the completed subtree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipSpec {
    /// Clip geometry in the element's local space.
    pub shape: ClipShapeSpec,
    /// Coverage multiplication by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<ClipMode>,
}

impl ClipSpec {
    /// Creates a coverage clip.
    pub fn new(shape: ClipShapeSpec) -> Self {
        Self { shape, mode: None }
    }

    /// Selects coverage or geometric clipping.
    pub fn mode(mut self, mode: ClipMode) -> Self {
        self.mode = Some(mode);
        self
    }
}

/// Clip geometry: a rectangle, rounded rectangle, circle or SVG path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ClipShapeSpec {
    /// Rectangle `[left, top, right, bottom]`.
    Rect(Animated<Rect>),
    /// Rectangle with an optional animated corner radius in design units; zero by default.
    RoundedRect(RectSpec),
    /// Circle.
    Circle(CircleSpec),
    /// SVG path data.
    Path(String),
}

/// A content mask rendered from another element.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaskSpec {
    /// Mask artwork in the element's local space.
    pub element: Element,
    /// Alpha by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<MaskMode>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_filters_accept_animated_axes_and_seeded_grain() -> serde_json::Result<()> {
        for source in [
            r#"{"blur_xy":{"sigma_x":{"expr":"2 + t"},"sigma_y":1}}"#,
            r#"{"grain":{"seed":18446744073709551615,"strength":{"expr":"0.1 * t"}}}"#,
        ] {
            let filter: FilterSpec = serde_json::from_str(source)?;
            assert_eq!(
                serde_json::from_str::<FilterSpec>(&serde_json::to_string(&filter)?)?,
                filter
            );
        }
        Ok(())
    }
}
