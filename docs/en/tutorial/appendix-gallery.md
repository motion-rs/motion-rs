# Appendix: Gallery Guide

Gallery combines finished short films with animation, rendering, content, particle and audio reference studies. Observe their structure, visual relationships and rhythm before returning to the relevant chapters for implementation details.

## Entry Points and Resources

```sh
cargo run -p motion-gallery --bin motion-gallery -- list
cargo run -p motion-gallery --bin motion-gallery -- still type-in-motion --time 4 --font tests/fixtures/fonts/NotoSans-Regular.ttf --output type.png
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo type-in-motion --font tests/fixtures/fonts/NotoSans-Regular.ttf
```

`list` initializes no fonts, formulas, audio or GPU. Other commands accept an explicit `--font`. Without it, Gallery discovers system fonts once at application startup, not implicitly within core frame callbacks.

Eighteen films are available by default, nineteen with `formula`; disabled optional entries are absent from the catalog. Films default to eight seconds, a 960x540 design canvas and exact 30 fps. `--params` accepts `seconds` from 1–60, a fixed seed and `count` from 4–128.

Capability atlases, including Particle Features and Ink Features, precede narrative
films in the catalog and CLI listing. Studio without a demo selection opens
Animation (`animation-atlas`). The table below follows the same catalog order.

## Choose by Learning Goal

| Film ID / Title | Key Chapters | What to Observe First |
| --- | --- | --- |
| `animation-atlas` / Animation | [03 Animation](03-animation.md) | Forty-three isolated studies with sampled curves, live markers, easing, spring damping, oscillators, path motion and time composition |
| `brush-atlas` / Brush Atlas | [06 Content](06-content.md), [09 Rendering](09-rendering.md) | Ten pages with one brush preset each, shared geometry, preset pressure and nib settings, ink materials, Multiply compositing and Bloom |
| `particle-atlas` / Particle Atlas | [07 Particles](07-particles.md) | One page per preset, with descriptions, fixed-step simulations, trails, procedural sprites, collision splashes, fireworks and colored vector assembly |
| `particle-features` / Particle Features | [07 Particles](07-particles.md) | Forty-seven capability studies covering emitters, trajectories, forces, groups, collisions, events, styles, materials, time control and worker parity |
| `filter-atlas` / Filter Atlas | [09 Rendering](09-rendering.md) | Seventeen comparisons of filters, optical effects, HDR highlights and processing order |
| `compositing-atlas` / Compositing Atlas | [05 Drawing](05-drawing.md), [09 Rendering](09-rendering.md) | Twenty-six studies of blend modes, masks, clips, opacity, sibling order and isolation |
| `vector-atlas` / Vector Atlas | [05 Drawing](05-drawing.md), [06 Content](06-content.md) | Thirteen studies of paths, strokes, winding rules, gradients, SVG, morphing and writing |
| `ink-features` / Ink Features | [06 Content](06-content.md), [09 Rendering](09-rendering.md) | Twenty-one studies of pressure, nibs, materials, recorded input, timed lifts, erasing and relighting |
| `noise-atlas` / Noise Atlas | [02 Foundations](02-foundations.md) | Twelve scalar and vector fields, including fractals, domain warp, analytic gradients and curl |
| `audio-atlas` / Audio Atlas | [10 Audio](10-audio.md) | Twenty-one audible studies of synthesis, envelopes, DSP, spatial mixing and limiting |
| `typography-atlas` / Typography Atlas | [06 Content](06-content.md) | Eighteen Chinese and English studies of rich text, paragraph layout, glyph styling, reveal and per-part animation |
| `caption-atlas` / Caption Atlas | [06 Content](06-content.md) | Twelve Chinese and English studies of subtitle layout, timing, SRT interchange and shared cue markers |
| `particle-garden` / Particle Garden | [07 Particles](07-particles.md) | Sub-emitters, colliders, boids, flow fields and paint-aware emitters |
| `data-story` / The Harbor Line | [03 Animation](03-animation.md), [06 Content](06-content.md) | Diagram scales and charts, `Keyframes` counters, `Row`/`Column` cards and a caption track |
| `logo-sting` / Motion | [16 Declarative](16-declarative.md), [06 Content](06-content.md) | A Motion M monogram and lowercase wordmark, with a JSON document composited with vector morphs, trim paths and springs |
| `type-in-motion` / Room to Move | [03 Animation](03-animation.md), [06 Content](06-content.md) | Glyphs, paragraphs, whitespace, page transitions |
| `ink-journey` / The Long Way Home | [06 Content](06-content.md), [09 Rendering](09-rendering.md) | Continuous strokes, materials, camera drift, film damage |
| `signal-garden` / Signal Garden | [07 Particles](07-particles.md), [10 Audio](10-audio.md) | Five FFT bands organizing the spectrum and HUD |
| `harmonics` / Harmonics | [08 Formulas](08-formula-typst.md), [03 Animation](03-animation.md) | Semantic matching and coordinated waveform changes |

