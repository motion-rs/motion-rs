//! Serializable spring, path, mapping and clock operations for document animations.

use crate::{Animated, EasingSpec};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A closed-form spring using the core defaults for omitted parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpringSpec<T> {
    /// Initial value.
    pub from: T,
    /// Target value.
    pub to: T,
    /// Positive mass; one by default.
    pub mass: Option<f64>,
    /// Positive stiffness; 100 by default.
    pub stiffness: Option<f64>,
    /// Positive damping; 10 by default.
    pub damping: Option<f64>,
    /// Initial velocity in endpoint distances per second; zero by default.
    pub initial_velocity: Option<f64>,
    /// Relative settling threshold in `(0, 1)`; 0.001 by default.
    pub settling_threshold: Option<f64>,
}

impl<T> SpringSpec<T> {
    /// Creates a spring with core defaults.
    pub fn new(from: T, to: T) -> Self {
        Self {
            from,
            to,
            mass: None,
            stiffness: None,
            damping: None,
            initial_velocity: None,
            settling_threshold: None,
        }
    }
}

/// Samples a source at `clamp(t * scale + offset, -1e6, 1e6)` seconds.
///
/// Scale and offset must be finite with magnitude at most 1e6. Negative scales
/// reverse the clock. Remapping includes expressions and their bindings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    deny_unknown_fields,
    bound(deserialize = "T: serde::de::DeserializeOwned")
)]
pub struct ClockSpec<T> {
    /// Source animation.
    pub source: Box<Animated<T>>,
    /// Clock multiplier.
    pub scale: f64,
    /// Signed clock offset in seconds.
    pub offset: f64,
}

/// Maps a numeric source through a bounded arithmetic expression.
///
/// Here `t` denotes the sampled input value, not seconds. Numeric document
/// parameters are supported; element bindings are not supported in the mapping.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MapSpec {
    /// Source animation.
    pub source: Box<Animated<f64>>,
    /// Mapping expression.
    pub expr: String,
}

/// Numeric component of an arc-length path sample.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PathComponent {
    /// Local X position.
    X,
    /// Local Y position.
    Y,
    /// Clockwise tangent angle in degrees.
    Angle,
}

/// A core FollowPath animation projected to one numeric component.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FollowPathSpec {
    /// SVG path data.
    pub data: String,
    /// Nonnegative duration in seconds.
    pub duration: f64,
    /// Component to sample.
    pub component: PathComponent,
    /// Distance easing; linear by default.
    #[serde(default)]
    pub easing: EasingSpec,
}

impl FollowPathSpec {
    /// Creates linear path motion over `duration` seconds.
    pub fn new(data: impl Into<String>, duration: f64, component: PathComponent) -> Self {
        Self {
            data: data.into(),
            duration,
            component,
            easing: EasingSpec::Linear,
        }
    }
}
