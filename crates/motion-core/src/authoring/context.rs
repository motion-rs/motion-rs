//! Stateless authoring time, design coordinates and optional shared computation.

use std::hash::Hash;
use std::sync::Arc;

use crate::{Animation, Duration, FrameRate, Node, Point, Random, Rect, Size, Time};

mod cache;
mod partition;
#[cfg(test)]
mod tests;

pub use cache::SceneCache;
pub use partition::{CacheLimits, CachePartition, RenderCache};

/// Immutable scene evaluation inputs in signed ticks and logical coordinates.
///
/// The caller supplies time explicitly and can reuse one [`SceneCache`] across
/// arbitrary frame orders and threads, without initializing rendering resources
/// or interpreting design dimensions as output pixels.
///
/// ```
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// use std::sync::Arc;
///
/// let cache = SceneCache::new(32);
/// let tween = Tween::new(0.0, 100.0, 1.seconds());
/// let evaluate = |time| {
///     let context = SceneContext::new(time, FrameRate::from(30), Size::new(640.0, 360.0), &cache)
///         .with_clip_duration(2.seconds());
///     let positions = context.cache(("dots", 8_u32), || {
///         (0..8_u32)
///             .map(|index| Point::new(context.random(("x", index)) * 640.0, 180.0))
///             .collect::<Vec<_>>()
///     });
///     let local = context.shifted(500.milliseconds());
///     assert_eq!(local.global_time(), time);
///     (local.sample(&tween), positions)
/// };
/// let (late, first) = evaluate(1500.milliseconds().after_origin());
/// let (early, second) = evaluate(500.milliseconds().after_origin());
/// let (again, third) = evaluate(1500.milliseconds().after_origin());
/// assert_eq!((late, early, again), (100.0, 0.0, 100.0));
/// assert!(Arc::ptr_eq(&first, &second) && Arc::ptr_eq(&first, &third));
/// ```
#[derive(Clone, Copy)]
pub struct SceneContext<'a> {
    quality: crate::RenderQuality,
    time: Time,
    global_time: Time,
    clip_duration: Option<Duration>,
    frame_rate: FrameRate,
    size: Size,
    nested_scene: bool,

    assets: Option<&'a crate::Assets>,

    cache: &'a SceneCache,

    diagnostics: Option<&'a crate::RequestDiagnostics>,
}

impl<'a> SceneContext<'a> {
    /// Creates root evaluation inputs with matching local/global time.
    ///
    /// Panics unless both design dimensions are finite and strictly positive.
    pub fn new(time: Time, frame_rate: FrameRate, size: Size, cache: &'a SceneCache) -> Self {
        Self::try_new(time, frame_rate, size, cache)
            .unwrap_or_else(|error| panic!("{}", error.message))
    }

    /// Creates evaluation inputs with recoverable design-size validation.
    pub fn try_new(
        time: Time,
        frame_rate: FrameRate,
        size: Size,
        cache: &'a SceneCache,
    ) -> Result<Self, crate::ValidationError> {
        if !size.width.is_finite()
            || size.width <= 0.0
            || !size.height.is_finite()
            || size.height <= 0.0
        {
            return Err(crate::ValidationError::invalid(
                "scene_context",
                "size",
                "scene design dimensions must be finite and positive",
            ));
        }
        Ok(Self {
            quality: crate::RenderQuality::Final,
            time,
            global_time: time,
            clip_duration: None,
            frame_rate,
            size,
            nested_scene: false,
            assets: None,
            cache,
            diagnostics: None,
        })
    }

    /// Selects preview approximations, preserved by nested and sampled contexts.
    pub fn with_quality(mut self, quality: crate::RenderQuality) -> Self {
        self.quality = quality;
        self
    }

    /// Explicit rendering quality; manually constructed contexts default to Final.
    pub fn render_quality(&self) -> crate::RenderQuality {
        self.quality
    }

