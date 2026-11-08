//! Opaque 0x00RRGGBB snapshots and their PPM/RGB24 serialization.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

/// Owned opaque row-major 0x00RRGGBB output.
pub struct Snapshot {
    width: usize,
    height: usize,
    pixels: Vec<u32>,
}

impl Snapshot {
    pub(crate) fn from_pixels(width: usize, height: usize, pixels: Vec<u32>) -> Self {
        Self {
            width,
            height,
            pixels,
        }
    }

    /// Returns the framebuffer width in pixels.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Returns the framebuffer height in pixels.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Borrows pixels in top-to-bottom, left-to-right order as `0x00RRGGBB` values.
    /// The slice length is `width() * height()`; pixel `(x, y)` is at
    /// `y * width() + x`. The high byte is zero, not a retained alpha channel.
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// Writes binary RGB PPM (P6), creating or truncating the destination file.
    /// Does not use FFmpeg or video overwrite policy; an existing PPM
    /// destination is overwritten unconditionally. Parent directories must exist.
    ///
    /// # Errors
    /// Returns file creation, write, or flush errors from the destination.
    pub fn write_ppm(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let mut output = BufWriter::with_capacity(64 * 1024, File::create(path)?);
        write!(output, "P6\n{} {}\n255\n", self.width, self.height)?;
        self.write_rgb(&mut output)?;
        output.flush()
    }

    /// Writes packed RGB24 rows without a file header or alpha reinterpretation.
    pub fn write_rgb(&self, output: &mut impl Write) -> io::Result<()> {
        let mut row = vec![0; self.width * 3];
        for pixels in self.pixels.chunks_exact(self.width) {
            for (pixel, channels) in pixels.iter().zip(row.as_chunks_mut::<3>().0) {
                channels.copy_from_slice(&[(pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8]);
            }
            output.write_all(&row)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ppm_output_preserves_header_rgb_order_and_truncation() -> io::Result<()> {
        let destination = tempfile::NamedTempFile::new()?;
        std::fs::write(destination.path(), [255; 128])?;
        let image = Snapshot::from_pixels(2, 2, vec![0x123456, 0xabcdef, 0x000000, 0xffffff]);
        image.write_ppm(destination.path())?;
        assert_eq!(
            std::fs::read(destination.path())?,
            b"P6\n2 2\n255\n\x12\x34\x56\xab\xcd\xef\x00\x00\x00\xff\xff\xff"
        );
        Ok(())
    }

    #[test]
    fn snapshot_writes_propagate_destination_and_stream_errors() -> io::Result<()> {
        struct FailedWriter;

        impl Write for FailedWriter {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("writer failure"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let image = Snapshot::from_pixels(2, 1, vec![0x123456, 0xabcdef]);
        assert_eq!(
            image.write_rgb(&mut FailedWriter).unwrap_err().to_string(),
            "writer failure"
        );
        let directory = tempfile::tempdir()?;
        assert!(image.write_ppm(directory.path()).is_err());
        let mut bytes = Vec::new();
        image.write_rgb(&mut bytes)?;
        assert_eq!(bytes, [0x12, 0x34, 0x56, 0xab, 0xcd, 0xef]);
        assert_eq!(image.pixels(), [0x123456, 0xabcdef]);
        Ok(())
    }
}
