//! Particle metadata passes and bounded prefix readback.

use super::*;

impl Gpu {
    pub(super) fn particle_bin_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        parameters: &wgpu::Buffer,
        buffers: [&wgpu::Buffer; 3],
        groups: u32,
    ) {
        let entries = [
            wgpu::BindGroupEntry {
                binding: 0,
                resource: parameters.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: buffers[0].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: buffers[1].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: buffers[2].as_entire_binding(),
            },
        ];
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle tile pass"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Build ordered particle tiles"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(groups, 1, 1);
    }

    pub(super) fn particle_summary(&self, buffer: &wgpu::Buffer) -> motion_core::Result<[u32; 4]> {
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.device.poll(wgpu::Maintain::Wait);
        self.loss.check()?;
        receiver
            .recv()
            .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?
            .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?;
        let mapped = buffer.slice(..).get_mapped_range();
        let result = std::array::from_fn(|index| {
            u32::from_le_bytes(
                mapped[index * 4..index * 4 + 4]
                    .try_into()
                    .expect("prefix word"),
            )
        });
        drop(mapped);
        buffer.unmap();
        Ok(result)
    }
}
