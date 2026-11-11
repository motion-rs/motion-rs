//! Audio owned by a timeline: placed clips, named submixes and master processing.

use super::invalid;
use crate::audio::{AudioBus, Master};
use crate::{AudioClip, Duration, FrameRate};
use std::sync::Arc;

const MAX_BUSES: usize = 16;

/// Audio clips, buses and master settings of one timeline.
///
/// Audio is not snapped to the video grid, may overlap freely and is grouped by
/// bus, so it is kept apart from the picture tracks. Clones share storage.
#[derive(Clone, Debug, Default)]
pub struct Soundtrack {
    pub(super) clips: Arc<Vec<AudioClip>>,
    pub(super) buses: Arc<Vec<AudioBus>>,
    pub(super) master: Master,
}

impl Soundtrack {
    /// Clips declared on this timeline, in insertion order; nested timelines
    /// contribute their audio when flattened for mixing.
    pub fn clips(&self) -> &[AudioClip] {
        &self.clips
    }

    /// Declared submixes in deterministic insertion order.
    pub fn buses(&self) -> &[AudioBus] {
        &self.buses
    }

    /// Master settings applied after all buses.
    pub fn master(&self) -> Master {
        self.master
    }

    pub(super) fn validate(&self) -> crate::Result<()> {
        self.master.validate()?;
        if self.buses.len() > MAX_BUSES {
            return Err(invalid("At most sixteen audio buses"));
        }
        for (index, bus) in self.buses.iter().enumerate() {
            Master {
                gain_db: bus.gain_db,
                ..Default::default()
            }
            .validate()?;
            if bus.name.is_empty()
                || bus.name.len() > 128
                || bus.effects.len() > 16
                || self.buses[..index]
                    .iter()
                    .any(|other| other.name == bus.name)
            {
                return Err(invalid("Invalid or duplicate audio bus"));
            }
        }
        Ok(())
    }

    /// Frame-aligned end of `clip` including bus effect tails. Add-time checks
    /// and the final duration share this one definition.
    pub(super) fn clip_end(&self, clip: &AudioClip, rate: FrameRate) -> crate::Result<Duration> {
        let mut end = clip.end()?.since_origin().magnitude();
        if let Some(bus) = self
            .buses
            .iter()
            .find(|bus| Some(bus.name.as_str()) == clip.parameters().bus)
        {
            for effect in &bus.effects {
                let tail = effect.tail();
                if tail.as_seconds() > 30.0 {
                    return Err(invalid("Bus effect tail requires 0..=30 seconds"));
                }
                end = Duration::from_ticks(
                    end.ticks()
                        .checked_add(tail.ticks())
                        .ok_or_else(|| invalid("Bus tail exceeds frame clock"))?,
                );
            }
        }
        if end.as_seconds() > 86400.0 {
            return Err(invalid("Audio end exceeds 24 hours"));
        }
        ceil_to_frame(end, rate)
    }

    /// Latest frame-aligned end over clips whose length is known; unresolved
    /// files without a duration contribute nothing until they are resolved.
    pub(super) fn end(&self, rate: FrameRate) -> crate::Result<Duration> {
        let mut end = Duration::ZERO;
        for clip in self.clips.iter() {
            if clip.unresolved_file() && clip.parameters().duration.is_none() {
                continue;
            }
            end = end.max(self.clip_end(clip, rate)?);
        }
        Ok(end)
    }
}

/// Smallest frame boundary at or after `end`.
pub(super) fn ceil_to_frame(end: Duration, rate: FrameRate) -> crate::Result<Duration> {
    let frame = end.to_frame(rate);
    let boundary = Duration::checked_from_frame(frame, rate)
        .ok_or_else(|| invalid("Audio end exceeds frame clock"))?;
    if boundary < end {
        Duration::checked_from_frame(frame + 1, rate)
            .ok_or_else(|| invalid("Audio end exceeds frame clock"))
    } else {
        Ok(boundary)
    }
}
