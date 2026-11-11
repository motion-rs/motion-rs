//! Video encoder built on libavcodec/libavformat: codec selection, settings, container
//! metadata and chapters, frame submission and muxing with an optional audio stream.

use std::ffi::CString;
use std::io;
use std::path::Path;
use std::ptr;

use super::audio::{Audio, AudioInput, AudioMix, AudioPcm};
use super::settings::{H265Profile, VideoCodec, VideoSettings};
use super::{check, ffi, path_string};

mod codec;
mod mux;

/// A thread-confined native H.264 encoder. Dropping closes output without committing it.
pub struct Encoder {
    codec: VideoCodec,
    settings: VideoSettings,
    format_context: *mut ffi::AVFormatContext,
    video_context: *mut ffi::AVCodecContext,
    frame: *mut ffi::AVFrame,
    rgb_frame: *mut ffi::AVFrame,
    packet: *mut ffi::AVPacket,
    scale_context: *mut ffi::SwsContext,
    stream: *mut ffi::AVStream,
    width: usize,
    height: usize,
    expected_frames: u32,
    written_frames: u32,
    frame_rate: (u32, u32),
    audio: Option<Audio>,
    failed: bool,
}

impl Encoder {
    /// Opens a local MP4 output; the caller owns temporary-file commit/rollback.
    pub fn new(
        path: &Path,
        size: (usize, usize),
        fps: u32,
        frames: u32,
        crf: u8,
        audio: Option<AudioMix<'_>>,
    ) -> io::Result<Self> {
        Self::new_with_frame_rate(path, size, (fps, 1), frames, crf, audio)
    }

    /// Opens MP4 with an exact positive (numerator, denominator) frame rate.
    /// Both components must fit FFmpeg's signed rational representation.
    /// Audio sample limits use the exact ratio and round up only at sample boundaries.
    /// MP4 movie/edit-list timing uses 705,600,000 ticks per second to avoid
    /// millisecond rounding of common video boundaries and 48 kHz audio samples.
    pub fn new_with_frame_rate(
        path: &Path,
        size: (usize, usize),
        frame_rate: (u32, u32),
        frames: u32,
        crf: u8,
        audio: Option<AudioMix<'_>>,
    ) -> io::Result<Self> {
        Self::new_with_codec(path, size, frame_rate, frames, crf, VideoCodec::H264, audio)
    }

    /// Opens a selected native codec after validating availability and container inputs.
    pub fn new_with_codec(
        path: &Path,
        size: (usize, usize),
        frame_rate: (u32, u32),
        frames: u32,
        crf: u8,
        codec: VideoCodec,
        audio: Option<AudioMix<'_>>,
    ) -> io::Result<Self> {
        Self::new_with_settings(
            path,
            size,
            frame_rate,
            frames,
            VideoSettings {
                codec,
                quality: crf,
                ..VideoSettings::default()
            },
            audio,
        )
    }

    /// Opens explicit precision, compression settings and transfer metadata.
    pub fn new_with_settings(
        path: &Path,
        size: (usize, usize),
        frame_rate: (u32, u32),
        frames: u32,
        settings: VideoSettings,
        audio: Option<AudioMix<'_>>,
    ) -> io::Result<Self> {
        Self::new_with_audio(
            path,
            size,
            frame_rate,
            frames,
            settings,
            audio.map(AudioInput::File),
        )
    }

    /// Opens explicit video settings with owned streaming PCM instead of a WAV bridge.
    pub fn new_with_pcm_settings(
        path: &Path,
        size: (usize, usize),
        frame_rate: (u32, u32),
        frames: u32,
        settings: VideoSettings,
        audio: Option<AudioPcm>,
    ) -> io::Result<Self> {
        Self::new_with_audio(
            path,
            size,
            frame_rate,
            frames,
            settings,
            audio.map(AudioInput::Pcm),
        )
    }

