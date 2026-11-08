# Appendix: Gallery Guide

Gallery contains finished short films, not one comparison panel per API. Observe their structure, visual relationships and rhythm before returning to the relevant chapters for implementation details.

## Entry Points and Resources

```sh
cargo run -p motion-gallery --bin motion-gallery -- list
cargo run -p motion-gallery --bin motion-gallery -- still type-in-motion --time 4 --font tests/fixtures/fonts/NotoSans-Regular.ttf --output type.png
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo type-in-motion --font tests/fixtures/fonts/NotoSans-Regular.ttf
```

`list` initializes no fonts, formulas, audio or GPU. Other commands accept an explicit `--font`. Without it, Gallery discovers system fonts once at application startup, not implicitly within core frame callbacks.

Ten films are available by default, eleven with `formula`; disabled optional entries are absent from the catalog. Films default to eight seconds, a 960x540 design canvas and exact 30 fps. `--params` accepts `seconds` from 1–60, a fixed seed and `count` from 4–128.

## Choose by Learning Goal

| Film ID / Title | Key Chapters | What to Observe First |
| --- | --- | --- |
| `data-story` / The Harbor Line | [03 Animation](03-animation.md), [06 Content](06-content.md) | Diagram scales and charts, `Keyframes` counters, `Row`/`Column` cards and a caption track |
| `puppet-show` / Pip Takes a Bow | [03 Animation](03-animation.md), [04 Scenes](04-scenes-and-timeline.md) | `Skeleton` forward kinematics with SVG limbs, `Keyframes<Pose>` and built-in transitions |
| `logo-sting` / Solstice | [16 Declarative](16-declarative.md), [06 Content](06-content.md) | A JSON document composited with vector morphs, trim paths and springs |
| `cinematic-look` / Last Order | [15 Cinematic](15-cinematic.md), [09 Rendering](09-rendering.md) | Tone map, bloom/halation, depth of field, atmosphere and a LUT grade |
| `particle-garden` / Particle Garden | [07 Particles](07-particles.md) | Sub-emitters, colliders, boids, flow fields and paint-aware emitters |
| `type-in-motion` / Room to Move | [03 Animation](03-animation.md), [06 Content](06-content.md) | Glyphs, paragraphs, whitespace, page transitions |
| `ink-journey` / The Long Way Home | [06 Content](06-content.md), [09 Rendering](09-rendering.md) | Continuous strokes, materials, camera drift, film damage |
| `signal-garden` / Signal Garden | [07 Particles](07-particles.md), [10 Audio](10-audio.md) | Five FFT bands organizing the spectrum and HUD |
| `chidori` / Chidori | [04 Scenes](04-scenes-and-timeline.md), [05 Drawing](05-drawing.md) | Procedural bolts re-rolled at 15 Hz, additive light, bloom and a burst timed by progress |
| `quiet-hours` / Quiet Hours | [04 Scenes](04-scenes-and-timeline.md), [05 Drawing](05-drawing.md) | Four dissolved shots built from layered gradients, silhouettes, wind-swayed grass and additive light |
| `harmonics` / Harmonics | [08 Formulas](08-formula-typst.md), [03 Animation](03-animation.md) | Semantic matching and coordinated waveform changes |

## The Harbor Line

[The source](../../gallery/src/demos/data_story.rs) explains a fictional harbour ferry: a scale-mapped bar chart grows year by year, a line chart traces falling commute times, keyframed counters tick up in a row of stat cards and a timed caption track narrates underneath.
Study how diagram scales map data to geometry, how `Row`/`Column` measure the cards, and how one caption track shares the film clock with the charts.

## Pip Takes a Bow

[The source](../../gallery/src/demos/puppet_show.rs) rigs a paper-cutout puppet on a 12-bone `Skeleton` with inline SVG limbs. Pip walks with a procedural gait, waves through `Keyframes<Pose>` blends on a stage backed by a photographic `Image`, then jumps under confetti. Built-in `Iris` and `Push` transitions join the three shots; their overlaps are subtracted so the film keeps its exact duration.

## Solstice

[The source](../../gallery/src/demos/logo_sting.rs) is a brand ident. A `motion::declarative` JSON document ([solstice-ident.json](../../gallery/assets/solstice-ident.json)) draws the construction grid, burst and wordmark lockup; Rust springs three primitives on screen, morphs them into the [SVG mark](../../gallery/assets/solstice-mark.svg) and sweeps a trimmed orbit around it. Read it to see where declarative documents end and code-built motion begins.

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

[The source](../../gallery/src/demos/signal_garden.rs) synthesizes kick, hats, bass and lead, routes them through audio buses with sidechain ducking, delay/reverb, pan/width and a limiting master, then reads the mix back. FFT bands drive the radial spectrum and Plexus constellations, the measured duck gain makes the garden pump with the kick, and bus meters drive the HUD; radar sweeps, shutter sampling and blurred discs create depth.

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