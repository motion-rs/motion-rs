//! Shared narration markers and measured, safe-area subtitle production.

use motion_core::ValidationError;
use std::{collections::BTreeMap, time::Duration};

mod layout;
mod track;

pub use track::{CaptionCue, CaptionTrack};

pub use layout::{CaptionLayout, CaptionStyle};

/// One composition clock for narration, captions and named animation markers.
/// This plans media; it never synthesizes speech or reads a playback clock.
#[derive(Clone, Debug)]
pub struct CueSheet {
    duration: Duration,
    captions: CaptionTrack,
    markers: BTreeMap<String, Duration>,
}

impl CueSheet {
    /// Validates a positive duration of at most 24 hours and contained captions.
    pub fn new(duration: Duration, captions: CaptionTrack) -> Result<Self, ValidationError> {
        if duration.is_zero()
            || duration > Duration::from_secs(86400)
            || captions.cues().iter().any(|cue| cue.end() > duration)
        {
            return Err(invalid(
                "Cue sheet requires 0 < duration <= 24h and contained captions",
            ));
        }
        Ok(Self {
            duration,
            captions,
            markers: BTreeMap::new(),
        })
    }

    /// Adds one unique marker, including an optional exact end marker.
    /// At most 256 names of 1..=128 UTF-8 bytes without controls are accepted.
    pub fn mark(
        mut self,
        name: impl Into<String>,
        time: Duration,
    ) -> Result<Self, ValidationError> {
        let name = name.into();
        if name.is_empty()
            || name.len() > 128
            || name.chars().any(char::is_control)
            || time > self.duration
            || self.markers.len() == 256
            || self.markers.contains_key(&name)
        {
            return Err(invalid(
                "Invalid, duplicate or out-of-range narration marker",
            ));
        }
        self.markers.insert(name, time);
        Ok(self)
    }

    /// Returns the declared duration, without rounding it to a frame rate.
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Returns captions in their original order, including intentional overlaps.
    pub fn captions(&self) -> &CaptionTrack {
        &self.captions
    }

    /// Uniformly retimes captions and markers to a new duration in `(0,24h]`.
    /// Exact integer ratios round nanoseconds to nearest, ties upward; collapsed
    /// cues are errors. Rebuild cached captions afterward. Audio is NOT stretched:
    /// remeasure/re-record it and use `audio_clip` to check the new intervals.
    pub fn retimed(&self, duration: Duration) -> Result<Self, ValidationError> {
        if duration.is_zero() || duration > Duration::from_secs(86400) {
            return Err(invalid("Retimed cue sheet requires duration in (0,24h]"));
        }
        let map = |time: Duration| {
            Duration::from_nanos(
                ((time.as_nanos() * duration.as_nanos() + self.duration.as_nanos() / 2)
                    / self.duration.as_nanos()) as u64,
            )
        };
        let cues = self
            .captions
            .cues()
            .iter()
            .map(|cue| CaptionCue::new(map(cue.start()), map(cue.end()), cue.text()))
            .collect::<Result<Vec<_>, ValidationError>>()?;
        let mut sheet = Self::new(duration, CaptionTrack::new(cues)?)?;
        for (name, time) in &self.markers {
            sheet = sheet.mark(name, map(*time))?;
        }
        Ok(sheet)
    }

    /// Resolves a name; missing names are errors rather than time zero.
    pub fn time(&self, name: &str) -> Result<Duration, ValidationError> {
        self.markers
            .get(name)
            .copied()
            .ok_or_else(|| invalid("Unknown narration marker"))
    }

    /// Returns elapsed time since a marker while this composition is active.
    /// Frame and marker times share one clock; sequence-local frames are not rebased.
    /// Comparisons use exact integer cross-products; elapsed nanoseconds round down.
    pub fn elapsed(
        &self,
        time: motion_core::Time,
        name: &str,
    ) -> Result<Option<Duration>, ValidationError> {
        let rate = motion_core::TICKS_PER_SECOND as u128;
        let marker = self.time(name)?.as_nanos() * rate;
        let Some(ticks) = u128::try_from(time.ticks()).ok() else {
            return Ok(None);
        };
        let now = ticks * 1_000_000_000;
        if now < marker || now >= self.duration.as_nanos() * rate {
            return Ok(None);
        }
        Ok(Some(Duration::from_nanos(((now - marker) / rate) as u64)))
    }

    /// Samples clamped animation progress between two strictly ordered markers.
    pub fn progress(
        &self,
        time: motion_core::Time,
        start: &str,
        end: &str,
    ) -> Result<f64, ValidationError> {
        let rate = motion_core::TICKS_PER_SECOND as u128;
        let start = self.time(start)?.as_nanos() * rate;
        let end = self.time(end)?.as_nanos() * rate;
        if start >= end {
            return Err(invalid("Animation markers require start < end"));
        }
        let now = u128::try_from(time.ticks()).unwrap_or(0) * 1_000_000_000;
        Ok((now.saturating_sub(start) as f64 / (end - start) as f64).clamp(0.0, 1.0))
    }

