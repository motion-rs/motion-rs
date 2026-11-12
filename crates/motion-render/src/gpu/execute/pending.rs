//! Submission completion, validated readback and resource recycling.

use super::*;
use motion_core::{PixelSize, ValidationError};

pub(super) fn linear_pixels(
    mapped: &[u8],
    raster_width: u32,
    output_bounds: PixelRect,
    path: &str,
    allocation_error: &str,
    color_error: &str,
) -> Result<Vec<[f32; 4]>, ValidationError> {
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(output_bounds.pixels() as usize)
        .map_err(|_| ValidationError::limit(path, "max_bytes", allocation_error))?;
    if cfg!(target_endian = "little")
        && let Ok(source) = bytemuck::try_cast_slice::<u8, [f32; 4]>(mapped)
    {
        for row in output_bounds.top..output_bounds.bottom {
            let start = row as usize * raster_width as usize + output_bounds.left as usize;
            let end = start + (output_bounds.right - output_bounds.left) as usize;
            let row = &source[start..end];
            if row.iter().flatten().any(|value| !value.is_finite()) {
                return Err(ValidationError::invalid(path, "color", color_error));
            }
            pixels.extend_from_slice(row);
        }
    } else {
        for row in output_bounds.top..output_bounds.bottom {
            for column in output_bounds.left..output_bounds.right {
                let offset = (row as usize * raster_width as usize + column as usize) * 16;
                let pixel = std::array::from_fn(|channel| {
                    let start = offset + channel * 4;
                    f32::from_le_bytes(
                        mapped[start..start + 4]
                            .try_into()
                            .expect("four-byte readback component"),
                    )
                });
                if pixel.iter().any(|value| !value.is_finite()) {
                    return Err(ValidationError::invalid(path, "color", color_error));
                }
                pixels.push(pixel);
            }
        }
    }
    Ok(pixels)
}

impl PendingTextureFrame {
    /// Keeps this submission's timestamp result separate from subsequent frames.
    pub fn timing_handle(&self) -> Option<GpuTimingHandle> {
        self.timestamps
            .as_ref()
            .map(crate::gpu::timing::TimestampRecorder::handle)
    }

    /// Polls only for completion; a ready frame still requires finish for errors.
    pub fn is_ready(&mut self) -> motion_core::Result<bool> {
        self.context
            .device
            .poll(wgpu::PollType::Poll)
            .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?;
        self.context.loss.check()?;
        if !self.mapped {
            match self.completion.try_recv() {
                Ok(Ok(())) => self.mapped = true,
                Ok(Err(error)) => return Err(motion_core::Error::Gpu(Box::new(error))),
                Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(false),
                Err(error) => return Err(motion_core::Error::Gpu(Box::new(error))),
            }
        }
        if let Some(timestamps) = &mut self.timestamps {
            return timestamps.is_ready();
        }
        Ok(true)
    }

    /// Waits for this submission and verifies status, validation and device health.
    pub fn finish(mut self) -> motion_core::Result<()> {
        wait_for_gpu(&self.context.device, Some(self.submission))?;
        self.context.loss.check()?;
        if !self.mapped {
            self.completion
                .recv()
                .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?
                .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?;
            self.mapped = true;
        }
        let validation = pollster::block_on(self.validation);
        let memory = pollster::block_on(self.memory);
        self.context.loss.check()?;
        let failed = self
            .readback
            .slice(..)
            .get_mapped_range()
            .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?
            .iter()
            .any(|byte| *byte != 0);
        self.readback.unmap();
        if let Some(error) = memory.or(validation) {
            return Err(motion_core::Error::Gpu(Box::new(error)));
        }
        if failed {
            return Err(ValidationError::invalid(
                "gpu.texture",
                "color",
                "Nonfinite GPU working color",
            )
            .into());
        }
        let timestamps = self
            .timestamps
            .map(crate::gpu::timing::TimestampRecorder::finish)
            .transpose()?;
        crate::gpu::timing::TimestampRecorder::publish(timestamps);
        self.working.push(self.readback);
        self.context.return_working(self.working, self.maximum);
        Ok(())
    }
}

impl PendingLinearFrame {
    /// Keeps this submission's timestamp result separate from subsequent frames.
    #[cfg(test)]
    pub(crate) fn timing_handle(&self) -> Option<GpuTimingHandle> {
        self.timestamps
            .as_ref()
            .map(crate::gpu::timing::TimestampRecorder::handle)
    }

    /// Waits and consumes the frame; another pending frame may already be executing.
    #[cfg(test)]
    pub(crate) fn finish(self) -> motion_core::Result<LinearFrame> {
        self.finish_reusable().map(|(frame, _, _, _)| frame)
    }

