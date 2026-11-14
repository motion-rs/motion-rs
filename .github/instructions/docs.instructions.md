---
description: 'Use when creating or modifying project documentation (docs/, README files). Covers language, accuracy, compiled Rust blocks, terminology and generated files.'
applyTo: 'docs/**/*.md,README.md,gallery/README.md'
---

# Documentation Conventions

Index: [docs/en/README.md](../../docs/en/README.md). Terms: the glossary in
[docs/design/README.md](../../docs/en/design/README.md#glossary). Use them consistently: Timeline,
Scene, Node, Project, Composition, still, film, Studio (capitalized), CPU backend / GPU backend (not
"renderer").

## Accuracy

- Verify every concrete claim against the source before writing it: type, function and file names,
  limits and defaults, feature flags, CLI flags, error behavior. If you cannot verify it, leave it out.
- Describe current behavior in the present tense. Proposals and future work must be clearly
  labeled as planned; never present them as available APIs.
- Design pages (`docs/design/`) state contracts and decisions: ownership, invariants, limits, failure
  and determinism. They do not list signatures; Rustdoc does. Update the owning page when a contract
  changes, in the same change.
- Reference files with relative links and check that the target exists; use backticks for identifiers.

## Language And Form

- English, complete sentences, headings in sentence case, no marketing tone. Do not mix languages
  within a page.
- Tag every fence: `rust,no_run`, `rust`, `sh`, `toml`, `text`. An untagged fence is compiled as Rust.
- Do not create new plan, notes or status documents.

## Rust Blocks Are Compiled

Rust blocks in `docs/tutorial/*.md`, `docs/cli.md`, `docs/design/rendering.md` and
`docs/design/resources.md` are included as Rustdoc tests of the facade crate
(`crates/motion/src/lib.rs`, module `tutorial`, behind `#[cfg(doctest)]`), each chapter gated by the
Cargo features it needs.

- Write blocks that compile against the current API with `use motion::prelude::*;` and explicit
  extra imports only for items the prelude lacks. Do not copy unused or duplicate imports.
- Use `rust,no_run` for anything touching files, GPUs, FFmpeg or windows; a plain `rust` block runs.
- A block that uses `?` outside `fn main` ends with `# Ok::<(), motion::Error>(())`.
- A new tutorial or reference page with Rust blocks must be added to the `tutorial` module with the
  right `#[cfg(feature = "…")]`. Check all three configurations:

```sh
cargo test -p motion-rs --no-default-features --doc tutorial
cargo test -p motion-rs --doc tutorial
cargo test -p motion-rs --all-features --doc tutorial
```

## Generated And Linked Files

- `node tools/check-docs.cjs` checks relative links, heading anchors and untagged fences in
  `docs/`, the READMEs and `.github/`; CI runs it. Run it after editing or moving any page.

- `docs/benchmarks.md` is handwritten; do not run benchmarks as a side effect of a docs change.
- When renaming or moving a page, update every inbound link and the indexes in `docs/en/README.md`
  (and `docs/zh-CN/README.md`), `docs/en/design/README.md` and `docs/en/tutorial/README.md`.
- `docs/quality-report.md` and benchmark figures are dated records; change them only by re-measuring
  and updating the provenance note.
