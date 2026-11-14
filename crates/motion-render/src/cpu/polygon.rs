//! Polygon path rasterization into sparse strips and spans using exact area coverage.

use crate::prepared::{FrameRecords, PixelRect, Span, Strip};
#[cfg(test)]
use crate::prepared::{PreparationStats, PreparedCpuFrame, PreparedItem, RenderPrepare};
use crate::raster::{Band, RasterScope, coverage_to_byte, pixel_bounds};
#[cfg(test)]
use motion_core::RasterLimits;
use motion_core::budget::Budget;
use motion_core::{FillRule, PathVerb, Point, Transform, ValidationError};
use std::mem::size_of;

mod intersections;
pub(crate) mod scratch;
pub(crate) mod shape_clip;
mod sweep;
#[cfg(test)]
mod tests;

pub use motion_core::geometry::flatten::{Edge, edge_bounds, path_edges};
use scratch::{ColumnScratch, PolygonScratch, RowScratch};

#[cfg(test)]
use kurbo::{CubicBez, ParamCurve};
#[cfg(test)]
use motion_core::geometry::flatten::{add_edge, flatten_curve};

#[derive(Clone, Copy)]
pub(super) struct Crossing {
    edge: Edge,
    horizontal: f64,
}

impl FrameRecords {
    pub(crate) fn polygon_path(
        &mut self,
        geometry: &motion_core::Path2D,
        transform: Transform,
        rule: FillRule,
        tolerance: f64,
        budget: &mut Budget,
        path: &str,
    ) -> Result<(), ValidationError> {
        self.polygon_path_with_scratch(
            geometry,
            transform,
            rule,
            tolerance,
            &mut PolygonScratch::default(),
            &mut RasterScope::new(budget, path),
        )
    }

    pub(super) fn polygon_path_with_scratch(
        &mut self,
        geometry: &motion_core::Path2D,
        transform: Transform,
        rule: FillRule,
        tolerance: f64,
        scratch: &mut PolygonScratch,
        scope: &mut RasterScope<'_>,
    ) -> Result<(), ValidationError> {
        let coefficients = transform.coefficients().map(f64::to_bits);
        if let Some(normalized) =
            super::coverage_cache::cache_transform(geometry.verbs(), coefficients, self.size)
        {
            let normalized = normalized.map(f64::from_bits);
            let dx = (f64::from_bits(coefficients[4]) - normalized[4]) as u32;
            let dy = (f64::from_bits(coefficients[5]) - normalized[5]) as u32;
            let strips = self.strips.len();
            let spans = self.spans.len();
            self.polygon_path_raw(
                geometry,
                Transform::from_coefficients(normalized),
                rule,
                tolerance,
                scratch,
                scope,
            )?;
            for strip in &mut self.strips[strips..] {
                strip.x += dx;
                strip.y += dy;
            }
            for span in &mut self.spans[spans..] {
                span.start_x += dx;
                span.end_x += dx;
                span.y += dy;
            }
            return Ok(());
        }
        self.polygon_path_raw(geometry, transform, rule, tolerance, scratch, scope)
    }

    fn polygon_path_raw(
        &mut self,
        geometry: &motion_core::Path2D,
        transform: Transform,
        rule: FillRule,
        tolerance: f64,
        scratch: &mut PolygonScratch,
        scope: &mut RasterScope<'_>,
    ) -> Result<(), ValidationError> {
        let (budget, path) = (&mut *scope.budget, scope.path);
        if let Some(bounds) = geometry.axis_aligned_rectangle() {
            let [xx, yx, xy, yy, _, _] = transform.coefficients();
            if (xy == 0.0 && yx == 0.0) || (xx == 0.0 && yy == 0.0) {
                let mapped = motion_core::display_list::mapped_bounds(bounds, transform, path)?;
                if [mapped.left, mapped.top, mapped.right, mapped.bottom]
                    .iter()
                    .all(|value| value.abs() <= 1048576.0 && value.fract() == 0.0)
                {
                    budget.work(geometry.verbs().len() as u64, path)?;
                    return self.rectangle(mapped, budget, path);
                }
            }
        }
        self.polygon_with_scratch(
            geometry.verbs().iter().copied(),
            transform,
            rule,
            tolerance,
            scratch,
            scope,
        )
    }

    pub(crate) fn polygon(
        &mut self,
        verbs: impl Iterator<Item = PathVerb>,
        transform: Transform,
        rule: FillRule,
        tolerance: f64,
        budget: &mut Budget,
        path: &str,
    ) -> Result<(), ValidationError> {
        self.polygon_with_scratch(
            verbs,
            transform,
            rule,
            tolerance,
            &mut PolygonScratch::default(),
            &mut RasterScope::new(budget, path),
        )
    }

