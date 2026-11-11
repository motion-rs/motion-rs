//! Transactional straight-alpha PNG image sequences.

use crate::{Codec, Encoder, EncoderCapabilities, ExportOptions, ExportReport, MixedAudio};

use motion_core::{FrameRate, Result};

use motion_render::{FrameBuffer, PixelFormat};
use std::time::Instant;

/// A second ordered encoder implementation using bounded per-frame PNG output.
#[derive(Default)]
pub struct PngSequenceEncoder {
    directory: Option<tempfile::TempDir>,
    options: Option<ExportOptions>,
    expected: u64,
    written: u64,
    bytes: u64,
    started: Option<Instant>,
}

impl PngSequenceEncoder {
    /// Creates an unopened PNG sequence encoder.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Encoder for PngSequenceEncoder {
    fn capabilities(&self) -> EncoderCapabilities {
        EncoderCapabilities {
            supports_audio: false,
            max_size: None,
            dimension_multiple: 1,
        }
    }

    fn input_format(&self) -> PixelFormat {
        PixelFormat::SrgbRgba8 {
            premultiplied: false,
        }
    }

    fn open(
        &mut self,
        options: &ExportOptions,
        frame_rate: FrameRate,
        audio: Option<&MixedAudio>,
    ) -> Result<()> {
        self.capabilities().validate_audio(audio.is_some())?;
        if self.options.is_some() || options.codec != Codec::PngSequence {
            return Err(crate::export::invalid(
                "encoder",
                "PNG encoder requires an unopened PNG sequence request",
            ));
        }
        if options.path.try_exists()? {
            if !options.overwrite {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "PNG sequence destination already exists",
                )
                .into());
            }
            if !std::fs::symlink_metadata(&options.path)?.is_dir() {
                return Err(crate::export::invalid(
                    "path",
                    "PNG sequence overwrite requires a real directory",
                ));
            }
        }
        let parent = options
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        let mut builder = tempfile::Builder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o777));
        }
        self.directory = Some(builder.prefix(".motion-png-").tempdir_in(parent)?);
        let range = options.range.as_ref().ok_or_else(|| {
            crate::export::invalid("range", "PNG encoder requires normalized range")
        })?;
        self.expected = (range.end.to_frame(frame_rate) - range.start.to_frame(frame_rate)) as u64;
        self.options = Some(options.clone());
        self.started = Some(Instant::now());
        Ok(())
    }

    fn write(&mut self, index: u64, frame: &FrameBuffer) -> Result<()> {
        let options = self
            .options
            .as_ref()
            .ok_or_else(|| crate::export::invalid("encoder", "PNG encoder is not open"))?;
        if index != self.written
            || index >= self.expected
            || Some(frame.size()) != options.render.size
            || frame.format() != self.input_format()
        {
            return Err(crate::export::invalid(
                "frame",
                "PNG sequence requires ordered matching straight RGBA frames",
            ));
        }
        let path = self
            .directory
            .as_ref()
            .expect("opened sequence")
            .path()
            .join(format!("frame-{index:06}.png"));
        frame.save_png(&path)?;
        self.bytes += std::fs::metadata(path)?.len();
        self.written += 1;
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<ExportReport> {
        let options = self
            .options
            .take()
            .ok_or_else(|| crate::export::invalid("encoder", "PNG encoder is not open"))?;
        if self.written != self.expected {
            return Err(crate::export::invalid(
                "frames",
                "PNG sequence audio is unsupported or frame count is incomplete",
            ));
        }
        let directory = self.directory.take().expect("opened sequence");
        let flags = if options.overwrite && options.path.try_exists()? {
            rustix::fs::RenameFlags::EXCHANGE
        } else {
            rustix::fs::RenameFlags::NOREPLACE
        };
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            directory.path(),
            rustix::fs::CWD,
            &options.path,
            flags,
        )
        .map_err(std::io::Error::from)?;
        Ok(ExportReport {
            fonts: Vec::new(),
            path: options.path,
            frames: self.written,
            range: options.range.expect("normalized range"),
            elapsed_seconds: self
                .started
                .expect("opened sequence")
                .elapsed()
                .as_secs_f64(),
            bytes: self.bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::{Color, Node, PixelSize, Size, TimeLiteral, Timeline};

    #[test]
    fn transparent_png_sequence_commits_only_on_complete_export() -> Result {
        let directory = tempfile::tempdir()?;
        let destination = directory.path().join("frames");
        let timeline = Timeline::new(Size::new(3.0, 3.0), 2)
            .track(|track| {
                track.push(motion_core::Scene::with_duration(
                    |_: &motion_core::SceneContext<'_>| Node::empty(),
                    1.seconds(),
                ))
            })
            .build()?;
        let mut options = ExportOptions::new(&destination)
            .codec(Codec::PngSequence)
            .size(PixelSize::new(3, 3));
        options.render.background = None;
        let report = crate::export(&timeline, &options, &crate::NoProgress)?;
        assert_eq!(report.frames, 2);
        assert_eq!(std::fs::read_dir(&destination)?.count(), 2);
        assert!(std::fs::read(destination.join("frame-000000.png"))?.starts_with(b"\x89PNG"));
        assert!(crate::export(&timeline, &options, &crate::NoProgress).is_err());
        std::fs::write(destination.join("stale.txt"), b"previous export")?;
        options.overwrite = true;
        crate::export(&timeline, &options, &crate::NoProgress)?;
        assert!(!destination.join("stale.txt").exists());
        assert_eq!(std::fs::read_dir(&destination)?.count(), 2);
        let _color = Color::TRANSPARENT;
        Ok(())
    }
}
