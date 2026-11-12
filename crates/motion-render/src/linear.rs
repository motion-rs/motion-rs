//! Backend-neutral linear-light frame and the work counters shared by every pipeline stage.

use motion_core::PixelSize;

/// Logical peak storage and admitted work of one pipeline stage; not a process memory cap.
///
/// Composition counts prepared bytes, captures, pixels and GPU buffers (excluding driver
/// overhead and reusable device state) against commands, mask records and blended pixels.
/// Output conversion counts source, output and pixel payload bytes (excluding the shared
/// sRGB threshold table) against converted pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkStats {
    /// Peak live logical bytes of the stage.
    pub peak_bytes: u64,
    /// Admitted work units of the stage.
    pub work: u64,
}

/// row-major linear-sRGB premultiplied f32 output, without quantization.
///
/// Storage is read-only and may contain finite extended/negative RGB values.
/// CPU/GPU consumers share this read-only output after optional GPU readback.
/// This is not a PNG/video encoder or resident GPU target.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LinearFrame {
    size: PixelSize,
    pixels: Vec<[f32; 4]>,
    stats: WorkStats,
}

impl LinearFrame {
    pub(crate) fn constant(size: PixelSize, pixel: [f32; 4]) -> Self {
        Self {
            size,
            pixels: vec![pixel; size.width as usize * size.height as usize],
            stats: WorkStats::default(),
        }
    }

    pub(crate) fn from_pixels(size: PixelSize, pixels: Vec<[f32; 4]>) -> Self {
        Self {
            size,
            pixels,
            stats: WorkStats::default(),
        }
    }

    pub(crate) fn from_pixels_with_stats(
        size: PixelSize,
        pixels: Vec<[f32; 4]>,
        stats: WorkStats,
    ) -> Self {
        Self {
            size,
            pixels,
            stats,
        }
    }

    /// Output dimensions.
    pub fn size(&self) -> PixelSize {
        self.size
    }

    /// Row-major finite linear-premultiplied RGBA pixels.
    pub fn pixels(&self) -> &[[f32; 4]] {
        &self.pixels
    }

    /// Releases the pixel storage so a scratch pool can reuse it.
    pub(crate) fn into_pixels(self) -> Vec<[f32; 4]> {
        self.pixels
    }

    /// One output pixel, or None outside the half-open output bounds.
    #[cfg(test)]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[f32; 4]> {
        if x >= self.size.width || y >= self.size.height {
            return None;
        }
        Some(self.pixels[y as usize * self.size.width as usize + x as usize])
    }

    /// Work and peak logical storage, including retained PreparedCpuFrame.
    pub fn stats(&self) -> WorkStats {
        self.stats
    }
}
