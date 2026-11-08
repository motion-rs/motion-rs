---
description: 'Use when creating or modifying WGSL shaders. Preserve the existing code style and add accurate comments for non-obvious shader logic and contracts.'
applyTo: '**/*.wgsl'
---

# WGSL Style And Comments

- Before editing, read the current shader and nearby implementations. Follow their established naming,
  indentation, line wrapping, attribute placement, declaration layout and comment style.
- Preserve the project's manual layout, including natural matrix rows and separation of calculation phases.
  When no local convention exists, use four spaces, LF line endings and a maximum of 120 columns.
- Add concise comments for new or changed non-obvious logic. Explain intent, coordinate spaces, packed fields,
  numeric limits, color/alpha conventions and GPU write ownership where relevant; do not narrate every assignment.
- Update existing comments when the behavior they describe changes. Verify shader contracts against the nearest
  host-side bindings, uniforms, dispatch or texture setup; do not guess meanings from opaque names.
- Follow the file's existing comment language; otherwise use concise ASCII English.
- Use the project's [WGSL readability skill](../skills/wgsl-readability/SKILL.md) for the formatting and comment
  workflow. Its comments-and-whitespace-only constraint applies to readability-only work, not to logic changes
  explicitly requested by the user.
- Do not run a formatter in write mode unless requested when it would overwrite the established manual style.
  Keep unrelated code and user changes intact; formatting and comment edits must not alter shader behavior.
