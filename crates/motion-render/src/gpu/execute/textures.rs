//! Texture working images with one output-boundary resolve.

use super::*;
use uploads::BatchUploads;

mod uploads;

/// Plan steps or batches recorded into one command encoder before it is submitted.
const PASSES_PER_SUBMISSION: usize = 64;

struct TextureInputs<'a> {
    images: &'a [wgpu::Texture],
    slots: &'a [usize],
    status: &'a wgpu::Buffer,
    deferred: &'a [bool],
    coverage: &'a wgpu::Buffer,
}

pub(super) struct EncodedTexture {
    pub(super) encoder: wgpu::CommandEncoder,
    pub(super) texture: wgpu::Texture,
    pub(super) bounds: PixelRect,
    pub(super) deferred: bool,
    pub(super) status: wgpu::Buffer,
    pub(super) stats: WorkStats,
    pub(super) working: WorkingLease,
    pub(super) timestamps: Option<super::super::timing::TimestampRecorder>,
}

pub(super) struct EncodedLinear {
    pub(super) encoder: wgpu::CommandEncoder,
    pub(super) output: wgpu::Buffer,
    pub(super) status: wgpu::Buffer,
    pub(super) stats: WorkStats,
    pub(super) working: WorkingLease,
    pub(super) timestamps: Option<super::super::timing::TimestampRecorder>,
}

