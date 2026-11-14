//! Functional checks for the performance benchmark scenes; the timing paths are not executed.

#[expect(
    dead_code,
    reason = "the timing entry points are not executed by these functional checks"
)]
#[path = "../benches/performance.rs"]
mod performance;
