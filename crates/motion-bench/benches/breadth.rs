//! Breadth benchmarks: the content, scale and pipeline stages that `performance` does not cover.
//!
//! `performance` freezes a small set of 640x360 cohorts for regression tracking. This target
//! instead widens coverage. Every case below is a deterministic scene rendered through the same
//! frame renderer, plus per-stage timings and primitive microbenchmarks.
//!
//! Environment (all optional):
//! - `MOTION_BENCH_CASES`: comma-separated render cases, default all.
//! - `MOTION_BENCH_BACKENDS`: `cpu`, `gpu` or both (gpu needs the `gpu` feature).
//! - `MOTION_BENCH_SIZE`: pixel size of the render cases, default 640x360.
//! - `MOTION_BENCH_SCALING`: `1` (default) measures `SCALING_CASES` at `MOTION_BENCH_SIZES`.
//! - `MOTION_BENCH_SIZES`: comma-separated `WxH` list, default `1280x720,1920x1080`.
//! - `MOTION_BENCH_STAGES`: `1` (default) records CPU stage timings for `STAGE_CASES`.
//! - `MOTION_BENCH_SEQUENCES`: `1` (default) records sequential and random-seek playback.
//! - `MOTION_BENCH_MICRO`: `1` (default) records primitive microbenchmarks.
//! - `MOTION_BENCH_FONT`: font file for text cases, default the repository Noto Sans fixture.
//!
//! `cargo test -p motion-bench --test breadth_cases` renders every case once at 160x90 without
//! timing, so scene regressions surface without a full benchmark run.

use harness::{composition, flag, limits, load_resources, parse_sizes, ppm, verify_replay};
use micro::microbenchmarks;
use motion::prelude::*;
use recording::{cpu_stages, record_frame, record_memory_frame, record_sequences};
use scenes::artwork;

#[path = "breadth/harness.rs"]
mod harness;
#[path = "breadth/micro.rs"]
mod micro;
#[path = "breadth/recording.rs"]
mod recording;
#[path = "breadth/scenes.rs"]
mod scenes;
#[path = "support.rs"]
mod support;

/// Render cases measured as warm frames, sequential playback and random seeks.
const CASES: &[&str] = &[
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

/// Cases whose CPU pipeline stages are timed separately.
const STAGE_CASES: &[&str] = &[
    "gradient-linear",
    "blend-modes",
    "clip-path",
    "path-complex",
    "layers-wide",
    "nodes-10k",
    "layout-grid",
    "blur-large",
    "text-paragraph",
];

const DESIGN: Size = Size::new(640.0, 360.0);

const MAX_NODES: usize = 100_016;

fn invalid(message: impl Into<String>) -> Error {
    support::invalid(message)
}

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
    let cases = support::selected("MOTION_BENCH_CASES", CASES)?;
    let renderers = support::renderers(cfg!(feature = "gpu"))?;
    let scaling = flag("MOTION_BENCH_SCALING", true);
    let sizes = parse_sizes(
        &std::env::var("MOTION_BENCH_SIZES").unwrap_or_else(|_| "1280x720,1920x1080".into()),
    )?;
    let stages = flag("MOTION_BENCH_STAGES", true);
    let sequences = flag("MOTION_BENCH_SEQUENCES", true);
    let sequence_frames = if sequences {
        support::sequence_length(90)?
    } else {
        0
    };
    let micro = flag("MOTION_BENCH_MICRO", true);
    let (resources, font) = load_resources()?;
    let mut report = Report::new(
        "breadth",
        settings.clone(),
        json!({
            "size": [size.width, size.height], "design": [640, 360], "cases": cases,
            "scaling": scaling, "sizes": sizes.iter().map(|size| [size.width, size.height]).collect::<Vec<_>>(),
            "backends": renderers.iter().map(|value| format!("{value:?}")).collect::<Vec<_>>(),
            "frames": [0, 45, 89], "frame_rate": [30, 1], "stages": stages,
            "micro": micro, "sequence_frames": sequence_frames, "font": font,
            "limits": {"bytes": limits().max_bytes, "work": limits().max_work},
            "features": {"gpu": cfg!(feature = "gpu"), "diagram": cfg!(feature = "diagram")},
            "workload_fingerprint": support::fingerprint(include_bytes!("breadth.rs")),
        }),
    )?;
    // Engine admission limits are findings, not failures: a rejected case is listed in
    // `rejected.txt` and the remaining cases still run.
    let mut rejected = Vec::new();
    for &renderer in &renderers {
        let base = composition(size, renderer, &resources)?;
        for case in &cases {
            let composition = if case == "display-transform" {
                base.clone().display(DisplayTransform {
                    tone_map: ToneMap::AgX,
                    grain: Some(FilmGrain {
                        seed: 9,
                        amount: 0.01,
                        size: 1.4,
                        luma_response: 1.0,
                        chroma: 0.1,
                    }),
                    ..DisplayTransform::default()
                })
            } else {
                base.clone()
            };
            let mut session = composition.frame_renderer(DESIGN)?;
            let name = format!("{renderer:?}/{case}");
            let result = (|| {
                verify_replay(&mut session, &name, case)?;
                let reference = session.render_frame(45, |context| artwork(case, context))?;
                report.reference(&format!("{renderer:?}-{case}-45.ppm"), &ppm(&reference))?;
                record_frame(&mut report, &mut session, name.clone(), case, &settings)?;
                if MEMORY_CASES.contains(&case.as_str()) {
                    record_memory_frame(
                        &mut report,
                        &composition,
                        renderer,
                        &name,
                        case,
                        &settings,
                    )?;
                }
                if sequences {
                    record_sequences(
                        &mut report,
                        &composition,
                        &name,
                        case,
                        sequence_frames,
                        &settings,
                    )?;
                }
                Ok(())
            })();
            skip_rejected(&name, &mut rejected, result)?;
            if stages && renderer == Renderer::Cpu && STAGE_CASES.contains(&case.as_str()) {
                cpu_stages(&mut report, case, size, &resources, &settings)?;
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
                    let result =
                        record_frame(&mut report, &mut session, name.clone(), case, &settings);
                    skip_rejected(&name, &mut rejected, result)?;
                }
            }
        }
    }
    rejected.sort();
    report.reference("rejected.txt", rejected.join("\n").as_bytes())?;
    if micro {
        microbenchmarks(&mut report, &settings, &resources)?;
    }
    report.finish()?;
    Ok(())
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "Test functions are stripped from harness-less bench builds"
)]
mod _case_tests {
    use super::*;

