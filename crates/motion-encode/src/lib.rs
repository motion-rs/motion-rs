//! Audio preparation and transactional encoding without a window system.

#![forbid(unsafe_code)]

pub mod audio;
pub mod encoder;
pub mod export;
#[cfg(test)]
#[path = "encoder/ffmpeg/export_tests.rs"]
mod export_tests;

#[cfg(feature = "audio")]
pub use audio::{AudioAnalysis, AudioSignal};
pub use audio::{MixOptions, MixSource, MixedAudio, PcmBuffer};
pub use encoder::ffmpeg::FfmpegEncoder;
#[cfg(feature = "ffmpeg")]
use encoder::ffmpeg::native::NativeOptions;
pub use encoder::gif::GifEncoder;
pub use encoder::png::PngSequenceEncoder;
pub use export::{
    BitDepth, Codec, Encoder, EncoderCapabilities, EncoderFactory, EncoderPreset, ExportOptions,
    ExportPhase, ExportReport, NoProgress, ProgressSink, export,
};

/// Encoding and audio operations layered on core scenes and renderer sessions.
pub mod prelude {
    #[cfg(feature = "audio")]
    pub use crate::{AudioAnalysis, AudioSignal};
    pub use crate::{
        BitDepth, Codec, Encoder, EncoderCapabilities, EncoderFactory, EncoderPreset,
        ExportOptions, ExportPhase, ExportReport, FfmpegEncoder, MixOptions, MixSource, MixedAudio,
        NoProgress, PcmBuffer, ProgressSink, export,
    };
}
