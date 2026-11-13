//! Ordered population, collision, field and event particle operators.

use super::{
    EmitterSpec, ParticleCurveSpec, ParticleImageSpec, ParticlePointsSpec, ParticleSurfaceSpec,
};
use crate::value::EasingSpec;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Data-only advanced operators, applied in the simulation modifier order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ParticleOperatorSpec {
    /// Corresponding stable-ID targets from fixed or compile-time sampled points.
    SeekTargets {
        /// Target points, 1 to 65536.
        targets: ParticlePointsSpec,
        /// Velocity-correction strength.
        strength: f64,
    },
    /// Radius-bounded attraction with explicit attenuation.
    RadialForce {
        /// Local center.
        center: [f64; 2],
        /// Signed acceleration scale.
        strength: f32,
        /// Positive influence radius.
        radius: f32,
        /// Spatial attenuation policy.
        falloff: ParticleFalloffSpec,
    },
    /// Image red/green channels interpreted as a vector flow map.
    ImageFlow {
        /// Explicit inline material.
        image: ParticleImageSpec,
        /// Local image domain.
        bounds: [f64; 4],
        /// Acceleration scale.
        strength: f32,
        /// Phase evolution per second.
        phase_speed: f64,
    },
    /// Shared curl flow field.
    CurlFlow {
        /// Explicit seed in `0..=4294967295`.
        seed: u64,
        /// Spatial frequency.
        frequency: f64,
        /// Acceleration scale.
        strength: f32,
    },
    /// Stable-order flocking from an immutable population snapshot.
    Boids {
        /// Neighbor radius.
        radius: f64,
        /// Separation strength.
        separation: f32,
        /// Alignment strength.
        alignment: f32,
        /// Cohesion strength.
        cohesion: f32,
    },
    /// Hooke springs between consecutive stable particle IDs.
    Spring {
        /// Rest distance.
        rest_length: f32,
        /// Spring acceleration coefficient.
        stiffness: f32,
        /// Damping along the connection.
        damping: f32,
        /// Connects the last particle to the first.
        closed: bool,
    },
    /// Equal-mass pairwise contact response.
    ParticleCollisions(ParticleResponseSpec),
    /// Solid circular obstacle.
    CircleCollider {
        /// Obstacle center.
        center: [f64; 2],
        /// Obstacle radius.
        radius: f64,
        /// Contact response.
        response: ParticleResponseSpec,
    },
    /// Finite line-segment obstacle.
    SegmentCollider {
        /// Start point.
        start: [f64; 2],
        /// End point.
        end: [f64; 2],
        /// Contact response.
        response: ParticleResponseSpec,
    },
    /// Prepared solid polygon obstacle.
    PolygonCollider {
        /// Polygon vertices, 3 to 256.
        vertices: Vec<[f64; 2]>,
        /// Contact response.
        response: ParticleResponseSpec,
    },
    /// Budgeted signed-distance grid of an explicit vector surface.
    SdfCollider {
        /// Source geometry and grid domain.
        surface: ParticleSurfaceSpec,
        /// Grid columns and rows, 2 to 512 each.
        resolution: [u32; 2],
        /// Contact response.
        response: ParticleResponseSpec,
    },
    /// Equally spaced premultiplied color stops over normalized age.
    ColorCurve {
        /// Premultiplied linear colors, 1 to 256.
        colors: Vec<[f32; 4]>,
        /// Interpolation easing.
        #[serde(default)]
        easing: EasingSpec,
    },
    /// Position or speed mapped through a linear color ramp.
    ColorField {
        /// Source coordinate.
        coordinate: ParticleCoordinateSpec,
        /// Normalization interval.
        range: [f32; 2],
        /// Premultiplied linear colors, 1 to 256.
        colors: Vec<[f32; 4]>,
    },
    /// Stable-ID assignment to one custom particle slot.
    CustomSlot {
        /// Slot index, 0 to 2; slot 3 is reserved for the root-only generation marker.
        slot: u8,
        /// Optional positive ID modulus before scaling.
        modulus: Option<u32>,
        /// Multiplier applied to the stable ID or its remainder.
        scale: f32,
    },
}

/// Spatial force attenuation policy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParticleFalloffSpec {
    /// Constant inside the radius.
    Constant,
    /// Linear decay to zero.
    Linear,
    /// Softened inverse-square distance.
    InverseSquare,
    /// Explicit normalized distance-to-strength curve.
    Curve(ParticleCurveSpec),
}

/// Source coordinate for particle color fields.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParticleCoordinateSpec {
    /// Horizontal position.
    X,
    /// Vertical position.
    Y,
    /// Velocity magnitude.
    Speed,
}

/// Finite normal and tangential particle contact policy.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParticleResponseSpec {
    /// Restitution in 0 to 1.
    pub restitution: f32,
    /// Tangential friction in 0 to 1.
    pub friction: f32,
    /// Removes the root particle on contact.
    pub kill_on_hit: bool,
}

/// Explicit nonrecursive event-triggered particle births.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParticleSubEmitterSpec {
    /// Trigger event.
    pub on: ParticleEventSpec,
    /// Local emitter definition.
    pub emitter: EmitterSpec,
    /// Parent-property inheritance.
    pub inherit: ParticleInheritSpec,
    /// Probability in 0 to 1.
    pub probability: f32,
    /// Only particles with custom slot three equal to zero may trigger this rule.
    #[serde(default)]
    pub root_only: bool,
}

/// Fixed-boundary event trigger.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParticleEventSpec {
    /// First live admission.
    Birth,
    /// Half-open lifetime expiry.
    Death,
    /// Geometric contact.
    Collision,
    /// First boundary at the specified absolute age in seconds.
    AgeReached(f32),
}

/// Parent properties inherited by local event births.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParticleInheritSpec {
    /// Keeps local emitter properties.
    None,
    /// Adds the parent's position.
    Position,
    /// Adds position and velocity.
    Velocity,
    /// Adds position and multiplies color.
    Color,
    /// Adds position, velocity and color.
    All,
}
