# Example Assets

- `landscape.jpg`: mountain lake photograph from Unsplash,
  https://images.unsplash.com/photo-1470770841072-f978cf4d019e?auto=format&fit=crop&w=1200&q=85
  (downloaded 2026-10-27). Use is subject to the [Unsplash license](https://unsplash.com/license).
- `ring.png`: generated RGBA8 ring, 256x256, with transparent pixels and a soft
  alpha edge. Created for this repository using FFmpeg's `geq` filter.

- `solstice-ident.json`: `motion::declarative` document for the Motion ident's
  M monogram, `motion` wordmark and MADE TO MOVE tagline. Created for this repository.
- `solstice-mark.svg`: Motion M monogram. Created for this repository.

All files are bundled so Gallery, benchmarks and render tests work offline.
The ident files are embedded in Gallery with `include_str!`. `landscape.jpg` and
`ring.png` are used only by the performance benchmarks, which decode them once
before frame evaluation, not on every frame.