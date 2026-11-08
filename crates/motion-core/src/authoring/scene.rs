//! Stateless scenes and explicit durations.

use super::*;

#[cfg(test)]
mod authoring_helpers_tests;

/// Converts ordinary drawable snapshots or recoverable scene failures.
pub trait IntoSceneOutput {
    /// Preserves the original failure category and source.
    fn into_scene_output(self) -> crate::Result<Node>;
}

impl<Output: Into<Node>> IntoSceneOutput for Output {
    fn into_scene_output(self) -> crate::Result<Node> {
        Ok(self.into())
    }
}

impl<Output: Into<Node>, Failure: Into<crate::Error>> IntoSceneOutput
    for std::result::Result<Output, Failure>
{
    fn into_scene_output(self) -> crate::Result<Node> {
        self.map(Into::into).map_err(Into::into)
    }
}

struct SceneFailure {
    error: Arc<crate::Error>,
    report: Arc<crate::Error>,
}

impl SceneFailure {
    fn new(error: crate::Error) -> Self {
        let error = Arc::new(error);
        let report = Arc::new(crate::Error::BuildErrors(vec![error.clone()]));
        Self { error, report }
    }

    fn diagnostic(&self, context: &DrawContext) -> ValidationError {
        context.diagnostics().record(self.report.clone());
        if let Some(error) = self.error.downcast_ref::<ValidationError>() {
            return error.clone();
        }
        ValidationError::invalid("scene", "output", self.error.to_string())
    }
}

pub(crate) fn failure(error: impl Into<crate::Error>) -> Node {
    Node::from(SceneFailure::new(error.into()))
}

impl Drawable for SceneFailure {
    fn bounds(&self, _: &DrawContext) -> Rect {
        Rect::new(0.0, 0.0, 0.0, 0.0)
    }

    fn try_bounds(&self, context: &DrawContext) -> Result<Rect, ValidationError> {
        Err(self.diagnostic(context))
    }

    fn draw(
        &self,
        context: &DrawContext,
        _: &mut crate::Canvas<'_>,
    ) -> Result<(), ValidationError> {
        Err(self.diagnostic(context))
    }
}

