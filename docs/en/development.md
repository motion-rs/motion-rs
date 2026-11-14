# Development Checks

This is the maintenance reference for compilation, tests, fixtures and CI.
Commands below are opt-in validation procedures; reading documentation does not
execute them. See [reliability](design/reliability.md) for failure/ownership guarantees and
[Benchmarks](benchmarks.md) for workload and measurement rules.

## Continuous Integration

Workflows live in `.github/workflows/` and run on pull requests and on pushes to `main`. A new
commit on a pull request cancels the previous run of the same workflow; every workflow has
read-only `contents` permission.

| Workflow | Jobs | Runner |
| --- | --- | --- |
| Structure | Tool self-tests, `check-structure`, `check-expects`, `check-docs`, `cargo fmt`, `rust-format --check` | Linux |
| Rust | Clippy with `-D warnings`, workspace tests, Rustdoc tests (including the compiled tutorial) and strict `cargo doc` with all features; tutorial doctests with no default features; `motion-content` unit tests for four feature sets | macOS for all features, Linux otherwise |
| Facade Features | The 24 independent facade consumers of `tools/check-features.cjs` | Linux, and macOS where FFmpeg is needed |

Native FFmpeg jobs install FFmpeg, `pkg-config` and LLVM with Homebrew. CI compiles and runs the
non-ignored tests only; GPU, font and FFmpeg fixtures marked ignored still need an explicit local
run, and a green CI does not certify Windows or other graphics adapters. `.github/dependabot.yml`
proposes weekly updates for GitHub Actions and Cargo dependencies, and pull requests use
`.github/pull_request_template.md`.

## Automated Checks

Source conventions are described in [Source style](style.md); `node tools/rust-format.cjs`
applies rustfmt plus the item-spacing rule, and the structure check enforces file layout.

```sh
CARGO_INCREMENTAL=0 cargo check --locked --offline -j 2 --workspace --all-features --all-targets
CARGO_INCREMENTAL=0 cargo clippy --locked --offline -j 2 --workspace --all-features --all-targets -- -D warnings
CARGO_INCREMENTAL=0 cargo test --locked --offline -j 2 --workspace --all-features --lib --bins --tests --examples
CARGO_INCREMENTAL=0 cargo test --locked --offline -p motion-core -p motion-render --test properties
CARGO_INCREMENTAL=0 cargo test --locked --offline -j 2 --workspace --all-features --doc
node tools/check-structure.cjs && node tools/check-expects.cjs && node tools/check-docs.cjs && node tools/rust-format.cjs --check
RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links -D rustdoc::private_intra_doc_links" cargo doc --locked --offline --workspace --all-features --no-deps
```

The Rust blocks in `docs/tutorial/*.md` are compiled as Rustdoc tests of the facade crate
(`crates/motion/src/lib.rs`, module `tutorial`), each chapter gated by the features it needs.
Use `rust,no_run` for examples that touch files, GPUs or windows, and `text`/`sh` for
anything that is not Rust; an untagged fence would be compiled as a test.

Also check the minimal facade and Studio without scene GPU, including
`cargo check --locked --offline -p motion-encode --no-default-features`.
The facade feature CI compiles independent consumers, not workspace test targets,
so unrelated packages and dev-dependencies cannot supply missing features.
Run the same 24-case matrix locally with `node tools/check-features.cjs --offline`,
or select a case with `node tools/check-features.cjs --features gpu,particles --offline`.
Each temporary consumer disables default features and depends only on the facade;
`default` explicitly tests the default feature set. The workspace lockfile is copied
when available, and build artifacts are reused in `target/feature-consumer`.
CI checks non-native cases on Linux and FFmpeg-dependent cases on macOS with
Homebrew FFmpeg development libraries. These are compile checks, not GPU/codec
runtime tests. Separate CI jobs run workspace unit/integration/example tests and
Rustdoc tests on macOS, plus content tests without text on Linux. Ignored native
fixtures still require explicit selection. Use `node tools/check-features.cjs --self-test`
to test the driver.
Raster kernels carry budget, error path and band geometry in `RasterScope` and
`Band`, so no `too_many_arguments` expectations remain; generated FFmpeg ABI
declarations are excluded only from type-complexity linting. Third-party future-incompatibility reports are not
project-source lint failures.

