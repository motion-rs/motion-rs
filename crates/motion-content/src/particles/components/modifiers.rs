//! Force, damping, collision and lifetime-curve modifiers for particle simulations.

use super::*;
use motion_core::{Rect, Vector2};

/// Constant acceleration, applied before semi-implicit integration.
#[derive(Clone, Copy, Debug)]
pub struct Gravity {
    /// Acceleration in scene units per second squared.
    pub acceleration: Vector2,
}

impl Gravity {
    /// Creates a constant acceleration.
    pub fn new(acceleration: Vector2) -> Self {
        Self { acceleration }
    }
}

/// Constant wind acceleration, independent of elapsed scene time.
#[derive(Clone, Copy, Debug)]
pub struct Wind {
    /// Acceleration in scene units per second squared.
    pub acceleration: Vector2,
}

impl Wind {
    /// Creates a constant wind acceleration.
    pub fn new(acceleration: Vector2) -> Self {
        Self { acceleration }
    }
}

macro_rules! force {
    ($name:ident) => {
        impl ParticleModifier for $name {
            fn apply(
                &self,
                particle: &mut Particle,
                step: f32,
                _: Time,
                _: &mut Random,
            ) -> Result<(), ValidationError> {
                particle.velocity[0] += self.acceleration.x as f32 * step;
                particle.velocity[1] += self.acceleration.y as f32 * step;
                particle.validate()
            }
        }
    };
}
force!(Gravity);
force!(Wind);

/// Per-step exponential velocity damping; simulation operator order is explicit.
#[derive(Clone, Copy, Debug)]
pub struct Drag {
    /// Exponential damping rate per second; finite and nonnegative.
    pub coefficient: f64,
}

impl Drag {
    /// Creates a damping rate.
    pub fn new(coefficient: f64) -> Self {
        Self { coefficient }
    }
}

impl ParticleModifier for Drag {
    fn apply(
        &self,
        particle: &mut Particle,
        step: f32,
        _: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        if !self.coefficient.is_finite() || self.coefficient < 0.0 {
            return Err(invalid("drag", "Drag must be finite and nonnegative"));
        }
        let damping = libm::exp(-self.coefficient * f64::from(step)) as f32;
        particle.velocity = particle.velocity.map(|value| value * damping);
        Ok(())
    }
}

/// Tangential acceleration about a softened point vortex.
#[derive(Clone, Copy, Debug)]
pub struct Vortex {
    /// Vortex center.
    pub center: Point,
    /// Tangential acceleration scale.
    pub strength: f64,
}

impl ParticleModifier for Vortex {
    fn apply(
        &self,
        particle: &mut Particle,
        step: f32,
        _: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        let horizontal = f64::from(particle.position[0]) - self.center.x;
        let vertical = f64::from(particle.position[1]) - self.center.y;
        let distance = libm::hypot(horizontal, vertical).max(1.0);
        particle.velocity[0] += (-vertical / distance * self.strength * f64::from(step)) as f32;
        particle.velocity[1] += (horizontal / distance * self.strength * f64::from(step)) as f32;
        particle.validate()
    }
}

/// Softened inverse-square attraction without singular acceleration at the center.
#[derive(Clone, Copy, Debug)]
pub struct Attractor {
    /// Attractor location.
    pub center: Point,
    /// Acceleration-strength coefficient.
    pub strength: f64,
}

impl ParticleModifier for Attractor {
    fn apply(
        &self,
        particle: &mut Particle,
        step: f32,
        _: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        let horizontal = self.center.x - f64::from(particle.position[0]);
        let vertical = self.center.y - f64::from(particle.position[1]);
        let distance = libm::hypot(horizontal, vertical).max(1.0);
        let amount = self.strength / (distance * distance * distance) * f64::from(step);
        particle.velocity[0] += (horizontal * amount) as f32;
        particle.velocity[1] += (vertical * amount) as f32;
        particle.validate()
    }
}

/// Stable-index seeking to a point set, for example VectorTree::sample_points.
#[derive(Clone, Debug)]
pub struct Seek {
    /// Immutable target points.
    pub targets: Arc<[Point]>,
    /// Velocity correction coefficient per second.
    pub strength: f64,
}

