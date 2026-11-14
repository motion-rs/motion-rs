//! Per-stage CPU timings for the performance target: scene build, compile, prepare,
//! linear render and sRGB output, each measured separately.

use super::{DESIGN, MAX_NODES, scene, support};
use motion::extend::RenderPrepare;
use motion::prelude::*;
use motion::render::{RasterLimits, SceneCache, ValidationLimits};
use std::hint::black_box;

fn validation() -> ValidationLimits {
    ValidationLimits {
        max_nodes: MAX_NODES,
        ..ValidationLimits::default()
    }
}

pub(super) fn cpu_stages(
    report: &mut support::Report,
    case: &str,
    size: PixelSize,
    limits: RasterLimits,
    resources: &Assets,
    settings: &support::Settings,
    particle_count: usize,
) -> motion::Result {
    use serde_json::json;
    let cache = SceneCache::default();
    let context = SceneContext::new(
        Time::from_frame(45, motion::fps!(30)),
        motion::fps!(30),
        DESIGN,
        &cache,
    )
    .with_assets(resources);
    let measured = support::measure_output(
        || Ok(scene(case, &context, particle_count)),
        |node| {
            black_box(node);
            45
        },
        settings,
        settings.iterations,
    )?;
    report.record(
        format!("Software/{case}/stage-build"),
        measured,
        json!({"immutable_caches_warm":true}),
    )?;
    let compile_node = |node: &Node| {
        let list = node.compile_with_fonts(
            context.size(),
            size,
            validation(),
            resources.fonts().expect("benchmark font environment"),
        )?;
        Ok::<_, Error>(list.with_background(Some(Color::WHITE)))
    };
    let fresh = support::measure_prepared(
        || Ok(scene(case, &context, particle_count)),
        compile_node,
        |list| {
            black_box(list);
            45
        },
        settings,
    )?;
    report.record(
        format!("Software/{case}/stage-compile-fresh-node"),
        fresh,
        json!({"includes_first_drawable_preparation":true,"build_outside_timer":true}),
    )?;
    let node = scene(case, &context, particle_count);
    let measured = support::measure_output(
        || compile_node(&node),
        |list| {
            black_box(list);
            45
        },
        settings,
        settings.iterations,
    )?;
    report.record(
        format!("Software/{case}/stage-compile-hot-node"),
        measured,
        json!({"input_reused":true}),
    )?;
    let list = compile_node(&node)?;
    let measured = support::measure_output(
        || Ok(list.prepare(validation(), limits)?),
        |prepared| {
            black_box(prepared);
            45
        },
        settings,
        settings.iterations,
    )?;
    let prepared = list.prepare(validation(), limits)?;
    report.record(
        format!("Software/{case}/stage-prepare"),
        measured,
        json!({"peak_bytes":prepared.stats().peak_bytes,"work":prepared.stats().work}),
    )?;
    let measured = support::measure_output(
        || Ok(prepared.render_linear(limits)?),
        |linear| {
            linear.linear_pixels().step_by(64).fold(0, |state, pixel| {
                support::fold_checksum(state, u64::from(pixel[3].to_bits()))
            })
        },
        settings,
        settings.iterations,
    )?;
    let linear = prepared.render_linear(limits)?;
    report.record(
        format!("Software/{case}/stage-linear"),
        measured,
        json!({"peak_bytes": linear.byte_len(), "work": linear.linear_pixels().len()}),
    )?;
    let measured = support::measure_output(
        || {
            Ok(linear.to_format(
                motion::render::PixelFormat::SrgbRgba8 {
                    premultiplied: false,
                },
                limits,
            )?)
        },
        |output| {
            output.bytes().iter().step_by(256).fold(0, |state, value| {
                support::fold_checksum(state, u64::from(*value))
            })
        },
        settings,
        settings.iterations,
    )?;
    report.record(
        format!("Software/{case}/stage-output"),
        measured,
        json!({"source_and_prepared_pinned":true}),
    )?;
    if size.width.is_multiple_of(2) && size.height.is_multiple_of(2) {
        for bit_depth in [8, 10] {
            let format = motion::render::PixelFormat::Yuv420 { bit_depth };
            let reference = linear.to_format(format, limits)?;
            for (index, plane) in reference.planes().iter().enumerate() {
                report.reference(
                    &format!("Cpu-{case}-yuv{bit_depth}-plane{index}.bin"),
                    plane.bytes(),
                )?;
            }
            let measured = support::measure_output(
                || Ok(linear.to_format(format, limits)?),
                |output| {
                    output.planes().iter().fold(0, |state, plane| {
                        plane
                            .bytes()
                            .iter()
                            .step_by(256)
                            .fold(state, |state, value| {
                                support::fold_checksum(state, u64::from(*value))
                            })
                    })
                },
                settings,
                settings.iterations,
            )?;
            report.record(
                format!("Software/{case}/stage-yuv{bit_depth}"),
                measured,
                json!({"source_and_prepared_pinned":true,"output_bytes":reference.byte_len(),"work":u64::from(size.width)*u64::from(size.height)*3}),
            )?;
        }
    }
    Ok(())
}
