# Motion (motion-rs) Instructions

Motion is a programmable 2D video engine in Rust (edition 2024, Cargo workspace). A scene is a
pure function of explicit time; the same prepared frame feeds a CPU backend and a GPU backend.
Read [docs/design/README.md](../docs/en/design/README.md) (principles, glossary) and
[docs/design/architecture.md](../docs/en/design/architecture.md) (package ownership and dependency
direction) before changing structure. Rust conventions are in [docs/style.md](../docs/en/style.md) and
are repeated for code generation in [rust.instructions.md](instructions/rust.instructions.md).

## Hard Rules

- **Determinism.** Visuals depend only on explicit time and fixed inputs. Randomness comes from
  explicit seeds and keyed streams. Do not add global or hidden mutable state.
- **Explicit IO.** Nothing reads files, scans system fonts, opens windows or picks devices
  implicitly. Resources arrive through `Assets`, `FontLibrary` and `RenderOptions`.
- **Unsafe.** Library crates `#![forbid(unsafe_code)]`. Only `motion-ffmpeg` has `unsafe`, and every
  block needs a `// SAFETY:` comment.
- **Limits first.** Admit bytes and work through `Budget` / `RasterLimits` before allocating or
  looping over untrusted sizes. Budget counters are private; use its methods.
- **Errors.** Fallible APIs return `Result<_, ValidationError>` (or the crate error). New APIs are
  fallible only; do not add a panicking twin. Every production `expect` is registered with a reason
  in `tools/check-expects.cjs`.
- **CPU/GPU parity.** A rendering change touches the CPU path, the GPU plan, the WGSL shader and the
  host ABI together. Host and WGSL struct names and field order match, and the layout assertions in
  `crates/motion-render/src/gpu/layout.rs` must keep passing.
- **Output stability.** Do not change golden images, seeded sequences or numeric results unless the
  task says so; call it out when a change could.
- **Dependency direction.** Lower-level crates never depend on the facade. Only the facade
  (`crates/motion`, package `motion-rs`, library `motion`) assembles the public authoring API.
  Examples and gallery code use `motion::prelude::*`, never internal paths.

## Layout At A Glance

- `crates/motion-core`: authoring model, geometry, animation, audio description, budgets, noise.
- `crates/motion-render`: prepared frames, CPU raster/composition, GPU planning/execution, outputs.
- `crates/motion-content`: images, SVG, text, particles, ink, diagrams, captions.
- `crates/motion-formula`: Typst/formula compilation. `crates/motion-encode`: audio mixing and export.
- `crates/motion-ffmpeg`: the only `unsafe` FFI boundary. `crates/motion-studio`: desktop workbench.
- `crates/motion-bench`, `gallery/`, `examples/`, `tests/`, `tools/`, `benches/`, `docs/`.

Put new code with its owner: shared types in core, backend work in render, authored content in
content, and artwork or demo films in `gallery/` (never in core).

## Do Not

- Create `mod.rs` files, source files over 800 lines (tests exempt), `TODO`/`FIXME` comments, or
  markdown planning/notes files unless asked.
- Run benchmarks, add dependencies or change CI as a side effect of an unrelated task.
- Rely on `cargo fmt` alone: item spacing comes from `node tools/rust-format.cjs`, which rewrites the
  whole workspace. With uncommitted changes you do not own, format only your files (see
  [rust.instructions.md](instructions/rust.instructions.md#after-editing)).

## Validate

Run the smallest set that covers the change, then widen. All commands run from the workspace root.

```sh
node tools/rust-format.cjs            # WRITES: rustfmt plus item spacing; use --check to only verify
node tools/check-structure.cjs        # file layout, 800-line limit, summaries, dependency rules
node tools/check-expects.cjs          # registered production expects
node tools/check-docs.cjs             # Markdown links, anchors and fence tags
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo test --workspace --all-features --lib --bins --tests --examples
cargo test --workspace --all-features --doc
RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links -D rustdoc::private_intra_doc_links" cargo doc --workspace --all-features --no-deps
node tools/check-features.cjs --offline   # feature matrix, when features or cfgs change
```

Ignored tests need a GPU, fonts or FFmpeg; select them by name and see
[docs/development.md](../docs/en/development.md). A zero-test filter or a successful compile is not
runtime acceptance.

## More

[Tutorial](../docs/en/tutorial/README.md) · [Command line](../docs/en/cli.md) ·
[Source style](../docs/en/style.md) ·
[Docs index](../docs/en/README.md). Documentation rules are in
[docs.instructions.md](instructions/docs.instructions.md) and shader rules in
[wgsl.instructions.md](instructions/wgsl.instructions.md).
