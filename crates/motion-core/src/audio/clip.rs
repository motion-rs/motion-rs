//! Tick-authored audio clip occurrences, their placement parameters and internal sources.

use super::{AudioEffect, AudioSource, Duck, DuckTrigger, Ducking, PanLaw, invalid};
use crate::{Duration, Time, ValidationError};
use std::sync::Arc;

/// tick-authored audio occurrence, separate from the retained root AudioClip.
#[derive(Clone)]
pub struct AudioClip {
    source: Arc<dyn AudioSource>,
    start: Time,
    offset: Duration,
    duration: Option<Duration>,
    volume: f64,
    fade_in: Duration,
    fade_out: Duration,
    pan: f64,
    envelope: Option<Arc<dyn crate::Signal<f64>>>,
    limit: Option<Duration>,
    effects: Arc<Vec<Arc<dyn AudioEffect>>>,
    pan_animation: Option<Arc<dyn crate::Signal<f64>>>,
    pan_law: PanLaw,
    stereo_width: f64,
    bus: Option<String>,
    ducking: Arc<Vec<Ducking>>,
}

impl<Source: AudioSource + ?Sized + 'static> From<Arc<Source>> for AudioClip {
    fn from(source: Arc<Source>) -> Self {
        Self::source(source)
    }
}

/// Immutable placement and envelope inputs consumed by audio mixers.
pub struct AudioClipParameters<'a> {
    /// Stateless source captured by this occurrence.
    pub source: &'a dyn AudioSource,
    /// Destination start on the local timeline.
    pub start: Time,
    /// First source sample time.
    pub offset: Duration,
    /// Authored duration override, absent when natural file duration is unresolved.
    pub duration: Option<Duration>,
    /// Optional containing-scene crop.
    pub limit: Option<Duration>,
    /// Linear amplitude multiplier.
    pub volume: f64,
    /// Fade-in duration.
    pub fade_in: Duration,
    /// Fade-out duration.
    pub fade_out: Duration,
    /// Optional pure local-time gain animation.
    pub envelope: Option<&'a dyn crate::Signal<f64>>,
    /// Mono/stereo pan setting.
    pub pan: f64,
    /// Optional local-time pan animation, replacing the constant pan.
    pub pan_animation: Option<&'a dyn crate::Signal<f64>>,
    /// Stereo balance law.
    pub pan_law: PanLaw,
    /// Mid/side width in 0..=2; one preserves the stereo source.
    pub stereo_width: f64,
    /// Effects applied at native rate before gain and pan.
    pub effects: &'a [Arc<dyn AudioEffect>],
    /// Optional named submix.
    pub bus: Option<&'a str>,
    /// Sidechain ducking by other clips, applied after gain and pan.
    pub ducking: &'a [Ducking],
}

impl std::fmt::Debug for AudioClip {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AudioClip")
            .field("start", &self.start)
            .field("offset", &self.offset)
            .field("duration", &self.duration)
            .field("volume", &self.volume)
            .field("pan", &self.pan)
            .finish_non_exhaustive()
    }
}

impl AudioClip {
    /// Retains a nested mix, including its own buses and master, without decoding.
    pub fn composition(timeline: crate::Timeline) -> Self {
        Self::source(CompositionSource(timeline))
    }

    /// Retains a local file path without opening, probing or decoding it.
    pub fn file(path: impl Into<std::path::PathBuf>) -> Self {
        Self::source(FileReference(path.into()))
    }

    /// Replaces a deferred source at an execution boundary, preserving all authored settings.
    pub fn with_resolved_source(&self, source: Arc<dyn AudioSource>) -> Self {
        let mut clip = self.clone();
        clip.source = source;
        clip
    }

    /// Whether metadata is still required to determine a natural file duration.
    pub fn unresolved_file(&self) -> bool {
        self.source.file_path().is_some() && self.source.duration().is_none()
    }

