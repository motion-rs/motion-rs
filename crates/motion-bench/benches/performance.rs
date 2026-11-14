//! Renderer performance cases, CPU stage timings, microbenchmarks and video export runs.
//!
//! `CASES` are the core 640x360 cohorts and `BREADTH_CASES` widen coverage to gradients,
//! blending, clipping, strokes, layers, node counts, filters, text and particle simulations.
//! Every case is a deterministic scene rendered through the same frame renderer.
//!
//! Environment (all optional):
//! - `MOTION_BENCH_CASES`: comma-separated cases, default all.
//! - `MOTION_BENCH_BACKENDS`: `cpu`, `gpu` or both (gpu needs the `gpu` feature).
//! - `MOTION_BENCH_SIZE`: pixel size of the render cases, default 640x360.
//! - `MOTION_BENCH_SCALING`: `1` measures `SCALING_CASES` at `MOTION_BENCH_SIZES`, default off.
//! - `MOTION_BENCH_SIZES`: comma-separated `WxH` list, default `1280x720,1920x1080`.
//! - `MOTION_BENCH_STAGES`: `1` records CPU stage timings for every selected case, default off.
//! - `MOTION_BENCH_SEQUENCES`: `1` (default) records sequential and random-seek playback.
//! - `MOTION_BENCH_COLD`: `1` records cold-session frames, default off.
//! - `MOTION_BENCH_MICRO`: `1` (default) records primitive and conversion microbenchmarks.
//! - `MOTION_BENCH_VIDEO`: `1` records video export runs (feature `ffmpeg`), default off.
//! - `MOTION_BENCH_FONT`: font file, default the repository Noto Sans fixture.
//!
//! `cargo test -p motion-bench --test performance_cases` renders every breadth case once at
//! 160x90 without timing, so scene regressions surface without a full benchmark run.

use artwork::artwork_with_particles;
use harness::{
    case_composition, composition, flag, limits, load_resources, parse_count, parse_sizes, ppm,
    replay_frames,
};
use micro::microbenchmarks;
use motion::prelude::*;
use motion::render::BackendKind;
use primitives::primitive_microbenchmarks;
use recording::{record_cold, record_frame, record_memory_frame, record_sequences};
use stages::cpu_stages;
use video::video_benchmark;

#[path = "performance/artwork.rs"]
mod artwork;
#[path = "performance/harness.rs"]
mod harness;
#[path = "performance/micro.rs"]
mod micro;
#[path = "performance/primitives.rs"]
mod primitives;
#[path = "performance/recording.rs"]
mod recording;
#[path = "performance/replay.rs"]
mod replay;
#[path = "performance/scenes.rs"]
mod scenes;
#[path = "performance/stages.rs"]
mod stages;
#[path = "support.rs"]
mod support;
#[path = "performance/video.rs"]
mod video;
#[cfg(feature = "gpu")]
#[expect(dead_code, reason = "only the Replay visual tier is used here")]
#[path = "../../motion-render/src/gpu/visual.rs"]
mod visual;

/// Core cases measured as warm frames, sequential playback and random seeks.
const CASES: &[&str] = &[
    "empty",
    "shapes",
    "text",
    "fill",
    "stroke",
    "ink-fill",
    "ink-stroke",
    "isolation",
    "blur",
    "shadow",
    "image",
    "image-alpha",
    "mask-alpha",
    "mask-luminance",
    "mask-shapes",
    "cross-fade",
    "wipe",
    "push",
    "bloom",
    "grade",
    "motion-blur",
    "particles-orbit",
    "particles-plexus",
    "ink-long-stroke",
    "text-parts",
    "svg",
    "path-sampling",
    "path-trim",
    "morph-curves",
];

/// Cases that widen coverage across content, scale and pipeline stages.
const BREADTH_CASES: &[&str] = &[
    "gradient-linear",
    "gradient-radial",
    "gradient-sweep",
    "blend-modes",
    "clip-path",
    "clip-nested",
    "stroke-dash",
    "stroke-wide",
    "hairlines",
    "path-complex",
    "layers-nested",
    "layers-wide",
    "nodes-1k",
    "nodes-10k",
    "nodes-50k",
    "layout-grid",
    "blur-large",
    "filter-chromatic",
    "filter-chain",
    "filter-bloom",
    "filter-lens",
    "filter-lut",
    "display-transform",
    "text-paragraph",
    "text-labels",
    "particles-ballistic",
    "particles-simulation",
    "particles-sub-emitters",
    "particles-collision",
    "particles-parallel",
    "filter-siblings",
];

