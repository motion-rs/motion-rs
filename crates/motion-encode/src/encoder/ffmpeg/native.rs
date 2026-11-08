//! Native FFmpeg export with transactional output and deterministic frame submission.

use std::io;
#[cfg(any(feature = "ffmpeg", test))]
use std::path::Path;
#[cfg(feature = "ffmpeg")]
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(feature = "ffmpeg")]
use tempfile::NamedTempFile;

use crate::audio;
#[cfg(any(feature = "ffmpeg", test))]
use motion_core::Result;

#[cfg(test)]
pub(crate) mod tests;

/// Native FFmpeg video export settings. Enable `ffmpeg` and install FFmpeg development libraries.
///
/// Configures the [`crate::Codec`] and its container (H.264/HEVC MP4, VP9 WebM, ProRes 4444 MOV),
/// 8- or 10-bit precision (HEVC Main 10), CRF and preset, x265 parameters, 2.4 gamma tagging,
/// container tags/chapters and overwrite. [`Self::audio_mix`] adds stereo PCM encoded as 192 kb/s
/// AAC at its 44.1, 48 or 96 kHz rate. Defaults are silent 8-bit H.264, CRF 18, `medium` preset
/// and no overwrite. Export uses native APIs, never an executable or fallback.
#[derive(Clone, Debug)]
pub(crate) struct NativeOptions {
    #[cfg(any(feature = "ffmpeg", test))]
    codec: crate::Codec,
    #[cfg(feature = "ffmpeg")]
    bit_depth: crate::BitDepth,
    #[cfg(any(feature = "ffmpeg", test))]
    crf: u8,
    #[cfg(feature = "ffmpeg")]
    preset: crate::EncoderPreset,
    #[cfg(feature = "ffmpeg")]
    x265_params: Option<String>,
    #[cfg(feature = "ffmpeg")]
    gamma24: bool,

    #[cfg(any(feature = "ffmpeg", test))]
    overwrite: bool,
    audio_mix: Option<Arc<crate::MixedAudio>>,
    #[cfg(feature = "ffmpeg")]
    metadata: motion_core::audio::AudioMetadata,
}

impl Default for NativeOptions {
    fn default() -> Self {
        Self {
            #[cfg(any(feature = "ffmpeg", test))]
            codec: crate::Codec::default(),
            #[cfg(feature = "ffmpeg")]
            bit_depth: crate::BitDepth::Eight,
            #[cfg(any(feature = "ffmpeg", test))]
            crf: 18,
            #[cfg(feature = "ffmpeg")]
            preset: crate::EncoderPreset::Medium,
            #[cfg(feature = "ffmpeg")]
            x265_params: None,
            #[cfg(feature = "ffmpeg")]
            gamma24: false,
            #[cfg(any(feature = "ffmpeg", test))]
            overwrite: false,
            audio_mix: None,
            #[cfg(feature = "ffmpeg")]
            metadata: Default::default(),
        }
    }
}

impl NativeOptions {
    #[cfg(feature = "ffmpeg")]
    pub(crate) fn encoding(mut self, options: &crate::ExportOptions) -> Self {
        self.bit_depth = options.bit_depth;
        self.preset = match options.codec {
            crate::Codec::H264 { preset, .. } => preset,
            _ => options.encoder_preset,
        };
        self.gamma24 = options.render.display.transfer == motion_render::OutputTransfer::Gamma24;
        self.x265_params = options.x265_params.clone();
        self.metadata = options.audio_metadata.clone();
        if let Some(range) = &options.range {
            self.metadata.chapters = self
                .metadata
                .chapters
                .into_iter()
                .filter_map(|mut chapter| {
                    chapter.start = chapter.start.max(range.start);
                    chapter.end = chapter.end.min(range.end);
                    if chapter.start >= chapter.end {
                        return None;
                    }
                    chapter.start -= range.start;
                    chapter.end -= range.start;
                    Some(chapter)
                })
                .collect();
        }
        self
    }

    #[cfg(feature = "ffmpeg")]
    pub(crate) fn codec(mut self, codec: crate::Codec) -> Self {
        self.codec = codec;
        self
    }

    /// Sets H.264 quality, from 0 (lossless) to 51 (lowest); default is 18.
    /// Values above 51 are rejected when export starts, not clamped by this setter.
    #[cfg(any(feature = "ffmpeg", test))]
    pub fn crf(mut self, crf: u8) -> Self {
        self.crf = crf;
        self
    }

    /// Allows replacing the destination only after a successful encode.
    #[cfg(feature = "ffmpeg")]
    pub fn overwrite(mut self, overwrite: bool) -> Self {
        self.overwrite = overwrite;
        self
    }

    /// Adds immutable prepared stereo PCM at a native AAC rate.
    #[cfg(any(feature = "ffmpeg", test))]
    pub fn audio_mix(mut self, audio: crate::MixedAudio) -> Result<Self> {
        if ![44100, 48000, 96000].contains(&motion_core::AudioSource::sample_rate(&audio)) {
            return Err(crate::audio::pcm::invalid(
                "export",
                "export requires 44.1, 48 or 96 kHz stereo",
            )
            .into());
        }
        self.audio_mix = Some(Arc::new(audio));
        Ok(self)
    }
}

