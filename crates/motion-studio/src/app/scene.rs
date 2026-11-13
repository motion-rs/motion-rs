//! Scene registration for the workbench: [`StudioScene`] pairs a render template with the
//! single timeline that preview, audio playback and export share.

use motion_core::{Result, Timeline};
use motion_render::Composition;
use std::path::PathBuf;
use std::sync::Arc;

/// A timeline registered in the workbench together with its render template. Preview,
/// audio playback and export all read the same [`Timeline`], so
/// what plays is what exports. Rendering runs on worker threads: avoid mutable playback
/// state, UI-only handles, wall clocks and unseeded random generators in scene builders.
#[derive(Clone)]
pub struct StudioScene {
    pub(crate) name: String,
    pub(crate) composition: Composition,
    pub(crate) timeline: Arc<Timeline>,
    pub(crate) reload: Option<Arc<ReloadSource>>,
}

/// Files that a reloadable scene is built from and the function that rebuilds it.
pub(crate) struct ReloadSource {
    pub(crate) watch: Vec<PathBuf>,
    pub(crate) load: Box<dyn Fn() -> Result<StudioScene> + Send + Sync>,
}

impl StudioScene {
    /// Registers a timeline for preview, audio playback and export.
    ///
    /// `composition` is the render template (output size, limits, backend and shutter);
    /// its frame clock is replaced by the timeline's exact rate and length. Registering
    /// never mixes audio or opens an output device, and playback uses the audible-sample
    /// clock. The same timeline feeds preview and export, which report progress and keep
    /// cancellation and transactional destination protection. No implicit font discovery
    /// occurs. GPU selection is explicit and requires the gpu feature.
    ///
    /// # Errors
    ///
    /// Fails when the timeline's audio cannot be fitted to its frame grid or when it has
    /// more than `u32::MAX` frames.
    ///
    /// ```
    /// use motion_core::prelude::*;
    /// use motion_render::Composition;
    /// use motion_studio::StudioScene;
    /// # fn main() -> motion_core::Result {
    /// let timeline = Timeline::try_new(Size::new(640.0, 360.0), fps!(30))?
    ///     .try_track(|track| {
    ///         let scene = (|context: &SceneContext<'_>| Node::from(context.bounds().fill(Color::WHITE)))
    ///             .with_duration(5.seconds());
    ///         track.try_push(scene)
    ///     })?
    ///     .build()?;
    /// let scene = StudioScene::new(
    ///     "Geometry",
    ///     Composition::new(640, 360, motion_render::FrameClock::new(fps!(30), 150)),
    ///     timeline,
    /// )?;
    /// # let _ = scene;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(
        name: impl Into<String>,
        composition: Composition,
        timeline: Timeline,
    ) -> Result<Self> {
        let timeline = motion_encode::audio::fit_timeline(&timeline)?.into_owned();
        let count = u32::try_from(timeline.frame_count()).map_err(|_| {
            motion_core::ValidationError::invalid(
                "audio",
                "frames",
                "Studio timeline exceeds u32 frames",
            )
        })?;
        let composition = composition.with_clock(motion_render::FrameClock::new(
            timeline.frame_rate(),
            u64::from(count),
        ));
        Ok(Self {
            name: name.into(),
            composition,
            timeline: Arc::new(timeline),
            reload: None,
        })
    }

    /// Rebuilds this scene while the workbench runs whenever a watched file changes.
    ///
    /// The workbench polls the modification times and lengths of `watch` a few times per second and,
    /// once a change has settled, calls `load` on a worker thread. A successful result
    /// replaces the scene: the playhead, cues, preview settings and export settings are
    /// kept (clamped to the new length) and every cached frame is dropped. A failure
    /// keeps the previous scene and shows the error in the header, so a file saved with
    /// a mistake never closes or blanks the preview. The scene returned by `load` need
    /// not be reloadable itself. If it is, its new watch list and loader replace the
    /// previous ones. Results superseded by another watched change are discarded.
    /// `load` must be deterministic and thread-safe.
    pub fn reloadable(
        mut self,
        watch: impl IntoIterator<Item = PathBuf>,
        load: impl Fn() -> Result<StudioScene> + Send + Sync + 'static,
    ) -> Self {
        self.reload = Some(Arc::new(ReloadSource {
            watch: watch.into_iter().collect(),
            load: Box::new(load),
        }));
        self
    }

    /// Whether the workbench rebuilds this scene when watched files change.
    pub fn is_reloadable(&self) -> bool {
        self.reload.is_some()
    }
}

#[cfg(any(test, feature = "studio"))]
impl StudioScene {
    /// Registers a closure scene that lasts as long as the composition's frame clock.
    pub(crate) fn from_render(
        name: impl Into<String>,
        composition: Composition,
        design_size: motion_core::Size,
        render: impl Fn(&motion_core::SceneContext<'_>) -> motion_core::Node + Send + Sync + 'static,
    ) -> Result<Self> {
        let rate = composition.frame_rate();
        let frames = i64::try_from(composition.frame_count())
            .map_err(|error| motion_core::Error::Parameters(Box::new(error)))?;
        let duration = motion_core::Time::from_frame(frames, rate)
            .since_origin()
            .magnitude();
        let timeline = Timeline::try_new(design_size, rate)?
            .try_track(|track| track.try_push(motion_core::Scene::with_duration(render, duration)))?
            .build()?;
        Self::new(name, composition, timeline)
    }
}