    /// Borrows mixer parameters without reading, decoding or copying PCM.
    pub fn parameters(&self) -> AudioClipParameters<'_> {
        AudioClipParameters {
            source: self.source.as_ref(),
            start: self.start,
            offset: self.offset,
            duration: self.duration,
            limit: self.limit,
            volume: self.volume,
            fade_in: self.fade_in,
            fade_out: self.fade_out,
            envelope: self.envelope.as_deref(),
            pan: self.pan,
            pan_animation: self.pan_animation.as_deref(),
            pan_law: self.pan_law,
            stereo_width: self.stereo_width,
            effects: &self.effects,
            bus: self.bus.as_deref(),
            ducking: &self.ducking,
        }
    }

    /// Captures a stateless source. Infinite sources require an explicit duration.
    pub fn source(source: impl AudioSource + 'static) -> Self {
        Self {
            source: Arc::new(source),
            start: Time::ZERO,
            offset: Duration::ZERO,
            duration: None,
            volume: 1.0,
            fade_in: Duration::ZERO,
            fade_out: Duration::ZERO,
            pan: 0.0,
            envelope: None,
            limit: None,
            effects: Arc::new(Vec::new()),
            pan_animation: None,
            pan_law: PanLaw::ConstantPower,
            stereo_width: 1.0,
            bus: None,
            ducking: Arc::new(Vec::new()),
        }
    }

    /// Sets destination start in nonnegative ticks.
    pub fn start(mut self, start: Time) -> Self {
        self.start = start;
        self
    }

    /// Sets source offset, rounded once to the nearest source-rate frame.
    pub fn offset(mut self, offset: Duration) -> Self {
        self.offset = offset;
        self
    }

    /// Sets a finite positive occurrence duration; EOF is padded with silence.
    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }

    /// Sets finite amplitude in 0..=16; mixing never normalizes peaks automatically.
    pub fn volume(mut self, volume: f64) -> Self {
        self.volume = volume;
        self
    }

    /// Multiplies volume by a typed animation sampled at clip-local output time.
    pub fn volume_envelope(mut self, envelope: impl crate::Signal<f64> + 'static) -> Self {
        self.envelope = Some(Arc::new(envelope));
        self
    }

    /// Linear amplitude fade anchored to the clip start; overlapping fades multiply.
    pub fn fade_in(mut self, duration: Duration) -> Self {
        self.fade_in = duration;
        self
    }

    /// Linear amplitude fade ending at the occurrence's exclusive end.
    pub fn fade_out(mut self, duration: Duration) -> Self {
        self.fade_out = duration;
        self
    }

    /// Constant-power mono pan; stereo uses balance. Range -1 left through +1 right.
    pub fn pan(mut self, pan: f64) -> Self {
        self.pan = pan;
        self
    }

    /// Appends an offline effect, rendered once from the clip origin.
    pub fn effect(mut self, effect: impl AudioEffect + 'static) -> Self {
        Arc::make_mut(&mut self.effects).push(Arc::new(effect));
        self
    }

    /// Lowers this clip while `trigger` is audible. The trigger is still mixed on its own
    /// when it is also added to the timeline; at most four triggers per clip. Ducking is
    /// rendered offline from the trigger envelope, so it is deterministic and random-access safe.
    pub fn ducked_by(mut self, trigger: &AudioClip, duck: Duck) -> Self {
        let mut trigger = trigger.clone();
        trigger.ducking = Arc::new(Vec::new());
        Arc::make_mut(&mut self.ducking).push(Ducking {
            trigger: DuckTrigger::Clip(Arc::new(trigger)),
            duck,
        });
        self
    }

    /// Routes the occurrence to a named timeline bus.
    pub fn bus(mut self, name: impl Into<String>) -> Self {
        self.bus = Some(name.into());
        self
    }

    /// Replaces stereo balance law; mono panning is unaffected.
    pub fn pan_law(mut self, law: PanLaw) -> Self {
        self.pan_law = law;
        self
    }

    /// Sets a pure local-time pan animation, with values in -1..=1.
    pub fn pan_animation(mut self, animation: impl crate::Signal<f64> + 'static) -> Self {
        self.pan_animation = Some(Arc::new(animation));
        self
    }

    /// Sets mid/side width in 0..=2, without changing the mono component.
    pub fn stereo_width(mut self, width: f64) -> Self {
        self.stereo_width = width;
        self
    }

    /// Couples screen position to pan and inverse-distance gain, multiplying the existing envelope.
    /// Width is the listener-to-edge distance; this is not HRTF or three-dimensional spatial audio.
    pub fn spatial(
        mut self,
        position: impl crate::Signal<crate::Point> + 'static,
        listener: crate::Point,
        width: f64,
        rolloff: f64,
    ) -> Result<Self, ValidationError> {
        if !listener.x.is_finite()
            || !listener.y.is_finite()
            || !width.is_finite()
            || width <= 0.0
            || !rolloff.is_finite()
            || !(0.0..=1000.0).contains(&rolloff)
        {
            return Err(invalid(
                "spatial",
                "Invalid listener, width or distance rolloff",
            ));
        }
        let position: Arc<dyn crate::Signal<crate::Point>> = Arc::new(position);
        self.pan_animation = Some(Arc::new(SpatialPan {
            position: position.clone(),
            listener,
            width,
        }));
        self.envelope = Some(Arc::new(SpatialGain {
            position,
            listener,
            width,
            rolloff,
            previous: self.envelope.take(),
        }));
        Ok(self)
    }

    /// Selects a positive source-time interval; natural duration is not required.
    /// An `end` not after `start` yields an empty duration, rejected when resolved.
    pub fn trim(self, start: Duration, end: Duration) -> Self {
        self.offset(start).duration(end.saturating_sub(start))
    }

    /// Validated natural/overridden duration, before destination placement.
    pub fn resolved_duration(&self) -> Result<Duration, ValidationError> {
        let mut duration = self
            .duration
            .or_else(|| {
                self.source
                    .duration()
                    .map(|duration| duration.saturating_sub(self.offset))
            })
            .ok_or_else(|| invalid("duration", "Unbounded audio requires an explicit duration"))?;
        if duration <= Duration::ZERO {
            return Err(invalid(
                "duration",
                "Clip source duration must be positive before effect tails",
            ));
        }
        if self.effects.len() > 16 {
            return Err(invalid("effects", "At most sixteen effects per clip"));
        }
        if self.ducking.len() > 4 {
            return Err(invalid("ducking", "At most four ducking triggers per clip"));
        }
        for ducking in self.ducking.iter() {
            ducking.duck.validate()?;
            match &ducking.trigger {
                DuckTrigger::Clip(trigger) if !trigger.unresolved_file() => {
                    trigger.resolved_duration()?;
                }
                DuckTrigger::Clip(_) => {}
                DuckTrigger::Bus(_) => {
                    return Err(invalid("ducking", "Clip ducking requires a clip trigger"));
                }
            }
        }
        for effect in self.effects.iter() {
            effect.validate(self.source.sample_rate())?;
            let tail = effect.tail();
            if tail.as_seconds() > 30.0 {
                return Err(invalid("tail", "Effect tail must be in 0..=30 seconds"));
            }
            duration = Duration::from_ticks(
                duration
                    .ticks()
                    .checked_add(tail.ticks())
                    .ok_or_else(|| invalid("tail", "Effect duration overflow"))?,
            );
        }
        if self.start < Time::ZERO
            || duration <= Duration::ZERO
            || duration.as_seconds() > 86400.0
            || self
                .start
                .checked_add(duration)
                .is_none_or(|end| end.ticks() > 86400 * crate::TICKS_PER_SECOND)
            || !self.volume.is_finite()
            || !(0.0..=16.0).contains(&self.volume)
            || !self.pan.is_finite()
            || !(-1.0..=1.0).contains(&self.pan)
            || !self.stereo_width.is_finite()
            || !(0.0..=2.0).contains(&self.stereo_width)
            || self
                .bus
                .as_ref()
                .is_some_and(|name| name.is_empty() || name.len() > 128)
            || self.fade_in > duration
            || self.fade_out > duration
            || !(1..=192_000).contains(&self.source.sample_rate())
            || !(1..=2).contains(&self.source.channels())
        {
            return Err(invalid(
                "clip",
                "Invalid audio timing, format, volume, pan or fade",
            ));
        }
        Ok(duration)
    }

    /// Exclusive destination end, checked without reading samples.
    pub fn end(&self) -> Result<Time, ValidationError> {
        let duration = self.resolved_duration()?;
        Ok(self.start + self.limit.map_or(duration, |limit| limit.min(duration)))
    }

    /// Places a nested audio occurrence in a parent timeline, cropping its audible window.
    /// Source offsets and fade positions remain anchored to the original clip.
    pub fn relocated(
        &self,
        offset: Duration,
        window: Duration,
    ) -> Result<Option<Self>, ValidationError> {
        if !self.unresolved_file() {
            self.resolved_duration()?;
        }
        let start = self
            .start
            .duration_since(Time::ZERO)
            .ok_or_else(|| invalid("clip", "Audio start must be nonnegative"))?;
        if start >= window {
            return Ok(None);
        }
        let mut clip = self.clone();
        let mut ducking = Vec::new();
        for item in self.ducking.iter() {
            if let DuckTrigger::Clip(trigger) = &item.trigger
                && let Some(trigger) = trigger.relocated(offset, window)?
            {
                ducking.push(Ducking {
                    trigger: DuckTrigger::Clip(Arc::new(trigger)),
                    duck: item.duck,
                });
            }
        }
        clip.ducking = Arc::new(ducking);
        let remaining = window - start;
        clip.limit = Some(clip.limit.map_or(remaining, |limit| limit.min(remaining)));
        clip.start = Time::ZERO
            .checked_add(offset)
            .and_then(|time| time.checked_add(start))
            .ok_or_else(|| invalid("nested_audio", "Nested audio placement overflow"))?;
        if !clip.unresolved_file() {
            clip.resolved_duration()?;
        }
        Ok(Some(clip))
    }
}

