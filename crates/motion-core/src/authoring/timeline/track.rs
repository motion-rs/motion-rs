//! Timeline clips, tracks and the fluent track builder that schedules them.

use super::{BoundaryAdjustment, BoundaryEdge, MAX_CLIPS, Scene, add_time, align, on_track};
use crate::authoring::{scene, transition};
use crate::deferred::DeferredErrors;
use crate::{AudioClip, Duration, Error, FrameRate, Node, SceneContext, Time, Timeline};
use std::{fmt, sync::Arc};

/// One immutable, positive-duration scene occurrence.
#[derive(Clone)]
pub struct Clip {
    pub(super) id: usize,
    pub(super) start: Time,
    pub(super) duration: Duration,
    pub(super) offset: Time,
    pub(super) scene: Arc<dyn Scene>,
    pub(super) transition_in: Option<TransitionIn>,
}

#[derive(Clone)]
pub(super) struct TransitionIn {
    pub(super) effect: Arc<dyn transition::Transition>,
    pub(super) duration: Duration,
}

impl fmt::Debug for Clip {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Clip")
            .field("id", &self.id)
            .field("start", &self.start)
            .field("duration", &self.duration)
            .finish_non_exhaustive()
    }
}

impl Clip {
    /// Stable insertion ID within its track, independent of sorted order.
    pub fn id(&self) -> usize {
        self.id
    }

    /// Snapped activation time relative to the timeline.
    pub fn start(&self) -> Time {
        self.start
    }

    /// Snapped active duration, also supplied as clip metadata to the callback.
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Scene-local time at the clip's first frame; zero unless the clip was placed trimmed.
    pub fn offset(&self) -> Time {
        self.offset
    }

    /// Exclusive end; validated at construction.
    pub fn end(&self) -> Time {
        self.start + self.duration
    }

    /// Retained stateless scene sampled through SceneContext.
    pub fn scene(&self) -> &dyn Scene {
        self.scene.as_ref()
    }

    /// Incoming transition and its aligned overlap duration, when present.
    pub fn transition_in(&self) -> Option<(&dyn transition::Transition, Duration)> {
        self.transition_in
            .as_ref()
            .map(|value| (value.effect.as_ref(), value.duration))
    }
}

/// Sorted clips with explicit adjacent-pair overlaps; later tracks paint on top.
#[derive(Clone, Debug)]
pub struct Track {
    pub(super) clips: Arc<[Clip]>,
    pub(super) duration: Duration,
}

impl Track {
    /// Temporal order; clip insertion IDs remain stable after absolute placement.
    pub fn clips(&self) -> &[Clip] {
        &self.clips
    }

    /// End of the track, including its trailing gap.
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Active incoming/latest clip, or none in gaps; during a transition the
    /// scheduler also evaluates its outgoing predecessor for composition.
    pub fn active_clip(&self, time: Time) -> Option<&Clip> {
        let index = self
            .clips
            .partition_point(|clip| clip.start <= time)
            .checked_sub(1)?;
        self.clips.get(index).filter(|clip| time < clip.end())
    }

    /// Renders the active clip at the context time, composing an incoming
    /// transition with its predecessor; `None` in gaps.
    pub fn render(&self, context: &SceneContext<'_>) -> Option<Node> {
        let index = self
            .clips
            .partition_point(|clip| clip.start <= context.time())
            .checked_sub(1)?;
        let clip = &self.clips[index];
        if context.time() >= clip.end() {
            return None;
        }
        let render = |clip: &Clip| {
            context.during(clip.start, clip.duration, |local| {
                if clip.offset == Time::ZERO {
                    clip.scene.render(local)
                } else {
                    clip.scene
                        .render(&local.shifted(Time::from_ticks(-clip.offset.ticks())))
                }
            })
        };
        let node = if let Some(transition) = clip
            .transition_in
            .as_ref()
            .filter(|transition| context.time() < clip.start + transition.duration)
        {
            let outgoing = render(&self.clips[index - 1]);
            let incoming = render(clip);
            let animation = crate::Tween::new(0.0f64, 1.0, transition.duration)
                .easing(transition.effect.easing());
            let progress = crate::Animation::sample(&animation, context.time() - clip.start);
            if progress.is_finite() {
                let local = context
                    .shifted(clip.start)
                    .with_clip_duration(transition.duration);
                transition
                    .effect
                    .compose(progress.clamp(0.0, 1.0), outgoing, incoming, &local)
            } else {
                transition::failure("easing", "Transition easing must produce finite progress")
            }
        } else {
            render(clip)
        };
        Some(Node::group([node]))
    }
}

