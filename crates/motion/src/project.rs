//! Lazy named timelines, bounded typed parameters and the shared CLI.
#[cfg(test)]
use crate::*;

use crate::encode::ExportReport;
use crate::extend::ProgressSink;
use crate::render::FrameBuffer;
use crate::{ExportOptions, RenderOptions, Result, Time, Timeline, TimelineBuilder};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::sync::Arc;

mod args;
mod cli;
mod flags;
mod validate;

pub use args::{
    Backend, ExportArgs, OutputArgs, ParameterArgs, ProjectCommand, RenderArgs, StillArgs,
    StudioArgs, ValidateArgs,
};
pub use cli::ProjectOutcome;
pub use validate::ValidationReport;

const MAX_PARAMETERS: usize = 1024 * 1024;

fn invalid(message: impl Into<String>) -> crate::Error {
    crate::Error::Parameters(Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message.into(),
    )))
}

fn json_error(error: serde_json::Error) -> crate::Error {
    crate::Error::Parameters(Box::new(error))
}

/// Discoverable metadata. Defaults describe serializable parameter values, not a
/// complete JSON Schema for arbitrary custom serde implementations.
#[derive(Clone, Debug, Serialize)]
pub struct ProjectTimelineInfo {
    /// Stable CLI identifier.
    pub name: String,
    /// Typed factory defaults, or None for a non-parameterized timeline.
    pub defaults: Option<Value>,
}

#[derive(Clone)]
struct Entry {
    info: ProjectTimelineInfo,
    build_film: Arc<dyn Fn(Option<Value>) -> Result<Film> + Send + Sync>,
    visual: Option<crate::VisualCheckPolicy>,
}

/// A validated, immutable registry of lazy timeline factories. Listing never
/// invokes a timeline builder or initializes fonts, devices or encoders. Create one
/// with [`Project::new`] and [`ProjectBuilder::build`].
#[derive(Clone)]
pub struct Project {
    name: String,
    entries: Vec<Entry>,
    options: RenderOptions,
    #[cfg(feature = "declarative")]
    cache: Arc<std::sync::Mutex<motion_scene::CompileCache>>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            name: "motion".into(),
            entries: Vec::new(),
            options: RenderOptions::default(),
            #[cfg(feature = "declarative")]
            cache: Default::default(),
        }
    }
}

