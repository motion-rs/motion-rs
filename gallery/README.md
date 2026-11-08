# Motion Gallery

A small program of finished motion studies. Each entry is an eight-second film,
not a feature comparison panel. Every film owns its metadata, visual composition
and timeline in one source file; its module comment describes the combined features.

```sh
cargo run -p motion-gallery --bin motion-gallery -- list
cargo run -p motion-gallery --bin motion-gallery -- still chromatic-tide --time 4 --font /path/to/font.ttf --output tide.png
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo chromatic-tide
cargo run -p motion-gallery --features ffmpeg --bin motion-gallery -- export signal-garden --params '{"seconds":8,"count":72}' --font /path/to/font.ttf --output signal.mp4
```

Listing is resource-free. Rendering accepts an explicit `--font` path; without
one, fonts are discovered once during application startup. The typography was designed
for a serif face such as Georgia: Room to Move's cover and body text fall back to the
sans-serif that discovery finds first (for example Arial Unicode) unless `--font` names one. `--size`, `--backend`,
`--range`, `--params` and overwrite/cancellation use the library's Project API.

Studio opens one demo, never the full catalog. Choose `studio --demo NAME`;
positional `studio NAME` and `--timeline NAME` also remain available. With no
selection Gallery opens the first catalog entry. Duplicate/unknown `--demo`
selections fail before font discovery or window startup. Use `list` for IDs.

Signal Garden includes a synthesized soundtrack; its five FFT bands drive the spectrum, core and HUD meters.
Studio Play uses the audible-sample clock for rational-frame-rate
synchronization; Mute disables only preview sound. Audio preparation runs outside
the rendering thread. Empty audio does not request an output device.

Use Release builds for playback. Studio defaults to 50% preview scale and a
256 MiB logical frame cache; `MOTION_STUDIO_CACHE_MIB=0..1024` adjusts that cache.
Playback is real-time only, without complete-film preparation or a per-frame
waiting mode. Slow renders skip missed frames to follow the wall/audio clock.
Playback waits for its first frame before starting the clock and prefetches about one
frame per preview slot; seek/pause stop prefetch. Completed landscape
brush strokes retain their exact geometry/material preparation across frames;
partially revealed strokes share the full-stroke material and recompute only their
per-progress outline. This does
not guarantee real-time rendering of every film. OS scheduling,
display refresh and audio devices still need physical-device acceptance.
Static titles, fixed rings, decorative paths and unchanged ribbon gradients are
retained through SceneContext caches; animation still updates original parameters,
transforms, waveforms and ribbon geometry each frame. Stroke outlines and gradient
tables can be reused without caching whole rendered frames or changing blend order.
See [Studio](../docs/studio.md).

## Film Program

