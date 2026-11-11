//! Encodes the compute pass that writes device fill coverage into the shared frame buffer.

use super::*;
use crate::gpu::plan::DeviceCoverage;

impl Gpu {
    /// Writes every device fill's alphas; later composition passes read them as ordinary strips.
    pub(super) fn encode_device_coverage(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        device: &DeviceCoverage,
        coverage: &wgpu::Buffer,
        working: &mut WorkingLease,
    ) {
        let pipeline = self.coverage_pipeline();
        let parameters = [
            (device.fills.len() / crate::gpu::plan::FILL_WORDS) as u32,
            device.words,
        ];
        let uniforms = self.upload_buffer(
            "Device coverage counts",
            bytemuck::cast_slice(&[parameters[0], parameters[1], 0, 0]),
            wgpu::BufferUsages::UNIFORM,
        );
        let fills = self.upload_buffer(
            "Device coverage fills",
            bytemuck::cast_slice(&device.fills),
            wgpu::BufferUsages::STORAGE,
        );
        let edges = self.upload_buffer(
            "Device coverage edges",
            bytemuck::cast_slice(&device.edges),
            wgpu::BufferUsages::STORAGE,
        );
        let rows = self.upload_buffer(
            "Device coverage rows",
            bytemuck::cast_slice(&device.rows),
            wgpu::BufferUsages::STORAGE,
        );
        let buffers = [&uniforms, &fills, &edges, &rows, coverage];
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
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Compute device fill coverage"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(device.words.div_ceil(256), 1, 1);
        drop(pass);
        working.extend([uniforms, fills, edges, rows]);
    }
}
