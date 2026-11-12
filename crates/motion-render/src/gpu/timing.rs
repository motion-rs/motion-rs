//! Optional timestamp queries owned by one submitted frame.

use super::*;
use std::sync::{Arc, OnceLock, mpsc};
use std::time::{Duration, Instant};

const QUERY_COUNT: u32 = 18;

const QUERY_BYTES: u64 = QUERY_COUNT as u64 * 8;

pub(super) type CompletedTimings = (GpuTimingHandle, GpuTimings);

/// Validity of a device timestamp interval.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuTimestampStatus {
    /// The corresponding pass did not execute.
    #[default]
    NotExecuted,
    /// The raw values passed the available ordering and wall-time checks.
    Valid,
    /// The end timestamp is less than the beginning timestamp.
    Reversed,
    /// Other timestamps in the same sequence are out of execution order.
    OutOfOrder,
    /// The interval exceeds the recorder's enclosing host wall time.
    ExceedsWallTime,
    /// The device timestamp period cannot be used for conversion.
    InvalidPeriod,
    /// The converted interval cannot be represented as a duration.
    Unrepresentable,
}

/// One device interval, including raw query values when its pass executed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuTimestamp {
    /// Raw beginning and end query values, or none when the pass did not execute.
    pub ticks: Option<[u64; 2]>,
    /// Elapsed time only when all available validity checks pass.
    pub duration: Option<Duration>,
    /// Outcome of the interval validity checks.
    pub status: GpuTimestampStatus,
}

/// Device timestamps, separate from CPU submission and completion wall time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GpuTimings {
    /// Device execution of composition, filters and their encoder copies.
    pub composition: GpuTimestamp,
    /// Device execution of final packing, resolve or texture presentation.
    pub output: GpuTimestamp,
    /// Device copies of frame output and finite status into readback storage.
    /// CPU mapping, waiting and output extraction are not included.
    pub readback_copy: GpuTimestamp,
    /// Device stages: flatten, count, all prefix passes, scatter, boundaries and coverage.
    pub coverage: [GpuTimestamp; 6],
    /// Nanoseconds per raw timestamp tick.
    pub timestamp_period_ns: f32,
    /// Host time from recorder creation to reading the completed queries.
    pub enclosing_wall_time: Duration,
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
    started: Instant,
    coverage_active: std::cell::Cell<[bool; 6]>,
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
            started: Instant::now(),
            coverage_active: std::cell::Cell::new([false; 6]),
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

    pub(super) fn coverage_pass(&self, stage: usize) -> wgpu::ComputePassTimestampWrites<'_> {
        self.coverage_pass_boundary(stage, true, true)
    }

    pub(super) fn coverage_pass_boundary(
        &self,
        stage: usize,
        beginning: bool,
        end: bool,
    ) -> wgpu::ComputePassTimestampWrites<'_> {
        let mut active = self.coverage_active.get();
        active[stage] = true;
        self.coverage_active.set(active);
        wgpu::ComputePassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: beginning.then_some(6 + stage as u32 * 2),
            end_of_pass_write_index: end.then_some(7 + stage as u32 * 2),
        }
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
        let timings = {
            let mut ticks = [0; QUERY_COUNT as usize];
            for (target, bytes) in ticks.iter_mut().zip(mapped.as_chunks::<8>().0) {
                *target = u64::from_le_bytes(*bytes);
            }
            let wall = self.started.elapsed();
            let marker_ordered = ticks[..6].windows(2).all(|pair| pair[0] <= pair[1]);
            let active = self.coverage_active.get();
            let mut last = None;
            let mut coverage_ordered = true;
            for (stage, is_active) in active.into_iter().enumerate() {
                if is_active {
                    let start = ticks[6 + stage * 2];
                    let end = ticks[7 + stage * 2];
                    coverage_ordered &= last.is_none_or(|last| start >= last) && end >= start;
                    last = Some(end);
                }
            }
            GpuTimings {
                composition: timestamp_interval(
                    [ticks[0], ticks[1]],
                    self.period,
                    wall,
                    marker_ordered,
                ),
                output: timestamp_interval([ticks[2], ticks[3]], self.period, wall, marker_ordered),
                readback_copy: timestamp_interval(
                    [ticks[4], ticks[5]],
                    self.period,
                    wall,
                    marker_ordered,
                ),
                coverage: {
                    let mut stages = [GpuTimestamp::default(); 6];
                    for (stage, is_active) in active.into_iter().enumerate() {
                        if is_active {
                            stages[stage] = timestamp_interval(
                                [ticks[6 + stage * 2], ticks[7 + stage * 2]],
                                self.period,
                                wall,
                                coverage_ordered,
                            );
                        }
                    }
                    stages
                },
                timestamp_period_ns: self.period,
                enclosing_wall_time: wall,
                instrumentation_bytes: QUERY_BYTES * 2,
            }
        };
        drop(mapped);
        self.readback.unmap();
        Ok((self.handle, timings))
    }

    pub(super) fn publish(completed: Option<CompletedTimings>) {
        if let Some((handle, timings)) = completed {
            let _ = handle.0.set(timings);
        }
    }
}