## The Harbor Line

[The source](../../../gallery/src/demos/data_story.rs) explains a fictional harbour ferry: a scale-mapped bar chart grows year by year, a line chart traces falling commute times, keyframed counters tick up in a row of stat cards and a timed caption track narrates underneath.
Study how diagram scales map data to geometry, how `Row`/`Column` measure the cards, and how one caption track shares the film clock with the charts.

## Motion

[The source](../../../gallery/src/demos/logo_sting.rs) is a Motion brand ident. A `motion::declarative` [JSON document](../../../gallery/assets/solstice-ident.json) draws the construction grid, burst, lowercase `motion` wordmark and MADE TO MOVE tagline; Rust springs three primitives on screen, morphs them into the three-stroke M [SVG mark](../../../gallery/assets/solstice-mark.svg) and sweeps a trimmed orbit around it. The original asset filenames remain unchanged. Read it to see where declarative documents end and code-built motion begins.

## Animation

[The source](../../../gallery/src/demos/animation_atlas.rs) presents 43 studies in the
shared atlas layout: a title and explanation on the left, with a clipped live
specimen on the right. All 24 easing variants have their own page, followed by
three spring damping regimes, four oscillator waveforms, seeded noise, a path
follower, staggered color/transforms and nine time-composition examples.

Curves and numeric values sample actual animation objects. Their independent
3.2-second local clocks retain overshoot, signed signals, delays and keyframe
holds regardless of the total film duration. Run
`studio --demo animation-atlas --params '{"seconds":60}'`, then pause or seek to
compare individual pages. The default eight-second duration is a quick overview.

## Brush Atlas

[The source](../../../gallery/src/demos/brush_atlas.rs) compares all ten `BrushPreset` variants across ten pages, with one explanation and one clipped specimen per page. Run `studio --demo brush-atlas --params '{"seconds":40}'` for four seconds per page. Shared path geometry and nominal width make the built-in pressure, nib, grain, transparency and glow differences visible without overriding their artistic settings. Light paper supports the dark and translucent presets; Neon retains a dark field and local Bloom.

Each swatch retains its full material preparation and animates a geometric clip rather than rebuilding the material at each reveal step. The demo explicitly admits up to 64 million material distance evaluations per stroke; the engine's default remains 32 million.

## Particle Atlas

[The source](../../../gallery/src/demos/particle_atlas.rs) presents all thirteen particle presets in `Preset::ALL` order. Each page places its name and a short description on the left and a clipped simulation viewport on the right. Run `studio --demo particle-atlas --params '{"seconds":52}'` for four seconds per page.

Simulations retain their explicit seed and prepared state across frames. Page progress selects a bounded local sample time, keeping short-lived Sparks and Fireworks visible even when the total film duration changes, including secondary firework bursts. Logo recipes share the existing colored vector source; Confetti uses a light background, while Smoke and Fire use closer framing. HDR recipes receive Bloom inside the viewport. No external images or raised budgets are required.

## Particle Features

[The source](../../../gallery/src/demos/particle_features.rs) presents 47 capability studies rather than preset recipes. Each page keeps an explanation on the left and a clipped specimen on the right. Run `studio --demo particle-features --params '{"seconds":60,"count":96}'` for an extended tour; pause or seek in Studio to inspect individual pages.

