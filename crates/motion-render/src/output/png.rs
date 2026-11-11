//! Transactional PNG saving for straight sRGB RGBA8 frames.

use super::{FrameBuffer, PixelFormat, output_invalid};

impl FrameBuffer {
    /// Saves PNG transactionally without replacing an existing destination.
    /// Only straight sRGB RGBA8 frames can be written.
    pub fn save_png(&self, path: impl AsRef<std::path::Path>) -> motion_core::Result {
        self.save_png_with_overwrite(path, false)
    }

    /// Saves PNG atomically; failure preserves an existing destination.
    /// Only straight sRGB RGBA8 frames can be written.
    pub fn save_png_with_overwrite(
        &self,
        path: impl AsRef<std::path::Path>,
        overwrite: bool,
    ) -> motion_core::Result {
        use image::ImageEncoder;
        if self.format
            != (PixelFormat::SrgbRgba8 {
                premultiplied: false,
            })
        {
            return Err(output_invalid("format", "PNG requires straight sRGB RGBA8").into());
        }
        let path = std::path::absolute(path)?;
        if !overwrite && path.try_exists()? {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "PNG destination exists",
            )
            .into());
        }
        let mut builder = tempfile::Builder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o666));
        }
        let mut file = builder
            .prefix(".motion-still-")
            .suffix(".png")
            .tempfile_in(
                path.parent()
                    .ok_or_else(|| output_invalid("path", "PNG path has no parent"))?,
            )?;
        image::codecs::png::PngEncoder::new(file.as_file_mut())
            .write_image(
                self.bytes(),
                self.size.width,
                self.size.height,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|error| motion_core::Error::Other(Box::new(error)))?;
        if overwrite {
            file.persist(path).map_err(|error| error.error)?;
        } else {
            file.persist_noclobber(path).map_err(|error| error.error)?;
        }
        Ok(())
    }
}
