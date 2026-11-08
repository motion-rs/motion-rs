//! Export options, codec selection, progress reporting and the encoder contract.

use super::{RenderOptions, invalid};
use motion_core::{Color, FrameRate, Result, Time};
use motion_render::{FrameBuffer, PixelFormat};
use std::{ops::Range, path::PathBuf};

/// Native encoder speed/compression preset passed to libx264 and libx265.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EncoderPreset {
    /// Maps to the `medium` preset, balancing encode speed and compression.
    #[default]
    Medium,
    /// Maps to the `slow` preset, spending more compression work for offline delivery.
    Slow,
    /// Maps to the `slower` preset, the highest supported offline effort.
    Slower,
}

/// Explicit video sample precision; ten-bit delivery is HEVC-only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BitDepth {
    /// Eight-bit samples, supported by every codec.
    #[default]
    Eight,
    /// Ten-bit samples in little-endian u16 storage.
    Ten,
}

impl BitDepth {
    /// Sample precision in bits.
    pub fn bits(self) -> u8 {
        match self {
            Self::Eight => 8,
            Self::Ten => 10,
        }
    }
}

/// Output format of an export, selecting the codec, container and quality parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    /// H.264/MP4 limited-range BT.709 without alpha.
    H264 {
        /// Constant-rate factor in 0..=51.
        quality: u8,
        /// Encoder preset passed to libx264.
        preset: EncoderPreset,
    },
    /// H.265/HEVC, with CRF in 0..=51.
    H265 {
        /// Constant-rate factor in 0..=51.
        quality: u8,
    },
    /// VP9/WebM, optionally retaining alpha, with quality in 0..=63.
    Vp9 {
        /// Constant-quality factor in 0..=63.
        quality: u8,
        /// Encodes the alpha plane instead of flattening transparency.
        alpha: bool,
    },
    /// ProRes 4444 with alpha in a MOV container.
    ProRes4444,
    /// Transactional directory of straight-alpha PNG frames.
    PngSequence,
    /// Palette GIF at the declared frame rate, using 2..=256 colors.
    Gif {
        /// Output sampling grid, independent of the timeline's frame grid.
        frame_rate: FrameRate,
        /// Maximum per-frame palette colors, including a transparent entry.
        colors: u16,
    },
}

impl Codec {
    /// Checks codec parameters before any output is opened.
    pub fn validate(self) -> Result<()> {
        match self {
            Self::H264 { quality, .. } | Self::H265 { quality } if quality > 51 => {
                Err(invalid("quality", "H264/H265 quality must be in 0..=51"))
            }
            Self::Vp9 { quality, .. } if quality > 63 => {
                Err(invalid("quality", "VP9 quality must be in 0..=63"))
            }
            Self::Gif { colors, .. } if !(2..=256).contains(&colors) => {
                Err(invalid("colors", "GIF needs 2..=256 colors"))
            }
            Self::Gif { frame_rate, .. }
                if u128::from(frame_rate.numerator())
                    > 100 * u128::from(frame_rate.denominator())
                    || (100 * u128::from(frame_rate.denominator()))
                        .div_ceil(u128::from(frame_rate.numerator()))
                        > u128::from(u16::MAX) =>
            {
                Err(invalid(
                    "frame_rate",
                    "GIF frame delays must fit 1..=65535 centiseconds",
                ))
            }
            _ => Ok(()),
        }
    }
}

impl Default for Codec {
    fn default() -> Self {
        Self::H264 {
            quality: 18,
            preset: EncoderPreset::Medium,
        }
    }
}

/// Export request shared by programmatic, Project CLI and custom encoders.
#[derive(Clone)]
pub struct ExportOptions {
    /// Destination committed only by a successful encoder finish.
    pub path: PathBuf,
    /// Rendering/resource settings. Opaque black is the default export background.
    pub render: RenderOptions,
    /// Selects absolute frame timestamps in [start,end); None exports the timeline.
    pub range: Option<Range<Time>>,
    /// Current native codec capability.
    pub codec: Codec,
    /// Ten-bit delivery requires H.265.
    pub bit_depth: BitDepth,
    /// HEVC preset; H.264 uses the preset in its codec variant.
    pub encoder_preset: EncoderPreset,
    /// Explicit native x265 parameter string, such as `aq-mode=3`.
    pub x265_params: Option<String>,
    /// Audio delivery rate; supported AAC values are 44100, 48000 and 96000 Hz.
    pub audio_sample_rate: u32,
    /// Resampler used when audio sources differ from `audio_sample_rate`.
    pub audio_resampler: crate::audio::ResampleQuality,
    /// Optional title/language/loudness tags and absolute timeline chapters.
    pub audio_metadata: motion_core::audio::AudioMetadata,
    /// Replaces destination only at successful commit.
    pub overwrite: bool,
    /// Requested render workers, in 1..=64; resolved CPU sessions may run in parallel.
    pub workers: usize,
    /// Logical allowance for parallel slots, scheduled from the first frame's peak.
    /// CPU slots divide this allowance; larger later frames resume serially.
    /// Excludes encoder storage, shared assets and session caches; not a process cap.
    pub max_parallel_bytes: u64,
}

