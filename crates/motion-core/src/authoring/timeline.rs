//! Tick-based multi-track scheduling, with exact rational frame boundaries.
//!
//! Current scheduling uses only scene contexts and rational frame boundaries.
//!
//! ```
//! use motion_core::prelude::*;
//! use motion_core::{
//!     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
//!     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
//! };
//! use motion_core::authoring::{Scene as _, timeline::Timeline};
//! let tile =
//!     (|_: &SceneContext<'_>| Node::from(Rect::new(0.0, 0.0, 8.0, 4.0).fill(Color::WHITE)))
//!         .with_duration(1.seconds());
//! let timeline = Timeline::new(Size::new(8.0, 4.0), 30)
//!     .track(|track| track.push(tile).gap(500.milliseconds()));
//! timeline.validate()?;
//! assert_eq!(timeline.frame_count(), 45);
//! let cache = SceneCache::default();
//! let context = SceneContext::new(Time::ZERO, timeline.frame_rate(), timeline.size(), &cache);
//! timeline.render(&context).compile(
//!     timeline.size(),
//!     PixelSize::new(8, 4),
//!     ValidationLimits::default(),
//! )?;
//! # Ok::<(), motion_core::Error>(())
//! ```

use super::Scene;
use crate::deferred::DeferredErrors;
use crate::{
    Color, Duration, Error, FrameRate, Node, NodeModifiers, SceneContext, Size, TICKS_PER_SECOND,
    Time,
};
use std::{fmt, sync::Arc};

mod soundtrack;
#[cfg(test)]
mod tests;
mod track;

pub use soundtrack::Soundtrack;
pub use track::{Clip, Track, TrackBuilder};

const MAX_TRACKS: usize = 256;

const MAX_CLIPS: usize = 16_384;

const MAX_ADJUSTMENTS: usize = 65_536;

fn invalid(message: impl Into<String>) -> Error {
    Error::Timeline {
        timeline: "<native>".into(),
        message: message.into(),
    }
}

fn on_track(track: usize, message: impl fmt::Display) -> Error {
    invalid(format!("track[{track}]: {message}"))
}

fn add_time(first: Time, second: Duration) -> crate::Result<Time> {
    first
        .ticks()
        .checked_add(second.ticks())
        .map(Time::from_ticks)
        .ok_or_else(|| invalid("Scheduled time exceeds signed ticks"))
}

fn align(time: Time, rate: FrameRate) -> crate::Result<(Time, i64)> {
    let quantum = i128::from(TICKS_PER_SECOND) * i128::from(rate.denominator());
    let numerator = i128::from(rate.numerator());
    let lower = (i128::from(time.ticks()) * numerator).div_euclid(quantum);
    let boundary = |frame: i128| {
        let ticks = frame * quantum;
        ticks.div_euclid(numerator) + i128::from(ticks.rem_euclid(numerator) != 0)
    };
    let left = boundary(lower);
    let right = boundary(lower + 1);
    let frame = if i128::from(time.ticks()) - left < right - i128::from(time.ticks()) {
        lower
    } else {
        lower + 1
    };
    let ticks = i64::try_from(boundary(frame))
        .map_err(|_| invalid("Nearest frame boundary exceeds signed ticks"))?;
    let frame =
        i64::try_from(frame).map_err(|_| invalid("Nearest frame index exceeds signed range"))?;
    Ok((Time::from_ticks(ticks), frame))
}

/// Boundary that moved when scheduling onto the timeline's frame grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryEdge {
    /// Clip activation start.
    Start,
    /// Clip exclusive end.
    End,
    /// Trailing track cursor, including an explicit gap.
    Cursor,
    /// Explicit timeline duration override.
    Duration,
}

/// Structured warning; construction never writes warnings to stderr.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundaryAdjustment {
    /// Track insertion index, absent for a timeline duration override.
    pub track: Option<usize>,
    /// Stable clip insertion ID, absent for cursors or duration overrides.
    pub clip: Option<usize>,
    /// Adjusted boundary category.
    pub edge: BoundaryEdge,
    /// Original tick position.
    pub requested: Time,
    /// Nearest representable frame boundary; ties choose the later frame.
    pub aligned: Time,
    /// Exact aligned frame index.
    pub frame: i64,
}

