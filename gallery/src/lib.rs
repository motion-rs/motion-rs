//! Short motion films built from the public Motion authoring and rendering APIs.
mod demos;

pub use demos::particle_recipes::comparison_frames;

use motion::prelude::*;
use motion::render::RasterLimits;
use motion::{ProjectCommand, StudioArgs};

/// Maximum logical bytes per Gallery frame, including Full HD transition intermediates.
const GALLERY_FRAME_BYTES: u64 = 1024 * 1024 * 1024;

/// Resource-free metadata for one demonstration.
#[derive(Clone, Copy, Debug)]
pub struct DemoMetadata {
    /// Stable Project/CLI identifier.
    pub id: &'static str,
    /// Artwork title.
    pub title: &'static str,
    /// Feature grouping.
    pub category: &'static str,
    /// Composable capabilities used by the film, in its visual narrative order.
    pub features: &'static [&'static str],
}

impl DemoMetadata {
    pub(crate) const fn new(
        id: &'static str,
        title: &'static str,
        category: &'static str,
        features: &'static [&'static str],
    ) -> Self {
        Self {
            id,
            title,
            category,
            features,
        }
    }
}

/// Shared bounded demonstration parameters.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DemoParameters {
    /// Finite seconds, in 1..=60.
    pub seconds: u32,
    /// Reproducible procedural seed.
    pub seed: u64,
    /// Bounded mark count, in 4..=128.
    pub count: u32,
}

impl Default for DemoParameters {
    fn default() -> Self {
        Self {
            seconds: 8,
            seed: 20261027,
            count: 72,
        }
    }
}

/// Lists supported demos without loading fonts or devices.
pub fn catalog() -> Vec<DemoMetadata> {
    demos::catalog()
}

/// Constructs lazy timelines with shared render/resource options.
pub fn project(options: RenderOptions) -> motion::Result<Project> {
    let mut project = Project::new("motion").render_options(options);
    for demo in demos::catalog() {
        project = project.try_timeline_with::<DemoParameters>(demo.id, move |parameters| {
            if !(1..=60).contains(&parameters.seconds) || !(4..=128).contains(&parameters.count) {
                return Err(motion::Error::Parameters(Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "seconds must be 1..=60 and count 4..=128",
                ))));
            }
            demos::timeline(demo, parameters)
        })?;
    }
    Ok(project.build()?)
}

/// Explicit startup preparation; metadata/listing never invokes it.
pub fn render_options(font: Option<&std::path::Path>) -> motion::Result<RenderOptions> {
    let mut fonts = FontLibrary::new();
    if let Some(path) = font {
        fonts.load_file(path)?;
    } else {
        fonts.load_system_fonts()?;
    }
    let assets = Assets::new(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets"))?
        .with_fonts(fonts);
    Ok(RenderOptions::default()
        .size(PixelSize::new(1920, 1080))
        .resources(assets)
        .limits(RasterLimits {
            max_bytes: GALLERY_FRAME_BYTES,
            ..RasterLimits::default()
        })
        .background(Color::from_hex(0xf4f6f5)))
}

/// Gallery command line: the shared project commands plus a font override.
#[derive(clap::Parser)]
#[command(name = "motion-gallery", about = "Motion gallery films")]
struct Cli {
    /// Font file used instead of the system fonts
    #[arg(long, global = true, value_name = "FILE")]
    font: Option<std::path::PathBuf>,
    /// Defaults to studio with feature studio, otherwise list; `--demo` selects a demo
    #[command(subcommand)]
    command: Option<ProjectCommand>,
}

fn parse_cli(arguments: impl IntoIterator<Item = String>) -> Result<Cli, clap::Error> {
    use clap::Parser;
    Cli::try_parse_from(arguments.into_iter().map(|argument| {
        if argument == "--demo" {
            "--timeline".into()
        } else if let Some(name) = argument.strip_prefix("--demo=") {
            format!("--timeline={name}")
        } else {
            argument
        }
    }))
}

/// Fills in the default command and the default Studio demo, rejecting unknown demos.
fn resolve(command: Option<ProjectCommand>) -> motion::Result<ProjectCommand> {
    let command = command.unwrap_or_else(|| {
        if cfg!(feature = "studio") {
            ProjectCommand::Studio(StudioArgs {
                name: None,
                timeline: None,
                parameters: Default::default(),
                render: Default::default(),
            })
        } else {
            ProjectCommand::List
        }
    });
    let ProjectCommand::Studio(mut arguments) = command else {
        return Ok(command);
    };
    match arguments.selection() {
        None => arguments.select(catalog()[0].id),
        Some(name) if !catalog().iter().any(|demo| demo.id == name) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Unknown demo: {name}; use list for available demos"),
            )
            .into());
        }
        Some(_) => {}
    }
    Ok(ProjectCommand::Studio(arguments))
}