impl ParticleModifier for Seek {
    fn apply(
        &self,
        particle: &mut Particle,
        step: f32,
        _: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        if self.targets.is_empty()
            || self.targets.len() > 65536
            || !self.strength.is_finite()
            || self.strength < 0.0
        {
            return Err(invalid("seek", "Invalid target set/strength"));
        }
        let target = self.targets[particle.index as usize % self.targets.len()];
        let amount = (self.strength * f64::from(step)).min(1.0);
        for (axis, value) in [target.x, target.y].into_iter().enumerate() {
            let desired = value - f64::from(particle.position[axis]);
            particle.velocity[axis] =
                (f64::from(particle.velocity[axis]) * (1.0 - amount) + desired * amount) as f32;
        }
        particle.validate()
    }
}

/// Reflects predicted next positions against a rectangle. Place last when other
/// operators change velocity; arbitrary multiple wall crossings are supported.
#[derive(Clone, Copy, Debug)]
pub struct BounceBox {
    /// Rectangle the particles bounce inside.
    pub bounds: Rect,
}

impl BounceBox {
    /// Creates a reflecting rectangle.
    pub fn new(bounds: Rect) -> Self {
        Self { bounds }
    }
}

impl ParticleModifier for BounceBox {
    fn apply(
        &self,
        particle: &mut Particle,
        step: f32,
        _: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        motion_core::drawing::validate_rect(self.bounds, "particles", "bounce_box")?;
        for (axis, (low, high)) in [
            (self.bounds.left, self.bounds.right),
            (self.bounds.top, self.bounds.bottom),
        ]
        .into_iter()
        .enumerate()
        {
            let extent = high - low;
            if extent <= 0.0 {
                return Err(invalid(
                    "bounce",
                    "Bounce box must have positive dimensions",
                ));
            }
            let next = f64::from(particle.position[axis])
                + f64::from(particle.velocity[axis]) * f64::from(step);
            let phase = (next - low).rem_euclid(extent * 2.0);
            let reflected = if phase <= extent {
                low + phase
            } else {
                high - (phase - extent)
            };
            if phase > extent {
                particle.velocity[axis] = -particle.velocity[axis];
            }
            particle.position[axis] =
                (reflected - f64::from(particle.velocity[axis]) * f64::from(step)) as f32;
        }
        particle.validate()
    }
}

/// Piecewise-linear linear-premultiplied RGBA over normalized lifetime.
#[derive(Clone, Debug)]
pub struct ColorOverLifetime {
    /// Evenly spaced color stops, one to 256.
    pub stops: Arc<[[f32; 4]]>,
}

impl ColorOverLifetime {
    /// Creates a color ramp from evenly spaced stops.
    pub fn new(stops: impl Into<Arc<[[f32; 4]]>>) -> Self {
        Self {
            stops: stops.into(),
        }
    }
}

impl ParticleModifier for ColorOverLifetime {
    fn apply(
        &self,
        particle: &mut Particle,
        _: f32,
        _: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        if self.stops.is_empty() || self.stops.len() > 256 {
            return Err(invalid("colors", "Require 1..=256 color stops"));
        }
        for color in self.stops.iter() {
            Particle {
                color: *color,
                ..Default::default()
            }
            .validate()?;
        }
        let position =
            (particle.age / particle.lifetime).clamp(0.0, 1.0) * (self.stops.len() - 1) as f32;
        let lower = libm::floorf(position) as usize;
        let upper = (lower + 1).min(self.stops.len() - 1);
        let amount = position - lower as f32;
        particle.color = std::array::from_fn(|channel| {
            self.stops[lower][channel] * (1.0 - amount) + self.stops[upper][channel] * amount
        });
        particle.validate()
    }
}

/// Linear nonnegative size multiplier over normalized lifetime.
#[derive(Clone, Copy, Debug)]
pub struct SizeOverLifetime {
    /// Birth multiplier.
    pub start: f64,
    /// End-of-life multiplier.
    pub end: f64,
}

impl ParticleModifier for SizeOverLifetime {
    fn apply(
        &self,
        particle: &mut Particle,
        _: f32,
        _: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        if !self.start.is_finite()
            || !self.end.is_finite()
            || !(0.0..=4096.0).contains(&self.start)
            || !(0.0..=4096.0).contains(&self.end)
        {
            return Err(invalid("size", "Size stops must be finite in 0..=4096"));
        }
        let progress = f64::from(particle.age / particle.lifetime).clamp(0.0, 1.0);
        particle.size = (self.start * (1.0 - progress) + self.end * progress) as f32;
        particle.validate()
    }
}
