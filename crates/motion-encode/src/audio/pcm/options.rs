//! Mix request: what to mix and the sample rate, interval and resampler to mix it with.

use super::{AudioClip, ResampleQuality};
use motion_core::Duration;
use std::ops::Range;

/// What a [`MixedAudio`](super::MixedAudio) is mixed from.
#[derive(Clone, Copy, Debug)]
pub enum MixSource<'a> {
    /// A timeline whose audio files are resolved by the mixer, then mixed through its buses
    /// and master.
    Timeline(&'a motion_core::Timeline),
    /// A timeline with its audio already resolved by the caller.
    Resolved {
        /// Timeline owning the buses and the master.
        timeline: &'a motion_core::Timeline,
        /// Resolved clips and the output duration they need.
        audio: &'a motion_core::ResolvedAudio,
    },
    /// Loose clips mixed without buses or a master.
    Clips {
        /// Clips in occurrence order.
        clips: &'a [AudioClip],
        /// Output duration without [`MixOptions::range`].
        duration: Duration,
    },
}

impl<'a> MixSource<'a> {
    /// Mixes a timeline whose audio was already resolved.
    pub fn resolved(
        timeline: &'a motion_core::Timeline,
        audio: &'a motion_core::ResolvedAudio,
    ) -> Self {
        Self::Resolved { timeline, audio }
    }

    /// Mixes loose clips into `duration` of output.
    pub fn clips(clips: &'a [AudioClip], duration: Duration) -> Self {
        Self::Clips { clips, duration }
    }
}

impl<'a> From<&'a motion_core::Timeline> for MixSource<'a> {
    fn from(timeline: &'a motion_core::Timeline) -> Self {
        Self::Timeline(timeline)
    }
}

impl<'a> From<&'a std::sync::Arc<motion_core::Timeline>> for MixSource<'a> {
    fn from(timeline: &'a std::sync::Arc<motion_core::Timeline>) -> Self {
        Self::Timeline(timeline)
    }
}

/// Output rate, interval and resampler of one mix.
#[derive(Clone, Debug, PartialEq)]
pub struct MixOptions {
    /// Output sample rate in Hz, in 1..=192000.
    pub sample_rate: u32,
    /// Mixes only this interval, keeping the original fade and envelope clocks.
    /// `None` mixes from the origin to the end of the source.
    pub range: Option<Range<Duration>>,
    /// Sample-rate converter for sources that differ from `sample_rate`.
    pub resampler: ResampleQuality,
}

impl MixOptions {
    /// Mixes the whole source at `sample_rate` with the default resampler.
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            range: None,
            resampler: ResampleQuality::default(),
        }
    }

    /// Mixes only the given interval.
    pub fn range(mut self, range: Range<Duration>) -> Self {
        self.range = Some(range);
        self
    }

    /// Selects the sample-rate converter.
    pub fn resampler(mut self, resampler: ResampleQuality) -> Self {
        self.resampler = resampler;
        self
    }
}
