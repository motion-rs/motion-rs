//! Renderer performance cases, CPU stage timings, microbenchmarks and video export runs.

use artwork::artwork_with_particles;
use micro::microbenchmarks;
use motion::prelude::*;
use motion::render::{Composition, RasterLimits, Snapshot};
use stages::cpu_stages;
use video::video_benchmark;

#[path = "performance/artwork.rs"]
mod artwork;
#[path = "performance/micro.rs"]
mod micro;
#[path = "performance/stages.rs"]
mod stages;
#[path = "support.rs"]
mod support;
#[path = "performance/video.rs"]
mod video;

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

fn invalid(message: impl Into<String>) -> Error {
    Error::Parameters(Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message.into(),
    )))
}

fn parse_count(value: &str, maximum: usize) -> motion::Result<usize> {
    let count = value
        .parse::<usize>()
        .map_err(|_| invalid("Expected a positive benchmark count"))?;
    if !(1..=maximum).contains(&count) {
        return Err(invalid(format!("Benchmark count must be in 1..={maximum}")));
    }
    Ok(count)
}

fn main() -> motion::Result {
    use serde_json::json;
    use support::{Report, Settings};
    let size = support::size((640, 360))?;
    let settings = Settings::from_env()?;
    let cases = support::selected("MOTION_BENCH_CASES", CASES)?;
    let renderers = support::renderers(cfg!(feature = "gpu"))?;
    let cold = std::env::var("MOTION_BENCH_COLD").unwrap_or_default() == "1";
    let stages = std::env::var("MOTION_BENCH_STAGES").unwrap_or_default() == "1";
    let sequences = std::env::var("MOTION_BENCH_SEQUENCES").unwrap_or_else(|_| "1".into()) == "1";
    let sequence_frames = if sequences {
        support::sequence_length(90)?
    } else {
        0
    };
    let micro = std::env::var("MOTION_BENCH_MICRO").unwrap_or_else(|_| "1".into()) == "1";
    let video = std::env::var("MOTION_BENCH_VIDEO").unwrap_or_default() == "1";
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
    let limits = RasterLimits {
        max_bytes: 1024 * 1024 * 1024,
        max_work: 20_000_000_000,
        ..RasterLimits::default()
    };
    let mut fonts = FontLibrary::new();
    let font_fingerprint = if cases.iter().any(|case| case.starts_with("text")) {
        let font = std::env::var_os("MOTION_BENCH_FONT")
            .ok_or_else(|| invalid("Text benchmark requires explicit MOTION_BENCH_FONT"))?;
        let fingerprint = support::fingerprint(&std::fs::read(&font)?);
        fonts.load_file(font)?;
        Some(fingerprint)
    } else {
        None
    };
    let resources = Assets::new(".")?.with_fonts(fonts);
    let mut report = Report::new(
        "engine",
        settings.clone(),
        json!({
            "size":[size.width,size.height], "design":[640,360], "cases":cases,
            "backends":renderers.iter().map(|value|format!("{value:?}")).collect::<Vec<_>>(),
            "frames":[0,45,17,89], "frame_rate":[30,1], "cold":cold,"stages":stages,"micro":micro,"video":video,"sequence_frames":sequence_frames,
            "limits":{"bytes":limits.max_bytes,"work":limits.max_work},"font":font_fingerprint,
            "quality":"Final",
            "particle_count":particle_count,"video_frames":if video {video_frames} else {0},
            "features":{"gpu":cfg!(feature="gpu"),"ffmpeg":cfg!(feature="ffmpeg"),"formula":cfg!(feature="formula")},
            "workload_fingerprint":support::fingerprint(include_bytes!("performance.rs")),
        }),
    )?;
    for &renderer in &renderers {
        let composition = Composition::new(size.width as usize, size.height as usize)
            .frame_count(90)
            .backend(renderer.into())
            .quality(motion::RenderQuality::Final)
            .resources(resources.clone())
            .raster_limits(limits)?;
        let mut session = composition.frame_renderer(Size::new(640.0, 360.0))?;
        for case in &cases {
            let name = format!("{renderer:?}/{case}");
            for index in [0, 45, 17, 89] {
                let first = session.render_frame(index, |context| {
                    artwork_with_particles(case, context, particle_count)
                })?;
                session.render_frame((index + 31) % 90, |context| {
                    artwork_with_particles(case, context, particle_count)
                })?;
                let replay = session.render_frame(index, |context| {
                    artwork_with_particles(case, context, particle_count)
                })?;
                if first.pixels() != replay.pixels() {
                    return Err(invalid(format!("Seek replay differs: {name}/{index}")));
                }
                report.reference(&format!("{renderer:?}-{case}-{index}.ppm"), &ppm(&first))?;
            }
            let measured = support::measure_output(
                || {
                    session.render_frame(45, |context| {
                        artwork_with_particles(case, context, particle_count)
                    })
                },
                |snapshot| checksum(snapshot.pixels()),
                &settings,
                settings.iterations,
            )?;
            let stats = *session.last_stats().expect("render stats");
            let adapter = format!("{:?}", session.gpu_info());
            report.record(format!("{name}/warm-frame"),measured,json!({
                "adapter":adapter,"peak_bytes":stats.peak_bytes(),"work":stats.work(),
                "prepare_retained_bytes":stats.preparation.retained_bytes,
                "prepare_peak_bytes":stats.preparation.peak_bytes,"prepare_work":stats.preparation.work,
                "linear_peak_bytes":stats.linear.peak_bytes,"linear_work":stats.linear.work,
                "output_peak_bytes":stats.output.peak_bytes,"output_work":stats.output.work,
                "pack_peak_bytes":stats.snapshot.peak_bytes,"pack_work":stats.snapshot.work,
            }))?;
            if sequences {
                for shuffled in [false, true] {
                    let frames = support::frame_order(sequence_frames, shuffled);
                    let mut playback = composition.frame_renderer(Size::new(640.0, 360.0))?;
                    let measured = support::measure_frames(
                        &frames,
                        |frame| {
                            playback.render_frame(frame, |context| {
                                artwork_with_particles(case, context, particle_count)
                            })
                        },
                        |snapshot| checksum(snapshot.pixels()),
                        &settings,
                    )?;
                    report.record(format!("{name}/{}",if shuffled {"random-seek"}else{"sequential-playback"}),measured,json!({"adapter":format!("{:?}",playback.gpu_info()),"complete_timeline":sequence_frames==90}))?;
                }
            }
            if cold {
                let measured = support::measure_output(
                    || {
                        let mut fresh = composition.frame_renderer(Size::new(640.0, 360.0))?;
                        let snapshot = fresh.render_frame(45, |context| {
                            artwork_with_particles(case, context, particle_count)
                        })?;
                        Ok((fresh, snapshot))
                    },
                    |(_, snapshot)| checksum(snapshot.pixels()),
                    &settings,
                    1,
                )?;
                report.record(format!("{name}/cold-session"),measured,json!({"adapter":adapter,"includes_gpu_device_creation":renderer==Renderer::Gpu}))?;
            }
            if stages && renderer == Renderer::Cpu {
                cpu_stages(
                    &mut report,
                    case,
                    size,
                    limits,
                    &resources,
                    &settings,
                    particle_count,
                )?;
            }
        }
    }
    if micro {
        microbenchmarks(&mut report, &settings)?;
    }
    if video {
        video_benchmark(
            &mut report,
            &settings,
            size,
            video_frames,
            &renderers,
            limits,
        )?;
    }
    report.finish()?;
    Ok(())
}