impl Project {
    /// Starts an empty registry with CPU/transparent still defaults.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(name: impl Into<String>) -> ProjectBuilder {
        ProjectBuilder::new(name)
    }

    /// Project name used by command-line help and diagnostics.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the same defaults available to programmatic render/export callers.
    pub fn options(&self) -> &RenderOptions {
        &self.options
    }

    /// Optional automated keyframe policy registered for a timeline, if any.
    /// Never read by rendering or export; checks are run by the caller.
    pub fn visual_check(&self, name: &str) -> Option<&crate::VisualCheckPolicy> {
        self.entries
            .iter()
            .find(|entry| entry.info.name == name)?
            .visual
            .as_ref()
    }

    /// Lists registration metadata without constructing any timeline.
    pub fn list(&self) -> Vec<ProjectTimelineInfo> {
        self.entries
            .iter()
            .map(|entry| entry.info.clone())
            .collect()
    }

    /// Builds one selected timeline after bounded parameter validation.
    ///
    /// With feature `declarative`, an unregistered name ending in `.json` loads
    /// that document file; see [`ProjectBuilder::document`].
    pub fn build(&self, name: &str, parameters: Option<Value>) -> Result<Timeline> {
        Ok(self.film(name, parameters)?.timeline)
    }

    /// Builds a film and applies its output-side look stage, if it declares one.
    pub(crate) fn film_options(
        &self,
        name: &str,
        parameters: Option<Value>,
        options: RenderOptions,
    ) -> Result<(Timeline, RenderOptions)> {
        let film = self.film(name, parameters)?;
        let options = film.options(options);
        Ok((film.timeline, options))
    }

    /// Calls the common alpha-preserving programmatic still implementation.
    pub fn render_still(
        &self,
        name: &str,
        parameters: Option<Value>,
        time: Time,
        options: &RenderOptions,
    ) -> Result<FrameBuffer> {
        let (timeline, options) = self.film_options(name, parameters, options.clone())?;
        crate::render_still(&timeline, time, &options)
    }

    /// Calls the common programmatic export/transaction implementation.
    pub fn export(
        &self,
        name: &str,
        parameters: Option<Value>,
        options: &ExportOptions,
        progress: &dyn ProgressSink,
    ) -> Result<ExportReport> {
        let film = self.film(name, parameters)?;
        let mut options = options.clone();
        options.render = film.options(options.render);
        crate::export(&film.timeline, &options, progress)
    }

    /// Builds only the selected registration with the shared render configuration.
    ///
    /// A declarative document, registered or named by its `.json` path, is rebuilt while
    /// the workbench runs whenever the document or a file it refers to changes.
    #[cfg(feature = "studio")]
    pub fn studio_scene(&self, name: &str) -> Result<crate::studio::StudioScene> {
        self.studio_scene_with(name, None, self.options.clone())
    }

    /// Builds a workbench scene with explicit parameters and render options.
    #[cfg(feature = "studio")]
    pub(crate) fn studio_scene_with(
        &self,
        name: &str,
        parameters: Option<Value>,
        options: RenderOptions,
    ) -> Result<crate::studio::StudioScene> {
        let (scene, watch) = self.studio_build(name, parameters.clone(), options.clone())?;
        if watch.is_empty() {
            return Ok(scene);
        }
        let project = self.clone();
        let name = name.to_owned();
        Ok(scene.reloadable(watch, move || {
            project.studio_scene_with(&name, parameters.clone(), options.clone())
        }))
    }

    #[cfg(feature = "studio")]
    fn studio_build(
        &self,
        name: &str,
        parameters: Option<Value>,
        options: RenderOptions,
    ) -> Result<(crate::studio::StudioScene, Vec<std::path::PathBuf>)> {
        let film = self.film(name, parameters)?;
        let options = film.options(options);
        let scene = crate::studio::StudioScene::new(
            name,
            crate::render::Composition::for_timeline(&film.timeline, &options)?,
            film.timeline,
        )?;
        Ok((scene, film.watch))
    }

    fn film(&self, name: &str, parameters: Option<Value>) -> Result<Film> {
        if let Some(parameters) = &parameters
            && serde_json::to_vec(parameters).map_err(json_error)?.len() > MAX_PARAMETERS
        {
            return Err(invalid("Parameters exceed 1 MiB"));
        }
        let film = match self.entries.iter().find(|entry| entry.info.name == name) {
            Some(entry) => (entry.build_film)(parameters)?,
            #[cfg(feature = "declarative")]
            None if name.ends_with(".json") => {
                Film::document(std::path::Path::new(name), parameters.as_ref(), &self.cache)?
            }
            None => return Err(invalid(format!("Unknown timeline: {name}"))),
        };
        Ok(film)
    }
}

/// Collects timeline registrations and render defaults; [`build`](Self::build) validates
/// them and returns the [`Project`].
#[derive(Clone)]
pub struct ProjectBuilder {
    name: String,
    entries: Vec<Entry>,
    options: RenderOptions,
    errors: motion_core::ErrorReport,
    #[cfg(feature = "declarative")]
    cache: Arc<std::sync::Mutex<motion_scene::CompileCache>>,
}

impl Default for ProjectBuilder {
    fn default() -> Self {
        Self::new("motion")
    }
}

