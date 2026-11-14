//! Declarative brush geometry, recorded pen input and deterministic ink materials.

use crate::{Animated, PaintSpec, ParticleImageSpec, PointsSpec};
use motion_content::{BrushCap, NibMode, PigmentMix};
use motion_core::{Color, Point};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A pressure-sensitive stroke with exactly one of `path`, `points` or `samples`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrushStrokeSpec {
    /// Nib width in design units, in 0..=512.
    pub width: f64,
    /// Stroke color or gradient, applied before material attenuation.
    pub paint: PaintSpec,
    /// Fixed SVG path data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Fixed or compile-time sampled polyline points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<PointsSpec>,
    /// Recorded input, with 2..=512 strictly increasing timestamps starting at zero.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub samples: Vec<InputSampleSpec>,
    /// Start, middle and end pressure in 0..=4; defaults to one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressure: Option<[f64; 3]>,
    /// Ordered progress/pressure pairs spanning 0..=1; at most 512.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pressure_curve: Vec<(f64, f64)>,
    /// Analytic pressure with `t` interpreted as normalized arc length, not scene time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressure_from: Option<String>,
    /// Elliptical minor/major nib ratio, in (0,1]; defaults to one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nib_aspect: Option<f64>,
    /// Fixed or tangent-following clockwise degrees, or recorded tilt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nib_mode: Option<NibMode>,
    /// Start and end endpoint treatment; defaults to round.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caps: Option<[BrushCap; 2]>,
    /// Start and end ink supply in 0..=1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ink_load: Option<[f64; 2]>,
    /// Deterministic material; absent means an ordinary geometric brush.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ink: Option<InkSpec>,
    /// Seeded offset bristles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bristles: Option<BristlesSpec>,
    /// Seeded path displacement, optionally changing every two explicit frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wobble: Option<WobbleSpec>,
    /// Analytic wash expansion from explicit age, not playback history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wetness: Option<WetnessSpec>,
    /// Geometry sampling intervals in 2..=512; defaults to 256.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_limit: Option<usize>,
    /// Material distance-evaluation allowance; defaults to 32 million.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material_work_limit: Option<u64>,
    /// Pressure response to speed, in 0..=1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_response: Option<f64>,
    /// Arc-length reveal in 0..=1; mutually exclusive with `write_time`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<Animated<f64>>,
    /// Explicit nonnegative reveal seconds, using recording or `timing`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_time: Option<Animated<f64>>,
    /// Moving duration and pen-up pauses for `write_time`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<BrushTimingSpec>,
}

/// One pen sample; positions, pressure, azimuth and recording time are explicit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputSampleSpec {
    /// Local design-space position.
    pub position: Point,
    /// Pressure in 0..=4.
    pub pressure: f64,
    /// Clockwise nib azimuth in degrees.
    pub tilt: f64,
    /// Recording seconds in 0..=86400.
    pub time: f64,
}

/// Stateless arc timing with pauses between subpaths.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrushTimingSpec {
    /// Positive moving seconds, excluding pen-up pauses.
    pub duration: f64,
    /// Monotone time/progress pairs spanning (0,0)..=(1,1); linear when omitted.
    #[serde(default)]
    pub curve: Vec<(f64, f64)>,
    /// Nonnegative pause seconds between subpaths.
    #[serde(default)]
    pub lift_pause: f64,
}

/// Bounded seeded bristles sharing one stroke.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BristlesSpec {
    /// Number of bristles, in 1..=64.
    pub count: usize,
    /// Offset spread in design units, in 0..=512.
    pub spread: f64,
    /// Jitter strength in 0..=1.
    pub jitter: f64,
    /// Explicit random seed.
    pub seed: u32,
}

/// Seeded analytic displacement of the complete path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WobbleSpec {
    /// Displacement amplitude in 0..=512 design units.
    pub amplitude: f64,
    /// Positive spatial frequency, at most 100.
    pub frequency: f64,
    /// Explicit random seed.
    pub seed: u32,
    /// Explicit nonnegative frame number for two-frame boil.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<Animated<f64>>,
}

/// Analytic absorption halo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WetnessSpec {
    /// Nonnegative age in seconds.
    pub age: Animated<f64>,
    /// Maximum wash radius in design units.
    pub radius: f32,
    /// Positive drying duration in seconds.
    pub drying: f64,
}

/// Seeded local ink properties; omitted fields retain the content engine defaults.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct InkSpec {
    /// Explicit material seed.
    pub seed: u32,
    /// Dry coverage strength in 0..=1.
    pub dryness: f32,
    /// Nonnegative soft wash radius.
    pub bleed: f32,
    /// Nonnegative geometric edge displacement.
    pub edge_roughness: f32,
    /// Coverage mottling in 0..=1.
    pub granulation: f32,
    /// Path-following dry fibers in 0..=1; stroke-only.
    pub fibers: f32,
    /// Explicit procedural or inline texture paper.
    pub paper: Option<PaperSpec>,
    /// Finite local-to-paper affine mapping; identity when omitted.
    pub paper_transform: Option<[f64; 6]>,
    /// Inner edge deposit `[strength, width]`.
    pub edge_pooling: Option<[f32; 2]>,
    /// Added intersection density in 0..=1; stroke-only.
    pub build_up: f64,
    /// RGB rim darkening in 0..=1.
    pub edge_darkening: f64,
    /// Directional coordinate ratio in 0.1..=10; defaults to one.
    pub edge_anisotropy: Option<f64>,
    /// Grayscale ridge strength in 0..=16; stroke-only.
    pub height_map: f64,
}

/// Explicit paper descriptions; neither variant performs implicit IO.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PaperSpec {
    /// Seeded noise attenuation.
    Procedural {
        /// Explicit paper seed.
        seed: u32,
        /// Coverage attenuation in 0..=1.
        strength: f32,
        /// Positive paper-space cell size.
        scale: f32,
    },
    /// Repeating inline height image using the shared bounded RGBA8 image protocol.
    Texture {
        /// Explicit image pixels, with sides at most 512.
        image: ParticleImageSpec,
        /// Positive paper-space units per texel.
        grain_scale: f64,
        /// Luminance attenuation in 0..=1.
        absorbency: f64,
    },
    /// Seeded RGBA8 height texture generated once during compilation.
    NoiseTexture {
        /// Pixel width and height, each in 1..=512.
        size: [u32; 2],
        /// Explicit integer hash seed.
        seed: u32,
        /// Positive paper-space units per texel.
        grain_scale: f64,
        /// Luminance attenuation in 0..=1.
        absorbency: f64,
    },
}

/// An even-odd contour fill with a bounded deterministic ink material.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InkFillSpec {
    /// SVG path data.
    pub path: String,
    /// Color or gradient before material attenuation.
    pub paint: PaintSpec,
    /// Deterministic ink properties.
    pub ink: InkSpec,
}

/// Animated color mixing through the native ink pigment policy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PigmentSpec {
    /// First pigment.
    pub first: Color,
    /// Second pigment.
    pub second: Color,
    /// Mixing amount in 0..=1.
    pub amount: Animated<f64>,
    /// Native mixing approximation.
    pub mode: PigmentMix,
}
