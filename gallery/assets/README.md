# Example Assets

- `landscape.jpg`: mountain lake photograph from Unsplash,
  https://images.unsplash.com/photo-1470770841072-f978cf4d019e?auto=format&fit=crop&w=1200&q=85
  (downloaded 2026-10-27). Use is subject to the [Unsplash license](https://unsplash.com/license).
- `ring.png`: generated RGBA8 ring, 256x256, with transparent pixels and a soft
  alpha edge. Created for this repository using FFmpeg's `geq` filter.

Both files are bundled so Gallery, benchmarks and render tests work offline.
They are decoded once before frame evaluation, not on every frame.