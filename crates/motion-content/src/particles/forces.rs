//! Deterministic flow sampling and stable-order population forces.

use super::*;
use motion_core::{Point, Rect};

/// Spatial attenuation with no singular denominator at a force center.
#[derive(Clone)]
pub enum ForceFalloff {
    /// Constant strength inside the authored radius.
    Constant,
    /// Linear falloff to zero at the radius.
    Linear,
    /// Softened inverse-square distance.
    InverseSquare,
    /// Explicit distance-to-strength curve.
    Curve(ParticleCurve),
}

/// Radius-bounded attraction/repulsion using an explicit attenuation policy.
#[derive(Clone)]
pub struct RadialForce {
    /// Force center.
    pub center: Point,
    /// Signed acceleration scale.
    pub strength: f32,
    /// Finite positive influence radius.
    pub radius: f32,
    /// Spatial attenuation.
    pub falloff: ForceFalloff,
}

impl ParticleModifier for RadialForce {
    fn apply(
        &self,
        particle: &mut Particle,
        step: f32,
        _: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        if !self.radius.is_finite()
            || self.radius <= 0.0
            || !self.strength.is_finite()
            || !self.center.x.is_finite()
            || !self.center.y.is_finite()
        {
            return Err(invalid(
                "force",
                "Require finite force center/strength and positive radius",
            ));
        }
        let delta = [
            self.center.x as f32 - particle.position[0],
            self.center.y as f32 - particle.position[1],
        ];
        let distance = libm::hypotf(delta[0], delta[1]);
        if distance >= self.radius || distance == 0.0 {
            return Ok(());
        }
        let amount = match &self.falloff {
            ForceFalloff::Constant => 1.0,
            ForceFalloff::Linear => 1.0 - distance / self.radius,
            ForceFalloff::InverseSquare => 1.0 / distance.max(1.0).powi(2),
            ForceFalloff::Curve(curve) => curve.sample(distance / self.radius)?,
        };
        for (velocity, delta) in particle.velocity.iter_mut().zip(delta) {
            *velocity += delta / distance * self.strength * amount * step;
        }
        particle.validate()
    }
}

/// Divergence-free curl of the shared seeded Perlin potential, advected over time.
#[derive(Clone, Copy, Debug)]
pub struct CurlNoise {
    /// Positive spatial frequency.
    pub frequency: f64,
    /// Signed acceleration multiplier.
    pub strength: f64,
    /// Explicit potential seed.
    pub seed: u32,
    /// Potential advection rate per second.
    pub phase_speed: f64,
}

fn shared_curl(
    position: [f32; 2],
    frequency: f64,
    phase: f64,
    seed: u32,
) -> Result<[f64; 2], ValidationError> {
    if !frequency.is_finite() || frequency <= 0.0 || !phase.is_finite() {
        return Err(invalid(
            "curl",
            "Require positive finite frequency and finite phase",
        ));
    }
    let point = position.map(|value| f64::from(value) * frequency + phase);
    if !motion_core::noise::lattice_supported(point) {
        return Err(invalid(
            "curl",
            "Curl coordinates exceed signed 32-bit lattice support",
        ));
    }
    Ok(motion_core::noise::curl2(point, seed))
}

impl ParticleModifier for CurlNoise {
    fn apply(
        &self,
        particle: &mut Particle,
        step: f32,
        time: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        if !self.strength.is_finite()
            || !self.phase_speed.is_finite()
            || !step.is_finite()
            || step < 0.0
        {
            return Err(invalid(
                "curl",
                "Require finite strength, phase speed and nonnegative step",
            ));
        }
        let direction = shared_curl(
            particle.position,
            self.frequency,
            time.as_seconds() * self.phase_speed,
            self.seed,
        )?;
        let velocity: [f32; 2] = std::array::from_fn(|axis| {
            (f64::from(particle.velocity[axis]) + direction[axis] * self.strength * f64::from(step))
                as f32
        });
        if velocity.into_iter().any(|value| !value.is_finite()) {
            return Err(invalid(
                "curl",
                "Curl velocity exceeds finite particle support",
            ));
        }
        particle.velocity = velocity;
        particle.validate()
    }

    fn work(&self) -> u64 {
        96
    }
}

/// Immutable source for an evolving velocity field.
#[derive(Clone)]
pub enum FlowSource {
    /// Two-dimensional curl of the shared analytic Perlin potential.
    Curl {
        /// Explicit stable seed.
        seed: u32,
        /// Positive spatial frequency.
        frequency: f64,
    },
    /// Red/green channels interpreted as a normalized vector map.
    Image {
        /// Immutable premultiplied material whose straight RG encodes vectors.
        image: Arc<crate::ImageAsset>,
        /// Local domain mapped onto the image.
        bounds: Rect,
    },
}

