//! Bounded Rayon render slots with ordered consumption on the export thread.

use super::{ProgressSink, Result, cancelled, invalid};
use std::{
    ops::Range,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::sync_channel,
    },
};

#[cfg(test)]
pub(super) fn ordered<Frame: Send, Render: FnMut(i64) -> Result<Frame>>(
    range: Range<i64>,
    workers: usize,
    progress: &dyn ProgressSink,
    make_renderer: impl Fn(usize) -> Result<Render> + Sync,
    mut write: impl FnMut(i64, Frame) -> Result<()>,
) -> Result<()> {
    ordered_until(range, workers, progress, make_renderer, |index, frame| {
        write(index, frame)?;
        Ok(true)
    })
}

pub(super) fn ordered_until<Frame: Send, Render: FnMut(i64) -> Result<Frame>>(
    range: Range<i64>,
    workers: usize,
    progress: &dyn ProgressSink,
    make_renderer: impl Fn(usize) -> Result<Render> + Sync,
    mut write: impl FnMut(i64, Frame) -> Result<bool>,
) -> Result<()> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .thread_name(|worker| format!("motion-render-{worker}"))
        .build()
        .map_err(std::io::Error::other)?;
    let failed = AtomicBool::new(false);
    pool.in_place_scope(|scope| {
        let mut slots = Vec::with_capacity(workers);
        for worker in 0..workers {
            let (jobs, receive_job) = sync_channel::<i64>(1);
            let (results, receive_result) = sync_channel(1);
            let make_renderer = &make_renderer;
            let failed = &failed;
            scope.spawn(move |_| {
                let mut renderer = None;
                while let Ok(frame) = receive_job.recv() {
                    let result = catch_unwind(AssertUnwindSafe(|| {
                        cancelled(progress)?;
                        if renderer.is_none() {
                            renderer = Some(make_renderer(worker)?);
                        }
                        renderer.as_mut().expect("initialized renderer")(frame)
                    }))
                    .unwrap_or_else(|payload| {
                        Err(motion_core::Error::scene_panic(
                            "export",
                            frame as u64,
                            payload,
                        ))
                    })
                    .map_err(|error| error.at_frame(frame as u64));
                    let error = result.is_err();
                    if error {
                        failed.store(true, Ordering::Release);
                    }
                    if results.send(result).is_err() || error {
                        break;
                    }
                }
            });
            slots.push((jobs, receive_result));
        }
        for (worker, (jobs, _)) in slots.iter().enumerate() {
            let frame = range.start + worker as i64;
            if frame < range.end {
                jobs.send(frame)
                    .map_err(|_| invalid("worker", "Render worker stopped"))?;
            }
        }
        for frame in range.clone() {
            cancelled(progress)?;
            let slot = (frame - range.start) as usize % workers;
            let pixels = slots[slot]
                .1
                .recv()
                .map_err(|_| invalid("worker", "Render worker stopped"))??;
            cancelled(progress)?;
            if !write(frame, pixels)? {
                break;
            }
            cancelled(progress)?;
            let next = frame + workers as i64;
            if next < range.end && !failed.load(Ordering::Acquire) {
                slots[slot]
                    .0
                    .send(next)
                    .map_err(|_| invalid("worker", "Render worker stopped"))?;
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::NoProgress;
    use std::sync::{Condvar, Mutex};

    #[test]
    fn stopping_ordered_consumption_joins_and_drops_unwritten_results() {
        let live = std::sync::atomic::AtomicUsize::new(0);

        struct Frame<'a>(&'a std::sync::atomic::AtomicUsize);

        impl Drop for Frame<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::Relaxed);
            }
        }

        let mut written = Vec::new();
        ordered_until(
            10..100,
            4,
            &NoProgress,
            |_| {
                Ok(|_| {
                    live.fetch_add(1, Ordering::Relaxed);
                    Ok(Frame(&live))
                })
            },
            |index, _| {
                if index == 12 {
                    return Ok(false);
                }
                written.push(index);
                Ok(true)
            },
        )
        .unwrap();
        assert_eq!(written, [10, 11]);
        assert_eq!(live.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn nested_rayon_export_keeps_writer_on_caller_and_uses_a_bounded_private_pool() {
        use std::{cell::RefCell, rc::Rc};

        let outer = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        outer.install(|| {
            let caller = std::thread::current().id();
            for workers in [1, 2, 4] {
                let written = Rc::new(RefCell::new(Vec::new()));
                let sink = written.clone();
                ordered(
                    5..14,
                    workers,
                    &NoProgress,
                    |_| {
                        assert_eq!(rayon::current_num_threads(), workers);
                        assert_ne!(std::thread::current().id(), caller);
                        Ok(Ok::<_, motion_core::Error>)
                    },
                    move |frame, pixels| {
                        assert_eq!(std::thread::current().id(), caller);
                        assert_eq!(rayon::current_num_threads(), 1);
                        sink.borrow_mut().push((frame, pixels));
                        Ok(())
                    },
                )
                .unwrap();
                assert_eq!(
                    *written.borrow(),
                    (5..14).map(|frame| (frame, frame)).collect::<Vec<_>>()
                );
            }
        });
    }

    #[test]
    fn delayed_first_slot_bounds_reordering_and_preserves_absolute_order() {
        let completed = (Mutex::new(Vec::new()), Condvar::new());
        let mut written = Vec::new();
        ordered(
            10..19,
            3,
            &NoProgress,
            |_| {
                Ok(|frame| {
                    let (lock, ready) = &completed;
                    let mut done = lock.lock().unwrap();
                    if frame == 10 {
                        while done.len() < 2 {
                            let (next, timeout) = ready
                                .wait_timeout(done, std::time::Duration::from_secs(5))
                                .unwrap();
                            done = next;
                            assert!(!timeout.timed_out(), "Later slots did not complete");
                        }
                        assert!(done.iter().all(|index| (11..13).contains(index)));
                    }
                    done.push(frame);
                    ready.notify_all();
                    Ok(frame)
                })
            },
            |index, pixels| {
                written.push((index, pixels));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            written,
            (10..19).map(|frame| (frame, frame)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn write_failure_and_worker_panic_join_without_draining_all_frames() {
        for panic_worker in [false, true] {
            let mut written = Vec::new();
            let result = ordered(
                0..100,
                3,
                &NoProgress,
                |_| {
                    Ok(|frame| {
                        assert!(!panic_worker || frame != 1, "worker panic");
                        Ok(frame)
                    })
                },
                |frame, _| {
                    if !panic_worker && frame == 1 {
                        return Err(invalid("write", "Injected write failure"));
                    }
                    written.push(frame);
                    Ok(())
                },
            );
            let error = result.unwrap_err();
            if panic_worker {
                assert_eq!(error.kind(), motion_core::ErrorKind::ScenePanic);
                assert_eq!(error.frame(), Some(1));
            }
            assert_eq!(written, [0]);
        }
    }

    #[test]
    fn cancellation_and_renderer_initialization_failure_join_all_slots() {
        struct Progress(AtomicBool);

        impl ProgressSink for Progress {
            fn is_cancelled(&self) -> bool {
                self.0.load(Ordering::Acquire)
            }
        }
        let progress = Progress(AtomicBool::new(false));
        let mut written = Vec::new();
        let error = ordered(
            20..40,
            4,
            &progress,
            |_| Ok(Ok::<_, motion_core::Error>),
            |frame, _| {
                written.push(frame);
                progress.0.store(true, Ordering::Release);
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), motion_core::ErrorKind::Cancelled);
        assert_eq!(written, [20]);
        let error = ordered(
            20..40,
            4,
            &NoProgress,
            |_| {
                Err::<fn(i64) -> Result<i64>, _>(invalid(
                    "initialization",
                    "Injected initialization failure",
                ))
            },
            |_, _| unreachable!(),
        )
        .unwrap_err();
        assert_eq!(error.frame(), Some(20));
    }
}
