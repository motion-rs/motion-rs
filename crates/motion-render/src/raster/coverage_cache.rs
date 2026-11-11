//! Content-addressed cache of rasterized path coverage reused across frames and translations.

use motion_core::{PixelSize, Rect, ValidationError};

use super::*;
#[cfg(test)]
use crate::raster::prepare::prepare_with_cache;
use motion_core::{FillRule, Path2D, Transform};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[cfg(test)]
mod tests;

#[derive(Clone)]
struct Entry {
    hash: u64,
    verbs: Arc<Vec<motion_core::PathVerb>>,
    transform: [u64; 6],
    size: PixelSize,
    rule: FillRule,
    tolerance: u64,
    strips: Vec<Strip>,
    spans: Vec<Span>,
    alphas: Vec<u8>,
    retained: u64,
    peak: u64,
    work: u64,
    bytes: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    hash: u64,
    transform: [u64; 6],
    translated: bool,
    width: u32,
    height: u32,
    rule: u8,
    tolerance: u64,
}

impl Entry {
    fn key(&self) -> Key {
        let translated = cache_transform(self.verbs.as_slice(), self.transform, self.size);
        Key {
            hash: self.hash,
            transform: translated.unwrap_or(self.transform),
            translated: translated.is_some(),
            width: self.size.width,
            height: self.size.height,
            rule: u8::from(self.rule == FillRule::EvenOdd),
            tolerance: self.tolerance,
        }
    }
}

#[derive(Clone, PartialEq)]
struct ContentKey {
    geometry: Arc<Vec<motion_core::PathVerb>>,
    device: Key,
}

impl Eq for ContentKey {}

impl Hash for ContentKey {
    fn hash<HasherType: Hasher>(&self, state: &mut HasherType) {
        self.device.hash(state);
    }
}

pub(crate) struct CoverageCache {
    shared: Arc<motion_core::SceneCache>,
    pub(super) paints: super::paint::TableCache,
    polygon_scratch: super::polygon::scratch::PolygonScratch,
}

impl Default for CoverageCache {
    fn default() -> Self {
        Self::new(&motion_core::RenderCache::default())
    }
}

pub(super) struct CachedCoverage(Arc<Entry>, i32, i32);

