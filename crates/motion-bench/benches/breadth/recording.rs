//! Breadth benchmark recorders for warm frames, memory-tracked frames, sequential and
//! random-seek playback, and per-stage CPU pipeline timings.

use super::harness::{checksum, limits};
use super::scenes::artwork;
use super::{DESIGN, MAX_NODES, invalid, support};
use motion::extend::RenderPrepare;
use motion::prelude::*;
use motion::render::{Composition, SceneCache, ValidationLimits};
use std::hint::black_box;

pub(super) fn record_frame(
    report: &mut support::Report,
    session: &mut motion::render::FrameRenderer,
    name: String,
    case: &str,
    settings: &support::Settings,
) -> motion::Result {
    use serde_json::json;
    let measured = support::measure_output(
        || session.render_frame(45, |context| artwork(case, context)),
        |snapshot| checksum(snapshot.pixels()),
        settings,
        settings.iterations,
    )?;
    let stats = *session.last_stats().expect("render stats");
    report.record(
        format!("{name}/warm-frame"),
        measured,
        json!({
            "adapter": format!("{:?}", session.gpu_info()),
            "peak_bytes": stats.peak_bytes(), "work": stats.work(),
            "prepare_retained_bytes": stats.preparation.retained_bytes,
            "prepare_work": stats.preparation.work,
            "linear_peak_bytes": stats.linear.peak_bytes, "linear_work": stats.linear.work,
        }),
    )
}

/// Measures `render_output_at` as straight RGBA8, the adaptive-band path, and requires
/// it to agree with `render_frame`: exactly on the CPU, within the GPU tolerance otherwise.
pub(super) fn record_memory_frame(
    report: &mut support::Report,
    composition: &Composition,
    renderer: Renderer,
    name: &str,
    case: &str,
    settings: &support::Settings,
) -> motion::Result {
    use serde_json::json;
    let mut session = composition.frame_renderer(DESIGN)?;
    let format = motion::render::PixelFormat::SrgbRgba8 {
        premultiplied: false,
    };
    let time = Time::from_frame(45, FrameRate::from(30));
    let render = |session: &mut motion::render::FrameRenderer| {
        session.render_output_at(time, Some(Color::WHITE), format, |context| {
            artwork(case, context)
        })
    };
    let reference = render(&mut session)?;
    let snapshot = session.render_frame(45, |context| artwork(case, context))?;
    let mut maximum = 0_u8;
    let mut over_one = 0_usize;
    for (pixel, expected) in reference.planes()[0]
        .bytes()
        .as_chunks::<4>()
        .0
        .iter()
        .zip(snapshot.pixels())
    {
        let expected = [
            (*expected >> 16) as u8,
            (*expected >> 8) as u8,
            *expected as u8,
            255,
        ];
        let difference = pixel
            .iter()
            .zip(expected)
            .map(|(value, expected)| value.abs_diff(expected))
            .max()
            .unwrap();
        maximum = maximum.max(difference);
        over_one += usize::from(difference > 1);
    }
    let pixels = snapshot.pixels().len();
    let exact = renderer == Renderer::Cpu;
    if (exact && maximum != 0) || (!exact && (maximum > 2 || over_one * 1000 >= pixels)) {
        return Err(invalid(format!(
            "Memory output differs from snapshot: {name}, max={maximum}, over_one={over_one}"
        )));
    }
    session.render_frame(76, |context| artwork(case, context))?;
    if render(&mut session)? != reference {
        return Err(invalid(format!("Memory seek replay differs: {name}")));
    }
    let measured = support::measure_output(
        || render(&mut session),
        |output| {
            output.planes()[0]
                .bytes()
                .iter()
                .step_by(256)
                .fold(0, |state, value| {
                    support::fold_checksum(state, u64::from(*value))
                })
        },
        settings,
        settings.iterations,
    )?;
    let stats = *session.last_stats().expect("memory render stats");
    report.record(
        format!("{name}/memory-frame"),
        measured,
        json!({
            "adapter": format!("{:?}", session.gpu_info()),
            "peak_bytes": stats.peak_bytes(), "work": stats.work(),
            "linear_peak_bytes": stats.linear.peak_bytes, "linear_work": stats.linear.work,
            "output_peak_bytes": stats.output.peak_bytes, "output_work": stats.output.work,
            "format": "SrgbRgba8-straight", "background": "White",
            "snapshot_max_difference": maximum, "snapshot_pixels_over_one": over_one,
        }),
    )
}

