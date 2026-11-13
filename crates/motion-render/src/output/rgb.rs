//! Encoded RGBA8 conversion, per-pixel access and PPM/RGB24 serialization of frame buffers.

use super::{FrameBuffer, PixelFormat};
use crate::RasterLimits;
use crate::linear::LinearFrame;
use motion_core::ValidationError;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

/// Bytes of the buffered PPM writer.
const PPM_BUFFER_BYTES: usize = 64 * 1024;

fn decode_linear(bytes: &[u8; 16]) -> [f32; 4] {
    std::array::from_fn(|channel| {
        let offset = channel * 4;
        f32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ])
    })
}

impl FrameBuffer {
    /// Returns the encoded `[r, g, b, a]` bytes of one pixel, or `None` outside the
    /// half-open output bounds or when the format is not [`PixelFormat::SrgbRgba8`].
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if !matches!(self.format, PixelFormat::SrgbRgba8 { .. })
            || x >= self.size.width
            || y >= self.size.height
        {
            return None;
        }
        let offset = (y as usize * self.size.width as usize + x as usize) * 4;
        self.planes[0].bytes[offset..offset + 4].try_into().ok()
    }

    /// Returns the `[r, g, b, a]` linear-light premultiplied components of one pixel, or
    /// `None` outside the half-open output bounds or when the format is not
    /// [`PixelFormat::LinearRgba32Float`].
    pub fn linear_pixel(&self, x: u32, y: u32) -> Option<[f32; 4]> {
        if self.format != PixelFormat::LinearRgba32Float
            || x >= self.size.width
            || y >= self.size.height
        {
            return None;
        }
        let offset = (y as usize * self.size.width as usize + x as usize) * 16;
        Some(decode_linear(
            self.planes[0].bytes[offset..offset + 16].try_into().ok()?,
        ))
    }

    /// Iterates [`PixelFormat::LinearRgba32Float`] pixels top-to-bottom, left-to-right;
    /// other formats yield no pixels.
    pub fn linear_pixels(&self) -> impl ExactSizeIterator<Item = [f32; 4]> + '_ {
        let bytes: &[u8] = if self.format == PixelFormat::LinearRgba32Float {
            &self.planes[0].bytes
        } else {
            &[]
        };
        bytes.as_chunks::<16>().0.iter().map(decode_linear)
    }

    /// Converts a [`PixelFormat::LinearRgba32Float`] buffer into `format`.
    ///
    /// SDR formats clip RGB to 0..=1, encode it and ordered-dither it with the fixed 4x4
    /// Bayer matrix; alpha is rounded without dithering. There is no HDR tone mapping and
    /// no display transform. `limits` admit the working and converted storage.
    ///
    /// # Errors
    /// Returns a [`ValidationError`] when this buffer is not linear f32 or when `format`
    /// rejects the pixels (for example YUV with non-opaque alpha).
    pub fn to_format(
        &self,
        format: PixelFormat,
        limits: RasterLimits,
    ) -> Result<FrameBuffer, ValidationError> {
        if self.format != PixelFormat::LinearRgba32Float {
            return Err(ValidationError::invalid(
                "output",
                "format",
                "Only LinearRgba32Float buffers convert to another format",
            ));
        }
        let linear = LinearFrame::from_pixels(self.size, self.linear_pixels().collect());
        FrameBuffer::from_linear(&linear, format, limits)
    }

    /// Returns the pixels in top-to-bottom, left-to-right order as `0x00RRGGBB` values.
    /// The high byte is zero, not a retained alpha channel; pixel `(x, y)` is at
    /// `y * width + x`. Other formats than [`PixelFormat::SrgbRgba8`] yield no pixels.
    pub fn packed_rgb(&self) -> Vec<u32> {
        if !matches!(self.format, PixelFormat::SrgbRgba8 { .. }) {
            return Vec::new();
        }
        self.planes[0]
            .bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| u32::from_be_bytes(*pixel) >> 8)
            .collect()
    }

    /// Writes binary RGB PPM (P6), creating or truncating the destination file.
    /// Does not use FFmpeg or video overwrite policy; an existing PPM
    /// destination is overwritten unconditionally. Parent directories must exist.
    ///
    /// # Errors
    /// Returns an `InvalidInput` error unless the buffer is fully opaque
    /// [`PixelFormat::SrgbRgba8`], and file creation, write or flush errors from the destination.
    pub fn write_ppm(&self, path: impl AsRef<Path>) -> io::Result<()> {
        self.require_opaque_rgba8()?;
        let mut output = BufWriter::with_capacity(PPM_BUFFER_BYTES, File::create(path)?);
        write!(
            output,
            "P6\n{} {}\n255\n",
            self.size.width, self.size.height
        )?;
        self.write_rgb(&mut output)?;
        output.flush()
    }

    /// Writes packed RGB24 rows without a file header or alpha reinterpretation.
    ///
    /// # Errors
    /// Returns an `InvalidInput` error unless the buffer is fully opaque
    /// [`PixelFormat::SrgbRgba8`], and write errors from the destination.
    pub fn write_rgb(&self, output: &mut impl Write) -> io::Result<()> {
        self.require_opaque_rgba8()?;
        let mut row = vec![0; self.size.width as usize * 3];
        for pixels in self.planes[0]
            .bytes
            .chunks_exact(self.size.width as usize * 4)
        {
            for (pixel, channels) in pixels
                .as_chunks::<4>()
                .0
                .iter()
                .zip(row.as_chunks_mut::<3>().0)
            {
                channels.copy_from_slice(&pixel[..3]);
            }
            output.write_all(&row)?;
        }
        Ok(())
    }

    fn require_opaque_rgba8(&self) -> io::Result<()> {
        let opaque = matches!(self.format, PixelFormat::SrgbRgba8 { .. })
            && self
                .planes
                .first()
                .is_some_and(|plane| plane.bytes.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        if opaque {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "RGB output requires fully opaque SrgbRgba8; render over an opaque background",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::{PixelSize, RasterLimits};

    fn frame(width: u32, height: u32, bytes: Vec<u8>) -> FrameBuffer {
        FrameBuffer::from_rgba8(
            PixelSize::new(width, height),
            bytes,
            RasterLimits::default(),
        )
        .unwrap()
    }

    #[test]
    fn ppm_output_preserves_header_rgb_order_and_truncation() -> io::Result<()> {
        let destination = tempfile::NamedTempFile::new()?;
        std::fs::write(destination.path(), [255; 128])?;
        let image = frame(
            2,
            2,
            vec![
                0x12, 0x34, 0x56, 255, 0xab, 0xcd, 0xef, 255, 0, 0, 0, 255, 255, 255, 255, 255,
            ],
        );
        image.write_ppm(destination.path())?;
        assert_eq!(
            std::fs::read(destination.path())?,
            b"P6\n2 2\n255\n\x12\x34\x56\xab\xcd\xef\x00\x00\x00\xff\xff\xff"
        );
        Ok(())
    }

    #[test]
    fn rgb_writes_propagate_destination_and_stream_errors() -> io::Result<()> {
        struct FailedWriter;

        impl Write for FailedWriter {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("writer failure"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let image = frame(2, 1, vec![0x12, 0x34, 0x56, 255, 0xab, 0xcd, 0xef, 255]);
        assert_eq!(
            image.write_rgb(&mut FailedWriter).unwrap_err().to_string(),
            "writer failure"
        );
        let directory = tempfile::tempdir()?;
        assert!(image.write_ppm(directory.path()).is_err());
        let mut bytes = Vec::new();
        image.write_rgb(&mut bytes)?;
        assert_eq!(bytes, [0x12, 0x34, 0x56, 0xab, 0xcd, 0xef]);
        assert_eq!(image.pixel(1, 0), Some([0xab, 0xcd, 0xef, 255]));
        assert_eq!(image.pixel(2, 0), None);
        assert_eq!(image.packed_rgb(), [0x123456, 0xabcdef]);
        Ok(())
    }

    #[test]
    fn rgb_output_rejects_transparent_pixels_and_other_formats() {
        let translucent = frame(1, 1, vec![1, 2, 3, 128]);
        let error = translucent.write_rgb(&mut Vec::new()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(translucent.write_ppm("unused.ppm").is_err());
        assert!(!std::path::Path::new("unused.ppm").exists());
    }
}