/// multi-track timeline with integer frame-aligned intervals.
///
/// Authoring builders retain diagnostics; try_* variants report immediately. Empty
/// timelines last zero ticks; trailing gaps contribute duration. An explicit
/// duration clips or extends all tracks without retiming their scenes.
/// This is the canonical root Timeline.
/// Nested output clips the child viewport before centered contain mapping into
/// its parent's design space; child frame rate does not retime parent ticks.
#[derive(Clone, Debug)]
pub struct Timeline {
    size: Size,
    rate: FrameRate,
    background: Option<Color>,
    tracks: Arc<Vec<Track>>,
    inferred: Duration,
    explicit: Option<Duration>,
    adjustments: Arc<Vec<BoundaryAdjustment>>,
    soundtrack: Soundtrack,
    resolved_audio: Option<Arc<Vec<crate::AudioClip>>>,
    errors: Arc<DeferredErrors>,
}

impl Timeline {
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

    /// Creates an authoring timeline; invalid dimensions/rates are reported by validate().
    pub fn new(size: Size, rate: impl Into<FrameRate>) -> Self {
        let rate = rate.into();
        Self::empty(size, rate).defer(|_| Self::try_new(size, rate))
    }

    fn empty(size: Size, rate: FrameRate) -> Self {
        Self {
            size,
            rate,
            background: None,
            tracks: Arc::new(Vec::new()),
            inferred: Time::ZERO,
            explicit: None,
            adjustments: Arc::new(Vec::new()),
            soundtrack: Soundtrack::default(),
            resolved_audio: None,
            errors: Arc::default(),
        }
    }

