# Appendix: Gallery Guide

Gallery contains finished short films, not one comparison panel per API. Observe their structure, visual relationships and rhythm before returning to the relevant chapters for implementation details.

## Entry Points and Resources

```sh
cargo run -p motion-gallery --bin motion-gallery -- list
cargo run -p motion-gallery --bin motion-gallery -- still chromatic-tide --time 4 --font tests/fixtures/fonts/NotoSans-Regular.ttf --output tide.png
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo type-in-motion --font tests/fixtures/fonts/NotoSans-Regular.ttf
```

`list` initializes no fonts, formulas, audio or GPU. Other commands accept an explicit `--font`. Without it, Gallery discovers system fonts once at application startup, not implicitly within core frame callbacks.

Six films are available by default, seven with `formula`; disabled optional entries are absent from the catalog. Films default to eight seconds, a 960x540 design canvas and exact 30 fps. `--params` accepts `seconds` from 1–60, a fixed seed and `count` from 4–128.

## Choose by Learning Goal

| Film ID / Title | Key Chapters | What to Observe First |
| --- | --- | --- |
| `chromatic-tide` / Chromatic Tide | [04 Scenes](04-scenes-and-timeline.md), [05 Drawing](05-drawing.md) | Shared three-act filter stack and additive light/color |
| `type-in-motion` / Room to Move | [03 Animation](03-animation.md), [06 Content](06-content.md) | Glyphs, paragraphs, whitespace, page transitions |
| `ink-journey` / The Long Way Home | [06 Content](06-content.md), [09 Rendering](09-rendering.md) | Continuous strokes, materials, camera drift, film damage |
| `signal-garden` / Signal Garden | [07 Particles](07-particles.md), [10 Audio](10-audio.md) | Five FFT bands organizing the spectrum and HUD |
| `chidori` / Chidori | [04 Scenes](04-scenes-and-timeline.md), [05 Drawing](05-drawing.md) | Procedural bolts re-rolled at 15 Hz, additive light, bloom and a burst timed by progress |
| `quiet-hours` / Quiet Hours | [04 Scenes](04-scenes-and-timeline.md), [05 Drawing](05-drawing.md) | Four dissolved shots built from layered gradients, silhouettes, wind-swayed grass and additive light |
| `harmonics` / Harmonics | [08 Formulas](08-formula-typst.md), [03 Animation](03-animation.md) | Semantic matching and coordinated waveform changes |

## Chromatic Tide

[The source](../../gallery/src/demos/chromatic_tide.rs) divides the night into three acts: sliced sun/grid, interfering light ribbons, then rain and a seeded skyline.
The key is not merely neon. Gradients, masks, Plus, bloom, chromatic fringing and glitch slices form one visual hierarchy, with a shared filter stack retained across act transitions.

Read timeline and act organization first, shared layers and filters next, then frame seeds, grain and scanlines. To reuse the style, establish subjects, act relationships and brightness hierarchy before applying the same filter stack to every layer.

## Room to Move

[The source](../../gallery/src/demos/type_in_motion.rs) uses ID `type-in-motion`, not a slug of the display title.
The cover has spring-settled graphemes; the middle spread uses a drop cap, pull quote and justified two-column paragraphs; the closing page is full-bleed. Masks and directional wipes maintain editorial rhythm.

Study how [text and fonts](06-content.md) affect layout, how animation addresses graphemes rather than UTF-8 byte indices, and how static typography coexists with animated text.
Fix fonts before comparing pixels. A layout with changed fonts is not the same reference.

## The Long Way Home

[The source](../../gallery/src/demos/ink_journey.rs) combines pressure strokes, typewriter captions, stamp/date marks, dust, scratches, light leaks and per-frame grain on aged paper.

Study continuous-stroke reveal and pressure/pigment clocks before camera drift. Complete static strokes can reuse exact material preparation; active strokes still compute by progress. Final cannot eliminate that cost by splitting strokes into independent pieces.

Fixed seeds preserve replay, while frame-derived seeds vary film damage over time. This is an explicit artistic choice, not mutation of unreproducible global state.

## Chidori

[The source](../../gallery/src/demos/chidori.rs) charges a floating orb, detonates it at the film's midpoint and lets it burn out. Bolts are midpoint-displacement polylines drawn as bloom, body and core strokes with Plus blending; forks, ground cracks, shockwaves and debris follow the same energy curve. Burst physics run on a virtual eight second clock, so every duration keeps the same beats.

## Quiet Hours

[The source](../../gallery/src/demos/quiet_hours.rs) splits any duration into four equal shots: a sunlit forest, a stream, a dusk village over clouds and a firefly night. Shots dissolve through a short overlap and share one vignette and grain; each shot adds its own bloom. Run `--params '{"seconds":60}'` for a one-minute film.

## Signal Garden

[The source](../../gallery/src/demos/signal_garden.rs) synthesizes a PCM soundtrack. Five FFT bands drive the radial spectrum, core and meters; Plexus, radar sweeps, shutter sampling and blurred discs create depth.

Find audio preparation and analysis, then trace frame sampling of `AudioSignal`. Do not put FFT work into draw callbacks.
Studio plays against audible samples when audio is present; export still mixes timeline occurrences. Mute does not change the film's audio track, which PNG/GIF cannot carry.

## Harmonics

Enable Gallery `formula`. In [the source](../../gallery/src/demos/harmonics.rs), examine Typst sources, selections and semantic matching before the relationship between equation writing/morphing and the frequency-doubling waveform.

Formulas are native Typst vectors, not text screenshots. Source/geometry matching does not imply understanding arbitrary mathematical equivalence. The trusted-compilation boundary is the same as in [chapter 08](08-formula-typst.md).

## Interpreting Performance

Gallery now uses unchanged engine authoring limits and BrushStroke's 32-million material allowance. Final ink charges actual indexed distance evaluations; artwork and continuous strokes are not simplified. See [default budgets](../../gallery/README.md#default-budgets) for full-size sampled admission and replay scope. These limits are not real-time playback guarantees or RSS caps.

These films serve as [films benchmark](../benchmarks.md) workloads. Reduce costs while preserving full composition, full-resolution checks, exact replay and CPU/GPU parity. Silently simplifying the artwork is not an optimization.

## Common Pitfalls

- Using display titles instead of actual catalog IDs.
- Comparing visuals across machines without fixed fonts.
- Treating the optional Harmonics film as a default catalog entry.
- Removing audio, safety guidance or continuous-stroke behavior to fit a budget.
- Treating Gallery's temporary allowances as library defaults or performance promises.

## Further Reading

Return to the [tutorial index](README.md). See [Gallery README](../../gallery/README.md) for project entry points, parameters and film descriptions.
Read [Performance and Reliability](13-performance-and-reliability.md) before optimizing, and [Architecture and Extensions](14-architecture-and-extending.md) before changing interfaces.