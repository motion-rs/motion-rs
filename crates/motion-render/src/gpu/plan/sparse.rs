//! Compact strip records and byte coverage; seven header words and eight per record.

use super::*;
use motion_core::ValidationError;

#[cfg(test)]
mod tests;

impl Planner<'_> {
    pub(super) fn uses_direct_coverage(data: &[f32]) -> bool {
        match data.first().map(|word| word.to_bits()) {
            Some(DIRECT_COVERAGE) => true,
            Some(CONDITIONAL_COVERAGE) => {
                let child = 8 + data[6].to_bits() as usize;
                let ancestor = child + data[7].to_bits() as usize;
                [&data[8..child], &data[child..ancestor], &data[ancestor..]]
                    .into_iter()
                    .any(Self::uses_direct_coverage)
            }
            _ => false,
        }
    }

    pub(super) fn shared_coverage(&mut self) -> Result<Vec<u32>, ValidationError> {
        let frame = self.frame;
        let words = frame
            .strips
            .len()
            .checked_mul(6)
            .and_then(|words| {
                frame
                    .spans
                    .len()
                    .checked_mul(4)
                    .and_then(|spans| words.checked_add(spans))
            })
            .and_then(|words| words.checked_add(frame.alphas.len().div_ceil(4)))
            .and_then(|words| words.checked_add(4))
            .and_then(|words| u32::try_from(words).ok())
            .ok_or_else(|| {
                ValidationError::limit(
                    "gpu.frame_coverage",
                    "max_bytes",
                    "Frame coverage exceeds GPU addressing",
                )
            })?;
        self.budget
            .bytes(u64::from(words) * 12, "gpu.frame_coverage")?;
        self.budget.work(
            u64::from(words) * 2 + frame.alphas.len() as u64,
            "gpu.frame_coverage",
        )?;
        let alpha_base = 4 + frame.strips.len() * 6 + frame.spans.len() * 4;
        let mut data = Vec::new();
        data.try_reserve_exact(words as usize).map_err(|_| {
            ValidationError::limit(
                "gpu.frame_coverage",
                "max_bytes",
                "Could not reserve frame coverage upload",
            )
        })?;
        let checked = |value: usize| {
            u32::try_from(value).map_err(|_| {
                ValidationError::limit(
                    "gpu.frame_coverage",
                    "max_bytes",
                    "Frame coverage index exceeds GPU addressing",
                )
            })
        };
        data.extend([
            checked(frame.strips.len())?,
            checked(frame.spans.len())?,
            checked(frame.alphas.len())?,
            checked(alpha_base)?,
        ]);
        for strip in &frame.strips {
            data.extend([
                strip.x,
                strip.y,
                strip.width,
                strip.height,
                checked(strip.alpha_offset)?,
                u32::from(strip.device),
            ]);
        }
        for span in &frame.spans {
            data.extend([span.start_x, span.y, span.end_x, span.height]);
        }
        for bytes in frame.alphas.chunks(4) {
            let mut packed = [0; 4];
            packed[..bytes.len()].copy_from_slice(bytes);
            data.push(u32::from_le_bytes(packed));
        }
        data.resize(words as usize, 0);
        Ok(data)
    }

    pub(super) fn direct_coverage(
        &mut self,
        strips: Range<usize>,
        spans: Range<usize>,
        clip: PixelRect,
    ) -> Result<Vec<f32>, ValidationError> {
        self.budget
            .work((strips.len() + spans.len()) as u64, "gpu.direct_coverage")?;
        let mut bounds = PixelRect {
            left: clip.right,
            top: clip.bottom,
            right: clip.left,
            bottom: clip.top,
        };
        let rectangles = self.frame.strips[strips.clone()]
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
            );
        for rectangle in rectangles {
            let rectangle = rectangle.intersect(clip);
            if rectangle.pixels() == 0 {
                continue;
            }
            bounds.left = bounds.left.min(rectangle.left);
            bounds.top = bounds.top.min(rectangle.top);
            bounds.right = bounds.right.max(rectangle.right);
            bounds.bottom = bounds.bottom.max(rectangle.bottom);
        }
        if bounds.left > bounds.right || bounds.top > bounds.bottom {
            bounds = PixelRect {
                right: clip.left,
                bottom: clip.top,
                ..clip
            };
        }
        let indices = [strips.start, strips.end, spans.start, spans.end].map(u32::try_from);
        let mut data = self.data(10, 0.0)?;
        for (target, word) in data[..6].iter_mut().zip([
            DIRECT_COVERAGE,
            10,
            bounds.left,
            bounds.top,
            bounds.right,
            bounds.bottom,
        ]) {
            *target = f32::from_bits(word);
        }
        for (target, index) in data[6..].iter_mut().zip(indices) {
            *target = f32::from_bits(index.map_err(|_| {
                ValidationError::limit(
                    "gpu.direct_coverage",
                    "max_bytes",
                    "Coverage range exceeds GPU addressing",
                )
            })?);
        }
        Ok(data)
    }

    pub(super) fn direct_coverage_at(&self, data: &[f32], column: u32, row: u32) -> f32 {
        let strips = data[6].to_bits() as usize..data[7].to_bits() as usize;
        for strip in &self.frame.strips[strips] {
            if column >= strip.x
                && row >= strip.y
                && column < strip.x + strip.width
                && row < strip.y + strip.height
            {
                let offset = strip.alpha_offset
                    + ((row - strip.y) * strip.width + column - strip.x) as usize;
                return self
                    .frame
                    .alphas
                    .get(offset)
                    .map_or(0.0, |alpha| f32::from(*alpha) / 255.0);
            }
        }
        let spans = data[8].to_bits() as usize..data[9].to_bits() as usize;
        for span in &self.frame.spans[spans] {
            if column >= span.start_x
                && column < span.end_x
                && row >= span.y
                && row < span.y + span.height
            {
                return 1.0;
            }
        }
        0.0
    }

    pub(super) fn sparse_coverage(
        &mut self,
        strips: Range<usize>,
        spans: Range<usize>,
        clip: PixelRect,
    ) -> Result<Option<Vec<f32>>, ValidationError> {
        let strip_rect = |strip: &Strip| PixelRect {
            left: strip.x,
            top: strip.y,
            right: strip.x + strip.width,
            bottom: strip.y + strip.height,
        };
        let span_rect = |span: &Span| PixelRect {
            left: span.start_x,
            top: span.y,
            right: span.end_x,
            bottom: span.y + span.height,
        };
        let visible = |rect: PixelRect| rect.intersect(clip).pixels() != 0;
        let mut bounds = PixelRect {
            left: clip.right,
            top: clip.bottom,
            right: clip.left,
            bottom: clip.top,
        };
        let mut records = 0usize;
        let mut alpha_words = 0usize;
        self.budget
            .work((strips.len() + spans.len()) as u64, "gpu.sparse_coverage")?;
        for rect in self.frame.strips[strips.clone()]
            .iter()
            .map(strip_rect)
            .chain(self.frame.spans[spans.clone()].iter().map(span_rect))
            .filter(|rect| visible(*rect))
        {
            let rect = rect.intersect(clip);
            bounds.left = bounds.left.min(rect.left);
            bounds.top = bounds.top.min(rect.top);
            bounds.right = bounds.right.max(rect.right);
            bounds.bottom = bounds.bottom.max(rect.bottom);
            records += 1;
        }
        for strip in self.frame.strips[strips.clone()]
            .iter()
            .filter(|strip| visible(strip_rect(strip)))
        {
            alpha_words += (strip.width as usize * strip.height as usize).div_ceil(4);
        }
        self.budget
            .work(alpha_words as u64 * 8, "gpu.sparse_coverage")?;
        let covered = self.frame.strips[strips.clone()]
            .iter()
            .map(strip_rect)
            .chain(self.frame.spans[spans.clone()].iter().map(span_rect))
            .map(|rect| rect.intersect(clip).pixels())
            .sum::<u64>();
        let opaque = records != 0
            && covered == bounds.pixels()
            && self.frame.strips[strips.clone()]
                .iter()
                .filter(|strip| visible(strip_rect(strip)))
                .all(|strip| {
                    self.frame.alphas[strip.alpha_offset
                        ..strip.alpha_offset + strip.width as usize * strip.height as usize]
                        .iter()
                        .all(|alpha| *alpha == 255)
                });
        let words = 7 + records * 8 + alpha_words;
        if records == 0 || opaque || words as u64 >= 4 + bounds.pixels().div_ceil(4) {
            return Ok(None);
        }
        let length = u32::try_from(words).map_err(|_| {
            ValidationError::limit(
                "gpu.sparse_coverage",
                "max_bytes",
                "Sparse coverage exceeds GPU addressing",
            )
        })?;
        let mut data = self.data(words, 0.0)?;
        for (target, word) in data[..7].iter_mut().zip([
            SPARSE_COVERAGE,
            length,
            bounds.left,
            bounds.top,
            bounds.right,
            bounds.bottom,
            records as u32,
        ]) {
            *target = f32::from_bits(word);
        }
        let mut record = 7;
        let mut payload = 7 + records * 8;
        for strip in self.frame.strips[strips]
            .iter()
            .filter(|strip| visible(strip_rect(strip)))
        {
            let rect = strip_rect(strip).intersect(clip);
            for (target, word) in data[record..record + 8].iter_mut().zip([
                rect.left,
                rect.top,
                rect.right,
                rect.bottom,
                payload as u32,
                strip.x,
                strip.y,
                strip.width,
            ]) {
                *target = f32::from_bits(word);
            }
            let count = strip.width as usize * strip.height as usize;
            for (index, bytes) in self.frame.alphas[strip.alpha_offset..strip.alpha_offset + count]
                .chunks(4)
                .enumerate()
            {
                let mut packed = [0; 4];
                packed[..bytes.len()].copy_from_slice(bytes);
                data[payload + index] = f32::from_bits(u32::from_le_bytes(packed));
            }
            payload += count.div_ceil(4);
            record += 8;
        }
        for span in self.frame.spans[spans]
            .iter()
            .filter(|span| visible(span_rect(span)))
        {
            let rect = span_rect(span).intersect(clip);
            for (target, word) in data[record..record + 8].iter_mut().zip([
                rect.left,
                rect.top,
                rect.right,
                rect.bottom,
                u32::MAX,
                0,
                0,
                0,
            ]) {
                *target = f32::from_bits(word);
            }
            record += 8;
        }
        Ok(Some(data))
    }
}
