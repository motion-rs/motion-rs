---
name: wgsl-readability
description: 'Manually format WGSL shaders and improve their comments without changing behavior. Use when asked to format .wgsl files, improve shader readability, add or refine shader comments, or continue a WGSL formatting and documentation task in this project.'
argument-hint: '[file-or-directory] [format-only|comments-and-format]'
user-invocable: true
---

# WGSL Readability

Improve WGSL layout and explain non-obvious shader contracts. Change only comments and whitespace unless the user
explicitly authorizes a behavior change.

## Scope

1. Use the requested file or directory. Otherwise, start with the selected or active WGSL file.
2. If asked to process remaining files, discover current `.wgsl` files, including newly added shader fragments.
   Skip generated files, build outputs, dependencies and frozen benchmark artifacts.
3. Read the current source and inspect its existing diff before editing. Preserve user changes and unrelated files.
   Capture the current source as a temporary verification baseline; do not assume `HEAD` contains the user's input.
4. Read only the nearest host code needed to verify bindings, uniform fields, format flags, dispatch dimensions,
   texture formats or output layouts. Locate shader assembly if the file is a fragment rather than a full module.
5. Briefly describe the edits and the focused check before applying a small patch. Validate that patch before
   continuing with another file or edit group. Do not expand the task into GPU optimization or bug fixing.

## Manual Formatting

- Use four-space indentation, LF line endings and a maximum of 120 columns unless the user requests otherwise.
- Separate declarations, resource bindings and distinct calculation phases with a single blank line.
- Expand single-line `case` and `default` blocks. Use `for (` consistently.
- Indent multiline arguments one level from the call; nested calls add one level, not an extra two levels.
- Break long expressions at operators without changing parentheses, evaluation order or arithmetic precision.
- Keep recognizable matrices and lookup tables in their natural rows, such as a four-row Bayer rank matrix.
- Group attributes belonging to the same field or parameter when that improves readability.
- Preserve identifiers, literals, operators, punctuation, attributes, bindings, declaration order and control flow.
  Do not add trailing commas merely because a call was split across lines.
- Use `apply_patch` for manual edits. Do not run a formatter in write mode unless explicitly requested;
  automatic formatting may undo the intended manual layout.

## Comments

- Add or refine comments by default. If the user requests formatting only, do not add comments.
- Follow the file's established comment language. For files without a convention, use concise ASCII English.
- Explain intent and contracts rather than narrating assignments or obvious operations.
- Prefer short comments before complex blocks; annotate packed uniform fields when their meaning is verified.
- Useful topics include coordinate spaces, source stride and crop origin, premultiplication, transfer functions,
  quantization thresholds, dither, color range, byte/plane layout, shared status bits and race-free write ownership.
- Verify numeric limits, color standards, mode values and host-side alignment requirements against their owners.
  Do not infer meanings from an opaque field name or copy assumptions from a similar shader.
- Distinguish linear RGB from encoded sRGB, source alpha from rounded alpha, and UNORM from sRGB texture formats.
- Do not call a range guard a complete finite-value check if it does not also reject NaN.
- Document the existing behavior, including limitations. Do not promise exact CPU parity, performance gains,
  device compatibility or safety properties that the surrounding implementation does not establish.
- Keep comments proportional to complexity. Do not add a comment to every line, license headers or tutorial prose.

## Validation

Immediately after each substantive patch:

1. Compare the edited executable code against the captured pre-edit source, allowing only comments and whitespace.
   Use a WGSL-aware lexer/parser when available. A fallback check may remove line comments and whitespace only
   after confirming there are no strings, block comments or preprocessing constructs, and separately confirming
   token boundaries are unchanged. Nested block comments require a proper lexer. Never treat naive text stripping
   alone as proof of semantic equivalence or compare only against `HEAD` when the file was already dirty.
2. Check that all lines fit within 120 columns and that the touched diff has no whitespace errors.
3. For a standalone shader, use `wgslfmt --json --check <file>` as a read-only parse/format probe when available.
   Capture its output and print only status, error messages and the summary, not entire sources or diffs.
   Convert summary counts with `Number(...)`: this CLI can report counts as strings.
   Require zero parse/format errors; missing or unrecognized report fields are not a passing check.
   Exit code 1 and a file status of `fail` may indicate only a difference from the automatic style.
   Report that distinction explicitly; do not claim the automatic-style check passed.
4. Do not parse an incomplete shader fragment as a standalone module. Validate it through the existing shader
   assembly path or a nearby assembly/validation test. If that requires an unrequested build, explain the gap
   instead of inventing a wrapper or silently claiming the fragment passed.
5. If `wgslfmt` is unavailable, do not install or update it without authorization. Use existing diagnostics or
   an available parser, and disclose any missing syntax validation.
6. Check editor diagnostics for touched files and resolve new issues attributable to the edits.
   Do not proactively run Cargo builds, Rust/GPU tests or benchmarks unless explicitly requested.

After all requested files are handled, inspect the final diffs and report the files changed, comment coverage and
checks actually performed. Distinguish code-content preservation, syntax parsing and semantic/device validation.
Do not commit, push, change VS Code settings or rewrite benchmark evidence unless explicitly requested.

## Project Anchors

- Rendering shaders live under `crates/motion-render/src/gpu/shaders/`.
   Discover new files rather than maintaining a fixed list.
- GPU host code under `context`, `execute` and `plan` defines shader assembly, bindings and uniform layouts.
  Start from the shader's actual include/assembly call site, not a broad subsystem search.
- Existing shader-validation tests can provide stronger evidence when the user requests tests.
  Tests needing a native GPU are not a routine formatting gate.