#[cfg(feature = "ffmpeg")]
pub(crate) fn video_codec(
    codec: crate::Codec,
    bit_depth: crate::BitDepth,
) -> Result<motion_ffmpeg::VideoCodec> {
    Ok(match codec {
        crate::Codec::H264 { .. } => motion_ffmpeg::VideoCodec::H264,
        crate::Codec::H265 { .. } => motion_ffmpeg::VideoCodec::H265 {
            profile: if bit_depth == crate::BitDepth::Ten {
                motion_ffmpeg::H265Profile::Main10
            } else {
                motion_ffmpeg::H265Profile::Main
            },
        },
        crate::Codec::Vp9 { alpha, .. } => motion_ffmpeg::VideoCodec::Vp9 { alpha },
        crate::Codec::ProRes4444 => motion_ffmpeg::VideoCodec::ProRes4444,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Codec is not a native video format",
            )
            .into());
        }
    })
}

#[cfg(any(feature = "ffmpeg", test))]
/// Without `ffmpeg`, opening always fails, so test builds never construct a value.
pub(crate) struct Encoder {
    #[cfg(feature = "ffmpeg")]
    native: Option<motion_ffmpeg::Encoder>,
    #[cfg(feature = "ffmpeg")]
    temporary: Option<NamedTempFile>,
    #[cfg(feature = "ffmpeg")]
    destination: PathBuf,
    #[cfg(feature = "ffmpeg")]
    overwrite: bool,
}

#[cfg(any(feature = "ffmpeg", test))]
impl Encoder {
    #[cfg(test)]
    pub(crate) fn new(
        destination: &Path,
        size: (usize, usize),
        fps: u32,
        frames: u32,
        options: &NativeOptions,
    ) -> Result<Self> {
        validate(size, fps, frames, options)?;
        Self::open(
            destination,
            size,
            motion_core::FrameRate::from(fps),
            frames,
            options,
        )
    }

    #[cfg(feature = "ffmpeg")]
    pub(crate) fn new_with_frame_rate(
        destination: &Path,
        size: (usize, usize),
        rate: motion_core::FrameRate,
        frames: u32,
        options: &NativeOptions,
    ) -> Result<Self> {
        validate_at_rate(size, rate, frames, options)?;
        Self::open(destination, size, rate, frames, options)
    }