    pub(super) fn finish_reusable(
        self,
    ) -> motion_core::Result<(LinearFrame, u64, wgpu::Buffer, wgpu::Buffer)> {
        let _submission = self.context.acquire_submission()?;
        wait_for_gpu(&self.context.device, Some(self.submission))?;
        self.context.loss.check()?;
        let validation = pollster::block_on(self.validation);
        let memory = pollster::block_on(self.memory);
        if let Some(error) = memory.or(validation) {
            return Err(motion_core::Error::Gpu(Box::new(error)));
        }
        for _ in 0..2 {
            self.completion
                .recv()
                .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?
                .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?;
        }
        let result = (|| {
            let status = self
                .status
                .slice(..)
                .get_mapped_range()
                .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?;
            if status.iter().any(|byte| *byte != 0) {
                return Err(ValidationError::invalid(
                    "gpu.pending",
                    "color",
                    "Nonfinite GPU working color",
                )
                .into());
            }
            let mapped = self
                .readback
                .slice(..)
                .get_mapped_range()
                .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?;
            let pixels = linear_pixels(
                &mapped,
                self.raster_width,
                self.output_bounds,
                "gpu.pending",
                "Could not reserve pending output",
                "Nonfinite GPU readback",
            )?;
            Ok(crate::cpu::LinearFrame::from_gpu(
                PixelSize::new(
                    self.output_bounds.right - self.output_bounds.left,
                    self.output_bounds.bottom - self.output_bounds.top,
                ),
                pixels,
                self.stats,
            ))
        })();
        self.readback.unmap();
        self.status.unmap();
        if result.is_ok() {
            let timestamps = self
                .timestamps
                .map(crate::gpu::timing::TimestampRecorder::finish)
                .transpose()?;
            crate::gpu::timing::TimestampRecorder::publish(timestamps);
            self.context.return_working(self.working, self.cache_limit);
        }
        result.map(|frame| {
            let bytes = self.readback.size();
            (frame, bytes, self.readback, self.status)
        })
    }
}

impl PendingOutputFrame {
    /// Keeps this submission's timestamp result separate from subsequent frames.
    #[cfg(test)]
    pub(crate) fn timing_handle(&self) -> Option<GpuTimingHandle> {
        self.timestamps
            .as_ref()
            .map(crate::gpu::timing::TimestampRecorder::handle)
    }

    /// Exact transferred payload bytes, excluding the four-byte finite/alpha status.
    #[cfg(test)]
    pub(crate) fn readback_bytes(&self) -> u64 {
        self.readback.size()
    }

    /// Waits for this frame, validates its scope/status, then returns typed packed planes.
    pub fn finish(self) -> motion_core::Result<crate::FrameBuffer> {
        self.finish_reusable().map(|(frame, _, _, _, _)| frame)
    }

    pub(super) fn finish_reusable(
        self,
    ) -> motion_core::Result<(
        crate::FrameBuffer,
        u64,
        wgpu::Buffer,
        wgpu::Buffer,
        wgpu::Buffer,
    )> {
        let _submission = self.context.acquire_submission()?;
        wait_for_gpu(&self.context.device, Some(self.submission))?;
        self.context.loss.check()?;
        let validation = pollster::block_on(self.validation);
        let memory = pollster::block_on(self.memory);
        if let Some(error) = memory.or(validation) {
            return Err(motion_core::Error::Gpu(Box::new(error)));
        }
        for _ in 0..2 {
            self.completion
                .recv()
                .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?
                .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?;
        }
        let result = (|| {
            let status = self
                .status
                .slice(..)
                .get_mapped_range()
                .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?;
            let status = status
                .get(..4)
                .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
                .map(u32::from_le_bytes)
                .ok_or_else(|| {
                    ValidationError::invalid(
                        "gpu.pack",
                        "status",
                        "Pack status readback is shorter than four bytes",
                    )
                })?;
            if status & 1 != 0 {
                return Err(ValidationError::invalid(
                    "gpu.pack",
                    "color",
                    "Nonfinite GPU working color",
                )
                .into());
            }
            if status & 2 != 0 {
                return Err(ValidationError::invalid(
                    "output",
                    "alpha",
                    "YUV420 requires an opaque background",
                )
                .into());
            }
            let mapped = self
                .readback
                .slice(..)
                .get_mapped_range()
                .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?;
            match self.format {
                crate::PixelFormat::SrgbRgba8 { premultiplied } => {
                    crate::FrameBuffer::from_rgba8_storage(
                        self.size,
                        mapped.to_vec(),
                        premultiplied,
                        self.limits,
                    )
                    .map_err(Into::into)
                }
                crate::PixelFormat::Yuv420 { bit_depth } => {
                    let count = self.size.width as usize * self.size.height as usize;
                    let luma_bytes = count * if bit_depth == 8 { 1 } else { 2 };
                    crate::FrameBuffer::from_yuv420_storage(
                        self.size,
                        [
                            mapped[..luma_bytes].to_vec(),
                            mapped[luma_bytes..luma_bytes + luma_bytes / 4].to_vec(),
                            mapped[luma_bytes + luma_bytes / 4..luma_bytes + luma_bytes / 2]
                                .to_vec(),
                        ],
                        bit_depth,
                        self.limits,
                    )
                    .map_err(Into::into)
                }
                _ => unreachable!("validated packed output"),
            }
        })();
        self.readback.unmap();
        self.status.unmap();
        if result.is_ok() {
            let timestamps = self
                .timestamps
                .map(crate::gpu::timing::TimestampRecorder::finish)
                .transpose()?;
            crate::gpu::timing::TimestampRecorder::publish(timestamps);
            self.context
                .return_working(self.working, self.limits.max_bytes / 4);
        }
        result.map(|frame| {
            let bytes = self.readback.size();
            (frame, bytes, self.readback, self.status, self.packed_output)
        })
    }
}
