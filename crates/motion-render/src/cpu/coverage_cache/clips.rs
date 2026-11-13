//! Geometric clip coverage reused across frames when every rasterization input matches exactly.

use super::{CoverageCache, copy};
use crate::cpu::polygon::shape_clip::{self, Region};
use crate::prepared::{FrameRecords, PixelRect, Span, Strip};
use motion_core::budget::Budget;
use motion_core::{Point, ValidationError};
use std::hash::Hasher;
use std::mem::size_of;
use std::sync::Arc;

/// Largest cached clip payload, matching the path coverage entry ceiling.
const MAX_CLIP_PAYLOAD: u64 = 16 * 1024 * 1024;

/// Shared coverage partition handle for shape clip regions.
#[derive(Clone)]
pub(crate) struct ClipCache(Arc<motion_core::SceneCache>);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    hash: u64,
    width: u32,
    height: u32,
    tolerance: u64,
}

struct Entry {
    inputs: Vec<u64>,
    bounds: PixelRect,
    strips: Vec<Strip>,
    spans: Vec<Span>,
    alphas: Vec<u8>,
    retained: u64,
    peak: u64,
    work: u64,
}

/// Every value `shape_clip::rasterize` reads besides frame size and tolerance, as exact bits.
fn inputs<'a>(base: &'a [Point], regions: &'a [&Region]) -> impl Iterator<Item = u64> + 'a {
    [base.len() as u64, regions.len() as u64]
        .into_iter()
        .chain(
            base.iter()
                .flat_map(|point| [point.x.to_bits(), point.y.to_bits()]),
        )
        .chain(regions.iter().flat_map(|region| region.fingerprint()))
}

impl CoverageCache {
    pub(crate) fn clips(&self) -> ClipCache {
        ClipCache(self.shared.clone())
    }
}

impl ClipCache {
    /// Rasterizes the intersection of `regions` and `base`, replaying an identical earlier result.
    ///
    /// A replay appends the same records and charges the same work, peak and retained bytes as
    /// rasterizing; when those charges no longer fit, the clip is rasterized to report the error.
    pub(crate) fn rasterize(
        &self,
        frame: &mut FrameRecords,
        base: &[Point],
        regions: &[&Region],
        tolerance: f64,
        budget: &mut Budget,
        path: &str,
    ) -> Result<PixelRect, ValidationError> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        inputs(base, regions).for_each(|value| hasher.write_u64(value));
        let key = Key {
            hash: hasher.finish(),
            width: frame.size.width,
            height: frame.size.height,
            tolerance: tolerance.to_bits(),
        };
        if let Some(entry) = self.0.lookup::<Key, Entry>(&key)
            && entry.inputs.iter().copied().eq(inputs(base, regions))
        {
            let fits = budget
                .live_bytes()
                .checked_add(entry.peak)
                .is_some_and(|bytes| bytes <= budget.limits.max_bytes)
                && budget
                    .work_done()
                    .checked_add(entry.work)
                    .is_some_and(|work| work <= budget.limits.max_work);
            if fits {
                return entry.restore(frame, budget, path);
            }
            return shape_clip::rasterize(frame, base, regions, tolerance, budget, path);
        }
        let (strips, spans, alphas) = (frame.strips.len(), frame.spans.len(), frame.alphas.len());
        let (bytes, work) = (budget.live_bytes(), budget.work_done());
        let previous_peak = budget.reset_peak();
        let result = shape_clip::rasterize(frame, base, regions, tolerance, budget, path);
        let peak = budget.peak_bytes().saturating_sub(bytes);
        budget.merge_peak(previous_peak);
        let bounds = result?;
        let retained = budget.live_bytes() - bytes;
        let count = inputs(base, regions).count();
        let payload = retained + (count * size_of::<u64>() + size_of::<Entry>()) as u64;
        let mut cached_inputs = Vec::new();
        if payload <= MAX_CLIP_PAYLOAD
            && cached_inputs.try_reserve_exact(count).is_ok()
            && let (Some(mut cached_strips), Some(cached_spans), Some(cached_alphas)) = (
                copy(&frame.strips[strips..]),
                copy(&frame.spans[spans..]),
                copy(&frame.alphas[alphas..]),
            )
        {
            cached_inputs.extend(inputs(base, regions));
            for strip in &mut cached_strips {
                strip.alpha_offset -= alphas;
            }
            self.0.insert_with_cost(
                key,
                Entry {
                    inputs: cached_inputs,
                    bounds,
                    strips: cached_strips,
                    spans: cached_spans,
                    alphas: cached_alphas,
                    retained,
                    peak,
                    work: budget.work_done() - work,
                },
                |_| payload as usize,
            );
        }
        Ok(bounds)
    }
}

impl Entry {
    fn restore(
        &self,
        frame: &mut FrameRecords,
        budget: &mut Budget,
        path: &str,
    ) -> Result<PixelRect, ValidationError> {
        let initial = budget.live_bytes();
        budget.work(self.work, path)?;
        budget.bytes(self.peak, path)?;
        let reserve = |message| ValidationError::limit(path, "max_bytes", message);
        frame
            .strips
            .try_reserve(self.strips.len())
            .map_err(|_| reserve("Could not reserve cached clip strips"))?;
        frame
            .spans
            .try_reserve(self.spans.len())
            .map_err(|_| reserve("Could not reserve cached clip spans"))?;
        frame
            .alphas
            .try_reserve(self.alphas.len())
            .map_err(|_| reserve("Could not reserve cached clip coverage"))?;
        let offset = frame.alphas.len();
        frame.strips.extend(self.strips.iter().map(|strip| Strip {
            alpha_offset: strip.alpha_offset + offset,
            ..*strip
        }));
        frame.spans.extend_from_slice(&self.spans);
        frame.alphas.extend_from_slice(&self.alphas);
        budget.set_live_bytes(initial + self.retained);
        Ok(self.bounds)
    }
}