/// Cases also measured through the in-memory RGBA8 output path. `filter-siblings` has three
/// independent blurred siblings, which exercises the output band halo.
const MEMORY_CASES: &[&str] = &["filter-siblings", "display-transform"];

/// Cases repeated at every size in `MOTION_BENCH_SIZES` to expose per-pixel and per-node scaling.
const SCALING_CASES: &[&str] = &["nodes-10k", "gradient-linear", "blend-modes", "blur-large"];

const DESIGN: Size = Size::new(640.0, 360.0);

const MAX_NODES: usize = 100_016;

fn invalid(message: impl Into<String>) -> Error {
    support::invalid(message)
}

fn scene(case: &str, context: &SceneContext<'_>, particle_count: usize) -> Node {
    if BREADTH_CASES.contains(&case) {
        scenes::artwork(case, context)
    } else {
        artwork_with_particles(case, context, particle_count)
    }
}

/// Engine admission limits are findings, not failures: a rejected case is listed in
/// `rejected.txt` and the remaining cases still run.
fn skip_rejected(name: &str, rejected: &mut Vec<String>, result: motion::Result) -> motion::Result {
    match result {
        Err(Error::Validation(error))
            if error.kind == motion::render::ValidationErrorKind::StructuralLimit =>
        {
            println!("{name}: rejected by the engine ({error})");
            rejected.push(format!("{name}\t{}\t{}", error.path, error.field));
            Ok(())
        }
        other => other,
    }
}