/// Fluent scheduler with deferred diagnostics and immediate try_* alternatives.
/// The requested cursor is not repeatedly rounded: each boundary is snapped once.
pub struct TrackBuilder {
    pub(super) index: usize,
    pub(super) rate: FrameRate,
    pub(super) clips: Vec<Clip>,
    pub(super) cursor: Time,
    pub(super) adjustments: Vec<BoundaryAdjustment>,
    pub(super) pending: Option<Arc<dyn transition::Transition>>,
    pub(super) errors: DeferredErrors,
    pub(super) audio: Vec<AudioClip>,
}

impl TrackBuilder {
    fn defer(mut self, operation: impl FnOnce(&mut Self) -> crate::Result<()>) -> Self {
        if let Err(error) = operation(&mut self) {
            self.errors.record(on_track(
                self.index,
                format!("clip[{}]: {error}", self.clips.len()),
            ));
        }
        self
    }

    /// Appends a finite scene, retaining invalid input for validate().
    pub fn push(self, scene: impl Scene + 'static) -> Self {
        let cursor = self.cursor;
        self.defer(|builder| builder.place_checked(cursor, None, Time::ZERO, scene))
    }

    /// Appends a finite authoring callback accepting drawables or Result outputs.
    pub fn scene<Output: scene::IntoSceneOutput + 'static>(
        self,
        duration: Duration,
        build: impl Fn(&SceneContext<'_>) -> Output + Send + Sync + 'static,
    ) -> Self {
        self.push(scene::scene(build).with_duration(duration))
    }

    /// Appends a scene at the cursor for `duration`, whatever its natural duration.
    pub fn push_for(self, duration: Duration, scene: impl Scene + 'static) -> Self {
        let cursor = self.cursor;
        self.defer(|builder| builder.place_checked(cursor, Some(duration), Time::ZERO, scene))
    }

    /// Places a scene at absolute ticks for `duration`, whatever its natural duration.
    pub fn place_for(self, start: Time, duration: Duration, scene: impl Scene + 'static) -> Self {
        self.defer(|builder| builder.place_checked(start, Some(duration), Time::ZERO, scene))
    }

    /// Appends a scene at the cursor that starts `offset` into its own timeline.
    pub fn push_trimmed(
        self,
        offset: Time,
        duration: Duration,
        scene: impl Scene + 'static,
    ) -> Self {
        let cursor = self.cursor;
        self.defer(|builder| builder.place_checked(cursor, Some(duration), offset, scene))
    }

    /// Immediately checks and appends a scene for `duration`.
    pub fn try_push_for(
        mut self,
        duration: Duration,
        scene: impl Scene + 'static,
    ) -> crate::Result<Self> {
        let cursor = self.cursor;
        self.place_checked(cursor, Some(duration), Time::ZERO, scene)?;
        Ok(self)
    }

    /// Immediately checks and places a scene for `duration`.
    pub fn try_place_for(
        mut self,
        start: Time,
        duration: Duration,
        scene: impl Scene + 'static,
    ) -> crate::Result<Self> {
        self.place_checked(start, Some(duration), Time::ZERO, scene)?;
        Ok(self)
    }

