//! Deterministic fixed-step simulations and coupled analytic particles.
//!
//! ```
//! use motion_content::prelude::*;
//! use motion_core::prelude::*;
//! use motion_core::{
//!     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
//!     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
//! };
//! use motion_render::{Composition, RenderPrepare, RenderOptions, BackendKind, render_still};
//! use motion_content::particles::*;
//! let cache = SceneCache::new(16);
//! let context = SceneContext::new(
//!     50.milliseconds().after_origin(),
//!     fps!(30),
//!     Size::new(16.0, 16.0),
//!     &cache,
//! )
//! .with_clip_duration(100.milliseconds());
//! let simulation = context.try_cache(("sparks", 42_u64, 100_i64), || {
//!     ParticleSimulation::new(42)
//!         .duration(100.milliseconds())
//!         .emitter(BurstEmitter::new(Time::ZERO, 4, Point::new(8.0, 8.0)).speed(0.0))
//!         .modifier(Gravity::new(Vector2::new(0.0, 10.0)))
//!         .style(ParticleStyle::Dots { radius: 2.0 })
//!         .build()
//! })?;
//! assert_eq!(simulation.sample(50.milliseconds().after_origin())?.len(), 4);
//! let frame = simulation
//!     .try_render(&context)?
//!     .compile(
//!         context.size(),
//!         PixelSize::new(16, 16),
//!         ValidationLimits::default(),
//!     )?
//!     .prepare(ValidationLimits::default(), RasterLimits::default())?
//!     .render_linear(RasterLimits::default())?;
//! assert!(frame.linear_pixel(8, 8).unwrap()[3] > 0.0);
//! # Ok::<(), motion_core::Error>(())
//! ```

use motion_core::{Duration, Random, Time, ValidationError};
use std::sync::{Arc, OnceLock};

mod batch;
mod collision;
mod components;
mod curves;
mod events;
mod extended;
mod forces;
mod lazy;
mod neighbors;
pub mod presets;
mod render;
mod simulation;
mod sprites;
mod style;
#[cfg(test)]
mod tests;
mod trajectory;

pub use batch::ParticleBatch;
pub use collision::{
    CircleCollider, CollisionResponse, PolygonCollider, SdfCollider, SegmentCollider,
    SignedDistanceGrid,
};
pub use components::{
    Attractor, BounceBox, BurstEmitter, ColorOverLifetime, DiskEmitter, Drag, Gravity,
    ImageDensity, ImageEmitter, PathEmitter, PointEmitter, RectEmitter, Seek, SizeOverLifetime,
    SurfaceEmitter, Vortex, Wind,
};
pub use curves::{
    ColorCurve, ColorField, FieldCoordinate, IntensityOverLifetime, ParticleCurve, Temperature,
};
pub use events::{Inherit, ParticleEvent, ParticleEventKind, SubEmitter};
pub use forces::{
    Boids, CurlNoise, FlowField, FlowSource, ForceFalloff, ParticleCollisions, RadialForce, Spring,
};
pub use simulation::{
    ParticleSimulation, ParticleSimulationBuilder, SimulationLimits, SimulationStats,
};
pub use sprites::{ProceduralSprite, SpriteAtlas, SpriteSheet, SpriteSheetMode};
pub use style::{
    BokehShape, ParticleBlend, ParticleShape, ParticleStyle, SpeedStretch, StyleOptions,
    TrailOptions,
};
pub use trajectory::{Ballistic, Converge, NoiseDrift, Orbit, ParticleTrajectory, TrajectoryModel};

use motion_core::instances::MAX_INSTANCE_COUNT;

/// Maximum particle size in design units accepted by validation.
const MAX_PARTICLE_SIZE: f32 = 4096.0;

/// Maximum particle lifetime in seconds accepted by validation.
const MAX_LIFETIME_SECONDS: f32 = 3600.0;

