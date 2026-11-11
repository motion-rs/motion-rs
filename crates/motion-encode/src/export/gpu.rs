//! Bounded CPU preparation followed by single-caller GPU submission and ordered encoding.

use super::*;
use motion_core::{FrameRate, Time};
use motion_render::PixelFormat;
use motion_render::gpu::OutputPlan;
use std::ops::Range;

#[cfg(test)]
mod tests;

/// True when the pipeline applies: the request allows the GPU, the output can be packed on
/// it, and the backend actually selected for this renderer is the GPU. An Automatic request
/// that resolves to the CPU keeps the CPU path.
pub(super) fn eligible(
    options: &ExportOptions,
    format: PixelFormat,
    frames: i64,
    renderer: &mut motion_render::FrameRenderer,
) -> Result<bool> {
    Ok(
        options.render.backend != motion_render::BackendPreference::Cpu
            && frames > 1
            && motion_render::gpu::Gpu::supports_packed_output(
                options.render.size.expect("normalized size"),
                format,
            )
            && renderer.selected_gpu_context()?.is_some(),
    )
}

fn schedule(
    options: &ExportOptions,
    frames: i64,
    preparation_peak: u64,
    output_peak: u64,
) -> (usize, usize, u64, u64) {
    let preparation_cost = preparation_peak.saturating_mul(2).max(1);
    if options.workers == 1
        || preparation_cost
            .checked_add(output_peak)
            .is_none_or(|minimum| minimum > options.max_parallel_bytes)
    {
        return (
            0,
            1,
            options.render.limits.max_bytes,
            options.render.limits.max_bytes,
        );
    }
    let slots = (options.max_parallel_bytes.saturating_sub(preparation_cost) / output_peak.max(1))
        .clamp(1, 3) as usize;
    let available = options
        .max_parallel_bytes
        .saturating_sub(output_peak.saturating_mul(slots as u64));
    let workers = options
        .workers
        .min(usize::try_from(frames).unwrap_or(usize::MAX))
        .min(usize::try_from(available / preparation_cost).unwrap_or(usize::MAX))
        .max(1);
    let preparation_bytes = (available / workers as u64).min(options.render.limits.max_bytes);
    let queue_bytes = options.max_parallel_bytes - preparation_bytes * workers as u64;
    (workers, slots, preparation_bytes, queue_bytes)
}

