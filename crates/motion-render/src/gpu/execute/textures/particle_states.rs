//! Analytic evaluation, primitive support preparation and bounded ordered particle tiles.

use super::*;

impl Gpu {
    pub(super) fn texture_particle_states(
        &self,
        step: &Step,
        buffers: [&wgpu::Buffer; 3],
        encoder: &mut wgpu::CommandEncoder,
        budget: &mut Budget,
    ) -> motion_core::Result<Option<wgpu::Buffer>> {
        let [parameters, data, status] = buffers;
        if step.parameters.operation.code != GpuOp::Particles as u32 {
            return Ok(None);
        }
        let count = step.data[0] as u32;
        if let Some(program) = &step.analytic {
            self.encode_analytic_particles(program, data, status, encoder);
        }
        let tiles = ParticleTiles::new(step.parameters.image_bounds(2), count)?;
        let tiled = step.parameters.flags.particle_tiles != 0;
        let initial = if tiled {
            tiles.bytes
        } else {
            (u64::from(count) * 48).max(16)
        };
        let limit_bytes = self
            .device
            .limits()
            .max_storage_buffer_binding_size
            .min(self.device.limits().max_buffer_size);
        if initial > limit_bytes {
            return Err(ValidationError::limit(
                "gpu.particle_tiles",
                "max_bytes",
                "Particle metadata exceeds device storage size",
            )
            .into());
        }
        let states = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Particle states"),
            size: initial,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Particle state bindings"),
            layout: &self.particle_states.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: parameters.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: states.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: data.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: status.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Precompute particles"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.particle_states);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(count.div_ceil(64).max(1), 1, 1);
        }
        if !tiled {
            return Ok(Some(states));
        }
        for (pipeline, groups) in [
            (&self.particle_bins[0], tiles.tiles.div_ceil(64).max(1)),
            (&self.particle_bins[1], 1),
        ] {
            self.particle_bin_pass(
                encoder,
                pipeline,
                parameters,
                [&states, data, status],
                groups,
            );
        }
        let summary = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Particle summary"),
            size: 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&states, tiles.summary, &summary, 0, 16);
        let next = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Particle composition"),
            });
        self.queue
            .submit([std::mem::replace(encoder, next).finish()]);
        let bytes = tiles.admit(self.particle_summary(&summary)?, budget, limit_bytes)?;
        let indexed = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Particle indices"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&states, 0, &indexed, 0, tiles.bytes);
        self.particle_bin_pass(
            encoder,
            &self.particle_bins[2],
            parameters,
            [&indexed, data, status],
            tiles.tiles.div_ceil(64).max(1),
        );
        Ok(Some(indexed))
    }
}