Emission studies cover point, disk, rectangle, burst, path, vector surface, image alpha/luminance and text sources. Analytic ballistic, convergence, orbit and noise trajectories are separate from fixed-step force, boid, spring and contact studies. Circle, polygon, segment and SDF contacts exercise real collision responses and secondary events. Appearance studies compare geometric shapes, historical and ribbon trails, Plexus, bokeh, blend modes, procedural sprite materials, frame selection, smoke evolution, speed stretch, flip, lifetime curves, color fields, temperature and a custom style callback.

The final pages show prewarm, looping, slow and reverse time mapping, and matching one/four-worker simulations. Explicit seeds and local source times make seek/replay independent of viewing order. Text uses the font library supplied at startup; image and sprite specimens use generated assets. Each simulation explicitly allows 400 million work units, while engine defaults remain unchanged. These representative studies do not certify every parameter combination or backend.

## Rendering, content and audio atlases

The ten capability atlases above share an explanatory layout, generic sans-serif
type and a clipped live specimen. Run a sixty-second tour and replace the demo ID
to switch studies:

```sh
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo filter-atlas --params '{"seconds":60}'
```

[Filter Atlas](../../../gallery/src/demos/filter_atlas.rs) uses the same source artwork
on both sides, including a real HDR highlight, and compares processing order on
its last page. [Compositing Atlas](../../../gallery/src/demos/compositing_atlas.rs)
uses shared overlapping shapes for all seventeen public blend modes. Its later
pages distinguish mask channels and inversion, clip modes, per-child versus group
opacity, sibling ordering and per-child versus isolated-group filtering.

[Vector Atlas](../../../gallery/src/demos/vector_atlas.rs) exercises actual trim,
dash and stroke geometry, three gradient types, extension modes and interpolation
spaces. Its final pages retain native SVG vectors for morphing and writing.
[Ink Features](../../../gallery/src/demos/ink_features.rs) separates brush capabilities
from the preset recipes in Brush Atlas. Recorded samples retain pressure, tilt,
timestamps and a stationary pause; timed writing has an actual pen-up interval.
Pigment mixing compares linear light with an approximate six-band model rather
than claiming physical spectral accuracy. Each stroke explicitly allows 128 million
material evaluations; the five-bristle page uses 96 path sample intervals.

[Noise Atlas](../../../gallery/src/demos/noise_atlas.rs) generates scalar textures
from explicit seeds and sample coordinates. Fractal pages explicitly use five
octaves. Its gradient and curl arrows use analytic derivatives, not a finite
difference estimate or an independently authored flow animation.

[Audio Atlas](../../../gallery/src/demos/audio_atlas.rs) contains real 48 kHz stereo
audio, not synthetic display-only waves. Dry mono and processed left/right lanes
read actual PCM, with full-page peak envelopes and a shared displayed visual gain.
The processed mix is audible in Studio and video export; dry audio is a visual
reference, not a second simultaneous output. Short transients and effect tails can
be silent at the selected instant even when their full-page envelope is visible.
Generated artwork and synthesis require no external image or audio files. Fonts
still arrive explicitly through the application font library.

## Typography and caption atlases

[Typography Atlas](../../../gallery/src/demos/typography_atlas.rs) limits its language
examples to Chinese and English. Its eighteen pages compare rich spans, weights,
sizes, wrapping, line spacing and all four paragraph alignments, then demonstrate
glyph outlines, glyph-bounds gradients, grapheme/word/wrapped-line visibility,
per-part transforms, vector handwriting and stable partial layouts. Reveal counts
come from the shaped paragraph parts, not byte offsets. Run
`studio --demo typography-atlas --params '{"seconds":60}'` for the extended tour.

[Caption Atlas](../../../gallery/src/demos/caption_atlas.rs) has twelve studies using
real `CaptionTrack`, `CaptionStyle` and `CueSheet` behavior. Read the safe rectangle
and local time rail alongside the subtitle: wrapping, padding and simultaneous
cues are measured before cached vectors are sampled. Exact half-open boundaries
prevent a duplicate cue at a cut. SRT round trips use millisecond timestamps;
shared markers drive motion on the same clock, and the final page compares an
original four-second sheet with its six-second retiming at the same local time.
Run `studio --demo caption-atlas --params '{"seconds":48}'` for four seconds per
page. This demo contains no narration or speech synthesis.

