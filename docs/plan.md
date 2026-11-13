# Development Plan

This plan describes remaining work, not a record of implementation changes.
Tasks are ordered within each priority. Correctness gates precede optimization.

## Reliability and API Quality

1. Keep package tests independent of the facade's copy of their own crate.
   Validate minimal, default, Studio-without-scene-GPU and all-feature builds;
   run ordinary tests and Rustdoc examples in each supported configuration.
2. Maintain explicit fixtures for fonts, GPU and H.264/AAC. Check pixel
   agreement, exact seek replay, cancellation, device loss, source overwrite
   protection and temporary-file cleanup. Ignored fixtures need named executions.
3. Keep public contracts complete: units, defaults, errors, resource ownership,
   threading, unsupported inputs and logical versus physical memory limits.
   Reject broken Rustdoc links and stale source links in CI.
4. Extend existing Linux/macOS feature-consumer CI to Windows and independent graphics-adapter runs.
   Establish platform-specific font/audio/FFmpeg setup and separate environmental
   failures from algorithm regressions. macOS results do not certify other hosts.
5. Improve interactive Studio coverage: rapid seek, resize, scene/backend changes,
   hot reload while playing or exporting, mute, audio underrun, close during export and device recovery. Automated worker
   fixtures do not substitute for actual window/audio-device acceptance.

## Performance