### Production Panic Inventory

Run `node tools/check-expects.cjs` for counts or append `--json` for every retained
production `expect` with its file, line, decision and owner-specific reason. The
driver tests its source lexer and cfg handling before scanning; unknown occurrences
fail the check. It excludes tests, masks strings/comments and conservatively
includes the union of feature/target branches. It is a source-level inventory,
not AST analysis or macro expansion, and does not inventory `unwrap` or `panic!`.
Review the surrounding controlling code when changing a reason: a matched message
does not prove an invariant. Distinguish validated private state, documented
convenience panics, poison fail-fast policy and bundled-source assumptions; the
latter three are not claims that failure is impossible.

Core and raster property tests use `proptest` with 128 cases per property and
shrinking/failure persistence. They cover signed rational-frame round trips,
finite/infinite `zip` and finite `then` duration/boundary rules, independent
per-pixel rectangle areas versus compact spans/strips, and finite bounded polygon
coverage consistent with CPU output. `PROPTEST_CASES` and `PROPTEST_RNG_SEED` can
increase coverage or reproduce a run; generated regressions should be retained.

Explicit fixture tests
need a supported graphics adapter and native FFmpeg codecs where indicated.
The paragraph fallback and Unicode line-breaking tests use bundled
[Noto fixtures](../../tests/fixtures/fonts/README.md) and run without system fonts or
environment variables.

For ignored font fixtures, set MOTION_TEST_FONT to the bundled
tests/fixtures/fonts/NotoSans-Regular.ttf and MOTION_TEST_FALLBACK_FONT to
tests/fixtures/fonts/NotoSansCJKsc-Regular.otf. Chinese-only fixtures require the
CJK font as MOTION_TEST_FONT instead. Color bitmap emoji
fixtures additionally require MOTION_TEST_EMOJI_FONT as described in the fixture
README. Select ignored tests by their full names. Do not equate an ignored
test, a zero-test filter or a successful compile with runtime acceptance.

Pixel checks require nonblank subject regions, a named
[visual acceptance](design/reliability.md#visual-acceptance) level or tier, exact CPU
seek replay and Replay-tier GPU replay. Audio fixtures inspect actual decoded samples and timing;
video fixtures decode frame counts and metadata. No test should invoke ffmpeg/ffprobe
executables. Native-window interaction, real output devices and other platforms need
separate validation.

### Independent backend acceptance

Native GPU fixtures prepare CPU and GPU coverage independently and compare them with
the [visual acceptance](design/reliability.md#visual-acceptance) tiers; geometry cases
must report independent GPU coverage.

```sh
cargo test --locked --offline -p motion-render --all-features --lib gpu:: -- --include-ignored --test-threads=1
```

The renderer suite includes 24 fixed random seeds under both winding rules,
exact duplicate even-odd cancellation, tile seams,
large coordinates, guard-band filters, procedural paints, analytic trajectories,
residency and exact byte/work/device-limit admission. Existing CPU reference
tests and goldens remain unchanged. GPU fixtures certify only the adapter actually
used. Large-scene acceptance across text, SVG, clipping and particles is
planned.

## Frame timing diagnostics

The breadth and performance benchmarks accept `MOTION_BENCH_TIMINGS=1` to collect per-frame host
stage observations and available GPU timestamps. This is separate from
`MOTION_BENCH_STAGES`, which measures selected CPU pipeline stages. Timing
diagnostics require `MOTION_BENCH_ITERATIONS=1`, exclude warmups from the recorded
stage arrays, and are disabled by default. Use a separate `MOTION_BENCH_OUTPUT`
directory; do not treat instrumented wall times as optimization acceptance runs.
Normal reports retain their original metrics when this switch is disabled.

Host encoding includes command encoding/submission; host readback includes
completion waits, mapping and extraction. Device intervals overlap these host
observations and must not be added to them. Preserve all raw device values, but
flag an interval longer than its enclosing frame wall time as untrusted. Plausible
values alone do not establish timestamp correctness. The diagnostic JSON preserves raw
ticks and distinguishes invalid intervals, passes that did not execute and devices
without timestamp support. Instrumentation payload is
reported separately from logical raster budgets and is not a physical-memory
measurement. Check free space before building heavyweight optional features, and
do not delete frozen benchmark artifacts to make room without an explicit decision.
