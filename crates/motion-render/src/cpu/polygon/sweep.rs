//! Band-wise edge sweep that limits polygon rasterization to edges live in each band.

use super::*;
use motion_core::ValidationError;

#[cfg(test)]
mod tests;

struct EdgeSweep<'edges> {
    edges: &'edges [Edge],
    starts: Vec<usize>,
    live: Vec<usize>,
    cursor: usize,
}

impl<'edges> EdgeSweep<'edges> {
    #[cfg(test)]
    fn new(
        edges: &'edges [Edge],
        budget: &mut Budget,
        path: &str,
    ) -> Result<Self, ValidationError> {
        Self::with_buffers(edges, Vec::new(), Vec::new(), budget, path)
    }

    /// Builds the sweep inside reused vectors; `finish` hands them back.
    fn with_buffers(
        edges: &'edges [Edge],
        mut starts: Vec<usize>,
        mut live: Vec<usize>,
        budget: &mut Budget,
        path: &str,
    ) -> Result<Self, ValidationError> {
        budget.work(edges.len() as u64, path)?;
        reuse_scratch(&mut starts, edges.len() as u64, budget, path)?;
        live.clear();
        starts.extend(0..edges.len());
        sort_work(starts.len(), budget, path)?;
        starts.sort_unstable_by(|first, second| {
            edges[*first]
                .top
                .y
                .total_cmp(&edges[*second].top.y)
                .then(first.cmp(second))
        });
        Ok(Self {
            edges,
            starts,
            live,
            cursor: 0,
        })
    }

    fn band(
        &mut self,
        row: u32,
        height: u32,
        budget: &mut Budget,
        path: &str,
        active: &mut Vec<Edge>,
    ) -> Result<(), ValidationError> {
        budget.work(self.live.len() as u64, path)?;
        let previous = self.live.len();
        self.live
            .retain(|index| self.edges[*index].bottom.y > f64::from(row));
        budget.release(((previous - self.live.len()) * size_of::<usize>()) as u64);
        let mut added = false;
        let remaining = &self.starts[self.cursor..];
        let entering =
            remaining.partition_point(|index| self.edges[*index].top.y < f64::from(row + height));
        let inspected = entering + usize::from(entering < remaining.len());
        let admitted = budget.work(inspected as u64, path).is_ok();
        while let Some(&index) = self.starts.get(self.cursor) {
            if !admitted {
                budget.work(1, path)?;
            }
            let edge = self.edges[index];
            if edge.top.y >= f64::from(row + height) {
                break;
            }
            self.cursor += 1;
            if edge.bottom.y > f64::from(row) {
                budget.push(&mut self.live, index, path)?;
                added = true;
            }
        }
        if added {
            sort_work(self.live.len(), budget, path)?;
            self.live.sort_unstable();
        }
        budget.work(self.live.len() as u64, path)?;
        reuse_scratch(active, self.live.len() as u64, budget, path)?;
        active.extend(self.live.iter().map(|index| self.edges[*index]));
        Ok(())
    }

    fn next_row(&self, minimum: u32, bottom: u32) -> u32 {
        self.starts.get(self.cursor).map_or(bottom, |index| {
            (self.edges[*index]
                .top
                .y
                .floor()
                .clamp(0.0, f64::from(bottom)) as u32
                / 4
                * 4)
            .max(minimum)
        })
    }

    fn finish(self, budget: &mut Budget) -> (Vec<usize>, Vec<usize>) {
        budget.release(((self.starts.len() + self.live.len()) * size_of::<usize>()) as u64);
        (self.starts, self.live)
    }
}

#[cfg(test)]
pub(crate) fn rasterize(
    frame: &mut FrameRecords,
    edges: &[Edge],
    pixels: PixelRect,
    rule: FillRule,
    budget: &mut Budget,
    path: &str,
) -> Result<(), ValidationError> {
    rasterize_with_scratch(
        frame,
        edges,
        pixels,
        rule,
        &mut PolygonScratch::default(),
        &mut RasterScope::new(budget, path),
    )
}

