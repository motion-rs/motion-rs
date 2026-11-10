//! FFmpeg adapter for the unified ordered encoder contract.

use crate::export::*;
use motion_core::{FrameRate, Result};
use motion_render::{FrameBuffer, PixelFormat};

#[cfg(any(feature = "ffmpeg", test))]
pub(crate) mod native;
#[cfg(feature = "ffmpeg")]
mod worker;

/// Ordered bounded streaming adapter; the native codec stays on its encoding thread.
pub struct FfmpegEncoder<'a> {
    codec: Codec,
    bit_depth: crate::BitDepth,
    progress: &'a dyn ProgressSink,
    options: Option<ExportOptions>,
    #[cfg(feature = "ffmpeg")]
    worker: Option<worker::EncodingWorker>,
    written: u64,
    expected: u64,
}

impl<'a> FfmpegEncoder<'a> {
    /// Creates an unopened adapter sharing cancellation with the export orchestrator.
    pub fn new(progress: &'a dyn ProgressSink) -> Self {
        Self {
            codec: Codec::default(),
            bit_depth: crate::BitDepth::Eight,
            progress,
            options: None,
            #[cfg(feature = "ffmpeg")]
            worker: None,
            written: 0,
            expected: 0,
        }
    }

    /// Selects native encoder input capabilities before opening the request.
    pub fn codec(mut self, codec: Codec) -> Self {
        self.codec = codec;
        self
    }

    /// Selects input precision before the orchestrator queries capabilities.
    pub fn bit_depth(mut self, depth: crate::BitDepth) -> Self {
        self.bit_depth = depth;
        self
    }
}

impl Encoder for FfmpegEncoder<'_> {
    fn capabilities(&self) -> EncoderCapabilities {
        EncoderCapabilities {
            supports_audio: !matches!(self.codec, Codec::Vp9 { .. }),
            max_size: Some(motion_core::PixelSize::new(16_384, 16_384)),
            dimension_multiple: 2,
        }
    }

    fn input_format(&self) -> PixelFormat {
        if matches!(
            self.codec,
            Codec::Vp9 { alpha: true, .. } | Codec::ProRes4444
        ) {
            PixelFormat::SrgbRgba8 {
                premultiplied: false,
            }
        } else {
            PixelFormat::Yuv420 {
                bit_depth: self.bit_depth.bits(),
            }
        }
    }

    fn open(
        &mut self,
        options: &ExportOptions,
        rate: FrameRate,
        audio: Option<&crate::MixedAudio>,
    ) -> Result<()> {
        options.validate_encoding()?;
        if self.options.is_some() {
            return Err(invalid("encoder", "Encoder is already open"));
        }
        #[cfg(feature = "ffmpeg")]
        {
            if matches!(options.codec, Codec::Vp9 { .. }) && audio.is_some() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "WebM audio requires an Opus/Vorbis adapter",
                )
                .into());
            }
            self.codec = options.codec;
            self.bit_depth = options.bit_depth;
            let native_codec = native::video_codec(options.codec, options.bit_depth)?;
            if !native_codec.available() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "Requested FFmpeg video encoder is not installed",
                )
                .into());
            }
            let size = options
                .render
                .size
                .ok_or_else(|| invalid("size", "Encoder requires normalized dimensions"))?;
            let range = options
                .range
                .as_ref()
                .ok_or_else(|| invalid("range", "Encoder requires normalized range"))?;
            let count = range.end.to_frame(rate) - range.start.to_frame(rate);
            let quality = match options.codec {
                Codec::H264 { quality, .. }
                | Codec::H265 { quality }
                | Codec::Vp9 { quality, .. } => quality,
                Codec::ProRes4444 => 18,
                _ => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        "Requested native codec is not available in this adapter",
                    )
                    .into());
                }
            };
            if count <= 0
                || count > i64::from(u32::MAX)
                || size.width == 0
                || size.height == 0
                || size.width % 2 != 0
                || size.height % 2 != 0
                || quality
                    > if matches!(options.codec, Codec::Vp9 { .. }) {
                        63
                    } else {
                        51
                    }
                || (range.end - range.start).as_seconds() > 86400.0
            {
                return Err(invalid("encoding", "Invalid H264 dimensions/range/quality"));
            }
            self.worker = Some(worker::EncodingWorker::spawn(
                options.clone(),
                rate,
                count as u64,
                audio.cloned(),
                self.progress,
            )?);
            self.expected = count as u64;
            self.options = Some(options.clone());
            Ok(())
        }
        #[cfg(not(feature = "ffmpeg"))]
        {
            let _ = (options, rate, audio);
            Err(motion_core::Error::Encoder(Box::new(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "export requires feature ffmpeg",
            ))))
        }
    }

    fn write(&mut self, index: u64, frame: &FrameBuffer) -> Result<()> {
        cancelled(self.progress)?;
        let options = self
            .options
            .as_ref()
            .ok_or_else(|| invalid("encoder", "Encoder is not open"))?;
        if index != self.written
            || index >= self.expected
            || Some(frame.size()) != options.render.size
            || frame.format() != self.input_format()
        {
            return Err(invalid(
                "frame",
                "Encoder requires ordered matching YUV420 frames",
            ));
        }
        #[cfg(feature = "ffmpeg")]
        {
            self.worker
                .as_mut()
                .ok_or_else(|| invalid("encoder", "Encoding worker is not open"))?
                .write(frame, self.progress)?;
            self.written += 1;
            Ok(())
        }
        #[cfg(not(feature = "ffmpeg"))]
        Err(invalid("encoder", "Encoding worker requires ffmpeg"))
    }

    fn finish(self: Box<Self>) -> Result<ExportReport> {
        cancelled(self.progress)?;
        if self.options.is_none() {
            return Err(invalid("encoder", "Encoder is not open"));
        }
        if self.written != self.expected {
            return Err(invalid("frames", "Encoder frame count is incomplete"));
        }
        #[cfg(feature = "ffmpeg")]
        return self
            .worker
            .ok_or_else(|| invalid("encoder", "Encoding worker is not open"))?
            .finish(self.progress);
        #[cfg(not(feature = "ffmpeg"))]
        Err(invalid("encoder", "Encoding worker requires ffmpeg"))
    }
}

