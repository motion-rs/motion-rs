//! Analytic trajectory evaluation into the primitive upload consumed by particle tiling.

use super::*;

impl Gpu {
    pub(super) fn encode_analytic_particles(
        &self,
        program: &motion_core::instances::AnalyticInstanceUpload,
        data: &wgpu::Buffer,
        status: &wgpu::Buffer,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let pipeline = self.analytic_particles.get_or_init(|| {
            let shader = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("Seeded analytic trajectories"),
                    source: wgpu::ShaderSource::Wgsl(
                        include_str!("../shaders/analytic.wgsl").into(),
                    ),
                });
            self.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("Evaluate analytic trajectories"),
                    layout: None,
                    module: &shader,
                    entry_point: Some("evaluate"),
                    compilation_options: Default::default(),
                    cache: None,
                })
        });
        let input = self.upload_buffer(
            "Seeded analytic initial states",
            bytemuck::cast_slice(program.words()),
            wgpu::BufferUsages::STORAGE,
        );
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Analytic trajectory bindings"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: data.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: status.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Evaluate analytic trajectories"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        let groups = (program.len() as u32).div_ceil(64);
        let width = groups.min(self.device.limits().max_compute_workgroups_per_dimension);
        if width != 0 {
            pass.dispatch_workgroups(width, groups.div_ceil(width), 1);
        }
    }
}
