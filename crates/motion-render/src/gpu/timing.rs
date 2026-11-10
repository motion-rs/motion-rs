//! Optional timestamp queries owned by one submitted frame.

use super::*;
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Duration;

const QUERY_COUNT: u32 = 6;

const QUERY_BYTES: u64 = QUERY_COUNT as u64 * 8;

pub(super) type CompletedTimings = (GpuTimingHandle, GpuTimings);

/// Device timestamps, separate from CPU submission and completion wall time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuTimings {
    /// Device execution of composition, filters and their encoder copies.
    pub composition: Duration,
    /// Device execution of final packing, resolve or texture presentation.
    pub output: Duration,
    /// Device copies of frame output and finite status into readback storage.
    /// CPU mapping, waiting and output extraction are not included.
    pub readback_copy: Duration,
    /// Fixed logical query resolve/readback payload, separate from raster totals.
    /// Query objects and driver allocations are not process memory measurements.
    pub instrumentation_bytes: u64,
}

/// Independently retained result of one frame, available only after successful completion.
#[derive(Clone, Debug, Default)]
pub struct GpuTimingHandle(Arc<OnceLock<GpuTimings>>);

impl GpuTimingHandle {
    /// Returns device timestamps only after the owning frame passes all checks.
    pub fn get(&self) -> Option<GpuTimings> {
        self.0.get().copied()
    }
}

pub(super) struct TimestampRecorder {
    queries: wgpu::QuerySet,
    marker: wgpu::ComputePipeline,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    completion: Option<mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
    mapped: bool,
    period: f32,
    handle: GpuTimingHandle,
}

impl TimestampRecorder {
    pub(super) fn new(context: &GpuContext, handle: Option<GpuTimingHandle>) -> Option<Self> {
        let handle = handle.filter(|_| context.supports_timestamps())?;
        let queries = context.device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("Frame stage timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: QUERY_COUNT,
        });
        let resolve = context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Frame timestamp resolve"),
            size: QUERY_BYTES,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Frame timestamp readback"),
            size: QUERY_BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Some(Self {
            queries,
            marker: context.timestamp_marker_pipeline().clone(),
            resolve,
            readback,
            completion: None,
            mapped: false,
            period: context.queue.get_timestamp_period(),
            handle,
        })
    }

    /// Records the time at this point of the encoder as query `index`.
    pub(super) fn write(&self, encoder: &mut wgpu::CommandEncoder, index: u32) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Frame timestamp"),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &self.queries,
                beginning_of_pass_write_index: Some(index),
                end_of_pass_write_index: None,
            }),
        });
        pass.set_pipeline(&self.marker);
        pass.dispatch_workgroups(1, 1, 1);
    }

    pub(super) fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.resolve_query_set(&self.queries, 0..QUERY_COUNT, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.readback, 0, QUERY_BYTES);
    }

    pub(super) fn map(&mut self) {
        let (sender, completion) = mpsc::channel();
        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.completion = Some(completion);
    }

    pub(super) fn is_ready(&mut self) -> motion_core::Result<bool> {
        if !self.mapped {
            match self
                .completion
                .as_ref()
                .ok_or_else(|| timing_error("Timestamp readback was not submitted"))?
                .try_recv()
            {
                Ok(Ok(())) => self.mapped = true,
                Ok(Err(error)) => return Err(motion_core::Error::Gpu(Box::new(error))),
                Err(mpsc::TryRecvError::Empty) => return Ok(false),
                Err(error) => return Err(motion_core::Error::Gpu(Box::new(error))),
            }
        }
        Ok(true)
    }

    pub(super) fn handle(&self) -> GpuTimingHandle {
        self.handle.clone()
    }

    pub(super) fn finish(self) -> motion_core::Result<CompletedTimings> {
        if !self.mapped {
            self.completion
                .ok_or_else(|| timing_error("Timestamp readback was not submitted"))?
                .recv()
                .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?
                .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?;
        }
        let mapped = self
            .readback
            .slice(..)
            .get_mapped_range()
            .map_err(|error| motion_core::Error::Gpu(Box::new(error)))?;
        let result = (|| {
            let mut ticks = [0; QUERY_COUNT as usize];
            for (target, bytes) in ticks.iter_mut().zip(mapped.as_chunks::<8>().0) {
                *target = u64::from_le_bytes(*bytes);
            }
            Ok(GpuTimings {
                composition: timestamp_delta(ticks[0], ticks[1], self.period)?,
                output: timestamp_delta(ticks[2], ticks[3], self.period)?,
                readback_copy: timestamp_delta(ticks[4], ticks[5], self.period)?,
                instrumentation_bytes: QUERY_BYTES * 2,
            })
        })();
        drop(mapped);
        self.readback.unmap();
        result.map(|timings| (self.handle, timings))
    }

    pub(super) fn publish(completed: Option<CompletedTimings>) {
        if let Some((handle, timings)) = completed {
            let _ = handle.0.set(timings);
        }
    }
}