struct CompositionSource(crate::Timeline);

struct SpatialPan {
    position: Arc<dyn crate::Signal<crate::Point>>,
    listener: crate::Point,
    width: f64,
}

impl crate::Signal<f64> for SpatialPan {
    fn sample(&self, time: Time) -> f64 {
        let point = self.position.sample(time);
        if !point.x.is_finite() || !point.y.is_finite() {
            return f64::NAN;
        }
        ((point.x - self.listener.x) / self.width).clamp(-1.0, 1.0)
    }
}

struct SpatialGain {
    position: Arc<dyn crate::Signal<crate::Point>>,
    listener: crate::Point,
    width: f64,
    rolloff: f64,
    previous: Option<Arc<dyn crate::Signal<f64>>>,
}

impl crate::Signal<f64> for SpatialGain {
    fn sample(&self, time: Time) -> f64 {
        let point = self.position.sample(time);
        if !point.x.is_finite() || !point.y.is_finite() {
            return f64::NAN;
        }
        let distance =
            libm::hypot(point.x - self.listener.x, point.y - self.listener.y) / self.width;
        self.previous
            .as_ref()
            .map_or(1.0, |previous| previous.sample(time))
            / (1.0 + self.rolloff * distance)
    }
}

impl AudioSource for CompositionSource {
    fn composition(&self) -> Option<&crate::Timeline> {
        Some(&self.0)
    }

    fn sample_rate(&self) -> u32 {
        48000
    }

    fn channels(&self) -> u16 {
        2
    }

    fn duration(&self) -> Option<Duration> {
        Some(self.0.duration())
    }

    fn read(&self, _: u64, output: &mut [f32]) -> usize {
        output.fill(0.0);
        0
    }

    fn try_read(&self, _: u64, _: &mut [f32]) -> crate::Result<usize> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Deferred composition requires audio preparation",
        )
        .into())
    }
}

struct FileReference(std::path::PathBuf);

impl AudioSource for FileReference {
    fn file_path(&self) -> Option<&std::path::Path> {
        Some(&self.0)
    }

    fn sample_rate(&self) -> u32 {
        48_000
    }

    fn channels(&self) -> u16 {
        2
    }

    fn read(&self, _: u64, output: &mut [f32]) -> usize {
        output.fill(0.0);
        0
    }

    fn try_read(&self, _: u64, _: &mut [f32]) -> crate::Result<usize> {
        Err(crate::Error::asset(
            "audio",
            self.0.display().to_string(),
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "File audio must resolve at an execution boundary",
            ),
        ))
    }

    fn duration(&self) -> Option<Duration> {
        None
    }
}
