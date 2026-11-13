//! Explicit procedural sprite materials and stateless atlas frame selection.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One deterministic generated sprite frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParticleSpriteSpec {
    /// Procedural material.
    pub material: ParticleMaterialSpec,
    /// Square frame side in texels, 2 to 256; all frames share a 4 MiB atlas budget.
    pub size: u32,
    /// Explicit material sample time in seconds; zero by default.
    #[serde(default)]
    pub time: f64,
}

/// Procedural materials with no external file dependencies.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ParticleMaterialSpec {
    /// Radial soft disc.
    SoftCircle,
    /// Seeded default-fractal smoke.
    Smoke {
        /// Spatial scale in normalized coordinates.
        scale: f64,
        /// Time evolution speed.
        evolution: f64,
    },
    /// Cross-shaped star.
    Star,
    /// Soft annular highlight.
    Ring,
    /// Elongated warm fire core.
    FireCore,
}

/// Stateless sprite-sheet frame policy.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParticleSheetSpec {
    /// Normalized age traverses one sheet pass.
    OverLifetime,
    /// Loops at explicit frames per second.
    Fps(f64),
    /// Stable-ID selection using this explicit seed.
    Random(u64),
    /// Selects using custom slot zero.
    Custom,
}