fn checksum(pixels: &[u32]) -> u64 {
    pixels.iter().fold(0, |state, pixel| {
        support::fold_checksum(state, u64::from(*pixel))
    })
}

fn ppm(snapshot: &Snapshot) -> Vec<u8> {
    let mut bytes = format!("P6\n{} {}\n255\n", snapshot.width(), snapshot.height()).into_bytes();
    for pixel in snapshot.pixels() {
        bytes.extend_from_slice(&[(pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8]);
    }
    bytes
}

#[cfg(test)]
mod _case_tests {
    #[test]
    fn profiling_counts_are_explicit_and_bounded() {
        for value in ["0", "-1", "10001", "invalid"] {
            assert!(super::parse_count(value, 10_000).is_err());
        }
        assert_eq!(super::parse_count("10000", 10_000).unwrap(), 10_000);
        assert_eq!(super::parse_count("60", 900).unwrap(), 60);
        assert!(super::parse_count("901", 900).is_err());
    }

    #[test]
    fn repeated_even_batches_retain_a_stable_checksum() -> motion::Result {
        let settings = super::support::Settings {
            warmups: 1,
            samples: 5,
            iterations: 4,
        };
        let first = super::support::measure(|| Ok(7), &settings, 4)?.checksum;
        let second = super::support::measure(|| Ok(7), &settings, 4)?.checksum;
        assert_ne!(first, "0000000000000000");
        assert_eq!(first, second);
        Ok(())
    }

    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT; functional render only, no timing/GPU/window"]
    fn all_benchmark_artworks_render() -> motion::Result {
        use super::artwork::artwork;
        use super::*;
        let font =
            std::env::var_os("MOTION_TEST_FONT").ok_or_else(|| invalid("Set MOTION_TEST_FONT"))?;
        let mut fonts = FontLibrary::new();
        fonts.load_file(font)?;
        let resources = Assets::new(".")?.with_fonts(fonts);
        let composition = Composition::new(160, 90)
            .frame_count(90)
            .resources(resources)
            .raster_limits(RasterLimits {
                max_bytes: 1024 * 1024 * 1024,
                max_work: 400_000_000,
                ..RasterLimits::default()
            })?;
        let mut renderer = composition.frame_renderer(Size::new(640.0, 360.0))?;
        for case in CASES {
            let first = renderer.render_frame(45, |context| artwork(case, context))?;
            let _other = renderer.render_frame(17, |context| artwork(case, context))?;
            let replay = renderer.render_frame(45, |context| artwork(case, context))?;
            assert_eq!(first.pixels(), replay.pixels(), "benchmark replay {case}");
            if *case != "empty" {
                assert!(
                    first
                        .pixels()
                        .iter()
                        .any(|pixel| *pixel != first.pixels()[0]),
                    "empty benchmark {case}"
                );
            }
        }
        Ok(())
    }
}
