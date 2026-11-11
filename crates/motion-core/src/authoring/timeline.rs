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
//!     .track(|track| track.push(tile).gap(500.milliseconds()))
//!     .build()?;
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
use crate::{
    Color, Duration, Error, FrameRate, Node, NodeModifiers, SceneContext, Shape, Size,
    TICKS_PER_SECOND, Time,
};
use std::{fmt, sync::Arc};

mod builder;
mod diagnostics;
mod resolved;
mod soundtrack;
#[cfg(test)]
mod tests;
mod track;

pub use builder::TimelineBuilder;
pub use diagnostics::{BoundaryAdjustment, BoundaryEdge, Diagnostics, TimelineWarning};
pub use resolved::ResolvedAudio;
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

/// Immutable, validated multi-track timeline with integer frame-aligned intervals.
///
/// Created by [`TimelineBuilder::build`]; a built timeline never changes. Empty
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
    duration: Duration,
    inferred_duration: bool,
    diagnostics: Diagnostics,
    soundtrack: Soundtrack,
}

impl Timeline {
    /// Starts a timeline; invalid dimensions or rates are reported by `build`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(size: Size, rate: impl Into<FrameRate>) -> TimelineBuilder {
        let rate = rate.into();
        match TimelineBuilder::checked(size, rate) {
            Ok(builder) => builder,
            Err(error) => TimelineBuilder::empty(Size::new(1.0, 1.0), rate).failed(error),
        }
    }

    /// Starts a timeline without discovering fonts, devices or encoders.
    /// Rates above one frame per tick cannot define a distinct frame grid.
    pub fn try_new(size: Size, rate: impl Into<FrameRate>) -> crate::Result<TimelineBuilder> {
        TimelineBuilder::checked(size, rate.into())
    }

    /// Reopens this timeline for further edits. Audio resolved by
    /// [`resolve_audio`](Self::resolve_audio) is dropped and must be resolved again.
    pub fn to_builder(&self) -> TimelineBuilder {
        TimelineBuilder::from_timeline(self)
    }

    /// Logical timeline viewport; supply this to composition rendering.
    pub fn size(&self) -> Size {
        self.size
    }

    /// Frame grid used for all schedule boundary adjustments.
    pub fn frame_rate(&self) -> FrameRate {
        self.rate
    }

    /// Background below every track, if any.
    pub fn background(&self) -> Option<Color> {
        self.background
    }

    /// Final duration: the explicit value, or the end of the tracks and audio,
    /// independent of any parent clip duration.
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Exact output frame count. Existing Composition needs checked u32 conversion.
    pub fn frame_count(&self) -> u64 {
        self.duration.to_frame(self.rate) as u64
    }

    /// Immutable tracks in bottom-to-top paint order.
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// Audio clips, buses and master settings declared on this timeline.
    pub fn soundtrack(&self) -> &Soundtrack {
        &self.soundtrack
    }

    /// Structured warnings, without stderr side effects.
    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    /// Resolves the soundtrack's clips at an I/O boundary, for example by reading
    /// files. The timeline itself is unchanged; the result carries the clips and the
    /// output duration they require.
    pub fn resolve_audio(
        &self,
        mut resolve: impl FnMut(&crate::AudioClip) -> crate::Result<crate::AudioClip>,
    ) -> crate::Result<ResolvedAudio> {
        let clips = self
            .soundtrack
            .clips
            .iter()
            .map(&mut resolve)
            .collect::<crate::Result<Vec<_>>>()?;
        let mut duration = self.duration;
        if self.inferred_duration {
            for clip in &clips {
                duration = duration.max(self.soundtrack.clip_end(clip, self.rate)?);
            }
        }
        Ok(ResolvedAudio { clips, duration })
    }

    /// Wraps this timeline as a picture-only [`Scene`], for embedding where audio
    /// must not follow, for example `SceneContext::embed`.
    pub fn visual(&self) -> TimelineVisual {
        TimelineVisual(self.clone())
    }

    /// Builds one snapshot without retaining previous-frame state. Times at or after
    /// [`duration`](Self::duration) produce nothing.
    pub fn render(&self, context: &SceneContext<'_>) -> Node {
        self.render_until(context, self.duration)
    }

    /// Like [`render`](Self::render) but keeps drawing until `end`, for exports whose
    /// resolved audio outlasts the picture.
    pub fn render_until(&self, context: &SceneContext<'_>, end: Duration) -> Node {
        if context.time() < Time::ZERO || context.time().since_origin() >= end {
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

/// Picture-only view of a [`Timeline`], created by [`Timeline::visual`].
///
/// Clips and nested scenes see the timeline's own size and frame rate; its audio is
/// not carried along.
#[derive(Clone, Debug)]
pub struct TimelineVisual(Timeline);

impl Scene for TimelineVisual {
    fn render(&self, context: &SceneContext<'_>) -> Node {
        self.0.render(context)
    }

    fn duration(&self) -> Option<Duration> {
        Some(self.0.duration)
    }
}