/// Time-evolving noise or image flow, with no mutable frame cursor.
#[derive(Clone)]
pub struct FlowField {
    /// Field material.
    pub source: FlowSource,
    /// Acceleration scale.
    pub strength: f32,
    /// Phase rate per second.
    pub phase_speed: f64,
}

impl FlowField {
    /// Constructs an explicit shared two-dimensional curl source.
    pub fn curl(seed: u32, frequency: f64, strength: f32) -> Self {
        Self {
            source: FlowSource::Curl { seed, frequency },
            strength,
            phase_speed: 1.0,
        }
    }
}

impl ParticleModifier for FlowField {
    fn apply(
        &self,
        particle: &mut Particle,
        step: f32,
        time: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        if !self.strength.is_finite() || !self.phase_speed.is_finite() {
            return Err(invalid("flow", "Require finite strength and phase speed"));
        }
        let phase = time.as_seconds() * self.phase_speed;
        let direction = match &self.source {
            FlowSource::Curl { seed, frequency } => {
                shared_curl(particle.position, *frequency, phase, *seed)?.map(|value| value as f32)
            }
            FlowSource::Image { image, bounds } => {
                motion_core::drawing::validate_rect(*bounds, "particles.flow", "bounds")?;
                if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
                    return Err(invalid("flow", "Require positive flow image bounds"));
                }
                let horizontal = ((f64::from(particle.position[0]) - bounds.left) / bounds.width()
                    + phase)
                    .rem_euclid(1.0);
                let vertical = ((f64::from(particle.position[1]) - bounds.top) / bounds.height())
                    .clamp(0.0, 1.0);
                let sample = image.levels()[0]
                    .pixel(
                        (horizontal * f64::from(image.width())) as u32,
                        ((vertical * f64::from(image.height())) as u32).min(image.height() - 1),
                    )
                    .ok_or_else(|| invalid("flow", "Missing image vector"))?;
                if sample[3] == 0.0 {
                    [0.0; 2]
                } else {
                    [
                        sample[0] / sample[3] * 2.0 - 1.0,
                        sample[1] / sample[3] * 2.0 - 1.0,
                    ]
                }
            }
        };
        for (velocity, direction) in particle.velocity.iter_mut().zip(direction) {
            *velocity += direction * self.strength * step;
        }
        particle.validate()
    }

    fn work(&self) -> u64 {
        if matches!(self.source, FlowSource::Curl { .. }) {
            96
        } else {
            1
        }
    }
}

/// Spatially hashed flocking evaluated from one immutable population snapshot.
#[derive(Clone, Copy, Debug)]
pub struct Boids {
    /// Positive neighbor radius.
    pub radius: f64,
    /// Separation strength.
    pub separation: f32,
    /// Alignment strength.
    pub alignment: f32,
    /// Cohesion strength.
    pub cohesion: f32,
    /// Maximum population admitted by this operator.
    pub max_particles: usize,
}

impl Default for Boids {
    fn default() -> Self {
        Self {
            radius: 32.0,
            separation: 1.0,
            alignment: 0.5,
            cohesion: 0.1,
            max_particles: 4096,
        }
    }
}

impl ParticleGroupModifier for Boids {
    fn apply(
        &self,
        particles: &mut [Particle],
        step: f32,
        _: Time,
        max_work: u64,
    ) -> Result<u64, ValidationError> {
        if self.max_particles == 0
            || particles.len() > self.max_particles
            || [self.separation, self.alignment, self.cohesion]
                .iter()
                .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(limit("Invalid flock parameters or population allowance"));
        }
        let pairs = super::neighbors::pairs(particles, self.radius, max_work / 16)?;
        let work = particles.len() as u64 * 4 + pairs.len() as u64 * 32;
        if work > max_work {
            return Err(limit("Flock work exhausted"));
        }
        let mut forces = vec![[0.0_f32; 6]; particles.len()];
        let mut counts = vec![0u32; particles.len()];
        for (first, second) in pairs {
            for (owner, other) in [(first, second), (second, first)] {
                let difference = [
                    particles[owner].position[0] - particles[other].position[0],
                    particles[owner].position[1] - particles[other].position[1],
                ];
                let square =
                    (difference[0] * difference[0] + difference[1] * difference[1]).max(1.0);
                for axis in 0..2 {
                    forces[owner][axis] += difference[axis] / square;
                    forces[owner][axis + 2] += particles[other].velocity[axis];
                    forces[owner][axis + 4] += particles[other].position[axis];
                }
                counts[owner] += 1;
            }
        }
        for (index, particle) in particles.iter_mut().enumerate() {
            if counts[index] == 0 {
                continue;
            }
            let count = counts[index] as f32;
            for axis in 0..2 {
                particle.velocity[axis] += step
                    * (forces[index][axis] * self.separation
                        + (forces[index][axis + 2] / count - particle.velocity[axis])
                            * self.alignment
                        + (forces[index][axis + 4] / count - particle.position[axis])
                            * self.cohesion);
            }
            particle.validate()?;
        }
        Ok(work)
    }
}

