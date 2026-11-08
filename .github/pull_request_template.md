## Summary

<!-- What changes and why, in a few sentences. Link the issue or discussion if there is one. -->

Closes #

## Kind of change

- [ ] Bug fix
- [ ] New capability or API
- [ ] Refactor with no behavior change
- [ ] Performance
- [ ] Documentation, tooling or CI

## Impact

<!-- Delete the lines that do not apply; say "none" where nothing changes. -->

- **Public API:** <!-- added / changed / removed items, or none. Breaking changes: how callers migrate. -->
- **Output:** <!-- Do pixels, audio samples, seeded sequences or golden images change? Why is it intended? -->
- **Backends:** <!-- CPU, GPU, or both. A rendering change touches the CPU path, GPU plan, WGSL and host ABI together. -->
- **Limits and failure:** <!-- New budgets, limits or error cases. Everything is admitted through Budget before it allocates. -->

## Validation

Run from the workspace root; tick what you ran and paste anything that failed or was skipped.

- [ ] `node tools/rust-format.cjs --check`
- [ ] `node tools/check-structure.cjs` and `node tools/check-expects.cjs`
- [ ] `node tools/check-docs.cjs` (when `docs/`, a README or `.github/` changed)
- [ ] `cargo clippy --workspace --all-features --all-targets -- -D warnings`
- [ ] `cargo test --workspace --all-features --lib --bins --tests --examples`
- [ ] `cargo test --workspace --all-features --doc`
- [ ] `node tools/check-features.cjs --offline` (when features or `cfg` changed)
- [ ] Ignored tests that need a GPU, fonts or FFmpeg, run by name: <!-- which -->
- [ ] Not run, with the reason: <!-- or delete -->

## Checklist

- [ ] New and changed behavior has a focused test; no assertion was weakened to pass.
- [ ] Source follows `docs/style.md`: file layout, field order, no file over 800 lines.
- [ ] Each new production `expect` is registered in `tools/check-expects.cjs`; each new `unsafe` block has a `// SAFETY:` comment.
- [ ] Public items are documented, and the design page, tutorial or `docs/cli.md` that owns the contract is updated.
- [ ] Tutorial and reference Rust blocks still compile as Rustdoc tests.
- [ ] No benchmark figures, generated files such as `docs/benchmarks.md`, or `Cargo.lock` changed unintentionally.

## Notes for reviewers

<!-- Where to start, what is deliberately out of scope, follow-ups. -->
