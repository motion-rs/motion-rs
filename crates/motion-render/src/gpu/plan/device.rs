//! Device coverage descriptors and statically admitted GPU tile binning resources.

use super::*;
use crate::gpu::abi::{CoverageCurve, CoverageEdge, CoverageFill, CoverageParams, CoverageTile};
use crate::raster::device::{
    COVERAGE_CARRY_STRIDE, COVERAGE_TILE_SIZE, CURVE_FLATTEN_WORK_PER_SEGMENT,
};
use motion_core::{FillRule, ValidationError};

/// GPU-owned counts, offsets, indices and row carries have no CPU-generated edge lists.
#[derive(Default)]
pub(in crate::gpu) struct DeviceCoverage {
    pub(in crate::gpu) fills: Vec<CoverageFill>,
    pub(in crate::gpu) edges: Vec<CoverageEdge>,
    pub(in crate::gpu) curves: Vec<CoverageCurve>,
    pub(in crate::gpu) edge_count: u32,
    pub(in crate::gpu) tiles: u32,
    pub(in crate::gpu) indices: u32,
    pub(in crate::gpu) rows: u32,
    pub(in crate::gpu) samples: u32,
}

impl DeviceCoverage {
    pub(in crate::gpu) fn buffer_sizes(&self) -> [u64; 6] {
        [
            self.fills.len() as u64 * size_of::<CoverageFill>() as u64,
            u64::from(self.edge_count).max(1) * size_of::<CoverageEdge>() as u64,
            u64::from(self.tiles) * size_of::<CoverageTile>() as u64,
            u64::from(self.indices).max(1) * 4,
            u64::from(self.tiles) * u64::from(COVERAGE_TILE_SIZE) * COVERAGE_CARRY_STRIDE * 4,
            (self.curves.len() as u64).max(1) * size_of::<CoverageCurve>() as u64,
        ]
    }
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

    pub(in crate::gpu) fn device_coverage(&mut self) -> Result<DeviceCoverage, ValidationError> {
        let frame = self.frame;
        let mut device = DeviceCoverage::default();
        if frame.device_fills.is_empty() {
            return Ok(device);
        }
        let path = "gpu.device_coverage";
        let index = |value: u64| {
            u32::try_from(value).map_err(|_| {
                ValidationError::limit(path, "max_bytes", "Device coverage exceeds GPU addressing")
            })
        };
        let fill_bytes = frame.device_fills.len() as u64 * size_of::<CoverageFill>() as u64;
        let edge_bytes = frame.device_edges.len() as u64 * size_of::<CoverageEdge>() as u64;
        let curve_bytes = frame.device_curves.len() as u64 * size_of::<CoverageCurve>() as u64;
        let generated = frame
            .device_fills
            .iter()
            .map(|fill| u64::from(fill.segments))
            .sum::<u64>();
        // Host descriptors, upload staging and device descriptors can coexist.
        self.budget.bytes(
            (fill_bytes + edge_bytes + curve_bytes) * 3
                + generated * size_of::<CoverageEdge>() as u64,
            path,
        )?;
        self.budget.work(
            frame.device_edges.len() as u64
                + generated * CURVE_FLATTEN_WORK_PER_SEGMENT
                + frame.device_totals.visits,
            path,
        )?;
        device
            .fills
            .try_reserve_exact(frame.device_fills.len())
            .map_err(|_| {
                ValidationError::limit(
                    path,
                    "max_bytes",
                    "Could not reserve device fill descriptors",
                )
            })?;
        device
            .curves
            .try_reserve_exact(frame.device_curves.len())
            .map_err(|_| {
                ValidationError::limit(
                    path,
                    "max_bytes",
                    "Could not reserve device curve descriptors",
                )
            })?;
        let mut generated_edge = index(frame.device_edges.len() as u64)?;
        device
            .edges
            .try_reserve_exact(frame.device_edges.len())
            .map_err(|_| {
                ValidationError::limit(
                    path,
                    "max_bytes",
                    "Could not reserve device edge descriptors",
                )
            })?;
        let alpha_base = self.device_alpha_base();
        for fill in frame.device_fills {
            let strip = &frame.strips[fill.strip];
            let fill_index = index(device.fills.len() as u64)?;
            let origin = [f64::from(strip.x), f64::from(strip.y)];
            let first_edge = if fill.curves.is_empty() {
                index(device.edges.len() as u64)?
            } else {
                generated_edge
            };
            for edge in &frame.device_edges[fill.edges.clone()] {
                let points = [
                    (edge[0] - origin[0]) as f32,
                    (edge[1] - origin[1]) as f32,
                    (edge[2] - origin[0]) as f32,
                    (edge[3] - origin[1]) as f32,
                ];
                device.edges.push(CoverageEdge {
                    points: if edge[4] > 0.0 {
                        points
                    } else {
                        [points[2], points[3], points[0], points[1]]
                    },
                    fill: fill_index,
                    reserved: [0; 3],
                });
            }
            for curve in &frame.device_curves[fill.curves.clone()] {
                let points = curve
                    .points
                    .map(|point| [(point.x - origin[0]) as f32, (point.y - origin[1]) as f32]);
                device.curves.push(CoverageCurve {
                    first: [points[0][0], points[0][1], points[1][0], points[1][1]],
                    second: [points[2][0], points[2][1], points[3][0], points[3][1]],
                    first_edge: generated_edge,
                    segments: curve.segments,
                    fill: fill_index,
                    reserved: 0,
                });
                generated_edge = index(u64::from(generated_edge) + u64::from(curve.segments))?;
            }
            device.fills.push(CoverageFill {
                size: [strip.width, strip.height],
                coverage: index((alpha_base + strip.alpha_offset) as u64)?,
                rule: u32::from(fill.rule == FillRule::EvenOdd),
                first_edge,
                edge_count: index(fill.edges.len() as u64 + u64::from(fill.segments))?,
                first_tile: device.tiles,
                columns: fill.tiles.columns,
                first_index: device.indices,
                index_capacity: fill.tiles.indices,
                first_row: device.rows,
                tile_rows: fill.tiles.rows,
            });
            device.tiles = index(u64::from(device.tiles) + fill.tiles.tile_count())?;
            device.indices = index(u64::from(device.indices) + u64::from(fill.tiles.indices))?;
            device.rows =
                index(u64::from(device.rows) + u64::from(fill.tiles.rows * COVERAGE_TILE_SIZE))?;
            device.samples = index(
                u64::from(device.samples) + u64::from(strip.width) * u64::from(strip.height),
            )?;
        }
        device.edge_count = generated_edge;
        let sizes = device.buffer_sizes();
        self.budget.bytes(
            sizes[2..5].iter().sum::<u64>()
                + if device.curves.is_empty() {
                    size_of::<CoverageCurve>() as u64
                } else {
                    0
                }
                + size_of::<CoverageParams>() as u64 * 3
                + (size_of::<wgpu::Buffer>() as u64 * 8 + size_of::<wgpu::BindGroup>() as u64) * 6,
            path,
        )?;
        Ok(device)
    }
}
