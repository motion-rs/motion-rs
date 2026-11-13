//! Fixed vector, inline image and glyph particle source data.

use super::ParticleVectorPathSpec;
use crate::content::PointsSpec;
use motion_core::Color;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Painted vector surface sampled uniformly by area.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParticleSurfaceSpec {
    /// Local bounds `[left, top, right, bottom]`.
    pub bounds: [f64; 4],
    /// Fixed painted paths; at most 256.
    pub paths: Vec<ParticleVectorPathSpec>,
    /// Retains source paint without changing emission density.
    #[serde(default)]
    pub inherit_color: bool,
}

/// Explicit inline RGBA8 pixels, avoiding implicit file access.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParticleImageSpec {
    /// Pixel width and height; each in 1 to 512.
    pub size: [u32; 2],
    /// Row-major straight sRGB RGBA8 bytes; exactly width times height times four.
    pub rgba: Vec<u8>,
}

/// Image-weighted particle emission with inherited pixel colors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParticleImageSourceSpec {
    /// Explicit source pixels.
    pub image: ParticleImageSpec,
    /// Local sampling bounds `[left, top, right, bottom]`.
    pub bounds: [f64; 4],
    /// Uses linear luminance instead of alpha for birth density.
    #[serde(default)]
    pub luminance: bool,
}

/// Glyph outlines sampled from explicitly supplied fonts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParticleTextSourceSpec {
    /// Text content.
    pub content: String,
    /// Font family; the supplied default font when empty.
    #[serde(default)]
    pub family: String,
    /// Font size in design units.
    pub size: f64,
    /// Fixed glyph fill.
    pub fill: Color,
}

/// Fixed or compile-time sampled particle target points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ParticlePointsSpec {
    /// Explicit point list.
    Fixed(Vec<[f64; 2]>),
    /// Existing geometry sampling protocol; per-frame animated points are rejected.
    Sampled(PointsSpec),
}