Both demos use the system font library loaded once at startup and the generic
default family, without bundled fonts or hardcoded family names. Omit `--font`
for the intended default behavior. Chinese glyphs require an installed supporting
font; installed-face fallback is supplied by the existing text engine. Font
discovery does not happen inside frame callbacks. Pixel results depend on the
machine's installed font set, while same-environment seek and replay are exact.

## Room to Move

[The source](../../../gallery/src/demos/type_in_motion.rs) uses ID `type-in-motion`, not a slug of the display title.
The cover has spring-settled graphemes; the middle spread uses a drop cap, pull quote and start-aligned two-column paragraphs; the closing page is full-bleed. Masks and directional wipes maintain editorial rhythm.

Study how [text and fonts](06-content.md) affect layout, how animation addresses graphemes rather than UTF-8 byte indices, and how static typography coexists with animated text.
Fix fonts before comparing pixels. A layout with changed fonts is not the same reference.

## The Long Way Home

[The source](../../../gallery/src/demos/ink_journey.rs) combines pressure strokes, typewriter captions, stamp/date marks, dust, scratches, light leaks and per-frame grain on aged paper.

Study continuous-stroke reveal and pressure/pigment clocks before camera drift. Complete static strokes can reuse exact material preparation; active strokes still compute by progress. Final cannot eliminate that cost by splitting strokes into independent pieces.

Fixed seeds preserve replay, while frame-derived seeds vary film damage over time. This is an explicit artistic choice, not mutation of unreproducible global state.

## Signal Garden

[The source](../../../gallery/src/demos/signal_garden.rs) synthesizes kick, hats, bass and lead, routes them through audio buses with sidechain ducking, delay/reverb, pan/width and a limiting master, then reads the mix back. FFT bands drive the radial spectrum and Plexus constellations, the measured duck gain makes the garden pump with the kick, and bus meters drive the HUD; radar sweeps, shutter sampling and blurred discs create depth.

Find audio preparation and analysis, then trace frame sampling of `AudioSignal`. Do not put FFT work into draw callbacks.
Studio plays against audible samples when audio is present; export still mixes timeline occurrences. Mute does not change the film's audio track, which PNG/GIF cannot carry.

## Harmonics

Enable Gallery `formula`. In [the source](../../../gallery/src/demos/harmonics.rs), examine Typst sources, selections and semantic matching before the relationship between equation writing/morphing and the frequency-doubling waveform.

Formulas are native Typst vectors, not text screenshots. Source/geometry matching does not imply understanding arbitrary mathematical equivalence. The trusted-compilation boundary is the same as in [chapter 08](08-formula-typst.md).

## Interpreting Performance

Gallery uses unchanged engine authoring limits and BrushStroke's 32-million default material allowance, with explicit per-stroke allowances of 64 million in Brush Atlas and 128 million in Ink Features. Particle Features explicitly raises each specimen's simulation work allowance from 100 million to 400 million; particle and byte limits remain unchanged. Final ink charges actual indexed distance evaluations; artwork and continuous strokes are not simplified. See [default budgets](../../../gallery/README.md#default-budgets) for full-size sampled admission and replay scope. These limits are not real-time playback guarantees or RSS caps.

These films serve as [films benchmark](../benchmarks.md) workloads. Reduce costs while preserving full composition, full-resolution checks, exact replay and CPU/GPU parity. Silently simplifying the artwork is not an optimization.

## Common Pitfalls

- Using display titles instead of actual catalog IDs.
- Comparing visuals across machines without fixed fonts.
- Treating the optional Harmonics film as a default catalog entry.
- Removing audio, safety guidance or continuous-stroke behavior to fit a budget.
- Treating Gallery's temporary allowances as library defaults or performance promises.

## Further Reading

Return to the [tutorial index](README.md). See [Gallery README](../../../gallery/README.md) for project entry points, parameters and film descriptions.
Read [Performance and Reliability](13-performance-and-reliability.md) before optimizing, and [Architecture and Extensions](14-architecture-and-extending.md) before changing interfaces.