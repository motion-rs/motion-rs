# Benchmarks

`motion-bench` owns four targets. They answer different questions, so their results are never
merged into one number.

| Target | Question | Harness | Frozen baseline |
| --- | --- | --- | --- |
| `performance` | Did a focused optimization change the 29 reference cohorts at 640x360 or 1080p? | custom | yes, see [Current Measurements](#current-measurements) |
| `films` | How long do real Gallery films take to render? | custom | no |
| `criterion_frames` | Quick statistical check of a handful of CPU frames. | criterion | no |
| `breadth` | What does the engine cost across content types, sizes, pipeline stages and primitives? | custom | no |

`breadth` exists so that coverage can grow without disturbing the frozen `performance`
workload: that target fingerprints its own source, so editing it invalidates registered runs.
Adding a case to `performance` changes its `workload_fingerprint`, which makes every
registered series incomparable; put new coverage in `breadth` unless a new series is
being started on purpose.

## Cinematic Coverage

The breadth target now includes `filter-bloom`, `filter-lens`, `filter-lut`, and
`display-transform` (AgX plus seeded encoded grain). These cases do not alter the
frozen performance cohorts. This implementation adds coverage only: no performance
benchmark was executed or compared. Functional/native parity and P6 image-quality
measurement are described in [the quality report](quality-report.md).

## Opt-In Film Benchmarks

The `films` target requires the `films` feature, which is not enabled by default.
Cargo skips it in ordinary benchmark selection, including `--benches`, unless
that feature is enabled. Explicitly run it when needed:

```sh
cargo bench -p motion-bench --bench films --features films            # CPU
cargo bench -p motion-bench --bench films --features films,gpu        # CPU and GPU
cargo bench -p motion-bench --bench films --features films,gpu,ffmpeg # Include native video export
```

The `quiet-hours` and `chidori` Gallery films are intentionally excluded from this target (and
from `MOTION_BENCH_CASES` selection): they are showcases, not performance workloads.

`--all-features` also enables `films`; avoid it when excluding long film runs.
The marker only gates target selection; GPU and native encoding remain separate
features and film measurement settings remain unchanged.

## CPU Criterion Frames

Criterion provides a separate warm CPU full-frame benchmark, not a replacement
for the frozen executable/reference-image and independent AB/BA process protocol:

```sh
cargo bench --locked --offline -p motion-bench --no-default-features --features text,particles --bench criterion_frames
cargo bench --locked --offline -p motion-bench --no-default-features --features text,particles --bench criterion_frames -- --test
```

The seven cases are 10,000 shapes, 100,000 particles in two 50,000-particle batches,
1,824 text characters, 32 gradients, 200 isolated layers and sigma-12 blur at
640x360, plus 64 tiles at 3840x2160.
All use the bundled Noto Sans font, explicit CPU sessions, structural `max_nodes`
100016, default 256 MiB logical byte allowance and default pixel allowance.
Particles and long text declare `max_work=4000000000`; the other cases use the
authoring default without a work ceiling. Reports captured before the benchmark
refactor are not measurements of these cases or the current defaults.
Default-budget rejection is not rendered-frame timing. Setup,
first frame and exact replay checks precede measurement. Each Criterion sample
includes a full warm-session frame; it is not cold startup, GPU, P95 or process
RSS evidence. Criterion currently uses ten samples, one-second warmup and
three-second measurement time per case; its batches estimate iteration cost,
not individual-frame tail latency. `--test` is only a functional smoke check.

## Running `breadth`

```sh
cargo bench -p motion-bench --bench breadth                      # CPU
cargo bench -p motion-bench --bench breadth --features gpu       # CPU and GPU
```

Results go to `target/performance/breadth-<timestamp>/` (override with `MOTION_BENCH_OUTPUT`)
using the same `report.json`, TSV and reference-image layout as the other targets, so
`MOTION_BENCH_COMPARE` works unchanged. The usual `MOTION_BENCH_SAMPLES`, `MOTION_BENCH_WARMUPS`
and `MOTION_BENCH_ITERATIONS` apply; use `MOTION_BENCH_SAMPLES=5` for a smoke run.

| Variable | Default | Effect |
| --- | --- | --- |
| `MOTION_BENCH_CASES` | all | Comma-separated render cases. |
| `MOTION_BENCH_BACKENDS` | `cpu` (`cpu,gpu` with the feature) | Backends to measure. |
| `MOTION_BENCH_SIZE` | `640x360` | Pixel size of the render cases; the design size stays 640x360. |
| `MOTION_BENCH_SCALING` | `1` | Repeat `SCALING_CASES` at every size in `MOTION_BENCH_SIZES`. |
| `MOTION_BENCH_SIZES` | `1280x720,1920x1080` | Sizes for the scaling group, up to 4096 per side. |
| `MOTION_BENCH_STAGES` | `1` | Time CPU pipeline stages for `STAGE_CASES`. |
| `MOTION_BENCH_SEQUENCES` | `1` | Record sequential playback and random seeks. |
| `MOTION_BENCH_MICRO` | `1` | Record primitive microbenchmarks. |
| `MOTION_BENCH_FONT` | repository Noto Sans fixture | Font for the text cases. |

### What it measures

- **Render cases (24).** Gradients (linear, radial, sweep), twelve blend modes, path, nested and
  shape clips, dashed, wide and hairline strokes, a self-intersecting even-odd path, nested and
  wide isolated layers, 1k/10k/50k-node scenes, an 800-leaf Row/Column layout, a large blur,
  chromatic aberration, a three-filter chain, a long paragraph, 200 labels, 20k ballistic and
  6k simulated particles and three blurred siblings. Each case records `warm-frame`, `sequential-playback` and `random-seek`
  per backend. Before anything is timed, three frames are replayed after an unrelated seek and
  must match pixel for pixel, and the frame must not be flat.
- **Memory output.** `filter-siblings` also records `memory-frame`, the straight RGBA8
  `render_output_at` path that uses adaptive bands, and checks it against `render_frame`
  (exactly on the CPU, within the GPU tolerance otherwise).
- **Scaling.** `nodes-10k`, `gradient-linear`, `blend-modes` and `blur-large` at each configured
  size, recorded as `<backend>/<case>@WxH/warm-frame`.
- **Stages.** For nine CPU cases: `build`, `compile-fresh-node`, `compile`, `prepare`, `linear`
  and `output`. `prepare` runs without the coverage cache, so it shows the cold cost that a
  `warm-frame` hides.
- **Microbenchmarks.** sRGB round trips, Oklab interpolation, transform composition, SVG path
  parsing, stroke outlines (uncached and cache hit), the deterministic random generator, noise,
  Row/Column measurement, a four-clip audio mix with 44.1 to 48 kHz resampling, RGBA8 conversion
  and PNG encoding at 1080p, and label layout.
- **Rejections.** An engine admission limit (`StructuralLimit`) is recorded in `rejected.txt`
  instead of aborting the run, because "this scene does not fit the budget" is a result.

### Functional checks

`cargo test -p motion-bench --test breadth_cases` renders every light case at 160x90 and checks
determinism and non-flat output without timing. Cases that take seconds in an unoptimized build
are in the ignored test:

```sh
cargo test --release -p motion-bench --test breadth_cases -- --include-ignored
```

## Non-Film Rerun

Use this scope for current-source measurements after the benchmark refactor. Keep
`films` disabled and explicitly select `performance`, `breadth` and
`criterion_frames`; `--all-features` enables the film gate and is not needed here.
If obsolete history must be cleared, use the separate [history reset](#benchmark-history-reset)
before starting, not between these runs.

```sh
cargo bench --locked --offline -p motion-bench --features gpu,ffmpeg --bench performance --bench breadth --bench criterion_frames --no-run
MOTION_BENCH_FONT=tests/fixtures/fonts/NotoSans-Regular.ttf cargo bench --locked --offline -p motion-bench --features gpu,ffmpeg --bench performance
cargo bench --locked --offline -p motion-bench --features gpu,ffmpeg --bench breadth
cargo bench --locked --offline -p motion-bench --features gpu,ffmpeg --bench criterion_frames
```

Custom targets default to two warmups, 101 samples and one iteration per sample;
record every override. Sequences use their configured per-frame schedule, not the
warm-frame sample count. Criterion uses its own sample/warmup/measurement settings.
Keep target results separate, retain budget rejections, and register only healthy
validated reports. These measurements do not establish Studio FPS, process RSS
or full-film acceptance. Comparison rules are in [Interpretation](#interpretation).

## Optional Focused Control Matrix

`benches/baseline.cjs` fixes 14 CPU/GPU cohorts at 1080p: an empty control,
shapes, SVG, text, 10000 orbit particles, long ink strokes and 60-frame H.264 export.
Each healthy cohort requires three independent executions of its frozen program.
Frame/operation phases use 101 samples; full-export phases use five and cannot
establish P95. The existing 1 GiB/20-billion-work benchmark policy is unchanged.
Failures retain logs and are reported without increasing limits or dropping cases.

```sh
cargo bench --locked --offline -p motion-bench --bench performance --features gpu,ffmpeg --no-run
node benches/baseline.cjs --phase probes --state target/performance/baseline.json --program <performance-executable>
node benches/baseline.cjs --phase repeats --state target/performance/baseline.json
node benches/baseline.cjs --phase register --state target/performance/baseline.json
```

This optional matrix is not the complete non-film rerun and is not implicitly
executed by it. It uses the `performance` target, not `films`, but includes video
export; run it only when that export measurement is intended. After registration,
use the [report update procedure](#update-procedure).

## Benchmark History Reset

Reset only when a source/workload refactor invalidates the active baseline, before
creating fresh measurements. Do not reset during a measurement or for routine
control/candidate comparisons; it deletes the preserved raw evidence.

```sh
node benches/baseline.cjs --phase reset
```

`reset` deletes the two ignored result roots (`target/performance` and
`crates/motion/target/performance`) and resets [performance-runs.json](performance-runs.json).
It does not delete build caches, sources, fonts or ordinary test logs. Removed
tracked history remains recoverable from Git; deleted ignored raw artifacts do not.

## Current Measurements

Historical smoke timings were removed after the benchmark refactor; they are not
a baseline for the current source or workload. The generated report below
records validated current runs, sampling, provenance and any rejected cases.
This refresh records only `performance`; `breadth` and `criterion_frames` are
deferred, and `films` is deliberately excluded. Do not infer film-export or Studio FPS from
these frame, stage and microbenchmark measurements.

The registered baseline predates the R3-8 rolling CPU blur change. It remains
evidence for its frozen executable. The separate focused comparison below uses
fresh control/candidate runs; it does not turn the original full-suite timings
into a measurement of the optimization.

### R3-8 Paired Check

Six alternating AB/BA pairs on Apple M3/Metal, 640x360 Final, two warmups and
101 samples. Only Empty/Shapes/Blur/Shadow/Bloom were measured, with CPU stages;
sequences, microbenchmarks, cold sessions and video were disabled. Both frozen
executables produced exactly matching reference pixels and checksums in all runs.

| CPU Warm Frame | Control P50 (ms) | Candidate P50 (ms) | Paired Improvement (95% Interval) | Assessment |
| --- | ---: | ---: | --- | --- |
| Blur (small kernel) | 13.228 | 27.372 | -106.70% [-116.20, -100.56] | regressed |
| Shadow (sigma 2) | 17.301 | 28.136 | -64.78% [-65.86, -58.27] | regressed |
| Bloom (sigma 8) | 83.408 | 46.990 | +42.88% [41.05, 45.79] | improved |

Displayed P50 values are medians across runs; improvement is the median of
paired relative changes, not the ratio of those two displayed medians. CPU
stage-linear measurements confirm the same directions. Empty/Shapes and GPU
warm-frame P50 changes are uncertain; GPU Empty P95 has a small borderline
regression (-1.12%, interval [-1.56, -0.01]) and remains in the report.

This first all-rolling variant did not pass performance acceptance: smaller
uniform kernels regressed and needed further work. Do not average these outcomes or extrapolate them
to films, Studio FPS, larger sigma/resolutions, or process RSS. All 50 case/phase
results were recorded in the local, untracked `target/performance/r3-8-1793849842040/`
report; incomplete or disk-full attempts are not counted as pairs.

### R3-8 Small-Kernel Follow-Up

Uniform radii 1/2 now use direct integer weighted taps; larger radii keep the
compensated rolling path. Six new alternating AB/BA pairs use the same settings
and original pre-R3-8 control, not the all-rolling candidate. This cohort is
registered separately; it is not pooled with the first comparison above.

| CPU Warm Frame | Control P50 (ms) | Candidate P50 (ms) | Paired Improvement (95% Interval) | Assessment |
| --- | ---: | ---: | --- | --- |
| Blur (small kernel) | 13.138 | 5.520 | +57.97% [57.26, 58.41] | improved |
| Shadow (sigma 2) | 16.949 | 7.477 | +55.89% [55.59, 56.24] | improved |
| Bloom (sigma 8) | 81.626 | 46.645 | +42.91% [42.76, 43.35] | improved |

All three CPU P95 and stage-linear estimates also improve. CPU Empty/Shapes
warm-frame and all GPU warm-frame P50/P95 changes are uncertain. The off-target
Shapes hot-node compilation P50 has a borderline regression (-1.39%, interval
[-4.58, -0.01]); it is retained and not attributed to filter logic. This supports
the measured CPU filter targets, not regression-free overall acceptance or
film/Studio FPS, RSS, larger-sigma/resolution or cross-device claims.

All 12 runs match exact reference pixels/checksums, configuration and frozen
programs. Default/GPU CPU-filter regressions, production checking and the native
53-case GPU parity check pass. Post-measurement source changes are comments and
existing tests only; measured runtime equivalence is verified. All 50 outcomes,
both quantiles and provenance were recorded in the local, untracked
`target/performance/r3-8-small-kernel-1793853335012/` report and its sibling
comparison/source-verification files. Previous regressions and
failed attempts remain intact.

The report markers delimit the section maintained by `node benches/report.cjs`;
the running guide above remains handwritten.

<!-- performance-report:start -->

These are current-source non-film benchmark observations, not an optimization against deleted historical measurements.
Registered measurements, verification notes and raw-run paths are in [performance-runs.json](performance-runs.json). Raw runs retain JSON/TSV, samples and reference images.

### Interpretation

- Time improvement = `(reference - current) / reference * 100%`. Positive is faster; negative is a regression.
- "Previous" compares with the immediately preceding step in the same series. "Initial" always compares with step 00.
- Values are medians across independent runs; MAD% reports run-to-run dispersion. Batch P95 is never called frame-tail latency.
- Previous-step improvement uses fresh paired controls with alternating A/B then B/A order. Historical baseline comparison is descriptive, not a causal estimate.
- Paired P50 changes have a deterministic bootstrap interval after at least five pairs. Smaller cohorts are exploratory; an interval crossing zero is uncertain.
- P95 requires at least 100 individual frame/single-operation samples in every run; otherwise it is explicitly insufficient.
- Cases, resolution, quality, budgets, fonts, features, sampling settings, machine/compiler and adapter must match within a series.
- Checksums, exact reference pixels and raw sample summaries are validated. Logical peak bytes/work are not process RSS or wall-clock time.
- No cross-case average speedup is presented. Cold sessions, warm frames, seeks, I/O and encode timings stay separate.

### Current Measurement

Source commit: 9cb8a9b1e5e3c4f72e817cbd259a5aaf47ef070c.

Targets: performance. Excluded: breadth, criterion_frames, films.

The full-suite initial baseline has one invocation, with CPU and GPU in the same process: two warmups, 101 warm-frame samples and one iteration per sample; playback and seek each use 90 frames. Warm-frame P95 is eligible, sequences and five batch-mean microbenchmarks are not. Separately, R3-8 has six fresh alternating AB/BA pairs for Empty/Shapes/Blur/Shadow/Bloom at 640x360, both backends, with CPU stages enabled and sequences/micro/video/cold disabled. A separate six-pair small-kernel follow-up compares the new candidate against the same original pre-R3-8 control; the two cohorts are not pooled. These focused pairs provide per-case uncertainty estimates, not a full-suite speedup claim.

Apple M3 (Metal GPU), macOS, aarch64, release, 640x360 Final, bundled Noto Sans, 72 particles, 1 GiB/20-billion-work benchmark policy. All 29 cases, both backends, playback/seek and five microbenchmarks were measured; cold sessions, stages and video export were disabled. GPU and FFmpeg were enabled in the frozen build, but films was not. Runtime/workload sources match the captured revision; post-build changes are reporting/archival-only edits to benches/report.cjs and benches/baseline.cjs, plus the scheduling-only films gate in crates/motion-bench/Cargo.toml. Breadth has incomplete artifacts and is deferred with Criterion; neither is registered. This is not Studio FPS, process RSS, full-film acceptance, cross-device evidence or improvement over deleted history. Verification details: target/performance/non-films-1793845901725/source-verification.json.

Historical series removed: 14.

### Conclusions

One row per series, using its latest step. Outcomes count case/phase comparisons, not a cross-case average. Representative deltas show up to two largest absolute P50 changes; every other result and negative control remains available in the detailed report.

| Series / Latest Step | Runs | Cases | P50 Outcomes | P95 Eligible Cases | Representative Timing / P50 Change |
| --- | ---: | ---: | --- | ---: | --- |
| [Engine / 640x360 / Apple M3](performance-runs.json) / 00 current baseline | 1 | 179 | baseline (no speedup claim): 179 | 58/179 | Cpu/empty/warm-frame: P50 1.255 ms; P95 1.914 ms<br>Cpu/shapes/warm-frame: P50 2.729 ms; P95 3.448 ms |
| [R3-8 focused / 640x360 / Apple M3](performance-runs.json) / 01 R3-8 rolling blur | 6 | 50 | uncertain: 43; regressed: 4; improved: 3 | 50/50 | Software/blur/stage-linear: -122.76% [-132.61%, -114.39%]; -35.59%<br>Cpu/blur/warm-frame: -106.70% [-116.20%, -100.56%]; -43.82% |
| [R3-8 small kernels / 640x360 / Apple M3](performance-runs.json) / 01 R3-8 small-kernel direct taps | 6 | 50 | uncertain: 36; improved: 13; regressed: 1 | 50/50 | Software/blur/stage-linear: +66.26% [+66.18%, +66.44%]; +79.45%<br>Software/shadow/stage-linear: +59.40% [+59.27%, +59.68%]; +66.64% |

### Raw Data And Detailed Tables

The active manifest preserves registered runs, change, verification and regression notes. Run directories contain validated samples, configuration, executable provenance and reference images; they are local artifacts, not bundled benchmark promises. Removed tracked history remains recoverable from Git, not active measurement rows.

Generate full per-case tables, MAD, incremental and initial deltas, P95 eligibility and step notes without expanding this page:

```sh
node benches/report.cjs --details --output target/performance/details.md
```

### Update Procedure

1. Keep the initial run directories and frozen executable. Run the same cohort before and after each focused optimization.
2. Register raw directories and change/verification/negative-result notes in [performance-runs.json](performance-runs.json). Use a new series for different workloads or hosts.
3. Run `node benches/report.cjs`. This validates every registered run and rebuilds the concise conclusions table; use --details with a separate --output for full tables.
4. Read the [performance plan](plan.md#performance) for the remaining optimization work and acceptance gates.

<!-- performance-report:end -->