/// Consecutive stable IDs coupled by Hooke springs, optionally forming a closed chain.
#[derive(Clone, Copy, Debug)]
pub struct Spring {
    /// Rest distance between consecutive records.
    pub rest_length: f32,
    /// Nonnegative spring acceleration coefficient.
    pub stiffness: f32,
    /// Nonnegative damping coefficient along the connection.
    pub damping: f32,
    /// Connect the last record back to the first.
    pub closed: bool,
}

impl ParticleGroupModifier for Spring {
    fn apply(
        &self,
        particles: &mut [Particle],
        step: f32,
        _: Time,
        max_work: u64,
    ) -> Result<u64, ValidationError> {
        if [self.rest_length, self.stiffness, self.damping]
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(invalid(
                "spring",
                "Require finite nonnegative spring parameters",
            ));
        }
        let work = particles.len() as u64 * 32;
        if work > max_work {
            return Err(limit("Spring work exhausted"));
        }
        let mut order = (0..particles.len()).collect::<Vec<_>>();
        order.sort_by_key(|index| particles[*index].index);
        let mut forces = vec![[0.0_f32; 2]; particles.len()];
        let count = order.len().saturating_sub(1) + usize::from(self.closed && order.len() > 2);
        for index in 0..count {
            let first = order[index];
            let second = order[(index + 1) % order.len()];
            let delta = [
                particles[second].position[0] - particles[first].position[0],
                particles[second].position[1] - particles[first].position[1],
            ];
            let distance = libm::hypotf(delta[0], delta[1]);
            if distance == 0.0 {
                continue;
            }
            let normal = delta.map(|value| value / distance);
            let speed = (particles[second].velocity[0] - particles[first].velocity[0]) * normal[0]
                + (particles[second].velocity[1] - particles[first].velocity[1]) * normal[1];
            let amount = (distance - self.rest_length) * self.stiffness + speed * self.damping;
            for axis in 0..2 {
                forces[first][axis] += normal[axis] * amount;
                forces[second][axis] -= normal[axis] * amount;
            }
        }
        for (particle, force) in particles.iter_mut().zip(forces) {
            for (velocity, force) in particle.velocity.iter_mut().zip(force) {
                *velocity += force * step;
            }
            particle.validate()?;
        }
        Ok(work)
    }
}

/// Optional equal-mass sphere contacts using stable spatial-hash pair ordering.
#[derive(Clone, Copy, Debug)]
pub struct ParticleCollisions {
    /// Contact restitution/friction; kill_on_hit is supported.
    pub response: CollisionResponse,
}

impl ParticleGroupModifier for ParticleCollisions {
    fn apply(
        &self,
        particles: &mut [Particle],
        _: f32,
        _: Time,
        max_work: u64,
    ) -> Result<u64, ValidationError> {
        self.response.validate()?;
        let radius = particles
            .iter()
            .fold(0.0_f32, |value, particle| value.max(particle.size))
            * 2.0;
        if radius == 0.0 {
            return Ok(particles.len() as u64);
        }
        let pairs = super::neighbors::pairs(particles, f64::from(radius), max_work / 16)?;
        let work = particles.len() as u64 + pairs.len() as u64 * 32;
        if work > max_work {
            return Err(limit("Particle contact work exhausted"));
        }
        for (first, second) in pairs {
            let delta = [
                particles[second].position[0] - particles[first].position[0],
                particles[second].position[1] - particles[first].position[1],
            ];
            let distance = libm::hypotf(delta[0], delta[1]);
            let minimum = particles[first].size + particles[second].size;
            if distance >= minimum {
                continue;
            }
            let normal = if distance == 0.0 {
                [1.0, 0.0]
            } else {
                delta.map(|value| value / distance)
            };
            let relative = [
                particles[second].velocity[0] - particles[first].velocity[0],
                particles[second].velocity[1] - particles[first].velocity[1],
            ];
            let speed = relative[0] * normal[0] + relative[1] * normal[1];
            for axis in 0..2 {
                let correction = normal[axis] * (minimum - distance + 1e-5) * 0.5;
                particles[first].position[axis] -= correction;
                particles[second].position[axis] += correction;
                if speed < 0.0 {
                    let impulse = normal[axis] * speed * (1.0 + self.response.restitution) * 0.5;
                    let friction =
                        (relative[axis] - speed * normal[axis]) * self.response.friction * 0.5;
                    particles[first].velocity[axis] += impulse + friction;
                    particles[second].velocity[axis] -= impulse + friction;
                }
            }
            if self.response.kill_on_hit {
                for index in [first, second] {
                    particles[index].lifetime = particles[index].age.max(f32::MIN_POSITIVE);
                }
            }
        }
        for particle in particles {
            particle.validate()?;
        }
        Ok(work)
    }

    fn collisions(&self) -> bool {
        true
    }
}