impl ExportOptions {
    /// Creates a native H.264 request with an opaque black background.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            render: RenderOptions {
                background: Some(Color::BLACK),
                ..RenderOptions::default()
            },
            range: None,
            codec: Codec::default(),
            bit_depth: BitDepth::Eight,
            encoder_preset: EncoderPreset::Medium,
            x265_params: None,
            audio_sample_rate: 48000,
            audio_resampler: Default::default(),
            audio_metadata: Default::default(),
            overwrite: false,
            workers: std::thread::available_parallelism()
                .map_or(1, usize::from)
                .min(64),
            max_parallel_bytes: 4 * 1024 * 1024 * 1024,
        }
    }

    /// Selects the encoder and container.
    pub fn codec(mut self, codec: Codec) -> Self {
        self.codec = codec;
        if matches!(
            codec,
            Codec::PngSequence
                | Codec::ProRes4444
                | Codec::Vp9 { alpha: true, .. }
                | Codec::Gif { .. }
        ) {
            self.render.background = None;
        }
        self
    }

    /// Selects HEVC precision; unsupported combinations fail before opening output.
    pub fn bit_depth(mut self, depth: BitDepth) -> Self {
        self.bit_depth = depth;
        self
    }

    /// Selects the HEVC compression preset.
    pub fn encoder_preset(mut self, preset: EncoderPreset) -> Self {
        self.encoder_preset = preset;
        self
    }

    /// Supplies an explicit bounded x265 parameter string.
    pub fn x265_params(mut self, parameters: impl Into<String>) -> Self {
        self.x265_params = Some(parameters.into());
        self
    }

    /// Selects 44100, 48000 (default) or 96000 Hz for native AAC delivery.
    pub fn audio_sample_rate(mut self, rate: u32) -> Self {
        self.audio_sample_rate = rate;
        self
    }

    /// Selects the sample-rate converter for mixed audio sources.
    pub fn audio_resampler(mut self, resampler: crate::audio::ResampleQuality) -> Self {
        self.audio_resampler = resampler;
        self
    }

    /// Sets bounded native container tags and tick-authored chapters.
    pub fn audio_metadata(mut self, metadata: motion_core::AudioMetadata) -> Self {
        self.audio_metadata = metadata;
        self
    }

    /// Adjusts the shared render options, for example `.render(|r| r.quality(RenderQuality::Draft))`.
    pub fn render(mut self, configure: impl FnOnce(RenderOptions) -> RenderOptions) -> Self {
        self.render = configure(self.render);
        self
    }

    /// Sets explicit output dimensions.
    pub fn size(mut self, size: motion_core::PixelSize) -> Self {
        self.render.size = Some(size);
        self
    }

    /// Selects a half-open timeline interval.
    pub fn range(mut self, range: Range<Time>) -> Self {
        self.range = Some(range);
        self
    }

    /// Selects the render backend.
    pub fn backend(mut self, backend: motion_render::BackendPreference) -> Self {
        self.render.backend = backend;
        self
    }

    /// Adds whole-frame shutter sampling.
    pub fn motion_blur(mut self, blur: motion_core::MotionBlur) -> Self {
        self.render.motion_blur = Some(blur);
        self
    }

    /// Controls replacement at successful output commit.
    pub fn overwrite(mut self, overwrite: bool) -> Self {
        self.overwrite = overwrite;
        self
    }

    /// Selects CPU render concurrency; one preserves serial scene evaluation.
    pub fn workers(mut self, workers: usize) -> Self {
        self.workers = workers;
        self
    }

    /// Checks native encoding combinations without opening a destination.
    pub fn validate_encoding(&self) -> Result<()> {
        self.audio_metadata.validate()?;
        if !self.audio_metadata.is_empty()
            && matches!(self.codec, Codec::Gif { .. } | Codec::PngSequence)
        {
            return Err(invalid(
                "metadata",
                "Container metadata requires a native video codec",
            ));
        }
        self.codec.validate()?;
        if ![44100, 48000, 96000].contains(&self.audio_sample_rate) {
            return Err(invalid(
                "audio_sample_rate",
                "Require 44100, 48000 or 96000 Hz",
            ));
        }
        if self.bit_depth == BitDepth::Ten && !matches!(self.codec, Codec::H265 { .. }) {
            return Err(invalid("bit_depth", "Ten-bit delivery requires H.265"));
        }
        if let Some(parameters) = &self.x265_params
            && (!matches!(self.codec, Codec::H265 { .. })
                || parameters.len() > 4096
                || parameters.contains('\0'))
        {
            return Err(invalid(
                "x265_params",
                "Require H.265 and at most 4096 bytes without NUL",
            ));
        }
        Ok(())
    }
}