#[cfg(all(test, feature = "ffmpeg"))]
mod codec_tests {
    use super::*;
    use motion_core::{Color, Node, NodeModifiers, Rect, Shape, Size, TimeLiteral, Timeline};

    #[test]
    #[ignore = "requires native FFmpeg with libx264"]
    fn streaming_opens_large_exports_before_frames_and_drop_preserves_destination() -> Result {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("protected.mp4");
        std::fs::write(&path, b"existing destination")?;
        let mut options = ExportOptions::new(&path).overwrite(true);
        options.render.size = Some(motion_core::PixelSize::new(1920, 1080));
        options.range = Some(motion_core::Time::ZERO..300.seconds());
        let mut encoder = FfmpegEncoder::new(&NoProgress);
        encoder.open(&options, FrameRate::from(30), None)?;
        let names = std::fs::read_dir(directory.path())?
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect::<std::io::Result<Vec<_>>>()?;
        assert!(names.iter().any(|name| name.starts_with(".motion-video-")));
        assert!(!names.iter().any(|name| name.starts_with(".motion-yuv-")));
        drop(encoder);
        assert_eq!(std::fs::read(&path)?, b"existing destination");
        assert_eq!(std::fs::read_dir(directory.path())?.count(), 1);
        Ok(())
    }

    #[test]
    #[ignore = "requires native FFmpeg with libx264/libx265/libvpx-vp9/prores_ks"]
    fn native_codecs_export_readable_ordered_frames_with_alpha_inputs() -> Result {
        let directory = tempfile::tempdir()?;
        let timeline = Timeline::new(Size::new(16.0, 16.0), 2)
            .track(|track| {
                track.push(motion_core::Scene::with_duration(
                    |_: &motion_core::SceneContext<'_>| {
                        Node::from(Rect::new(2.0, 2.0, 14.0, 14.0).fill(Color::WHITE)).opacity(0.5)
                    },
                    1.seconds(),
                ))
            })
            .build()?;
        for (name, codec) in [
            ("h264.mp4", Codec::default()),
            ("hevc.mp4", Codec::H265 { quality: 28 }),
            (
                "vp9.webm",
                Codec::Vp9 {
                    quality: 24,
                    alpha: false,
                },
            ),
            (
                "vp9-alpha.webm",
                Codec::Vp9 {
                    quality: 24,
                    alpha: true,
                },
            ),
            ("prores.mov", Codec::ProRes4444),
        ] {
            let options = ExportOptions::new(directory.path().join(name)).codec(codec);
            let report = crate::export(&timeline, &options, &crate::NoProgress)?;
            assert_eq!(report.frames, 2);
            let media = motion_ffmpeg::inspect::media(&report.path)?;
            assert_eq!((media.width, media.height), (16, 16));
            assert_eq!(motion_ffmpeg::inspect::video_frames(&report.path, 10)?, 2);
            eprintln!("codec fixture passed: {name}");
        }
        Ok(())
    }
}