pub(super) fn rasterize_with_scratch(
    frame: &mut FrameRecords,
    edges: &[Edge],
    pixels: PixelRect,
    rule: FillRule,
    scratch: &mut PolygonScratch,
    scope: &mut RasterScope<'_>,
) -> Result<(), ValidationError> {
    let (budget, path) = (&mut *scope.budget, scope.path);
    let PolygonScratch {
        areas,
        starts,
        live,
        active,
        row: row_scratch,
    } = scratch;
    let left = pixels.left / 4 * 4;
    let right = (u64::from(pixels.right).div_ceil(4) * 4).min(u64::from(frame.size.width)) as u32;
    let mut row = pixels.top / 4 * 4;
    let mut sweep = EdgeSweep::with_buffers(
        edges,
        std::mem::take(starts),
        std::mem::take(live),
        budget,
        path,
    )?;
    while row < pixels.bottom {
        let height = (frame.size.height - row).min(4);
        sweep.band(row, height, budget, path, active)?;
        if active.is_empty() {
            row = sweep.next_row(row + height, pixels.bottom);
            continue;
        }
        let count = u64::from(right - left) * u64::from(height);
        budget.work(count, path)?;
        reuse_scratch(areas, count, budget, path)?;
        areas.resize(count as usize, 0.0);
        for vertical in row..row + height {
            integrate_row_with(
                active,
                Band {
                    row: vertical,
                    left,
                    right,
                },
                rule,
                &mut areas[(vertical - row) as usize * (right - left) as usize..],
                row_scratch,
                &mut RasterScope::new(budget, path),
            )?;
        }
        #[cfg(test)]
        diagnostics::band(active, row, height);
        frame.polygon_band(
            areas,
            Band { row, left, right },
            height,
            &mut RasterScope::new(budget, path),
        )?;
        budget.release(count * size_of::<f64>() as u64 + (active.len() * size_of::<Edge>()) as u64);
        row += height;
    }
    (*starts, *live) = sweep.finish(budget);
    Ok(())
}

#[cfg(test)]
pub(super) mod diagnostics {
    use std::cell::Cell;

    #[derive(Clone, Copy, Debug, Default)]
    pub(in super::super) struct Totals {
        pub bands: u64,
        pub crossing_bands: u64,
        pub rows: u64,
        pub crossing_rows: u64,
    }

    thread_local! {
        static TOTALS: Cell<Option<Totals>> = const { Cell::new(None) };
    }

    pub(in super::super) fn row(crossing: bool) {
        TOTALS.with(|value| {
            if let Some(mut totals) = value.get() {
                totals.rows += 1;
                totals.crossing_rows += u64::from(crossing);
                value.set(Some(totals));
            }
        });
    }

    pub(super) fn band(edges: &[super::Edge], row: u32, height: u32) {
        TOTALS.with(|value| {
            if let Some(mut totals) = value.get() {
                let crossing = edges.iter().enumerate().any(|(index, first)| {
                    edges[..index].iter().any(|second| {
                        let start = f64::from(row).max(first.top.y).max(second.top.y);
                        let end = f64::from(row + height)
                            .min(first.bottom.y)
                            .min(second.bottom.y);
                        if start >= end {
                            return false;
                        }
                        let before = first.at(start) - second.at(start);
                        let after = first.at(end) - second.at(end);
                        (before < 0.0 && after > 0.0) || (before > 0.0 && after < 0.0)
                    })
                });
                totals.bands += 1;
                totals.crossing_bands += u64::from(crossing);
                value.set(Some(totals));
            }
        });
    }

    pub(in super::super) fn measure(action: impl FnOnce()) -> Totals {
        TOTALS.with(|value| value.set(Some(Totals::default())));
        action();
        TOTALS.with(|value| value.take().unwrap())
    }
}