pub(super) fn cache_transform(
    verbs: &[motion_core::PathVerb],
    coefficients: [u64; 6],
    size: PixelSize,
) -> Option<[u64; 6]> {
    let [xx, yx, xy, yy, tx, ty] = coefficients.map(f64::from_bits);
    if [tx, ty].iter().any(|value| !exact_grid(*value)) {
        return None;
    }
    let mut bounds = Rect::new(
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for verb in verbs {
        let points: &[motion_core::Point] = match verb {
            motion_core::PathVerb::MoveTo(point) | motion_core::PathVerb::LineTo(point) => {
                std::slice::from_ref(point)
            }
            motion_core::PathVerb::QuadraticTo(control, end) => &[*control, *end],
            motion_core::PathVerb::CubicTo(first, second, end) => &[*first, *second, *end],
            motion_core::PathVerb::Close => &[],
        };
        for point in points {
            let terms = [xx * point.x, xy * point.y, yx * point.x, yy * point.y];
            if terms.iter().any(|value| !exact_grid(*value)) {
                return None;
            }
            let horizontal = terms[0] + terms[1] + tx;
            let vertical = terms[2] + terms[3] + ty;
            bounds.left = bounds.left.min(horizontal);
            bounds.right = bounds.right.max(horizontal);
            bounds.top = bounds.top.min(vertical);
            bounds.bottom = bounds.bottom.max(vertical);
        }
    }
    if !bounds.left.is_finite()
        || bounds.left < 4.0
        || bounds.top < 4.0
        || bounds.right > f64::from(size.width) - 4.0
        || bounds.bottom > f64::from(size.height) - 4.0
    {
        return None;
    }
    let mut result = coefficients;
    result[4] = (tx - (bounds.left.floor() - 4.0)).to_bits();
    result[5] = (ty - (bounds.top.floor() - 4.0)).to_bits();
    Some(result)
}

fn exact_grid(value: f64) -> bool {
    value.is_finite() && value.abs() <= 1_048_576.0 && (value * 65_536.0).fract() == 0.0
}

fn copy<Value: Clone>(values: &[Value]) -> Option<Vec<Value>> {
    let mut result = Vec::new();
    result.try_reserve_exact(values.len()).ok()?;
    result.extend_from_slice(values);
    Some(result)
}

impl CoverageCache {
    pub(crate) fn new(cache: &motion_core::RenderCache) -> Self {
        Self {
            shared: cache
                .partition(motion_core::CachePartition::Coverage)
                .clone(),
            paints: super::paint::TableCache::new(cache),
            polygon_scratch: Default::default(),
        }
    }

    pub(crate) fn retained_paint_bytes(&self) -> u64 {
        self.paints.retained_bytes()
    }

    fn insert(&self, entry: Entry) {
        self.shared.insert_with_cost(
            ContentKey {
                geometry: entry.verbs.clone(),
                device: entry.key(),
            },
            entry,
            |entry| entry.bytes as usize,
        );
    }

    pub(crate) fn retained_bytes(&self) -> u64 {
        self.shared.retained_bytes() as u64
    }

    fn rasterize_path(
        &mut self,
        frame: &mut PreparedScene,
        geometry: &Path2D,
        transform: Transform,
        rule: FillRule,
        tolerance: f64,
        scope: &mut RasterScope<'_>,
    ) -> Result<(), ValidationError> {
        let result = frame.polygon_path_with_scratch(
            geometry,
            transform,
            rule,
            tolerance,
            &mut self.polygon_scratch,
            scope,
        );
        self.polygon_scratch.trim();
        result
    }

    /// Whether a prepared coverage entry already exists for the fill.
    pub(super) fn lookup(
        &self,
        geometry: &Path2D,
        transform: Transform,
        size: PixelSize,
        rule: FillRule,
        tolerance: f64,
    ) -> Option<CachedCoverage> {
        let hash = geometry.fingerprint();
        let coefficients = transform.coefficients().map(f64::to_bits);
        let translated = cache_transform(geometry.verbs(), coefficients, size);
        let key = Key {
            hash,
            transform: translated.unwrap_or(coefficients),
            translated: translated.is_some(),
            width: size.width,
            height: size.height,
            rule: u8::from(rule == FillRule::EvenOdd),
            tolerance: tolerance.to_bits(),
        };
        let entry: Arc<Entry> = self.shared.lookup(&ContentKey {
            geometry: geometry.shared_verbs(),
            device: key,
        })?;
        let dx = (f64::from_bits(coefficients[4]) - f64::from_bits(entry.transform[4])) as i32;
        let dy = (f64::from_bits(coefficients[5]) - f64::from_bits(entry.transform[5])) as i32;
        Some(CachedCoverage(entry, dx, dy))
    }

    pub(crate) fn polygon(
        &mut self,
        frame: &mut PreparedScene,
        geometry: &Path2D,
        transform: Transform,
        rule: FillRule,
        tolerance: f64,
        scope: &mut RasterScope<'_>,
    ) -> Result<(), ValidationError> {
        let (budget, path) = (&mut *scope.budget, scope.path);
        let hash = geometry.fingerprint();
        let coefficients = transform.coefficients().map(f64::to_bits);
        if let Some(coverage) = self.lookup(geometry, transform, frame.size, rule, tolerance) {
            let entry = &coverage.0;
            let fits = budget
                .live_bytes()
                .checked_add(entry.peak)
                .is_some_and(|bytes| bytes <= budget.limits.max_bytes)
                && budget
                    .work_done()
                    .checked_add(entry.work)
                    .is_some_and(|work| work <= budget.limits.max_work);
            if fits {
                let result = restore(frame, entry, budget, path, coverage.1, coverage.2);
                return result;
            }
            return self.rasterize_path(frame, geometry, transform, rule, tolerance, scope);
        }
        let strips = frame.strips.len();
        let spans = frame.spans.len();
        let alphas = frame.alphas.len();
        let bytes = budget.live_bytes();
        let work = budget.work_done();
        let previous_peak = budget.reset_peak();
        let result = self.rasterize_path(
            frame,
            geometry,
            transform,
            rule,
            tolerance,
            &mut RasterScope::new(budget, path),
        );
        let peak = budget.peak_bytes().saturating_sub(bytes);
        budget.merge_peak(previous_peak);
        result?;
        let retained = budget.live_bytes() - bytes;
        let payload = retained
            + geometry.verbs().len() as u64 * size_of::<motion_core::PathVerb>() as u64
            + (size_of::<Entry>() + size_of::<Key>() + 2 * size_of::<u64>()) as u64;
        if payload <= 16 * 1024 * 1024
            && let (Some(mut cached_strips), Some(cached_spans), Some(cached_alphas)) = (
                copy(&frame.strips[strips..]),
                copy(&frame.spans[spans..]),
                copy(&frame.alphas[alphas..]),
            )
        {
            for strip in &mut cached_strips {
                strip.alpha_offset -= alphas;
            }
            self.insert(Entry {
                hash,
                verbs: geometry.shared_verbs(),
                transform: coefficients,
                size: frame.size,
                rule,
                tolerance: tolerance.to_bits(),
                strips: cached_strips,
                spans: cached_spans,
                alphas: cached_alphas,
                retained,
                peak,
                work: budget.work_done() - work,
                bytes: payload,
            });
        }
        Ok(())
    }
}

fn restore(
    frame: &mut PreparedScene,
    entry: &Entry,
    budget: &mut Budget,
    path: &str,
    dx: i32,
    dy: i32,
) -> Result<(), ValidationError> {
    let initial = budget.live_bytes();
    budget.work(entry.work, path)?;
    budget.bytes(entry.peak, path)?;
    frame.strips.try_reserve(entry.strips.len()).map_err(|_| {
        ValidationError::limit(path, "max_bytes", "Could not reserve cached strips")
    })?;
    frame
        .spans
        .try_reserve(entry.spans.len())
        .map_err(|_| ValidationError::limit(path, "max_bytes", "Could not reserve cached spans"))?;
    frame.alphas.try_reserve(entry.alphas.len()).map_err(|_| {
        ValidationError::limit(path, "max_bytes", "Could not reserve cached coverage")
    })?;
    let offset = frame.alphas.len();
    frame.strips.extend(entry.strips.iter().map(|strip| Strip {
        x: (i64::from(strip.x) + i64::from(dx)) as u32,
        y: (i64::from(strip.y) + i64::from(dy)) as u32,
        alpha_offset: strip.alpha_offset + offset,
        ..*strip
    }));
    frame.spans.extend(entry.spans.iter().map(|span| Span {
        start_x: (i64::from(span.start_x) + i64::from(dx)) as u32,
        end_x: (i64::from(span.end_x) + i64::from(dx)) as u32,
        y: (i64::from(span.y) + i64::from(dy)) as u32,
        ..*span
    }));
    frame.alphas.extend_from_slice(&entry.alphas);
    budget.set_live_bytes(initial + entry.retained);
    Ok(())
}
