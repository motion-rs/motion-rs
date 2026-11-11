//! Subtree filters, clips and masks.

use crate::content::CircleSpec;
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
        strength: f64,
        /// Positive radius in design units.
        radius: f64,
    },
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
    pub intensity: Option<f64>,
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

/// Clip geometry, written as `{"rect": [l, t, r, b]}`, `{"circle": {...}}` or `{"path": "M..."}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ClipShapeSpec {
    /// Rectangle `[left, top, right, bottom]`.
    Rect(Animated<Rect>),
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