fn main() -> motion::Result {
    use serde_json::json;
    use support::{Report, Settings};
    let size = support::size((640, 360))?;
    let settings = Settings::from_env()?;
    let observe = std::env::var("MOTION_BENCH_TIMINGS").unwrap_or_default() == "1";
    if observe && settings.iterations != 1 {
        return Err(invalid(
            "Frame timing diagnostics require one iteration per sample",
        ));
    }
    let all: Vec<&str> = CASES.iter().chain(BREADTH_CASES).copied().collect();
    let cases = support::selected("MOTION_BENCH_CASES", &all)?;
    let renderers = support::renderers(cfg!(feature = "gpu"))?;
    let cold = flag("MOTION_BENCH_COLD", false);
    let scaling = flag("MOTION_BENCH_SCALING", false);
    let sizes = parse_sizes(
        &std::env::var("MOTION_BENCH_SIZES").unwrap_or_else(|_| "1280x720,1920x1080".into()),
    )?;
    let stages = flag("MOTION_BENCH_STAGES", false);
    let sequences = flag("MOTION_BENCH_SEQUENCES", true);
    let sequence_frames = if sequences {
        support::sequence_length(90)?
    } else {
        0
    };
    let micro = flag("MOTION_BENCH_MICRO", true);
    let video = flag("MOTION_BENCH_VIDEO", false);
    let particle_count = parse_count(
        &std::env::var("MOTION_BENCH_PARTICLES").unwrap_or_else(|_| "72".into()),
        10_000,
    )?;
    let video_frames = parse_count(
        &std::env::var("MOTION_BENCH_VIDEO_FRAMES").unwrap_or_else(|_| "8".into()),
        900,
    )? as u32;
    if video && !cfg!(feature = "ffmpeg") {
        return Err(invalid("Video measurements require feature ffmpeg"));
    }
    let (resources, font) = load_resources()?;
    let mut workload = Vec::new();
    for source in [
        &include_bytes!("performance.rs")[..],
        include_bytes!("performance/artwork.rs"),
        include_bytes!("performance/scenes.rs"),
    ] {
        workload.extend_from_slice(source);
    }
    let mut report = Report::new(
        "performance",
        settings.clone(),
        json!({
            "size": [size.width, size.height], "design": [640, 360], "cases": cases,
            "scaling": scaling, "sizes": sizes.iter().map(|size| [size.width, size.height]).collect::<Vec<_>>(),
            "backends": renderers.iter().map(|value| format!("{value:?}")).collect::<Vec<_>>(),
            "frames": harness::REFERENCE_FRAMES, "frame_rate": [30, 1], "cold": cold,
            "stages": stages, "micro": micro, "video": video, "sequence_frames": sequence_frames,
            "limits": {"bytes": limits().max_bytes, "work": limits().max_work}, "font": font,
            "quality": "Final", "particle_count": particle_count,
            "video_frames": if video { video_frames } else { 0 },
            "features": {
                "gpu": cfg!(feature = "gpu"), "ffmpeg": cfg!(feature = "ffmpeg"),
                "formula": cfg!(feature = "formula"), "diagram": cfg!(feature = "diagram"),
            },
            "workload_fingerprint": support::fingerprint(&workload),
        }),
    )?;
    let mut rejected = Vec::new();
    for &renderer in &renderers {
        let base = composition(size, renderer, &resources)?;
        for case in &cases {
            let composition = case_composition(&base, case);
            let mut session = composition.frame_renderer(DESIGN)?;
            let name = format!("{renderer:?}/{case}");
            let result = (|| {
                for (index, frame) in
                    replay_frames(&mut session, renderer, &name, case, particle_count)?
                {
                    report.reference(&format!("{renderer:?}-{case}-{index}.ppm"), &ppm(&frame))?;
                }
                record_frame(
                    &mut report,
                    &mut session,
                    name.clone(),
                    case,
                    particle_count,
                    &settings,
                )?;
                if MEMORY_CASES.contains(&case.as_str()) {
                    record_memory_frame(
                        &mut report,
                        &composition,
                        renderer,
                        &name,
                        case,
                        particle_count,
                        &settings,
                    )?;
                }
                if sequences {
                    record_sequences(
                        &mut report,
                        &composition,
                        &name,
                        case,
                        particle_count,
                        sequence_frames,
                        &settings,
                    )?;
                }
                if cold {
                    record_cold(
                        &mut report,
                        &composition,
                        renderer,
                        &name,
                        case,
                        particle_count,
                    )?;
                }
                Ok(())
            })();
            skip_rejected(&name, &mut rejected, result)?;
            if stages && renderer == BackendKind::Cpu {
                cpu_stages(
                    &mut report,
                    case,
                    size,
                    limits(),
                    &resources,
                    &settings,
                    particle_count,
                )?;
            }
        }
        if scaling {
            for &scaled in &sizes {
                let composition = self::composition(scaled, renderer, &resources)?;
                let mut session = composition.frame_renderer(DESIGN)?;
                for case in SCALING_CASES
                    .iter()
                    .filter(|case| cases.iter().any(|name| name == *case))
                {
                    let name = format!("{renderer:?}/{case}@{}x{}", scaled.width, scaled.height);
                    let result = record_frame(
                        &mut report,
                        &mut session,
                        name.clone(),
                        case,
                        particle_count,
                        &settings,
                    );
                    skip_rejected(&name, &mut rejected, result)?;
                }
            }
        }
    }
    rejected.sort();
    report.reference("rejected.txt", rejected.join("\n").as_bytes())?;
    if micro {
        microbenchmarks(&mut report, &settings)?;
        primitive_microbenchmarks(&mut report, &settings, &resources)?;
    }
    if video {
        video_benchmark(
            &mut report,
            &settings,
            size,
            video_frames,
            &renderers,
            limits(),
        )?;
    }
    report.finish()?;
    Ok(())
}

#[cfg(test)]
#[allow(
    dead_code,
    unused_imports,
    reason = "Test functions are stripped from harness-less bench builds"
)]
mod _case_tests {
    use super::*;
    use motion::render::{Composition, RasterLimits};

    #[test]
    fn profiling_counts_are_explicit_and_bounded() {
        for value in ["0", "-1", "10001", "invalid"] {
            assert!(parse_count(value, 10_000).is_err());
        }
        assert_eq!(parse_count("10000", 10_000).unwrap(), 10_000);
        assert_eq!(parse_count("60", 900).unwrap(), 60);
        assert!(parse_count("901", 900).is_err());
    }

    #[test]
    fn repeated_even_batches_retain_a_stable_checksum() -> motion::Result {
        let settings = support::Settings {
            warmups: 1,
            samples: 5,
            iterations: 4,
        };
        let first = support::measure(|| Ok(7), &settings, 4)?.checksum;
        let second = support::measure(|| Ok(7), &settings, 4)?.checksum;
        assert_ne!(first, "0000000000000000");
        assert_eq!(first, second);
        Ok(())
    }

