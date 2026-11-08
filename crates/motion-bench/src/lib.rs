//! Workspace-only benchmark fixtures and validation.

#[cfg(all(
    test,
    feature = "image",
    feature = "text",
    feature = "svg",
    feature = "particles",
    feature = "audio"
))]
#[expect(
    dead_code,
    reason = "the benchmark binary entry points and report plumbing are unused by the test build"
)]
#[path = "../benches/performance.rs"]
mod benchmark_artwork;