fn timestamp_interval(ticks: [u64; 2], period: f32, wall: Duration, ordered: bool) -> GpuTimestamp {
    let status = if !period.is_finite() || period <= 0.0 {
        GpuTimestampStatus::InvalidPeriod
    } else if ticks[1] < ticks[0] {
        GpuTimestampStatus::Reversed
    } else if !ordered {
        GpuTimestampStatus::OutOfOrder
    } else {
        let seconds = (ticks[1] - ticks[0]) as f64 * f64::from(period) * 1.0e-9;
        match Duration::try_from_secs_f64(seconds) {
            Ok(duration) if duration <= wall => {
                return GpuTimestamp {
                    ticks: Some(ticks),
                    duration: Some(duration),
                    status: GpuTimestampStatus::Valid,
                };
            }
            Ok(_) => GpuTimestampStatus::ExceedsWallTime,
            Err(_) => GpuTimestampStatus::Unrepresentable,
        }
    };
    GpuTimestamp {
        ticks: Some(ticks),
        duration: None,
        status,
    }
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
    fn invalid_timestamp_periods_are_reported() {
        for period in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let interval = timestamp_interval([0, 1], period, Duration::MAX, true);
            assert_eq!(interval.status, GpuTimestampStatus::InvalidPeriod);
            assert_eq!(interval.ticks, Some([0, 1]));
            assert_eq!(interval.duration, None);
        }
    }

    #[test]
    fn unrepresentable_timestamp_interval_retains_raw_ticks() {
        let ticks = [0, u64::MAX];
        let interval = timestamp_interval(ticks, f32::MAX, Duration::MAX, true);
        assert_eq!(interval.status, GpuTimestampStatus::Unrepresentable);
        assert_eq!(interval.ticks, Some(ticks));
        assert_eq!(interval.duration, None);
    }

    #[test]
    fn timestamps_distinguish_zero_invalid_and_not_executed() {
        let wall = Duration::from_millis(1);
        let interval = |ticks, ordered| timestamp_interval(ticks, 1.0, wall, ordered);
        assert_eq!(
            GpuTimestamp::default().status,
            GpuTimestampStatus::NotExecuted
        );
        assert_eq!(GpuTimestamp::default().ticks, None);
        assert_eq!(interval([10, 10], true).duration, Some(Duration::ZERO));
        assert_eq!(interval([10, 3], true).status, GpuTimestampStatus::Reversed);
        assert_eq!(
            interval([10, 20], false).status,
            GpuTimestampStatus::OutOfOrder
        );
        assert_eq!(
            interval([10, 1_000_011], true).status,
            GpuTimestampStatus::ExceedsWallTime
        );
        assert_eq!(interval([10, 1_000_010], true).duration, Some(wall));
        assert_eq!(interval([10, 3], true).ticks, Some([10, 3]));
        assert_eq!(interval([10, 3], true).duration, None);
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
        for index in 0..6 {
            recorder.write(&mut encoder, index);
        }
        for stage in [0, 2, 5] {
            if stage == 2 {
                for (beginning, end) in [(true, false), (false, true)] {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("Test prefix timestamp boundary"),
                        timestamp_writes: Some(
                            recorder.coverage_pass_boundary(stage, beginning, end),
                        ),
                    });
                    pass.set_pipeline(&recorder.marker);
                    pass.dispatch_workgroups(1, 1, 1);
                }
                continue;
            }
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Test coverage timestamp"),
                timestamp_writes: Some(recorder.coverage_pass(stage)),
            });
            pass.set_pipeline(&recorder.marker);
            pass.dispatch_workgroups(1, 1, 1);
        }
        recorder.resolve(&mut encoder);
        let submission = context.queue.submit(Some(encoder.finish()));
        recorder.map();
        assert!(handle.get().is_none());
        crate::gpu::wait_for_gpu(&context.device, Some(submission))?;
        assert!(pollster::block_on(validation_scope.pop()).is_none());
        let completed = recorder.finish()?;
        let timings = completed.1;
        TimestampRecorder::publish(Some(completed));
        assert_eq!(handle.get(), Some(timings));
        assert_eq!(timings.instrumentation_bytes, QUERY_BYTES * 2);
        for stage in [0, 2, 5] {
            assert!(timings.coverage[stage].ticks.is_some());
            assert_ne!(
                timings.coverage[stage].status,
                GpuTimestampStatus::NotExecuted
            );
        }
        for stage in [1, 3, 4] {
            assert_eq!(timings.coverage[stage], GpuTimestamp::default());
        }
        eprintln!(
            "Native timestamp query plumbing supported on {}",
            context.info().name
        );
        Ok(())
    }
}