    fn polygon_with_scratch(
        &mut self,
        verbs: impl Iterator<Item = PathVerb>,
        transform: Transform,
        rule: FillRule,
        tolerance: f64,
        scratch: &mut PolygonScratch,
        scope: &mut RasterScope<'_>,
    ) -> Result<(), ValidationError> {
        let (budget, path) = (&mut *scope.budget, scope.path);
        let edges = path_edges(verbs, transform, tolerance, budget, path)?;
        self.polygon_edges(&edges, rule, scratch, scope)?;
        scope
            .budget
            .release((edges.len() * size_of::<Edge>()) as u64);
        Ok(())
    }

    /// Rasterizes already flattened edges; the caller owns their storage charge.
    pub(crate) fn polygon_edges(
        &mut self,
        edges: &[Edge],
        rule: FillRule,
        scratch: &mut PolygonScratch,
        scope: &mut RasterScope<'_>,
    ) -> Result<(), ValidationError> {
        let (budget, path) = (&mut *scope.budget, scope.path);
        budget.work(edges.len() as u64, path)?;
        let pixels = pixel_bounds(edge_bounds(edges), self.size);
        if !pixels.is_empty() {
            sweep::rasterize_with_scratch(
                self,
                edges,
                pixels,
                rule,
                scratch,
                &mut RasterScope::new(budget, path),
            )?;
        }
        Ok(())
    }

    fn polygon_band(
        &mut self,
        areas: &mut [f64],
        band: Band,
        height: u32,
        scope: &mut RasterScope<'_>,
    ) -> Result<(), ValidationError> {
        let Band { row, left, right } = band;
        let (budget, path) = (&mut *scope.budget, scope.path);
        let width = right - left;
        let classify =
            |areas: &mut [f64], column: u32, budget: &mut Budget| -> Result<u8, ValidationError> {
                let end = column.saturating_add(4).min(right);
                budget.work(u64::from(end - column) * u64::from(height), path)?;
                let mut full = end - column == 4;
                let mut empty = true;
                for vertical in 0..height {
                    let offset = vertical as usize * width as usize + (column - left) as usize;
                    let count = (end - column) as usize;
                    let bytes: [u8; 4] = std::array::from_fn(|index| {
                        if index < count {
                            coverage_to_byte(areas[offset + index])
                        } else {
                            0
                        }
                    });
                    for (area, alpha) in areas[offset..offset + count].iter_mut().zip(bytes) {
                        *area = f64::from(alpha);
                        full &= alpha == 255;
                        empty &= alpha == 0;
                    }
                }
                Ok(if empty {
                    0
                } else if full {
                    2
                } else {
                    1
                })
            };
        let mut column = left;
        let mut next_kind = None;
        while column < right {
            let start = column;
            let kind = match next_kind.take() {
                Some(kind) => kind,
                None => classify(areas, column, budget)?,
            };
            column = column.saturating_add(4).min(right);
            while column < right {
                let candidate = classify(areas, column, budget)?;
                if candidate != kind {
                    next_kind = Some(candidate);
                    break;
                }
                column = column.saturating_add(4).min(right);
            }
            if kind == 0 {
                continue;
            } else if kind == 2 {
                budget.push(
                    &mut self.spans,
                    Span {
                        start_x: start,
                        end_x: column,
                        y: row,
                        height,
                    },
                    path,
                )?;
            } else {
                let count = u64::from(column - start) * u64::from(height);
                budget.work(count, path)?;
                budget.bytes(count, path)?;
                self.alphas.try_reserve(count as usize).map_err(|_| {
                    ValidationError::limit(path, "max_bytes", "Could not reserve coverage storage")
                })?;
                let alpha_offset = self.alphas.len();
                for vertical in 0..height {
                    for horizontal in start..column {
                        self.alphas.push(
                            areas[vertical as usize * width as usize + (horizontal - left) as usize]
                                as u8,
                        );
                    }
                }
                budget.push(
                    &mut self.strips,
                    Strip {
                        x: start,
                        y: row,
                        width: column - start,
                        height,
                        alpha_offset,
                        device: false,
                    },
                    path,
                )?;
            }
        }
        Ok(())
    }
}

fn scratch<Value>(
    count: u64,
    budget: &mut Budget,
    path: &str,
) -> Result<Vec<Value>, ValidationError> {
    let mut values = Vec::new();
    reuse_scratch(&mut values, count, budget, path)?;
    Ok(values)
}

