//! Batch staging and shared vertex/uniform/data uploads with device-limit fallbacks.

use super::{GpuContext, Plan, Step};

pub(super) struct BatchUploads<'a> {
    pub(super) context: &'a GpuContext,
    pub(super) staging: Option<&'a wgpu::Buffer>,
    pub(super) mapped: Option<wgpu::BufferViewMut>,
    pub(super) offset: u64,
}

impl BatchUploads<'_> {
    pub(super) fn upload(
        &mut self,
        label: &str,
        bytes: &[u8],
        usage: wgpu::BufferUsages,
        encoder: &mut wgpu::CommandEncoder,
    ) -> wgpu::Buffer {
        #[cfg(test)]
        if usage == wgpu::BufferUsages::STORAGE {
            self.context
                .batch_data_buffers_encoded
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.context
                .batch_data_bytes_encoded
                .fetch_add(bytes.len() as u64, std::sync::atomic::Ordering::Relaxed);
        }
        let Some(staging) = self.staging else {
            return self.context.upload_buffer(label, bytes, usage);
        };
        let buffer = self
            .context
            .working_buffer(label, bytes.len() as u64, usage);
        let end = self.offset + bytes.len() as u64;
        self.mapped
            .as_mut()
            .unwrap()
            .slice(self.offset as usize..end as usize)
            .copy_from_slice(bytes);
        encoder.copy_buffer_to_buffer(staging, self.offset, &buffer, 0, bytes.len() as u64);
        #[cfg(test)]
        if usage == wgpu::BufferUsages::STORAGE {
            self.context
                .batch_data_copies_encoded
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        self.offset = end;
        buffer
    }

    pub(super) fn upload_vertices(
        &mut self,
        steps: &[Step],
        encoder: &mut wgpu::CommandEncoder,
    ) -> Option<wgpu::Buffer> {
        let bytes = Plan::batch_vertex_bytes(steps, self.context.device.limits().max_buffer_size)?;
        let buffer = self.context.working_buffer(
            "Shared batch instances",
            bytes,
            wgpu::BufferUsages::VERTEX,
        );
        let start = self.offset;
        let mut offset = 0;
        for step in steps {
            let rectangle = step.parameters.rectangle();
            let source = if step.quads.is_empty() {
                bytemuck::cast_slice(&rectangle)
            } else {
                bytemuck::cast_slice(&step.quads)
            };
            if self.staging.is_some() {
                self.mapped
                    .as_mut()
                    .unwrap()
                    .slice(
                        (start + offset) as usize..(start + offset + source.len() as u64) as usize,
                    )
                    .copy_from_slice(source);
            } else {
                self.context.queue.write_buffer(&buffer, offset, source);
            }
            offset += source.len() as u64;
        }
        if let Some(staging) = self.staging {
            encoder.copy_buffer_to_buffer(staging, start, &buffer, 0, bytes);
            self.offset += bytes;
        }
        Some(buffer)
    }

    pub(super) fn upload_uniforms(
        &mut self,
        steps: &[Step],
        deferred: &[bool],
        encoder: &mut wgpu::CommandEncoder,
    ) -> Option<wgpu::Buffer> {
        let limits = self.context.device.limits();
        let bytes = Plan::batch_uniform_bytes(
            steps,
            limits.max_buffer_size,
            limits.min_uniform_buffer_offset_alignment,
        )?;
        let buffer = self.context.working_buffer(
            "Shared batch parameters",
            bytes,
            wgpu::BufferUsages::UNIFORM,
        );
        let start = self.offset;
        let mut offset = 0;
        for step in steps {
            let mut uniforms = step.parameters;
            uniforms.flags.second_deferred = u32::from(deferred[step.second]);
            let source = bytemuck::bytes_of(&uniforms);
            if self.staging.is_some() {
                self.mapped
                    .as_mut()
                    .unwrap()
                    .slice(
                        (start + offset) as usize..(start + offset + source.len() as u64) as usize,
                    )
                    .copy_from_slice(source);
            } else {
                self.context.queue.write_buffer(&buffer, offset, source);
            }
            offset += source.len() as u64;
        }
        if let Some(staging) = self.staging {
            encoder.copy_buffer_to_buffer(staging, start, &buffer, 0, bytes);
            self.offset += bytes;
        }
        Some(buffer)
    }

    pub(super) fn upload_data(
        &mut self,
        steps: &[Step],
        bytes: Option<u64>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Option<wgpu::Buffer> {
        let bytes = bytes?;
        let buffer = self.context.working_buffer(
            "Shared batch operation data",
            bytes,
            wgpu::BufferUsages::STORAGE,
        );
        #[cfg(test)]
        {
            self.context
                .batch_data_buffers_encoded
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.context
                .batch_data_bytes_encoded
                .fetch_add(buffer.size(), std::sync::atomic::Ordering::Relaxed);
        }
        let start = self.offset;
        let alignment = u64::from(Plan::BATCH_DATA_ALIGNMENT);
        let mut offset = 0u64;
        for step in steps {
            let next = offset.div_ceil(alignment) * alignment;
            let source = bytemuck::cast_slice(&step.data);
            if self.staging.is_some() {
                let mapped = self.mapped.as_mut().unwrap();
                mapped
                    .slice((start + offset) as usize..(start + next) as usize)
                    .fill(0);
                mapped
                    .slice((start + next) as usize..(start + next + source.len() as u64) as usize)
                    .copy_from_slice(source);
            } else {
                self.context.queue.write_buffer(&buffer, next, source);
            }
            offset = next + source.len() as u64;
        }
        assert_eq!(offset, bytes);
        if let Some(staging) = self.staging {
            encoder.copy_buffer_to_buffer(staging, start, &buffer, 0, bytes);
            #[cfg(test)]
            self.context
                .batch_data_copies_encoded
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.offset += bytes;
        }
        Some(buffer)
    }
}
