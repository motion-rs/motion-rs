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
[GPU rendering](design/gpu.md) and [architecture](design/architecture.md).

Performance measurements are paused. Three merged deliverables remain from earlier reviews,
and R10–R12 come from the Gallery profile in [review 7](reviews/review7.md), which also lists
output-changing proposals that need separate approval. IDs track review requests, not a
measured priority ranking; duplicate requests are grouped by deliverable.

| ID | Remaining Work | Acceptance Boundary |
| --- | --- | --- |
| R1 | Row-wise SIMD (blur, spans) only if measured against the scalar paths; per-pixel lanes were removed (P-6 / H-2) | Scalar agreement, tails, overflow and exceptional values; no implicit FMA changes |
| R4 | General cold item-parallel prepare (S-3) | Per-item admission/scratch, ordered assembly and deterministic failure; disabled cached-copy experiment removed |
| R6 | Current-source performance acceptance (N-6 / M2 / M4) | Record actual suite workload/sampling and absolute timings; full films, process RSS, GUI presentation and cross-device automated milestone gates remain separate |
| R10 | Extend session polygon scratch reuse to active-edge and intersection buffers; band-area reuse has focused cold-path evidence but unresolved warm/off-target regressions | Bit-identical coverage; logical byte and work admission unchanged; separate measured cold gains from general acceptance |
| R11 | Admit Last Order (cinematic-look) at 1080p on the CPU and on the GPU after frame 88 | `full_size_default_budgets_admit_original_demos_and_replay` and the GPU film fixture pass; any output or limit change is approved and documented in the Gallery README |
| R12 | Prepare independent audio buses in parallel and merge them in a fixed order | Bit-identical PCM and loudness reports for every worker count |

Whole-band area/cover and Gallery default-budget restoration are implemented; their
functional gates do not establish timing gains or all-frame admission. Studio's
first-review L shortcut is tracked as R7. Streaming native encoding, measured frame
workers, pooled GPU textures, shared content caches and CPU pixel/encoded-plane pooling are
implemented, not new tasks. Full-frame linear APIs remain supported; automatic subtree
raster caching and DisplayList/PreparedFrame object pooling are not implied by storage
reuse. Further locality, preview wait reduction and decode scratch tuning need profiling
after measurements resume, without changing geometry, density, alpha or replay.

R3's explicit isolated-layer raster cache is implemented with bounded source identity
and lifetime. Rasterized replay differences have separate opt-in approval; ordinary
nodes remain unchanged. CPU functional tests and a named native GPU replay fixture
pass on the current macOS host. Timing gains and Gallery adoption remain unmeasured
until performance measurements resume.

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
6. Explore IK/constraints, richer procedural materials, video sources and project
   persistence. Define bounded resources, timing, failure and cache contracts first.

## Declarative Documents

The format, parameters, bindings, content kinds, hot reload and diagnostics are described in
[declarative documents](design/documents.md). Remaining work, in order:

1. Add reusable definitions: named styles, templates and component references
   (`defs` / `use`) that expand during load, so large films stop repeating subtrees.
   Expansion must stay bounded and report the referring path like `$param` does.
2. Close the content gaps. Particles lack image emitters, force fields, sprite sheets,
   bokeh and trail options; formulas lack subexpression selection; Typst page documents,
   animated SVG and image-sequence sources are not describable; audio lacks
   analysis-driven signals, synthesis, speed/reverse transforms, metadata and chapters.
3. Extend bindings: read an element's world transform and bounds, bind across scenes and
   to audio analysis, and reference whole points (`@id.position`) without components.
4. Add format migration: a `migrate` command and compatibility rules for the next
   version. Until a second version exists, other versions are rejected.
5. Evaluate Lottie/Bodymovin import as a converter into this format. Decide between an
   existing Rust implementation and a first-party subset only after checking license
   and binary size, and keep the dependency optional.
6. Export code-built timelines to documents for the builders that have a data form.
   Closure scenes cannot be serialized, so the supported subset must be explicit.
7. Test the parser and compiler with fuzzing or property tests (no panics, bounded
   work, every diagnostic path resolvable) and pin golden frames for the example
   documents on both backends.
8. Cache compiled documents in `Project` by file modification time, and make the
   retained-diagnostic cap (256) configurable.
9. Hot reload polish: watch resources that appear after launch, preserve the
   per-scene GPU context across reloads, and accept native window acceptance on real
   hosts. The automated fixtures use headless egui and do not replace it.
10. Shrink the schema (currently about 340 KB because every value position accepts a
    `$param` reference) by moving the reference into shared definitions, and generate
    value-range constraints where JSON Schema can express them.

## Completion Rules

Every change needs focused assertions and applicable package/feature compilation,
ordinary tests, documentation tests and explicit environmental fixtures. Performance
or hardware claims require their own measured evidence. Keep tests inline in their
owning source modules until they exceed roughly 300 lines, then move them to a sibling
`tests.rs` (see [Source style](style.md)); do not remove independent assertions to make a
gate pass.