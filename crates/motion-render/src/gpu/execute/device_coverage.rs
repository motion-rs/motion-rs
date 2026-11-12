//! GPU count, prefix, scatter, analytic boundary and tile coverage dispatch.

use super::*;
use crate::gpu::abi::CoverageParams;
use crate::gpu::plan::DeviceCoverage;
use crate::gpu::raster::device::COVERAGE_TILE_SIZE;

impl Gpu {
    pub(super) fn encode_device_coverage(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        device: &DeviceCoverage,
        samples: &wgpu::Buffer,
        working: &mut WorkingLease,
        timestamps: Option<&crate::gpu::timing::TimestampRecorder>,
    ) {
        let pipelines = self.coverage_pipelines();
        let params = CoverageParams {
            fills: device.fills.len() as u32,
            edges: device.edge_count,
            tiles: device.tiles,
            rows: device.rows,
        };
        let uniforms = self.upload_buffer(
            "Device coverage counts",
            bytemuck::bytes_of(&params),
            wgpu::BufferUsages::UNIFORM,
        );
        let fills = self.upload_buffer(
            "Device coverage fills",
            bytemuck::cast_slice(&device.fills),
            wgpu::BufferUsages::STORAGE,
        );
        let sizes = device.buffer_sizes();
        let edges = self.working_buffer(
            "Device generated coverage edges",
            sizes[1],
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        if !device.edges.is_empty() {
            self.queue
                .write_buffer(&edges, 0, bytemuck::cast_slice(&device.edges));
        }
        let curves = self.upload_buffer(
            "Device coverage curves",
            if device.curves.is_empty() {
                &[0; 48]
            } else {
                bytemuck::cast_slice(&device.curves)
            },
            wgpu::BufferUsages::STORAGE,
        );
        let tiles = self.working_buffer(
            "Device coverage tiles",
            sizes[2],
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        // Pooled buffers contain the preceding frame's atomics; clear before counting.
        encoder.clear_buffer(&tiles, 0, None);
        let indices = self.working_buffer(
            "Device coverage indices",
            sizes[3],
            wgpu::BufferUsages::STORAGE,
        );
        let row_areas = self.working_buffer(
            "Device boundary row areas",
            sizes[4],
            wgpu::BufferUsages::STORAGE,
        );
        let buffers = [
            &uniforms, &fills, &edges, &tiles, &indices, &row_areas, samples, &curves,
        ];
        let entries = buffers
            .iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect::<Vec<_>>();
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Device coverage bindings"),
            layout: &pipelines.layout,
            entries: &entries,
        });
        let counts = [
            (device.curves.len() as u32).div_ceil(64),
            params.edges.div_ceil(64),
            params.tiles.div_ceil(64),
            params.fills,
            params.tiles.div_ceil(64),
            params.edges.div_ceil(64),
            params.tiles * COVERAGE_TILE_SIZE,
            params.tiles,
        ];
        for (pass_index, (pipeline, groups)) in pipelines.passes.iter().zip(counts).enumerate() {
            if groups == 0 {
                continue;
            }
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Device coverage tiles"),
                timestamp_writes: timestamps.and_then(|recorder| match pass_index {
                    2 => Some(recorder.coverage_pass_boundary(2, true, false)),
                    3 => None,
                    4 => Some(recorder.coverage_pass_boundary(2, false, true)),
                    _ => Some(recorder.coverage_pass(if pass_index < 2 {
                        pass_index
                    } else {
                        pass_index - 2
                    })),
                }),
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            let groups_x = groups.min(self.device.limits().max_compute_workgroups_per_dimension);
            pass.dispatch_workgroups(groups_x, groups.div_ceil(groups_x), 1);
        }
        working.extend([uniforms, fills, edges, tiles, indices, row_areas, curves]);
    }
}
