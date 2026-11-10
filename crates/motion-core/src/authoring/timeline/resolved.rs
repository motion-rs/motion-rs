//! Soundtrack clips resolved at an I/O boundary.

use crate::{AudioClip, Duration};

/// A timeline's soundtrack after its clips were resolved, for example from files.
///
/// Produced by [`Timeline::resolve_audio`](super::Timeline::resolve_audio) and held by
/// mixing, export and Studio next to the unchanged timeline.
#[derive(Clone, Debug)]
pub struct ResolvedAudio {
    pub(super) clips: Vec<AudioClip>,
    pub(super) duration: Duration,
}

impl ResolvedAudio {
    /// Resolved clips in the order of the soundtrack.
    pub fn clips(&self) -> &[AudioClip] {
        &self.clips
    }

    /// Output duration needed to play every clip. Equals the timeline duration
    /// unless that was inferred and a resolved clip is longer.
    pub fn duration(&self) -> Duration {
        self.duration
    }
}
