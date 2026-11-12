//! GPU context creation, device-limit validation of plans and render pipeline setup.

use super::*;
use motion_core::sync::MutexExt;
use motion_core::{PixelSize, ValidationError};

mod batch;
mod self_check;
pub(in crate::gpu) mod textures;

/// Timestamps are written as the start of tiny marker passes, which only needs
/// `TIMESTAMP_QUERY`. Writing them between commands needs `TIMESTAMP_QUERY_INSIDE_ENCODERS`,
/// which Metal advertises on Apple GPUs that can only sample at encoder boundaries and then
/// emulates with queued samples; a frame using that emulation never completed.
pub(super) fn timestamp_features(available: wgpu::Features) -> wgpu::Features {
    available & wgpu::Features::TIMESTAMP_QUERY
}

impl Plan {
    pub(super) fn validate_device(&self, limits: &wgpu::Limits) -> Result<(), ValidationError> {
        if self.device.samples != 0 {
            if self.device.buffer_sizes().iter().any(|bytes| {
                *bytes > limits.max_storage_buffer_binding_size || *bytes > limits.max_buffer_size
            }) {
                return Err(device_error(
                    "buffer_size",
                    "Coverage tile buffers exceed device limits",
                ));
            }
            let maximum_groups = u64::from(limits.max_compute_workgroups_per_dimension).pow(2);
            let groups = [
                u64::from(self.device.edge_count).div_ceil(64),
                (self.device.curves.len() as u64).div_ceil(64),
                (self.device.fills.len() as u64).div_ceil(64),
                u64::from(self.device.rows).div_ceil(64),
                u64::from(self.device.tiles),
            ];
            if groups.iter().any(|count| *count > maximum_groups) {
                return Err(device_error(
                    "dispatch",
                    "Coverage tile dispatch exceeds device limits",
                ));
            }
        }
        let coverage_bytes = (self.coverage.len() as u64).max(u64::from(self.device.samples)) * 4;
        if coverage_bytes > limits.max_storage_buffer_binding_size
            || coverage_bytes > limits.max_buffer_size
        {
            return Err(device_error(
                "buffer_size",
                "Shared coverage exceeds device buffer limits",
            ));
        }
        let output_bytes = self.packed_output.map_or_else(
            || u64::from(self.size.width) * u64::from(self.size.height) * 16,
            |(size, format)| super::plan::packed_buffer_bytes(size, format),
        );
        if output_bytes > limits.max_storage_buffer_binding_size
            || output_bytes > limits.max_buffer_size
        {
            return Err(device_error(
                "buffer_size",
                "Native GPU output exceeds the selected device buffer limits",
            ));
        }
        if self.size.width.div_ceil(8) > limits.max_compute_workgroups_per_dimension
            || self.size.height.div_ceil(8) > limits.max_compute_workgroups_per_dimension
        {
            return Err(device_error(
                "dispatch",
                "Native GPU raster exceeds the selected device dispatch limits",
            ));
        }
        if self.steps.iter().any(|step| {
            step.data.len() as u64 * 4 > limits.max_storage_buffer_binding_size
                || step.data.len() as u64 * 4 > limits.max_buffer_size
                || step.resource.as_ref().is_some_and(|resource| {
                    resource.len() as u64 * 4 > limits.max_storage_buffer_binding_size
                        || resource.len() as u64 * 4 > limits.max_buffer_size
                })
        }) {
            return Err(device_error(
                "buffer_size",
                "Native GPU upload exceeds the selected device buffer limits",
            ));
        }
        for step in &self.steps {
            if let Some(program) = &step.analytic
                && (program.bytes() > limits.max_storage_buffer_binding_size
                    || program.bytes() > limits.max_buffer_size
                    || (program.len() as u64).div_ceil(64)
                        > u64::from(limits.max_compute_workgroups_per_dimension).pow(2))
            {
                return Err(device_error(
                    "analytic_particles",
                    "Analytic upload or dispatch exceeds device limits",
                ));
            }
            if step.parameters.operation.code == GpuOp::Particles as u32 {
                let tiles =
                    ParticleTiles::new(step.parameters.image_bounds(2), step.data[0] as u32)?;
                if tiles.bytes
                    > limits
                        .max_storage_buffer_binding_size
                        .min(limits.max_buffer_size)
                    || tiles.tiles.div_ceil(64) > limits.max_compute_workgroups_per_dimension
                    || tiles.count.div_ceil(64) > limits.max_compute_workgroups_per_dimension
                {
                    return Err(device_error(
                        "particle_tiles",
                        "Native GPU particle metadata or dispatch exceeds device limits",
                    ));
                }
            }
        }
        if self
            .texture_sizes
            .iter()
            .chain(&self.compute_texture_sizes)
            .any(|size| {
                size.width > limits.max_texture_dimension_2d
                    || size.height > limits.max_texture_dimension_2d
            })
        {
            return Err(device_error(
                "texture_size",
                "Native GPU working texture exceeds device limits",
            ));
        }
        Ok(())
    }
}

