//! Compilation of validated documents into engine timelines with their look.

use crate::document::{Document, LookSpec, SceneSpec, TrackItem, TransitionEffect};
use crate::element::Element;
use element::Built;
use motion_core::{
    CrossFade, Duration, Iris, Node, NodeModifiers, PixelSize, Push, Scene, SceneContext, Signal,
    Size, Slide, Timeline, TrackBuilder, ValidationError, Wipe, ZoomBlur,
};
use motion_render::{Look, RenderOptions};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

mod audio;
mod binding;
mod captions;
mod content;
mod diagram;
mod element;
mod expr;
mod failure;
mod formula;
mod morph;
mod particles;
mod style;
mod text;
mod value;

/// Largest accepted time, duration or offset in seconds (about 11.5 days).
pub(crate) const MAX_SECONDS: f64 = 1e6;

/// Largest accepted design width or height.
const MAX_DESIGN_EXTENT: f64 = 65_536.0;

/// Most diagnostics retained from one compilation.
const MAX_ERRORS: usize = 256;

/// Inputs that are not part of the document itself.
#[derive(Clone, Debug, Default)]
pub struct CompileOptions {
    /// Directory against which relative font, image, SVG and audio paths resolve.
    /// Without it, images and SVG resolve against the render assets root and
    /// audio against the process working directory.
    pub base_dir: Option<PathBuf>,
}

impl CompileOptions {
    /// Resolves relative resource paths against `directory`.
    pub fn base_dir(mut self, directory: impl Into<PathBuf>) -> Self {
        self.base_dir = Some(directory.into());
        self
    }
}

/// A compiled document: an engine timeline plus the look's display stage and fonts.
#[derive(Clone, Debug)]
pub struct CompiledDocument {
    timeline: Timeline,
    look: Option<Look>,
    resources: Vec<PathBuf>,
    #[cfg(feature = "text")]
    fonts: Option<(motion_content::FontLibrary, PathBuf)>,
}

impl CompiledDocument {
    /// Files the document refers to: fonts, images, SVG, captions and audio, resolved
    /// against the base directory, in first-use order without duplicates. [`load`]
    /// puts the document file itself first, so the list is what a file watcher needs.
    pub fn resources(&self) -> &[PathBuf] {
        &self.resources
    }

    /// Returns the timeline for rendering, export or Studio.
    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    /// Returns the selected look, whose scene stages are already applied.
    pub fn look(&self) -> Option<&Look> {
        self.look.as_ref()
    }

    /// Applies the look's display transform and the document's fonts, if any.
    ///
    /// Text elements, captions and formulas carry the document fonts themselves.
    /// Rulers, callouts and charts draw text through the render font environment,
    /// which this installs; it replaces the fonts of any resources already set.
    pub fn render_options(&self, options: RenderOptions) -> RenderOptions {
        let options = match &self.look {
            Some(look) => look.render_options(options),
            None => options,
        };
        self.with_fonts(options)
    }

    #[cfg(feature = "text")]
    fn with_fonts(&self, options: RenderOptions) -> RenderOptions {
        let Some((fonts, root)) = &self.fonts else {
            return options;
        };
        let assets = match &options.resources {
            Some(assets) => Ok(assets.clone()),
            None => motion_core::Assets::new(root),
        };
        match assets {
            Ok(assets) => options.resources(assets.with_fonts(fonts.clone())),
            Err(_) => options,
        }
    }

    #[cfg(not(feature = "text"))]
    fn with_fonts(&self, options: RenderOptions) -> RenderOptions {
        options
    }

    /// Separates the timeline and look.
    pub fn into_parts(self) -> (Timeline, Option<Look>) {
        (self.timeline, self.look)
    }
}

