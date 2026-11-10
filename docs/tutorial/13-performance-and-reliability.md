# 13 Performance and Reliability

## What This Chapter Covers

Separate preparation, composition, output and presentation costs; choose the right benchmark; and configure budgets for trusted and restricted inputs.
The basic budget example needs no optional features. Enable benchmark `text`, `particles`, `gpu` or native codec environments according to the workload.

## Minimal Example: Explicit Work Limits

```rust,no_run
use motion::prelude::*;
use motion::render::RasterLimits;

fn main() -> motion::Result {
    let timeline = Timeline::new(Size::new(320.0, 180.0), 30).track(|track| {
        track.scene(1.seconds(), |context| {
            context.bounds().fill(Color::rgb(20, 150, 125))
        })
    }).build()?;
    let options = RenderOptions::default().limits(RasterLimits {
        max_work: 1,
        ..RasterLimits::sandbox()
    });
    match render_still(&timeline, Time::ZERO, &options) {
        Ok(_) => println!("frame accepted"),
        Err(error) => eprintln!("frame rejected: {error}"),
    }
    Ok(())
}
```

This deliberately tiny work allowance demonstrates admission failure, not an optimization recommendation. Rejection is a result, not a successful-render timing.

## Budget Layers

| Boundary | Limit or Meaning |
| --- | --- |
| `ValidationLimits` | Input complexity such as nodes, structure and commands |
| `RasterLimits` | Pixels, active logical bytes, raster work allowance |
| `ResourcePool` / `RenderCache` | Long-term logical retention and entry counts |
| `ExportOptions.max_parallel_bytes` | Allowance for multiple active export-render slots |
| Studio frame LRU | Final-preview-frame retention, not the renderer working set |

The default `RasterLimits::authoring()` has no work ceiling but still checks pixels, active bytes and other boundaries. `sandbox()` retains an explicit 200,000,000-unit ceiling. Neither is a deadline, process RSS limit or Rust callback sandbox.

Large transparent layers, blur halos, images, dense text/particles, motion blur and output dimensions can increase cost. Hidden content is still validated; opacity=0 does not bypass invalid structure.

## Locate the Stage First

Fix the film, time, dimensions, quality, assets, fonts, backend and budgets before locating the slowdown:

1. Authoring evaluation and compilation: is unchanged content rebuilt, and is memo used correctly?
2. Coverage / layout / material: do cold caches dominate, and does seeking differ from replaying one frame?
3. CPU/GPU composition: how do filters, layer counts and pixel counts change?
4. Output / readback: was a CPU frame requested instead of an external texture?
5. Encode / I/O / UI: are encoding, playback scheduling or presentation waits counted as rendering?

One fps number cannot answer all five questions. GPU submission time is not GPU completion time, and submit/finish CPU wall time is not a GPU timestamp query.

## Benchmark Entry Points

| Target | Question |
| --- | --- |
| `performance` | Causal before/after comparisons of frozen reference cohorts |
| `films` | Rendering costs of real Gallery films |
| `criterion_frames` | Statistical observations of a few warm CPU full-frame workloads |
| `breadth` | Costs across content types, dimensions, stages and primitives |

Run these in the repository when needed:

```sh
cargo bench --locked --offline -p motion-bench --no-default-features --features text,particles --bench criterion_frames
cargo bench --locked --offline -p motion-bench --bench breadth
cargo bench --locked --offline -p motion-bench --bench films --features films,gpu
```

These commands compile and measure actual workloads; they are not implicit steps of reading this tutorial. See the [benchmark entry points](../benchmarks.md).

## Evidence for Optimization Claims

Fresh controls and candidates for one workload must fix fonts, features, machine/compiler, adapter, quality, dimensions, allowances and sampling. Freeze source/executable provenance, verify images and seek replay, and pair independent processes in AB/BA order.

Report each case's P50, P95 with sufficient samples, run-to-run dispersion and uncertainty intervals. Batch P95 is not single-frame tail latency; old reports do not represent current sources or revised default budgets.

[Benchmark measurements](../benchmarks.md#current-measurements) include source identity and raw data. They are not real-time guarantees for all current content, nor values to average across workloads into an overall speedup factor.

## Errors and Determinism

`motion::Error` retains category, frame context and source. Use `kind`, `frame` and `downcast_ref` rather than depending on fixed diagnostic wording.

Synchronous callback panics are panics in trusted Rust code; worker boundaries can convert them into scene-panic diagnostics. Automatic retries apply only to permitted capacity cases, not panics, invalid values or work-limit failures.

Determinism requires fixed inputs, including seeds, font bytes, resource environments and time. Check pixel replay in A, B, then A order. Use declared CPU/GPU tolerances rather than assuming bit-identical floating-point behavior on all drivers.

Font-selection/hash reports help audit actual sources, not prove identical glyph rasterization across platforms. Lazy file sources must remain unchanged during execution.

## Validation Strategy

For small changes, start with narrow behavior or boundary checks, then cover affected feature combinations. Use property tests for pure geometry and clocks; visible subjects and exact replay for content; actual samples for audio; and decoded frame counts, timing and container metadata for video.

Successful compilation, ignored fixtures and zero-test filters are not runtime acceptance. Validate window interaction, real audio devices and cross-platform driver recovery separately. Tests and performance measurements also support different conclusions.

## Common Pitfalls

- Conflating logical peak, idle pools, external holders and RSS.
- Changing artwork, fonts, quality or allowances for speed without disclosure.
- Measuring only the same warm frame and promising random-seek or cold-start performance.
- Treating default-budget rejection as execution timing or worker concurrency as actual throughput.
- Assuming structural validation constrains arbitrary Rust callback I/O or infinite loops.

## Next Steps

Next chapter: [14 Architecture and Extensions](14-architecture-and-extending.md).
Full strategies: [reliability](../design/reliability.md), [benchmarks](../benchmarks.md).
Real workloads: [Gallery guide](appendix-gallery.md).