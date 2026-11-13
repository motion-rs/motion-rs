# Example Assets

- `landscape.jpg`: mountain lake photograph from Unsplash,
  https://images.unsplash.com/photo-1470770841072-f978cf4d019e?auto=format&fit=crop&w=1200&q=85
  (downloaded 2026-10-27). Use is subject to the [Unsplash license](https://unsplash.com/license).

- `ring.png`: generated RGBA8 ring, 256x256, with transparent pixels and a soft
  alpha edge. Created for this repository using FFmpeg's `geq` filter.

- `solstice-ident.json`: `motion::declarative` document for the Motion ident's
  M monogram, `motion` wordmark and MADE TO MOVE tagline. Created for this repository.

- `solstice-mark.svg`: Motion M monogram. Created for this repository.

- [animation-atlas.json](animation-atlas.json):
  the JSON source for all forty-three Animation Atlas studies. Shared elements and
  the seeded noise study use `defs` / `use`; `$calc` computes frame-aligned timing
  from `seconds`, while `$param` supplies the resolved values. Sampled polylines
  compile all curves, including seeded noise, from the JSON animation definitions.
  The file compiles directly with eight-second defaults. Gallery only overrides
  `seconds`, `seed` and the shared atlas `title`, then omits zero-frame pages for
  short durations. Select it with `animation-atlas` and `mode: "json"`.
  Created for this repository.

- [compositing-atlas.json](compositing-atlas.json):
  the JSON source for all twenty-six Compositing Atlas pages. Definitions reuse
  shapes, gradients and text styles; scene-clock bindings select the current page
  and animate its specimen and footer. Two mutually exclusive circles with fixed
  sibling order reproduce the native z-index switch. The file compiles directly
  with eight-second defaults. Select it with `compositing-atlas` and `mode: "json"`.
  Created for this repository.

- [filter-atlas.json](filter-atlas.json):
  seventeen Filter Atlas pages using shared spatial and optical filters, animated
  strengths and a full-width deterministic grain seed. Select it with `filter-atlas`
  and `mode: "json"`. Created for this repository.

- [vector-atlas.json](vector-atlas.json):
  thirteen Vector Atlas pages using animated points and gradients, embedded SVG,
  SVG morphing and configurable vector writing. Select it with `vector-atlas` and
  `mode: "json"`. Created for this repository.

- [particle-atlas.json](particle-atlas.json):
  thirteen Particle Atlas pages using shared seeded presets, explicit simulation
  sample times, colored vector logo sources, smoke/fire magnification and local
  Bloom. `count` scales density from the native baseline of 72. Select it with
  `particle-atlas` and `mode: "json"`. Created for this repository.

- [particle-features.json](particle-features.json):
  all forty-seven Particle Features studies as independent JSON scenes. Explicit
  emitters, analytic trajectories, ordered forces, population operators, colliders,
  root-only event rules, generated sprite materials, lifetime curves and time mapping
  reproduce the native specimens. The custom-style page uses data-driven circles,
  not a Rust callback. `seconds`, `seed` and `count` share the native defaults.
  Select it with `particle-features` and `mode: "json"`. Created for this repository.

Filter, Vector and Particle Atlases and Particle Features compile directly with
eight-second defaults. Each page is a separate frame-aligned scene; its clock retains the native timeline's global
progress. Regenerate these resources with `node tools/generate-atlases.cjs` from the
workspace root, or verify them without writing with
`node tools/generate-atlases.cjs --check`.

All files are bundled so Gallery, benchmarks and render tests work offline.
The ident files and JSON atlases are embedded in Gallery with `include_str!`.
`landscape.jpg` and `ring.png` are used only by the performance benchmarks, which
decode them once before frame evaluation, not on every frame.
