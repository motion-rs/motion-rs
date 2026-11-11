//! Unified audio sources, bounded mixing and deferred file decoding.

#[cfg(any(feature = "ffmpeg", test))]
use std::io;

#[cfg(feature = "audio")]
pub mod analysis;
mod ducking;
pub mod effects;
pub mod export;
pub mod master;
pub mod pcm;
mod sinc;
pub mod sources;
pub mod synth;
pub mod waveform;

pub use master::LoudnessReport;
pub use motion_core::{
    AudioBus, AudioEffect, Duck, DuckTrigger, Ducking, Limiter, Loudness, Master, PanLaw,
};
pub use sources::{AudioClipExt, LoopSource, ReverseSource, SpeedSource};
pub use waveform::WaveformPyramid;

#[cfg(not(feature = "audio"))]
mod resample_disabled;

#[cfg(feature = "audio")]
pub use analysis::{AudioAnalysis, AudioMarkers, AudioSignal, AudioSpectrum, BeatGrid};
pub use export::{AudioChannels, AudioExportOptions, AudioExportReport, AudioFormat, export_audio};
pub use motion_core::{AudioChapter, AudioMetadata};
pub use pcm::{
    AudioClip, AudioSource, MixOptions, MixSource, MixedAudio, PcmBuffer, ResampleQuality,
};

#[cfg(feature = "audio")]
mod file_source;

/// Sample rate of the streamed file mixer and AAC export boundary.
pub const SAMPLE_RATE: u64 = 48_000;

/// Most clips one prepared mix, file export or Studio preview list admits.
pub const MAX_CLIPS: usize = 256;

/// Maximum declared file/preview/export interval in seconds.
pub const MAX_SECONDS: u64 = 86_400;

/// Largest encoded audio source, in bytes, decoded from memory or a local file.
pub(crate) const MAX_ENCODED_BYTES: usize = 64 * 1024 * 1024;

thread_local! {
    static COMPOSITION_DEPTH:std::cell::Cell<usize>=const {std::cell::Cell::new(0)};
}

/// Resolves file metadata at an execution boundary without retaining decoded PCM.
pub fn resolve_clip(clip: &AudioClip) -> motion_core::Result<AudioClip> {
    if let Some(timeline) = clip.parameters().source.composition() {
        let _guard = CompositionGuard::enter()?;
        return Ok(
            clip.with_resolved_source(std::sync::Arc::new(MixedAudio::new(
                timeline,
                MixOptions::new(clip.parameters().source.sample_rate()),
            )?)),
        );
    }
    #[cfg(feature = "audio")]
    return file_source::resolve(clip);
    #[cfg(not(feature = "audio"))]
    {
        if clip.unresolved_file() {
            return Err(motion_core::Error::asset(
                "audio",
                "<file>",
                std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "File audio requires feature audio",
                ),
            ));
        }
        Ok(clip.clone())
    }
}

/// Resolves the timeline's flattened audio before output scheduling and encoder startup.
pub(crate) fn prepare_timeline(
    timeline: &motion_core::Timeline,
) -> motion_core::Result<motion_core::ResolvedAudio> {
    timeline.resolve_audio(resolve_clip)
}

/// The timeline itself, or a copy lasting as long as its resolved audio when that is longer.
/// Only timelines with an inferred duration are ever extended. Resolving the audio files is
/// part of the call; a failing file is an error.
pub fn fit_timeline(
    timeline: &motion_core::Timeline,
) -> motion_core::Result<std::borrow::Cow<'_, motion_core::Timeline>> {
    fitted(timeline, &prepare_timeline(timeline)?)
}

pub(crate) fn fitted<'a>(
    timeline: &'a motion_core::Timeline,
    resolved: &motion_core::ResolvedAudio,
) -> motion_core::Result<std::borrow::Cow<'a, motion_core::Timeline>> {
    if resolved.duration() <= timeline.duration() {
        return Ok(std::borrow::Cow::Borrowed(timeline));
    }
    Ok(std::borrow::Cow::Owned(
        timeline
            .to_builder()
            .try_duration(resolved.duration())?
            .build()?,
    ))
}

struct CompositionGuard;

impl CompositionGuard {
    fn enter() -> motion_core::Result<Self> {
        COMPOSITION_DEPTH.with(|depth| {
            if depth.get() >= 16 {
                return Err(motion_core::ValidationError::invalid(
                    "audio",
                    "composition_depth",
                    "At most 16 nested audio preparations",
                )
                .into());
            }
            depth.set(depth.get() + 1);
            Ok(Self)
        })
    }
}

impl Drop for CompositionGuard {
    fn drop(&mut self) {
        COMPOSITION_DEPTH.with(|depth| depth.set(depth.get() - 1));
    }
}

#[cfg(any(feature = "ffmpeg", test))]
pub(crate) fn video_samples(frames: u32, rate: motion_core::FrameRate) -> io::Result<u64> {
    let samples = (u128::from(frames) * u128::from(SAMPLE_RATE) * u128::from(rate.denominator()))
        .div_ceil(u128::from(rate.numerator()));
    u64::try_from(samples).map_err(|_| invalid("Video duration exceeds the audio sample range"))
}

#[cfg(any(feature = "ffmpeg", test))]
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractional_video_audio_uses_exact_sample_ceiling() -> io::Result<()> {
        for (rate, frames, expected) in [
            (motion_core::FrameRate::NTSC, 1, 1602),
            (motion_core::FrameRate::NTSC, 3, 4805),
            (motion_core::FrameRate::NTSC, 30_000, 48_048_000),
            (motion_core::FrameRate::NTSC_DOUBLE, 1, 801),
            (motion_core::fps!(11), 1, 4364),
        ] {
            assert_eq!(video_samples(frames, rate)?, expected);
        }
        assert!(
            video_samples(
                u32::MAX,
                motion_core::FrameRate::try_new(1, u32::MAX).unwrap()
            )
            .is_err()
        );
        Ok(())
    }
}