/// 72-byte particle record; colors are finite linear-premultiplied RGBA.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "gpu", derive(bytemuck::Pod, bytemuck::Zeroable))]
pub struct Particle {
    /// Local center in design coordinates.
    pub position: [f32; 2],
    /// Units per second.
    pub velocity: [f32; 2],
    /// Linear-premultiplied RGBA, not encoded bytes.
    pub color: [f32; 4],
    /// Nonnegative style size multiplier.
    pub size: f32,
    /// Clockwise radians.
    pub rotation: f32,
    /// Canonical seconds since birth, owned by the simulation.
    pub age: f32,
    /// Positive lifetime seconds, half-open at death.
    pub lifetime: f32,
    /// Stable nonrecycled emission index.
    pub index: u64,
    /// User-owned values retained by snapshots.
    pub custom: [f32; 4],
}

impl Default for Particle {
    fn default() -> Self {
        Self {
            position: [0.0; 2],
            velocity: [0.0; 2],
            color: [1.0; 4],
            size: 1.0,
            rotation: 0.0,
            age: 0.0,
            lifetime: 1.0,
            index: 0,
            custom: [0.0; 4],
        }
    }
}

impl Particle {
    pub(crate) fn validate(&self) -> Result<(), ValidationError> {
        // Simulation steps validate every particle after every modifier, so the finiteness
        // check avoids a chained iterator that does not vectorize.
        let finite = |values: &[f32]| values.iter().all(|value| value.is_finite());
        if !(finite(&self.position)
            && finite(&self.velocity)
            && finite(&self.color)
            && finite(&self.custom)
            && finite(&[self.size, self.rotation, self.age, self.lifetime]))
            || self.size < 0.0
            || self.size > MAX_PARTICLE_SIZE
            || self.age < 0.0
            || self.lifetime <= 0.0
            || self.lifetime > MAX_LIFETIME_SECONDS
            || !(0.0..=1.0).contains(&self.color[3])
            || (self.color[3] == 0.0 && self.color[..3].iter().any(|value| *value != 0.0))
        {
            return Err(invalid(
                "state",
                "Invalid finite particle geometry, lifetime or premultiplied color",
            ));
        }
        Ok(())
    }
}

/// Fixed-step emitter. Callbacks are deterministic and cooperative: output is
/// bounded after return, not a hard allocator sandbox. IDs/age are assigned by
/// the simulation. Random streams are keyed per emitter and step.
pub trait ParticleEmitter: Send + Sync {
    /// Filters event-triggered births. Root emission is unaffected; ordinary emitters accept all events.
    fn accepts_event(&self, _: &ParticleEvent) -> bool {
        true
    }

    /// Appends births at this exact fixed-step boundary.
    fn emit(
        &self,
        step: u64,
        time: Time,
        random: &mut Random,
        output: &mut Vec<Particle>,
    ) -> Result<(), ValidationError>;
}

/// Ordered per-particle simulation operator, followed by velocity integration.
pub trait ParticleModifier: Send + Sync {
    /// Updates one live particle; index and canonical age remain engine-owned.
    fn apply(
        &self,
        particle: &mut Particle,
        step_seconds: f32,
        time: Time,
        random: &mut Random,
    ) -> Result<(), ValidationError>;

    /// Requests birth-color restoration before the ordered modifier chain.
    /// Multiplicative lifetime energy curves use this to avoid frame-count accumulation.
    fn resets_color(&self) -> bool {
        false
    }

    /// Conservative per-particle operator work charged before replay.
    fn work(&self) -> u64 {
        1
    }

    /// Optional predicted-step collision response, preserving the original callback API.
    fn apply_event(
        &self,
        particle: &mut Particle,
        step_seconds: f32,
        time: Time,
        random: &mut Random,
    ) -> Result<bool, ValidationError> {
        self.apply(particle, step_seconds, time, random)?;
        Ok(false)
    }
}

/// A stable-order population operator, evaluated before independent modifiers.
pub trait ParticleGroupModifier: Send + Sync {
    /// Applies a bounded deterministic population update and returns logical work.
    fn apply(
        &self,
        particles: &mut [Particle],
        step: f32,
        time: Time,
        max_work: u64,
    ) -> Result<u64, ValidationError>;

    /// Whether displaced records should emit collision events.
    fn collisions(&self) -> bool {
        false
    }
}

fn invalid(field: &str, message: &str) -> ValidationError {
    ValidationError::invalid("particles", field, message)
}

fn limit(message: &str) -> ValidationError {
    ValidationError::limit("particles", "budget", message)
}
