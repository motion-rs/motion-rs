//! GPU fill geometry, bounded curve uploads and explicit CPU reference materialization.

#[cfg(test)]
use super::polygon::scratch::PolygonScratch;
use super::polygon::{Edge, edge_bounds, path_edges};
use super::{RasterScope, pixel_bounds};
#[cfg(feature = "gpu")]
use crate::prepared::GpuPreparationStats;
#[cfg(test)]
use crate::prepared::PreparedItem;
use crate::prepared::{DeviceFill, GpuFallbackCounts, PreparedScene, Strip};
#[cfg(test)]
use motion_core::{Point, RasterLimits};
use motion_core::{Transform, ValidationError};
use std::mem::size_of;

mod curves;
mod tiles;

pub(crate) use curves::DeviceCurve;

pub(crate) use tiles::TileLayout;
#[cfg(feature = "gpu")]
pub(crate) use tiles::{
    COVERAGE_CURVE_BYTES, COVERAGE_EDGE_BYTES, COVERAGE_FILL_BYTES, COVERAGE_PARAMS_BYTES,
    COVERAGE_TILE_SIZE,
};

/// Conservative device work units for de Casteljau evaluation and one generated edge write.
#[cfg(feature = "gpu")]
pub(crate) const CURVE_FLATTEN_WORK_PER_SEGMENT: u64 = 32;

/// Largest local coordinate magnitude that f32 edge records resolve to a small fraction of a byte.
const DEVICE_COORDINATE_LIMIT: f64 = 8192.0;

/// Most fills one frame may leave to the GPU.
const DEVICE_MAX_FILLS: usize = 1 << 15;

/// Most bounding-box pixels one frame may leave to the GPU, bounding the dispatch size.
const DEVICE_MAX_PIXELS: u64 = 1 << 24;

/// Most conservatively admitted tile coverage work units in one fill.
const DEVICE_MAX_FILL_VISITS: u64 = 1 << 26;

/// Most conservatively admitted tile coverage work units in one frame.
const DEVICE_MAX_FRAME_VISITS: u64 = 1 << 28;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DeviceGeometry {
    pub(crate) coverage_enabled: bool,
    pub(crate) device_fills: Vec<DeviceFill>,
    pub(crate) device_edges: Vec<[f64; 5]>,
    pub(crate) device_curves: Vec<DeviceCurve>,
    pub(crate) device_totals: [u64; 2],
    pub(crate) fallback_paths: GpuFallbackCounts,
    pub(crate) analytic_cpu_fallbacks: u64,
}

