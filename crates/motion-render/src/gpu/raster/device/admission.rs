//! Conservative edge envelopes and intersection work admission for device boundary integration.

use super::tiles::{COVERAGE_TILE_SIZE, MAX_TILE_EDGES, TileLayout};
use crate::prepared::PixelRect;
use motion_core::budget::Budget;
use motion_core::{Point, ValidationError};

/// Bounds f32 interpolation error within the admitted 8192-pixel coordinate range.
const PROJECTION_PADDING: f64 = 1.0 / 64.0;

#[derive(Clone, Copy)]
pub(super) struct Envelope {
    points: [Point; 4],
    line: bool,
}

impl Envelope {
    pub(super) fn new(points: [Point; 4], line: bool, bounds: PixelRect) -> Self {
        Self {
            points: points.map(|point| {
                Point::new(
                    f64::from((point.x - f64::from(bounds.left)) as f32),
                    f64::from((point.y - f64::from(bounds.top)) as f32),
                )
            }),
            line,
        }
    }

    fn row_bounds(self, top: f64, bottom: f64) -> Option<[f64; 4]> {
        let padding = if self.line { 0.0 } else { PROJECTION_PADDING };
        let minimum_y = self
            .points
            .iter()
            .map(|p| p.y)
            .fold(f64::INFINITY, f64::min)
            - padding;
        let maximum_y = self
            .points
            .iter()
            .map(|p| p.y)
            .fold(f64::NEG_INFINITY, f64::max)
            + padding;
        let top = top.max(minimum_y);
        let bottom = bottom.min(maximum_y);
        if top >= bottom {
            return None;
        }
        let (left, right) = if self.line {
            let first = self.at(top);
            let last = self.at(bottom);
            (first.min(last), first.max(last))
        } else {
            (
                self.points
                    .iter()
                    .map(|p| p.x)
                    .fold(f64::INFINITY, f64::min),
                self.points
                    .iter()
                    .map(|p| p.x)
                    .fold(f64::NEG_INFINITY, f64::max),
            )
        };
        Some([
            left - PROJECTION_PADDING,
            top,
            right + PROJECTION_PADDING,
            bottom,
        ])
    }

    fn at(self, y: f64) -> f64 {
        let [first, _, _, last] = self.points;
        first.x + (last.x - first.x) * ((y - first.y) / (last.y - first.y))
    }

    fn may_intersect(self, other: Self, a: [f64; 4], b: [f64; 4]) -> bool {
        let start = a[1].max(b[1]);
        let end = a[3].min(b[3]);
        if start >= end || a[2] < b[0] || b[2] < a[0] {
            return false;
        }
        if !self.line || !other.line {
            return true;
        }
        let first = self.at(start) - other.at(start);
        let last = self.at(end) - other.at(end);
        !(first.min(last) > PROJECTION_PADDING || first.max(last) < -PROJECTION_PADDING)
    }
}