    /// Always available resources; an unconfigured context uses an empty owner.
    pub fn assets(&self) -> &'a crate::Assets {
        self.assets.unwrap_or(&self.cache.empty_assets)
    }

    /// Attaches the common resource owner without reading or decoding.
    pub fn with_assets(mut self, assets: &'a crate::Assets) -> Self {
        self.assets = Some(assets);
        self
    }

    /// Attaches a request-owned sink suitable for explicit background resource reads.
    pub fn with_diagnostics(mut self, diagnostics: &'a crate::RequestDiagnostics) -> Self {
        self.diagnostics = Some(diagnostics);
        self
    }

    /// Diagnostic scope retained by shifted, sampled and nested contexts.
    pub fn diagnostics(&self) -> Option<&'a crate::RequestDiagnostics> {
        self.diagnostics
    }

    /// Sets finite clip metadata; zero denotes an instantaneous clip.
    ///
    /// Panics for negative duration. Sampling time is not clamped to the clip.
    pub fn with_clip_duration(mut self, duration: Duration) -> Self {
        assert!(
            duration >= Duration::ZERO,
            "clip duration must be nonnegative"
        );
        self.clip_duration = Some(duration);
        self
    }

    /// Sets clip metadata with a recoverable negative-duration diagnostic.
    pub fn try_with_clip_duration(
        mut self,
        duration: Duration,
    ) -> Result<Self, crate::ValidationError> {
        if duration < Duration::ZERO {
            return Err(crate::ValidationError::invalid(
                "scene_context",
                "clip_duration",
                "clip duration must be nonnegative",
            ));
        }
        self.clip_duration = Some(duration);
        Ok(self)
    }

    /// Signed clip-local time.
    pub fn time(&self) -> Time {
        self.time
    }

    /// Clip-local time in floating-point seconds, for animation mathematics.
    pub fn seconds(&self) -> f64 {
        self.time.as_seconds()
    }

    /// Root evaluation time, unaffected by local shifts.
    pub fn global_time(&self) -> Time {
        self.global_time
    }

    /// Explicit clip duration, absent for an unconstrained scene.
    pub fn clip_duration(&self) -> Option<Duration> {
        self.clip_duration
    }

    /// Clip-local time as a duration since the clip start.
    pub fn elapsed(&self) -> Duration {
        self.time.since_origin()
    }

    /// Clamped local progress; unconstrained scenes return zero.
    ///
    /// A zero-duration clip jumps from zero to one at local time zero.
    pub fn progress(&self) -> f64 {
        self.clip_duration
            .map_or(0.0, |duration| self.progress_in(Time::ZERO, duration))
    }

    /// Clamped progress in a local interval, using integer boundary comparisons.
    ///
    /// Zero duration jumps at start. Panics for negative duration. Wide integer
    /// differences allow queries across opposite signed tick extremes.
    pub fn progress_in(&self, start: Time, duration: Duration) -> f64 {
        assert!(
            duration >= Duration::ZERO,
            "interval duration must be nonnegative"
        );
        let elapsed = i128::from(self.time.ticks()) - i128::from(start.ticks());
        if elapsed < 0 {
            0.0
        } else if elapsed >= i128::from(duration.ticks()) {
            1.0
        } else {
            elapsed as f64 / duration.ticks() as f64
        }
    }

    /// Eased local interval progress, with the same clamping as progress_in.
    pub fn ease_in(&self, start: Time, duration: Duration, easing: crate::Easing) -> f64 {
        easing.apply(self.progress_in(start, duration))
    }

    /// Exact rational authoring frame rate shared by still, preview and export clocks.
    pub fn frame_rate(&self) -> FrameRate {
        self.frame_rate
    }

    pub(crate) fn for_timeline(&self, size: Size, frame_rate: FrameRate) -> Self {
        Self {
            frame_rate,
            size,
            nested_scene: true,
            ..*self
        }
    }

    pub(crate) fn is_nested_scene(&self) -> bool {
        self.nested_scene
    }

    /// Converts a signed frame count directly to ticks without cumulative rounding.
    pub fn frames(&self, count: i64) -> Duration {
        Duration::from_frame(count, self.frame_rate)
    }

    /// Samples an animation using local, not global, time.
    pub fn sample<Value>(&self, animation: &(impl Animation<Value> + ?Sized)) -> Value {
        animation.sample(self.time.since_origin())
    }

    /// Subtracts an offset from local time, preserving global time and clip metadata.
    ///
    /// Offsets can be negative. Panics if the resulting local ticks overflow.
    pub fn shifted(&self, offset: Duration) -> Self {
        Self {
            time: self.time - offset,
            ..*self
        }
    }

    /// Subtracts a local offset without panicking on tick overflow.
    pub fn try_shifted(&self, offset: Duration) -> Result<Self, crate::ValidationError> {
        let ticks = self
            .time
            .ticks()
            .checked_sub(offset.ticks())
            .ok_or_else(|| {
                crate::ValidationError::invalid(
                    "scene_context",
                    "time",
                    "Shifted local ticks overflow",
                )
            })?;
        Ok(Self {
            time: Time::from_ticks(ticks),
            ..*self
        })
    }

    pub(crate) fn resampled(&self, offset: Duration) -> Result<Self, crate::ValidationError> {
        let shift = |time: Time| {
            time.ticks()
                .checked_add(offset.ticks())
                .map(Time::from_ticks)
                .ok_or_else(|| {
                    crate::ValidationError::invalid(
                        "motion_blur",
                        "time",
                        "Subframe local/global ticks overflow",
                    )
                })
        };
        Ok(Self {
            time: shift(self.time)?,
            global_time: shift(self.global_time)?,
            ..*self
        })
    }

    /// Resamples a local subtree at 1..=64 centered integer shutter midpoints.
    /// Both local/global clocks move; clip metadata, resources and cache persist.
    /// Signed samples outside a clip are supplied unchanged for caller-owned gating.
    pub fn try_motion_blur(
        &self,
        shutter: Duration,
        samples: u32,
        build: impl FnMut(&SceneContext<'_>) -> Node,
    ) -> Result<Node, crate::ValidationError> {
        crate::authoring::sampling::capture(self, shutter, samples, build)
    }

    /// Captures shutter samples, deferring invalid sampling parameters to compilation.
    pub fn motion_blur(
        &self,
        shutter: Duration,
        samples: u32,
        build: impl FnMut(&SceneContext<'_>) -> Node,
    ) -> Node {
        self.try_motion_blur(shutter, samples, build)
            .unwrap_or_else(super::scene::failure)
    }

    /// Builds only in [start, start + duration), with shifted local clip inputs.
    ///
    /// Outside the interval, including zero duration, returns an empty node and
    /// does not call build. Global time stays unchanged. Integer differences avoid
    /// overflowing the endpoint sum. Panics for negative duration.
    pub fn during(
        &self,
        start: Time,
        duration: Duration,
        build: impl FnOnce(&SceneContext<'a>) -> Node,
    ) -> Node {
        assert!(
            duration >= Duration::ZERO,
            "interval duration must be nonnegative"
        );
        let elapsed = i128::from(self.time.ticks()) - i128::from(start.ticks());
        if elapsed < 0 || elapsed >= i128::from(duration.ticks()) {
            Node::empty()
        } else {
            build(&Self {
                nested_scene: true,
                ..self
                    .shifted(start.since_origin())
                    .with_clip_duration(duration)
            })
            .with_scene_time(Time::from_ticks(elapsed as i64))
        }
    }

    /// Evaluates a child scene starting at a parent-local offset.
    ///
    /// Finite children use their half-open duration. Infinite children are active
    /// from offset onward and clear inherited clip-duration metadata. The parent
    /// context itself is never clamped; its caller controls the parent interval.
    /// Panics for invalid child duration or unrepresentable shifted local ticks.
    /// Timeline children clip and contain their own design viewport;
    /// changing the child frame grid never quantizes the elapsed parent ticks.
    pub fn embed(&self, scene: &dyn crate::authoring::Scene, offset: Time) -> Node {
        match scene.duration() {
            Some(duration) => self.during(offset, duration, |context| scene.render(context)),
            None if self.time < offset => Node::empty(),
            None => scene.render(&Self {
                clip_duration: None,
                nested_scene: true,
                ..self.shifted(offset.since_origin())
            }),
        }
    }

    /// Logical design dimensions, not device pixels.
    pub fn size(&self) -> Size {
        self.size
    }

    /// Opposite-corner logical bounds at the origin.
    pub fn bounds(&self) -> Rect {
        self.size.to_rect()
    }

    /// Center of the logical design bounds.
    pub fn center(&self) -> Point {
        self.bounds().center()
    }

    /// Measures content or declared layout in parent coordinates, independently of time.
    ///
    /// Uses default structural limits without rendering or f32 pixel conversion.
    /// For explicit budgets use [`Node::measure`]. Leaves supply their own bounds
    /// with the shared explicit font environment. This does not invoke drawing
    /// or infer filter extents; command validation occurs during compilation.
    pub fn measure(&self, node: &Node) -> Result<Rect, crate::ValidationError> {
        let drawing = crate::DrawContext::default().with_assets(self.assets());
        node.measure_with(&drawing, crate::ValidationLimits::default())
    }

    /// Measures like [`measure`](Self::measure) but substitutes `fallback` when measurement fails,
    /// for example `context.measure_or(&label, Rect::default())`. The failure still surfaces
    /// when the node is compiled.
    pub fn measure_or(&self, node: &Node, fallback: Rect) -> Rect {
        self.measure(node).unwrap_or(fallback)
    }

    /// Places a measured snapshot next to a reference in parent coordinates.
    ///
    /// Vertical sides center horizontally; horizontal sides center vertically.
    /// Finite negative gaps intentionally overlap. Returns errors for unknown
    /// geometry, invalid gaps or unrepresentable offsets; inputs stay unchanged.
    /// This is immediate snapshot positioning, not a live constraint to reference.
    ///
    /// ```
    /// use motion_core::prelude::*;
    /// use motion_core::{
    ///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
    ///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
    /// };
    /// let cache = SceneCache::new(0);
    /// let context = SceneContext::new(
    ///     Time::ZERO,
    ///     FrameRate::from(30),
    ///     Size::new(200.0, 100.0),
    ///     &cache,
    /// );
    /// let reference = Node::from(Rect::new(10.0, 10.0, 50.0, 30.0).fill(Color::WHITE));
    /// let card = Node::from(Rect::new(0.0, 0.0, 8.0, 4.0).fill(Color::WHITE));
    /// let card = context.next_to(card, &reference, Side::Right, 6.0);
    /// assert_eq!(context.measure(&card)?, Rect::new(56.0, 18.0, 64.0, 22.0));
    /// let card = context.align_to(card, &reference, Side::Bottom);
    /// assert_eq!(context.measure(&card)?, Rect::new(56.0, 26.0, 64.0, 30.0));
    /// # Ok::<(), motion_core::ValidationError>(())
    /// ```
    pub fn try_next_to(
        &self,
        node: impl Into<Node>,
        reference: &Node,
        side: crate::authoring::Side,
        gap: f64,
    ) -> Result<Node, crate::ValidationError> {
        if !gap.is_finite() {
            return Err(crate::ValidationError::invalid(
                "layout",
                "gap",
                "Relative layout gap must be finite",
            ));
        }
        let node = node.into();
        let offset = side.adjacent(self.measure(&node)?, self.measure(reference)?, gap);
        self.translated_layout(node, offset)
    }

    /// Aligns one measured edge with a reference, preserving the other axis.
    ///
    /// Uses the already transformed parent-space AABBs. Existing position,
    /// rotation, scale and pivot remain intact; only translation is added.
    pub fn try_align_to(
        &self,
        node: impl Into<Node>,
        reference: &Node,
        edge: crate::authoring::Side,
    ) -> Result<Node, crate::ValidationError> {
        let node = node.into();
        let offset = edge.aligned(self.measure(&node)?, self.measure(reference)?);
        self.translated_layout(node, offset)
    }

    /// Measures and places the snapshot next to its reference during compilation.
    pub fn next_to(
        &self,
        node: impl Into<Node>,
        reference: &Node,
        side: super::Side,
        gap: f64,
    ) -> Node {
        node.into().next_to(reference, side, gap)
    }

    /// Aligns one edge using the current compile-time font and drawing environment.
    pub fn align_to(&self, node: impl Into<Node>, reference: &Node, edge: super::Side) -> Node {
        node.into().align_to(reference, edge)
    }

    fn translated_layout(
        &self,
        node: Node,
        offset: crate::Vector2,
    ) -> Result<Node, crate::ValidationError> {
        use crate::NodeModifiers;
        if !offset.x.is_finite() || !offset.y.is_finite() {
            return Err(crate::ValidationError::invalid(
                "layout",
                "translation",
                "Relative layout offset must remain finite",
            ));
        }
        let node = node.translate(offset.x, offset.y);
        self.measure(&node)?;
        Ok(node)
    }

    /// Returns a key-derived value in [0, 1), independent of time and cache state.
    pub fn random(&self, key: impl Hash) -> f64 {
        self.random_generator(key).next_float()
    }

    /// Starts a fresh key-derived stream, independent of all other evaluations.
    pub fn random_generator(&self, key: impl Hash) -> Random {
        Random::from_key(key)
    }

    /// Reuses a pure value by calling location, value type and an equal typed key.
    ///
    /// Keys must distinguish all inputs, including resource revisions. Keys are
    /// retained, so use owned data or static references rather than borrowed locals.
    /// Hash collisions are resolved by key type and equality.
    /// Builders may be rerun after eviction, clear, disabled caching or races.
    #[track_caller]
    pub fn cache<Key: Eq + Hash + Send + Sync + 'static, Value: Send + Sync + 'static>(
        &self,
        key: Key,
        build: impl FnOnce() -> Value,
    ) -> Arc<Value> {
        self.cache.get(key, build)
    }

    /// Like [`cache`](Self::cache) for a fallible builder, for example a validated
    /// `build()`. A failure is returned and not retained, so the next call tries again.
    #[track_caller]
    pub fn try_cache<
        Key: Eq + Hash + Send + Sync + 'static,
        Value: Send + Sync + 'static,
        Error,
    >(
        &self,
        key: Key,
        build: impl FnOnce() -> Result<Value, Error>,
    ) -> Result<Arc<Value>, Error> {
        self.cache.try_get(key, build)
    }

    /// Reuses a pure static subtree without exposing its shared Arc handle.
    #[track_caller]
    pub fn memo<Key: Eq + Hash + Send + Sync + 'static, Output: Into<Node>>(
        &self,
        key: Key,
        build: impl FnOnce() -> Output,
    ) -> Node {
        self.cache(key, || build().into()).as_ref().clone()
    }

    /// Caches a pure value with an explicit logical retained-byte estimate.
    /// Cost includes the key, value and their owned heap payloads, with a minimum
    /// of their combined inline sizes. Entry metadata, allocator overhead and
    /// shared external pins are excluded.
    /// Oversized values are returned without retention; cost runs outside the lock.
    /// Keys retain the same equality-checked, callsite-scoped contract as cache().
    #[track_caller]
    pub fn cache_with_cost<Key: Eq + Hash + Send + Sync + 'static, Value: Send + Sync + 'static>(
        &self,
        key: Key,
        build: impl FnOnce() -> Value,
        cost: impl FnOnce(&Value) -> usize,
    ) -> Arc<Value> {
        self.cache.get_with_cost(key, build, cost)
    }
}
