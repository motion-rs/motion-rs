//! Poison-tolerant locking shared by every crate.
//!
//! A mutex is poisoned when a thread panics while holding it. Scene callbacks run
//! outside these locks and worker panics are caught at thread boundaries and reported
//! as errors, so a poisoned lock never means the process must stop. Every state guarded
//! by a library mutex is plain data that stays valid at each statement (caches,
//! mailboxes, counters and progress), so these helpers continue with the guarded value
//! instead of propagating the poison as a second panic. The font engine's lock is the
//! exception: it guards third-party shaping state, so a poisoned engine is reported as
//! a validation error.

use std::sync::{Condvar, Mutex, MutexGuard, PoisonError, WaitTimeoutResult};
use std::time::Duration;

/// Locks a [`Mutex`] and continues with its value when a previous holder panicked.
pub trait MutexExt<T> {
    /// Acquires the lock, recovering the guarded value from a poisoned mutex.
    fn lock_unpoisoned(&self) -> MutexGuard<'_, T>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn lock_unpoisoned(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Waits on a [`Condvar`] and continues when the mutex was poisoned meanwhile.
pub trait CondvarExt {
    /// Blocks until notified, recovering the guard from a poisoned mutex.
    fn wait_unpoisoned<'a, T>(&self, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T>;

    /// Blocks until notified or `timeout` elapses, recovering from a poisoned mutex.
    fn wait_timeout_unpoisoned<'a, T>(
        &self,
        guard: MutexGuard<'a, T>,
        timeout: Duration,
    ) -> (MutexGuard<'a, T>, WaitTimeoutResult);
}

impl CondvarExt for Condvar {
    fn wait_unpoisoned<'a, T>(&self, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
        self.wait(guard).unwrap_or_else(PoisonError::into_inner)
    }

    fn wait_timeout_unpoisoned<'a, T>(
        &self,
        guard: MutexGuard<'a, T>,
        timeout: Duration,
    ) -> (MutexGuard<'a, T>, WaitTimeoutResult) {
        self.wait_timeout(guard, timeout)
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn poisoned_locks_keep_their_last_value() {
        let shared = Arc::new(Mutex::new(1));
        let writer = shared.clone();
        let panicked = std::thread::spawn(move || {
            let mut value = writer.lock().unwrap();
            *value = 7;
            panic!("poison the lock");
        })
        .join();
        assert!(panicked.is_err() && shared.is_poisoned());
        assert_eq!(*shared.lock_unpoisoned(), 7);
        let (guard, result) =
            Condvar::new().wait_timeout_unpoisoned(shared.lock_unpoisoned(), Duration::ZERO);
        assert_eq!(*guard, 7);
        assert!(result.timed_out());
    }
}
