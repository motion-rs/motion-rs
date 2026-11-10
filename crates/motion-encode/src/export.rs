//! Unified still/export entry points and pluggable ordered encoders.
//!
//! ```
//! use motion_encode::prelude::*;
//! use motion_core::prelude::*;
//! use motion_core::{
//!     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
//!     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
//! };
//! use motion_render::{Composition, RenderPrepare, RenderOptions, Renderer, render_still};
//! use motion_core::Scene as _;
//! let timeline = Timeline::new(Size::new(4.0, 2.0), 30).track(|track| {
//!     track.push((|_: &SceneContext<'_>| Node::empty()).with_duration(1.seconds()))
//! });
//! let bitmap = render_still(&timeline, 250.milliseconds(), &RenderOptions::default())?;
//! assert_eq!(bitmap.bytes(), &[0; 32]);
//! # Ok::<(), motion_core::Error>(())
//! ```

use crate::encoder::ffmpeg::FfmpegEncoder;

pub use motion_render::{RenderOptions, composition, render_still};

use motion_core::{Result, Timeline};

use motion_core::Scene;
use motion_render::FrameBuffer;

#[cfg(feature = "gpu")]
mod gpu;
mod options;
mod parallel;
mod render;
#[cfg(test)]
mod tests;

pub use options::{
    BitDepth, Codec, Encoder, EncoderCapabilities, EncoderPreset, ExportOptions, ExportPhase,
    ExportReport, NoProgress, ProgressSink,
};
use render::{output_rate, render_frame, render_workers, selected};

pub(crate) fn invalid(field: &str, message: &str) -> motion_core::Error {
    motion_core::ValidationError::invalid("export", field, message).into()
}

/// Exports using the selected native video codec, PNG sequence or GIF encoder.
/// Native video encoding runs in-process without launching a subprocess.
pub fn export(
    timeline: &Timeline,
    options: &ExportOptions,
    progress: &dyn ProgressSink,
) -> Result<ExportReport> {
    let encoder: Box<dyn Encoder + '_> = match options.codec {
        Codec::PngSequence => Box::new(crate::encoder::png::PngSequenceEncoder::new()),
        Codec::Gif { .. } => Box::new(crate::encoder::gif::GifEncoder::new()),
        _ => Box::new(
            FfmpegEncoder::new(progress)
                .codec(options.codec)
                .bit_depth(options.bit_depth),
        ),
    };
    export_with_encoder(timeline, options, progress, encoder)
}