fn reuse_scratch<Value>(
    values: &mut Vec<Value>,
    count: u64,
    budget: &mut Budget,
    path: &str,
) -> Result<(), ValidationError> {
    let bytes = count
        .checked_mul(size_of::<Value>() as u64)
        .ok_or_else(|| ValidationError::limit(path, "max_bytes", "Scratch storage overflow"))?;
    budget.bytes(bytes, path)?;
    let count = usize::try_from(count).map_err(|_| {
        ValidationError::limit(path, "max_bytes", "Scratch exceeds addressable storage")
    })?;
    values.clear();
    values.try_reserve_exact(count).map_err(|_| {
        ValidationError::limit(path, "max_bytes", "Could not reserve geometry scratch")
    })?;
    Ok(())
}

fn sort_work(count: usize, budget: &mut Budget, path: &str) -> Result<(), ValidationError> {
    budget.work(
        (count as u64)
            .saturating_mul(u64::from(count.max(1).ilog2()) + 1)
            .saturating_mul(16),
        path,
    )
}

#[cfg(test)]
fn integrate_row(
    edges: &[Edge],
    band: Band,
    rule: FillRule,
    areas: &mut [f64],
    scope: &mut RasterScope<'_>,
) -> Result<(), ValidationError> {
    integrate_row_with(edges, band, rule, areas, &mut RowScratch::default(), scope)
}

fn integrate_row_with(
    edges: &[Edge],
    band: Band,
    rule: FillRule,
    areas: &mut [f64],
    scratch: &mut RowScratch,
    scope: &mut RasterScope<'_>,
) -> Result<(), ValidationError> {
    intersections::fill_row_cuts(
        edges,
        band.row,
        band.right,
        true,
        scratch,
        scope.budget,
        scope.path,
    )?;
    integrate_cuts(edges, band, rule, areas, scratch, CutMode::Merged, scope)
}

#[cfg(test)]
fn integrate_row_adjacent(
    edges: &[Edge],
    band: Band,
    rule: FillRule,
    areas: &mut [f64],
    scope: &mut RasterScope<'_>,
) -> Result<(), ValidationError> {
    let mut scratch = RowScratch {
        cuts: intersections::row_cuts(edges, band.row, band.right, scope.budget, scope.path)?,
        ..RowScratch::default()
    };
    integrate_cuts(
        edges,
        band,
        rule,
        areas,
        &mut scratch,
        CutMode::Adjacent,
        scope,
    )
}

#[cfg(test)]
fn integrate_row_reference(
    edges: &[Edge],
    band: Band,
    rule: FillRule,
    areas: &mut [f64],
    scope: &mut RasterScope<'_>,
) -> Result<(), ValidationError> {
    let mut scratch = RowScratch {
        cuts: intersections::row_cuts_reference(
            edges,
            band.row,
            band.right,
            scope.budget,
            scope.path,
        )?,
        ..RowScratch::default()
    };
    integrate_cuts(
        edges,
        band,
        rule,
        areas,
        &mut scratch,
        CutMode::Reference,
        scope,
    )
}

/// How adjacent inside regions of one row interval are integrated.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
enum CutMode {
    /// Consecutive inside crossings form one trapezoid; the production path.
    Merged,
    /// One trapezoid per pair of crossings, for comparison against `Merged`.
    Adjacent,
    /// Per-pair integration with the reference cut generator.
    Reference,
}

