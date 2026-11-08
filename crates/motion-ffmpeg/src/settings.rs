//! Encoder settings: codec and container selection, HEVC profiles, compression options and
//! container metadata with chapters.

use super::ffi;

/// Supported HEVC profiles and their native input precision.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum H265Profile {
    /// Main profile with eight-bit input.
    #[default]
    Main,
    /// Main 10 profile with planar ten-bit little-endian input.
    Main10,
}

/// Native video/container selection supported by the safe encoding boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoCodec {
    /// H.264 in MP4.
    #[default]
    H264,
    /// HEVC in MP4 with an explicit profile and matching input precision.
    H265 {
        /// Encoder profile; selects the input bit depth.
        profile: H265Profile,
    },
    /// VP9 in WebM, optionally retaining alpha.
    Vp9 {
        /// Encode the alpha channel as well as color.
        alpha: bool,
    },
    /// ProRes 4444 in MOV.
    ProRes4444,
}

impl VideoCodec {
    pub(crate) fn encoder(self) -> &'static std::ffi::CStr {
        match self {
            Self::H264 => c"libx264",
            Self::H265 { .. } => c"libx265",
            Self::Vp9 { .. } => c"libvpx-vp9",
            Self::ProRes4444 => c"prores_ks",
        }
    }

    pub(crate) fn container(self) -> &'static std::ffi::CStr {
        match self {
            Self::Vp9 { .. } => c"webm",
            Self::ProRes4444 => c"mov",
            _ => c"mp4",
        }
    }

    pub(crate) fn alpha(self) -> bool {
        matches!(self, Self::Vp9 { alpha: true } | Self::ProRes4444)
    }

    pub(crate) fn pixel_format(self) -> ffi::AVPixelFormat {
        match self {
            Self::H265 {
                profile: H265Profile::Main10,
            } => ffi::AV_PIX_FMT_YUV420P10LE,
            Self::Vp9 { alpha: true } => ffi::AV_PIX_FMT_YUVA420P,
            Self::ProRes4444 => ffi::AV_PIX_FMT_YUVA444P10LE,
            _ => ffi::AV_PIX_FMT_YUV420P,
        }
    }

    /// Probes compiled codec availability without opening a file or allocating an encoder.
    pub fn available(self) -> bool {
        // SAFETY: The name is a static NUL-terminated string and the lookup only reads FFmpeg's
        // static codec registry.
        unsafe { !ffi::avcodec_find_encoder_by_name(self.encoder().as_ptr()).is_null() }
    }
}

/// Native compression and transfer metadata, independent of the render backend.
#[derive(Clone, Debug, Default)]
pub struct MediaMetadata {
    /// UTF-8, NUL-free tag name/value pairs; at most 64.
    pub tags: Vec<(String, String)>,
    /// At most 4096 ordered chapters, using 705600000 integer ticks per second.
    pub chapters: Vec<MediaChapter>,
}

/// Native chapter payload with the timeline's integer tick clock.
#[derive(Clone, Debug)]
pub struct MediaChapter {
    /// Inclusive start, in 705600000 ticks per second.
    pub start: i64,
    /// Exclusive end, in the same clock.
    pub end: i64,
    /// UTF-8 NUL-free chapter title.
    pub title: String,
}

/// Native compression, transfer and container metadata.
#[derive(Clone, Debug)]
pub struct VideoSettings {
    /// Encoder/container selection.
    pub codec: VideoCodec,
    /// Constant-rate factor.
    pub quality: u8,
    /// `medium`, `slow` or `slower`.
    pub preset: &'static str,
    /// Pure 2.4 input gamma rather than the sRGB transfer.
    pub gamma24: bool,
    /// Optional x265-specific parameters.
    pub x265_params: Option<String>,
    /// Optional container tags and chapter table.
    pub metadata: MediaMetadata,
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            codec: VideoCodec::H264,
            quality: 18,
            preset: "medium",
            gamma24: false,
            x265_params: None,
            metadata: MediaMetadata::default(),
        }
    }
}
