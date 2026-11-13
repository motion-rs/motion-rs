//! Explicit fixed-boundary sub-emitter rules and nonrecursive root filtering.

use super::Compiler;
use super::particles::Emitter;
use crate::particles::{ParticleEventSpec, ParticleInheritSpec, ParticleSubEmitterSpec};
use motion_content::particles::{
    Inherit, Particle, ParticleEmitter, ParticleEvent, ParticleEventKind, SubEmitter,
};
use motion_core::{Random, Time, ValidationError};
use std::sync::Arc;

/// Custom particle slot reserved for the root-only generation marker: zero for
/// root particles and one for particles emitted by a `root_only` rule.
pub(super) const GENERATION_SLOT: usize = 3;

impl Compiler<'_> {
    pub(super) fn particle_sub_emitter(
        &mut self,
        spec: &ParticleSubEmitterSpec,
        path: &str,
    ) -> Option<SubEmitter> {
        self.bounded(f64::from(spec.probability), 0.0, 1.0, path, "probability")?;
        let on = match spec.on {
            ParticleEventSpec::Birth => ParticleEventKind::Birth,
            ParticleEventSpec::Death => ParticleEventKind::Death,
            ParticleEventSpec::Collision => ParticleEventKind::Collision,
            ParticleEventSpec::AgeReached(age) => {
                self.bounded(
                    f64::from(age),
                    f64::from(f32::MIN_POSITIVE),
                    3600.0,
                    path,
                    "on.age_reached",
                )?;
                ParticleEventKind::AgeReached(age)
            }
        };
        let emitter: Arc<dyn ParticleEmitter> =
            match self.emitter(&spec.emitter, &format!("{path}.emitter"))? {
                Emitter::Point(emitter) => Arc::new(emitter),
                Emitter::Disk(emitter) => Arc::new(emitter),
                Emitter::Rect(emitter) => Arc::new(emitter),
                Emitter::Path(emitter) => Arc::new(emitter),
                Emitter::Burst(emitter) => Arc::new(emitter),
                Emitter::Surface(emitter) => Arc::new(emitter),
                Emitter::Image(emitter) => Arc::new(emitter),
                #[cfg(feature = "text")]
                Emitter::Text(_, _) => {
                    self.error(path, "emitter", "Glyph sources cannot be event-triggered");
                    return None;
                }
            };
        let emitter = if spec.root_only {
            Arc::new(RootEmitter { emitter }) as Arc<dyn ParticleEmitter>
        } else {
            emitter
        };
        let inherit = match spec.inherit {
            ParticleInheritSpec::None => Inherit::None,
            ParticleInheritSpec::Position => Inherit::Position,
            ParticleInheritSpec::Velocity => Inherit::Velocity,
            ParticleInheritSpec::Color => Inherit::Color,
            ParticleInheritSpec::All => Inherit::All,
        };
        Some(SubEmitter {
            on,
            emitter,
            inherit,
            probability: spec.probability,
        })
    }
}

struct RootEmitter {
    emitter: Arc<dyn ParticleEmitter>,
}

impl ParticleEmitter for RootEmitter {
    fn accepts_event(&self, event: &ParticleEvent) -> bool {
        event.particle.custom[GENERATION_SLOT] == 0.0 && self.emitter.accepts_event(event)
    }

    fn emit(
        &self,
        step: u64,
        time: Time,
        random: &mut Random,
        output: &mut Vec<Particle>,
    ) -> Result<(), ValidationError> {
        let start = output.len();
        self.emitter.emit(step, time, random, output)?;
        for particle in &mut output[start..] {
            particle.custom[GENERATION_SLOT] = 1.0;
        }
        Ok(())
    }
}