impl Gpu {
    pub(super) fn encode(&self, plan: &Plan) -> motion_core::Result<EncodedLinear> {
        let EncodedTexture {
            mut encoder,
            texture,
            bounds,
            deferred,
            status,
            stats,
            mut working,
            timestamps,
        } = self.encode_texture(plan)?;
        let size = image_size(bounds);
        let output = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Final linear output boundary"),
            size: image_bytes(bounds),
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let uniforms = GpuUniforms {
            sources: [bounds.into(), UniformRect::default(), bounds.into()],
            flags: SourceFlags {
                first_deferred: u32::from(deferred),
                ..Default::default()
            },
            ..Default::default()
        };
        let parameters = self.upload_buffer(
            "Output boundary",
            bytemuck::bytes_of(&uniforms),
            wgpu::BufferUsages::UNIFORM,
        );
        let view = texture.create_view(&Default::default());
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Texture output boundary"),
            layout: &self.textures.resolve.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: parameters.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: status.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Resolve output boundary"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.textures.resolve);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(size.width.div_ceil(8), size.height.div_ceil(8), 1);
        }
        working.extend([parameters, output.clone()]);
        Ok(EncodedLinear {
            encoder,
            output,
            status,
            stats,
            working,
            timestamps,
        })
    }

    pub(super) fn encode_texture(&self, plan: &Plan) -> motion_core::Result<EncodedTexture> {
        let mut working = WorkingLease::default();
        let (slots, sizes) = if self.textures.blends.is_some() {
            (&plan.texture_slots, &plan.texture_sizes)
        } else {
            (&plan.compute_texture_slots, &plan.compute_texture_sizes)
        };
        working
            .try_reserve_exact(plan.steps.len() * 6 + 8)
            .map_err(|_| {
                ValidationError::limit("gpu.pool", "max_bytes", "Could not reserve buffer leases")
            })?;
        working
            .textures
            .try_reserve_exact(sizes.len())
            .map_err(|_| {
                ValidationError::limit("gpu.pool", "max_bytes", "Could not reserve texture leases")
            })?;
        let mut images = Vec::new();
        images.try_reserve_exact(sizes.len()).map_err(|_| {
            ValidationError::limit("gpu.pool", "max_bytes", "Could not reserve texture handles")
        })?;
        for size in sizes {
            if size.width > self.device.limits().max_texture_dimension_2d
                || size.height > self.device.limits().max_texture_dimension_2d
            {
                return Err(ValidationError::limit(
                    "gpu.textures",
                    "max_pixels",
                    "Working texture exceeds device dimensions",
                )
                .into());
            }
            let mut lease = WorkingLease::default();
            let texture = self.working_texture(
                &mut lease,
                *size,
                wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                "linear working image",
            )?;
            images.push(texture);
            working.textures.extend(lease.textures);
        }
        let status = self.upload_buffer(
            "finite-color status",
            &[0; 4],
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let dummy = self.upload_buffer(
            "unused particle metadata",
            &[0; 16],
            wgpu::BufferUsages::STORAGE,
        );
        working.extend([status.clone(), dummy.clone()]);
        let coverage = if plan.coverage.is_empty() {
            dummy.clone()
        } else {
            let buffer = self.upload_buffer(
                "Shared frame strips spans and alpha",
                bytemuck::cast_slice(&plan.coverage),
                wgpu::BufferUsages::STORAGE,
            );
            working.push(buffer.clone());
            buffer
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Texture working images"),
            });
        let timestamps =
            super::super::timing::TimestampRecorder::new(&self.context, self.timing.clone());
        if let Some(timestamps) = &timestamps {
            timestamps.write(&mut encoder, 0);
        }
        let mut budget = Budget::resume(
            plan.limits,
            plan.stats.peak_bytes,
            plan.stats.peak_bytes,
            plan.stats.work,
        );
        let mut deferred = vec![false; plan.steps.len() + 1];
        let mut consumed = 0;
        let mut recorded = 0;
        for (position, step) in plan.steps.iter().enumerate() {
            if position < consumed {
                continue;
            }
            if recorded >= PASSES_PER_SUBMISSION {
                self.submit_recorded(&mut encoder);
                recorded = 0;
            }
            recorded += 1;
            if self.textures.blends.is_some() && plan.texture_blend(position) {
                let range = plan.texture_run(position);
                consumed = range.end;
                self.encode_texture_batch(
                    plan,
                    range.clone(),
                    TextureInputs {
                        images: &images,
                        slots,
                        status: &status,
                        deferred: &deferred,
                        coverage: &coverage,
                    },
                    &mut encoder,
                    &mut working,
                )?;
                deferred[range.end] = plan.steps[range.clone()].iter().any(|step| {
                    step.parameters.operation.code == GpuOp::Composite as u32
                        && step.parameters.operation.kind != 0
                });
                continue;
            }
            let mut uniforms = step.parameters;
            uniforms.flags.first_deferred = u32::from(deferred[step.first]);
            uniforms.flags.second_deferred = u32::from(deferred[step.second]);
            let parameters = self.upload_buffer(
                "texture parameters",
                bytemuck::bytes_of(&uniforms),
                wgpu::BufferUsages::UNIFORM,
            );
            let data = self.upload_buffer(
                "coverage or kernel",
                bytemuck::cast_slice(&step.data),
                wgpu::BufferUsages::STORAGE,
            );
            working.extend([parameters.clone(), data.clone()]);
            let first = images[slots[step.first]].create_view(&Default::default());
            let second = images[slots[step.second]].create_view(&Default::default());
            let output = images[slots[position + 1]].create_view(&Default::default());
            let size = image_size(step.parameters.image_bounds(2));
            {
                let states = self.texture_particle_states(
                    step,
                    [&parameters, &data, &status],
                    &mut encoder,
                    &mut budget,
                )?;
                let pipeline = if step.parameters.flags.particle_tiles != 0 {
                    &self.textures.particles
                } else {
                    &self.textures.compute
                };
                let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Texture compute"),
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: parameters.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: states.as_ref().unwrap_or(&dummy).as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: data.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: status.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 7,
                            resource: wgpu::BindingResource::TextureView(&first),
                        },
                        wgpu::BindGroupEntry {
                            binding: 8,
                            resource: wgpu::BindingResource::TextureView(&second),
                        },
                        wgpu::BindGroupEntry {
                            binding: 9,
                            resource: wgpu::BindingResource::TextureView(&output),
                        },
                        wgpu::BindGroupEntry {
                            binding: 10,
                            resource: coverage.as_entire_binding(),
                        },
                    ],
                });
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Texture filter or composition"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(size.width.div_ceil(8), size.height.div_ceil(8), 1);
                if let Some(states) = states {
                    working.push(states);
                }
            }
        }
        if let Some(timestamps) = &timestamps {
            timestamps.write(&mut encoder, 1);
            timestamps.write(&mut encoder, 2);
        }
        let bounds = plan.steps[plan.output - 1].parameters.image_bounds(2);
        Ok(EncodedTexture {
            encoder,
            texture: images[slots[plan.output]].clone(),
            bounds,
            deferred: deferred[plan.output],
            status,
            stats: WorkStats {
                peak_bytes: budget.peak_bytes(),
                work: budget.work_done(),
            },
            working,
            timestamps,
        })
    }

    /// Metal queues a bounded number of command buffers per queue and each pass can own
    /// several, so a frame with hundreds of layers must hand work to the GPU while it is
    /// still being encoded instead of waiting for the final submission.
    fn submit_recorded(&self, encoder: &mut wgpu::CommandEncoder) {
        let next = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Texture working images"),
            });
        self.queue
            .submit([std::mem::replace(encoder, next).finish()]);
    }

    fn encode_texture_batch(
        &self,
        plan: &Plan,
        range: Range<usize>,
        inputs: TextureInputs<'_>,
        encoder: &mut wgpu::CommandEncoder,
        working: &mut WorkingLease,
    ) -> motion_core::Result {
        let TextureInputs {
            images,
            slots,
            status,
            deferred,
            coverage,
        } = inputs;
        let step = &plan.steps[range.start];
        let accumulating = slots[step.first] == slots[range.end];
        let bounds = step.parameters.image_bounds(2);
        let size = image_size(bounds);
        let device_limits = self.device.limits();
        let shared_data_bytes = plan
            .texture_data_bytes(
                range.start,
                device_limits.max_buffer_size,
                device_limits.min_storage_buffer_offset_alignment,
            )
            .filter(|_| {
                plan.steps[range.clone()].iter().all(|step| {
                    step.data.len() as u64 * size_of::<f32>() as u64
                        <= device_limits.max_storage_buffer_binding_size
                })
            });
        let data_payload_bytes = plan.steps[range.clone()]
            .iter()
            .map(|step| step.data.len() as u64 * size_of::<f32>() as u64)
            .sum::<u64>();
        let padding = shared_data_bytes.map_or(0, |bytes| bytes - data_payload_bytes);
        let bytes = Plan::batch_upload_bytes(&plan.steps[range.clone()]) + padding;
        let staging = (bytes <= self.device.limits().max_buffer_size)
            .then(|| self.context.staging_buffer(bytes));
        let mut uploads = BatchUploads {
            context: &self.context,
            staging: staging.as_ref(),
            mapped: staging
                .as_ref()
                .map(|buffer| buffer.slice(..).get_mapped_range_mut())
                .transpose()
                .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?,
            offset: 0,
        };
        let mut uniforms = step.parameters;
        uniforms.flags.first_deferred = u32::from(deferred[step.first]);
        let parameters = uploads.upload(
            "Batch backdrop",
            bytemuck::bytes_of(&uniforms),
            wgpu::BufferUsages::UNIFORM,
            encoder,
        );
        let rectangle = [
            bounds.left as f32,
            bounds.top as f32,
            bounds.right as f32,
            bounds.bottom as f32,
        ];
        let vertices = uploads.upload(
            "Batch rectangle",
            bytemuck::cast_slice(&rectangle),
            wgpu::BufferUsages::VERTEX,
            encoder,
        );
        working.extend([parameters.clone(), vertices.clone()]);
        let first = images[slots[step.first]].create_view(&Default::default());
        let output = images[slots[range.end]].create_view(&Default::default());
        let backdrop = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Batch backdrop"),
            layout: &self.textures.backdrop.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: parameters.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: status.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&first),
                },
            ],
        });
        let blends = self
            .textures
            .blends
            .as_ref()
            .expect("hardware blend capability");
        let mut draws = Vec::new();
        draws.try_reserve_exact(range.len()).map_err(|_| {
            ValidationError::limit("gpu.batch", "max_bytes", "Could not reserve batch bindings")
        })?;
        let shared_vertices = uploads.upload_vertices(&plan.steps[range.clone()], encoder);
        if let Some(vertices) = &shared_vertices {
            working.push(vertices.clone());
        }
        let shared_parameters =
            uploads.upload_uniforms(&plan.steps[range.clone()], deferred, encoder);
        if let Some(parameters) = &shared_parameters {
            working.push(parameters.clone());
        }
        let shared_data =
            uploads.upload_data(&plan.steps[range.clone()], shared_data_bytes, encoder);
        if let Some(data) = &shared_data {
            working.push(data.clone());
        }
        let mut data_offset = 0u64;
        for (index, step) in plan.steps[range.clone()].iter().enumerate() {
            let mut uniforms = step.parameters;
            uniforms.flags.second_deferred = u32::from(deferred[step.second]);
            let parameters = if let Some(parameters) = &shared_parameters {
                parameters.clone()
            } else {
                let parameters = uploads.upload(
                    "Batch source",
                    bytemuck::bytes_of(&uniforms),
                    wgpu::BufferUsages::UNIFORM,
                    encoder,
                );
                working.push(parameters.clone());
                parameters
            };
            let data = if let Some(data) = &shared_data {
                data_offset = data_offset.div_ceil(u64::from(Plan::BATCH_DATA_ALIGNMENT))
                    * u64::from(Plan::BATCH_DATA_ALIGNMENT);
                data.clone()
            } else {
                let data = uploads.upload(
                    "Batch coverage",
                    bytemuck::cast_slice(&step.data),
                    wgpu::BufferUsages::STORAGE,
                    encoder,
                );
                working.push(data.clone());
                data
            };
            let vertices = if let Some(vertices) = &shared_vertices {
                vertices.clone()
            } else {
                let rectangle = step.parameters.rectangle();
                let vertices = uploads.upload(
                    "Batch instances",
                    if step.quads.is_empty() {
                        bytemuck::cast_slice(&rectangle)
                    } else {
                        bytemuck::cast_slice(&step.quads)
                    },
                    wgpu::BufferUsages::VERTEX,
                    encoder,
                );
                working.push(vertices.clone());
                vertices
            };
            let plus = step.parameters.operation.code == GpuOp::Composite as u32
                && step.parameters.operation.kind != 0;
            let pipeline = &blends[usize::from(plus)];
            let second = images[slots[step.second]].create_view(&Default::default());
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Batch source"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &parameters,
                            offset: if shared_parameters.is_some() {
                                index as u64 * size_of::<GpuUniforms>() as u64
                            } else {
                                0
                            },
                            size: wgpu::BufferSize::new(size_of::<GpuUniforms>() as u64),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &data,
                            offset: if shared_data.is_some() {
                                data_offset
                            } else {
                                0
                            },
                            size: wgpu::BufferSize::new(
                                step.data.len() as u64 * size_of::<f32>() as u64,
                            ),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: status.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: wgpu::BindingResource::TextureView(&second),
                    },
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: coverage.as_entire_binding(),
                    },
                ],
            });
            #[cfg(test)]
            self.batch_bind_groups_encoded
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if shared_data.is_some() {
                data_offset += step.data.len() as u64 * size_of::<f32>() as u64;
            }
            draws.push((
                usize::from(plus),
                group,
                vertices,
                step.quads.len().max(1) as u32,
            ));
            #[cfg(test)]
            if plus {
                self.hardware_plus_encoded
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        assert_eq!(uploads.offset, if staging.is_some() { bytes } else { 0 });
        drop(uploads);
        if let Some(staging) = staging {
            staging.unmap();
            working.push(staging);
        }
        // A backdrop built only from bounded fills is finite already; anything else is checked.
        if accumulating && !plan.texture_bounded[step.first] {
            let bindings = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Check accumulation backdrop"),
                layout: &self.textures.check.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: parameters.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: status.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::TextureView(&first),
                    },
                ],
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Validate in-place background"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.textures.check);
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups(size.width.div_ceil(8), size.height.div_ceil(8), 1);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Ordered SourceOver and Plus"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if accumulating {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_viewport(0.0, 0.0, size.width as f32, size.height as f32, 0.0, 1.0);
            pass.set_scissor_rect(0, 0, size.width, size.height);
            if !accumulating {
                pass.set_pipeline(&self.textures.backdrop);
                pass.set_bind_group(0, &backdrop, &[]);
                pass.set_vertex_buffer(0, vertices.slice(..));
                pass.draw(0..6, 0..1);
            }
            let mut vertex_offset = 0;
            for (blend, group, vertices, count) in &draws {
                let start = if shared_vertices.is_some() {
                    vertex_offset
                } else {
                    0
                };
                let end = start + u64::from(*count) * size_of::<[f32; 4]>() as u64;
                pass.set_pipeline(&blends[*blend]);
                pass.set_bind_group(0, group, &[]);
                pass.set_vertex_buffer(0, vertices.slice(start..end));
                pass.draw(0..6, 0..*count);
                vertex_offset = end;
            }
        }
        #[cfg(test)]
        if range.len() > 1 {
            self.fill_batches_encoded
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(())
    }

    fn texture_particle_states(
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
