//! Frame-range selection, worker sizing and per-frame rendering for export.

use super::{Codec, ExportOptions, invalid};
use motion_core::{FrameRate, Result, Time, Timeline};
use motion_render::{FrameBuffer, PixelFormat};

pub(super) fn selected(
    timeline: &Timeline,
    options: &ExportOptions,
) -> Result<(ExportOptions, i64, i64)> {
    options.validate_encoding()?;
    if !(1..=64).contains(&options.workers) || options.max_parallel_bytes == 0 {
        return Err(invalid(
            "workers",
            "Require 1..=64 workers and positive parallel allowance",
        ));
    }
    let range = options
        .range
        .clone()
        .unwrap_or(Time::ZERO..timeline.duration().after_origin());
    if range.start < Time::ZERO
        || range.start >= range.end
        || range.end > timeline.duration().after_origin()
    {
        return Err(invalid("range", "Require 0<=start<end<=timeline duration"));
    }
    let rate = output_rate(timeline, options);
    let ceil = |time: Time| {
        let frame = time.to_frame(rate);
        if Time::from_frame(frame, rate) < time {
            frame + 1
        } else {
            frame
        }
    };
    let first = ceil(range.start);
    let end = ceil(range.end);
    if first >= end {
        return Err(invalid("range", "Range contains no frame timestamp"));
    }
    let mut options = options.clone();
    options.render.size = Some(motion_render::options::size(timeline, &options.render)?);
    options.range = Some(Time::from_frame(first, rate)..Time::from_frame(end, rate));
    Ok((options, first, end))
}

pub(super) fn output_rate(timeline: &Timeline, options: &ExportOptions) -> FrameRate {
    match options.codec {
        Codec::Gif { frame_rate, .. } => frame_rate,
        _ => timeline.frame_rate(),
    }
}

pub(super) fn render_workers(
    options: &ExportOptions,
    frames: i64,
    observed_peak: u64,
    backend: Option<&motion_render::BackendDecision>,
) -> usize {
    if !backend.is_some_and(|decision| decision.renderer == motion_render::Renderer::Cpu) {
        return 1;
    }
    let slot_bytes = observed_peak.saturating_mul(2).max(1);
    options
        .workers
        .min(usize::try_from(frames).unwrap_or(usize::MAX))
        .min(usize::try_from(options.max_parallel_bytes / slot_bytes).unwrap_or(usize::MAX))
        .max(1)
}

pub(super) fn render_frame(
    renderer: &mut motion_render::FrameRenderer,
    timeline: &Timeline,
    options: &ExportOptions,
    rate: FrameRate,
    format: PixelFormat,
    frame: i64,
) -> Result<FrameBuffer> {
    let parallel_cpu = options.workers > 1
        && (options.render.backend == motion_render::BackendPreference::Cpu
            || renderer
                .backend_decision()
                .is_some_and(|decision| decision.renderer == motion_render::Renderer::Cpu));
    let render = || {
        renderer.render_output_at(
            Time::from_frame(frame, rate),
            options.render.background,
            format,
            |context| timeline.render(context),
        )
    };
    let result = if parallel_cpu {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(render)).unwrap_or_else(|payload| {
            Err(motion_core::Error::scene_panic(
                "export",
                frame as u64,
                payload,
            ))
        })
    } else {
        let mut render = render;
        render()
    };
    result.map_err(|error| error.at_frame(frame as u64))
}