pub(super) fn record_sequences(
    report: &mut support::Report,
    composition: &Composition,
    name: &str,
    case: &str,
    frames: u32,
    settings: &support::Settings,
) -> motion::Result {
    use serde_json::json;
    for shuffled in [false, true] {
        let order = support::frame_order(frames, shuffled);
        let mut playback = composition.frame_renderer(DESIGN)?;
        let measured = support::measure_frames(
            &order,
            |frame| playback.render_frame(frame, |context| artwork(case, context)),
            |snapshot| checksum(snapshot.pixels()),
            settings,
        )?;
        report.record(
            format!(
                "{name}/{}",
                if shuffled {
                    "random-seek"
                } else {
                    "sequential-playback"
                }
            ),
            measured,
            json!({"adapter": format!("{:?}", playback.gpu_info()), "complete_timeline": frames == 90}),
        )?;
    }
    Ok(())
}

pub(super) fn cpu_stages(
    report: &mut support::Report,
    case: &str,
    size: PixelSize,
    resources: &Assets,
    settings: &support::Settings,
) -> motion::Result {
    use serde_json::json;
    let cache = SceneCache::default();
    let rate = FrameRate::from(30);
    let context =
        SceneContext::new(Time::from_frame(45, rate), rate, DESIGN, &cache).with_assets(resources);
    let validation = ValidationLimits {
        max_nodes: MAX_NODES,
        ..ValidationLimits::default()
    };
    let fonts = resources.fonts().expect("benchmark font environment");
    let label = |stage: &str| format!("Software/{case}/stage-{stage}");

    let built = support::measure_output(
        || Ok(artwork(case, &context)),
        |node| {
            black_box(node);
            45
        },
        settings,
        settings.iterations,
    )?;
    report.record(
        label("build"),
        built,
        json!({"immutable_caches_warm": true}),
    )?;

    let compile_node = |node: &Node| -> motion::Result<motion::render::ValidatedDisplayList> {
        Ok(node
            .compile_with_fonts(DESIGN, size, validation, fonts)?
            .with_background(Some(Color::WHITE)))
    };
    let fresh = support::measure_prepared(
        || Ok(artwork(case, &context)),
        compile_node,
        |list| {
            black_box(list);
            45
        },
        settings,
    )?;
    report.record(
        label("compile-fresh-node"),
        fresh,
        json!({"includes_first_drawable_preparation": true, "build_outside_timer": true}),
    )?;
    let node = artwork(case, &context);
    let compiled = support::measure_output(
        || compile_node(&node),
        |list| {
            black_box(list);
            45
        },
        settings,
        settings.iterations,
    )?;
    report.record(label("compile"), compiled, json!({"input_reused": true}))?;

    let list = compile_node(&node)?;
    let prepared_timing = support::measure_output(
        || Ok(list.prepare(validation, limits())?),
        |prepared| {
            black_box(prepared);
            45
        },
        settings,
        settings.iterations,
    )?;
    let prepared = list.prepare(validation, limits())?;
    report.record(
        label("prepare"),
        prepared_timing,
        json!({"peak_bytes": prepared.stats().peak_bytes, "work": prepared.stats().work,
               "retained_bytes": prepared.stats().retained_bytes}),
    )?;

    let linear_timing = support::measure_output(
        || Ok(prepared.render_linear(limits())?),
        |linear| {
            linear.pixels().iter().step_by(64).fold(0, |state, pixel| {
                support::fold_checksum(state, u64::from(pixel[3].to_bits()))
            })
        },
        settings,
        settings.iterations,
    )?;
    let linear = prepared.render_linear(limits())?;
    report.record(
        label("linear"),
        linear_timing,
        json!({"peak_bytes": linear.stats().peak_bytes, "work": linear.stats().work}),
    )?;

    let output_timing = support::measure_output(
        || Ok(linear.to_srgb_rgba8(false, limits())?),
        |output| {
            output.bytes().iter().step_by(256).fold(0, |state, value| {
                support::fold_checksum(state, u64::from(*value))
            })
        },
        settings,
        settings.iterations,
    )?;
    report.record(label("output"), output_timing, json!({}))
}
