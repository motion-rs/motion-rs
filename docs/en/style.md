# Source Style

Conventions for Rust sources in this workspace. Rules marked **(CI)** are
checked by `tools/check-structure.cjs` or `tools/rust-format.cjs --check` in the
Structure workflow; the rest are review conventions. See
[development checks](development.md) for the commands.

Assistant instructions in `.github/copilot-instructions.md` and `.github/instructions/` repeat these
rules for code generation; change them together.

## Formatting

- `cargo fmt` with the repository `rustfmt.toml` (edition 2024). **(CI)**
- `node tools/rust-format.cjs` adds one blank line between items. Consecutive
  `use` declarations and consecutive `mod name;` declarations stay contiguous so
  rustfmt sorts each group. **(CI)**

## File Layout

Each non-test source file follows this order:

```text
//! One contiguous module summary block.          (CI: present, contiguous)
use …;                                             (CI: header only)
mod …; / pub mod …; pub use …;                     (CI: header only)
const / static
public type A
    impl A { constructors → accessors → builders → operations }
    impl Trait for A { … }   // trait impls, including Drop, follow A
public type B …
public free functions
private types and helpers
#[cfg(test)] mod tests { … } // always last          (CI)
```

- The summary starts with an uppercase letter and describes the file's
  responsibility, not its imports. **(CI)**
- An out-of-line `#[cfg(test)] mod tests;` may sit with the other declarations.
- A helper with a single caller follows that caller.
- Implementation files stay at or under 800 lines. Test files (`tests.rs`,
  `*_tests.rs`, `tests/`) are exempt. **(CI)**

## Modules

- Use `foo.rs` + `foo/`; do not add `mod.rs` files.
- Split a file by responsibility, not by line count alone, and keep the original
  path as a re-exporting facade so public paths do not change.
- Inline tests are fine up to roughly 300 lines. Larger suites move to
  `foo/tests.rs`, declared with `#[cfg(test)] mod tests;`.
- Avoid glob imports outside tests and preludes.

## Types

- Structs list fields as: required inputs → optional configuration in
  data-flow order → derived, cached or runtime state.
- `Default` impls, constructors and builder methods list fields in declaration
  order. If reordering a struct literal would change evaluation order of
  side-effecting initializers, bind those values with `let` first.
- Only plain values without invariants (for example `Point`, `Rect`) expose
  public fields. Types with invariants keep fields private behind constructors
  and accessors. Do not add public tuple structs; use named fields.
- `#[repr(C)]` and GPU ABI structs follow the shader layout. Keep host and WGSL
  names identical and assert sizes/offsets at compile time.
- `match` arms follow the enum's declaration order where practical.

## Construction

- `new` / `try_new` take only the required inputs and apply defaults for everything else.
- `builder()` is the configuration path: optional settings are chained, and `build()` validates once
  (`Spring`, `Noise`, `Oscillator`, `BrushStroke`, `Path2D`).
- A type with only required inputs has no builder. Novice examples use `new` / `try_new`; reach for
  `builder()` only when a default must change.
- `Codec` follows the same rule through presets (`Codec::h265(18)`) refined with chained methods
  (`.ten_bit()`, `.with_preset(..)`).

## Errors And Panics

- Fallible operations return `Result<_, ValidationError>` (or the crate error).
- New APIs are fallible only. Existing panicking conveniences delegate to their
  `try_*` counterpart in one line and document the condition under `# Panics`.
- Every production `expect` is registered in `tools/check-expects.cjs` with its
  reason, and a registered message must still occur. Production code has no bare
  `unwrap()` and no `unreachable!()` without a message naming the invariant. **(CI)**
- Lock a `Mutex` or wait on a `Condvar` through `motion_core::sync` (`lock_unpoisoned`,
  `wait_unpoisoned`) so a poisoned lock continues with its plain-data value instead of
  panicking a second time; never `expect` a lock.

## Constants

Limits, protocol values and budget units used in validation get a named `const`
with a short doc comment instead of an inline literal.

## Comments

- Comments explain invariants, contracts and non-obvious reasons; they do not
  restate the code.
- Every `unsafe` block has a `// SAFETY:` comment, and every `unsafe fn` has a
  `# Safety` section (`clippy::undocumented_unsafe_blocks`,
  `clippy::missing_safety_doc`).
- Doc comments are complete English sentences starting with an uppercase
  letter; link items as ``[`Type`]``.
- Do not leave `TODO`/`FIXME`; open an issue instead.
- WGSL follows [the shader instructions](../../.github/instructions/wgsl.instructions.md).

## Lints

- Library crates forbid `unsafe_code`, except `motion-ffmpeg`, which denies
  `unsafe_op_in_unsafe_fn`.
- Workspace lints in the root `Cargo.toml` apply to every package through
  `lints.workspace = true`.
- Prefer `#[expect(lint)]` over `#[allow(lint)]` on the narrowest item so stale
  suppressions are reported.