impl GpuContext {
    /// Creates a context without fonts, windows or an encoder.
    pub fn new() -> motion_core::Result<Self> {
        Self::create(wgpu::PowerPreference::default())
    }

    /// Creates a context with the requested adapter power preference.
    pub fn create(power: wgpu::PowerPreference) -> motion_core::Result<Self> {
        pollster::block_on(async {
            let instance = crate::gpu::instance(crate::Renderer::Gpu)?;
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: power,
                    ..Default::default()
                })
                .await
                .map_err(|error| motion_core::Error::GpuUnavailable(Box::new(error)))?;
            let info = crate::gpu::adapter_info(&adapter)?;
            let format = adapter.get_texture_format_features(wgpu::TextureFormat::Rgba32Float);
            let hardware_fills = adapter
                .features()
                .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
                && format.allowed_usages.contains(
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                )
                && format
                    .flags
                    .contains(wgpu::TextureFormatFeatureFlags::BLENDABLE);
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("Motion PreparedFrame compute"),
                    required_features: if hardware_fills {
                        wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
                    } else {
                        wgpu::Features::empty()
                    } | timestamp_features(adapter.features()),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                    ..Default::default()
                })
                .await
                .map_err(|error| motion_core::Error::GpuUnavailable(Box::new(error)))?;
            Self::from_device(device, queue, info, hardware_fills)
        })
    }

    /// Imports an existing native device/queue (for example eframe's render state).
    /// Reuse clones of this context for every session on that device.
    pub fn from_existing(
        device: wgpu::Device,
        queue: wgpu::Queue,
        adapter: wgpu::AdapterInfo,
    ) -> motion_core::Result<Self> {
        Self::from_device(
            device,
            queue,
            crate::gpu::info_from_adapter(adapter)?,
            false,
        )
    }

    /// Existing device handle; textures must be created on this same device.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Existing queue handle for compatible external texture submissions.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Whether the existing device can bracket command-encoder stages with GPU timestamps.
    /// Imported devices without both timestamp features remain fully supported.
    pub fn supports_timestamps(&self) -> bool {
        !timestamp_features(self.device.features()).is_empty()
    }

    /// Selected adapter diagnostics.
    pub fn info(&self) -> &crate::GpuInfo {
        &self.info
    }

    /// Whether clones/session registrations use this exact shared context.
    pub fn same_device(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.submissions, &other.submissions)
    }

    /// Checks the shared device-loss monitor without submitting work.
    pub fn check_device(&self) -> motion_core::Result<()> {
        self.loss.check()
    }

    /// Serializes extension submissions with 2D execution and checks device loss.
    pub fn acquire_submission(&self) -> motion_core::Result<std::sync::MutexGuard<'_, ()>> {
        let guard = self.submissions.lock_unpoisoned();
        self.loss.check()?;
        Ok(guard)
    }

    /// Submits to a same-device texture without waiting for GPU completion.
    /// The caller must finish the returned frame before publishing the texture.
    pub fn submit_texture(
        &self,
        frame: &PreparedFrame,
        texture: &wgpu::Texture,
        format: crate::PixelFormat,
        limits: RasterLimits,
    ) -> motion_core::Result<PendingTextureFrame> {
        let plan = Plan::new(frame, limits)?;
        Gpu::from_context(self.clone()).submit_texture(frame, plan, texture, format)
    }

    /// Renders prepared content to memory or a same-device external texture.
    /// Texture formats are Rgba16Float for linear output and Rgba8Unorm/Bgra8Unorm
    /// for encoded RGBA8; no implicit alpha loss or CPU pixel roundtrip occurs.
    pub fn render(
        &self,
        frame: &PreparedFrame,
        target: crate::RenderTarget<'_>,
        format: crate::PixelFormat,
        limits: RasterLimits,
    ) -> motion_core::Result<()> {
        let mut renderer = Gpu::from_context(self.clone());
        match target {
            crate::RenderTarget::Memory(target) => {
                if Gpu::supports_packed_output(frame.size(), format) {
                    let mut remaining = limits;
                    if !target.planes().is_empty() {
                        remaining.max_bytes = remaining
                            .max_bytes
                            .checked_sub(target.retained_bytes())
                            .filter(|value| *value > 0)
                            .ok_or_else(|| {
                                ValidationError::limit(
                                    "gpu.output",
                                    "max_bytes",
                                    "Retained target exhausts GPU allowance",
                                )
                            })?;
                    }
                    let plan = Plan::new_packed(frame, remaining, format)?;
                    let replacement = renderer.submit_output(frame, plan, format)?.finish()?;
                    *target = replacement;
                    return Ok(());
                }
                let plan = Plan::new(frame, limits)?;
                let linear = renderer.render(frame, plan)?;
                crate::FrameBuffer::write_linear(
                    &linear,
                    crate::RenderTarget::Memory(target),
                    format,
                    limits,
                )?;
                Ok(())
            }
            crate::RenderTarget::Texture(texture) => {
                let plan = Plan::new(frame, limits)?;
                renderer.render_texture(frame, plan, texture, format)
            }
        }
    }

    /// Rejects software adapters unless the caller explicitly opted in.
    pub fn validate_policy(&self, allow_software: bool) -> motion_core::Result<()> {
        let software = crate::backend::software_adapter(&self.info.name, &self.info.device_type);
        if software && !allow_software {
            return Err(motion_core::Error::GpuUnavailable(Box::new(
                std::io::Error::other(
                    "Software GPU adapters require explicit allow_software opt-in",
                ),
            )));
        }
        self.loss.check()
    }

    fn from_device(
        device: wgpu::Device,
        queue: wgpu::Queue,
        info: crate::GpuInfo,
        hardware_fills: bool,
    ) -> motion_core::Result<Self> {
        pollster::block_on(async move {
            let loss = crate::gpu::DeviceMonitor::new(&device);
            let memory_scope = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            let validation_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Motion linear compute"),
                source: wgpu::ShaderSource::Wgsl(textures::shader_source().into()),
            });
            let particle_states =
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("Motion particle state precompute"),
                    layout: None,
                    module: &shader,
                    entry_point: Some("prepare_particle_states"),
                    compilation_options: Default::default(),
                    cache: None,
                });
            let particle_bins = [
                "count_particle_tiles",
                "prefix_particle_tiles",
                "fill_particle_tiles",
            ]
            .map(|entry| {
                std::sync::Arc::new(device.create_compute_pipeline(
                    &wgpu::ComputePipelineDescriptor {
                        label: Some(entry),
                        layout: None,
                        module: &shader,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        cache: None,
                    },
                ))
            });
            let textures =
                std::sync::Arc::new(TexturePipelines::new(&device, &shader, hardware_fills));
            let validation = validation_scope.pop().await;
            let memory = memory_scope.pop().await;
            if let Some(error) = validation {
                return Err(motion_core::Error::GpuUnavailable(Box::new(error)));
            }
            if let Some(error) = memory {
                return Err(motion_core::Error::GpuUnavailable(Box::new(error)));
            }
            let context = Self {
                device,
                loss: std::sync::Arc::new(loss),
                queue,
                particle_states: std::sync::Arc::new(particle_states),
                particle_bins,
                #[cfg(test)]
                fill_batches_encoded: Default::default(),
                #[cfg(test)]
                batch_resolves_encoded: Default::default(),
                #[cfg(test)]
                layer_captures_encoded: Default::default(),
                #[cfg(test)]
                working_textures_created: Default::default(),
                #[cfg(test)]
                color_image_buffers_encoded: Default::default(),
                #[cfg(test)]
                hardware_plus_encoded: Default::default(),
                #[cfg(test)]
                batch_data_buffers_encoded: Default::default(),
                #[cfg(test)]
                batch_data_copies_encoded: Default::default(),
                #[cfg(test)]
                batch_data_bytes_encoded: Default::default(),
                #[cfg(test)]
                batch_bind_groups_encoded: Default::default(),
                textures,
                present_shader: Default::default(),
                present_check: Default::default(),
                packing: Default::default(),
                device_coverage: Default::default(),
                analytic_particles: Default::default(),
                timestamp_marker: Default::default(),
                alpha_thresholds: Default::default(),
                working_pool: Default::default(),
                residency: Default::default(),
                present: std::sync::Arc::new(std::array::from_fn(|_| std::sync::OnceLock::new())),
                info,
                submissions: std::sync::Arc::new(std::sync::Mutex::new(())),
                self_check_result: std::sync::Arc::new(std::sync::OnceLock::new()),
            };
            context.warm_up()?;
            Ok(context)
        })
    }

    fn warm_up(&self) -> motion_core::Result<()> {
        let _submission = self.acquire_submission()?;
        let memory_scope = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation_scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        self.alpha_thresholds();
        self.packing_pipeline();
        self.coverage_pipelines();
        self.present_check_pipeline();
        for format in [
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Bgra8Unorm,
        ] {
            self.present_pipeline(format);
        }
        let encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Motion context warm-up"),
            });
        let submission = self.queue.submit([encoder.finish()]);
        wait_for_gpu(&self.device, Some(submission))?;
        let validation = pollster::block_on(validation_scope.pop());
        let memory = pollster::block_on(memory_scope.pop());
        self.loss.check()?;
        if let Some(error) = validation.or(memory) {
            return Err(motion_core::Error::GpuUnavailable(Box::new(error)));
        }
        Ok(())
    }

    pub(super) fn return_working(&self, lease: WorkingLease, maximum: u64) {
        let maximum = maximum.min(64 * 1024 * 1024);
        let mut pool = self.working_pool.lock_unpoisoned();
        pool.trim(maximum);
        for buffer in lease.buffers {
            let bytes = buffer.size();
            if bytes > maximum || bytes == 0 {
                continue;
            }
            if buffer.usage() == wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC {
                if pool.staging.try_reserve(1).is_ok() {
                    let mapped = std::sync::Arc::new(std::sync::OnceLock::new());
                    let completion = mapped.clone();
                    buffer
                        .slice(..)
                        .map_async(wgpu::MapMode::Write, move |result| {
                            let _ = completion.set(result.is_ok());
                        });
                    pool.bytes += bytes;
                    pool.staging.push(StagingBuffer { buffer, mapped });
                    pool.trim(maximum);
                }
            } else if buffer.usage().contains(wgpu::BufferUsages::COPY_DST)
                && pool.buffers.try_reserve(1).is_ok()
            {
                pool.bytes += bytes;
                pool.buffers.push(buffer);
                pool.trim(maximum);
            }
        }
        for texture in lease.textures {
            let bytes = u64::from(texture.width()) * u64::from(texture.height()) * 16;
            if bytes > maximum {
                continue;
            }
            if pool.textures.try_reserve(1).is_ok() {
                pool.bytes += bytes;
                pool.textures.push(texture);
                pool.trim(maximum);
            }
        }
    }

    pub(super) fn working_texture(
        &self,
        working: &mut WorkingLease,
        size: PixelSize,
        usage: wgpu::TextureUsages,
        label: &str,
    ) -> motion_core::Result<wgpu::Texture> {
        let matches = |texture: &&wgpu::Texture| {
            texture.width() == size.width
                && texture.height() == size.height
                && texture.usage() == usage
                && texture.format() == wgpu::TextureFormat::Rgba32Float
        };
        if let Some(texture) = working.textures.iter().find(matches) {
            return Ok(texture.clone());
        }
        working.textures.try_reserve(1).map_err(|_| {
            ValidationError::limit(
                "gpu.pool",
                "max_bytes",
                "Could not reserve working texture lease",
            )
        })?;
        let cached = (|pool: &mut WorkingResourceStore| {
            let index = pool.textures.iter().position(|texture| matches(&texture))?;
            let texture = pool.textures.swap_remove(index);
            pool.bytes -= u64::from(size.width) * u64::from(size.height) * 16;
            Some(texture)
        })(&mut self.working_pool.lock_unpoisoned());
        let texture = cached.unwrap_or_else(|| {
            #[cfg(test)]
            self.working_textures_created
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size.width,
                    height: size.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage,
                view_formats: &[],
            })
        });
        working.textures.push(texture.clone());
        Ok(texture)
    }

    pub(super) fn staging_buffer(&self, size: u64) -> wgpu::Buffer {
        let cached = (|pool: &mut WorkingResourceStore| {
            while let Some(index) = pool
                .staging
                .iter()
                .position(|entry| entry.mapped.get() == Some(&false))
            {
                let entry = pool.staging.swap_remove(index);
                pool.bytes -= entry.buffer.size();
            }
            let index = pool.staging.iter().position(|entry| {
                entry.buffer.size() == size && entry.mapped.get() == Some(&true)
            })?;
            let entry = pool.staging.swap_remove(index);
            pool.bytes -= entry.buffer.size();
            Some(entry.buffer)
        })(&mut self.working_pool.lock_unpoisoned());
        cached.unwrap_or_else(|| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Batch upload staging"),
                size,
                usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: true,
            })
        })
    }

    pub(super) fn working_buffer(
        &self,
        label: &str,
        size: u64,
        usage: wgpu::BufferUsages,
    ) -> wgpu::Buffer {
        let usage = usage | wgpu::BufferUsages::COPY_DST;
        let cached = (|pool: &mut WorkingResourceStore| {
            let index = pool
                .buffers
                .iter()
                .position(|buffer| buffer.size() == size && buffer.usage() == usage)?;
            let buffer = pool.buffers.swap_remove(index);
            pool.bytes -= buffer.size();
            Some(buffer)
        })(&mut self.working_pool.lock_unpoisoned());
        cached.unwrap_or_else(|| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        })
    }

    pub(super) fn upload_buffer(
        &self,
        label: &str,
        bytes: &[u8],
        usage: wgpu::BufferUsages,
    ) -> wgpu::Buffer {
        let buffer = self.working_buffer(label, bytes.len() as u64, usage);
        self.queue.write_buffer(&buffer, 0, bytes);
        buffer
    }

    pub(super) fn alpha_thresholds(&self) -> &wgpu::Buffer {
        use wgpu::util::DeviceExt;
        self.alpha_thresholds.get_or_init(|| {
            let values: [f32; 255] = std::array::from_fn(|index| {
                let target = index as f64 + 0.5;
                let value = (target / 255.0) as f32;
                if f64::from(value) * 255.0 < target {
                    value.next_up()
                } else {
                    value
                }
            });
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("exact alpha quantization thresholds"),
                    contents: bytemuck::cast_slice(&values),
                    usage: wgpu::BufferUsages::STORAGE,
                })
        })
    }

    pub(super) fn coverage_pipelines(&self) -> &CoveragePipelines {
        self.device_coverage.get_or_init(|| {
            let shader = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("GPU fill coverage"),
                    source: wgpu::ShaderSource::Wgsl(include_str!("shaders/coverage.wgsl").into()),
                });
            let entries = (0..8)
                .map(|binding| wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: if binding == 0 {
                            wgpu::BufferBindingType::Uniform
                        } else {
                            wgpu::BufferBindingType::Storage {
                                read_only: binding == 1 || binding == 7,
                            }
                        },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                })
                .collect::<Vec<_>>();
            let layout = self
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("GPU coverage tiles"),
                    entries: &entries,
                });
            let pipeline_layout =
                self.device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("GPU coverage tiles"),
                        bind_group_layouts: &[Some(&layout)],
                        immediate_size: 0,
                    });
            let passes = [
                "flatten_curves",
                "count_tiles",
                "prefix_tiles",
                "fill_tiles",
                "carry_rows",
                "coverage_pass",
            ]
            .map(|entry| {
                self.device
                    .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some(entry),
                        layout: Some(&pipeline_layout),
                        module: &shader,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        cache: None,
                    })
            });
            CoveragePipelines { layout, passes }
        })
    }

    pub(super) fn packing_pipeline(&self) -> &wgpu::ComputePipeline {
        self.packing.get_or_init(|| {
            let shader = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("persistent GPU output packing"),
                    source: wgpu::ShaderSource::Wgsl(
                        super::output_shader(include_str!("shaders/pack.wgsl")).into(),
                    ),
                });
            self.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("GPU output packing"),
                    layout: None,
                    module: &shader,
                    entry_point: Some("pack"),
                    compilation_options: Default::default(),
                    cache: None,
                })
        })
    }

    pub(super) fn timestamp_marker_pipeline(&self) -> &wgpu::ComputePipeline {
        self.timestamp_marker.get_or_init(|| {
            let shader = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("timestamp marker"),
                    source: wgpu::ShaderSource::Wgsl(include_str!("shaders/marker.wgsl").into()),
                });
            self.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("timestamp marker"),
                    layout: None,
                    module: &shader,
                    entry_point: Some("marker"),
                    compilation_options: Default::default(),
                    cache: None,
                })
        })
    }

    fn present_shader(&self) -> &wgpu::ShaderModule {
        self.present_shader.get_or_init(|| {
            self.device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("persistent texture output"),
                    source: wgpu::ShaderSource::Wgsl(
                        super::output_shader(include_str!("shaders/present.wgsl")).into(),
                    ),
                })
        })
    }

    pub(super) fn present_check_pipeline(&self) -> &wgpu::ComputePipeline {
        self.present_check.get_or_init(|| {
            self.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("cropped texture finite check"),
                    layout: None,
                    module: self.present_shader(),
                    entry_point: Some("check_outside"),
                    compilation_options: Default::default(),
                    cache: None,
                })
        })
    }

    pub(super) fn present_pipeline(&self, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        let index = match format {
            wgpu::TextureFormat::Rgba16Float => 0,
            wgpu::TextureFormat::Rgba8Unorm => 1,
            wgpu::TextureFormat::Bgra8Unorm => 2,
            _ => unreachable!("validated present format"),
        };
        self.present[index].get_or_init(|| {
            let shader = self.present_shader();
            self.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("persistent external output"),
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: shader,
                        entry_point: Some("vertex"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: shader,
                        entry_point: Some("fragment"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                })
        })
    }

    /// Idle reusable working-buffer payload retained by this shared context.
    pub fn pooled_working_bytes(&self) -> u64 {
        self.working_pool.lock_unpoisoned().bytes
    }
}
