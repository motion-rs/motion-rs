//! Functional checks for the breadth benchmark scenes; the timing paths are not executed.

#[expect(
    dead_code,
    reason = "the timing entry points are not executed by these functional checks"
)]
#[path = "../benches/breadth.rs"]
mod breadth;
