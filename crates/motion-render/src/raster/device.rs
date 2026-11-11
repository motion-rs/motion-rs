//! Fills whose coverage the GPU computes from flattened edges, and their CPU materialization.

use super::polygon::{Edge, edge_bounds, path_edges, scratch::PolygonScratch};
use super::{RasterScope, pixel_bounds};
use crate::prepared::{DeviceFill, PixelRect, PreparedFrame, PreparedItem, Strip};
use motion_core::{FillRule, Path2D, Point, RasterLimits, Transform, ValidationError};
use std::mem::size_of;

/// Largest local coordinate magnitude that f32 edge records resolve to a small fraction of a byte.
const DEVICE_COORDINATE_LIMIT: f64 = 8192.0;

/// Most fills one frame may leave to the GPU.
const DEVICE_MAX_FILLS: usize = 1 << 15;

/// Most bounding-box pixels one frame may leave to the GPU, bounding the dispatch size.
const DEVICE_MAX_PIXELS: u64 = 1 << 24;

/// Most per-pixel edge visits a single fill may cost, estimated as row crossings times width.
const DEVICE_MAX_FILL_VISITS: u64 = 1 << 26;

/// Most per-pixel edge visits all device fills of a frame may cost.
const DEVICE_MAX_FRAME_VISITS: u64 = 1 << 28;

impl PreparedFrame {
    /// Flattens a fill and records its edges for GPU coverage.
    ///
    /// Returns `false` without recording anything when the path needs CPU coverage: its
    /// coordinates exceed f32 resolution or its estimated GPU cost exceeds the frame allowance.
    pub(super) fn device_fill(
        &mut self,
        geometry: &Path2D,
        transform: Transform,
        rule: FillRule,
        tolerance: f64,
        scope: &mut RasterScope<'_>,
    ) -> Result<bool, ValidationError> {
        let (budget, path) = (&mut *scope.budget, scope.path);
        let edges = path_edges(
            geometry.verbs().iter().copied(),
            transform,
            tolerance,
            budget,
            path,
        )?;
        let charged = (edges.len() * size_of::<Edge>()) as u64;
        let bounds = edge_bounds(&edges);
        let pixels = pixel_bounds(bounds, self.size);
        if edges.is_empty() || pixels.is_empty() {
            budget.release(charged);
            return Ok(true);
        }
        let area = pixels.pixels();
        let visits = Self::device_visits(&edges, pixels);
        let within = |value: f64| value.abs() <= DEVICE_COORDINATE_LIMIT;
        let [used_pixels, used_visits] = self.device_totals;
        let eligible = self.device_fills.len() < DEVICE_MAX_FILLS
            && used_pixels + area <= DEVICE_MAX_PIXELS
            && visits <= DEVICE_MAX_FILL_VISITS
            && used_visits + visits <= DEVICE_MAX_FRAME_VISITS
            && edges.iter().all(|edge| {
                within(edge.top.x - f64::from(pixels.left))
                    && within(edge.bottom.x - f64::from(pixels.left))
                    && within(edge.top.y - f64::from(pixels.top))
                    && within(edge.bottom.y - f64::from(pixels.top))
            });
        if !eligible {
            budget.release(charged);
            return Ok(false);
        }
        budget.work(edges.len() as u64, path)?;
        budget.bytes(charged, path)?;
        self.device_edges.try_reserve(edges.len()).map_err(|_| {
            ValidationError::limit(path, "max_bytes", "Could not reserve device fill edges")
        })?;
        let first = self.device_edges.len();
        self.device_edges.extend(edges.iter().map(|edge| {
            [
                edge.top.x,
                edge.top.y,
                edge.bottom.x,
                edge.bottom.y,
                f64::from(edge.winding),
            ]
        }));
        budget.release(charged);
        let alpha_offset = self
            .device_fills
            .last()
            .map_or(0, |fill| Self::device_sample_end(&self.strips[fill.strip]));
        let strip = Strip {
            x: pixels.left,
            y: pixels.top,
            width: pixels.width(),
            height: pixels.height(),
            alpha_offset,
        };
        self.device_fills.push(DeviceFill {
            strip: self.strips.len(),
            edges: first..self.device_edges.len(),
            rule,
        });
        budget.push(&mut self.strips, strip, path)?;
        budget.bytes(size_of::<DeviceFill>() as u64, path)?;
        self.device_totals = [used_pixels + area, used_visits + visits];
        Ok(true)
    }

    /// First sample after a device strip's f32 coverage block.
    pub(crate) fn device_sample_end(strip: &Strip) -> usize {
        strip.alpha_offset + (strip.width * strip.height) as usize
    }

    /// Estimates how many edge visits the GPU makes: row crossings times the pixels per row.
    fn device_visits(edges: &[Edge], pixels: PixelRect) -> u64 {
        let top = f64::from(pixels.top);
        let bottom = f64::from(pixels.bottom);
        let rows = edges
            .iter()
            .map(|edge| {
                let first = edge.top.y.max(top).floor();
                let last = edge.bottom.y.min(bottom).ceil();
                (last - first).max(0.0) as u64
            })
            .sum::<u64>();
        rows.saturating_mul(u64::from(pixels.width()))
    }

    /// Replaces every device fill with CPU strips, spans and alphas for CPU rendering.
    ///
    /// Strips are appended and the owning items repointed, so earlier indices stay valid.
    pub(crate) fn materialize_device_coverage(
        &mut self,
        limits: RasterLimits,
    ) -> Result<(), ValidationError> {
        if self.device_fills.is_empty() {
            return Ok(());
        }
        let fills = std::mem::take(&mut self.device_fills);
        let device_edges = std::mem::take(&mut self.device_edges);
        let mut budget = motion_core::budget::Budget::new(self.size, limits)?;
        let mut scratch = PolygonScratch::default();
        let mut next = fills.iter().peekable();
        for index in 0..self.items.len() {
            let PreparedItem::Fill { strips, spans, .. } = &self.items[index] else {
                continue;
            };
            let Some(fill) = next.next_if(|fill| strips.len() == 1 && strips.start == fill.strip)
            else {
                continue;
            };
            debug_assert!(spans.is_empty());
            let edges = device_edges[fill.edges.clone()]
                .iter()
                .map(|edge| Edge {
                    top: Point::new(edge[0], edge[1]),
                    bottom: Point::new(edge[2], edge[3]),
                    winding: edge[4] as i32,
                })
                .collect::<Vec<_>>();
            let (strip_start, span_start) = (self.strips.len(), self.spans.len());
            self.polygon_edges(
                &edges,
                fill.rule,
                &mut scratch,
                &mut RasterScope::new(&mut budget, "gpu.device_fill"),
            )?;
            if let PreparedItem::Fill { strips, spans, .. } = &mut self.items[index] {
                *strips = strip_start..self.strips.len();
                *spans = span_start..self.spans.len();
            }
        }
        Ok(())
    }
}
