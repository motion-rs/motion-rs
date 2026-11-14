//! Performance recorders for warm frames, memory-tracked frames, sequential and random-seek
//! playback, and cold sessions. MOTION_BENCH_TIMINGS=1 adds diagnostic host/device
//! observations to warm frames, with one iteration per sample.

use super::harness::checksum;
use super::{DESIGN, invalid, scene, support};
use motion::prelude::*;
use motion::render::BackendKind;
use motion::render::Composition;

pub(super) fn record_frame(
    report: &mut support::Report,
    session: &mut motion::render::FrameRenderer,
    name: String,
    case: &str,
    particle_count: usize,
    settings: &support::Settings,
) -> motion::Result {
    use serde_json::json;
    let observe = std::env::var("MOTION_BENCH_TIMINGS").unwrap_or_default() == "1";
    if observe && settings.iterations != 1 {
        return Err(invalid(
            "Frame timing diagnostics require one iteration per sample",
        ));
    }
    session.enable_timings(observe);
    let mut observations = Vec::new();
    let measured = support::measure_output(
        || {
            let snapshot =
                session.render_frame(45, |context| scene(case, context, particle_count))?;
            Ok((snapshot, session.timings()))
        },
        |(snapshot, timings)| {
            if let Some(timings) = timings {
                observations.push(timings);
            }
            checksum(&snapshot.packed_rgb())
        },
        settings,
        settings.iterations,
    )?;
    let stats = *session.last_stats().expect("render stats");
    let mut metrics = json!({
        "adapter": format!("{:?}", session.gpu_info()),
        "peak_bytes": stats.peak_bytes(), "work": stats.work(),
        "prepare_retained_bytes": stats.preparation.retained_bytes,
        "prepare_peak_bytes": stats.preparation.peak_bytes,
        "prepare_work": stats.preparation.work,
        "linear_peak_bytes": stats.linear.peak_bytes, "linear_work": stats.linear.work,
        "output_peak_bytes": stats.output.peak_bytes, "output_work": stats.output.work,
    });
    if observe {
        if observations.len() != settings.warmups + settings.samples {
            return Err(invalid("Missing diagnostic frame observations"));
        }
        let gpu_status = support::gpu_timing_status(session);
        metrics["diagnostic_timings"] = json!({
            "unit": "microseconds", "warmups_excluded": true,
            "performance_acceptance": false,
            "samples": observations.into_iter().skip(settings.warmups)
                .map(|timings| support::frame_timing_json(timings, gpu_status))
                .collect::<Vec<_>>(),
        });
    }
    session.enable_timings(false);
    report.record(format!("{name}/warm-frame"), measured, metrics)
}

/// Measures `render_output_at` as straight RGBA8, the adaptive-band path, and requires
/// it to agree with `render_frame`: exactly on the CPU, within the GPU tolerance otherwise.
pub(super) fn record_memory_frame(
    report: &mut support::Report,
    composition: &Composition,
    renderer: BackendKind,
    name: &str,
    case: &str,
    particle_count: usize,
    settings: &support::Settings,
) -> motion::Result {
    use serde_json::json;
    let mut session = composition.frame_renderer(DESIGN)?;
    let format = motion::render::PixelFormat::SrgbRgba8 {
        premultiplied: false,
    };
    let time = Time::from_frame(45, motion::fps!(30));
    let render = |session: &mut motion::render::FrameRenderer| {
        session.render_output_at(time, Some(Color::WHITE), format, |context| {
            scene(case, context, particle_count)
        })
    };
    let reference = render(&mut session)?;
    let snapshot = session.render_frame(45, |context| scene(case, context, particle_count))?;
    let mut maximum = 0_u8;
    let mut over_one = 0_usize;
    for (pixel, expected) in reference.planes()[0]
        .bytes()
        .as_chunks::<4>()
        .0
        .iter()
        .zip(snapshot.bytes().as_chunks::<4>().0)
    {
        let difference = pixel
            .iter()
            .zip(expected)
            .map(|(value, expected)| value.abs_diff(*expected))
            .max()
            .unwrap();
        maximum = maximum.max(difference);
        over_one += usize::from(difference > 1);
    }
    let pixels = snapshot.bytes().len() / 4;
    let exact = renderer == BackendKind::Cpu;
    if (exact && maximum != 0) || (!exact && (maximum > 2 || over_one * 1000 >= pixels)) {
        return Err(invalid(format!(
            "Memory output differs from snapshot: {name}, max={maximum}, over_one={over_one}"
        )));
    }
    session.render_frame(76, |context| scene(case, context, particle_count))?;
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
    particle_count: usize,
    frames: u32,
    settings: &support::Settings,
) -> motion::Result {
    use serde_json::json;
    for shuffled in [false, true] {
        let order = support::frame_order(frames, shuffled);
        let mut playback = composition.frame_renderer(DESIGN)?;
        let measured = support::measure_frames(
            &order,
            |frame| playback.render_frame(frame, |context| scene(case, context, particle_count)),
            |snapshot| checksum(&snapshot.packed_rgb()),
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

/// Measures one frame on a fresh session, including GPU device creation on the GPU backend.
pub(super) fn record_cold(
    report: &mut support::Report,
    composition: &Composition,
    renderer: BackendKind,
    name: &str,
    case: &str,
    particle_count: usize,
) -> motion::Result {
    use serde_json::json;
    let settings = support::Settings::from_env()?;
    let mut adapter = String::new();
    let measured = support::measure_output(
        || {
            let mut fresh = composition.frame_renderer(DESIGN)?;
            let snapshot =
                fresh.render_frame(45, |context| scene(case, context, particle_count))?;
            adapter = format!("{:?}", fresh.gpu_info());
            Ok((fresh, snapshot))
        },
        |(_, snapshot)| checksum(&snapshot.packed_rgb()),
        &settings,
        1,
    )?;
    report.record(
        format!("{name}/cold-session"),
        measured,
        json!({"adapter": adapter, "includes_gpu_device_creation": renderer == BackendKind::Gpu}),
    )
}