/// Executes the shared CLI; Studio opens only on an explicit command or with its feature.
pub fn run() -> motion::Result {
    let cli = parse_cli(std::env::args()).unwrap_or_else(|error| error.exit());
    let command = resolve(cli.command)?;
    let options = if matches!(command, ProjectCommand::List) {
        RenderOptions::default()
    } else {
        render_options(cli.font.as_deref())?
    };
    let project = project(options)?;
    project.print_outcome(project.execute(command)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "ffmpeg")]
    use motion::encode::{NoProgress, PcmBuffer};
    #[cfg(feature = "ffmpeg")]
    use motion::extend::ProgressSink;
    use std::collections::BTreeSet;

    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT; Gallery output size and design-space preservation"]
    fn default_full_hd_output_preserves_demo_layouts() -> motion::Result {
        let font = std::env::var_os("MOTION_TEST_FONT")
            .ok_or_else(|| std::io::Error::other("Set MOTION_TEST_FONT"))?;
        let options = render_options(Some(std::path::Path::new(&font)))?;
        assert_eq!(options.size, Some(PixelSize::new(1920, 1080)));
        assert_eq!(options.limits.max_bytes, GALLERY_FRAME_BYTES);
        options.validate()?;
        let gallery = project(options.clone())?;
        for demo in catalog() {
            let film = gallery.build(demo.id, None)?;
            assert_eq!(film.size(), Size::new(960.0, 540.0), "{}", demo.id);
            assert_eq!(film.frame_count(), 240, "{}", demo.id);
        }
        let overridden = options.size(PixelSize::new(1280, 720));
        assert_eq!(overridden.size, Some(PixelSize::new(1280, 720)));
        overridden.validate()?;
        Ok(())
    }

    #[test]
    fn studio_selects_exactly_one_demo_without_loading_resources() {
        let selected = catalog()[1].id;
        let parse = |values: &[&str]| {
            let cli = parse_cli(
                std::iter::once("motion-gallery")
                    .chain(values.iter().copied())
                    .map(String::from),
            )
            .map_err(|error| std::io::Error::other(error.to_string()))?;
            resolve(cli.command)
        };
        let Ok(ProjectCommand::Studio(chosen)) = parse(&["studio", "--demo", selected]) else {
            panic!("studio command expected");
        };
        assert_eq!(chosen.selection(), Some(selected));
        let Ok(ProjectCommand::Studio(chosen)) = parse(&["studio"]) else {
            panic!("studio command expected");
        };
        assert_eq!(chosen.selection(), Some(catalog()[0].id));
        for invalid in [
            vec!["studio", "--demo"],
            vec!["studio", "--demo", "missing"],
            vec!["studio", selected, "--demo", selected],
            vec!["studio", "--demo", selected, "--demo", selected],
        ] {
            assert!(parse(&invalid).is_err(), "{invalid:?}");
        }
    }

    #[cfg(feature = "ffmpeg")]
    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT and H264/AAC; no output device/window"]
    fn audio_film_exports_short_range_and_preserves_destinations() -> motion::Result {
        use motion::extend::AudioSource;
        let font = std::env::var_os("MOTION_TEST_FONT")
            .ok_or_else(|| std::io::Error::other("Set MOTION_TEST_FONT"))?;
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("mix.mp4");
        let options = render_options(Some(std::path::Path::new(&font)))?;
        let project = project(options.clone())?;
        let timeline = project.build("signal-garden", Some(serde_json::json!({"seconds":1})))?;
        let mut request = ExportOptions::new(&path)
            .render(|_| options.size(PixelSize::new(96, 54)))
            .range(Time::ZERO..100.milliseconds().after_origin());
        let report = export(&timeline, &request, &NoProgress)?;
        assert_eq!(report.frames, 3);
        let media = motion_ffmpeg::inspect::media(&path)?;
        assert!(media.h264 && media.aac && media.bt709_primaries_matrix_limited);
        assert_eq!(media.color_transfer, 13);
        assert_eq!(motion_ffmpeg::inspect::video_frames(&path, 3)?, 3);
        let decoded = PcmBuffer::open(&path)?;
        let mut samples = vec![0.0; 9600];
        assert_eq!(decoded.read(0, &mut samples), 4800);
        assert!(samples.iter().any(|sample| sample.abs() > 0.01));
        let committed = std::fs::read(&path)?;
        assert!(export(&timeline, &request, &NoProgress).is_err());

        struct Cancel(std::sync::atomic::AtomicBool);

        impl ProgressSink for Cancel {
            fn frame_completed(&self, _: u64, _: u64) {
                self.0.store(true, std::sync::atomic::Ordering::Relaxed);
            }

            fn is_cancelled(&self) -> bool {
                self.0.load(std::sync::atomic::Ordering::Relaxed)
            }
        }
        request.overwrite = true;
        let error = export(
            &timeline,
            &request,
            &Cancel(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Cancelled);
        assert_eq!(std::fs::read(&path)?, committed);
        assert_eq!(std::fs::read_dir(directory.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn film_program_has_unique_ids_composed_features_and_exact_duration() -> motion::Result {
        let entries = catalog();
        let expected = [
            "animation-atlas",
            "brush-atlas",
            "particle-atlas",
            "particle-features",
            "filter-atlas",
            "compositing-atlas",
            "vector-atlas",
            "ink-features",
            "noise-atlas",
            "audio-atlas",
            "typography-atlas",
            "caption-atlas",
            "particle-garden",
            "data-story",
            "logo-sting",
            "type-in-motion",
            "ink-journey",
            "signal-garden",
        ]
        .into_iter()
        .chain(cfg!(feature = "formula").then_some("harmonics"))
        .collect::<Vec<_>>();
        assert_eq!(
            entries.iter().map(|entry| entry.id).collect::<Vec<_>>(),
            expected
        );
        let mut ids = BTreeSet::new();
        for entry in &entries {
            assert!(ids.insert(entry.id), "duplicate demo: {}", entry.id);
            assert!(
                entry.features.len() >= 3,
                "film must compose capabilities: {}",
                entry.id
            );
            assert_eq!(
                entry.features.iter().collect::<BTreeSet<_>>().len(),
                entry.features.len()
            );
        }
        for required in [
            "type-in-motion",
            "ink-journey",
            "signal-garden",
            "particle-garden",
            "data-story",
            "logo-sting",
            "animation-atlas",
            "brush-atlas",
            "particle-atlas",
            "particle-features",
            "filter-atlas",
            "compositing-atlas",
            "vector-atlas",
            "noise-atlas",
            "ink-features",
            "audio-atlas",
            "typography-atlas",
            "caption-atlas",
        ] {
            assert!(ids.contains(required), "missing film: {required}");
        }
        for removed in ["chidori", "puppet-show", "quiet-hours", "cinematic-look"] {
            assert!(
                !ids.contains(removed),
                "removed demo must not be listed: {removed}"
            );
        }
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.id == "harmonics")
                .count(),
            usize::from(cfg!(feature = "formula"))
        );
        assert_eq!(entries.len(), 18 + usize::from(cfg!(feature = "formula")));
        let project = project(RenderOptions::default())?;
        for removed in ["chidori", "puppet-show", "quiet-hours", "cinematic-look"] {
            assert!(project.build(removed, None).is_err());
        }
        let registered = project
            .list()
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        assert_eq!(registered, expected);
        for entry in entries {
            for seconds in [1, 8] {
                let timeline =
                    project.build(entry.id, Some(serde_json::json!({"seconds":seconds})))?;
                assert_eq!(timeline.size(), Size::new(960.0, 540.0));
                assert_eq!(timeline.duration(), seconds.seconds());
                assert_eq!(timeline.frame_count(), seconds as u64 * 30);
            }
        }
        Ok(())
    }

    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT; no window, GPU or audio device"]
    fn every_demo_renders_its_subject_and_replays_exactly() -> motion::Result {
        let font = std::env::var_os("MOTION_TEST_FONT")
            .ok_or_else(|| std::io::Error::other("Set MOTION_TEST_FONT"))?;
        let options =
            render_options(Some(std::path::Path::new(&font)))?.size(PixelSize::new(192, 108));
        let project = project(options.clone())?;
        for entry in catalog()
            .into_iter()
            .filter(|entry| entry.category != "Models")
        {
            let timeline =
                project.build(entry.id, Some(serde_json::json!({"seconds":4,"count":16})))?;
            let first = render_still(&timeline, 1.seconds().after_origin(), &options)?;
            let later = render_still(&timeline, 2.seconds().after_origin(), &options)?;
            let replay = render_still(&timeline, 1.seconds().after_origin(), &options)?;
            assert_eq!(first.bytes(), replay.bytes(), "seek replay: {}", entry.id);
            let pixels = first.bytes();
            let background = &pixels[..4];
            let painted = (30..100)
                .flat_map(|row| (10..182).map(move |column| (row * 192 + column) * 4))
                .filter(|offset| &pixels[*offset..*offset + 4] != background)
                .count();
            assert!(painted > 10, "empty demo subject: {}", entry.id);
            let changed = (30..100)
                .flat_map(|row| (10..182).map(move |column| (row * 192 + column) * 4))
                .filter(|offset| {
                    first.bytes()[*offset..*offset + 4] != later.bytes()[*offset..*offset + 4]
                })
                .count();
            assert!(changed > 30, "film subject did not animate: {}", entry.id);
        }
        Ok(())
    }

    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT; full-size sampled admission/replay, not a benchmark"]
    fn full_size_default_budgets_admit_catalog_and_replay() -> motion::Result {
        let font = std::env::var_os("MOTION_TEST_FONT")
            .ok_or_else(|| std::io::Error::other("Set MOTION_TEST_FONT"))?;
        let options = render_options(Some(std::path::Path::new(&font)))?
            .size(PixelSize::new(1920, 1080))
            .backend(BackendPreference::Cpu);
        let project = project(options.clone())?;
        let parameters = DemoParameters::default();
        let times = [
            Time::ZERO,
            (parameters.seconds / 2).seconds().after_origin(),
            (parameters.seconds * 1000 - 1)
                .milliseconds()
                .after_origin(),
        ];
        let mut cases = 0;
        for entry in catalog() {
            let timeline = project.build(entry.id, None)?;
            for quality in [motion::RenderQuality::Final, motion::RenderQuality::Draft] {
                let options = options.clone().quality(quality);
                for time in times {
                    println!(
                        "_gallery_admission demo={} quality={quality:?} ticks={}",
                        entry.id,
                        time.ticks()
                    );
                    let actual = render_still(&timeline, time, &options)?;
                    let replay = render_still(&timeline, time, &options)?;
                    assert_eq!(actual.bytes().len(), 1920 * 1080 * 4);
                    assert_eq!(
                        actual.bytes(),
                        replay.bytes(),
                        "full-size replay: {} {quality:?}",
                        entry.id
                    );
                    cases += 1;
                }
            }
        }
        assert_eq!(cases, catalog().len() * times.len() * 2);
        println!(
            "_gallery_admission cases={cases} size=1920x1080 count={} seed={} seconds={} replay=exact",
            parameters.count, parameters.seed, parameters.seconds
        );
        Ok(())
    }

    #[cfg(feature = "gpu")]
    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT and native GPU; no window or encoder"]
    fn films_match_cpu_gpu_and_replay() -> motion::Result {
        let font = std::env::var_os("MOTION_TEST_FONT")
            .ok_or_else(|| std::io::Error::other("Set MOTION_TEST_FONT"))?;
        let cpu = render_options(Some(std::path::Path::new(&font)))?.size(PixelSize::new(192, 108));
        let gpu = cpu
            .clone()
            .backend(BackendPreference::gpu())
            .gpu_context(motion::render::GpuContext::new()?);
        let project = project(cpu.clone())?;
        let mut cases = 0;
        let mut maximum = 0;
        for entry in catalog() {
            let timeline =
                project.build(entry.id, Some(serde_json::json!({"seconds":4,"count":16})))?;
            for time in [600.milliseconds(), 2.seconds(), 3400.milliseconds()] {
                let expected = render_still(&timeline, time.after_origin(), &cpu)?;
                let actual = render_still(&timeline, time.after_origin(), &gpu)?;
                let replay = render_still(&timeline, time.after_origin(), &gpu)?;
                assert_eq!(actual.bytes(), replay.bytes(), "GPU replay: {}", entry.id);
                let difference = expected
                    .bytes()
                    .iter()
                    .zip(actual.bytes())
                    .map(|(expected, actual)| expected.abs_diff(*actual))
                    .max()
                    .unwrap_or(0);
                maximum = maximum.max(difference);
                assert!(
                    difference <= 2,
                    "CPU/GPU mismatch: {} at {} ticks: {difference}",
                    entry.id,
                    time.ticks()
                );
                cases += 1;
            }
        }
        assert_eq!(cases, catalog().len() * 3);
        println!("Gallery content cases={cases}, max_byte={maximum}, GPU replay=exact");
        Ok(())
    }
}
