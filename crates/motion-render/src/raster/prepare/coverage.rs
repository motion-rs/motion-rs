//! Clip, rectangle and strip coverage emission plus the shared pixel-bounds and paint helpers
//! used by preparation.

use crate::prepared::{ClipRecord, PaintRecord, PixelRect, PreparedScene, Span, Strip};
use crate::raster::{Band, RasterScope};
use motion_core::budget::Budget;
use motion_core::{Color, PixelSize, Rect, ValidationError};

impl PreparedScene {
    pub(in crate::raster) fn prepare_ir_clip(
        &mut self,
        clip: &motion_core::PathClip,
        geometry: &crate::raster::frame_ir::IrPath<'_>,
        tolerance: f64,
        budget: &mut Budget,
        path: &str,
    ) -> Result<usize, ValidationError> {
        let strip_start = self.strips.len();
        let span_start = self.spans.len();
        self.polygon(
            clip.path.verbs().iter().copied(),
            clip.transform,
            clip.rule,
            tolerance,
            budget,
            path,
        )?;
        let record = ClipRecord {
            bounds: pixel_bounds(geometry.bounds, self.size),
            strips: strip_start..self.strips.len(),
            spans: span_start..self.spans.len(),
        };
        let index = self.clips.len();
        budget.push(&mut self.clips, record, path)?;
        Ok(index)
    }

    pub(crate) fn rectangle(
        &mut self,
        bounds: Rect,
        budget: &mut Budget,
        path: &str,
    ) -> Result<(), ValidationError> {
        if bounds.right == bounds.left || bounds.bottom == bounds.top {
            return Ok(());
        }
        let bounds = Rect::new(
            bounds.left.clamp(0.0, f64::from(self.size.width)),
            bounds.top.clamp(0.0, f64::from(self.size.height)),
            bounds.right.clamp(0.0, f64::from(self.size.width)),
            bounds.bottom.clamp(0.0, f64::from(self.size.height)),
        );
        if bounds.right <= bounds.left || bounds.bottom <= bounds.top {
            return Ok(());
        }
        let left = bounds.left.floor() as u32;
        let right = bounds.right.ceil() as u32;
        let bottom = bounds.bottom.ceil() as u32;
        let full_left = ((bounds.left.ceil() as u64).div_ceil(4) * 4).min(u64::from(right)) as u32;
        let full_right = (bounds.right.floor() as u32) / 4 * 4;
        let mut row = (bounds.top.floor() as u32) / 4 * 4;
        while row < bottom {
            budget.work(1, path)?;
            let height = (self.size.height - row).min(4);
            if f64::from(row) >= bounds.top
                && f64::from(row + height) <= bounds.bottom
                && full_right > full_left
            {
                self.strip(
                    bounds,
                    Band {
                        row,
                        left,
                        right: full_left,
                    },
                    height,
                    &mut RasterScope::new(budget, path),
                )?;
                budget.work(1, path)?;
                budget.push(
                    &mut self.spans,
                    Span {
                        start_x: full_left,
                        end_x: full_right,
                        y: row,
                        height,
                    },
                    path,
                )?;
                self.strip(
                    bounds,
                    Band {
                        row,
                        left: full_right,
                        right,
                    },
                    height,
                    &mut RasterScope::new(budget, path),
                )?;
            } else {
                self.strip(
                    bounds,
                    Band { row, left, right },
                    height,
                    &mut RasterScope::new(budget, path),
                )?;
            }
            row += height;
        }
        Ok(())
    }

    pub(crate) fn strip(
        &mut self,
        bounds: Rect,
        band: Band,
        height: u32,
        scope: &mut RasterScope<'_>,
    ) -> Result<(), ValidationError> {
        let Band { row, left, right } = band;
        let (budget, path) = (&mut *scope.budget, scope.path);
        if right <= left {
            return Ok(());
        }
        let count = u64::from(right - left) * u64::from(height);
        budget.work(count, path)?;
        budget.bytes(count, path)?;
        let count = usize::try_from(count).map_err(|_| {
            ValidationError::limit(path, "max_bytes", "Coverage exceeds addressable storage")
        })?;
        self.alphas.try_reserve(count).map_err(|_| {
            ValidationError::limit(path, "max_bytes", "Could not reserve coverage storage")
        })?;
        let offset = self.alphas.len();
        budget.push(
            &mut self.strips,
            Strip {
                x: left,
                y: row,
                width: right - left,
                height,
                alpha_offset: offset,
            },
            path,
        )?;
        for vertical in row..row + height {
            let coverage_y = overlap(bounds.top, bounds.bottom, vertical);
            for horizontal in left..right {
                self.alphas.push(
                    (overlap(bounds.left, bounds.right, horizontal) * coverage_y * 255.0).round()
                        as u8,
                );
            }
        }
        Ok(())
    }
}

pub(crate) fn overlap(start: f64, end: f64, pixel: u32) -> f64 {
    (end.min(f64::from(pixel) + 1.0) - start.max(f64::from(pixel))).clamp(0.0, 1.0)
}

pub(crate) fn pixel_bounds(bounds: Rect, size: PixelSize) -> PixelRect {
    PixelRect {
        left: bounds.left.floor().clamp(0.0, f64::from(size.width)) as u32,
        top: bounds.top.floor().clamp(0.0, f64::from(size.height)) as u32,
        right: bounds.right.ceil().clamp(0.0, f64::from(size.width)) as u32,
        bottom: bounds.bottom.ceil().clamp(0.0, f64::from(size.height)) as u32,
    }
}

pub(crate) fn paint_record(color: Color, path: &str) -> Result<PaintRecord, ValidationError> {
    let linear_premultiplied = color.to_linear_premultiplied().map(|value| value as f32);
    if linear_premultiplied.iter().any(|value| !value.is_finite()) {
        return Err(ValidationError::invalid(
            path,
            "paint",
            "Linear premultiplied paint must fit finite f32 working storage",
        ));
    }
    Ok(PaintRecord {
        linear_premultiplied,
    })
}