    #[test]
    fn size_lists_are_bounded() {
        assert_eq!(parse_sizes("32x18,1920x1080").unwrap().len(), 2);
        for value in ["", "10x10", "5000x100", "100", "ax1"] {
            assert!(parse_sizes(value).is_err(), "{value}");
        }
    }

    #[test]
    fn case_lists_are_unique_and_consistent() {
        for list in [CASES, SCALING_CASES, STAGE_CASES, MEMORY_CASES] {
            let mut sorted = list.to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), list.len());
        }
        assert!(SCALING_CASES.iter().all(|case| CASES.contains(case)));
        assert!(STAGE_CASES.iter().all(|case| CASES.contains(case)));
        assert!(MEMORY_CASES.iter().all(|case| CASES.contains(case)));
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

    fn render_cases(heavy: bool) -> motion::Result {
        let (resources, _) = load_resources()?;
        let composition = composition(PixelSize::new(160, 90), Renderer::Cpu, &resources)?;
        for case in CASES.iter().filter(|case| HEAVY.contains(case) == heavy) {
            let configured = if *case == "display-transform" {
                composition.clone().display(DisplayTransform {
                    tone_map: ToneMap::AgX,
                    grain: Some(FilmGrain {
                        seed: 9,
                        amount: 0.01,
                        size: 1.4,
                        luma_response: 1.0,
                        chroma: 0.1,
                    }),
                    ..DisplayTransform::default()
                })
            } else {
                composition.clone()
            };
            let mut session = configured.frame_renderer(DESIGN)?;
            verify_replay(&mut session, case, case)?;
        }
        Ok(())
    }

    #[test]
    fn light_breadth_cases_render_a_stable_non_flat_frame() -> motion::Result {
        render_cases(false)
    }

    #[test]
    #[ignore = "slow in unoptimized builds; run with --release --include-ignored"]
    fn heavy_breadth_cases_render_a_stable_non_flat_frame() -> motion::Result {
        render_cases(true)
    }

    #[test]
    fn stage_pipeline_runs_for_a_small_case() -> motion::Result {
        use motion::extend::RenderPrepare;
        use motion::render::{SceneCache, ValidationLimits};

        let (resources, _) = load_resources()?;
        let cache = SceneCache::default();
        let rate = FrameRate::from(30);
        let context = SceneContext::new(Time::from_frame(45, rate), rate, DESIGN, &cache)
            .with_assets(&resources);
        let node = artwork("layers-wide", &context);
        let list = node.compile_with_fonts(
            DESIGN,
            PixelSize::new(160, 90),
            ValidationLimits::default(),
            resources.fonts().unwrap(),
        )?;
        let linear = list
            .prepare(ValidationLimits::default(), limits())?
            .render_linear(limits())?;
        assert!(linear.pixels().iter().any(|pixel| pixel[3] > 0.0));
        Ok(())
    }
}
