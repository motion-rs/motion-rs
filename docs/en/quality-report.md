# P6 Image Quality Measurement

> **Provenance.** Dated snapshot; the numbers below do not update themselves.
> Figures in [Results](#results) were re-measured at `bbbdad3` on macOS 26.6.2 (arm64,
> Apple M1 Max) with `rustc 1.99.1 (58cf8f9c3 2026-10-13)`; they match the original run.
> The workspace and Rustdoc totals in [Acceptance](#acceptance) were recorded earlier during the
> cinematic acceptance and are kept as history; the current totals are listed under
> [Current Totals](#current-totals).

## Method

`examples/quality_metrics.rs` prints the P6 report as JSON; the measurements live in
`examples/quality_metrics/metrics.rs`, which `crates/motion/tests/quality_metrics.rs`
includes so the same code backs a non-ignored regression test. Everything evaluates the
production CPU compile/prepare/linear render path, without performance timing.

- **Fine lines.** 0.25, 0.5 and 1px lines, vertical and horizontal, move in 1/16px steps
  (17 positions). Each pixel across the line is compared with its exact analytic
  coverage (`max_coverage_error`, `mean_coverage_error`). Integrated alpha is compared
  with the analytic area (`energy_relative_error`) and across the sweep
  (`energy_relative_range`, the flicker measure), alongside the adjacent-frame mean
  absolute alpha difference.
- **Text.** "Motion" in bundled Noto Sans moves by the same steps: relative integrated
  alpha range, per-pixel direction reversals above one coverage code (also as a fraction
  of pixels that change during the sweep) and the largest one-step alpha change.
- **Gradient banding.** A 1024px black-to-white float gradient: distinct linear f32
  values, monotonicity, longest run of equal values, and a plain 8-bit sRGB quantization
  (distinct codes and largest code step). The output stage adds Bayer dither on top.

These are measurements, not perceptual acceptance scores. A text pixel can reverse
when another contour crosses it; reversal counts alone do not prove a raster bug.
The renderer stores analytical path coverage at 8-bit precision, while paints and
composition remain f32. No new AA/coverage algorithm is introduced without evidence.

## Results

Measured on the native macOS CPU path with
`cargo run --locked --offline -p motion-rs --all-features --example quality_metrics`.

| Line Width | Max Coverage Error | Mean Coverage Error | Energy Error vs Area | Energy Range |
| --- | --- | --- | --- | --- |
| 0.25px | 0.00098 (0.25 code) | 0.000082 | 0.39% | 3e-8 |
| 0.5px | 0.00196 (0.5 code) | 0.000163 | 0.39% | 4e-8 |
| 1px | 0.00196 (0.5 code) | 0.000154 | 0.39% | 0.39% |

Coverage is exact to half an 8-bit code in both orientations; the residual area error
and the 1px line's 0.39% energy range are independently rounded 8-bit edge coverage,
and narrow-line energy is effectively constant across the sequence. Text integrated
alpha varies by 0.05%; 48 of 650 pixels that change during the sweep (7.4%) reverse
direction above one coverage code, and no pixel moves by more than 0.063 (one 1/16px
step). The pre-quantization gradient has **1024 distinct monotone f32 values across
1024 pixels** with no equal neighbours; plain 8-bit sRGB quantization already reaches
all 256 codes with a largest step of one code, so there is no intermediate paint
collapse before the output stage.

The regression test asserts: coverage error <= 1.5 codes (mean <= 0.75 code), line
energy error and flicker <= 1%, text energy range <= 0.5%, text reversal fraction
<= 12% and one-step alpha change <= 0.1, and a monotone, fully distinct float gradient
with >= 250 8-bit codes and steps <= 4 codes.

These data do not justify an unmeasured AA rewrite. Keep the current coverage
policy and use the same fixtures to evaluate future subpixel/coverage proposals.
Native CPU/GPU optical/display parity and native Main10 precision/metadata tests
also passed, including public export with CPU and GPU scene rendering.
No performance benchmark or timing comparison was executed for this report.

## Acceptance

The final all-feature workspace functional run (excluding `motion-bench`) passed
767 tests; 113 remained ignored under their existing native/slow-test policies.
All 71 public API doctests passed. Explicit native cinematic GPU parity, native
Main10 precision/metadata and public CPU/GPU Main10 export checks also passed.
The breadth light-case functional test passed without invoking its timing paths.
LUT generation passed four-style, red-fast identity, finite-entry, invalid-input
and no-overwrite checks. Clean/film stills were generated through the example and
visually inspected: [clean](../assets/cinematic-clean.png),
[film](../assets/cinematic-film.png).

The complete film recipe exposed a preparation guard-band bug: sibling shutter
samples were counted as serial filters. Preparation now sums nested filter chains
and takes the maximum across siblings; a 64-sibling regression passes.

Structure and production `expect` inventory checks passed. All 42 touched Rust
files passed formatting/declaration-spacing checks. The repository-wide formatter
is blocked by a pre-existing formatting difference in
`.github/skills/motion-video/assets/main.rs`; that unrelated file was left unchanged.

## Current Totals

At `bbbdad3`, on the same host: `cargo test --workspace --all-features --lib --bins --tests
--examples` passes 876 tests with 125 ignored (native fixtures), and
`cargo test --workspace --all-features --doc` passes 113 doctests, 36 of them compiled
tutorial chapters. `cargo clippy --workspace --all-features --all-targets -- -D warnings`
and `node tools/check-structure.cjs`, `node tools/check-expects.cjs`,
`node tools/rust-format.cjs --check` are clean. No benchmark or timing run is implied.

## Reproduction

Run from the workspace root. The text fixture is
`tests/fixtures/fonts/NotoSans-Regular.ttf`; no system font discovery is involved.
Cinematic CPU/GPU parity and Main10 precision are functional tests. Their native
fixtures are ignored by default and selected explicitly during acceptance.