    /// Validates every retained authoring diagnostic and nested scene before execution.
    pub fn validate(&self) -> crate::Result<()> {
        self.soundtrack.validate()?;
        let mut errors = self.errors.as_slice().to_vec();
        for (track_index, track) in self.tracks.iter().enumerate() {
            for clip in track.clips.iter() {
                if let Err(error) = clip.scene.validate() {
                    errors.push(Arc::new(on_track(
                        track_index,
                        format!("clip[{}]: {error}", clip.id),
                    )));
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(Error::BuildErrors(errors))
        }
    }

    /// Constructs one track while accumulating all invalid operations.
    pub fn track(self, build: impl FnOnce(TrackBuilder) -> TrackBuilder) -> Self {
        self.defer(|timeline| timeline.try_track(|builder| Ok(build(builder))))
    }

    /// Adds timeline audio, deferring validation failures to execution.
    pub fn audio(self, clip: crate::AudioClip) -> Self {
        self.defer(|timeline| timeline.try_audio(clip))
    }

    /// Sets optional master processing, shared by preview and export.
    pub fn master(mut self, master: crate::audio::Master) -> Self {
        self.soundtrack.master = master;
        self
    }

    /// Declares a named offline submix before the master.
    pub fn audio_bus(self, bus: crate::audio::AudioBus) -> Self {
        self.defer(|mut timeline| {
            Arc::make_mut(&mut timeline.soundtrack.buses).push(bus);
            for clip in timeline.soundtrack.clips.as_ref().clone() {
                if clip.unresolved_file() && clip.parameters().duration.is_none() {
                    continue;
                }
                timeline.inferred = timeline.inferred.max(timeline.audio_end(&clip)?);
            }
            Ok(timeline)
        })
    }

    /// Audio clips, buses and master settings declared on this timeline.
    pub fn soundtrack(&self) -> &Soundtrack {
        &self.soundtrack
    }

    /// Immutable master settings at the audio execution boundary.
    pub fn audio_master(&self) -> crate::audio::Master {
        self.soundtrack.master
    }

    /// Declared submixes in deterministic insertion order.
    pub fn audio_buses(&self) -> &[crate::audio::AudioBus] {
        &self.soundtrack.buses
    }

    /// Overrides output duration, deferring validation failures to execution.
    pub fn duration(self, duration: Duration) -> Self {
        self.defer(|timeline| timeline.try_duration(duration))
    }

    /// Creates a timeline without discovering fonts, devices or encoders.
    /// Rates above one frame per tick cannot define a distinct frame grid.
    pub fn try_new(size: Size, rate: impl Into<FrameRate>) -> crate::Result<Self> {
        let rate = rate.into();
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

    /// Adds a full-design background below every track; alpha is retained.
    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    /// Adds audio without snapping its placement to the video grid. Audio
    /// extends inferred output to the next frame boundary, never trims its tail.
    pub fn try_audio(mut self, clip: crate::AudioClip) -> crate::Result<Self> {
        if self.soundtrack.clips.len() >= 256 {
            return Err(invalid("Timeline exceeds 256 audio clips"));
        }
        if clip.unresolved_file() && clip.parameters().duration.is_none() {
            Arc::make_mut(&mut self.soundtrack.clips).push(clip);
            return Ok(self);
        }
        self.inferred = self.inferred.max(self.audio_end(&clip)?);
        Arc::make_mut(&mut self.soundtrack.clips).push(clip);
        Ok(self)
    }

    fn audio_end(&self, clip: &crate::AudioClip) -> crate::Result<Time> {
        let mut end = clip.end()?;
        if let Some(bus) = self
            .soundtrack
            .buses
            .iter()
            .find(|bus| Some(bus.name.as_str()) == clip.parameters().bus)
        {
            for effect in &bus.effects {
                let tail = effect.tail();
                if tail < Time::ZERO || tail.as_seconds() > 30.0 {
                    return Err(invalid("Bus effect tail requires 0..=30 seconds"));
                }
                end = Time::from_ticks(
                    end.ticks()
                        .checked_add(tail.ticks())
                        .ok_or_else(|| invalid("Bus tail exceeds frame clock"))?,
                );
            }
        }
        if end.as_seconds() > 86400.0 {
            return Err(invalid("Audio end exceeds 24 hours"));
        }
        self.ceil_to_frame(end)
    }

    /// Smallest frame boundary at or after `end`.
    fn ceil_to_frame(&self, end: Time) -> crate::Result<Time> {
        let frame = end.to_frame(self.rate);
        let boundary = Time::checked_from_frame(frame, self.rate)
            .ok_or_else(|| invalid("Audio end exceeds frame clock"))?;
        if boundary < end {
            Time::checked_from_frame(frame + 1, self.rate)
                .ok_or_else(|| invalid("Audio end exceeds frame clock"))
        } else {
            Ok(boundary)
        }
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
            errors: DeferredErrors::default(),
        };
        let builder = build(builder)?;
        if builder.index != self.tracks.len() || builder.rate != self.rate {
            return Err(invalid(
                "Track builder belongs to a different index or frame grid",
            ));
        }
        let (track, adjustments) = builder.finish()?;
        let total = self
            .tracks
            .iter()
            .map(|track| track.clips.len())
            .sum::<usize>()
            + track.clips.len();
        if total > MAX_CLIPS || self.adjustments.len() + adjustments.len() > MAX_ADJUSTMENTS {
            return Err(invalid("Timeline exceeds clip or adjustment allowance"));
        }
        self.inferred = self.inferred.max(track.duration);
        Arc::make_mut(&mut self.tracks).push(track);
        Arc::make_mut(&mut self.adjustments).extend(adjustments);
        Ok(self)
    }

    /// Overrides total duration on the nearest frame grid; zero is intentionally empty.
    pub fn try_duration(mut self, duration: Duration) -> crate::Result<Self> {
        if duration < Time::ZERO {
            return Err(invalid("Timeline duration must be nonnegative"));
        }
        let (aligned, frame) = align(duration, self.rate)?;
        if duration > Time::ZERO && aligned == Time::ZERO {
            return Err(invalid(
                "Timeline duration becomes empty after frame alignment",
            ));
        }
        let adjustments = Arc::make_mut(&mut self.adjustments);
        adjustments.retain(|adjustment| adjustment.edge != BoundaryEdge::Duration);
        if duration != aligned {
            if adjustments.len() >= MAX_ADJUSTMENTS {
                return Err(invalid("Timeline exceeds adjustment allowance"));
            }
            adjustments.push(BoundaryAdjustment {
                track: None,
                clip: None,
                edge: BoundaryEdge::Duration,
                requested: duration,
                aligned,
                frame,
            });
        }
        self.explicit = Some(aligned);
        Ok(self)
    }

    /// Logical timeline viewport; supply this to composition rendering.
    pub fn size(&self) -> Size {
        self.size
    }

    /// Frame grid used for all schedule boundary adjustments.
    pub fn frame_rate(&self) -> FrameRate {
        self.rate
    }

    /// Finite inferred/overridden duration, independent of any parent clip duration.
    pub fn total_duration(&self) -> Duration {
        self.explicit.unwrap_or(self.inferred)
    }

    /// Exact output frame count. Existing Composition needs checked u32 conversion.
    pub fn frame_count(&self) -> u64 {
        self.total_duration().to_frame(self.rate) as u64
    }

    /// Immutable tracks in bottom-to-top paint order.
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// Accumulated structured boundary warnings, without stderr side effects.
    pub fn adjustments(&self) -> &[BoundaryAdjustment] {
        &self.adjustments
    }

    /// Resolves flattened audio at an I/O boundary while preserving visual scene clocks.
    pub fn resolve_audio(
        &self,
        mut resolve: impl FnMut(&crate::AudioClip) -> crate::Result<crate::AudioClip>,
    ) -> crate::Result<Self> {
        let clips = self
            .audio_clips()?
            .iter()
            .map(&mut resolve)
            .collect::<crate::Result<Vec<_>>>()?;
        let mut timeline = self.clone();
        for clip in &clips {
            timeline.inferred = timeline.inferred.max(timeline.audio_end(clip)?);
        }
        timeline.resolved_audio = Some(Arc::new(clips));
        Ok(timeline)
    }
}

impl Scene for Timeline {
    fn audio_composition(&self) -> Option<&Timeline> {
        Some(self)
    }

    fn validate(&self) -> crate::Result<()> {
        Timeline::validate(self)
    }

    fn duration(&self) -> Option<Duration> {
        Some(self.total_duration())
    }

    fn audio_clips(&self) -> crate::Result<Vec<crate::AudioClip>> {
        self.validate()?;
        if let Some(clips) = &self.resolved_audio {
            return Ok(clips.as_ref().clone());
        }
        let mut audio = self.soundtrack.clips.as_ref().clone();
        for track in self.tracks.iter() {
            for clip in track.clips.iter() {
                let sources = if let Some(child) = clip.scene.audio_composition().filter(|child| {
                    child.audio_master() != crate::Master::default()
                        || !child.audio_buses().is_empty()
                }) {
                    vec![crate::AudioClip::composition(child.clone()).duration(clip.duration)]
                } else {
                    clip.scene.audio_clips()?
                };
                for source in sources {
                    if let Some(source) = source.relocated(clip.start, clip.duration)? {
                        audio.push(source);
                    }
                    if audio.len() > 256 {
                        return Err(invalid("Nested timeline exceeds 256 audio clips"));
                    }
                }
            }
        }
        Ok(audio)
    }

    fn render(&self, context: &SceneContext<'_>) -> Node {
        if let Err(error) = self.validate() {
            return super::transition::diagnostic(crate::ValidationError::invalid(
                "timeline",
                "build",
                error.to_string(),
            ));
        }
        if context.time() < Time::ZERO || context.time() >= self.total_duration() {
            return Node::empty();
        }
        let parent_size = context.size();
        let nested = context.is_nested_scene();
        let context = context.for_timeline(self.size, self.rate);
        let background = self
            .background
            .map(|color| Node::from(self.size.to_rect().fill(color)));
        let clips = self
            .tracks
            .iter()
            .filter_map(|track| track.render(&context));
        let node = Node::group(background.into_iter().chain(clips));
        if parent_size == self.size {
            return if nested {
                node.clip_with(self.size.to_rect(), crate::ClipMode::Geometric)
            } else {
                node
            };
        }
        let scale =
            (parent_size.width / self.size.width).min(parent_size.height / self.size.height);
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            f64::NAN
        };
        node.clip_with(self.size.to_rect(), crate::ClipMode::Geometric)
            .pivot(crate::Pivot::Point(crate::Point::ORIGIN))
            .scale(scale)
            .translate(
                (parent_size.width - self.size.width * scale) * 0.5,
                (parent_size.height - self.size.height * scale) * 0.5,
            )
    }
}
