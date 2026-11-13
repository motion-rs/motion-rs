//! Plain content policies available without optional text or particle preparation.

/// Paragraph alignment, with start/end resolved by the paragraph direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum TextAlignment {
    /// Follow paragraph start.
    #[default]
    Start,
    /// Follow paragraph end.
    End,
    /// Center each line within the requested width.
    Center,
    /// Justify nonfinal lines.
    Justified,
}

/// Units used for visibility and per-part animation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum TextSplit {
    /// Extended Unicode grapheme clusters, not scalar chars.
    #[default]
    Characters,
    /// Unicode words, with separators attached to the preceding word.
    Words,
    /// Wrapped visual lines.
    Lines,
}

/// Curated, independently configurable particle recipes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Preset {
    /// Tapered HDR ballistic burst.
    Sparks,
    /// Cooling, upward-moving glowing embers.
    Embers,
    /// Slow depth-aware dust.
    Dust,
    /// Soft floating motes.
    Motes,
    /// Three parallax depth layers of wind-driven snow.
    Snow,
    /// Stretched rain with collision-triggered splashes.
    Rain,
    /// Expanding procedural smoke sprites.
    Smoke,
    /// Buoyant procedural fire cores.
    Fire,
    /// Flipping, colored falling quads.
    Confetti,
    /// Rocket death, spark burst, then secondary spark deaths.
    Fireworks,
    /// Drifting and breathing light points.
    Fireflies,
    /// Color-preserving vector surface dispersal.
    LogoDissolve,
    /// Color-preserving vector surface convergence.
    LogoAssemble,
}

/// Scalar coordinate used by a particle color field.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum FieldCoordinate {
    /// Horizontal position.
    X,
    /// Vertical position.
    Y,
    /// Velocity magnitude.
    Speed,
}

/// Bounded normal restitution and tangential friction.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CollisionResponse {
    /// Normal velocity restitution in 0..=1.
    pub restitution: f32,
    /// Fraction of tangential velocity removed in 0..=1.
    pub friction: f32,
    /// Remove the particle at its first contact.
    pub kill_on_hit: bool,
}

impl Default for CollisionResponse {
    fn default() -> Self {
        Self {
            restitution: 1.0,
            friction: 0.0,
            kill_on_hit: false,
        }
    }
}

/// Fixed-boundary particle event.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ParticleEventKind {
    /// First admission of a live particle.
    Birth,
    /// Half-open lifetime expiry, including kill-on-hit.
    Death,
    /// Geometric or population contact.
    Collision,
    /// Age threshold in seconds, reported at its first simulation boundary.
    AgeReached(f32),
}

/// Parent properties added to an emitter's local child record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Inherit {
    /// Keep emitter-authored values.
    None,
    /// Translate local emission to the event location.
    #[default]
    Position,
    /// Translate and add parent velocity.
    Velocity,
    /// Translate and multiply parent premultiplied color.
    Color,
    /// Translate, add velocity and inherit color.
    All,
}
