//! Frame submission, output packing and synchronous wrappers.

use super::*;
use motion_core::PixelSize;
use motion_core::sync::MutexExt;

impl Gpu {
    /// Whether this size/format can be packed without byte-write races on the GPU.
    pub fn supports_packed_output(size: PixelSize, format: crate::PixelFormat) -> bool {
        match format {
            crate::PixelFormat::SrgbRgba8 { .. } => true,
            crate::PixelFormat::Yuv420 { bit_depth: 8 | 10 } => {
                size.width.is_multiple_of(2) && size.height.is_multiple_of(2)
            }
            _ => false,
        }
    }

    /// Submits RGBA8 or aligned 8/10-bit YUV output and reads back only packed bytes.
    /// Accepts dedicated packed plans or conservative linear plans with extra packing charges.
    pub(crate) fn submit_output(
        &mut self,
        frame: &PreparedScene,
        mut plan: Plan,
        format: crate::PixelFormat,
    ) -> motion_core::Result<PendingOutputFrame> {
        use wgpu::util::DeviceExt;
        self.begin_timings();
        let started = self.timing.as_ref().map(|_| std::time::Instant::now());
        plan.stats = plan.packed_stats(frame.size(), format)?;
        let context = self.context.clone();
        let _submission = context.acquire_submission()?;
        plan.validate_device(&self.device.limits())?;
        let memory_scope = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation_scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let encoded = self.encode_texture(&plan);
        let super::textures::EncodedTexture {
            mut encoder,
            texture,
            bounds,
            deferred,
            status,
            working,
            mut timestamps,
            ..
        } = match encoded {
            Ok(encoded) => encoded,
            Err(error) => {
                wait_for_gpu(&self.device, None)?;
                let _ = pollster::block_on(validation_scope.pop());
                let _ = pollster::block_on(memory_scope.pop());
                return Err(error);
            }
        };
        let size = frame.size();
        let count = u64::from(size.width) * u64::from(size.height);
        let tiled = size.width.is_multiple_of(8);
        let (bytes, mode, invocations) = match format {
            crate::PixelFormat::SrgbRgba8 { premultiplied } => {
                (count * 4, u32::from(premultiplied), count)
            }
            crate::PixelFormat::Yuv420 { bit_depth: 8 } if tiled => (count * 3 / 2, 2, count / 16),
            crate::PixelFormat::Yuv420 { bit_depth: 8 } => {
                let bytes = (count * 3 / 2).next_multiple_of(4);
                (bytes, 4, bytes / 4)
            }
            crate::PixelFormat::Yuv420 { bit_depth: 10 } if tiled => (count * 3, 3, count / 16),
            crate::PixelFormat::Yuv420 { bit_depth: 10 } => (count * 3, 5, count * 3 / 4),
            _ => unreachable!("validated packing format"),
        };
        let packed = self
            .packed_output
            .take()
            .filter(|(size, _)| *size == bytes)
            .map(|(_, buffer)| buffer)
            .unwrap_or_else(|| {
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("GPU packed output"),
                    size: bytes,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                })
            });
        let source_size = image_size(bounds);
        let scan_outside = bounds.left < frame.output_bounds.left
            || bounds.top < frame.output_bounds.top
            || bounds.right > frame.output_bounds.right
            || bounds.bottom > frame.output_bounds.bottom;
        let invocations = if scan_outside {
            invocations.max(u64::from(source_size.width) * u64::from(source_size.height))
        } else {
            invocations
        };
        let groups = (invocations as u32).div_ceil(64);
        let groups_x = groups.min(self.device.limits().max_compute_workgroups_per_dimension);
        let groups_y = groups.div_ceil(groups_x);
        let mut words = vec![
            frame.output_bounds.left,
            frame.output_bounds.top,
            size.width,
            size.height,
            bounds.left,
            bounds.top,
            bounds.right,
            bounds.bottom,
            mode,
            groups_x,
            u32::from(deferred),
            u32::from(scan_outside),
        ];
        words.extend(plan.display);
        let settings = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("GPU packing settings"),
                contents: bytemuck::cast_slice(&words),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let pipeline = self.packing_pipeline();
        let view = texture.create_view(&Default::default());
        let lut = self.upload_buffer(
            "Output LUT",
            bytemuck::cast_slice(&plan.lut_data),
            wgpu::BufferUsages::STORAGE,
        );
        let bindings = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("GPU packing bindings"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: settings.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: packed.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: status.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.alpha_thresholds().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: lut.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Pack GPU output"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups(groups_x, groups_y, 1);
        }
        let reused = self.readback.take().filter(|(size, _, _)| *size == bytes);
        let (readback, status_readback) = if let Some((_, pixels, status)) = reused {
            (pixels, status)
        } else {
            let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("packed output readback"),
                size: bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let status_readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("packed status readback"),
                size: 4,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            (readback, status_readback)
        };
        if let Some(timestamps) = &timestamps {
            timestamps.write(&mut encoder, 3);
            timestamps.write(&mut encoder, 4);
        }
        encoder.copy_buffer_to_buffer(&packed, 0, &readback, 0, bytes);
        encoder.copy_buffer_to_buffer(&status, 0, &status_readback, 0, 4);
        if let Some(timestamps) = &timestamps {
            timestamps.write(&mut encoder, 5);
            timestamps.resolve(&mut encoder);
        }
        let submission = self.queue.submit(Some(encoder.finish()));
        if let Some(timestamps) = &mut timestamps {
            timestamps.map();
        }
        let (sender, completion) = std::sync::mpsc::channel();
        for buffer in [&readback, &status_readback] {
            let sender = sender.clone();
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = sender.send(result);
                });
        }
        if let Some(started) = started {
            self.encode_time = started.elapsed();
        }
        Ok(PendingOutputFrame {
            context: self.context.clone(),
            submission,
            readback,
            packed_output: packed,
            status: status_readback,
            completion,
            validation: Box::pin(validation_scope.pop()),
            memory: Box::pin(memory_scope.pop()),
            size,
            format,
            limits: plan.limits,
            working,
            timestamps,
        })
    }

    pub(crate) fn render_output(
        &mut self,
        frame: &PreparedScene,
        plan: Plan,
        format: crate::PixelFormat,
    ) -> motion_core::Result<crate::FrameBuffer> {
        let pending = self.submit_output(frame, plan, format)?;
        let started = self.timing.as_ref().map(|_| std::time::Instant::now());
        let (frame, bytes, readback, status, packed) = pending.finish_reusable()?;
        if let Some(started) = started {
            self.readback_time = started.elapsed();
        }
        self.readback = Some((bytes, readback, status));
        self.packed_output = Some((bytes, packed));
        Ok(frame)
    }

    /// Submits without waiting for readback. The caller must bound all in-flight plans.
    pub(crate) fn submit_linear(
        &mut self,
        frame: &PreparedScene,
        plan: Plan,
    ) -> motion_core::Result<PendingLinearFrame> {
        if plan.packed_output.is_some() {
            return Err(motion_core::ValidationError::invalid(
                "gpu.output",
                "format",
                "Packed plans cannot submit floating-point readback",
            )
            .into());
        }
        self.begin_timings();
        let started = self.timing.as_ref().map(|_| std::time::Instant::now());
        let context = self.context.clone();
        let _submission = context.acquire_submission()?;
        plan.validate_device(&self.device.limits())?;
        let memory_scope = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation_scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let encoded = self.encode(&plan);
        let super::textures::EncodedLinear {
            mut encoder,
            output,
            status,
            stats,
            working,
            mut timestamps,
        } = match encoded {
            Ok(encoded) => encoded,
            Err(error) => {
                wait_for_gpu(&self.device, None)?;
                let _ = pollster::block_on(validation_scope.pop());
                let _ = pollster::block_on(memory_scope.pop());
                return Err(error);
            }
        };
        let bytes = u64::from(plan.size.width) * u64::from(plan.size.height) * 16;
        let reused = self.readback.take().filter(|(size, _, _)| *size == bytes);
        let (readback, status_readback) = if let Some((_, pixels, status)) = reused {
            (pixels, status)
        } else {
            let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pending linear readback"),
                size: bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let status_readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pending status readback"),
                size: 4,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            (readback, status_readback)
        };
        if let Some(timestamps) = &timestamps {
            timestamps.write(&mut encoder, 3);
            timestamps.write(&mut encoder, 4);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
        encoder.copy_buffer_to_buffer(&status, 0, &status_readback, 0, 4);
        if let Some(timestamps) = &timestamps {
            timestamps.write(&mut encoder, 5);
            timestamps.resolve(&mut encoder);
        }
        let submission = self.queue.submit(Some(encoder.finish()));
        if let Some(timestamps) = &mut timestamps {
            timestamps.map();
        }
        let (sender, completion) = std::sync::mpsc::channel();
        for buffer in [&readback, &status_readback] {
            let sender = sender.clone();
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = sender.send(result);
                });
        }
        if let Some(started) = started {
            self.encode_time = started.elapsed();
        }
        Ok(PendingLinearFrame {
            context: self.context.clone(),
            submission,
            readback,
            status: status_readback,
            completion,
            validation: Box::pin(validation_scope.pop()),
            memory: Box::pin(memory_scope.pop()),
            raster_width: plan.size.width,
            output_bounds: frame.output_bounds,
            stats,
            working,
            cache_limit: plan.limits.max_bytes / 4,
            timestamps,
        })
    }

    /// Creates a session with a newly initialized supported graphics device.
    pub fn new() -> motion_core::Result<Self> {
        Ok(Self::from_context(GpuContext::new()?))
    }

    /// Shares an existing device and its submission lock without allocating a new device.
    pub fn from_context(context: GpuContext) -> Self {
        Self {
            context,
            readback: None,
            packed_output: None,
            timing: None,
            encode_time: std::time::Duration::ZERO,
            readback_time: std::time::Duration::ZERO,
        }
    }

    /// Borrows the shared device owner for resource creation and session reuse.
    pub fn context(&self) -> &GpuContext {
        &self.context
    }

    /// Idle readback capacity retained by this session, excluding active plan storage.
    pub fn pooled_readback_bytes(&self) -> u64 {
        self.readback.as_ref().map_or(0, |(bytes, _, _)| bytes + 4)
    }

    /// Idle packed output storage retained by this session, excluding active frames.
    pub fn pooled_output_bytes(&self) -> u64 {
        self.packed_output.as_ref().map_or(0, |(bytes, _)| *bytes)
    }

    /// Reports the actual selected adapter without probing a second device.
    pub fn info(&self) -> &crate::GpuInfo {
        &self.info
    }

    /// Logical peak bytes and work of the plan [`Gpu::render_linear`] would execute, computed
    /// without a device.
    pub fn plan_stats(
        frame: &PreparedFrame,
        limits: RasterLimits,
    ) -> Result<crate::WorkStats, ValidationError> {
        Ok(Plan::new(frame, limits)?.stats)
    }

    /// Plans `frame` within `limits`, executes it synchronously and returns the read-back
    /// pixels as unquantized [`crate::PixelFormat::LinearRgba32Float`] output; convert them
    /// with [`crate::FrameBuffer::to_format`]. Planning admits the storage before device access.
    /// Device loss, driver validation or nonfinite arithmetic fails the entire frame.
    pub fn render_linear(
        &mut self,
        frame: &PreparedFrame,
        limits: RasterLimits,
    ) -> motion_core::Result<crate::FrameBuffer> {
        let linear = self.render(frame, Plan::new(frame, limits)?)?;
        crate::FrameBuffer::linear_unbounded(&linear).map_err(Into::into)
    }

    /// Executes an admitted plan synchronously and returns finite linear-premultiplied pixels.
    /// Device loss, driver validation or nonfinite arithmetic fails the entire frame.
    pub(crate) fn render(
        &mut self,
        frame: &PreparedScene,
        plan: Plan,
    ) -> motion_core::Result<LinearFrame> {
        self.begin_timings();
        let context = self.context.clone();
        let _submission = context.submissions.lock_unpoisoned();
        self.loss.check()?;
        plan.validate_device(&self.device.limits())?;
        let memory_scope = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation_scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let result = self.submit(frame, &plan);
        wait_for_gpu(&self.device, None)?;
        self.loss.check()?;
        let validation = pollster::block_on(validation_scope.pop());
        let memory = pollster::block_on(memory_scope.pop());
        if let Some(error) = memory {
            return Err(motion_core::Error::Gpu(Box::new(error)));
        }
        if let Some(error) = validation {
            return Err(motion_core::Error::Gpu(Box::new(error)));
        }
        let (frame, timestamps) = result?;
        super::super::timing::TimestampRecorder::publish(timestamps);
        Ok(frame)
    }

    pub(in crate::gpu) fn render_texture(
        &mut self,
        frame: &PreparedScene,
        plan: Plan,
        texture: &wgpu::Texture,
        format: crate::PixelFormat,
    ) -> motion_core::Result<()> {
        self.submit_texture(frame, plan, texture, format)?.finish()
    }

    pub(crate) fn submit_texture(
        &mut self,
        frame: &PreparedScene,
        plan: Plan,
        texture: &wgpu::Texture,
        format: crate::PixelFormat,
    ) -> motion_core::Result<PendingTextureFrame> {
        if plan.packed_output.is_some() {
            return Err(motion_core::ValidationError::invalid(
                "gpu.output",
                "format",
                "Packed readback plans cannot submit external textures",
            )
            .into());
        }
        self.begin_timings();
        let started = self.timing.as_ref().map(|_| std::time::Instant::now());
        let linear = match (format, texture.format()) {
            (crate::PixelFormat::LinearRgba16Float, wgpu::TextureFormat::Rgba16Float) => true,
            (
                crate::PixelFormat::SrgbRgba8 { .. },
                wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm,
            ) => false,
            _ => {
                return Err(device_error(
                    "texture_format",
                    "Texture/output color format mismatch or unsupported texture format",
                )
                .into());
            }
        };
        let extent = texture.size();
        if extent.width != frame.size().width
            || extent.height != frame.size().height
            || extent.depth_or_array_layers != 1
            || texture.sample_count() != 1
            || texture.dimension() != wgpu::TextureDimension::D2
            || texture.mip_level_count() != 1
            || !texture
                .usage()
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        {
            return Err(device_error(
                "texture",
                "Target must be a matching single-layer/single-sample render attachment",
            )
            .into());
        }
        let context = self.context.clone();
        let _submission = context.submissions.lock_unpoisoned();
        self.loss.check()?;
        plan.validate_device(&self.device.limits())?;
        let memory_scope = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation_scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let result = (|| -> motion_core::Result<_> {
            let super::textures::EncodedTexture {
                mut encoder,
                texture: source,
                bounds,
                deferred,
                status,
                mut working,
                mut timestamps,
                ..
            } = self.encode_texture(&plan)?;
            let pipeline = self.present_pipeline(texture.format());
            let mut words = vec![
                frame.output_bounds.left,
                frame.output_bounds.top,
                frame.size().width,
                frame.size().height,
                bounds.left,
                bounds.top,
                bounds.right,
                bounds.bottom,
                u32::from(linear),
                u32::from(matches!(
                    format,
                    crate::PixelFormat::SrgbRgba8 {
                        premultiplied: true
                    }
                )),
                u32::from(deferred),
                0,
            ];
            words.extend(plan.display);
            let uniform = self.upload_buffer(
                "output settings",
                bytemuck::cast_slice(&words),
                wgpu::BufferUsages::UNIFORM,
            );
            working.push(uniform.clone());
            let source_view = source.create_view(&Default::default());
            let lut = self.upload_buffer(
                "Output LUT",
                bytemuck::cast_slice(&plan.lut_data),
                wgpu::BufferUsages::STORAGE,
            );
            working.push(lut.clone());
            let entries = [
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&source_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: status.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: lut.as_entire_binding(),
                },
            ];
            if bounds.left < frame.output_bounds.left
                || bounds.top < frame.output_bounds.top
                || bounds.right > frame.output_bounds.right
                || bounds.bottom > frame.output_bounds.bottom
            {
                let check = self.present_check_pipeline();
                let bindings = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("cropped texture check bindings"),
                    layout: &check.get_bind_group_layout(0),
                    entries: &entries[..3],
                });
                let size = image_size(bounds);
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Check final texture outside crop"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(check);
                pass.set_bind_group(0, &bindings, &[]);
                pass.dispatch_workgroups(size.width.div_ceil(8), size.height.div_ceil(8), 1);
            }
            let bindings = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("output bindings"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &entries,
            });
            let view = texture.create_view(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("external texture pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &bindings, &[]);
                pass.draw(0..3, 0..1);
            }
            let readback =
                self.upload_buffer("finite status only", &[0; 4], wgpu::BufferUsages::MAP_READ);
            if let Some(timestamps) = &timestamps {
                timestamps.write(&mut encoder, 3);
                timestamps.write(&mut encoder, 4);
            }
            encoder.copy_buffer_to_buffer(&status, 0, &readback, 0, 4);
            if let Some(timestamps) = &timestamps {
                timestamps.write(&mut encoder, 5);
                timestamps.resolve(&mut encoder);
            }
            let submission = self.queue.submit(Some(encoder.finish()));
            if let Some(timestamps) = &mut timestamps {
                timestamps.map();
            }
            let (sender, receiver) = std::sync::mpsc::channel();
            readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = sender.send(result);
                });
            Ok((submission, readback, receiver, working, timestamps))
        })();
        let validation = Box::pin(validation_scope.pop());
        let memory = Box::pin(memory_scope.pop());
        if let Some(started) = started {
            self.encode_time = started.elapsed();
        }
        match result {
            Ok((submission, readback, completion, working, timestamps)) => {
                Ok(PendingTextureFrame {
                    context: self.context.clone(),
                    submission,
                    readback,
                    completion,
                    mapped: false,
                    validation,
                    memory,
                    working,
                    maximum: plan.limits.max_bytes / 4,
                    timestamps,
                })
            }
            Err(error) => {
                wait_for_gpu(&self.device, None)?;
                let _ = pollster::block_on(validation);
                let _ = pollster::block_on(memory);
                Err(error)
            }
        }
    }

    fn submit(
        &mut self,
        frame: &PreparedScene,
        plan: &Plan,
    ) -> motion_core::Result<(LinearFrame, Option<super::super::timing::CompletedTimings>)> {
        let started = self.timing.as_ref().map(|_| std::time::Instant::now());
        let bytes = u64::from(plan.size.width) * u64::from(plan.size.height) * 16;
        let super::textures::EncodedLinear {
            mut encoder,
            output,
            status,
            stats,
            working,
            mut timestamps,
        } = self.encode(plan)?;
        let reused = self.readback.take().filter(|(size, _, _)| *size == bytes);
        let (readback, status_readback) = if let Some((_, pixels, status)) = reused {
            (pixels, status)
        } else {
            let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("linear readback"),
                size: bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let status_readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("finite-color readback"),
                size: 4,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            (readback, status_readback)
        };
        if let Some(timestamps) = &timestamps {
            timestamps.write(&mut encoder, 3);
            timestamps.write(&mut encoder, 4);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
        encoder.copy_buffer_to_buffer(&status, 0, &status_readback, 0, 4);
        if let Some(timestamps) = &timestamps {
            timestamps.write(&mut encoder, 5);
            timestamps.resolve(&mut encoder);
        }
        self.queue.submit(Some(encoder.finish()));
        if let Some(timestamps) = &mut timestamps {
            timestamps.map();
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        for buffer in [&readback, &status_readback] {
            let sender = sender.clone();
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = sender.send(result);
                });
        }
        if let Some(started) = started {
            self.encode_time = started.elapsed();
        }
        let completed = self.timing.as_ref().map(|_| std::time::Instant::now());
        wait_for_gpu(&self.device, None)?;
        self.loss.check()?;
        let result = (|| {
            for _ in 0..2 {
                receiver
                    .recv()
                    .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?
                    .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?;
            }
            let status = status_readback
                .slice(..)
                .get_mapped_range()
                .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?;
            if status.iter().any(|byte| *byte != 0) {
                return Err(ValidationError::invalid(
                    "gpu.compute",
                    "color",
                    "Native GPU arithmetic exceeded finite f32 working storage",
                )
                .into());
            }
            let mapped = readback
                .slice(..)
                .get_mapped_range()
                .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?;
            let pixels = super::pending::linear_pixels(
                &mapped,
                frame.size.width,
                frame.output_bounds,
                "gpu.output",
                "Could not reserve native GPU output",
                "Native GPU readback must remain finite",
            )?;
            Ok(crate::cpu::LinearFrame::from_gpu(
                frame.size(),
                pixels,
                stats,
            ))
        })();
        readback.unmap();
        status_readback.unmap();
        let timings = if result.is_ok() {
            timestamps
                .map(super::super::timing::TimestampRecorder::finish)
                .transpose()?
        } else {
            None
        };
        if result.is_ok() {
            self.context
                .return_working(working, plan.limits.max_bytes / 4);
        }
        if bytes <= plan.limits.max_bytes / 4 && bytes <= 64 * 1024 * 1024 {
            self.readback = Some((bytes, readback, status_readback));
        }
        if let Some(completed) = completed {
            self.readback_time = completed.elapsed();
        }
        result.map(|frame| (frame, timings))
    }
}