pub(super) fn render(
    timeline: &Timeline,
    options: &ExportOptions,
    range: Range<i64>,
    rate: FrameRate,
    progress: &dyn ProgressSink,
    encoder: &mut dyn Encoder,
    renderer: &mut motion_render::FrameRenderer,
) -> Result<()> {
    cancelled(progress)?;
    let format = encoder.input_format();
    let context = renderer
        .selected_gpu_context()?
        .ok_or_else(|| invalid("gpu", "Explicit GPU pipeline has no selected context"))?;
    let (first, limits) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        renderer.prepare_output_at(
            Time::from_frame(range.start, rate),
            options.render.background,
            |context| timeline.render(context),
        )
    }))
    .unwrap_or_else(|payload| {
        Err(motion_core::Error::scene_panic(
            "export",
            range.start as u64,
            payload,
        ))
    })
    .map_err(|error| error.at_frame(range.start as u64))?;
    let plan = OutputPlan::new(&first, limits, format)
        .and_then(|plan| plan.with_display(&options.render.display, range.start as u64))
        .map_err(|error| motion_core::Error::from(error).at_frame(range.start as u64))?;
    let peak = plan
        .peak_bytes()
        .map_err(|error| motion_core::Error::from(error).at_frame(range.start as u64))?;
    let preparation_peak = first.stats().peak_bytes.max(plan.planning_peak_bytes());
    let (workers, slots, preparation_bytes, queue_bytes) =
        schedule(options, range.end - range.start - 1, preparation_peak, peak);
    let mut queue = motion_render::gpu::FrameQueue::new(context.clone(), queue_bytes)?;
    queue
        .submit_output(&first, plan)
        .map_err(|error| error.at_frame(range.start as u64))?;
    drop(first);
    let mut completed = range.start;
    let total = (range.end - range.start) as u64;
    let mut encode_oldest = |queue: &mut motion_render::gpu::FrameQueue| -> Result<()> {
        cancelled(progress)?;
        let frame = queue
            .finish_next_output()
            .map_err(|error| error.at_frame(completed as u64))?
            .ok_or_else(|| invalid("gpu.queue", "No ordered output available"))?;
        cancelled(progress)?;
        encoder
            .write((completed - range.start) as u64, &frame)
            .map_err(|error| error.at_frame(completed as u64))?;
        completed += 1;
        progress.frame_completed((completed - range.start) as u64, total);
        Ok(())
    };
    let mut resume = (workers == 0).then_some(range.start + 1);
    if workers > 0 {
        let mut slot_options = options.clone();
        slot_options.render.limits.max_bytes = preparation_bytes;
        super::parallel::ordered_until(
            range.start + 1..range.end,
            workers,
            progress,
            |_| {
                let slot_options = &slot_options;
                let mut renderer =
                    composition(timeline, &slot_options.render)?.frame_renderer(timeline.size())?;
                Ok(move |frame| {
                    let frame_index = frame;
                    match renderer
                        .prepare_output_at(
                            Time::from_frame(frame, rate),
                            slot_options.render.background,
                            |context| timeline.render(context),
                        )
                        .and_then(|(frame, mut limits)| {
                            limits.max_bytes = options.render.limits.max_bytes;
                            let plan = OutputPlan::with_planning_limit(
                                &frame,
                                limits,
                                preparation_bytes,
                                format,
                            )?
                            .with_display(&options.render.display, frame_index as u64)?;
                            let charge = plan.peak_bytes()?;
                            Ok((frame, plan, charge))
                        }) {
                        Ok(prepared) => Ok(Some(prepared)),
                        Err(error)
                            if preparation_bytes < options.render.limits.max_bytes
                                && error
                                    .downcast_ref::<motion_core::ValidationError>()
                                    .is_some_and(|error| {
                                        error.kind
                                            == motion_core::ValidationErrorKind::StructuralLimit
                                            && error.field == "max_bytes"
                                    }) =>
                        {
                            Ok(None)
                        }
                        Err(error) => Err(error),
                    }
                })
            },
            |index, prepared| {
                cancelled(progress)?;
                let Some((frame, plan, charge)) = prepared else {
                    resume = Some(index);
                    return Ok(false);
                };
                while queue.len() == slots
                    || queue.in_flight_bytes().saturating_add(charge) > queue_bytes
                {
                    if queue.is_empty() {
                        resume = Some(index);
                        return Ok(false);
                    }
                    encode_oldest(&mut queue)?;
                }
                queue
                    .submit_output(&frame, plan)
                    .map_err(|error| error.at_frame(index as u64))?;
                Ok(true)
            },
        )?;
    }
    while !queue.is_empty() {
        encode_oldest(&mut queue)?;
    }
    if let Some(resume) = resume {
        queue =
            motion_render::gpu::FrameQueue::new(context.clone(), options.render.limits.max_bytes)?;
        for index in resume..range.end {
            cancelled(progress)?;
            let (frame, limits) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                renderer.prepare_output_at(
                    Time::from_frame(index, rate),
                    options.render.background,
                    |context| timeline.render(context),
                )
            }))
            .unwrap_or_else(|payload| {
                Err(motion_core::Error::scene_panic(
                    "export",
                    index as u64,
                    payload,
                ))
            })
            .map_err(|error| error.at_frame(index as u64))?;
            let plan = OutputPlan::new(&frame, limits, format)
                .map_err(|error| motion_core::Error::from(error).at_frame(index as u64))?;
            queue
                .submit_output(&frame, plan)
                .map_err(|error| error.at_frame(index as u64))?;
            encode_oldest(&mut queue)?;
        }
    }
    cancelled(progress)
}