impl DeviceGeometry {
    /// Flattens a fill and records its edges for GPU coverage.
    ///
    /// Returns `false` without recording anything when the path needs CPU coverage: its
    /// coordinates exceed f32 resolution or its estimated GPU cost exceeds the frame allowance.
    pub(super) fn device_fill(
        &mut self,
        scene: &mut PreparedScene,
        geometry: &super::frame_ir::IrPath<'_>,
        tolerance: f64,
        scope: &mut RasterScope<'_>,
    ) -> Result<bool, ValidationError> {
        if geometry.device.iter().any(|verb| {
            matches!(
                verb,
                motion_core::PathVerb::QuadraticTo(..) | motion_core::PathVerb::CubicTo(..)
            )
        }) {
            return self.curve_fill(scene, geometry, tolerance, scope);
        }
        let (budget, path) = (&mut *scope.budget, scope.path);
        let edges = path_edges(
            geometry.device.iter().copied(),
            Transform::IDENTITY,
            tolerance,
            budget,
            path,
        )?;
        debug_assert!(edges.len() as u64 <= geometry.segment_upper_bound);
        let charged = (edges.len() * size_of::<Edge>()) as u64;
        let bounds = edge_bounds(&edges);
        let pixels = pixel_bounds(bounds, scene.size);
        if edges.is_empty() || pixels.is_empty() {
            budget.release(charged);
            return Ok(true);
        }
        let area = pixels.pixels();
        let within = |value: f64| value.abs() <= DEVICE_COORDINATE_LIMIT;
        let [used_pixels, used_visits] = self.device_totals;
        if !edges.iter().all(|edge| {
            within(edge.top.x - f64::from(pixels.left))
                && within(edge.bottom.x - f64::from(pixels.left))
                && within(edge.top.y - f64::from(pixels.top))
                && within(edge.bottom.y - f64::from(pixels.top))
        }) {
            self.fallback_paths.coordinate_range += 1;
            budget.release(charged);
            return Ok(false);
        }
        if self.device_fills.len() >= DEVICE_MAX_FILLS || used_pixels + area > DEVICE_MAX_PIXELS {
            self.fallback_paths.frame_limit += 1;
            budget.release(charged);
            return Ok(false);
        }
        let Some(tiles) = TileLayout::admit(&edges, pixels, budget, path)? else {
            self.fallback_paths.tile_capacity += 1;
            budget.release(charged);
            return Ok(false);
        };
        let visits = tiles.work;
        if visits > DEVICE_MAX_FILL_VISITS || used_visits + visits > DEVICE_MAX_FRAME_VISITS {
            self.fallback_paths.work_limit += 1;
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
            .map_or(0, |fill| Self::device_sample_end(&scene.strips[fill.strip]));
        let strip = Strip {
            x: pixels.left,
            y: pixels.top,
            width: pixels.width(),
            height: pixels.height(),
            alpha_offset,
        };
        budget.push(
            &mut self.device_fills,
            DeviceFill {
                strip: scene.strips.len(),
                edges: first..self.device_edges.len(),
                curves: 0..0,
                segments: 0,
                rule: geometry.rule,
                tiles,
            },
            path,
        )?;
        budget.push(&mut scene.strips, strip, path)?;
        self.device_totals = [used_pixels + area, used_visits + visits];
        Ok(true)
    }

    /// First sample after a device strip's f32 coverage block.
    pub(crate) fn device_sample_end(strip: &Strip) -> usize {
        strip.alpha_offset + (strip.width * strip.height) as usize
    }

    #[cfg(feature = "gpu")]
    pub(crate) fn stats(&self, preparation: crate::PreparationStats) -> GpuPreparationStats {
        GpuPreparationStats {
            preparation,
            segment_count: self.device_edges.len() as u64
                + self
                    .device_fills
                    .iter()
                    .map(|fill| u64::from(fill.segments))
                    .sum::<u64>(),
            tile_count: self
                .device_fills
                .iter()
                .map(|fill| fill.tiles.tile_count())
                .sum(),
            tile_index_capacity: self
                .device_fills
                .iter()
                .map(|fill| u64::from(fill.tiles.indices))
                .sum(),
            geometry_upload_bytes: if self.device_fills.is_empty() {
                0
            } else {
                self.device_fills.len() as u64 * COVERAGE_FILL_BYTES
                    + self.device_edges.len() as u64 * COVERAGE_EDGE_BYTES
                    + self.device_curves.len() as u64 * COVERAGE_CURVE_BYTES
                    + COVERAGE_PARAMS_BYTES
            },
            analytic_cpu_fallbacks: self.analytic_cpu_fallbacks,
            fallback_paths: self.fallback_paths,
        }
    }

    /// Replaces every device fill with CPU strips, spans and alphas for CPU rendering.
    ///
    /// Strips are appended and the owning items repointed, so earlier indices stay valid.
    #[cfg(test)]
    pub(crate) fn materialize_device_coverage(
        &mut self,
        scene: &mut PreparedScene,
        limits: RasterLimits,
    ) -> Result<(), ValidationError> {
        if self.device_fills.is_empty() {
            return Ok(());
        }
        let fills = std::mem::take(&mut self.device_fills);
        let device_edges = std::mem::take(&mut self.device_edges);
        let mut budget = motion_core::budget::Budget::new(scene.size, limits)?;
        budget.bytes(scene.stats.retained_bytes, "gpu.materialize")?;
        let mut scratch = PolygonScratch::default();
        let mut next = fills.iter().peekable();
        for index in 0..scene.items.len() {
            let PreparedItem::Fill { strips, spans, .. } = &scene.items[index] else {
                continue;
            };
            let Some(fill) = next.next_if(|fill| strips.len() == 1 && strips.start == fill.strip)
            else {
                continue;
            };
            debug_assert!(spans.is_empty());
            let mut edges = Vec::new();
            if !fill.curves.is_empty() {
                let mut verbs = Vec::new();
                let curves = &self.device_curves[fill.curves.clone()];
                let mut current = None;
                for curve in curves {
                    if current != Some(curve.points[0]) {
                        budget.push(
                            &mut verbs,
                            motion_core::PathVerb::MoveTo(curve.points[0]),
                            "gpu.materialize",
                        )?;
                    }
                    let [_, first, second, end] = curve.points;
                    budget.push(
                        &mut verbs,
                        if curve.segments == 1 {
                            motion_core::PathVerb::LineTo(end)
                        } else {
                            motion_core::PathVerb::CubicTo(first, second, end)
                        },
                        "gpu.materialize",
                    )?;
                    current = Some(end);
                }
                edges = path_edges(
                    verbs.iter().copied(),
                    Transform::IDENTITY,
                    curves[0].tolerance,
                    &mut budget,
                    "gpu.materialize",
                )?;
                budget.release(std::mem::size_of_val(verbs.as_slice()) as u64);
            }
            for edge in &device_edges[fill.edges.clone()] {
                budget.push(
                    &mut edges,
                    Edge {
                        top: Point::new(edge[0], edge[1]),
                        bottom: Point::new(edge[2], edge[3]),
                        winding: edge[4] as i32,
                    },
                    "gpu.materialize",
                )?;
            }
            let (strip_start, span_start) = (scene.strips.len(), scene.spans.len());
            scene.polygon_edges(
                &edges,
                fill.rule,
                &mut scratch,
                &mut RasterScope::new(&mut budget, "gpu.device_fill"),
            )?;
            budget.release((edges.len() * size_of::<Edge>()) as u64);
            if let PreparedItem::Fill { strips, spans, .. } = &mut scene.items[index] {
                *strips = strip_start..scene.strips.len();
                *spans = span_start..scene.spans.len();
            }
        }
        scene.stats.retained_bytes = budget.live_bytes();
        scene.stats.peak_bytes = scene.stats.peak_bytes.max(budget.peak_bytes());
        scene.stats.work += budget.work_done();
        Ok(())
    }
}
