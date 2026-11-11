//! GPU coverage records: per-fill descriptors, directed edges and per-row edge lists.

use super::*;
use motion_core::{FillRule, ValidationError};

/// Words in one fill descriptor; the layout is shared with `coverage.wgsl`.
pub(in crate::gpu) const FILL_WORDS: usize = 8;

/// Fills, edges and row lists the coverage pass turns into frame alphas.
#[derive(Default)]
pub(in crate::gpu) struct DeviceCoverage {
    /// Per fill: x, y, width, height, coverage word, row table start, rule, first sample.
    pub(in crate::gpu) fills: Vec<u32>,
    /// Per edge: x0, y0, x1, y1 relative to the owning fill's origin, directed by winding.
    pub(in crate::gpu) edges: Vec<f32>,
    /// Per fill: height + 1 absolute offsets, then the edge indices crossing each row.
    pub(in crate::gpu) rows: Vec<u32>,
    /// f32 samples the pass writes, one per bounding-box pixel.
    pub(in crate::gpu) samples: u32,
}

impl Planner<'_> {
    /// Words before the device f32 block in the shared coverage buffer.
    pub(super) fn device_alpha_base(&self) -> usize {
        4 + self.frame.strips.len() * 6
            + self.frame.spans.len() * 4
            + self.frame.alphas.len().div_ceil(4)
    }

    pub(super) fn device_words(&self) -> usize {
        self.frame.device_fills.last().map_or(0, |fill| {
            crate::raster::device::DeviceGeometry::device_sample_end(&self.frame.strips[fill.strip])
        })
    }

    pub(super) fn device_coverage(&mut self) -> Result<DeviceCoverage, ValidationError> {
        let frame = self.frame;
        let mut device = DeviceCoverage::default();
        if frame.device_fills.is_empty() {
            return Ok(device);
        }
        let path = "gpu.device_coverage";
        let overflow =
            || ValidationError::limit(path, "max_bytes", "Device coverage exceeds GPU addressing");
        let index = |value: usize| u32::try_from(value).map_err(|_| overflow());
        self.budget.bytes(
            (frame.device_fills.len() * FILL_WORDS * 4 + frame.device_edges.len() * 16) as u64,
            path,
        )?;
        self.budget.work(frame.device_edges.len() as u64, path)?;
        self.budget
            .work(frame.device_totals[0] + frame.device_totals[1], path)?;
        let alpha_base = self.device_alpha_base();
        let mut next_word = 0;
        for fill in frame.device_fills {
            let strip = &frame.strips[fill.strip];
            let (width, height) = (strip.width, strip.height as usize);
            let rows_start = device.rows.len();
            let first_edge = device.edges.len() / 4;
            let origin = [f64::from(strip.x), f64::from(strip.y)];
            let local = |edge: &[f64; 5]| {
                let (start, end) = if edge[4] > 0.0 {
                    ([edge[0], edge[1]], [edge[2], edge[3]])
                } else {
                    ([edge[2], edge[3]], [edge[0], edge[1]])
                };
                [
                    (start[0] - origin[0]) as f32,
                    (start[1] - origin[1]) as f32,
                    (end[0] - origin[0]) as f32,
                    (end[1] - origin[1]) as f32,
                ]
            };
            let rows_of = |edge: &[f64; 5]| {
                let first = (edge[1] - origin[1]).max(0.0).floor() as usize;
                let last = ((edge[3] - origin[1]).min(height as f64).ceil().max(0.0)) as usize;
                first..last.max(first)
            };
            let edges = &frame.device_edges[fill.edges.clone()];
            let mut counts = vec![0_u32; height];
            for edge in edges {
                for row in rows_of(edge) {
                    counts[row] += 1;
                }
            }
            let entries = counts.iter().map(|count| u64::from(*count)).sum::<u64>();
            self.budget.bytes(
                (height as u64 + 1 + entries) * 4 + edges.len() as u64 * 16,
                path,
            )?;
            self.budget.work(entries + height as u64, path)?;
            let table = rows_start;
            let mut cursor = table + height + 1;
            device.rows.resize(cursor, 0);
            for (row, count) in counts.iter().enumerate() {
                device.rows[table + row] = index(cursor)?;
                cursor += *count as usize;
            }
            device.rows[table + height] = index(cursor)?;
            device.rows.resize(cursor, 0);
            let mut fill_cursor = counts
                .iter()
                .enumerate()
                .map(|(row, _)| device.rows[table + row] as usize)
                .collect::<Vec<_>>();
            for (offset, edge) in edges.iter().enumerate() {
                for row in rows_of(edge) {
                    device.rows[fill_cursor[row]] = index(first_edge + offset)?;
                    fill_cursor[row] += 1;
                }
            }
            for edge in edges {
                device.edges.extend(local(edge));
            }
            let samples = crate::raster::device::DeviceGeometry::device_sample_end(strip)
                - strip.alpha_offset;
            device.fills.extend([
                strip.x,
                strip.y,
                width,
                strip.height,
                index(alpha_base + strip.alpha_offset)?,
                index(rows_start)?,
                u32::from(fill.rule == FillRule::EvenOdd),
                index(next_word)?,
            ]);
            next_word += samples;
        }
        device.samples = index(next_word)?;
        Ok(device)
    }
}
