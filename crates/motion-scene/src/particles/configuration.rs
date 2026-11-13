//! Data-only simulation timing and batch presentation configuration.

use crate::value::EasingSpec;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Optional simulation time mapping and execution policy.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ParticleConfigurationSpec {
    /// Prepared lead-in in seconds; zero by default.
    pub prewarm: f64,
    /// Two-phase loop period in seconds; no loop by default.
    pub loop_period: Option<f64>,
    /// Query mapping `[scale, offset_seconds]`; identity by default.
    pub time_remap: Option<[f64; 2]>,
    /// Explicit opt-in for negative query scales.
    pub allow_reverse: bool,
    /// Fixed worker count, 1 to 64; engine default when omitted.
    pub workers: Option<u8>,
    /// Simulation preparation and query work allowance; engine default when omitted.
    pub max_work: Option<u64>,
    /// Historical segment, taper and smoothing options.
    pub trails: Option<ParticleTrailSpec>,
    /// Velocity elongation `[scale, maximum_aspect]`.
    pub stretch: Option<[f32; 2]>,
    /// Depth displacement `[x, y]`, enabling stable depth sorting.
    pub parallax: Option<[f32; 2]>,
    /// Age-dependent quad flipping; disabled by default.
    pub flip: bool,
}

/// Bounded normalized scalar curve for particle appearance and force falloff.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParticleCurveSpec {
    /// Strictly increasing normalized positions and nonnegative values; at most 256.
    pub stops: Vec<[f32; 2]>,
    /// Interpolation easing; linear by default.
    #[serde(default)]
    pub easing: EasingSpec,
}

/// Shared historical trail geometry options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParticleTrailSpec {
    /// Number of historical intervals, 1 to 64.
    pub segments: u8,
    /// Width curve along the historical interval.
    #[serde(default)]
    pub taper: Option<ParticleCurveSpec>,
    /// Premultiplied fade curve along the historical interval.
    #[serde(default)]
    pub fade: Option<ParticleCurveSpec>,
    /// Bounded Catmull-Rom interpolation.
    #[serde(default)]
    pub smooth: bool,
    /// Continuous flat-ended ribbon geometry.
    #[serde(default)]
    pub ribbon: bool,
}
