//! Random-access PCM for timeline mixing and transactional output.
//!
//! ```
//! use motion_encode::prelude::*;
//! use motion_core::prelude::*;
//! use motion_core::{
//!     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
//!     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
//! };
//! use motion_render::{Composition, RenderPrepare, RenderOptions, Renderer, render_still};
//! let source = PcmBuffer::new(48_000, 1, vec![0.25; 4800])?;
//! let clip = AudioClip::source(source).start(50.milliseconds()).pan(-1.0);
//! let timeline = Timeline::new(Size::new(8.0, 4.0), 30).audio(clip).build()?;
//! let mixed = MixedAudio::from_timeline(&timeline, 48_000)?;
//! let mut pcm = [0.0; 2];
//! assert_eq!(mixed.read(2400, &mut pcm), 1);
//! assert_eq!(pcm, [0.25, 0.0]);
//! # Ok::<(), motion_core::Error>(())
//! ```

pub use motion_core::audio::{AudioClip, AudioSource, ceil_frames, frames};
use motion_core::{Duration, Time, ValidationError};
use std::sync::Arc;

mod meter;
mod mix;
mod resample;
#[cfg(test)]
mod tests;

pub use meter::BusMeter;
pub use mix::MixedAudio;
pub use resample::ResampleQuality;

pub(crate) const MAX_SAMPLES: usize = 32 * 1024 * 1024;

/// Creates a field-level audio diagnostic shared by preview and codec inputs.
pub(crate) fn invalid(field: &str, message: &str) -> ValidationError {
    ValidationError::invalid("audio", field, message)
}

#[cfg(feature = "audio")]
fn media_frames(value: u64, scale: u64, rate: u32) -> Result<usize, ValidationError> {
    usize::try_from(u128::from(value) * u128::from(rate) / u128::from(scale))
        .map_err(|_| invalid("edit_list", "Media interval exceeds address space"))
}

/// Immutable finite mono/stereo f32 PCM; clones share samples, never a file cursor.
#[derive(Clone, Debug)]
pub struct PcmBuffer {
    rate: u32,
    channels: u16,
    samples: Arc<[f32]>,
    #[cfg(feature = "audio")]
    pub(crate) analysis: super::analysis::Cache,
}

impl PcmBuffer {
    /// Decodes a bounded WAV/MP3/AAC/FLAC/Vorbis stream into immutable PCM.
    /// Corrupt packets, channel/rate changes and unsupported layouts fail explicitly.
    pub fn from_bytes(bytes: &[u8]) -> motion_core::Result<Self> {
        #[cfg(feature = "audio")]
        {
            use symphonia::core::{
                audio::SampleBuffer, codecs::DecoderOptions, formats::FormatOptions,
                io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
            };
            let error = |cause| motion_core::Error::asset("audio", "<memory>", cause);
            if bytes.len() > super::MAX_ENCODED_BYTES {
                return Err(invalid("encoded_bytes", "Audio source exceeds 64 MiB").into());
            }
            let stream = MediaSourceStream::new(
                Box::new(std::io::Cursor::new(bytes.to_vec())),
                Default::default(),
            );
            let mut format = symphonia::default::get_probe()
                .format(
                    &Hint::new(),
                    stream,
                    &FormatOptions {
                        enable_gapless: true,
                        ..Default::default()
                    },
                    &MetadataOptions::default(),
                )
                .map_err(error)?
                .format;
            let track = format
                .tracks()
                .iter()
                .find(|track| track.codec_params.sample_rate.is_some())
                .or_else(|| format.default_track())
                .ok_or_else(|| invalid("stream", "No audio track"))?;
            let id = track.id;
            let aac = track.codec_params.codec == symphonia::core::codecs::CODEC_TYPE_AAC;
            let mut decoder = symphonia::default::get_codecs()
                .make(&track.codec_params, &DecoderOptions::default())
                .map_err(error)?;
            let mut values = Vec::new();
            let mut specification = None;
            loop {
                let packet = match format.next_packet() {
                    Ok(packet) => packet,
                    Err(symphonia::core::errors::Error::IoError(error))
                        if error.kind() == std::io::ErrorKind::UnexpectedEof =>
                    {
                        break;
                    }
                    Err(cause) => return Err(error(cause)),
                };
                if packet.track_id() != id {
                    continue;
                }
                let decoded = decoder.decode(&packet).map_err(error)?;
                let spec = *decoded.spec();
                let channels = spec.channels.count();
                if channels == 0
                    || channels > 2
                    || !(1..=192_000).contains(&spec.rate)
                    || specification.is_some_and(|previous| previous != (spec.rate, channels))
                {
                    return Err(invalid(
                        "format",
                        "Require stable mono/stereo PCM at 1..=192000 Hz",
                    )
                    .into());
                }
                specification = Some((spec.rate, channels));
                if decoded.capacity().saturating_mul(channels) > MAX_SAMPLES
                    || decoded.frames().saturating_mul(channels)
                        > MAX_SAMPLES.saturating_sub(values.len())
                {
                    return Err(invalid("decoded_bytes", "Decoded PCM exceeds 128 MiB").into());
                }
                let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                buffer.copy_interleaved_ref(decoded);
                values.extend_from_slice(buffer.samples());
            }
            let (rate, channels) =
                specification.ok_or_else(|| invalid("stream", "Audio stream has no PCM frames"))?;
            if aac {
                let container =
                    mp4parse::read_mp4(&mut std::io::Cursor::new(bytes)).map_err(|error| {
                        motion_core::Error::asset("audio", "<mp4-edit-list>", error)
                    })?;
                let track = container
                    .tracks
                    .iter()
                    .find(|track| {
                        track.id == id as usize && track.track_type == mp4parse::TrackType::Audio
                    })
                    .ok_or_else(|| {
                        invalid("edit_list", "Selected AAC track has no matching MP4 timing")
                    })?;
                if track.looped.unwrap_or(false)
                    || track.empty_duration.is_some_and(|duration| duration.0 != 0)
                {
                    return Err(invalid(
                        "edit_list",
                        "Looped or leading-empty AAC edit lists are not supported",
                    )
                    .into());
                }
                let scale = track
                    .timescale
                    .filter(|scale| scale.0 != 0)
                    .ok_or_else(|| invalid("edit_list", "AAC track time scale is missing"))?
                    .0;
                let start = media_frames(track.media_time.map_or(0, |time| time.0), scale, rate)?;
                let length = if let (Some(duration), Some(scale)) = (
                    track.edited_duration,
                    container.timescale.filter(|scale| scale.0 != 0),
                ) {
                    media_frames(duration.0, scale.0, rate)?
                } else {
                    media_frames(track.duration.map_or(0, |time| time.0), scale, rate)?
                        .checked_sub(start)
                        .ok_or_else(|| {
                            invalid("edit_list", "AAC edit starts beyond media duration")
                        })?
                };
                if start > values.len() / channels || length > values.len() / channels - start {
                    return Err(
                        invalid("edit_list", "AAC edit interval exceeds decoded PCM").into(),
                    );
                }
                values.copy_within(start * channels..(start + length) * channels, 0);
                values.truncate(length * channels);
            }
            Ok(Self::new(rate, channels as u16, values)?)
        }
        #[cfg(not(feature = "audio"))]
        {
            let _ = bytes;
            Err(invalid("codec", "Audio decoding requires the audio feature").into())
        }
    }