    fn open(
        destination: &Path,
        size: (usize, usize),
        rate: motion_core::FrameRate,
        frames: u32,
        options: &NativeOptions,
    ) -> Result<Self> {
        #[cfg(not(feature = "ffmpeg"))]
        let _ = (size, rate, frames);
        let destination = std::path::absolute(destination)?;
        if !options.overwrite && destination.try_exists()? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Video destination already exists; enable overwrite explicitly",
            )
            .into());
        }
        #[cfg(not(feature = "ffmpeg"))]
        return Err(motion_core::Error::Encoder(Box::new(unavailable())));

        #[cfg(feature = "ffmpeg")]
        {
            let mut builder = tempfile::Builder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                builder.permissions(std::fs::Permissions::from_mode(0o666));
            }
            let temporary = builder
                .prefix(".motion-video-")
                .suffix(match options.codec {
                    crate::Codec::Vp9 { .. } => ".webm",
                    crate::Codec::ProRes4444 => ".mov",
                    _ => ".mp4",
                })
                .tempfile_in(
                    destination
                        .parent()
                        .ok_or_else(|| io::Error::other("Invalid video destination"))?,
                )?;
            let mix = options
                .audio_mix
                .as_ref()
                .map(|audio| motion_ffmpeg::AudioPcm {
                    sample_rate: motion_core::AudioSource::sample_rate(audio.as_ref()),
                    source: Box::new(PcmInput(audio.clone())),
                });
            let native = motion_ffmpeg::Encoder::new_with_pcm_settings(
                temporary.path(),
                size,
                (rate.numerator(), rate.denominator()),
                frames,
                motion_ffmpeg::VideoSettings {
                    codec: video_codec(options.codec, options.bit_depth)?,
                    quality: options.crf,
                    preset: match options.preset {
                        crate::EncoderPreset::Medium => "medium",
                        crate::EncoderPreset::Slow => "slow",
                        crate::EncoderPreset::Slower => "slower",
                    },
                    gamma24: options.gamma24,
                    x265_params: options.x265_params.clone(),
                    metadata: motion_ffmpeg::MediaMetadata {
                        tags: options
                            .metadata
                            .tags
                            .iter()
                            .map(|(key, value)| (key.clone(), value.clone()))
                            .collect(),
                        chapters: options
                            .metadata
                            .chapters
                            .iter()
                            .map(|chapter| motion_ffmpeg::MediaChapter {
                                start: chapter.start.ticks(),
                                end: chapter.end.ticks(),
                                title: chapter.title.clone(),
                            })
                            .collect(),
                    },
                },
                mix,
            )
            .map_err(|error| motion_core::Error::Encoder(Box::new(error)))?;
            Ok(Self {
                native: Some(native),
                temporary: Some(temporary),
                destination,
                overwrite: options.overwrite,
            })
        }
    }

    #[cfg(feature = "ffmpeg")]
    pub(crate) fn write_buffer(&mut self, frame: &motion_render::FrameBuffer) -> Result {
        let planes = frame.planes();
        let native = self.native.as_mut().ok_or_else(|| {
            motion_core::Error::Encoder(Box::new(io::Error::other("Encoder is closed")))
        })?;
        match frame.format() {
            motion_render::PixelFormat::Yuv420 { bit_depth: 10 } => {
                native.write_yuv420_10(planes[0].bytes(), planes[1].bytes(), planes[2].bytes())
            }
            motion_render::PixelFormat::Yuv420 { bit_depth: 8 } => {
                native.write_yuv420(planes[0].bytes(), planes[1].bytes(), planes[2].bytes())
            }
            motion_render::PixelFormat::SrgbRgba8 {
                premultiplied: false,
            } => native.write_rgba(planes[0].bytes()),
            _ => {
                return Err(crate::audio::pcm::invalid(
                    "format",
                    "Native video requires YUV420 or straight RGBA",
                )
                .into());
            }
        }
        .map_err(|error| motion_core::Error::Encoder(Box::new(error)))
    }

    #[cfg(all(test, feature = "ffmpeg"))]
    pub(crate) fn finish(self) -> Result {
        self.finish_with_check(|| false).map(|_| ())
    }

    #[cfg(feature = "ffmpeg")]
    pub(crate) fn finish_with_check(mut self, cancelled: impl Fn() -> bool) -> Result<bool> {
        if cancelled() {
            return Ok(false);
        }
        let result = self
            .native
            .take()
            .ok_or_else(|| {
                motion_core::Error::Encoder(Box::new(io::Error::other("Encoder is closed")))
            })?
            .finish(|| {
                if cancelled() {
                    Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "Export cancelled",
                    ))
                } else {
                    Ok(())
                }
            });
        if cancelled() {
            return Ok(false);
        }
        result.map_err(|error| motion_core::Error::Encoder(Box::new(error)))?;
        // Commit is the sole destination mutation: every earlier exit leaves the old file intact.
        let temporary = self
            .temporary
            .take()
            .ok_or_else(|| io::Error::other("Video output is unavailable"))?;
        if self.overwrite {
            temporary
                .persist(&self.destination)
                .map_err(|error| error.error)?;
        } else {
            temporary
                .persist_noclobber(&self.destination)
                .map_err(|error| error.error)?;
        }
        Ok(true)
    }
}

#[cfg(feature = "ffmpeg")]
struct PcmInput(Arc<crate::MixedAudio>);

#[cfg(feature = "ffmpeg")]
impl motion_ffmpeg::PcmSource for PcmInput {
    fn read(&mut self, start: u64, output: &mut [f32]) -> io::Result<usize> {
        Ok(motion_core::AudioSource::read(
            self.0.as_ref(),
            start,
            output,
        ))
    }
}

#[cfg(any(feature = "ffmpeg", test))]
impl Drop for Encoder {
    fn drop(&mut self) {
        #[cfg(feature = "ffmpeg")]
        self.native.take();
    }
}

#[cfg(not(feature = "ffmpeg"))]
#[cfg(any(feature = "ffmpeg", test))]
fn unavailable() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "video export requires the ffmpeg Cargo feature and FFmpeg development libraries",
    )
}

#[cfg(test)]
fn validate(
    size: (usize, usize),
    fps: u32,
    frames: u32,
    options: &NativeOptions,
) -> io::Result<()> {
    if fps == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "MP4 export requires nonzero FPS",
        ));
    }
    validate_at_rate(size, motion_core::FrameRate::from(fps), frames, options)
}

#[cfg(any(feature = "ffmpeg", test))]
fn validate_at_rate(
    size: (usize, usize),
    rate: motion_core::FrameRate,
    frames: u32,
    options: &NativeOptions,
) -> io::Result<()> {
    if size.0 == 0
        || size.1 == 0
        || !size.0.is_multiple_of(2)
        || !size.1.is_multiple_of(2)
        || size.0 > i32::MAX as usize / 3
        || size.1 > i32::MAX as usize
        || size
            .0
            .checked_mul(size.1)
            .and_then(|pixels| pixels.checked_mul(3))
            .is_none()
        || rate.numerator() > i32::MAX as u32
        || rate.denominator() > i32::MAX as u32
        || frames == 0
        || options.crf
            > if matches!(options.codec, crate::Codec::Vp9 { .. }) {
                63
            } else {
                51
            }
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "MP4 export requires positive even dimensions, nonzero FPS/frame count, and CRF between 0 and 51",
        ));
    }
    let samples = audio::video_samples(frames, rate)?;
    if options.audio_mix.is_some() && samples > audio::MAX_SECONDS * audio::SAMPLE_RATE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Audio export requires a video duration no longer than 24 hours",
        ));
    }
    Ok(())
}