impl ProjectBuilder {
    /// Starts an empty registry with CPU/transparent still defaults.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            entries: Vec::new(),
            options: RenderOptions::default(),
            errors: Default::default(),
            #[cfg(feature = "declarative")]
            cache: Default::default(),
        }
    }

    /// Checks every registration made so far, reporting all deferred failures at once
    /// without invoking factories, and freezes the registry.
    pub fn build(self) -> std::result::Result<Project, motion_core::ErrorReport> {
        let mut errors = self.errors;
        if self.name.trim().is_empty() || self.name.len() > 256 {
            errors.record(invalid("Project name requires 1..=256 bytes"));
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(Project {
            name: self.name,
            entries: self.entries,
            options: self.options,
            #[cfg(feature = "declarative")]
            cache: self.cache,
        })
    }

    /// Builds the project and runs its command line, printing errors with Display and
    /// returning a process exit code.
    pub fn run(self) -> std::process::ExitCode {
        match self.build() {
            Ok(project) => project.run(),
            Err(error) => {
                eprintln!("{error}");
                std::process::ExitCode::FAILURE
            }
        }
    }

    /// Registers a lazy factory returning a timeline builder, deferring duplicate/name
    /// errors. Builder failures are reported when the timeline is built.
    pub fn timeline(
        self,
        name: impl Into<String>,
        build: impl Fn() -> TimelineBuilder + Send + Sync + 'static,
    ) -> Self {
        self.defer(|project| project.try_timeline(name, move || Ok(build().build()?)))
    }

    /// Registers a factory borrowing bounded, deserialized typed parameters.
    pub fn timeline_with<P: Default + Serialize + DeserializeOwned + 'static>(
        self,
        name: impl Into<String>,
        build: impl Fn(&P) -> TimelineBuilder + Send + Sync + 'static,
    ) -> Self {
        self.defer(|project| {
            project.try_timeline_with::<P>(name, move |parameters| Ok(build(&parameters).build()?))
        })
    }

    /// Sets rendering/resources shared by this project's CLI commands.
    pub fn render_options(mut self, options: RenderOptions) -> Self {
        self.options = options;
        self
    }

    /// Registers a fallible, non-parameterized timeline builder.
    pub fn try_timeline(
        self,
        name: impl Into<String>,
        build: impl Fn() -> Result<Timeline> + Send + Sync + 'static,
    ) -> Result<Self> {
        self.register(
            ProjectTimelineInfo {
                name: name.into(),
                defaults: None,
            },
            move |parameters| {
                if parameters.is_some() {
                    return Err(invalid("Timeline does not accept parameters"));
                }
                build().map(Film::plain)
            },
        )
    }

    /// Registers typed serde object parameters. Top-level overrides merge over
    /// Default's serialized fields; unknown fields fail before calling the builder.
    /// Defaults/custom deserializers must be deterministic and free of side effects.
    pub fn try_timeline_with<P: Default + Serialize + DeserializeOwned + 'static>(
        self,
        name: impl Into<String>,
        build: impl Fn(P) -> Result<Timeline> + Send + Sync + 'static,
    ) -> Result<Self> {
        let defaults = serde_json::to_value(P::default()).map_err(json_error)?;
        if !defaults.is_object()
            || serde_json::to_vec(&defaults).map_err(json_error)?.len() > MAX_PARAMETERS
        {
            return Err(invalid(
                "Parameters require a serializable object no larger than 1 MiB",
            ));
        }
        let saved = defaults.clone();
        self.register(
            ProjectTimelineInfo {
                name: name.into(),
                defaults: Some(defaults),
            },
            move |parameters| {
                let mut merged = saved.clone();
                if let Some(parameters) = parameters {
                    let values = parameters
                        .as_object()
                        .ok_or_else(|| invalid("Parameters must be a JSON object"))?;
                    let target = merged.as_object_mut().expect("validated defaults");
                    for (key, value) in values {
                        if !target.contains_key(key) {
                            return Err(invalid(format!("Unknown parameter: {key}")));
                        }
                        target.insert(key.clone(), value.clone());
                    }
                }
                build(serde_json::from_value(merged).map_err(json_error)?).map(Film::plain)
            },
        )
    }

    /// Registers a declarative JSON document, read lazily on each build.
    /// Unchanged documents reuse this project's bounded compilation cache.
    /// Relative resources resolve against the document's directory; its look's
    /// display stage applies to this project's stills and exports. `--params`
    /// overrides the parameters the document declares; `list` cannot show them
    /// because listing never reads files.
    #[cfg(feature = "declarative")]
    pub fn document(self, name: impl Into<String>, path: impl Into<std::path::PathBuf>) -> Self {
        self.defer(|project| project.try_document(name, path))
    }

    /// Registers a declarative JSON document, failing immediately for invalid names.
    #[cfg(feature = "declarative")]
    pub fn try_document(
        self,
        name: impl Into<String>,
        path: impl Into<std::path::PathBuf>,
    ) -> Result<Self> {
        let path = path.into();
        let cache = self.cache.clone();
        self.register(
            ProjectTimelineInfo {
                name: name.into(),
                defaults: None,
            },
            move |parameters| Film::document(&path, parameters.as_ref(), &cache),
        )
    }

    /// Attaches optional visual-check metadata to an already registered timeline, deferring
    /// unknown names and structurally invalid policies. Does not build the timeline; use
    /// [`VisualCheckPolicy::preflight`](crate::VisualCheckPolicy::preflight) against its composition.
    pub fn visual_check(self, name: &str, policy: crate::VisualCheckPolicy) -> Self {
        self.defer(|mut project| {
            policy.check_structure()?;
            let entry = project
                .entries
                .iter_mut()
                .find(|entry| entry.info.name == name)
                .ok_or_else(|| invalid(format!("Unknown timeline: {name}")))?;
            entry.visual = Some(policy);
            Ok(project)
        })
    }

    fn register(
        mut self,
        info: ProjectTimelineInfo,
        build: impl Fn(Option<Value>) -> Result<Film> + Send + Sync + 'static,
    ) -> Result<Self> {
        if self.entries.len() >= 256
            || info.name.is_empty()
            || info.name.len() > 128
            || !info
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            || self
                .entries
                .iter()
                .any(|entry| entry.info.name == info.name)
        {
            return Err(invalid(
                "Require a unique 1..128-byte timeline identifier; registry limit is 256",
            ));
        }
        self.entries.push(Entry {
            info,
            build_film: Arc::new(build),
            visual: None,
        });
        Ok(self)
    }

    fn defer(self, operation: impl FnOnce(Self) -> Result<Self>) -> Self {
        let mut previous = self.clone();
        match operation(self) {
            Ok(value) => value,
            Err(error) => {
                previous.errors.record(error);
                previous
            }
        }
    }
}