Benchmark workloads, commands and recorded results live in
[Benchmarks](benchmarks.md); all measured step results are kept in its
[current measurements](benchmarks.md#current-measurements), with per-case changes against
both the previous step and the initial baseline. Do not create per-step reports.
Contracts that already exist are in [rendering](design/rendering.md),
[GPU rendering](design/rendering.md) and [architecture](design/architecture.md).

Performance measurements are paused. Four merged deliverables remain from earlier reviews,
while R12 comes from the Gallery profile in review 7, which also lists
output-changing proposals that need separate approval. IDs track review requests, not a
measured priority ranking; duplicate requests are grouped by deliverable.

| ID | Remaining Work | Acceptance Boundary |
| --- | --- | --- |
| R1 | Row-wise SIMD (blur, spans) only if measured against the scalar paths; per-pixel lanes were removed (P-6 / H-2) | Scalar agreement, tails, overflow and exceptional values; no implicit FMA changes |
| R3 | Explicit opt-in subtree raster cache (P-9 / S-7) | Isolated-layer API, bounded lifetime/identity and separate output-change approval |
| R4 | General cold item-parallel prepare (S-3) | Per-item admission/scratch, ordered assembly and deterministic failure; disabled cached-copy experiment removed |
| R6 | Current-source performance acceptance (N-6 / M2 / M4) | Record actual suite workload/sampling and absolute timings; full films, process RSS, GUI presentation and cross-device automated milestone gates remain separate |
| R12 | Prepare independent audio buses in parallel and merge them in a fixed order | Bit-identical PCM and loudness reports for every worker count |

Whole-band area/cover, session polygon scratch reuse (R10) and Gallery default-budget
restoration are implemented; their functional gates do not establish timing gains or all-frame admission. Studio's
first-review L shortcut is tracked as R7. Streaming native encoding, measured frame
workers, pooled GPU textures, shared content caches and CPU pixel/encoded-plane pooling are
implemented, not new tasks. Full-frame linear APIs remain supported; automatic subtree
raster caching and DisplayList/PreparedCpuFrame object pooling are not implied by storage
reuse. Further locality, preview wait reduction and decode scratch tuning need profiling
after measurements resume, without changing geometry, density, alpha or replay.

Do not count approved low-level APIs, checked `try_*` alternatives, migration notes,
test-only audio reference fixtures or optional LaTeX/disk-cache extensions as deprecated
production compatibility paths.

### Acceptance Workflow

1. Establish a validated current-source baseline for the affected workload using the
   [benchmark procedures](benchmarks.md#non-film-rerun), not deleted historical timings.
2. Check the task-specific boundary above and record the actual validation command and
   source revision. Output-changing proposals require separate approval.
3. Register measured results, regressions and verification scope using the
   [report update procedure](benchmarks.md#update-procedure). Ordinary tests, benchmark
   evidence and native/hardware acceptance are separate gates.

### Validation Debt

- Minimal-feature content unit-test gates still reference text/font-only fixtures from
  asset/vector tests; independent consumer compilation is not unit-test coverage.
- Library and warning-clean `--all-targets` checks pass for isolated feature sets of
  `motion-content`, `motion-encode`, `motion-render`, `motion-formula`, `motion-scene`,
  `motion-studio` and `motion-gallery`. `motion-rs` library targets are warning-clean
  without features, but several integration tests and examples still fail to compile
  without their feature sets; they need `required-features` or per-test gating.
- Windows, independent graphics adapters and actual Studio window/audio-device
  interactions need explicit runtime acceptance. Existing Linux/macOS feature CI and
  offline worker fixtures do not replace these checks. The Studio loop shortcut has
  regression fixtures but still needs native UI acceptance.

## Feature Extensions

### Independent GPU preparation

The v3 backend split has normalized FrameIr semantics and separate CPU/GPU preparation
results. FrameIr retains admitted f64 device curves, Wang segment bounds, ordered
scopes, paints, clips and filter domains without coverage. Eligible GPU curves are
flattened into statically bounded edge storage on the device, followed by
GPU-generated f32 coverage, not a CPU PreparedCpuFrame.
GPU coverage uses bounded 16-by-16 tile binning with signed row carries.
Preparation reports expose segment/tile counts, geometry-upload bytes and classified
explicit unsupported-path errors (no CPU coverage fallback); the GPU startup self-check
exercises independent CPU/GPU preparation with visual comparisons.
Immutable image/mip/atlas buffers have bounded context-local cross-frame residency.
Eligible analytic particles generate the consumed primitive records on the GPU;
CPU materialization remains available for unsupported or imprecise cases.
Mesh and generated atmosphere paints retain descriptors and evaluate independently
on CPU/GPU; explicit image-baking APIs preserve their existing behavior.
Remaining acceptance:

1. Add a large-scene native fixture that shares the breadth benchmark scenes: text,
   SVG paths with gradients, holes, clipping and filters, crossing paths under nested
   clips, and analytic particles with blur, mesh paint and procedural atmosphere.
   Render several frames at 640x360, 1920x1080 and 3840x2160, compare each with the
   [visual acceptance](design/reliability.md#visual-acceptance) tiers, and precede
   each replay with an unrelated seek. It needs an explicit logical byte allowance
   without changing the breadth benchmark budget.
2. Move the remaining CPU/GPU parity fixtures (Bloom, text, frame-renderer sessions,
   particles and procedural paints) from their local per-channel or f32 checks to the
   named levels and tiers, sharing the comparator across crates. Align
   `VisualCheckPolicy::max_rgb_difference` with the tiers.
3. Validate complex text, SVG, clipping and filtered scenes at larger resolutions
   before claiming performance gains. Functional GPU flattening, procedural evaluation
   and residency do not establish throughput acceptance.

### Additional capabilities

1. Expand output codecs/presets, alpha-capable output and color-management options.
   Each combination needs explicit capability checks and decoded-output fixtures.
2. Extend SVG/filter support only through the shared vector/filter model. Unsupported
   semantics remain diagnostics, not silent raster substitutions.
3. Add variable-font/color-glyph formats and richer text decoration with source and
   grapheme fidelity. Keep one paragraph shaper and explicit frozen font snapshots.
4. Add audio device/rate/channel selection, seamless looping and streaming analysis.
   Keep offline deterministic preparation separate from real-time device ownership.
5. Extend particle models and generic field/simulation operators, then optimize
   dense neighbor queries. Smoke/fire artwork stays in applications, not core.
6. Explore IK/constraints, richer procedural materials and project persistence.
   Define bounded resources, timing, failure and cache contracts first.
7. Add path boolean operations (union, intersection, difference, xor) to the shared
   vector model. Results must be deterministic, bounded through `Budget` and
   `RasterLimits`, and identical for the CPU and GPU backends; degenerate and
   self-intersecting inputs need explicit fixtures and diagnostics.
8. Add mesh gradients (patch grids with per-vertex colors). Admit patch and subdivision
   counts before allocation, define color space and interpolation, and keep the CPU
   backend, GPU plan, WGSL shader and host ABI in agreement.
9. Add 2.5D perspective: transform layers in 3D with a camera, depth ordering and
   perspective-correct sampling, without turning the engine into a 3D renderer. Define
   projection, clipping and backface behavior once, with CPU/GPU pixel agreement.
10. Add video input sources decoded through `motion-ffmpeg`. Time maps explicitly from
    Timeline time to source frames, so seeking replays exactly. Define bounded decode
    memory, unsupported-codec and variable-frame-rate failure, and audio extraction.
    The decoder opens only files named through `Assets`.
11. Add hardware-accelerated encoding (for example VideoToolbox, NVENC, VAAPI) as an
    explicit, opt-in encoder choice. Check capability before use and report
    unavailability instead of falling back silently; keep software encoding the
    deterministic reference. Each encoder needs decoded-output fixtures and a named
    ignored hardware execution.

## Declarative Documents

The format, parameters, bindings, content kinds, hot reload and diagnostics are described in
[declarative documents](design/documents.md). Remaining work, in order:

1. Close the content gaps. Particles lack image emitters, force fields, sprite sheets,
   bokeh and trail options; formulas lack subexpression selection; Typst page documents,
   animated SVG and image-sequence sources are not describable; audio lacks
   analysis-driven signals, synthesis, speed/reverse transforms, metadata and chapters.
2. Extend bindings: read an element's world transform and bounds, bind across scenes and
   to audio analysis, and reference whole points (`@id.position`) without components.
3. Add format migration: a `migrate` command and compatibility rules for the next
   version. Until a second version exists, other versions are rejected.
4. Evaluate Lottie/Bodymovin import as a converter into this format. Decide between an
   existing Rust implementation and a first-party subset only after checking license
   and binary size, and keep the dependency optional.
5. Export code-built timelines to documents for the builders that have a data form.
   Closure scenes cannot be serialized, so the supported subset must be explicit.
6. Test the parser and compiler with fuzzing or property tests (no panics, bounded
   work, every diagnostic path resolvable) and pin golden frames for the example
   documents on both backends.
7. Make the retained-diagnostic cap (256) configurable.
8. Hot reload polish: preserve the per-scene GPU context across reloads, and accept native window acceptance on real
   hosts. The automated fixtures use headless egui and do not replace it.
9. Extend static schema range coverage beyond document times and particle inputs.
   Animated results and resource availability still require runtime validation.

## Completion Rules

Every change needs focused assertions and applicable package/feature compilation,
ordinary tests, documentation tests and explicit environmental fixtures. Performance
or hardware claims require their own measured evidence. Keep tests inline in their
owning source modules until they exceed roughly 300 lines, then move them to a sibling
`tests.rs` (see [Source style](style.md)); do not remove independent assertions to make a
gate pass.