impl Document {
    /// Validates every property and builds the engine timeline.
    ///
    /// Element diagnostics are collected together and returned as
    /// [`motion_core::Error::BuildErrors`] of [`ValidationError`]s whose paths
    /// follow the JSON structure, such as `tracks[0].items[1].scene.content.circle.radius`.
    pub fn compile(&self, options: &CompileOptions) -> motion_core::Result<CompiledDocument> {
        self.check_version()?;
        let mut compiler = Compiler::new(options.base_dir.as_deref(), &self.params);
        let prepared = compiler.document(self);
        if !compiler.errors.is_empty() {
            return Err(compiler.into_error());
        }
        let Some(Prepared {
            size,
            rate,
            scenes,
            audio,
        }) = prepared
        else {
            return Err(compiler.into_error());
        };
        let look = self.look.as_ref().map(|look| match look {
            LookSpec::Clean => Look::clean(),
            LookSpec::Cinematic => Look::cinematic(),
            LookSpec::Film { seed } => Look::film(*seed),
        });
        let output = PixelSize::new(size.width.round() as u32, size.height.round() as u32);
        let mut timeline = Timeline::try_new(size, rate)?;
        if let Some(background) = &self.background {
            timeline = timeline.background(background.color());
        }
        let mut scenes = scenes.into_iter();
        for (index, track) in self.tracks.iter().enumerate() {
            let mut failure = None;
            let built = timeline.try_track(|builder| {
                let mut builder = builder;
                for (item_index, item) in track.items.iter().enumerate() {
                    let path = format!("tracks[{index}].items[{item_index}]");
                    let scene = match item {
                        TrackItem::Scene(_) => scenes.next(),
                        _ => None,
                    };
                    match add_item(builder, item, scene, look.as_ref(), output) {
                        Ok(next) => builder = next,
                        Err(error) => {
                            failure = Some(path);
                            return Err(error);
                        }
                    }
                }
                Ok(builder)
            });
            timeline = built.map_err(|error| located(failure, error))?;
        }
        for bus in audio.buses {
            timeline = timeline.audio_bus(bus);
        }
        if let Some(master) = audio.master {
            timeline = timeline.master(master);
        }
        for (index, clip) in audio.clips.into_iter().enumerate() {
            timeline = timeline
                .try_audio(clip)
                .map_err(|error| located(Some(format!("audio[{index}]")), error))?;
        }
        if let Some(duration) = self.duration {
            let duration = Duration::from_seconds(duration);
            timeline = timeline
                .try_duration(duration)
                .map_err(|error| located(Some("duration".into()), error))?;
        }
        Ok(CompiledDocument {
            timeline: timeline.build()?,
            look,
            resources: std::mem::take(&mut compiler.resources),
            #[cfg(feature = "text")]
            fonts: compiler.fonts.take().map(|fonts| {
                let root = options
                    .base_dir
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("."));
                (fonts, root)
            }),
        })
    }
}

/// Reads, compiles and resolves paths relative to the document's directory.
pub fn load(path: impl AsRef<Path>) -> motion_core::Result<CompiledDocument> {
    load_with(path, None)
}

/// Like [`load`], applying parameter overrides; see [`Document::from_json_with`].
pub fn load_with(
    path: impl AsRef<Path>,
    overrides: Option<&Value>,
) -> motion_core::Result<CompiledDocument> {
    let path = std::path::absolute(path.as_ref())?;
    let text = std::fs::read_to_string(&path)?;
    let document = Document::from_json_with(&text, overrides)?;
    let mut options = CompileOptions::default();
    if let Some(directory) = path.parent() {
        options = options.base_dir(directory);
    }
    let mut compiled = document
        .compile(&options)
        .map_err(|error| crate::diagnostics::annotate_error(error, &text))?;
    compiled.resources.insert(0, path);
    Ok(compiled)
}

/// Validated document-level values and the compiled pieces of one document.
pub(crate) struct Prepared {
    size: Size,
    rate: motion_core::FrameRate,
    scenes: Vec<Built>,
    audio: audio::AudioPlan,
}

/// Diagnostics and shared resources for one compilation.
pub(crate) struct Compiler<'a> {
    base_dir: Option<&'a Path>,
    /// Design size, valid once document values are checked.
    canvas: Size,
    params: &'a BTreeMap<String, Value>,
    errors: Vec<ValidationError>,
    /// Resolved resource files, without duplicates.
    resources: Vec<PathBuf>,
    /// Elements of the scene being compiled, by `id`.
    ids: HashMap<String, &'a Element>,
    /// Serialized forms of referenced elements, by `id`.
    documents: HashMap<String, Value>,
    /// Resolved bindings, by `id.property`.
    bindings: HashMap<String, Arc<dyn Signal<f64>>>,
    /// Bindings being resolved, for cycle detection.
    resolving: Vec<String>,
    #[cfg(feature = "text")]
    fonts: Option<motion_content::FontLibrary>,
}