| Film | Composition | Source |
| --- | --- | --- |
| Audio Brush | Asset-free drum synthesis drives Bloom while an extended pressure/bristle brush and PathEmitter share a reveal clock. Includes original single-line lettering, safe master output and random-order pixel/PCM replay. | [audio_brush.rs](src/demos/audio_brush.rs) |
| Chromatic Tide (cyberpunk) | A neon night in three acts: a sliced synthwave sun over a perspective grid, interfering light ribbons, and rain over a seeded skyline. Gradients, masks, Plus blending, bloom, chromatic fringing, glitch slices, scanlines and grain, with acts dissolved inside one shared filter stack. | [chromatic_tide.rs](src/demos/chromatic_tide.rs) |
| Last Order (cinematic) | A rainy night seen from a neon diner in four acts: split-screen clip vs AgX shoulder, bloom/halation/anamorphic streaks, rack-focus DepthOfField with hexagonal bokeh, `Elevation` shadows, Atmosphere fog/dust/light leak, a CDL + ColorWheels LUT grade and finally `Look::film` via `Look::scene`. Captions name each feature as it appears; the tone map is baked into a `Lut3d` because the gallery has no display stage. | [cinematic_look.rs](src/demos/cinematic_look.rs) |
| Room to Move (magazine) | An editorial issue: a serif cover with spring-settled letters, a justified two-column spread with drop cap and pull quote, and a full-bleed closing page. Grapheme animation, paragraph layout, masks and directional wipes. | [type_in_motion.rs](src/demos/type_in_motion.rs) |
| The Long Way Home (nostalgia) | A brush-and-ink landscape on aged paper with a typewriter caption, rust stamp, orange date mark, dust, scratches, light leak and flickering grain. Pressure strokes, ink material, camera drift and per-frame seeded film damage. | [ink_journey.rs](src/demos/ink_journey.rs) |
| Signal Garden (sci-fi) | An instrument display: a radial spectrum driven by five FFT bands, Plexus constellations, a motion-blurred radar sweep, HUD readouts and blurred depth-of-field discs. PCM synthesis, band analysis, particles, shutter sampling and filters. | [signal_garden.rs](src/demos/signal_garden.rs) |
| Sidechain Pulse (audio) | Eight seconds of asset-free synthesis: a four-on-the-floor kick sidechain-ducks the bass pad, a voice-like clip ducks the music bus, with hi-hat/metronome grid, delay/reverb lead, pan and stereo-width motion, and a limiter + loudness-normalized master. Radial equalizer, rings and a duck-gain trace are read back from the prepared mix (bus meters, spectrum, onsets, beat grid). | [sidechain_pulse.rs](src/demos/sidechain_pulse.rs) |
| Chidori (lightning) | A crackling orb gathers in a moonlit valley; sparks and thin arcs swell into a blinding burst, then a full-power orb throws branching bolts, cracks the ground and flings debris before fading. Midpoint-displacement bolts re-rolled at 15 Hz, plus blending, bloom, chromatic fringing and camera shake, paced by progress so any duration plays the same beats. | [chidori.rs](src/demos/chidori.rs) |
| Quiet Hours (pastoral) | A hand-painted day in four equal shots: a sunlit ancient forest with light beams and golden dust, a clear stream with sparkling water and wildflowers, a dusk village above a sea of clouds, and a firefly night under the stars. Procedural gradients and silhouettes, wind-swayed grass, screen/plus light, bloom and grain. Use `--params '{"seconds":60}'` for a one-minute film. | [quiet_hours.rs](src/demos/quiet_hours.rs) |
| Particle Recipes | Thirteen particle recipes in sequence, with plain and Bloom views: sparks, fire, smoke, snow, rain, confetti, fireworks, depth and colored logo motion. | [particle_recipes.rs](src/demos/particle_recipes.rs) |
| Particle Garden | Four acts: fireworks with sub-emitters bouncing off polygon/segment colliders; a boid murmuration over a curl flow field with ribbon trails; text, image and path emitters dissolving into paint-coloured sprites from a padded atlas; bokeh dust and Temperature-curve embers under noise-driven Atmosphere. | [particle_garden.rs](src/demos/particle_garden.rs) |
| Harmonics (`formula`) | An equation writes itself and morphs as a waveform doubles frequency. Native Typst vectors, semantic matching and function geometry. | [harmonics.rs](src/demos/harmonics.rs) |

Eight entries are available by default, nine with `formula`.
All use a 960x540 design canvas and exact 30 fps timelines. Parameters
accept 1..=30 seconds, a deterministic seed and a mark density of 4..=128.
`DemoMetadata.features` describes combinations; features may recur across films.
Optional entries are absent when their feature is disabled. Listing initializes
neither formulas, audio, fonts nor GPU devices.

## Particle Comparisons

The resource-free comparison generator writes 52 PNGs: every preset with Bloom
off/on and real None/AgX display transforms. It rejects overwrites.

```sh
cargo run -p motion-gallery --bin particle_comparisons -- /tmp/motion-particle-comparisons
```

## Default Budgets

Gallery uses unchanged engine authoring admission: 256 MiB of logical live bytes,
16777216 pixels and no frame-work cap. The temporary 1 GiB/20-billion frame policy
and 100-million brush override are removed. Continuous strokes retain their
geometry, pressure and load; Final ink charges actual indexed distance evaluations
against BrushStroke's unchanged 32-million default. These are not process-memory or real-time playback guarantees.

With the bundled Noto Sans fixture, all six default films retain their 72 marks,
seed 20261027 and eight-second timelines. First/middle/last sampled frames at
1920x1080 pass default admission and exact CPU replay in Final and Draft. This is
36 sampled cases, not every frame, custom parameter set, optional formula film or
GPU full-resolution acceptance. Native backend parity is checked separately;
do not simplify artwork or infer throughput from admission tests.

## Validation

```sh
CARGO_INCREMENTAL=0 cargo test --locked --offline -j 2 -p motion-gallery --lib
MOTION_TEST_FONT=/path/to/font.ttf CARGO_INCREMENTAL=0 cargo test --locked --offline -j 2 -p motion-gallery --lib every_demo_renders_its_subject_and_replays_exactly -- --ignored --nocapture
MOTION_TEST_FONT=/path/to/font.ttf CARGO_INCREMENTAL=0 cargo test --locked --offline -j 2 -p motion-gallery --all-features --lib films_match_cpu_gpu_and_replay -- --ignored --nocapture
```