/// Gives closures the context bound needed for inference and defers failures to rendering.
pub fn scene<Build, Output>(build: Build) -> impl Fn(&SceneContext<'_>) -> Node + Send + Sync
where
    Build: Fn(&SceneContext<'_>) -> Output + Send + Sync,
    Output: IntoSceneOutput,
{
    move |context| match build(context).into_scene_output() {
        Ok(node) => node,
        Err(error) => {
            let failure = SceneFailure::new(error);
            if let Some(diagnostics) = context.diagnostics() {
                diagnostics.record(failure.report.clone());
            }
            Node::from(failure)
        }
    }
}

/// Groups heterogeneous drawable or node values in source order.
#[macro_export]
macro_rules! group {
    () => { $crate::Node::empty() };
    ($($child:expr),* $(,)?) => {
        $crate::Node::group([$($crate::Node::from($child)),*])
    };
}

/// A stateless scene evaluated from explicit authoring inputs.
///
/// Closures implement this canonical root [`crate::Scene`] protocol.
///
/// ```
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
///
/// let child = (|context: &SceneContext<'_>| {
///     assert_eq!(context.time(), 1.seconds());
///     assert_eq!(context.global_time(), 2.seconds());
///     Node::from(Rect::new(0.0, 0.0, 32.0, 24.0).fill(Color::rgb(20, 150, 125)))
/// })
/// .with_duration(2.seconds());
/// let cache = SceneCache::new(0);
/// let context = SceneContext::new(
///     2.seconds(),
///     FrameRate::from(30),
///     Size::new(640.0, 360.0),
///     &cache,
/// );
/// let node = context.during(Time::ZERO, 4.seconds(), |local| {
///     local.embed(&child, 1.seconds())
/// });
/// let frame = node.compile(
///     context.size(),
///     PixelSize::new(64, 36),
///     ValidationLimits::default(),
/// )?;
/// # Ok::<(), motion_core::ValidationError>(())
/// ```
pub trait Scene: Send + Sync {
    /// Validates deferred authoring errors without evaluating a frame.
    fn validate(&self) -> crate::Result<()> {
        Ok(())
    }

    /// Builds one snapshot without retaining previous-frame state.
    fn render(&self, context: &SceneContext<'_>) -> Node;

    /// Natural finite duration, or None when a caller must provide one.
    fn duration(&self) -> Option<Duration> {
        None
    }

    /// Returns pure audio metadata for precomposition/export, not playback.
    /// Default scenes are silent; callers must preserve local clip coordinates.
    fn audio_clips(&self) -> crate::Result<Vec<crate::AudioClip>> {
        Ok(Vec::new())
    }

    /// Optional nested timeline ownership, preserved when its audio is relocated.
    fn audio_composition(&self) -> Option<&crate::Timeline> {
        None
    }

    /// Overrides clip metadata without retiming or clamping the callback.
    ///
    /// Panics for negative duration. Zero duration has an empty active interval.
    fn with_duration(self, duration: Duration) -> WithDuration<Self>
    where
        Self: Sized,
    {
        assert!(duration >= Time::ZERO, "scene duration must be nonnegative");
        WithDuration {
            scene: self,
            duration,
        }
    }
}

impl<Callback> Scene for Callback
where
    Callback: Fn(&SceneContext<'_>) -> Node + Send + Sync,
{
    fn render(&self, context: &SceneContext<'_>) -> Node {
        self(context)
    }
}

impl Scene for Arc<dyn Scene> {
    fn audio_composition(&self) -> Option<&crate::Timeline> {
        self.as_ref().audio_composition()
    }

    fn validate(&self) -> crate::Result<()> {
        self.as_ref().validate()
    }

    fn render(&self, context: &SceneContext<'_>) -> Node {
        self.as_ref().render(context)
    }

    fn duration(&self) -> Option<Duration> {
        self.as_ref().duration()
    }

    fn audio_clips(&self) -> crate::Result<Vec<crate::AudioClip>> {
        self.as_ref().audio_clips()
    }
}

/// A scene with an explicit finite authoring duration.
pub struct WithDuration<Content> {
    scene: Content,
    duration: Duration,
}

impl<Content: Scene> Scene for WithDuration<Content> {
    fn audio_composition(&self) -> Option<&crate::Timeline> {
        self.scene.audio_composition()
    }

    fn validate(&self) -> crate::Result<()> {
        self.scene.validate()
    }

    fn render(&self, context: &SceneContext<'_>) -> Node {
        self.scene.render(context)
    }

    fn duration(&self) -> Option<Duration> {
        Some(self.duration)
    }

    fn audio_clips(&self) -> crate::Result<Vec<crate::AudioClip>> {
        self.scene
            .audio_clips()?
            .iter()
            .filter_map(|clip| clip.relocated(Time::ZERO, self.duration).transpose())
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}

/// Explicit sound-declaration scene; the time-zero metadata pass is cached once.
pub struct SoundScene<Build> {
    build: Build,
    duration: Duration,
    size: crate::Size,
    rate: crate::FrameRate,
    sounds: std::sync::Mutex<Option<Vec<crate::AudioClip>>>,
}

/// Opts a scene into local `SceneContext::sound` declarations without changing silent closure scenes.
/// The declaration pass has time zero and no rendering/device initialization.
pub fn scene_with_sound<Build, Output>(
    build: Build,
    duration: Duration,
    size: crate::Size,
    rate: impl Into<crate::FrameRate>,
) -> SoundScene<Build>
where
    Build: Fn(&SceneContext<'_>) -> Output + Send + Sync,
    Output: IntoSceneOutput,
{
    SoundScene {
        build,
        duration,
        size,
        rate: rate.into(),
        sounds: std::sync::Mutex::new(None),
    }
}

impl<Build, Output> Scene for SoundScene<Build>
where
    Build: Fn(&SceneContext<'_>) -> Output + Send + Sync,
    Output: IntoSceneOutput,
{
    fn validate(&self) -> crate::Result {
        if self.duration <= Time::ZERO || self.duration.as_seconds() > 86400.0 {
            return Err(ValidationError::invalid(
                "scene_audio",
                "duration",
                "Require duration in (0,24h]",
            )
            .into());
        }
        SceneContext::try_new(
            Time::ZERO,
            self.rate,
            self.size,
            &crate::SceneCache::default(),
        )?;
        Ok(())
    }

    fn duration(&self) -> Option<Duration> {
        Some(self.duration)
    }

    fn render(&self, context: &SceneContext<'_>) -> Node {
        match (self.build)(context).into_scene_output() {
            Ok(node) => node,
            Err(error) => failure(error),
        }
    }

    fn audio_clips(&self) -> crate::Result<Vec<crate::AudioClip>> {
        self.validate()?;
        let mut cached = self.sounds.lock().map_err(|_| {
            ValidationError::invalid("scene_audio", "lock", "Sound scene cache is poisoned")
        })?;
        if let Some(sounds) = cached.as_ref() {
            return Ok(sounds.clone());
        }
        let cache = crate::SceneCache::default();
        let sounds = std::sync::Mutex::new(Vec::new());
        let context = SceneContext::try_new(Time::ZERO, self.rate, self.size, &cache)?
            .with_clip_duration(self.duration)
            .collecting_sounds(&sounds);
        (self.build)(&context).into_scene_output()?;
        let sounds = sounds.into_inner().map_err(|_| {
            ValidationError::invalid("scene_audio", "lock", "Sound declaration lock is poisoned")
        })?;
        *cached = Some(sounds.clone());
        Ok(sounds)
    }
}