/// Ordered export phase notifications, not a confirmation of commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportPhase {
    /// Input/configuration checks.
    Preparing,
    /// Scene evaluation and output conversion.
    Rendering,
    /// encoding/finalization.
    Finalizing,
    /// Successful committed output.
    Completed,
}

/// Thread-safe progress and cooperative cancellation without UI dependencies.
pub trait ProgressSink: Send + Sync {
    /// Observes an ordered phase change.
    fn phase(&self, _phase: ExportPhase) {}

    /// Observes completed render frames, not committed output.
    fn frame_completed(&self, _completed: u64, _total: u64) {}

    /// Cancellation is checked before work and throughout finalization.
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// No callbacks and no cancellation.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoProgress;

impl ProgressSink for NoProgress {}

/// Successful committed output metadata.
#[derive(Clone, Debug)]
pub struct ExportReport {
    /// Committed path.
    pub path: PathBuf,
    /// Submitted frame count.
    pub frames: u64,
    /// Selected absolute timeline interval.
    pub range: Range<Time>,
    /// Observed wall-clock export seconds, not scene time.
    pub elapsed_seconds: f64,
    /// Final output bytes.
    pub bytes: u64,
    /// Selected font faces and source hashes actually painted by this export.
    pub fonts: Vec<motion_core::FontIdentity>,
}

/// Declarative input restrictions checked independently of codec/device setup.
/// Defaults preserve the unrestricted contract of existing custom encoders.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct EncoderCapabilities {
    /// Whether a prepared timeline audio mix can be accepted by open().
    pub supports_audio: bool,
    /// Optional maximum output width and height.
    pub max_size: Option<motion_core::PixelSize>,
    /// Required width/height multiple; one accepts arbitrary positive dimensions.
    pub dimension_multiple: u32,
}

impl Default for EncoderCapabilities {
    fn default() -> Self {
        Self {
            supports_audio: true,
            max_size: None,
            dimension_multiple: 1,
        }
    }
}

impl EncoderCapabilities {
    /// Validates normalized dimensions before an encoder opens any output.
    pub fn validate_size(&self, size: motion_core::PixelSize) -> Result<()> {
        if self.dimension_multiple == 0
            || size.width == 0
            || size.height == 0
            || !size.width.is_multiple_of(self.dimension_multiple)
            || !size.height.is_multiple_of(self.dimension_multiple)
            || self
                .max_size
                .is_some_and(|maximum| size.width > maximum.width || size.height > maximum.height)
        {
            return Err(invalid(
                "size",
                "Output dimensions are not supported by this encoder",
            ));
        }
        Ok(())
    }

    /// Rejects unsupported timeline audio before PCM preparation or frame evaluation.
    pub fn validate_audio(&self, present: bool) -> Result<()> {
        if present && !self.supports_audio {
            return Err(invalid(
                "audio",
                "This encoder does not accept timeline audio",
            ));
        }
        Ok(())
    }
}

/// Ordered, sendable output extension. Custom implementations own their own
/// destination transactions; dropping after any error must discard partial output.
pub trait Encoder: Send {
    /// Optional restrictions; existing implementations retain permissive defaults.
    fn capabilities(&self) -> EncoderCapabilities {
        EncoderCapabilities::default()
    }

    /// Required memory format, never silently reinterpreted.
    fn input_format(&self) -> PixelFormat;

    /// Receives normalized size/range and cropped audio before frame callbacks.
    fn open(
        &mut self,
        options: &ExportOptions,
        frame_rate: FrameRate,
        audio: Option<&crate::MixedAudio>,
    ) -> Result<()>;

    /// Frames arrive as consecutive zero-based export indices.
    fn write(&mut self, index: u64, frame: &FrameBuffer) -> Result<()>;

    /// Finishes and commits only after every ordered frame has been submitted.
    fn finish(self: Box<Self>) -> Result<ExportReport>;
}