fn timestamp_delta(start: u64, end: u64, period: f32) -> motion_core::Result<Duration> {
    if !period.is_finite() || period <= 0.0 {
        return Err(timing_error("Invalid GPU timestamp period"));
    }
    let seconds = end.wrapping_sub(start) as f64 * f64::from(period) * 1.0e-9;
    Duration::try_from_secs_f64(seconds).map_err(|error| motion_core::Error::Gpu(Box::new(error)))
}

fn timing_error(message: &str) -> motion_core::Error {
    ValidationError::invalid("gpu.timings", "timestamp", message).into()
}

impl Gpu {
    /// Enables per-frame GPU timestamps when supported; disabled by default.
    /// No query or readback resources are allocated when disabled or unsupported.
    pub fn enable_timings(&mut self, enabled: bool) {
        self.timing = enabled.then(GpuTimingHandle::default);
    }

    /// Latest submission's timestamps, available only after successful completion.
    pub fn timings(&self) -> Option<GpuTimings> {
        self.timing.as_ref().and_then(GpuTimingHandle::get)
    }

    /// Retains the latest submission's result independently of later submissions.
    pub fn timing_handle(&self) -> Option<GpuTimingHandle> {
        self.timing.clone()
    }

    pub(crate) fn host_timings(&self) -> (Duration, Duration) {
        (self.encode_time, self.readback_time)
    }

    pub(super) fn begin_timings(&mut self) {
        self.encode_time = Duration::ZERO;
        self.readback_time = Duration::ZERO;
        if self.timing.is_some() {
            self.timing = Some(GpuTimingHandle::default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_timestamp_periods_return_errors() {
        for period in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(timestamp_delta(0, 1, period).is_err());
        }
    }

    #[test]
    #[ignore = "requires native GPU; validates timestamp plumbing without latency assertions"]
    fn native_timestamp_queries_resolve_and_map_with_their_submission() -> motion_core::Result {
        let context = GpuContext::new()?;
        assert!(TimestampRecorder::new(&context, None).is_none());
        let handle = GpuTimingHandle::default();
        let Some(mut recorder) = TimestampRecorder::new(&context, Some(handle.clone())) else {
            assert!(!context.supports_timestamps());
            assert!(handle.get().is_none());
            return Ok(());
        };
        let validation_scope = context
            .device
            .push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = context.device.create_command_encoder(&Default::default());
        for index in 0..QUERY_COUNT {
            recorder.write(&mut encoder, index);
        }
        recorder.resolve(&mut encoder);
        let submission = context.queue.submit(Some(encoder.finish()));
        recorder.map();
        assert!(handle.get().is_none());
        super::super::wait_for_gpu(&context.device, Some(submission))?;
        assert!(pollster::block_on(validation_scope.pop()).is_none());
        let completed = recorder.finish()?;
        let timings = completed.1;
        TimestampRecorder::publish(Some(completed));
        assert_eq!(handle.get(), Some(timings));
        assert_eq!(timings.instrumentation_bytes, QUERY_BYTES * 2);
        eprintln!(
            "Native timestamp query plumbing supported on {}",
            context.info().name
        );
        Ok(())
    }
}
