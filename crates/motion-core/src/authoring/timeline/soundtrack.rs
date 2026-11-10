//! Audio owned by a timeline: placed clips, named submixes and master processing.

use super::invalid;
use crate::AudioClip;
use crate::audio::{AudioBus, Master};
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
}