/// Exports through a caller-owned encoder using the same clock, resources and limits.
pub fn export_with_encoder(
    timeline: &Timeline,
    options: &ExportOptions,
    progress: &dyn ProgressSink,
    mut encoder: Box<dyn Encoder + '_>,
) -> Result<ExportReport> {
    cancelled(progress)?;
    progress.phase(ExportPhase::Preparing);
    cancelled(progress)?;
    let prepared_timeline = crate::audio::prepare_timeline(timeline)?;
    let timeline = &prepared_timeline;
    let (mut options, first, end) = selected(timeline, options)?;
    options.render.font_usage = motion_core::FontUsage::default();
    let output_rate = output_rate(timeline, &options);
    if !options.overwrite && options.path.try_exists()? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "Export destination exists",
        )
        .into());
    }
    let capabilities = encoder.capabilities();
    capabilities.validate_size(options.render.size.expect("normalized size"))?;
    let mut renderer = composition(timeline, &options.render)?.frame_renderer(timeline.size())?;
    let clips = timeline.audio_clips()?;
    if options.path.try_exists()? {
        let destination = options.path.canonicalize()?;
        for clip in &clips {
            if let Some(path) = clip.parameters().source.file_path()
                && path.canonicalize()? == destination
            {
                return Err(invalid(
                    "path",
                    "Export destination must not replace an audio source",
                ));
            }
        }
    }
    capabilities.validate_audio(!clips.is_empty())?;
    let audio = if clips.is_empty() {
        None
    } else {
        let range = options.range.as_ref().expect("normalized range");
        Some(crate::MixedAudio::from_window_with(
            timeline,
            range.clone(),
            options.audio_sample_rate,
            options.audio_resampler,
        )?)
    };
    cancelled(progress)?;
    encoder.open(&options, output_rate, audio.as_ref())?;
    cancelled(progress)?;
    progress.phase(ExportPhase::Rendering);
    let format = encoder.input_format();
    #[cfg(feature = "gpu")]
    if gpu::eligible(&options, format, end - first, &mut renderer)? {
        gpu::render(
            timeline,
            &options,
            first..end,
            output_rate,
            progress,
            &mut *encoder,
            &mut renderer,
        )?;
        cancelled(progress)?;
        progress.phase(ExportPhase::Finalizing);
        cancelled(progress)?;
        let mut report = encoder.finish()?;
        report.fonts = options.render.font_usage.fonts();
        progress.phase(ExportPhase::Completed);
        return Ok(report);
    }
    cancelled(progress)?;
    let first_pixels = if !matches!(
        options.render.backend,
        motion_render::BackendPreference::Gpu { .. }
    ) && options.workers > 1
    {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            render_frame(
                &mut renderer,
                timeline,
                &options,
                output_rate,
                format,
                first,
            )
        }))
        .unwrap_or_else(|payload| {
            Err(motion_core::Error::scene_panic(
                "export",
                first as u64,
                payload,
            ))
        })?
    } else {
        render_frame(
            &mut renderer,
            timeline,
            &options,
            output_rate,
            format,
            first,
        )?
    };
    let observed_peak = renderer
        .last_stats()
        .map_or(options.render.limits.max_bytes, |stats| stats.peak_bytes());
    let workers = render_workers(
        &options,
        end - first - 1,
        observed_peak,
        renderer.backend_decision(),
    );
    let mut write = |frame: i64, pixels: FrameBuffer| -> Result<()> {
        cancelled(progress)?;
        let index = (frame - first) as u64;
        encoder
            .write(index, &pixels)
            .map_err(|error| error.at_frame(frame as u64))?;
        progress.frame_completed(index + 1, (end - first) as u64);
        Ok(())
    };
    write(first, first_pixels)?;
    if workers == 1 {
        for frame in first + 1..end {
            cancelled(progress)?;
            let pixels = render_frame(
                &mut renderer,
                timeline,
                &options,
                output_rate,
                format,
                frame,
            )?;
            write(frame, pixels)?;
        }
    } else {
        let mut slot_options = options.clone();
        slot_options.render.backend = motion_render::BackendPreference::Cpu;
        slot_options.render.limits.max_bytes = options
            .render
            .limits
            .max_bytes
            .min(options.max_parallel_bytes / workers as u64);
        let mut resume = None;
        parallel::ordered_until(
            first + 1..end,
            workers,
            progress,
            |_| {
                let slot_options = &slot_options;
                let mut renderer =
                    composition(timeline, &slot_options.render)?.frame_renderer(timeline.size())?;
                Ok(move |frame| {
                    match render_frame(
                        &mut renderer,
                        timeline,
                        slot_options,
                        output_rate,
                        format,
                        frame,
                    ) {
                        Ok(pixels) => Ok(Some(pixels)),
                        Err(error)
                            if slot_options.render.limits.max_bytes
                                < options.render.limits.max_bytes
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
            |frame, pixels| {
                let Some(pixels) = pixels else {
                    resume = Some(frame);
                    return Ok(false);
                };
                write(frame, pixels)?;
                Ok(true)
            },
        )?;
        if let Some(resume) = resume {
            for frame in resume..end {
                cancelled(progress)?;
                let pixels = render_frame(
                    &mut renderer,
                    timeline,
                    &options,
                    output_rate,
                    format,
                    frame,
                )?;
                write(frame, pixels)?;
            }
        }
    }
    cancelled(progress)?;
    progress.phase(ExportPhase::Finalizing);
    cancelled(progress)?;
    let mut report = encoder.finish()?;
    report.fonts = options.render.font_usage.fonts();
    progress.phase(ExportPhase::Completed);
    Ok(report)
}

pub(crate) fn cancelled(progress: &dyn ProgressSink) -> Result<()> {
    if progress.is_cancelled() {
        Err(motion_core::Error::Cancelled)
    } else {
        Ok(())
    }
}
