//! Per-stage CPU timings for the performance target: scene build, compile, prepare,
//! linear render and sRGB output, each measured separately.

use super::artwork::artwork_with_particles;
use super::support;
use motion::extend::RenderPrepare;
use motion::prelude::*;
use motion::render::{RasterLimits, SceneCache, ValidationLimits};
use std::hint::black_box;

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
        Time::from_frame(45, FrameRate::from(30)),
        FrameRate::from(30),
        Size::new(640.0, 360.0),
        &cache,
    )
    .with_assets(resources);
    let measured = support::measure_output(
        || Ok(artwork_with_particles(case, &context, particle_count)),
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
            ValidationLimits::default(),
            resources.fonts().expect("benchmark font environment"),
        )?;
        Ok::<_, Error>(list.with_background(Some(Color::WHITE)))
    };
    let fresh = support::measure_prepared(
        || Ok(artwork_with_particles(case, &context, particle_count)),
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
    let node = artwork_with_particles(case, &context, particle_count);
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
        || Ok(list.prepare(ValidationLimits::default(), limits)?),
        |prepared| {
            black_box(prepared);
            45
        },
        settings,
        settings.iterations,
    )?;
    let prepared = list.prepare(ValidationLimits::default(), limits)?;
    report.record(
        format!("Software/{case}/stage-prepare"),
        measured,
        json!({"peak_bytes":prepared.stats().peak_bytes,"work":prepared.stats().work}),
    )?;
    let measured = support::measure_output(
        || Ok(prepared.render_linear(limits)?),
        |linear| {
            linear.pixels().iter().step_by(64).fold(0, |state, pixel| {
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
        json!({"peak_bytes":linear.stats().peak_bytes,"work":linear.stats().work}),
    )?;
    let measured = support::measure_output(
        || Ok(linear.to_srgb_rgba8(false, limits)?),
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
            let reference = motion::render::FrameBuffer::from_linear(&linear, format, limits)?;
            for (index, plane) in reference.planes().iter().enumerate() {
                report.reference(
                    &format!("Cpu-{case}-yuv{bit_depth}-plane{index}.bin"),
                    plane.bytes(),
                )?;
            }
            let measured = support::measure_output(
                || {
                    Ok(motion::render::FrameBuffer::from_linear(
                        &linear, format, limits,
                    )?)
                },
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
