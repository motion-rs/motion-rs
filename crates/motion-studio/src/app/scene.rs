//! Scene registration for the workbench: [`StudioScene`] pairs a composition with the
//! thread-safe builder, export capability and optional audio timeline that preview and
//! export workers share.

use motion_core::{Node, Result, SceneContext, Size};
use motion_render::Composition;
use std::path::PathBuf;
use std::sync::Arc;

/// A compiled Rust composition registered in the workbench. Builders run on worker threads.
/// Preview and export can invoke the same shared builder concurrently.
/// Explicit preview_only registrations remain preview-only. Avoid mutable playback
/// state, UI-only handles, wall clocks, and unseeded random generators.
#[derive(Clone)]
pub struct StudioScene {
    pub(crate) name: String,
    pub(crate) composition: Composition,
    pub(crate) definition: SceneDefinition,
    pub(crate) reload: Option<Arc<ReloadSource>>,
}

/// Files that a reloadable scene is built from and the function that rebuilds it.
pub(crate) struct ReloadSource {
    pub(crate) watch: Vec<PathBuf>,
    pub(crate) load: Box<dyn Fn() -> Result<StudioScene> + Send + Sync>,
}

#[derive(Clone)]
pub(crate) struct SceneDefinition {
    pub(crate) design_size: Size,
    pub(crate) build_scene: Arc<dyn Fn(&SceneContext<'_>) -> Node + Send + Sync>,
    pub(crate) video_export: bool,
    pub(crate) audio_timeline: Option<Arc<motion_core::Timeline>>,
}

impl StudioScene {
    /// Registers a Node scene for CPU/GPU preview and video export.
    ///
    /// Preview and export independently use the same builder, owned immutable
    /// resources and logical dimensions. Export reports progress and preserves
    /// cancellation and transactional destination protection. No scene
    /// conversion or implicit font discovery occurs. GPU requires the gpu feature.
    ///
    /// ```
    /// use motion_studio::prelude::*;
    /// use motion_core::prelude::*;
    /// use motion_core::{
    ///     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
    ///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
    /// };
    /// use motion_render::{Composition, RenderPrepare, RenderOptions, Renderer, render_still};
    /// use motion_studio::StudioScene;
    /// let scene = StudioScene::new(
    ///     "Geometry",
    ///     Composition::new(640, 360, motion_render::FrameClock::new(fps!(30), 150)),
    ///     Size::new(640.0, 360.0),
    ///     |context| Node::from(context.bounds().fill(Color::WHITE)),
    /// );
    /// assert!(scene.supports_video_export());
    /// ```
    pub fn new(
        name: impl Into<String>,
        composition: Composition,
        design_size: Size,
        build: impl Fn(&SceneContext<'_>) -> Node + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            composition,
            definition: SceneDefinition {
                design_size,
                build_scene: Arc::new(build),
                video_export: true,
                audio_timeline: None,
            },
            reload: None,
        }
    }

    /// Registers a Node scene for CPU/GPU preview with logical dimensions.
    ///
    /// The worker retains a FrameRenderer and its bounded scene cache.
    /// No fonts are discovered.
    /// GPU selection is explicit and requires the gpu feature. This registration
    /// opts out of video export; use [`Self::new`] for both preview and export.
    /// Dimensions validate at launch or session creation, before evaluation.
    ///
    /// ```
    /// use motion_studio::prelude::*;
    /// use motion_core::prelude::*;
    /// use motion_core::{
    ///     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
    ///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
    /// };
    /// use motion_render::{Composition, RenderPrepare, RenderOptions, Renderer, render_still};
    /// use motion_studio::StudioScene;
    /// let scene = StudioScene::preview_only(
    ///     "Geometry",
    ///     Composition::new(640, 360, motion_render::FrameClock::new(fps!(30), 150)),
    ///     Size::new(640.0, 360.0),
    ///     |context| Node::from(context.bounds().fill(Color::WHITE)),
    /// );
    /// assert!(!scene.supports_video_export());
    /// ```
    pub fn preview_only(
        name: impl Into<String>,
        composition: Composition,
        design_size: Size,
        build: impl Fn(&SceneContext<'_>) -> Node + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            composition,
            definition: SceneDefinition {
                design_size,
                build_scene: Arc::new(build),
                video_export: false,
                audio_timeline: None,
            },
            reload: None,
        }
    }

    /// Rebuilds this scene while the workbench runs whenever a watched file changes.
    ///
    /// The workbench polls the modification times of `watch` a few times per second and,
    /// once a change has settled, calls `load` on a worker thread. A successful result
    /// replaces the scene: the playhead, cues, preview settings and export settings are
    /// kept (clamped to the new length) and every cached frame is dropped. A failure
    /// keeps the previous scene and shows the error in the header, so a file saved with
    /// a mistake never closes or blanks the preview. The scene returned by `load` need
    /// not be reloadable itself. `load` must be deterministic and thread-safe.
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

    /// Registers a timeline with worker-prepared audio preview and export.
    /// Merely registering never mixes audio or opens an output device. Playback
    /// uses the audible-sample clock; template resources/backend/shutter remain.
    pub fn timeline(
        name: impl Into<String>,
        composition: Composition,
        timeline: motion_core::Timeline,
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
        let size = timeline.size();
        let timeline = Arc::new(timeline);
        let visual = timeline.clone();
        let mut scene = Self::new(name, composition, size, move |context| {
            visual.render(context)
        });
        let SceneDefinition { audio_timeline, .. } = &mut scene.definition;
        *audio_timeline = Some(timeline);
        Ok(scene)
    }

    /// Whether the workbench rebuilds this scene when watched files change.
    pub fn is_reloadable(&self) -> bool {
        self.reload.is_some()
    }

    /// Whether this registration enables video export.
    pub fn supports_video_export(&self) -> bool {
        matches!(
            &self.definition,
            SceneDefinition {
                video_export: true,
                ..
            }
        )
    }
}
