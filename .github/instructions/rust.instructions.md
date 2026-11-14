---
description: 'Use when creating or modifying Rust source in this workspace. Enforces file layout, field order, visibility, error, constant, comment and test conventions checked by tools/check-structure.cjs.'
applyTo: '**/*.rs'
---

# Rust Conventions

Source of truth: [docs/style.md](../../docs/en/style.md). Before editing, read the file and its
neighbors and match them. Move code rather than rewriting it when reorganizing.

## File Layout

Every non-test source file is ordered like this (the structure check enforces the starred items):

1. `//!` module summary on line 1, one contiguous block, starts uppercase, says what the file is
   responsible for. *
2. `use` declarations, then `mod` / `pub mod` / `pub use` declarations, all in the header. Consecutive
   `use` lines stay contiguous and consecutive `mod name;` lines stay contiguous (rustfmt sorts them). *
3. `const` / `static`, each with a doc comment when it is a limit, protocol value or budget unit.
4. Public types. Each type is followed by its inherent `impl` (constructors, accessors, builders,
   operations) and then its trait impls, `Drop` included.
5. Public free functions, then private types and helpers. A helper with one caller sits after it.
6. `#[cfg(test)] mod tests { … }` last in the file. *

A new file needs a summary and a place in its parent module. Use `foo.rs` + `foo/bar.rs`, never
`mod.rs`. Implementation files stay at or under 800 lines (tests exempt) *; split by responsibility and
re-export from the original path so public paths do not move. Inline tests stay inline until about 300
lines, then move to `foo/tests.rs` declared with `#[cfg(test)] mod tests;` in the header.

## Types

- Field order: required inputs → optional configuration in data-flow order → derived, cached or runtime
  state. `Default`, `new` and struct literals list fields in declaration order; bind side-effecting or
  moving initializers with `let` first instead of reordering the literal.
- Only plain values without invariants (`Point`, `Rect`) expose public fields. Types with invariants use
  private fields with `new` / `try_new`, accessors and builders. Never add a public tuple struct; use
  named fields and a `new` constructor.
- `#[repr(C)]`, bytemuck and GPU ABI structs follow the WGSL layout; do not reorder them for style.
- `match` arms follow variant declaration order. Names are complete words; keep established acronyms
  (GPU, SVG, RGBA, YUV). Avoid vague private names such as `check`, `selected`, `Settings`.
- Avoid glob imports outside tests and preludes. Prefer `pub(super)` / `pub(crate)` for items that only
  cross file boundaries after a split; do not widen the public API by accident.

## Errors, Limits And Constants

- Return `Result<_, ValidationError>`; add no panicking convenience next to a fallible API. An existing
  convenience delegates to its `try_*` form in one line and documents `# Panics`.
- A production `expect` must be an invariant, keeps its exact message when code moves, and has an entry
  in `tools/check-expects.cjs` (file path suffix + message + reason). Prefer returning an error.
  Do not add `unwrap()` or message-less `unreachable!()` to production code. Lock mutexes and wait on
  condvars with `motion_core::sync::{MutexExt, CondvarExt}`; never `expect` a lock.
- Validate before allocating: `Budget::bytes`, `Budget::work`, `release` and `reset_peak` /
  `merge_peak` for nested measurement. Do not poke budget fields.
- No magic numbers in validation, limits or budget code: name the constant, document its unit, keep the
  value identical when extracting.

## Comments And Docs

- Public items have doc comments (`missing_docs` is on): complete English sentences starting uppercase,
  intra-doc links as ``[`Type`]``. Document units, defaults, errors, ownership and panics.
- Comment invariants, contracts and non-obvious reasons only; never restate the code. Update a comment
  when the behavior it describes changes. No `TODO` / `FIXME`.
- Every `unsafe` block has a `// SAFETY:` comment naming the concrete reason it is sound (pointer
  validity, ownership, lengths, threading). Every `unsafe fn` has a `# Safety` section.
- Prefer `#[expect(lint)]` on the narrowest item to `#[allow(lint)]`.
- Rust examples in doc comments are compiled: mark them `no_run` when they touch files, GPUs or windows,
  and tag non-Rust fences `text` or `sh`.

## Tests

- Every behavior change gets a focused assertion. Do not delete or weaken an assertion to pass a gate.
- Tests that need a GPU adapter, fonts or FFmpeg are `#[ignore]`d and run explicitly by name.
- Visual or numeric claims need an explicit tolerance and a CPU/GPU or same-backend replay check where
  the code path has both. Seeds and times are explicit; no wall-clock dependence.
- Cross-crate tests live in the facade's `tests/`. Moving a test keeps its function name.

## After Editing

```sh
node tools/rust-format.cjs && node tools/check-structure.cjs && node tools/check-expects.cjs
cargo clippy -p <crate> --all-features --all-targets -- -D warnings
cargo test -p <crate> --all-features
```

`rust-format.cjs` rewrites the whole workspace and `cargo fmt` adds no item spacing. On a tree with
uncommitted changes you do not own, format only your files with
`rustfmt --edition 2024 --config-path rustfmt.toml --config skip_children=true <files>`, separate items
with one blank line by hand (consecutive `use` and consecutive `mod x;` lines excepted), and verify with
`node tools/rust-format.cjs --check`. Keep unrelated user changes intact.
