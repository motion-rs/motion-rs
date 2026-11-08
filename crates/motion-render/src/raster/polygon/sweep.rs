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
    fn new(
        edges: &'edges [Edge],
        budget: &mut Budget,
        path: &str,
    ) -> Result<Self, ValidationError> {
        budget.work(edges.len() as u64, path)?;
        let mut starts = scratch::<usize>(edges.len() as u64, budget, path)?;
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
            live: Vec::new(),
            cursor: 0,
        })
    }

    fn band(
        &mut self,
        row: u32,
        height: u32,
        budget: &mut Budget,
        path: &str,
    ) -> Result<Vec<Edge>, ValidationError> {
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
        let mut result = scratch::<Edge>(self.live.len() as u64, budget, path)?;
        result.extend(self.live.iter().map(|index| self.edges[*index]));
        Ok(result)
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

    fn release(self, budget: &mut Budget) {
        budget.release(((self.starts.len() + self.live.len()) * size_of::<usize>()) as u64);
    }
}

pub(crate) fn rasterize(
    frame: &mut PreparedFrame,
    edges: &[Edge],
    pixels: PixelRect,
    rule: FillRule,
    budget: &mut Budget,
    path: &str,
) -> Result<(), ValidationError> {
    let left = pixels.left / 4 * 4;
    let right = (u64::from(pixels.right).div_ceil(4) * 4).min(u64::from(frame.size.width)) as u32;
    let mut row = pixels.top / 4 * 4;
    let mut sweep = EdgeSweep::new(edges, budget, path)?;
    while row < pixels.bottom {
        let height = (frame.size.height - row).min(4);
        let active = sweep.band(row, height, budget, path)?;
        if active.is_empty() {
            row = sweep.next_row(row + height, pixels.bottom);
            continue;
        }
        let count = u64::from(right - left) * u64::from(height);
        budget.work(count, path)?;
        let mut areas = scratch::<f64>(count, budget, path)?;
        areas.resize(count as usize, 0.0);
        for vertical in row..row + height {
            integrate_row(
                &active,
                Band {
                    row: vertical,
                    left,
                    right,
                },
                rule,
                &mut areas[(vertical - row) as usize * (right - left) as usize..],
                &mut RasterScope::new(budget, path),
            )?;
        }
        #[cfg(test)]
        diagnostics::band(&active, row, height);
        frame.polygon_band(
            &mut areas,
            Band { row, left, right },
            height,
            &mut RasterScope::new(budget, path),
        )?;
        budget.release(count * size_of::<f64>() as u64 + (active.len() * size_of::<Edge>()) as u64);
        row += height;
    }
    sweep.release(budget);
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