impl<'a> Compiler<'a> {
    fn new(base_dir: Option<&'a Path>, params: &'a BTreeMap<String, Value>) -> Self {
        Self {
            base_dir,
            canvas: Size::new(1.0, 1.0),
            params,
            errors: Vec::new(),
            resources: Vec::new(),
            ids: HashMap::new(),
            documents: HashMap::new(),
            bindings: HashMap::new(),
            resolving: Vec::new(),
            #[cfg(feature = "text")]
            fonts: None,
        }
    }

    pub(crate) fn error(&mut self, path: &str, field: &str, message: impl Into<String>) {
        self.push(ValidationError::invalid(path, field, message));
    }

    pub(crate) fn limit(&mut self, path: &str, field: &str, message: impl Into<String>) {
        self.push(ValidationError::limit(path, field, message));
    }

    #[cfg_attr(
        all(
            feature = "svg",
            feature = "particles",
            feature = "diagram",
            feature = "captions",
            feature = "formula",
            feature = "audio"
        ),
        expect(dead_code)
    )]
    pub(crate) fn unsupported(&mut self, path: &str, message: impl Into<String>) {
        self.push(ValidationError::unsupported(path, "feature", message));
    }

    /// Converts nonnegative seconds within [`MAX_SECONDS`] to ticks.
    pub(crate) fn seconds(&mut self, value: f64, path: &str, field: &str) -> Option<Duration> {
        self.bounded(value, 0.0, MAX_SECONDS, path, field)
            .map(Duration::from_seconds)
    }

    /// Resolves a resource path, rejecting empty paths and parent traversal.
    pub(crate) fn resolve(&mut self, source: &str, path: &str) -> Option<PathBuf> {
        let resolved = self.resolve_quiet(source);
        if let Some(file) = &resolved
            && !self.resources.contains(file)
        {
            self.resources.push(file.clone());
        }
        if resolved.is_none() {
            self.error(
                path,
                "source",
                "Resource paths must be nonempty and must not contain `..`",
            );
        }
        resolved
    }

    fn resolve_quiet(&self, source: &str) -> Option<PathBuf> {
        let relative = Path::new(source);
        if source.is_empty()
            || relative
                .components()
                .any(|component| component == Component::ParentDir)
        {
            return None;
        }
        Some(match self.base_dir {
            Some(base) if relative.is_relative() => base.join(relative),
            _ => relative.to_path_buf(),
        })
    }

    fn push(&mut self, error: ValidationError) {
        if self.errors.len() < MAX_ERRORS {
            self.errors.push(error);
        }
    }

    fn into_error(self) -> motion_core::Error {
        let mut errors = self.errors;
        if errors.len() == 1 {
            return errors.remove(0).into();
        }
        if errors.is_empty() {
            errors.push(ValidationError::invalid(
                "document",
                "document",
                "Invalid document",
            ));
        }
        motion_core::Error::BuildErrors(
            errors
                .into_iter()
                .map(|error| std::sync::Arc::new(error.into()))
                .collect(),
        )
    }

    /// Validates document-level values and compiles every scene in track order.
    fn document(&mut self, document: &'a Document) -> Option<Prepared> {
        let width = self.bounded(document.size[0], 1.0, MAX_DESIGN_EXTENT, "size", "width");
        let height = self.bounded(document.size[1], 1.0, MAX_DESIGN_EXTENT, "size", "height");
        let rate = document.fps.frame_rate();
        if rate.is_none() {
            self.error("fps", "fps", "Frame-rate components must be nonzero");
        }
        if let Some(duration) = document.duration {
            self.positive_seconds(duration, "duration", "duration");
        }
        if let (Some(width), Some(height)) = (width, height) {
            self.canvas = Size::new(width, height);
        }
        self.fonts(document);
        let audio = self.audio_plan(document);
        let mut scenes = Vec::new();
        for (index, track) in document.tracks.iter().enumerate() {
            for (item_index, item) in track.items.iter().enumerate() {
                let path = format!("tracks[{index}].items[{item_index}]");
                match item {
                    TrackItem::Scene(scene) => {
                        if let Some(built) = self.scene(scene, &format!("{path}.scene")) {
                            scenes.push(built);
                        }
                    }
                    TrackItem::Transition(transition) => {
                        self.positive_seconds(
                            transition.duration,
                            &format!("{path}.transition"),
                            "duration",
                        );
                        if let TransitionEffect::Wipe { angle, feather } = transition.effect {
                            let path = format!("{path}.transition.effect.wipe");
                            self.bounded(angle, -1e6, 1e6, &path, "angle");
                            self.bounded(feather, 0.0, MAX_DESIGN_EXTENT, &path, "feather");
                        }
                    }
                    TrackItem::Gap(seconds) => {
                        self.seconds(*seconds, &path, "gap");
                    }
                }
            }
        }
        Some(Prepared {
            size: Size::new(width?, height?),
            rate: rate?,
            scenes,
            audio,
        })
    }

    fn scene(&mut self, scene: &'a SceneSpec, path: &str) -> Option<Built> {
        self.ids.clear();
        self.documents.clear();
        self.bindings.clear();
        self.resolving.clear();
        self.index_ids(&scene.content, &format!("{path}.content"));
        self.positive_seconds(scene.duration, path, "duration");
        if let Some(start) = scene.start {
            self.seconds(start, path, "start");
        }
        let built = self.element(&scene.content, &format!("{path}.content"), 0)?;
        match &scene.name {
            Some(name) => {
                let name = name.clone();
                Some(Built::with_context(built.varying(), move |context| {
                    built.at(context).named(&name)
                }))
            }
            None => Some(built),
        }
    }

    fn positive_seconds(&mut self, value: f64, path: &str, field: &str) {
        if self.seconds(value, path, field).is_some() && value <= 0.0 {
            self.error(path, field, "Duration must be positive");
        }
    }

    #[cfg(feature = "text")]
    fn fonts(&mut self, document: &Document) {
        if document.fonts.is_empty() {
            return;
        }
        let mut library = motion_content::FontLibrary::new();
        for (index, font) in document.fonts.iter().enumerate() {
            let path = format!("fonts[{index}]");
            if let Some(file) = self.resolve(font, &path)
                && let Err(error) = library.load_file(&file)
            {
                self.error(&path, "source", error.to_string());
            }
        }
        self.fonts = Some(library);
    }

    #[cfg(not(feature = "text"))]
    fn fonts(&mut self, document: &Document) {
        if !document.fonts.is_empty() {
            self.unsupported("fonts", "Fonts require feature `text`");
        }
    }
}

