//! Bounded reconstruction of prepared path-clip coverage for a live capture.

use super::*;
use motion_core::ValidationError;

pub(crate) struct ClipMask {
    bounds: PixelRect,
    pixels: Vec<u8>,
}

impl ClipMask {
    pub(crate) fn new(
        frame: &PreparedFrame,
        record: &ClipRecord,
        bounds: PixelRect,
        budget: &mut Budget,
        path: &str,
    ) -> Result<Self, ValidationError> {
        let count = bounds.pixels();
        budget.work(count, path)?;
        budget.bytes(count, path)?;
        let count = usize::try_from(count).map_err(|_| {
            ValidationError::limit(path, "max_bytes", "Clip mask exceeds addressable storage")
        })?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(count).map_err(|_| {
            ValidationError::limit(path, "max_bytes", "Could not reserve clip mask storage")
        })?;
        pixels.resize(count, 0);
        let mut mask = Self { bounds, pixels };
        for strip in &frame.strips[record.strips.clone()] {
            budget.work(1, path)?;
            let visible = bounds.intersect(PixelRect {
                left: strip.x,
                top: strip.y,
                right: strip.x + strip.width,
                bottom: strip.y + strip.height,
            });
            budget.work(visible.pixels(), path)?;
            for row in visible.top..visible.bottom {
                for column in visible.left..visible.right {
                    let source = strip.alpha_offset
                        + (row - strip.y) as usize * strip.width as usize
                        + (column - strip.x) as usize;
                    let target = mask.index(column, row);
                    mask.pixels[target] = frame.alphas[source];
                }
            }
        }
        for span in &frame.spans[record.spans.clone()] {
            budget.work(1, path)?;
            let visible = bounds.intersect(PixelRect {
                left: span.start_x,
                top: span.y,
                right: span.end_x,
                bottom: span.y + span.height,
            });
            budget.work(visible.pixels(), path)?;
            for row in visible.top..visible.bottom {
                for column in visible.left..visible.right {
                    let target = mask.index(column, row);
                    mask.pixels[target] = 255;
                }
            }
        }
        Ok(mask)
    }

    pub(crate) fn alpha(&self, x: u32, y: u32) -> u8 {
        self.pixels[self.index(x, y)]
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.pixels.len() as u64
    }

    fn index(&self, x: u32, y: u32) -> usize {
        (y - self.bounds.top) as usize * (self.bounds.right - self.bounds.left) as usize
            + (x - self.bounds.left) as usize
    }
}

#[cfg(test)]
mod tests;
