//! Bounded palette GIF output using the unified transactional encoder contract.

use crate::{Codec, Encoder, EncoderCapabilities, ExportOptions, ExportReport, MixedAudio};

use motion_core::{FrameRate, Result};

use motion_render::{FrameBuffer, PixelFormat};
use std::time::Instant;

/// Transactional GIF encoder with per-frame palettes and cumulative rational delays.
#[derive(Default)]
pub struct GifEncoder {
    temporary: Option<tempfile::NamedTempFile>,
    encoder: Option<gif::Encoder<std::fs::File>>,
    options: Option<ExportOptions>,
    rate: Option<FrameRate>,
    expected: u64,
    written: u64,
    started: Option<Instant>,
}

fn error(cause: gif::EncodingError) -> motion_core::Error {
    motion_core::Error::Encoder(Box::new(cause))
}

impl GifEncoder {
    /// Creates an unopened GIF encoder; no output file exists until open().
    pub fn new() -> Self {
        Self::default()
    }
}

impl Encoder for GifEncoder {
    fn capabilities(&self) -> EncoderCapabilities {
        EncoderCapabilities {
            supports_audio: false,
            max_size: Some(motion_core::PixelSize::new(
                u16::MAX.into(),
                u16::MAX.into(),
            )),
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
        rate: FrameRate,
        audio: Option<&MixedAudio>,
    ) -> Result<()> {
        self.capabilities().validate_audio(audio.is_some())?;
        if self.options.is_some() || !matches!(options.codec, Codec::Gif { .. }) {
            return Err(crate::export::invalid(
                "encoder",
                "GIF encoder requires an unopened GIF request",
            ));
        }
        options.codec.validate()?;
        let size = options
            .render
            .size
            .ok_or_else(|| crate::export::invalid("size", "GIF requires normalized size"))?;
        self.capabilities().validate_size(size)?;
        let parent = options
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        let mut builder = tempfile::Builder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o666));
        }
        let temporary = builder
            .prefix(".motion-gif-")
            .suffix(".gif")
            .tempfile_in(parent)?;
        let mut encoder = gif::Encoder::new(
            temporary.reopen()?,
            size.width as u16,
            size.height as u16,
            &[],
        )
        .map_err(error)?;
        encoder.set_repeat(gif::Repeat::Infinite).map_err(error)?;
        let range = options
            .range
            .as_ref()
            .ok_or_else(|| crate::export::invalid("range", "GIF requires normalized range"))?;
        self.expected = (range.end.to_frame(rate) - range.start.to_frame(rate)) as u64;
        self.temporary = Some(temporary);
        self.encoder = Some(encoder);
        self.options = Some(options.clone());
        self.rate = Some(rate);
        self.started = Some(Instant::now());
        Ok(())
    }

    fn write(&mut self, index: u64, frame: &FrameBuffer) -> Result<()> {
        let options = self
            .options
            .as_ref()
            .ok_or_else(|| crate::export::invalid("encoder", "GIF encoder is not open"))?;
        if index != self.written
            || index >= self.expected
            || Some(frame.size()) != options.render.size
            || frame.format() != self.input_format()
        {
            return Err(crate::export::invalid(
                "frame",
                "GIF requires ordered matching straight RGBA frames",
            ));
        }
        let Codec::Gif { colors, .. } = options.codec else {
            unreachable!("open rejects every codec except GIF")
        };
        let mut rgba = frame.planes()[0].bytes().to_vec();
        let transparent = rgba.as_chunks::<4>().0.iter().any(|pixel| pixel[3] < 128);
        for pixel in rgba.as_chunks_mut::<4>().0 {
            if pixel[3] < 128 {
                pixel.fill(0);
            } else {
                pixel[3] = 255;
            }
        }
        let reserved = usize::from(transparent);
        let quantizer = color_quant::NeuQuant::new(10, usize::from(colors) - reserved, &rgba);
        let mut palette = if transparent {
            vec![0, 0, 0]
        } else {
            Vec::new()
        };
        palette.extend(quantizer.color_map_rgb());
        let pixels = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| {
                if transparent && pixel[3] == 0 {
                    0
                } else {
                    (quantizer.index_of(pixel) + reserved) as u8
                }
            })
            .collect::<Vec<_>>();
        let size = frame.size();
        let mut output = gif::Frame::from_palette_pixels(
            size.width as u16,
            size.height as u16,
            pixels,
            palette,
            transparent.then_some(0),
        );
        output.dispose = gif::DisposalMethod::Background;
        let rate = self.rate.expect("opened GIF");
        let boundary = |frame: u64| {
            (u128::from(frame) * 100 * u128::from(rate.denominator())
                + u128::from(rate.numerator()) / 2)
                / u128::from(rate.numerator())
        };
        output.delay = u16::try_from(boundary(index + 1) - boundary(index))
            .map_err(|_| crate::export::invalid("frame_rate", "GIF delay overflow"))?;
        self.encoder
            .as_mut()
            .expect("opened GIF")
            .write_frame(&output)
            .map_err(error)?;
        self.written += 1;
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<ExportReport> {
        let options = self
            .options
            .take()
            .ok_or_else(|| crate::export::invalid("encoder", "GIF encoder is not open"))?;
        if self.written != self.expected {
            return Err(crate::export::invalid(
                "frames",
                "GIF audio is unsupported or frame count is incomplete",
            ));
        }
        let file = self
            .encoder
            .take()
            .expect("opened GIF")
            .into_inner()
            .map_err(error)?;
        file.sync_all()?;
        drop(file);
        let temporary = self.temporary.take().expect("opened GIF");
        let bytes = temporary.as_file().metadata()?.len();
        if options.overwrite {
            temporary
                .persist(&options.path)
                .map_err(|error| error.error)?;
        } else {
            temporary
                .persist_noclobber(&options.path)
                .map_err(|error| error.error)?;
        }
        Ok(ExportReport {
            fonts: Vec::new(),
            path: options.path,
            frames: self.written,
            range: options.range.expect("normalized range"),
            elapsed_seconds: self.started.expect("opened GIF").elapsed().as_secs_f64(),
            bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::{Node, Size, TimeLiteral, Timeline};

    #[test]
    fn gif_resamples_at_its_requested_rate_and_preserves_transparency() -> Result {
        let directory = tempfile::tempdir()?;
        let timeline = Timeline::new(Size::new(3.0, 3.0), motion_core::fps!(30))
            .track(|track| {
                track.push(motion_core::Scene::with_duration(
                    |_: &motion_core::SceneContext<'_>| Node::empty(),
                    1.seconds(),
                ))
            })
            .build()?;
        let options = ExportOptions::new(directory.path().join("output.gif")).codec(Codec::Gif {
            frame_rate: motion_core::fps!(10),
            colors: 16,
        });
        let report = crate::export(&timeline, &options, &crate::NoProgress)?;
        assert_eq!(report.frames, 10);
        let mut decoder = gif::DecodeOptions::new();
        decoder.set_color_output(gif::ColorOutput::RGBA);
        let mut reader = decoder
            .read_info(std::fs::File::open(&options.path)?)
            .map_err(|cause| motion_core::Error::Encoder(Box::new(cause)))?;
        let mut frames = 0;
        while let Some(frame) = reader
            .read_next_frame()
            .map_err(|cause| motion_core::Error::Encoder(Box::new(cause)))?
        {
            assert_eq!(frame.delay, 10);
            assert!(
                frame
                    .buffer
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|pixel| pixel[3] == 0)
            );
            frames += 1;
        }
        assert_eq!(frames, 10);
        Ok(())
    }
}
