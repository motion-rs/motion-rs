# Benchmarks

`motion-bench` owns three targets. They answer different questions, so their results are never
merged into one number.

| Target | Question | Harness |
| --- | --- | --- |
| `performance` | What does the engine cost across the 60 core and breadth cases, sizes, pipeline stages and primitives, and did a change move them? | custom |
| `films` | How long do real Gallery films take to render? | custom |
| `criterion_frames` | Quick statistical check of a handful of CPU frames. | criterion |

No baseline is frozen or registered in the repository. Raw run directories are local,
git-ignored artifacts; a before/after claim comes from a paired comparison run on one machine
(see [Comparing Two Builds](#comparing-two-builds)).

## Cinematic Coverage

The breadth cases include `filter-bloom`, `filter-lens`, `filter-lut`, and
`display-transform` (AgX plus seeded encoded grain). Functional/native parity and P6
image-quality measurement are described in [the quality report](quality-report.md).

## Opt-In Film Benchmarks

The `films` target requires the `films` feature, which is not enabled by default.
Cargo skips it in ordinary benchmark selection, including `--benches`, unless
that feature is enabled. Explicitly run it when needed:

```sh
cargo bench -p motion-bench --bench films --features films            # CPU
cargo bench -p motion-bench --bench films --features films,gpu        # CPU and GPU
cargo bench -p motion-bench --bench films --features films,gpu,ffmpeg # Include native video export
```

Every Gallery catalog entry is a `films` case; select a subset with `MOTION_BENCH_CASES`.

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

## Running `performance`

```sh
cargo bench -p motion-bench --bench performance                  # CPU
cargo bench -p motion-bench --bench performance --features gpu   # CPU and GPU
```

Results go to `target/performance/performance-<timestamp>/` (override with
`MOTION_BENCH_OUTPUT`) as `report.json`, TSV and reference images. The target needs the
`image`, `text`, `svg`, `particles`, `audio` and `diagram` features, all default. Custom
targets default to two warmups, 101 samples and one iteration per sample; record every
override. Use `MOTION_BENCH_SAMPLES=5` for a smoke run.

| Variable | Default | Effect |
| --- | --- | --- |
| `MOTION_BENCH_CASES` | all 60 | Comma-separated cases from the core and breadth groups. |
| `MOTION_BENCH_BACKENDS` | `cpu` (`cpu,gpu` with the feature) | Backends to measure. |
| `MOTION_BENCH_SIZE` | `640x360` | Pixel size of the render cases; the design size stays 640x360. |
| `MOTION_BENCH_SCALING` | `0` | Repeat `SCALING_CASES` at every size in `MOTION_BENCH_SIZES`. |
| `MOTION_BENCH_SIZES` | `1280x720,1920x1080` | Sizes for the scaling group, up to 4096 per side. |
| `MOTION_BENCH_STAGES` | `0` | Time CPU pipeline stages for every selected case. |
| `MOTION_BENCH_SEQUENCES` | `1` | Record sequential playback and random seeks. |
| `MOTION_BENCH_COLD` | `0` | Record cold-session frames. |
| `MOTION_BENCH_MICRO` | `1` | Record microbenchmarks. |
| `MOTION_BENCH_VIDEO` | `0` | Record video export runs (feature `ffmpeg`); `MOTION_BENCH_VIDEO_FRAMES` sets the length. |
| `MOTION_BENCH_PARTICLES` | `72` | Particle count of the core particle cases. |
| `MOTION_BENCH_FONT` | repository Noto Sans fixture | Font for the text cases. |

### What it measures

- **Core cases (29).** Shapes, fills, strokes, ink, isolation, blur, shadow, images, masks,
  transitions, bloom, grading, motion blur, particles, text, SVG, path sampling and morphing,
  rendered at 640x360.
- **Breadth cases (31).** Gradients (linear, radial, sweep), twelve blend modes, path, nested and
  shape clips, dashed, wide and hairline strokes, a self-intersecting even-odd path, nested and
  wide isolated layers, 1k/10k/50k-node scenes, an 800-leaf Row/Column layout, a large blur,
  chromatic aberration, a three-filter chain, bloom, lens and LUT filters, a display transform, a
  long paragraph, 200 labels, ballistic, simulated, sub-emitter, collision and parallel particle
  systems and three blurred siblings.
- **Per case.** `warm-frame`, `sequential-playback` and `random-seek` per backend. Before
  anything is timed, four frames are replayed after an unrelated seek and must match (exactly on
  the CPU, within the visual tolerance on the GPU); the frame must not be flat.
- **Memory output.** `filter-siblings` and `display-transform` also record `memory-frame`, the
  straight RGBA8 `render_output_at` path that uses adaptive bands, checked against `render_frame`.
- **Scaling.** `nodes-10k`, `gradient-linear`, `blend-modes` and `blur-large` at each configured
  size, recorded as `<backend>/<case>@WxH/warm-frame`.
- **Stages.** `build`, `compile-fresh-node`, `compile`, `prepare`, `linear` and `output` for CPU
  cases. `prepare` runs without the coverage cache, so it shows the cold cost that a
  `warm-frame` hides.
- **Microbenchmarks.** sRGB round trips, Oklab interpolation, transform composition, SVG path
  parsing, stroke outlines, the deterministic random generator, noise, Row/Column measurement, a
  four-clip audio mix with resampling, RGBA8 conversion and PNG encoding at 1080p, and label
  layout.
- **Rejections.** An engine admission limit (`StructuralLimit`) is recorded in `rejected.txt`
  instead of aborting the run, because "this scene does not fit the budget" is a result.

These measurements do not establish Studio FPS, process RSS or full-film acceptance.

### Functional checks

`cargo test -p motion-bench --test performance_cases` renders every light breadth case at 160x90
and checks determinism and non-flat output without timing. Cases that take seconds in an
unoptimized build are in the ignored test:

```sh
cargo test --release -p motion-bench --test performance_cases -- --include-ignored
```

## One-Command Comparison

`benches/run.cjs` builds and runs the `performance` target for a control and for the working
tree, runs the alternating pairs and writes a report and a chart:

```sh
node benches/run.cjs --control-ref HEAD --pairs 6 --features gpu
MOTION_BENCH_CASES=shapes,blur,text node benches/run.cjs --control-ref main --label blur-opt
```

The control is built from `--control-ref` (default `HEAD`) in a temporary git worktree, or taken
from an existing run directory with `--control <run-dir>`. Any `MOTION_BENCH_*` variable is
passed through. Output goes to `target/performance/compare-<timestamp>/` (override with
`--output`): `compare.md` is the table and `compare.svg` is a bar chart of the paired P50 change
per case with its bootstrap interval (`--phase` selects the phase, default `warm-frame`; green is
improved, red regressed, grey uncertain). `node benches/chart.cjs --pairs <dir> --output <file>`
redraws a chart from an existing pair directory. The run executes the full benchmark
`2 x pairs + 2` times, so narrow it with `MOTION_BENCH_CASES` and `MOTION_BENCH_SAMPLES`.

## Comparing Two Builds

Keep the control executable and compare it with the candidate on the same machine. Each of
`--control` and `--candidate` is a completed run directory containing `report.json` and the
frozen `benchmark-executable`.

```sh
node benches/pair.cjs --control <control-run> --candidate <candidate-run> --output target/performance/pairs --label <name> --pairs 6
node benches/report.cjs --pairs target/performance/pairs --output target/performance/compare.md
```

`pair.cjs` runs the two executables in alternating A/B then B/A order with the configuration
of the control report, and refuses runs whose configuration, environment or executable
provenance differ. `report.cjs` prints, per case and phase, the paired P50 change with a
deterministic bootstrap interval, an assessment, the P95 change and the MAD.

- Time improvement = `(control - candidate) / control * 100%`. Positive is faster.
- The interval needs at least five pairs; smaller sets are exploratory, and an interval
  crossing zero is uncertain.
- P95 requires at least 100 individual frame or operation samples in every run.
- Cases, size, quality, budgets, fonts, features, sampling, machine, compiler and adapter must match.
- Checksums and exact reference pixels are validated. Logical peak bytes and work are not process RSS.
- No cross-case average speedup is presented. Cold sessions, warm frames, seeks, I/O and encode
  timings stay separate.

## Optional Focused Control Matrix

`benches/baseline.cjs` fixes 14 CPU/GPU cohorts at 1080p: an empty control,
shapes, SVG, text, 10000 orbit particles, long ink strokes and 60-frame H.264 export.
Each healthy cohort requires three independent executions of its frozen program.
Frame/operation phases use 101 samples; full-export phases use five and cannot
establish P95. Failures retain logs and are reported without increasing limits or dropping cases.

```sh
cargo bench --locked --offline -p motion-bench --bench performance --features gpu,ffmpeg --no-run
node benches/baseline.cjs --phase probes --state target/performance/baseline.json --program <performance-executable>
node benches/baseline.cjs --phase repeats --state target/performance/baseline.json
```

It includes video export; run it only when that measurement is intended.