    fn new_with_audio(
        path: &Path,
        size: (usize, usize),
        frame_rate: (u32, u32),
        frames: u32,
        settings: VideoSettings,
        audio: Option<AudioInput<'_>>,
    ) -> io::Result<Self> {
        let metadata = &settings.metadata;
        if metadata.tags.len() > 64
            || metadata.chapters.len() > 4096
            || metadata.tags.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > 128
                    || value.len() > 4096
                    || key.contains('\0')
                    || value.contains('\0')
            })
            || metadata.chapters.iter().any(|chapter| {
                chapter.start < 0
                    || chapter.end <= chapter.start
                    || chapter.title.len() > 4096
                    || chapter.title.contains('\0')
            })
            || metadata
                .chapters
                .windows(2)
                .any(|pair| pair[0].end > pair[1].start)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid native container metadata",
            ));
        }
        let codec = settings.codec;
        let crf = settings.quality;
        if !["medium", "slow", "slower"].contains(&settings.preset)
            || settings.x265_params.as_ref().is_some_and(|value| {
                value.len() > 4096
                    || value.contains('\0')
                    || !matches!(codec, VideoCodec::H265 { .. })
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid native compression settings",
            ));
        }
        if !codec.available() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Requested FFmpeg video encoder is not installed",
            ));
        }
        if matches!(codec, VideoCodec::Vp9 { .. }) && audio.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "WebM audio requires an Opus/Vorbis adapter",
            ));
        }
        let pixels = size
            .0
            .checked_mul(size.1)
            .and_then(|value| value.checked_mul(if codec.alpha() { 4 } else { 3 }));
        if size.0 == 0
            || size.1 == 0
            || !size.0.is_multiple_of(2)
            || !size.1.is_multiple_of(2)
            || size.0 > i32::MAX as usize / if codec.alpha() { 4 } else { 3 }
            || size.1 > i32::MAX as usize
            || pixels.is_none()
            || frame_rate.0 == 0
            || frame_rate.1 == 0
            || frame_rate.0 > i32::MAX as u32
            || frame_rate.1 > i32::MAX as u32
            || frames == 0
            || crf
                > if matches!(codec, VideoCodec::Vp9 { .. }) {
                    63
                } else {
                    51
                }
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid H.264 dimensions, timing or CRF",
            ));
        }
        let mut encoder = Self {
            codec,
            settings,
            format_context: ptr::null_mut(),
            video_context: ptr::null_mut(),
            frame: ptr::null_mut(),
            rgb_frame: ptr::null_mut(),
            packet: ptr::null_mut(),
            scale_context: ptr::null_mut(),
            stream: ptr::null_mut(),
            width: size.0,
            height: size.1,
            expected_frames: frames,
            written_frames: 0,
            frame_rate,
            audio: None,
            failed: false,
        };
        encoder.audio_samples(frames)?;
        encoder.open(&std::path::absolute(path)?, crf, audio)?;
        Ok(encoder)
    }

    /// Converts and submits exactly one tightly packed RGB24 frame.
    pub fn write_rgb(&mut self, rgb: &[u8]) -> io::Result<()> {
        self.write_pixels(rgb, 3)
    }

    /// Converts straight encoded RGBA to an alpha-capable native video frame.
    pub fn write_rgba(&mut self, rgba: &[u8]) -> io::Result<()> {
        self.write_pixels(rgba, 4)
    }

    /// Submits tightly packed limited-range BT.709 8-bit Y/U/V planes without
    /// RGB conversion. The caller must not provide transparent/odd-sized pixels.
    pub fn write_yuv420(&mut self, y: &[u8], u: &[u8], v: &[u8]) -> io::Result<()> {
        if matches!(
            self.codec,
            VideoCodec::H265 {
                profile: H265Profile::Main10,
            }
        ) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Main 10 requires ten-bit input",
            ));
        }
        self.write_yuv_planes(y, u, v, 1)
    }

    /// Submits planar ten-bit LE data; each native row stride is measured in bytes.
    pub fn write_yuv420_10(&mut self, y: &[u8], u: &[u8], v: &[u8]) -> io::Result<()> {
        if !matches!(
            self.codec,
            VideoCodec::H265 {
                profile: H265Profile::Main10,
            }
        ) || [y, u, v].into_iter().any(|plane| {
            !plane.len().is_multiple_of(2)
                || plane
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .any(|sample| u16::from_le_bytes(*sample) > 1023)
        }) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Require Main 10 and samples in 0..=1023",
            ));
        }
        self.write_yuv_planes(y, u, v, 2)
    }

    fn write_yuv_planes(&mut self, y: &[u8], u: &[u8], v: &[u8], depth: usize) -> io::Result<()> {
        if self.failed
            || self.codec.alpha()
            || self.written_frames >= self.expected_frames
            || y.len() != self.width * self.height * depth
            || u.len() != self.width * self.height / 4 * depth
            || v.len() != u.len()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Unexpected YUV planes/count or failed encoder",
            ));
        }
        self.failed = true;
        // SAFETY: A constructed encoder owns an allocated, opened `video_context` and a `frame`
        // whose buffers were allocated for `width`x`height` in the codec's 4:2:0 pixel format.
        // Plane lengths were validated above, so each copied row is `width * depth` bytes
        // (halved for chroma) and starts at `row * linesize`, inside the frame's plane.
        unsafe {
            check(
                ffi::av_frame_make_writable(self.frame),
                "Make YUV frame writable",
            )?;
            for (index, plane) in [y, u, v].into_iter().enumerate() {
                let width = depth
                    * if index == 0 {
                        self.width
                    } else {
                        self.width / 2
                    };
                for (row, bytes) in plane.chunks_exact(width).enumerate() {
                    ptr::copy_nonoverlapping(
                        bytes.as_ptr(),
                        (*self.frame).data[index].add(row * (*self.frame).linesize[index] as usize),
                        width,
                    );
                }
            }
            (*self.frame).pts = i64::from(self.written_frames);
            (*self.frame).duration = 1;
            check(
                ffi::avcodec_send_frame(self.video_context, self.frame),
                "Submit YUV video frame",
            )?;
        }
        self.drain(self.video_context, self.stream)?;
        self.written_frames += 1;
        let samples = self.audio_samples(self.written_frames)?;
        self.write_audio_until(samples, &mut || Ok(()))?;
        self.failed = false;
        Ok(())
    }

    /// Flushes both codecs and closes output. The caller still controls commit.
    pub fn finish(mut self, mut check_cancelled: impl FnMut() -> io::Result<()>) -> io::Result<()> {
        if self.failed || self.written_frames != self.expected_frames {
            return Err(io::Error::other("Incomplete or failed video export"));
        }
        let total = self.audio_samples(self.expected_frames)?;
        self.write_audio_until(total, &mut check_cancelled)?;
        check_cancelled()?;
        // SAFETY: `video_context` was opened during construction; a null frame starts the flush.
        unsafe {
            check(
                ffi::avcodec_send_frame(self.video_context, ptr::null()),
                "Flush H.264 encoder",
            )?;
        }
        self.drain(self.video_context, self.stream)?;
        if let Some(audio) = &self.audio {
            let codec = audio.codec;
            let stream = audio.stream;
            // SAFETY: `codec` is the opened AAC context owned by `self.audio`, which is still
            // alive; a null frame starts the flush.
            unsafe {
                check(
                    ffi::avcodec_send_frame(codec, ptr::null()),
                    "Flush AAC encoder",
                )?;
            }
            self.drain(codec, stream)?;
        }
        check_cancelled()?;
        // SAFETY: `format_context` had its header written and `pb` opened in `open_container`;
        // `avio_closep` closes it and nulls `pb`, so `Drop` will not close it again.
        unsafe {
            check(
                ffi::av_write_trailer(self.format_context),
                "Write MP4 trailer",
            )?;
            check(
                ffi::avio_closep(&mut (*self.format_context).pb),
                "Close MP4 output",
            )?;
        }
        Ok(())
    }

    fn open(&mut self, path: &Path, crf: u8, audio: Option<AudioInput<'_>>) -> io::Result<()> {
        let path = path_string(path)?;
        let quality = CString::new(crf.to_string())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let codec = self.allocate_video(&path)?;
        self.configure_codec(&quality)?;
        self.allocate_video_frames(codec)?;
        self.open_container(&path, audio)
    }

    fn write_pixels(&mut self, rgb: &[u8], channels: usize) -> io::Result<()> {
        if self.failed
            || self.written_frames >= self.expected_frames
            || rgb.len() != self.width * self.height * channels
            || channels != if self.codec.alpha() { 4 } else { 3 }
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Unexpected RGB frame size/count or failed encoder",
            ));
        }
        self.failed = true;
        // SAFETY: `rgb_frame` buffers were allocated for `width`x`height` packed RGB(A) with
        // `channels` bytes per pixel, so each `width * channels` row fits at `row * stride`. The
        // scaler converts between the frame formats it was created for, and `frame` was made
        // writable before receiving its output.
        unsafe {
            check(
                ffi::av_frame_make_writable(self.frame),
                "Make YUV frame writable",
            )?;
            let stride = (*self.rgb_frame).linesize[0] as usize;
            for (row, pixels) in rgb.chunks_exact(self.width * channels).enumerate() {
                ptr::copy_nonoverlapping(
                    pixels.as_ptr(),
                    (*self.rgb_frame).data[0].add(row * stride),
                    pixels.len(),
                );
            }
            let source = [
                (*self.rgb_frame).data[0] as *const u8,
                ptr::null(),
                ptr::null(),
                ptr::null(),
            ];
            let rows = ffi::sws_scale(
                self.scale_context,
                source.as_ptr(),
                (*self.rgb_frame).linesize.as_ptr(),
                0,
                self.height as i32,
                (*self.frame).data.as_ptr(),
                (*self.frame).linesize.as_ptr(),
            );
            if rows != self.height as i32 {
                return Err(io::Error::other("Incomplete RGB/YUV conversion"));
            }
            (*self.frame).pts = i64::from(self.written_frames);
            (*self.frame).duration = 1;
            check(
                ffi::avcodec_send_frame(self.video_context, self.frame),
                "Submit video frame",
            )?;
        }
        self.drain(self.video_context, self.stream)?;
        self.written_frames += 1;
        let samples = self.audio_samples(self.written_frames)?;
        self.write_audio_until(samples, &mut || Ok(()))?;
        self.failed = false;
        Ok(())
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        drop(self.audio.take());
        // SAFETY: `Encoder` exclusively owns every pointer. Free functions accept null and reset
        // the fields; the scaler and format context are null-checked, and `pb` is only closed if
        // `finish` did not already close it. Audio, whose stream belongs to the format context,
        // is dropped first.
        unsafe {
            ffi::av_packet_free(&mut self.packet);
            ffi::av_frame_free(&mut self.frame);
            ffi::av_frame_free(&mut self.rgb_frame);
            ffi::avcodec_free_context(&mut self.video_context);
            if !self.scale_context.is_null() {
                ffi::sws_freeContext(self.scale_context);
            }
            if !self.format_context.is_null() {
                if !(*self.format_context).pb.is_null() {
                    ffi::avio_closep(&mut (*self.format_context).pb);
                }
                ffi::avformat_free_context(self.format_context);
            }
        }
    }
}