    /// Immediately checks and appends a scene that starts `offset` into its own timeline.
    pub fn try_push_trimmed(
        mut self,
        offset: Time,
        duration: Duration,
        scene: impl Scene + 'static,
    ) -> crate::Result<Self> {
        let cursor = self.cursor;
        self.place_checked(cursor, Some(duration), offset, scene)?;
        Ok(self)
    }

    /// Appends a nested timeline at the cursor, using its own duration. Its picture
    /// is clipped and fitted into this timeline's design space; its audio joins this
    /// timeline's soundtrack, shifted and cropped to the clip.
    pub fn nest(self, child: Timeline) -> Self {
        let cursor = self.cursor;
        self.defer(|builder| builder.nest_checked(cursor, None, child))
    }

    /// Like [`nest`](Self::nest) with an explicit clip duration: the child's picture
    /// and audio are cropped to it, or padded with silence when it is longer.
    pub fn nest_for(self, duration: Duration, child: Timeline) -> Self {
        let cursor = self.cursor;
        self.defer(|builder| builder.nest_checked(cursor, Some(duration), child))
    }

    /// Places a nested timeline at an absolute time, like [`nest`](Self::nest).
    pub fn place_timeline(self, start: Time, child: Timeline) -> Self {
        self.defer(|builder| builder.nest_checked(start, None, child))
    }

    /// Immediately checks and appends a nested timeline at the cursor.
    pub fn try_nest(mut self, child: Timeline) -> crate::Result<Self> {
        let cursor = self.cursor;
        self.nest_checked(cursor, None, child)?;
        Ok(self)
    }

    /// Immediately checks and appends a nested timeline with an explicit duration.
    pub fn try_nest_for(mut self, duration: Duration, child: Timeline) -> crate::Result<Self> {
        let cursor = self.cursor;
        self.nest_checked(cursor, Some(duration), child)?;
        Ok(self)
    }

    /// Immediately checks and places a nested timeline.
    pub fn try_place_timeline(mut self, start: Time, child: Timeline) -> crate::Result<Self> {
        self.nest_checked(start, None, child)?;
        Ok(self)
    }

    fn nest_checked(
        &mut self,
        start: Time,
        duration: Option<Duration>,
        child: Timeline,
    ) -> crate::Result<()> {
        let id = self.clips.len();
        match duration {
            Some(duration) => {
                self.place_checked(start, Some(duration), Time::ZERO, child.visual())?
            }
            None => self.place_checked(start, None, Time::ZERO, child.visual())?,
        }
        let clip = self
            .clips
            .iter()
            .find(|clip| clip.id == id)
            .ok_or_else(|| on_track(self.index, "Nested clip was not placed"))?;
        let (start, duration) = (clip.start, clip.duration);
        let soundtrack = child.soundtrack();
        let sources =
            if soundtrack.master() != crate::Master::default() || !soundtrack.buses().is_empty() {
                vec![AudioClip::composition(child.clone()).duration(duration)]
            } else {
                soundtrack.clips().to_vec()
            };
        for source in sources {
            if let Some(source) = source.relocated(start, duration)? {
                self.audio.push(source);
            }
        }
        Ok(())
    }

    /// Adds an adjacent transition, retaining invalid input for validate().
    pub fn transition(
        self,
        effect: impl transition::Transition + 'static,
        duration: Duration,
    ) -> Self {
        self.defer(|builder| builder.transition_checked(effect, duration))
    }

    /// Places a scene at absolute ticks, retaining invalid input for validate().
    pub fn place(self, start: Time, scene: impl Scene + 'static) -> Self {
        self.defer(|builder| builder.place_checked(start, None, Time::ZERO, scene))
    }

    /// Advances the cursor, retaining invalid input for validate().
    pub fn gap(self, duration: Duration) -> Self {
        self.defer(|builder| builder.gap_checked(duration))
    }