    #[test]
    fn size_lists_are_bounded() {
        assert_eq!(parse_sizes("32x18,1920x1080").unwrap().len(), 2);
        for value in ["", "10x10", "5000x100", "100", "ax1"] {
            assert!(parse_sizes(value).is_err(), "{value}");
        }
    }

    #[test]
    fn case_lists_are_unique_and_consistent() {
        for list in [CASES, BREADTH_CASES, SCALING_CASES, MEMORY_CASES] {
            let mut sorted = list.to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), list.len());
        }
        assert!(CASES.iter().all(|case| !BREADTH_CASES.contains(case)));
        assert!(
            SCALING_CASES
                .iter()
                .all(|case| BREADTH_CASES.contains(case))
        );
        assert!(MEMORY_CASES.iter().all(|case| BREADTH_CASES.contains(case)));
    }

    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT; functional render only, no timing/GPU/window"]
    fn all_benchmark_artworks_render() -> motion::Result {
        let font =
            std::env::var_os("MOTION_TEST_FONT").ok_or_else(|| invalid("Set MOTION_TEST_FONT"))?;
        let mut fonts = FontLibrary::new();
        fonts.load_file(font)?;
        let resources = Assets::new(".")?.with_fonts(fonts);
        let composition = Composition::new(
            160,
            90,
            motion::render::FrameClock::new(motion::fps!(30), 90),
        )
        .with_options(
            motion::render::RenderOptions::default()
                .resources(resources)
                .limits(RasterLimits {
                    max_bytes: 1024 * 1024 * 1024,
                    max_work: 400_000_000,
                    ..RasterLimits::default()
                }),
        )?;
        let mut renderer = composition.frame_renderer(DESIGN)?;
        for case in CASES {
            let first = renderer.render_frame(45, |context| scene(case, context, 72))?;
            let _other = renderer.render_frame(17, |context| scene(case, context, 72))?;
            let replay = renderer.render_frame(45, |context| scene(case, context, 72))?;
            assert_eq!(
                first.packed_rgb(),
                replay.packed_rgb(),
                "benchmark replay {case}"
            );
            if *case != "empty" {
                assert!(
                    first
                        .packed_rgb()
                        .iter()
                        .any(|pixel| *pixel != first.packed_rgb()[0]),
                    "empty benchmark {case}"
                );
            }
        }
        Ok(())
    }

    /// Cases that take many seconds per frame in unoptimized builds.
    const HEAVY: &[&str] = &[
        "stroke-dash",
        "path-complex",
        "nodes-10k",
        "nodes-50k",
        "particles-ballistic",
        "particles-simulation",
    ];

    fn render_breadth_cases(heavy: bool) -> motion::Result {
        let (resources, _) = load_resources()?;
        let base = composition(PixelSize::new(160, 90), BackendKind::Cpu, &resources)?;
        for case in BREADTH_CASES
            .iter()
            .filter(|case| HEAVY.contains(case) == heavy)
        {
            let mut session = case_composition(&base, case).frame_renderer(DESIGN)?;
            replay_frames(&mut session, BackendKind::Cpu, case, case, 72)?;
        }
        Ok(())
    }

    #[test]
    fn light_breadth_cases_render_a_stable_non_flat_frame() -> motion::Result {
        render_breadth_cases(false)
    }

    #[test]
    #[ignore = "slow in unoptimized builds; run with --release --include-ignored"]
    fn heavy_breadth_cases_render_a_stable_non_flat_frame() -> motion::Result {
        render_breadth_cases(true)
    }

    #[test]
    fn stage_pipeline_runs_for_a_small_case() -> motion::Result {
        use motion::extend::RenderPrepare;
        use motion::render::{SceneCache, ValidationLimits};

        let (resources, _) = load_resources()?;
        let cache = SceneCache::default();
        let rate = motion::fps!(30);
        let context = SceneContext::new(Time::from_frame(45, rate), rate, DESIGN, &cache)
            .with_assets(&resources);
        let node = scene("layers-wide", &context, 72);
        let list = node.compile_with_fonts(
            DESIGN,
            PixelSize::new(160, 90),
            ValidationLimits::default(),
            resources.fonts().unwrap(),
        )?;
        let linear = list
            .prepare(ValidationLimits::default(), limits())?
            .render_linear(limits())?;
        assert!(linear.linear_pixels().any(|pixel| pixel[3] > 0.0));
        Ok(())
    }
}
