//! Coverage payload construction and lookup for planned steps: dense, packed, clip and
//! conditional coverage words, plus their CPU-side bounds and sample evaluation.

use super::*;
use motion_core::ValidationError;

impl Planner<'_> {
    pub(super) fn coverage(
        &mut self,
        strips: Range<usize>,
        spans: Range<usize>,
        value: f32,
    ) -> Result<Vec<f32>, ValidationError> {
        self.budget
            .work((strips.len() + spans.len()) as u64, "gpu.coverage_bounds")?;
        let mut bounds = PixelRect {
            left: self.frame.size.width,
            top: self.frame.size.height,
            right: 0,
            bottom: 0,
        };
        for rect in self.frame.strips[strips.clone()]
            .iter()
            .map(|strip| PixelRect {
                left: strip.x,
                top: strip.y,
                right: strip.x + strip.width,
                bottom: strip.y + strip.height,
            })
            .chain(
                self.frame.spans[spans.clone()]
                    .iter()
                    .map(|span| PixelRect {
                        left: span.start_x,
                        top: span.y,
                        right: span.end_x,
                        bottom: span.y + span.height,
                    }),
            )
        {
            bounds.left = bounds.left.min(rect.left);
            bounds.top = bounds.top.min(rect.top);
            bounds.right = bounds.right.max(rect.right);
            bounds.bottom = bounds.bottom.max(rect.bottom);
        }
        if bounds.right < bounds.left || bounds.bottom < bounds.top {
            bounds = PixelRect::default();
        }
        let area = self.frame.strips[strips.clone()]
            .iter()
            .map(|strip| u64::from(strip.width) * u64::from(strip.height))
            .chain(
                self.frame.spans[spans.clone()]
                    .iter()
                    .map(|span| u64::from(span.end_x - span.start_x) * u64::from(span.height)),
            )
            .sum::<u64>();
        self.budget.work(area, "gpu.coverage_classify")?;
        let opaque = area == bounds.pixels()
            && self.frame.strips[strips.clone()].iter().all(|strip| {
                self.frame.alphas
                    [strip.alpha_offset..strip.alpha_offset + (strip.width * strip.height) as usize]
                    .iter()
                    .all(|alpha| *alpha == 255)
            });
        if opaque && area != 0 {
            let mut data = self.data(5, 1.0)?;
            for (target, word) in data[..4].iter_mut().zip([
                bounds.left | (1 << 31),
                bounds.top,
                bounds.right,
                bounds.bottom,
            ]) {
                *target = f32::from_bits(word);
            }
            return Ok(data);
        }
        let mut data = self.coverage_packed(bounds, value)?;
        let width = (bounds.right - bounds.left) as usize;
        for strip in &self.frame.strips[strips] {
            self.budget.work(
                u64::from(strip.width) * u64::from(strip.height),
                "gpu.coverage",
            )?;
            for row in 0..strip.height {
                for column in 0..strip.width {
                    let index = (strip.y + row - bounds.top) as usize * width
                        + (strip.x + column - bounds.left) as usize;
                    let alpha = self.frame.alphas
                        [strip.alpha_offset + (row * strip.width + column) as usize];
                    Self::set_coverage_byte(&mut data, index, alpha);
                }
            }
        }
        for span in &self.frame.spans[spans] {
            self.budget.work(
                u64::from(span.end_x - span.start_x) * u64::from(span.height),
                "gpu.coverage",
            )?;
            for row in span.y..span.y + span.height {
                for column in span.start_x..span.end_x {
                    let index =
                        (row - bounds.top) as usize * width + (column - bounds.left) as usize;
                    Self::set_coverage_byte(&mut data, index, 255);
                }
            }
        }
        Ok(data)
    }

    pub(super) fn coverage_packed(
        &mut self,
        bounds: PixelRect,
        value: f32,
    ) -> Result<Vec<f32>, ValidationError> {
        let count = usize::try_from(bounds.pixels().div_ceil(4)).map_err(|_| {
            ValidationError::limit(
                "gpu.coverage",
                "max_bytes",
                "Coverage exceeds addressable storage",
            )
        })?;
        let alpha = if value == 0.0 { 0 } else { u32::MAX };
        let mut data = self.data(
            count.checked_add(4).ok_or_else(|| {
                ValidationError::limit("gpu.coverage", "max_bytes", "Coverage storage overflow")
            })?,
            f32::from_bits(alpha),
        )?;
        for (target, word) in data[..4].iter_mut().zip([
            bounds.left,
            bounds.top | (1 << 31),
            bounds.right,
            bounds.bottom,
        ]) {
            *target = f32::from_bits(word);
        }
        Ok(data)
    }

    pub(super) fn set_coverage_byte(data: &mut [f32], index: usize, alpha: u8) {
        let shift = (index % 4) * 8;
        let target = &mut data[4 + index / 4];
        *target =
            f32::from_bits((target.to_bits() & !(255 << shift)) | (u32::from(alpha) << shift));
    }

    pub(in crate::gpu) fn coverage_bounds(&self, data: &[f32]) -> PixelRect {
        if data.len() == 1 {
            PixelRect {
                left: 0,
                top: 0,
                right: self.frame.size.width,
                bottom: self.frame.size.height,
            }
        } else if matches!(
            data[0].to_bits(),
            SPARSE_COVERAGE | CONDITIONAL_COVERAGE | DIRECT_COVERAGE
        ) {
            PixelRect {
                left: data[2].to_bits(),
                top: data[3].to_bits(),
                right: data[4].to_bits(),
                bottom: data[5].to_bits(),
            }
        } else {
            PixelRect {
                left: data[0].to_bits() & 0x7fffffff,
                top: data[1].to_bits() & 0x7fffffff,
                right: data[2].to_bits(),
                bottom: data[3].to_bits(),
            }
        }
    }

    pub(in crate::gpu) fn coverage_at(&self, data: &[f32], column: u32, row: u32) -> f32 {
        if data.len() == 1 {
            return data[0];
        }
        let bounds = self.coverage_bounds(data);
        if column < bounds.left
            || column >= bounds.right
            || row < bounds.top
            || row >= bounds.bottom
        {
            return 0.0;
        }
        if data[0].to_bits() == DIRECT_COVERAGE {
            return self.direct_coverage_at(data, column, row);
        }
        if data[0].to_bits() == CONDITIONAL_COVERAGE {
            let child_offset = 8 + data[6].to_bits() as usize;
            let ancestor_offset = child_offset + data[7].to_bits() as usize;
            let alpha = self.coverage_at(&data[8..child_offset], column, row);
            let numerator = self.coverage_at(&data[child_offset..ancestor_offset], column, row);
            let denominator = self.coverage_at(&data[ancestor_offset..], column, row);
            return alpha
                * if denominator == 0.0 {
                    0.0
                } else {
                    (numerator / denominator).min(1.0)
                };
        }
        if data[0].to_bits() == SPARSE_COVERAGE {
            for record in data[7..7 + data[6].to_bits() as usize * 8]
                .as_chunks::<8>()
                .0
            {
                let words: [u32; 8] = std::array::from_fn(|index| record[index].to_bits());
                if column >= words[0] && row >= words[1] && column < words[2] && row < words[3] {
                    if words[4] == u32::MAX {
                        return 1.0;
                    }
                    let index = ((row - words[6]) * words[7] + column - words[5]) as usize;
                    return ((data[words[4] as usize + index / 4].to_bits() >> ((index % 4) * 8))
                        & 255) as f32
                        / 255.0;
                }
            }
            return 0.0;
        }
        if data[0].to_bits() & (1 << 31) != 0 {
            return data[4];
        }
        let index = (row - bounds.top) as usize * (bounds.right - bounds.left) as usize
            + (column - bounds.left) as usize;
        if data[1].to_bits() & (1 << 31) != 0 {
            return ((data[4 + index / 4].to_bits() >> ((index % 4) * 8)) & 255) as f32 / 255.0;
        }
        data[4 + index]
    }

    pub(super) fn clip(&mut self, index: Option<usize>) -> Result<Vec<f32>, ValidationError> {
        if let Some(index) = index {
            let clip = &self.frame.clips[index];
            // Every pixel of a composite or mask scans all direct records, so large clips use
            // dense coverage whose lookup cost does not grow with the record count.
            if !clip.strips.is_empty()
                && clip.strips.len() + clip.spans.len() <= DIRECT_CLIP_RECORDS
            {
                return self.direct_coverage(clip.strips.clone(), clip.spans.clone(), clip.bounds);
            }
            self.coverage(clip.strips.clone(), clip.spans.clone(), 0.0)
        } else {
            self.data(1, 1.0)
        }
    }

    pub(super) fn clip_work(data: &[f32]) -> u64 {
        if data[0].to_bits() == DIRECT_COVERAGE {
            (u64::from(data[7].to_bits() - data[6].to_bits())
                + u64::from(data[9].to_bits() - data[8].to_bits()))
            .saturating_mul(8)
            .max(1)
        } else if data[0].to_bits() == SPARSE_COVERAGE {
            u64::from(data[6].to_bits()).saturating_mul(8).max(1)
        } else {
            1
        }
    }

    pub(super) fn conditional_clip(
        &mut self,
        alpha: Vec<f32>,
        child: Vec<f32>,
        ancestor: Vec<f32>,
    ) -> Result<Vec<f32>, ValidationError> {
        let bounds = self
            .coverage_bounds(&alpha)
            .intersect(self.coverage_bounds(&child))
            .intersect(self.coverage_bounds(&ancestor));
        let words = alpha
            .len()
            .checked_add(child.len())
            .and_then(|words| words.checked_add(ancestor.len()))
            .and_then(|words| words.checked_add(8))
            .and_then(|words| u32::try_from(words).ok())
            .ok_or_else(|| {
                ValidationError::limit(
                    "gpu.clips",
                    "max_bytes",
                    "Conditional coverage exceeds GPU addressing",
                )
            })?;
        let mut data = self.data(words as usize, 0.0)?;
        for (target, word) in data[..8].iter_mut().zip([
            CONDITIONAL_COVERAGE,
            words,
            bounds.left,
            bounds.top,
            bounds.right,
            bounds.bottom,
            alpha.len() as u32,
            child.len() as u32,
        ]) {
            *target = f32::from_bits(word);
        }
        let mut offset = 8;
        for payload in [&alpha, &child, &ancestor] {
            data[offset..offset + payload.len()].copy_from_slice(payload);
            offset += payload.len();
        }
        self.budget.release((words as u64 - 8) * 4);
        Ok(data)
    }
}