fn integrate_cuts(
    edges: &[Edge],
    band: Band,
    rule: FillRule,
    areas: &mut [f64],
    scratch: &mut RowScratch,
    mode: CutMode,
    scope: &mut RasterScope<'_>,
) -> Result<(), ValidationError> {
    let (budget, path) = (&mut *scope.budget, scope.path);
    let RowScratch {
        cuts,
        crossings,
        columns,
        ..
    } = scratch;
    let top = f64::from(band.row);
    sort_work(cuts.len(), budget, path)?;
    cuts.sort_unstable_by(f64::total_cmp);
    reuse_scratch(crossings, edges.len() as u64, budget, path)?;
    for interval in cuts.windows(2) {
        let [start, end] = [interval[0], interval[1]];
        budget.work(1, path)?;
        if start == end {
            continue;
        }
        crossings.clear();
        budget.work(edges.len() as u64, path)?;
        for edge in edges {
            if edge.top.y <= start && edge.bottom.y >= end {
                crossings.push(Crossing {
                    edge: *edge,
                    horizontal: edge.at(start) * 0.5 + edge.at(end) * 0.5,
                });
            }
        }
        sort_work(crossings.len(), budget, path)?;
        crossings.sort_unstable_by(|first, second| first.horizontal.total_cmp(&second.horizontal));
        let mut winding = 0i32;
        let mut previous: Option<Edge> = None;
        for crossing in crossings.iter() {
            budget.work(1, path)?;
            let inside = match rule {
                FillRule::NonZero => winding != 0,
                FillRule::EvenOdd => winding % 2 != 0,
            };
            let next_winding = winding + crossing.edge.winding;
            let next_inside = match rule {
                FillRule::NonZero => next_winding != 0,
                FillRule::EvenOdd => next_winding % 2 != 0,
            };
            if inside && (mode != CutMode::Merged || !next_inside) {
                #[cfg(test)]
                if mode == CutMode::Reference {
                    integrate_trapezoid_reference(
                        previous.expect("inside span follows a crossing"),
                        crossing.edge,
                        start - top,
                        end - top,
                        band,
                        areas,
                        &mut RasterScope::new(budget, path),
                    )?;
                    winding += crossing.edge.winding;
                    previous = Some(crossing.edge);
                    continue;
                }
                integrate_trapezoid(
                    previous.expect("inside span follows a crossing"),
                    crossing.edge,
                    start - top,
                    end - top,
                    band,
                    areas,
                    columns,
                    &mut RasterScope::new(budget, path),
                )?;
            }
            winding = next_winding;
            if mode != CutMode::Merged || !inside || !next_inside {
                previous = Some(crossing.edge);
            }
        }
        if winding != 0 {
            return Err(ValidationError::invalid(
                path,
                "geometry",
                "Unbalanced linear contour winding",
            ));
        }
    }
    budget.release((edges.len() * size_of::<Crossing>() + cuts.len() * size_of::<f64>()) as u64);
    Ok(())
}

/// Signed cover and area of the segments that cross one row of columns `left..right`.
/// Everything left of `left` accumulates in `backdrop`; a segment is split at every
/// integer column boundary so each piece stays inside one column.
struct Columns<'a> {
    left: u32,
    right: u32,
    backdrop: f64,
    cover: &'a mut [f64],
    area: &'a mut [f64],
}

impl<'a> Columns<'a> {
    fn new(left: u32, right: u32, cover: &'a mut [f64], area: &'a mut [f64]) -> Self {
        Self {
            left,
            right,
            backdrop: 0.0,
            cover,
            area,
        }
    }

    fn piece(&mut self, begin: Point, finish: Point) {
        let height = finish.y - begin.y;
        let middle = begin.x * 0.5 + finish.x * 0.5;
        if middle < f64::from(self.left) {
            self.backdrop += height;
        } else if middle < f64::from(self.right) {
            let column = middle.floor() as u32;
            let index = (column - self.left) as usize;
            self.cover[index] += height;
            self.area[index] += height
                * (1.0 - ((begin.x - f64::from(column)) + (finish.x - f64::from(column))) * 0.5);
        }
    }

    fn segment(
        &mut self,
        begin: Point,
        finish: Point,
        budget: &mut Budget,
        path: &str,
    ) -> Result<(), ValidationError> {
        let lower = begin
            .x
            .min(finish.x)
            .ceil()
            .clamp(f64::from(self.left), f64::from(self.right)) as u32;
        let upper = begin
            .x
            .max(finish.x)
            .floor()
            .clamp(f64::from(self.left), f64::from(self.right)) as u32;
        let mut previous = begin;
        if begin.x != finish.x && lower <= upper {
            budget.work(u64::from(upper - lower + 1), path)?;
            let mut visit = |columns: &mut Self, boundary: u32| {
                let progress =
                    ((f64::from(boundary) - begin.x) / (finish.x - begin.x)).clamp(0.0, 1.0);
                let current = Point::new(
                    f64::from(boundary),
                    begin.y * (1.0 - progress) + finish.y * progress,
                );
                columns.piece(previous, current);
                previous = current;
            };
            if finish.x > begin.x {
                for boundary in lower..=upper {
                    visit(self, boundary);
                }
            } else {
                for boundary in (lower..=upper).rev() {
                    visit(self, boundary);
                }
            }
        }
        self.piece(previous, finish);
        Ok(())
    }

    /// Turns the accumulated area into per-column coverage in `0..=limit`.
    fn resolve(&mut self, limit: f64) {
        for (area, cover) in self.area.iter_mut().zip(&*self.cover) {
            *area = (self.backdrop + *area).clamp(0.0, limit);
            self.backdrop += cover;
        }
    }
}