    /// Immediately checks and applies one adjacent transition.
    pub fn try_transition(
        mut self,
        effect: impl transition::Transition + 'static,
        duration: Duration,
    ) -> crate::Result<Self> {
        self.transition_checked(effect, duration)?;
        Ok(self)
    }

    /// Immediately checks and places one scene.
    pub fn try_place(mut self, start: Time, scene: impl Scene + 'static) -> crate::Result<Self> {
        self.place_checked(start, None, Time::ZERO, scene)?;
        Ok(self)
    }

    /// Immediately checks and advances the cursor.
    pub fn try_gap(mut self, duration: Duration) -> crate::Result<Self> {
        self.gap_checked(duration)?;
        Ok(self)
    }

    /// Appends at the requested cursor; finite positive Scene::duration is required.
    pub fn try_push(self, scene: impl Scene + 'static) -> crate::Result<Self> {
        let cursor = self.cursor;
        self.try_place(cursor, scene)
    }

    /// Advances the next pushed clip into the previous clip by a positive duration.
    /// Only one adjacent pair may overlap; a pending transition must be consumed.
    fn transition_checked(
        &mut self,
        effect: impl transition::Transition + 'static,
        duration: Duration,
    ) -> crate::Result<()> {
        if self.pending.is_some() || duration <= Time::ZERO {
            return Err(on_track(
                self.index,
                "Transition needs positive duration and no pending transition",
            ));
        }
        let previous = self
            .clips
            .last()
            .ok_or_else(|| on_track(self.index, "Transition requires a preceding clip"))?;
        if align(self.cursor, self.rate)?.0 != previous.end() {
            return Err(on_track(self.index, "Transition cannot follow a gap"));
        }
        let start = self
            .cursor
            .ticks()
            .checked_sub(duration.ticks())
            .map(Time::from_ticks)
            .filter(|time| *time >= Time::ZERO)
            .ok_or_else(|| on_track(self.index, "Transition exceeds the preceding clip"))?;
        let aligned = align(start, self.rate)?.0;
        if aligned < previous.start
            || aligned >= previous.end()
            || previous
                .transition_in
                .as_ref()
                .is_some_and(|incoming| aligned < previous.start + incoming.duration)
        {
            return Err(on_track(
                self.index,
                "Transition would be empty or overlap another transition",
            ));
        }
        self.cursor = start;
        self.pending = Some(Arc::new(effect));
        Ok(())
    }

