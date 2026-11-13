//! Stable event policies and sub-emitter inheritance, independent of rendering.

use super::*;

pub use crate::{Inherit, ParticleEventKind};

/// Event payload ordered by stable nonrecycled particle ID.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleEvent {
    /// Exact simulation boundary time (including prewarm offset).
    pub time: Time,
    /// Particle state at the event.
    pub particle: Particle,
    /// Trigger kind.
    pub kind: ParticleEventKind,
}

/// Event-triggered emitter. It receives local step zero and Time::ZERO for each trigger.
#[derive(Clone)]
pub struct SubEmitter {
    /// Birth/death/contact/age trigger.
    pub on: ParticleEventKind,
    /// Immutable local emitter definition.
    pub emitter: Arc<dyn ParticleEmitter>,
    /// Parent property policy.
    pub inherit: Inherit,
    /// Probability in 0..=1, keyed by parent/event/rule.
    pub probability: f32,
}

impl SubEmitter {
    pub(super) fn validate(&self) -> Result<(), ValidationError> {
        if !self.probability.is_finite() || !(0.0..=1.0).contains(&self.probability) {
            return Err(invalid(
                "sub_emitter",
                "Probability must be finite in 0..=1",
            ));
        }
        if let ParticleEventKind::AgeReached(age) = self.on
            && (!age.is_finite() || age <= 0.0 || age > 3600.0)
        {
            return Err(invalid(
                "sub_emitter",
                "Age threshold must be finite in (0,3600]",
            ));
        }
        Ok(())
    }
}
