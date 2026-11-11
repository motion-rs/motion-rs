//! Fluent construction of an immutable, validated [`Timeline`].

use super::diagnostics::{BoundaryAdjustment, BoundaryEdge, Diagnostics, TimelineWarning};
use super::{
    MAX_ADJUSTMENTS, MAX_CLIPS, MAX_TRACKS, Timeline, Track, TrackBuilder, align, invalid, on_track,
};
use crate::ErrorReport;
use crate::authoring::timeline::soundtrack::Soundtrack;
use crate::{Color, Duration, Error, FrameRate, Size, TICKS_PER_SECOND, Time};
use std::sync::Arc;

/// Collects tracks, audio and settings, then [`build`](Self::build)s a [`Timeline`].
///
/// Fluent methods retain failures and report them all from `build`; `try_*`
/// variants report immediately. Empty timelines last zero ticks; trailing gaps
/// contribute duration. Without [`duration`](Self::duration) the timeline lasts
/// until the end of its tracks and audio (including bus tails, rounded up to a
/// frame); an explicit duration clips or extends the tracks without retiming
/// their scenes.
#[derive(Clone, Debug)]
pub struct TimelineBuilder {
    size: Size,
    rate: FrameRate,
    background: Option<Color>,
    tracks: Arc<Vec<Track>>,
    track_end: Duration,
    adjustments: Arc<Vec<BoundaryAdjustment>>,
    soundtrack: Soundtrack,
    nested_audio: Arc<Vec<crate::AudioClip>>,
    duration: Option<Duration>,
    errors: Arc<ErrorReport>,
}

impl TimelineBuilder {
    pub(super) fn empty(size: Size, rate: FrameRate) -> Self {
        Self {
            size,
            rate,
            background: None,
            tracks: Arc::new(Vec::new()),
            track_end: Duration::ZERO,
            adjustments: Arc::new(Vec::new()),
            soundtrack: Soundtrack::default(),
            nested_audio: Arc::default(),
            duration: None,
            errors: Arc::default(),
        }
    }

    /// Reopens a built timeline for further edits, dropping resolved audio.
    pub(super) fn from_timeline(timeline: &Timeline) -> Self {
        Self {
            size: timeline.size,
            rate: timeline.rate,
            background: timeline.background,
            tracks: timeline.tracks.clone(),
            track_end: timeline
                .tracks
                .iter()
                .map(|track| track.duration)
                .max()
                .unwrap_or(Duration::ZERO),
            adjustments: Arc::new(
                timeline
                    .diagnostics
                    .boundary_adjustments()
                    .cloned()
                    .collect(),
            ),
            soundtrack: timeline.soundtrack.clone(),
            nested_audio: Arc::default(),
            duration: (!timeline.inferred_duration).then_some(timeline.duration),
            errors: Arc::default(),
        }
    }

    pub(super) fn checked(size: Size, rate: FrameRate) -> crate::Result<Self> {
        if !size.width.is_finite()
            || !size.height.is_finite()
            || size.width <= 0.0
            || size.height <= 0.0
        {
            return Err(invalid(
                "Timeline design dimensions must be finite and positive",
            ));
        }
        if i128::from(rate.numerator())
            > i128::from(TICKS_PER_SECOND) * i128::from(rate.denominator())
        {
            return Err(invalid("Frame rate exceeds one frame per tick"));
        }
        Ok(Self::empty(size, rate))
    }

    pub(super) fn failed(mut self, error: Error) -> Self {
        Arc::make_mut(&mut self.errors).record(error);
        self
    }

    fn defer(self, operation: impl FnOnce(Self) -> crate::Result<Self>) -> Self {
        let mut previous = self.clone();
        match operation(self) {
            Ok(value) => value,
            Err(error) => {
                Arc::make_mut(&mut previous.errors).record(error);
                previous
            }
        }
    }

    /// Adds a full-design background below every track; alpha is retained.
    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    /// Constructs one track while accumulating all invalid operations.
    pub fn track(self, build: impl FnOnce(TrackBuilder) -> TrackBuilder) -> Self {
        self.defer(|builder| builder.try_track(|track| Ok(build(track))))
    }

    /// Atomically builds one track; all tracks together admit at most 16384
    /// clips, 256 tracks and 65536 frame-adjustment warnings per snapshot.
    pub fn try_track(
        mut self,
        build: impl FnOnce(TrackBuilder) -> crate::Result<TrackBuilder>,
    ) -> crate::Result<Self> {
        if self.tracks.len() >= MAX_TRACKS {
            return Err(invalid("Timeline exceeds 256 tracks"));
        }
        let builder = TrackBuilder {
            index: self.tracks.len(),
            rate: self.rate,
            clips: Vec::new(),
            cursor: Time::ZERO,
            adjustments: Vec::new(),
            pending: None,
            errors: ErrorReport::default(),
            audio: Vec::new(),
        };
        let builder = build(builder)?;
        if builder.index != self.tracks.len() || builder.rate != self.rate {
            return Err(invalid(
                "Track builder belongs to a different index or frame grid",
            ));
        }
        let (track, adjustments, audio) = builder.finish()?;
        let total = self
            .tracks
            .iter()
            .map(|track| track.clips.len())
            .sum::<usize>()
            + track.clips.len();
        if total > MAX_CLIPS || self.adjustments.len() + adjustments.len() > MAX_ADJUSTMENTS {
            return Err(invalid("Timeline exceeds clip or adjustment allowance"));
        }
        self.track_end = self.track_end.max(track.duration);
        Arc::make_mut(&mut self.tracks).push(track);
        Arc::make_mut(&mut self.adjustments).extend(adjustments);
        Arc::make_mut(&mut self.nested_audio).extend(audio);
        Ok(self)
    }

