# Motion Gallery

A small program of finished motion studies. Each entry is an eight-second film,
not a feature comparison panel. Every film owns its metadata, visual composition
and timeline in one source file; its module comment describes the combined features.

```sh
cargo run -p motion-gallery --bin motion-gallery -- list
cargo run -p motion-gallery --bin motion-gallery -- still type-in-motion --time 4 --font /path/to/font.ttf --output type.png
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo type-in-motion
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

Signal Garden includes a synthesized, sidechain-ducked mix; its FFT bands drive the spectrum and constellations and its bus meters drive the HUD.
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
| The Harbor Line (infographic) | A flat data-journalism explainer about a fictional harbour ferry: a scale-mapped bar chart grows year by year, an axis-labelled line chart traces falling commute times, keyframed counters tick up in a `Row` of stat cards and a timed caption track narrates underneath. Diagram charts and scales, `Keyframes`, `Row`/`Column` layout and captions. | [data_story.rs](src/demos/data_story.rs) |
| Pip Takes a Bow (character) | A paper-cutout puppet in three shots: Pip walks past parallax hills, waves from a theatre stage backed by a photographic `Image`, then jumps for joy under confetti. Inline SVG limbs on a 12-bone `Skeleton`, procedural gaits and `Keyframes<Pose>` blends, a spring-loaded hat, joined by built-in `Iris` and `Push` transitions. | [puppet_show.rs](src/demos/puppet_show.rs) |
| Solstice (branding) | A brand ident: a circle, square and triangle spring onto a construction grid and morph into the SVG sun-on-horizon mark while a trimmed orbit draws around them, then a burst and the wordmark lockup. Grid, chrome, burst and lockup are a `motion::declarative` JSON document composited with Rust-built morphs. | [logo_sting.rs](src/demos/logo_sting.rs) |
| Last Order (cinematic) | A rainy night seen from a neon diner in four acts: split-screen clip vs AgX shoulder, bloom/halation/anamorphic streaks, rack-focus DepthOfField with hexagonal bokeh, `Elevation` shadows, Atmosphere fog/dust/light leak, a CDL + ColorWheels LUT grade and finally `Look::film` via `Look::scene`. Captions name each feature as it appears; the tone map is baked into a `Lut3d` because the gallery has no display stage. | [cinematic_look.rs](src/demos/cinematic_look.rs) |
| Room to Move (magazine) | An editorial issue: a serif cover with spring-settled letters, a justified two-column spread with drop cap and pull quote, and a full-bleed closing page. Grapheme animation, paragraph layout, masks and directional wipes. | [type_in_motion.rs](src/demos/type_in_motion.rs) |
| The Long Way Home (nostalgia) | A brush-and-ink landscape on aged paper with a typewriter caption, rust stamp, orange date mark, dust, scratches, light leak and flickering grain. Pressure strokes, ink material, camera drift and per-frame seeded film damage. | [ink_journey.rs](src/demos/ink_journey.rs) |
| Signal Garden (sci-fi audio) | An instrument display wired to a live mix: synthesized kick, hats, bass and lead are routed through buses with sidechain ducking, delay/reverb, pan/width and a limiting master. FFT bands drive the radial spectrum and Plexus constellations, the measured duck gain makes the garden pump with the kick, and a motion-blurred radar sweep and bus meters read back the mix. | [signal_garden.rs](src/demos/signal_garden.rs) |
| Chidori (lightning) | A crackling orb gathers in a moonlit valley; sparks and thin arcs swell into a blinding burst, then a full-power orb throws branching bolts, cracks the ground and flings debris before fading. Midpoint-displacement bolts re-rolled at 15 Hz, plus blending, bloom, chromatic fringing and camera shake, paced by progress so any duration plays the same beats. | [chidori.rs](src/demos/chidori.rs) |
| Quiet Hours (pastoral) | A hand-painted day in four equal shots: a sunlit ancient forest with light beams and golden dust, a clear stream with sparkling water and wildflowers, a dusk village above a sea of clouds, and a firefly night under the stars. Procedural gradients and silhouettes, wind-swayed grass, screen/plus light, bloom and grain. Use `--params '{"seconds":60}'` for a one-minute film. | [quiet_hours.rs](src/demos/quiet_hours.rs) |
| Particle Garden | Four acts: fireworks with sub-emitters bouncing off polygon/segment colliders; a boid murmuration over a curl flow field with ribbon trails; text, image and path emitters dissolving into paint-coloured sprites from a padded atlas; bokeh dust and Temperature-curve embers under noise-driven Atmosphere. | [particle_garden.rs](src/demos/particle_garden.rs) |
| Harmonics (`formula`) | An equation writes itself and morphs as a waveform doubles frequency. Native Typst vectors, semantic matching and function geometry. | [harmonics.rs](src/demos/harmonics.rs) |

Ten entries are available by default, eleven with `formula`.
All use a 960x540 design canvas and exact 30 fps timelines. Parameters
accept 1..=60 seconds, a deterministic seed and a mark density of 4..=128.
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

With the bundled Noto Sans fixture, all ten default films retain their 72 marks,
seed 20261027 and eight-second timelines. First/middle/last sampled frames at
1920x1080 pass default admission and exact CPU replay in Final and Draft. This is
60 sampled cases, not every frame, custom parameter set, optional formula film or
GPU full-resolution acceptance. Native backend parity is checked separately;
do not simplify artwork or infer throughput from admission tests.

## Validation

```sh
CARGO_INCREMENTAL=0 cargo test --locked --offline -j 2 -p motion-gallery --lib
MOTION_TEST_FONT=/path/to/font.ttf CARGO_INCREMENTAL=0 cargo test --locked --offline -j 2 -p motion-gallery --lib every_demo_renders_its_subject_and_replays_exactly -- --ignored --nocapture
MOTION_TEST_FONT=/path/to/font.ttf CARGO_INCREMENTAL=0 cargo test --locked --offline -j 2 -p motion-gallery --all-features --lib films_match_cpu_gpu_and_replay -- --ignored --nocapture
```
