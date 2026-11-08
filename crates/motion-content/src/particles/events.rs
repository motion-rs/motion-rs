//! Stable event policies and sub-emitter inheritance, independent of rendering.

use super::*;

/// Fixed-boundary particle event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParticleEventKind {
    /// First admission of a live particle.
    Birth,
    /// Half-open lifetime expiry, including kill-on-hit.
    Death,
    /// Geometric or population contact.
    Collision,
    /// First boundary at/after an absolute age in seconds.
    AgeReached(f32),
}

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

/// Parent properties added to an emitter's local child record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