pub(super) fn admit(
    edges: &[Envelope],
    bounds: PixelRect,
    budget: &mut Budget,
    path: &str,
) -> Result<Option<TileLayout>, ValidationError> {
    let columns = bounds.width().div_ceil(COVERAGE_TILE_SIZE);
    let rows = bounds.height().div_ceil(COVERAGE_TILE_SIZE);
    let tile_count = u64::from(columns) * u64::from(rows);
    let headers = tile_count * size_of::<Vec<usize>>() as u64;
    budget.bytes(headers, path)?;
    budget.work(tile_count + edges.len() as u64, path)?;
    let mut tiles = Vec::new();
    tiles.try_reserve_exact(tile_count as usize).map_err(|_| {
        ValidationError::limit(
            path,
            "max_bytes",
            "Could not reserve boundary tile admission",
        )
    })?;
    tiles.resize_with(tile_count as usize, Vec::<usize>::new);
    let mut indices = 0_u64;
    let mut crowded = false;
    for (index, edge) in edges.iter().enumerate() {
        for row in 0..rows {
            budget.work(1, path)?;
            let Some(rect) = edge.row_bounds(
                f64::from(row * COVERAGE_TILE_SIZE),
                f64::from((row + 1) * COVERAGE_TILE_SIZE),
            ) else {
                continue;
            };
            // Every left crossing participates in winding classification, including
            // offscreen contours. One seam tile covers device interpolation rounding.
            let left = ((rect[0] / f64::from(COVERAGE_TILE_SIZE))
                .floor()
                .clamp(0.0, f64::from(columns - 1)) as u32)
                .saturating_sub(1);
            for column in left..columns {
                let tile = &mut tiles[(row * columns + column) as usize];
                budget.push(tile, index, path)?;
                indices += 1;
                crowded |= tile.len() > MAX_TILE_EDGES as usize;
            }
        }
    }
    let scratch = headers + indices * size_of::<usize>() as u64;
    if crowded || indices > u64::from(u32::MAX) {
        budget.release(scratch);
        return Ok(None);
    }
    let mut work = indices * 2 + tile_count * 96 + bounds.pixels();
    for (tile_index, tile) in tiles.iter().enumerate() {
        let row_bytes = tile.len() as u64 * size_of::<(usize, [f64; 4])>() as u64;
        budget.bytes(row_bytes, path)?;
        let mut active = Vec::new();
        active.try_reserve_exact(tile.len()).map_err(|_| {
            ValidationError::limit(
                path,
                "max_bytes",
                "Could not reserve boundary row admission",
            )
        })?;
        let band = tile_index as u32 / columns * COVERAGE_TILE_SIZE;
        for row in band..(band + COVERAGE_TILE_SIZE).min(bounds.height()) {
            let top = f64::from(row);
            let bottom = top + 1.0;
            active.clear();
            budget.work(tile.len() as u64, path)?;
            for &index in tile {
                if let Some(rect) = edges[index].row_bounds(top, bottom) {
                    active.push((index, rect));
                }
            }
            let mut intervals = active.len() as u64;
            for (position, &(index, rect)) in active.iter().enumerate() {
                budget.work(position as u64, path)?;
                for &(other, other_rect) in &active[..position] {
                    if edges[index].may_intersect(edges[other], rect, other_rect) {
                        // A pair contributes at most two cuts to each directed edge:
                        // either a proper crossing or the ends of a coincident overlap.
                        intervals += 4;
                    }
                }
            }
            work = work.saturating_add(
                intervals.saturating_mul(2 * tile.len() as u64 + u64::from(COVERAGE_TILE_SIZE)),
            );
            work = work.saturating_add(tile.len() as u64 + u64::from(COVERAGE_TILE_SIZE));
        }
        budget.release(row_bytes);
    }
    budget.release(scratch);
    Ok(Some(TileLayout {
        columns,
        rows,
        indices: indices as u32,
        work,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::{PixelSize, RasterLimits};

    #[test]
    fn boundary_admission_enforces_exact_work_and_byte_limits() {
        let bounds = PixelRect {
            left: 0,
            top: 0,
            right: 32,
            bottom: 17,
        };
        let line = |a, b| Envelope::new([a, a, b, b], true, bounds);
        let edges = [
            line(Point::new(-3.0, 0.25), Point::new(30.0, 16.75)),
            line(Point::new(30.0, 0.25), Point::new(-3.0, 16.75)),
            line(Point::new(-3.0, 0.25), Point::new(30.0, 16.75)),
        ];
        let run = |limits| {
            let mut budget = Budget::new(PixelSize::new(32, 17), limits).unwrap();
            let result = admit(&edges, bounds, &mut budget, "boundary");
            (result, budget)
        };
        let (expected, budget) = run(RasterLimits::default());
        let expected = expected.unwrap().unwrap();
        assert!(expected.work > u64::from(expected.indices) * 256);
        let exact = RasterLimits {
            max_bytes: budget.peak_bytes(),
            max_work: budget.work_done(),
            ..RasterLimits::default()
        };
        let (actual, completed) = run(exact);
        assert_eq!(actual.unwrap(), Some(expected));
        assert_eq!(completed.live_bytes(), 0);
        for limits in [
            RasterLimits {
                max_bytes: exact.max_bytes - 1,
                ..exact
            },
            RasterLimits {
                max_work: exact.max_work - 1,
                ..exact
            },
        ] {
            assert!(run(limits).0.is_err());
        }
    }
}