#[cfg(test)]
mod cinematic_tests {
    use super::*;

    #[test]
    fn hevc_profiles_select_pixel_formats() {
        for (profile, pixel_format) in [
            (H265Profile::Main, ffi::AV_PIX_FMT_YUV420P),
            (H265Profile::Main10, ffi::AV_PIX_FMT_YUV420P10LE),
        ] {
            let codec = VideoCodec::H265 { profile };
            assert_eq!(codec.encoder(), c"libx265");
            assert_eq!(codec.container(), c"mp4");
            assert_eq!(codec.pixel_format(), pixel_format);
            assert!(!codec.alpha());
        }
    }

    #[test]
    #[ignore = "requires native FFmpeg with libx265; Main10 functional inspection, not a benchmark"]
    fn native_main10_precision_and_metadata() -> io::Result<()> {
        assert!(
            VideoCodec::H265 {
                profile: H265Profile::Main10,
            }
            .available(),
            "libx265 must be installed for Main10 acceptance"
        );
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("motion-main10-{}-{unique}", std::process::id()));
        std::fs::create_dir(&directory)?;
        let result = (|| {
            for gamma24 in [false, true] {
                let output = directory.join(format!("gradient-{gamma24}.mp4"));
                let settings = VideoSettings {
                    codec: VideoCodec::H265 {
                        profile: H265Profile::Main10,
                    },
                    quality: 0,
                    gamma24,
                    x265_params: Some(
                        "lossless=1:log-level=error:pools=none:frame-threads=1".into(),
                    ),
                    ..VideoSettings::default()
                };
                let mut encoder =
                    Encoder::new_with_settings(&output, (512, 16), (24, 1), 1, settings, None)?;
                let luma = (0..16)
                    .flat_map(|_| {
                        (0..512u32)
                            .flat_map(|column| ((64 + column * 876 / 511) as u16).to_le_bytes())
                    })
                    .collect::<Vec<_>>();
                let chroma = vec![512u16.to_le_bytes(); 512 * 16 / 4]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>();
                encoder.write_yuv420_10(&luma, &chroma, &chroma)?;
                encoder.finish(|| Ok(()))?;
                let info = crate::inspect::media(&output)?;
                assert!(
                    info.h265 && info.video_bit_depth == 10 && info.bt709_primaries_matrix_limited
                );
                assert_eq!(info.color_transfer, if gamma24 { 1 } else { 13 });
                let decoded = crate::inspect::video_luma10(&output, 512 * 16)?;
                assert!(
                    decoded
                        .into_iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        > 256
                );
            }
            Ok(())
        })();
        std::fs::remove_dir_all(directory)?;
        result
    }
}