    /// Places at an absolute nonnegative tick time and sorts by snapped start.
    /// The cursor advances to max(previous cursor, requested end), never backwards.
    /// Invalid or rounding-collapsed clips and same-track overlaps fail immediately.
    fn place_checked(
        &mut self,
        start: Time,
        duration: Option<Duration>,
        offset: Time,
        scene: impl Scene + 'static,
    ) -> crate::Result<()> {
        if self.clips.len() >= MAX_CLIPS {
            return Err(on_track(self.index, "At most 16384 clips are allowed"));
        }
        if start < Time::ZERO {
            return Err(on_track(self.index, "Clip start must be nonnegative"));
        }
        if offset < Time::ZERO {
            return Err(on_track(self.index, "Clip offset must be nonnegative"));
        }
        let duration = duration.or_else(|| scene.duration()).ok_or_else(|| {
            on_track(
                self.index,
                "Scene has no finite duration; use push_for or with_duration",
            )
        })?;
        if duration <= Time::ZERO {
            return Err(on_track(self.index, "Clip duration must be positive"));
        }
        let requested_end =
            add_time(start, duration).map_err(|error| on_track(self.index, error))?;
        let (aligned_start, start_frame) =
            align(start, self.rate).map_err(|error| on_track(self.index, error))?;
        let (aligned_end, end_frame) =
            align(requested_end, self.rate).map_err(|error| on_track(self.index, error))?;
        if aligned_end <= aligned_start {
            return Err(on_track(
                self.index,
                "Clip becomes empty after frame alignment; increase its duration",
            ));
        }
        let index = self
            .clips
            .partition_point(|clip| clip.start <= aligned_start);
        let id = self.clips.len();
        let overlaps = index
            .checked_sub(1)
            .and_then(|previous| self.clips.get(previous))
            .filter(|clip| clip.end() > aligned_start)
            .or_else(|| {
                self.clips
                    .get(index)
                    .filter(|clip| clip.start < aligned_end)
            });
        let transition_in = if let Some(effect) = &self.pending {
            let previous = self
                .clips
                .last()
                .ok_or_else(|| on_track(self.index, "Transition requires a preceding clip"))?;
            if start != self.cursor
                || index != self.clips.len()
                || aligned_start < previous.start
                || aligned_start >= previous.end()
                || aligned_end < previous.end()
                || previous
                    .transition_in
                    .as_ref()
                    .is_some_and(|incoming| aligned_start < previous.start + incoming.duration)
            {
                return Err(on_track(
                    self.index,
                    "Incoming clip must cover the adjacent overlap without a triple overlap",
                ));
            }
            Some(TransitionIn {
                effect: effect.clone(),
                duration: previous.end() - aligned_start,
            })
        } else {
            None
        };
        if let Some(other) = overlaps.filter(|_| transition_in.is_none()) {
            return Err(on_track(
                self.index,
                format!(
                    "clip[{id}] [{}, {}) overlaps clip[{}] [{}, {}); use an explicit adjacent transition or separate track",
                    aligned_start.ticks(),
                    aligned_end.ticks(),
                    other.id,
                    other.start.ticks(),
                    other.end().ticks()
                ),
            ));
        }
        for (edge, requested, aligned, frame) in [
            (BoundaryEdge::Start, start, aligned_start, start_frame),
            (BoundaryEdge::End, requested_end, aligned_end, end_frame),
        ] {
            if requested != aligned {
                self.adjustments.push(BoundaryAdjustment {
                    track: Some(self.index),
                    clip: Some(id),
                    edge,
                    requested,
                    aligned,
                    frame,
                });
            }
        }
        self.clips.insert(
            index,
            Clip {
                id,
                start: aligned_start,
                duration: aligned_end - aligned_start,
                offset,
                scene: Arc::new(scene),
                transition_in,
            },
        );
        self.cursor = self.cursor.max(requested_end);
        self.pending = None;
        Ok(())
    }

    /// Advances the raw cursor, contributing empty time even when it trails all clips.
    fn gap_checked(&mut self, duration: Duration) -> crate::Result<()> {
        if self.pending.is_some() {
            return Err(on_track(
                self.index,
                "A pending transition must be followed by a clip, not a gap",
            ));
        }
        if duration < Time::ZERO {
            return Err(on_track(self.index, "Gap must be nonnegative"));
        }
        self.cursor =
            add_time(self.cursor, duration).map_err(|error| on_track(self.index, error))?;
        Ok(())
    }

    /// Requested append position, before frame alignment.
    pub fn cursor(&self) -> Time {
        self.cursor
    }

    pub(super) fn finish(
        mut self,
    ) -> crate::Result<(Track, Vec<BoundaryAdjustment>, Vec<AudioClip>)> {
        if !self.errors.is_empty() {
            return Err(Error::BuildErrors(self.errors.into_vec()));
        }
        if self.pending.is_some() {
            return Err(on_track(
                self.index,
                "Pending transition has no incoming clip",
            ));
        }
        let (duration, frame) =
            align(self.cursor, self.rate).map_err(|error| on_track(self.index, error))?;
        if duration != self.cursor {
            self.adjustments.push(BoundaryAdjustment {
                track: Some(self.index),
                clip: None,
                edge: BoundaryEdge::Cursor,
                requested: self.cursor,
                aligned: duration,
                frame,
            });
        }
        Ok((
            Track {
                clips: self.clips.into(),
                duration,
            },
            self.adjustments,
            self.audio,
        ))
    }
}