    /// Adds timeline audio, deferring validation failures to `build`.
    pub fn audio(self, clip: crate::AudioClip) -> Self {
        self.defer(|builder| builder.try_audio(clip))
    }

    /// Adds audio without snapping its placement to the video grid. Audio
    /// extends the inferred duration to the next frame boundary, never trims its tail.
    pub fn try_audio(mut self, clip: crate::AudioClip) -> crate::Result<Self> {
        if self.soundtrack.clips.len() >= 256 {
            return Err(invalid("Timeline exceeds 256 audio clips"));
        }
        if !(clip.unresolved_file() && clip.parameters().duration.is_none()) {
            self.soundtrack.clip_end(&clip, self.rate)?;
        }
        Arc::make_mut(&mut self.soundtrack.clips).push(clip);
        Ok(self)
    }

    /// Sets optional master processing, shared by preview and export.
    pub fn master(mut self, master: crate::audio::Master) -> Self {
        self.soundtrack.master = master;
        self
    }

    /// Declares a named offline submix before the master.
    pub fn audio_bus(mut self, bus: crate::audio::AudioBus) -> Self {
        Arc::make_mut(&mut self.soundtrack.buses).push(bus);
        self
    }

    /// Overrides output duration, deferring validation failures to `build`.
    pub fn duration(self, duration: Duration) -> Self {
        self.defer(|builder| builder.try_duration(duration))
    }

    /// Overrides total duration on the nearest frame grid; zero is intentionally empty.
    pub fn try_duration(mut self, duration: Duration) -> crate::Result<Self> {
        if duration < Duration::ZERO {
            return Err(invalid("Timeline duration must be nonnegative"));
        }
        let (aligned, frame) = align(duration.after_origin(), self.rate)?;
        if duration > Duration::ZERO && aligned == Time::ZERO {
            return Err(invalid(
                "Timeline duration becomes empty after frame alignment",
            ));
        }
        let adjustments = Arc::make_mut(&mut self.adjustments);
        adjustments.retain(|adjustment| adjustment.edge != BoundaryEdge::Duration);
        if duration != aligned.since_origin() {
            if adjustments.len() >= MAX_ADJUSTMENTS {
                return Err(invalid("Timeline exceeds adjustment allowance"));
            }
            adjustments.push(BoundaryAdjustment {
                track: None,
                clip: None,
                edge: BoundaryEdge::Duration,
                requested: duration.after_origin(),
                aligned,
                frame,
            });
        }
        self.duration = Some(aligned.since_origin());
        Ok(self)
    }

    /// Validates everything retained so far and freezes the timeline. All
    /// deferred failures, invalid audio settings and invalid nested scenes are
    /// reported together as one [`ErrorReport`].
    pub fn build(self) -> Result<Timeline, ErrorReport> {
        let mut errors = self.errors.as_ref().clone();
        if let Err(error) = self.soundtrack.validate() {
            errors.record(error);
        }
        for (track_index, track) in self.tracks.iter().enumerate() {
            for clip in track.clips.iter() {
                if let Err(error) = clip.scene.validate() {
                    errors.record(on_track(track_index, format!("clip[{}]: {error}", clip.id)));
                }
            }
        }
        let audio_end = match self.soundtrack.end(self.rate) {
            Ok(end) => end,
            Err(error) => {
                errors.record(error);
                Duration::ZERO
            }
        };
        if !errors.is_empty() {
            return Err(errors);
        }
        let mut soundtrack = self.soundtrack;
        if !self.nested_audio.is_empty() {
            let mut clips = soundtrack.clips.as_ref().clone();
            clips.extend(self.nested_audio.iter().cloned());
            if clips.len() > 256 {
                return Err(invalid("Nested timeline exceeds 256 audio clips").into());
            }
            soundtrack.clips = Arc::new(clips);
        }
        let content = self.track_end.max(audio_end);
        let mut warnings = self
            .adjustments
            .iter()
            .cloned()
            .map(TimelineWarning::Boundary)
            .collect::<Vec<_>>();
        if let Some(duration) = self.duration.filter(|duration| *duration < content) {
            warnings.push(TimelineWarning::Truncated { duration, content });
        }
        Ok(Timeline {
            size: self.size,
            rate: self.rate,
            background: self.background,
            tracks: self.tracks,
            duration: self.duration.unwrap_or(content),
            inferred_duration: self.duration.is_none(),
            diagnostics: Diagnostics::new(warnings),
            soundtrack,
        })
    }
}