/// Appends one validated track item; `scene` carries the compiled scene content.
fn add_item(
    builder: TrackBuilder,
    item: &TrackItem,
    scene: Option<Built>,
    look: Option<&Look>,
    output: PixelSize,
) -> motion_core::Result<TrackBuilder> {
    match item {
        TrackItem::Scene(spec) => {
            let content = scene.unwrap_or(Built::Fixed(Node::empty()));
            let look = look.cloned();
            let render = motion_core::scene(move |context: &SceneContext<'_>| match &look {
                Some(look) => look.scene(context, output, |context| content.at(context)),
                None => Ok(content.at(context)),
            })
            .with_duration(Duration::from_seconds(spec.duration));
            match spec.start {
                Some(start) => builder.try_place(motion_core::Time::from_seconds(start), render),
                None => builder.try_push(render),
            }
        }
        TrackItem::Transition(transition) => {
            let duration = Duration::from_seconds(transition.duration);
            match transition.effect {
                TransitionEffect::CrossFade => builder.try_transition(CrossFade, duration),
                TransitionEffect::Slide { direction } => {
                    builder.try_transition(Slide { direction }, duration)
                }
                TransitionEffect::Push { direction } => {
                    builder.try_transition(Push { direction }, duration)
                }
                TransitionEffect::Wipe { angle, feather } => builder.try_transition(
                    Wipe {
                        angle: angle.to_radians(),
                        feather,
                    },
                    duration,
                ),
                TransitionEffect::Iris { shape } => {
                    builder.try_transition(Iris { shape }, duration)
                }
                TransitionEffect::ZoomBlur => builder.try_transition(ZoomBlur, duration),
            }
        }
        TrackItem::Gap(seconds) => builder.try_gap(Duration::from_seconds(*seconds)),
    }
}

/// Prefixes a timeline construction failure with its document path.
fn located(path: Option<String>, error: motion_core::Error) -> motion_core::Error {
    match path {
        Some(path) => ValidationError::invalid(path, "timeline", error.to_string()).into(),
        None => error,
    }
}