    /// Places a measured source interval at a caption's start, without truncation.
    /// The caller obtains source duration separately; this does not probe media.
    pub fn audio_clip(
        &self,
        cue: usize,
        audio: std::sync::Arc<dyn motion_core::AudioSource>,
        source: (motion_core::Time, motion_core::Time),
    ) -> motion_core::Result<motion_core::AudioClip> {
        let cue = self
            .captions
            .cues()
            .get(cue)
            .ok_or_else(|| invalid("Unknown narration cue"))?;
        if source.0 < motion_core::Time::ZERO || source.1 <= source.0 {
            return Err(invalid("Narration source requires 0 <= in < out").into());
        }
        let duration = source.1 - source.0;
        if duration.ticks() as u128 * 1_000_000_000
            > (cue.end() - cue.start()).as_nanos() * motion_core::TICKS_PER_SECOND as u128
        {
            return Err(
                invalid("Narration exceeds its caption interval; retime explicitly").into(),
            );
        }
        let start = (cue.start().as_nanos() * motion_core::TICKS_PER_SECOND as u128 + 500_000_000)
            / 1_000_000_000;
        Ok(motion_core::AudioClip::source(audio)
            .start(motion_core::Time::from_ticks(start as i64))
            .offset(source.0.since_origin())
            .duration(duration))
    }
}

fn invalid(message: &str) -> ValidationError {
    ValidationError::invalid("captions", "input", message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CaptionCue;
    use motion_core::{AudioSource, FrameRate, Time, TimeLiteral};

    #[test]
    fn markers_use_the_frame_clock_and_reject_ambiguous_names() -> Result<(), ValidationError> {
        let sheet = CueSheet::new(Duration::from_secs(10), CaptionTrack::default())?
            .mark("in", Duration::from_millis(100))?
            .mark("out", Duration::from_secs(1))?;
        let frame = |index| Time::from_frame(index, FrameRate::from(30));
        assert_eq!(sheet.elapsed(frame(2), "in")?, None);
        assert_eq!(sheet.elapsed(frame(3), "in")?, Some(Duration::ZERO));
        assert_eq!(sheet.progress(frame(30), "in", "out")?, 1.0);
        assert!(sheet.progress(frame(3), "out", "in").is_err());
        assert!(sheet.time("missing").is_err());
        assert_eq!(sheet.elapsed(frame(300), "in")?, None);
        let fractional = sheet
            .clone()
            .mark("fractional", Duration::from_nanos(100_000_001))?;
        assert_eq!(fractional.elapsed(frame(3), "fractional")?, None);
        assert!(sheet.clone().mark("in", Duration::ZERO).is_err());
        let captions = CaptionTrack::new([CaptionCue::new(
            Duration::ZERO,
            Duration::from_secs(11),
            "long",
        )?])?;
        assert!(CueSheet::new(Duration::from_secs(10), captions).is_err());
        Ok(())
    }

    #[test]
    fn retiming_preserves_shared_boundaries_and_audio_must_fit() -> motion_core::Result {
        let track = CaptionTrack::new([
            CaptionCue::new(Duration::from_secs(1), Duration::from_secs(3), "one")?,
            CaptionCue::new(Duration::from_secs(3), Duration::from_secs(5), "two")?,
        ])?;
        let sheet =
            CueSheet::new(Duration::from_secs(10), track)?.mark("cut", Duration::from_secs(3))?;
        let short = sheet.retimed(Duration::from_secs(5))?;
        assert_eq!(short.time("cut")?, Duration::from_millis(1500));
        assert_eq!(
            short.captions().cues()[0].end(),
            short.captions().cues()[1].start()
        );
        assert!(sheet.retimed(Duration::ZERO).is_err());
        assert!(sheet.retimed(Duration::from_nanos(1)).is_err());
        let audio: std::sync::Arc<dyn AudioSource> =
            std::sync::Arc::new(motion_encode::PcmBuffer::new(48_000, 1, vec![0.1; 96_000])?);
        assert!(
            short
                .audio_clip(
                    0,
                    audio.clone(),
                    (Time::ZERO, 1001.milliseconds().after_origin())
                )
                .is_err()
        );
        let clip = short.audio_clip(0, audio.clone(), (Time::ZERO, 1.seconds().after_origin()))?;
        let mixed = motion_encode::MixedAudio::new(&[clip], 48_000, 2.seconds())?;
        let mut sample = [0.0; 2];
        assert_eq!(mixed.read(0, &mut sample), 1);
        assert_eq!(sample, [0.0; 2]);
        assert_eq!(mixed.read(24_000, &mut sample), 1);
        assert!(sample.iter().all(|value| *value > 0.0));
        assert!(
            short
                .audio_clip(2, audio, (Time::ZERO, 1.seconds().after_origin()))
                .is_err()
        );
        Ok(())
    }
}
