//! Custom particle emitters and modifiers used by Particle Garden: staged sub-emitter
//! bursts, offset emitters, a trapezoidal rate envelope, solid colliders, boid leashing,
//! cruise speed and the dissolve pull.

use super::{HEIGHT, WIDTH};
use motion::particles::{
    BurstEmitter, FlowField, Particle, ParticleEmitter, ParticleEvent, ParticleModifier,
};
use motion::prelude::*;

/// Birth stage lives in `custom[3]`, so a sub-emitter reacts only to its own parent generation.
pub(super) struct Stage {
    pub(super) parent: f32,
    pub(super) stage: f32,
    pub(super) burst: BurstEmitter,
}

impl ParticleEmitter for Stage {
    fn accepts_event(&self, event: &ParticleEvent) -> bool {
        event.particle.custom[3] == self.parent
    }

    fn emit(
        &self,
        step: u64,
        time: Time,
        random: &mut Random,
        output: &mut Vec<Particle>,
    ) -> Result<(), ValidationError> {
        let start = output.len();
        self.burst.emit(step, time, random, output)?;
        for particle in &mut output[start..] {
            particle.custom[3] = self.stage;
        }
        Ok(())
    }
}

/// Moves another emitter without changing its sampling or random draws.
pub(super) struct Placed {
    pub(super) inner: Box<dyn ParticleEmitter>,
    pub(super) offset: [f32; 2],
}

impl ParticleEmitter for Placed {
    fn emit(
        &self,
        step: u64,
        time: Time,
        random: &mut Random,
        output: &mut Vec<Particle>,
    ) -> Result<(), ValidationError> {
        let start = output.len();
        self.inner.emit(step, time, random, output)?;
        for particle in &mut output[start..] {
            particle.position[0] += self.offset[0];
            particle.position[1] += self.offset[1];
        }
        Ok(())
    }
}

/// Trapezoidal birth-rate envelope: instant onset, linear fade over the last `fade` seconds.
pub(super) struct Pulse {
    pub(super) start: f64,
    pub(super) end: f64,
    pub(super) peak: f64,
    pub(super) fade: f64,
}

impl Animation<f64> for Pulse {
    fn sample(&self, time: Time) -> f64 {
        let seconds = time.as_seconds();
        if seconds < self.start || seconds >= self.end {
            0.0
        } else {
            self.peak * ((self.end - seconds) / self.fade).min(1.0)
        }
    }
}

/// Applies a collider only to the rocket and spark generations. Contact sparks are born
/// on the surface, where a swept test would crawl along it, so they pass through instead.
pub(super) struct Solid<Collider>(pub(super) Collider);

impl<Collider: ParticleModifier> ParticleModifier for Solid<Collider> {
    fn apply(
        &self,
        particle: &mut Particle,
        step: f32,
        time: Time,
        random: &mut Random,
    ) -> Result<(), ValidationError> {
        self.apply_event(particle, step, time, random).map(|_| ())
    }

    fn work(&self) -> u64 {
        self.0.work()
    }

    fn apply_event(
        &self,
        particle: &mut Particle,
        step: f32,
        time: Time,
        random: &mut Random,
    ) -> Result<bool, ValidationError> {
        if particle.custom[3] >= 2.0 {
            return Ok(false);
        }
        self.0.apply_event(particle, step, time, random)
    }
}

/// Soft pull toward a Lissajous point so the flock wanders across the frame.
pub(super) struct Leash {
    pub(super) strength: f64,
}

impl ParticleModifier for Leash {
    fn apply(
        &self,
        particle: &mut Particle,
        step: f32,
        time: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        let seconds = time.as_seconds();
        let target = [
            WIDTH * 0.5 + 250.0 * math::sin(seconds * 0.8),
            HEIGHT * 0.5 + 80.0 * math::sin(seconds * 1.4 + 1.0),
        ];
        for ((velocity, position), target) in particle
            .velocity
            .iter_mut()
            .zip(particle.position)
            .zip(target)
        {
            *velocity += ((target - f64::from(position)) * self.strength) as f32 * step;
        }
        Ok(())
    }
}

/// Eases speed toward a cruise value; flocking forces alone let it drift away.
pub(super) struct Cruise {
    pub(super) speed: f32,
    pub(super) rate: f32,
}

impl ParticleModifier for Cruise {
    fn apply(
        &self,
        particle: &mut Particle,
        step: f32,
        _: Time,
        _: &mut Random,
    ) -> Result<(), ValidationError> {
        let speed = math::hypotf(particle.velocity[0], particle.velocity[1]).max(1e-3);
        let scale = 1.0 + (self.speed / speed - 1.0) * (self.rate * step).min(1.0);
        particle.velocity[0] *= scale;
        particle.velocity[1] *= scale;
        Ok(())
    }
}

/// Curl flow that grows with age, so a freshly born outline is crisp and then drifts apart.
pub(super) struct Dissolve {
    pub(super) field: FlowField,
}

impl ParticleModifier for Dissolve {
    fn apply(
        &self,
        particle: &mut Particle,
        step: f32,
        time: Time,
        random: &mut Random,
    ) -> Result<(), ValidationError> {
        let age = (particle.age / particle.lifetime).clamp(0.0, 1.0);
        self.field.apply(particle, step * age * age, time, random)
    }
}