/// A built timeline with the render-option adjustments it declares and the files it
/// was built from.
struct Film {
    timeline: Timeline,
    #[cfg(feature = "declarative")]
    document: Option<Arc<motion_scene::CompiledDocument>>,
    watch: Vec<std::path::PathBuf>,
}

impl Film {
    fn plain(timeline: Timeline) -> Self {
        Self {
            timeline,
            #[cfg(feature = "declarative")]
            document: None,
            watch: Vec::new(),
        }
    }

    #[cfg(feature = "declarative")]
    fn document(
        path: &std::path::Path,
        parameters: Option<&Value>,
        cache: &std::sync::Mutex<motion_scene::CompileCache>,
    ) -> Result<Self> {
        let document = cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .load_with(path, parameters)?;
        Ok(Self {
            timeline: document.timeline().clone(),
            watch: document.resources().to_vec(),
            document: Some(document),
        })
    }

    /// Applies the document's look and fonts to the render options.
    fn options(&self, options: RenderOptions) -> RenderOptions {
        #[cfg(feature = "declarative")]
        if let Some(document) = &self.document {
            return document.render_options(options);
        }
        options
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cli_help_frame_indices_and_transparent_sequence_options_are_real() -> Result<()> {
        use super::*;
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = calls.clone();
        let project = Project::new("cli-fixture")
            .timeline("main", move || {
                observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Timeline::new(Size::new(3.0, 3.0), crate::fps!(4)).track(|track| {
                    track.push(crate::scene(|_| Node::empty()).with_duration(1.seconds()))
                })
            })
            .build()?;
        assert!(
            matches!(project.cli(["--help".into()])?, ProjectOutcome::Help(help) if help.contains("cli-fixture") && help.contains("validate") && !help.contains("<app>"))
        );
        assert!(
            matches!(project.cli(["export".into(), "--help".into()])?, ProjectOutcome::Help(help) if help.contains("--codec"))
        );
        assert!(matches!(project.cli([])?, ProjectOutcome::Help(_)));
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(
            project.options().backend,
            crate::BackendPreference::default()
        );
        assert_eq!(parse_time("500ms")?, 500.milliseconds().after_origin());
        assert_eq!(parse_time("2s")?, 2.seconds().after_origin());
        assert!(parse_codec("h264", None, true, crate::fps!(30)).is_err());
        assert!(parse_codec("vp9", Some(64), true, crate::fps!(30)).is_err());
        let directory = tempfile::tempdir()?;
        let destination = directory.path().join("frames");
        let arguments = [
            "export",
            "main",
            "--codec",
            "png",
            "--alpha",
            "--range",
            "0s..500ms",
            "--backend",
            "cpu",
            "--output",
            destination.to_str().unwrap(),
        ]
        .into_iter()
        .map(str::to_owned);
        assert!(
            matches!(project.cli(arguments)?, ProjectOutcome::Exported(report) if report.frames == 2)
        );
        assert_eq!(std::fs::read_dir(destination)?.count(), 2);
        let still = directory.path().join("frame.png");
        project.cli(
            [
                "still",
                "main",
                "--frame",
                "2",
                "--backend",
                "cpu",
                "--output",
                still.to_str().unwrap(),
            ]
            .into_iter()
            .map(str::to_owned),
        )?;
        assert!(still.is_file());
        assert!(
            project
                .cli(
                    ["still", "main", "--frame", "2", "--time", "0.5"]
                        .into_iter()
                        .map(str::to_owned)
                )
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn visual_checks_are_optional_registration_metadata() {
        use super::*;
        let policy = || {
            crate::VisualCheckPolicy::new(
                vec![0, 3],
                vec![crate::VisualRegion::new(
                    "art",
                    crate::Rect::new(0.0, 0.0, 4.0, 4.0),
                )],
            )
        };
        let timeline = || Timeline::new(Size::new(8.0, 4.0), crate::fps!(30));
        let project = Project::new("checks")
            .timeline("main", timeline)
            .timeline("plain", timeline)
            .visual_check("main", policy())
            .build()
            .unwrap();
        assert_eq!(project.visual_check("main").unwrap().frames, vec![0, 3]);
        assert!(project.visual_check("plain").is_none());
        let error = Project::new("checks")
            .timeline("main", timeline)
            .visual_check("missing", policy())
            .visual_check("main", crate::VisualCheckPolicy::new(vec![0], vec![]))
            .build()
            .err()
            .unwrap();
        assert_eq!(error.len(), 2);
    }

    #[test]
    fn fluent_registration_collects_errors_without_invoking_factories() {
        use super::*;
        let error = Project::new("deferred")
            .timeline("main", || {
                Timeline::new(Size::new(8.0, 4.0), crate::fps!(30))
            })
            .timeline("main", || panic!("duplicate factory must stay lazy"))
            .timeline("", || panic!("invalid factory must stay lazy"))
            .build()
            .err()
            .unwrap();
        assert_eq!(error.len(), 2);
        let project = Project::new("valid")
            .timeline("main", || {
                Timeline::new(Size::new(8.0, 4.0), crate::fps!(30)).track(|track| {
                    track.push(crate::scene(|_| Node::empty()).with_duration(1.seconds()))
                })
            })
            .build()
            .unwrap();
        assert_eq!(project.build("main", None).unwrap().frame_count(), 30);
    }

    use super::flags::{parse_codec, parse_time};
    use super::*;
    use crate::{Size, TimeLiteral};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(serde::Serialize, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Parameters {
        seconds: u32,
    }

    impl Default for Parameters {
        fn default() -> Self {
            Self { seconds: 1 }
        }
    }

    #[test]
    fn registry_listing_parameters_and_cli_share_config_without_loading_resources() -> Result<()> {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let project = Project::new("motion")
            .try_timeline_with::<Parameters>("geometry", move |parameters| {
                count.fetch_add(1, Ordering::SeqCst);
                if parameters.seconds == 0 || parameters.seconds > 4 {
                    return Err(invalid("seconds must be 1..=4"));
                }
                Timeline::try_new(Size::new(4.0, 2.0), crate::fps!(10))?
                    .try_track(|track| {
                        track.try_push(
                            (|_: &crate::SceneContext<'_>| crate::Node::empty())
                                .with_duration(i64::from(parameters.seconds).seconds()),
                        )
                    })?
                    .build()
                    .map_err(crate::Error::from)
            })?
            .build()?;
        assert_eq!(
            project.list()[0].defaults,
            Some(serde_json::json!({"seconds": 1}))
        );
        assert!(matches!(
            project.cli(["list".into()])?,
            ProjectOutcome::Listed(_)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(
            project
                .build("geometry", Some(serde_json::json!({"unknown": 1})))
                .is_err()
        );
        assert!(
            project
                .build("geometry", Some(serde_json::json!({"seconds": "bad"})))
                .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("still.png");
        let parameters = directory.path().join("parameters.json");
        std::fs::write(&parameters, r#"{"seconds":2}"#)?;
        let outcome = project.cli([
            "still".into(),
            "geometry".into(),
            "--params-file".into(),
            parameters.display().to_string(),
            "--time".into(),
            "0.25".into(),
            "--size".into(),
            "8x4".into(),
            "--output".into(),
            path.display().to_string(),
        ])?;
        assert!(matches!(outcome, ProjectOutcome::Still(_)));
        let image = ::image::open(&path)
            .map_err(|error| crate::Error::Parameters(Box::new(error)))?
            .into_rgba8();
        assert_eq!(image.dimensions(), (8, 4));
        assert!(image.pixels().all(|pixel| pixel.0 == [0, 0, 0, 0]));
        assert_eq!(
            project
                .build("geometry", Some(serde_json::json!({})))?
                .duration(),
            1.seconds()
        );
        assert!(
            project
                .cli([
                    "still".into(),
                    "geometry".into(),
                    "--time".into(),
                    "NaN".into()
                ])
                .is_err()
        );
        Ok(())
    }
}