#[expect(clippy::too_many_arguments)]
fn integrate_trapezoid(
    left_edge: Edge,
    right_edge: Edge,
    start: f64,
    end: f64,
    band: Band,
    areas: &mut [f64],
    columns: &mut ColumnScratch,
    scope: &mut RasterScope<'_>,
) -> Result<(), ValidationError> {
    let Band { left, right, .. } = band;
    let row = f64::from(band.row);
    let (budget, path) = (&mut *scope.budget, scope.path);
    let first_x = left_edge.at(row + start).min(left_edge.at(row + end));
    let last_x = right_edge.at(row + start).max(right_edge.at(row + end));
    let first = first_x.floor().clamp(f64::from(left), f64::from(right)) as u32;
    let last = last_x.ceil().clamp(f64::from(left), f64::from(right)) as u32;
    if first >= last {
        return Ok(());
    }
    let width = (last - first) as usize;
    budget.work(width as u64 * 3, path)?;
    let ColumnScratch { area, cover } = columns;
    reuse_scratch(area, width as u64, budget, path)?;
    reuse_scratch(cover, width as u64, budget, path)?;
    area.resize(width, 0.0);
    cover.resize(width, 0.0);
    let mut columns = Columns::new(first, last, cover, area);
    for (begin, finish) in [
        (
            Point::new(left_edge.at(row + start), start),
            Point::new(left_edge.at(row + end), end),
        ),
        (
            Point::new(right_edge.at(row + end), end),
            Point::new(right_edge.at(row + start), start),
        ),
    ] {
        columns.segment(begin, finish, budget, path)?;
    }
    columns.resolve(end - start);
    for (target, value) in areas[(first - left) as usize..].iter_mut().zip(area.iter()) {
        *target += value;
    }
    budget.release(width as u64 * 16);
    Ok(())
}

#[cfg(test)]
fn integrate_trapezoid_reference(
    left_edge: Edge,
    right_edge: Edge,
    start: f64,
    end: f64,
    band: Band,
    areas: &mut [f64],
    scope: &mut RasterScope<'_>,
) -> Result<(), ValidationError> {
    let Band { left, right, .. } = band;
    let row = f64::from(band.row);
    let (budget, path) = (&mut *scope.budget, scope.path);
    let corners = [
        Point::new(left_edge.at(row + start), start),
        Point::new(right_edge.at(row + start), start),
        Point::new(right_edge.at(row + end), end),
        Point::new(left_edge.at(row + end), end),
    ];
    let minimum = corners
        .iter()
        .map(|point| point.x)
        .fold(f64::INFINITY, f64::min);
    let maximum = corners
        .iter()
        .map(|point| point.x)
        .fold(f64::NEG_INFINITY, f64::max);
    let first = minimum.floor().clamp(f64::from(left), f64::from(right)) as u32;
    let last = maximum.ceil().clamp(f64::from(left), f64::from(right)) as u32;
    budget.work(u64::from(last - first), path)?;
    for column in first..last {
        let mut clipped = [Point::ORIGIN; 8];
        let count = clip(&corners, &mut clipped, f64::from(column), true);
        let mut polygon = [Point::ORIGIN; 8];
        let count = clip(
            &clipped[..count],
            &mut polygon,
            f64::from(column) + 1.0,
            false,
        );
        let mut area = 0.0;
        if count > 0 {
            let origin = polygon[0];
            for index in 1..count - 1 {
                let first = polygon[index] - origin;
                let second = polygon[index + 1] - origin;
                area += first.x * second.y - first.y * second.x;
            }
        }
        areas[(column - left) as usize] += area.abs() * 0.5;
    }
    Ok(())
}

#[cfg(test)]
fn clip(input: &[Point], output: &mut [Point; 8], boundary: f64, keep_right: bool) -> usize {
    let mut count = 0;
    if let Some(mut previous) = input.last().copied() {
        let inside = |point: Point| {
            if keep_right {
                point.x >= boundary
            } else {
                point.x <= boundary
            }
        };
        for &current in input {
            if inside(previous) != inside(current) {
                let fraction = ((boundary - previous.x) / (current.x - previous.x)).clamp(0.0, 1.0);
                output[count] = Point::new(
                    boundary,
                    previous.y * (1.0 - fraction) + current.y * fraction,
                );
                count += 1;
            }
            if inside(current) {
                output[count] = current;
                count += 1;
            }
            previous = current;
        }
    }
    count
}