    /// Explicit eager local-file decode; the returned snapshot survives file deletion.
    pub fn open(path: impl AsRef<std::path::Path>) -> motion_core::Result<Self> {
        use std::io::Read;
        let path = path.as_ref();
        let metadata = std::fs::metadata(path)?;
        if !metadata.is_file() || metadata.len() > super::MAX_ENCODED_BYTES as u64 {
            return Err(invalid("file", "Audio requires a regular file <=64 MiB").into());
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(super::MAX_ENCODED_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes)
            .map_err(|error| motion_core::Error::asset("audio", path.display().to_string(), error))
    }

    /// Registers finite interleaved PCM, at most 32 million samples / 128 MiB.
    pub fn new(rate: u32, channels: u16, samples: Vec<f32>) -> Result<Self, ValidationError> {
        if !(1..=192_000).contains(&rate)
            || !(1..=2).contains(&channels)
            || samples.is_empty()
            || samples.len() > MAX_SAMPLES
            || !samples.len().is_multiple_of(usize::from(channels))
            || samples.iter().any(|value| !value.is_finite())
        {
            return Err(invalid(
                "pcm",
                "Require 1..=192000 Hz, mono/stereo complete finite frames and 1..=33554432 samples",
            ));
        }
        Ok(Self {
            rate,
            channels,
            samples: samples.into(),
            #[cfg(feature = "audio")]
            analysis: Default::default(),
        })
    }

    /// Complete source frames retained in memory.
    pub fn frames(&self) -> u64 {
        (self.samples.len() / usize::from(self.channels)) as u64
    }

    /// Logical decoded payload, excluding allocation and decoder overhead.
    pub fn retained_bytes(&self) -> usize {
        self.samples.len() * std::mem::size_of::<f32>()
    }

    /// Immutable interleaved samples in native rate/channel order.
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }
}

impl motion_core::Asset for PcmBuffer {
    const KIND: &'static str = "audio";

    fn decode(bytes: &[u8], _: &motion_core::LoadContext) -> motion_core::Result<Self> {
        Self::from_bytes(bytes)
    }

    fn retained_bytes(&self) -> usize {
        self.retained_bytes()
    }
}

impl AudioSource for PcmBuffer {
    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn read(&self, start: u64, output: &mut [f32]) -> usize {
        output.fill(0.0);
        let channels = usize::from(self.channels);
        let frames = (self.frames().saturating_sub(start) as usize).min(output.len() / channels);
        if frames != 0 {
            let start = start as usize * channels;
            output[..frames * channels]
                .copy_from_slice(&self.samples[start..start + frames * channels]);
        }
        frames
    }

    fn duration(&self) -> Option<Duration> {
        Some(Time::from_ticks(
            ((u128::from(self.frames()) * motion_core::TICKS_PER_SECOND as u128)
                .div_ceil(u128::from(self.rate))) as i64,
        ))
    }
}
