# Motion Gallery

A program of motion studies, with narrative films and animation, rendering, content and audio references.
Each entry defaults to eight seconds. Every film owns its metadata, visual composition
and timeline in its source module; its module comment describes the combined features.

```sh
cargo run -p motion-gallery --bin motion-gallery -- list
cargo run -p motion-gallery --bin motion-gallery -- still type-in-motion --time 4 --font /path/to/font.ttf --output type.png
cargo run -p motion-gallery --bin motion-gallery -- still animation-atlas --time 0.6 --size 1920x1080 --output atlas.png
cargo run -p motion-gallery --bin motion-gallery -- still animation-atlas --params '{"mode":"json"}' --time 0.6 --size 1920x1080 --output json-atlas.png
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo type-in-motion
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo animation-atlas --params '{"seconds":60}'
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo animation-atlas --params '{"seconds":60,"mode":"json"}'
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo brush-atlas --params '{"seconds":40}'
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo particle-atlas --params '{"seconds":52}'
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo particle-atlas --params '{"seconds":52,"mode":"json"}'
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo particle-features --params '{"seconds":60,"count":96}'
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo particle-features --params '{"seconds":60,"count":96,"mode":"json"}'
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo filter-atlas --params '{"seconds":60}'
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo compositing-atlas --params '{"seconds":60,"mode":"json"}'
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo audio-atlas --params '{"seconds":60}'
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo typography-atlas --params '{"seconds":60}'
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo caption-atlas --params '{"seconds":48}'
cargo run -p motion-gallery --features ffmpeg --bin motion-gallery -- export signal-garden --params '{"seconds":8,"count":72}' --font /path/to/font.ttf --output signal.mp4
```

Listing is resource-free. Rendering accepts an explicit `--font` path; without
one, fonts are discovered once during application startup. The typography was designed
for a serif face such as Georgia: Room to Move's cover and body text fall back to the
sans-serif that discovery finds first (for example Arial Unicode) unless `--font` names one. `--size`, `--backend`,
`--range`, `--params` and overwrite/cancellation use the library's Project API.

All demos default to 1920x1080 output for stills, video exports and Studio at 100%
preview render scale. The internal design canvas remains 960x540 to preserve
layout and animation coordinates; rendering rasterizes directly at Full HD rather
than enlarging a finished 960x540 image. Pass an explicit `--size`, such as
`--size 1280x720`, to override the output resolution.

Studio opens one demo, never the full catalog. Choose `studio --demo NAME`;
positional `studio NAME` and `--timeline NAME` also remain available. With no
selection Gallery opens Animation (`animation-atlas`), the first catalog entry. Duplicate/unknown `--demo`
selections fail before font discovery or window startup. Use `list` for IDs.

Signal Garden includes a synthesized, sidechain-ducked mix; its FFT bands drive the spectrum and constellations and its bus meters drive the HUD.
Studio Play uses the audible-sample clock for rational-frame-rate
synchronization; Mute disables only preview sound. Audio preparation runs outside
the rendering thread. Empty audio does not request an output device.

Use Release builds for playback and select 100% preview render scale when checking
fine detail. Viewer zoom changes the displayed size, not the rendered pixel count.
The preview frame cache defaults to 256 MiB of logical payload;
`MOTION_STUDIO_CACHE_MIB=0..1024` adjusts that cache.
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
See [Studio](../docs/en/studio.md).

## Film Program

Capability atlases, including Particle Features and Ink Features, appear first
in catalog and CLI listing order, followed by narrative films and optional Harmonics.

| Film | Composition | Source |
| --- | --- | --- |
| Animation (animation reference) | Forty-three pages isolate all 24 `Easing` variants, three spring damping regimes, four oscillator waveforms, seeded noise, `FollowPath`, staggered color/transforms and nine time-composition studies. One `animation-atlas` entry shares metadata across two modes: `default` uses native Rust signals, and `json` loads bundled JSON with `defs` / `use`, `$param` and `$calc`. JSON mode compiles sampled curves and aligns scene boundaries to the 30fps grid, omitting pages without an output frame. Use `--params '{"seconds":60,"mode":"json"}'`, then pause or seek to compare pages. | [native](src/demos/animation_atlas.rs), [JSON](assets/animation-atlas.json), [loader](src/demos/json_animation_atlas.rs) |
| Brush Atlas (brush reference) | Ten pages show every `BrushPreset`, one per page, using the same path and nominal nib width. Explanations sit beside a clipped specimen. Each retains its preset pressure, nib and material settings, including Multiply compositing and Neon Bloom. A geometric clip reveals retained strokes. Use `brush-atlas` with `--params '{"seconds":40}'` for four seconds per page. | [brush_atlas.rs](src/demos/brush_atlas.rs) |
| Particle Atlas (particle reference) | Thirteen pages show every particle `Preset`, with a description on the left and a live simulation on the right. Seeded physics, trails, bokeh, collision splashes, procedural sprites, fireworks and colored vector dispersal/convergence use retained simulations and independent local clocks. HDR recipes receive local Bloom. The same entry accepts `mode: "json"`, sharing the core presets and `seed` / `count` inputs. Use `--params '{"seconds":52,"mode":"json"}'` for four seconds per page. | [native](src/demos/particle_atlas.rs), [JSON](assets/particle-atlas.json), [loader](src/demos/json_atlas.rs) |
| Particle Features (capability reference) | Forty-seven pages pair left-hand descriptions with isolated live specimens: emitters, analytic trajectories, forces, groups, colliders, lifecycle events, styles, sprite sheets, appearance curves and time control. The final page compares one and four workers. Both authoring modes share `seconds`, `seed` and `count`; JSON describes each specimen independently, including a data-only equivalent of the custom circle callback. Use `particle-features` with `--params '{"seconds":60,"count":96,"mode":"json"}'` for the JSON tour. | [native](src/demos/particle_features.rs), [JSON](assets/particle-features.json), [loader](src/demos/json_atlas.rs) |
| Filter Atlas (rendering reference) | Seventeen pages compare unfiltered and processed specimens: blur, shadow, glow, color adjustment, Bloom, halation, anamorphic streaks, distortion, dispersion, vignette, bokeh, radial blur, flare, relighting, grain, directional blur and filter order. The same entry accepts `mode: "json"`. | [native](src/demos/filter_atlas.rs), [JSON](assets/filter-atlas.json), [loader](src/demos/json_atlas.rs) |
| Compositing Atlas (rendering reference) | Twenty-six pages cover all seventeen public blend modes, alpha/luminance and inverted masks, coverage/geometric clips, group opacity, sibling order and filter isolation. The same catalog entry accepts `mode: "json"` for a bundled document using `defs` / `use`, parameters and scene-clock bindings. | [native](src/demos/compositing_atlas.rs), [JSON](assets/compositing-atlas.json), [loader](src/demos/json_atlas.rs) |
| Vector Atlas (content reference) | Sixteen native pages show arc-length trim, dashes, caps, joins, winding rules, linear/radial/sweep gradients, extension and interpolation, native SVG, morphing, vector writing, path booleans, mesh gradients and planar perspective. The same entry accepts `mode: "json"` for thirteen declarative pages covering trim through vector writing. | [native](src/demos/vector_atlas.rs), [JSON](assets/vector-atlas.json), [loader](src/demos/json_atlas.rs) |
| Ink Features (content reference) | Twenty-one pages isolate pressure, nibs, five bristles, caps, gradient paint, depletion, dry fibers, grain, paper, intersection build-up, rim darkening, wobble, boil, wetness, recorded input, pen lifts, erasing, pigment mixing and height-based relighting. The same entry accepts `mode: "json"` for all 21 pages. | [native](src/demos/ink_features.rs), [JSON](assets/ink-features.json), [loader](src/demos/json_atlas.rs) |
| Noise Atlas (procedural reference) | Twelve pages sample value, Perlin, Simplex, 3D Perlin slices, Worley distances and edges, five-octave fBm, turbulence, ridged noise, domain warp, analytic gradients and curl fields. | [noise_atlas.rs](src/demos/noise_atlas.rs) |
| Audio Atlas (audio reference) | Twenty-one audible pages cover oscillators, seeded noise, ADSR, chirp, drums, EQ, dynamics, delay, reverb, ducking, pan, stereo width, a master limiter and eleven sound presets. Dry and processed PCM drive the waveforms. | [audio_atlas.rs](src/demos/audio_atlas.rs) |
| Typography Atlas (text reference) | Eighteen Chinese and English studies show rich spans, weights, hierarchy, wrapping, line height, four alignments, glyph outlines, gradients, grapheme/word/line reveal, per-part motion, vector handwriting and stable partial visibility. | [typography_atlas.rs](src/demos/typography_atlas.rs) |
| Caption Atlas (subtitle reference) | Twelve Chinese and English studies show timed cues, bilingual lines, automatic wrapping, safe areas, padding, contrast, keyword highlights, overlapping cues, exact boundaries, SRT round trips, shared markers and uniform retiming. | [caption_atlas.rs](src/demos/caption_atlas.rs) |
| Particle Garden | Four acts: fireworks with sub-emitters bouncing off polygon/segment colliders; a boid murmuration over a curl flow field with ribbon trails; text, image and path emitters dissolving into paint-coloured sprites from a padded atlas; bokeh dust and Temperature-curve embers under noise-driven Atmosphere. | [particle_garden.rs](src/demos/particle_garden.rs) |
| The Harbor Line (infographic) | A flat data-journalism explainer about a fictional harbour ferry: a scale-mapped bar chart grows year by year, an axis-labelled line chart traces falling commute times, keyframed counters tick up in a `Row` of stat cards and a timed caption track narrates underneath. Diagram charts and scales, `Keyframes`, `Row`/`Column` layout and captions. | [data_story.rs](src/demos/data_story.rs) |
| Motion (branding) | A Motion ident: a circle, square and triangle spring onto a construction grid and morph into a three-stroke SVG M monogram while a trimmed orbit draws around them, then a burst reveals the lowercase `motion` wordmark and MADE TO MOVE tagline. Grid, chrome, burst and lockup are a `motion::declarative` JSON document composited with Rust-built morphs. | [logo_sting.rs](src/demos/logo_sting.rs) |
| Room to Move (magazine) | An editorial issue: a serif cover with spring-settled letters, a start-aligned two-column spread with 16-unit body text, drop cap and pull quote, and a full-bleed closing page. Grapheme animation, paragraph layout, masks and directional wipes. | [type_in_motion.rs](src/demos/type_in_motion.rs) |
| The Long Way Home (nostalgia) | A brush-and-ink landscape on aged paper with a typewriter caption, rust stamp, orange date mark, dust, scratches, light leak and flickering grain. Pressure strokes, ink material, camera drift and per-frame seeded film damage. The same entry accepts `mode: "json"`; birds use bounded sampled wings, while dust, scratches, hairs and exposure use a separate deterministic expression-based sequence rather than the native keyed random stream. | [native](src/demos/ink_journey.rs), [JSON](assets/ink-journey.json), [loader](src/demos/json_atlas.rs) |
| Signal Garden (sci-fi audio) | An instrument display wired to a live mix: synthesized kick, hats, bass and lead are routed through buses with sidechain ducking, delay/reverb, pan/width and a limiting master. FFT bands drive the radial spectrum and Plexus constellations, the measured duck gain makes the garden pump with the kick, and a motion-blurred radar sweep and bus meters read back the mix. | [signal_garden.rs](src/demos/signal_garden.rs) |
| Harmonics (`formula`) | An equation writes itself and morphs as a waveform doubles frequency. Native Typst vectors, semantic matching and function geometry. | [harmonics.rs](src/demos/harmonics.rs) |

Eighteen entries are available by default, nineteen with `formula`.
All use a 960x540 design canvas and exact 30 fps timelines. Parameters
accept 1..=60 seconds, a deterministic seed and a mark density of 4..=128.
The shared `mode` parameter accepts `default` or `json` and defaults
to `default`. Demos without alternate implementations accept and ignore it.
Animation, Compositing, Filter, Vector and Particle Atlases, plus Particle Features,
Ink Features and Ink Journey, use their native Rust implementations
in `default` mode, while `json` loads the bundled documents.

```sh
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo ink-features --params '{"seconds":60,"mode":"json"}'
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo ink-journey --params '{"mode":"json"}'
```

The two Ink documents also load directly through `motion-cli`. Their `fonts`
parameter defaults to an empty list; provide explicit local font paths for text.
For example, from a POSIX shell at the workspace root, this uses the test fixture
font for verification, not a production typewriter face:

```sh
cargo run -p motion-cli -- validate gallery/assets/ink-features.json --params "{\"fonts\":[\"$PWD/tests/fixtures/fonts/NotoSans-Regular.ttf\"]}"
```

Gallery mode uses its configured fonts and derives `ink_seed` from the shared
64-bit `seed`: Ink Features XORs the low and high halves, while Ink Journey uses
the low half, matching the native recipes. Direct JSON loading exposes `ink_seed`
separately; use it for brush materials and retain `seed` for full-width film grain.
`DemoMetadata.features` describes combinations; features may recur across films.
Optional entries are absent when their feature is disabled. Listing initializes
neither formulas, audio, fonts nor GPU devices.

## Capability Atlases

`animation-atlas`, `brush-atlas`, `filter-atlas`, `compositing-atlas`, `vector-atlas`, `ink-features`, `noise-atlas`,
`audio-atlas`, `typography-atlas` and `caption-atlas` share a left-hand explanation
and a clipped specimen card; `particle-atlas` and `particle-features` use the same
frame. Their 193 atlas pages use explicit local progress and
seeds; seeking does not depend on
the previously viewed frame. Use `--params '{"seconds":60}'` for a slower tour,
then pause or seek in Studio. The default eight seconds is a quick overview,
not a reading pace. The atlas typography uses the supplied generic sans-serif face.

The shared page frame uses 48-unit outer margins and a 568x337 rounded specimen
card, with a soft drop shadow, on the 960x540 canvas. The header shows the category
label, the demo title and the current page number. The caption column names the
page, explains it in three lines and previews the next page. Particle pages also
show their capability group under the explanation. A segmented footer marks
each page and fills with playback progress.

Animation maps every page to an independent 3.2-second sample clock, so changing
the overall duration does not truncate its authored curves or composed motion.
Sampled values retain overshoot and signed signals. Brush Atlas keeps each full
stroke's geometry and material preparation, scales the specimen into the shared
viewport and reveals it with an outer geometric clip. Light paper preserves dark
and translucent materials; Neon retains its dark field and local Bloom.

The specimens call the actual rendering, vector, ink, noise and audio APIs, rather
than approximating their output with decorative animation. Vector artwork, paper
textures and noise fields are generated from fixed inputs; no external image or
audio files are required. Audio Atlas prepares real 48 kHz stereo mixes outside
frame callbacks. Its three lanes show dry mono, processed left and processed right
PCM, with full-page peak envelopes behind the current waveform. One displayed
visual gain is shared across the lanes; it does not change playback volume.
Studio and video export play the processed mix. PNG stills contain no audio.

Typography Atlas and Caption Atlas cover Chinese and English only. Both use the
font library discovered once at application startup, with the generic default
family and ordinary installed-face fallback. No font files or family names are
bundled or hardcoded. Leave out `--font` for the default system-font experience;
the system must have Chinese glyph coverage. Output can differ between machines
with different installed fonts. Text reveal counts come from actual paragraph
parts, not UTF-8 byte lengths. Caption vectors are measured and cached with the
supplied font snapshot; frame sampling never discovers fonts or parses SRT.
The caption specimens use a four-second local clock, except the six-second
retiming example. Captions and markers are visual studies, not synthesized speech.

## Particle Comparisons

The resource-free comparison generator writes 52 PNGs: every preset with Bloom
off/on and real None/AgX display transforms. It rejects overwrites.

```sh
cargo run -p motion-gallery --bin particle_comparisons -- /tmp/motion-particle-comparisons
```

## Default Budgets

Gallery explicitly allows 1 GiB of logical live bytes per frame for Full HD
transition intermediates. The engine default remains 256 MiB; Gallery retains
the unchanged 16777216-pixel limit and no frame-work cap. This admission limit
does not preallocate 1 GiB. Continuous strokes retain their
geometry, pressure and load; Final ink charges actual indexed distance evaluations
against BrushStroke's unchanged 32-million default. Brush Atlas explicitly raises
only its per-stroke material-work allowance to 64 million evaluations for the
full-length, five-bristle comparison; other engine limits remain unchanged.
Ink Features explicitly allows 128 million material evaluations per stroke. Its
five-bristle study uses 96 path sample intervals to stay within the existing path
verb limit; neither engine defaults nor admission limits are raised globally.
Particle Features explicitly allows 1.6 billion simulation work units per specimen,
up from the 100-million default. Every specimen derives its population from twice
the `count` parameter, without changing particle, byte or authoring limits.
These are not process-memory or real-time playback guarantees.

With the bundled Noto Sans CJK fixture, every catalog entry keeps seed 20261027 and an
eight-second timeline. First/middle/last sampled frames at 1920x1080 pass default
admission and exact CPU replay in Final and Draft. This is 108 sampled cases (114 with
`formula`), not every frame, custom parameter set or GPU full-resolution acceptance. Native backend parity is checked separately;
do not simplify artwork or infer throughput from admission tests.

## Validation

Gallery tests cover catalog registration, parameter validation, authored adapters,
rendering, deterministic replay and backend parity. Shared duration checks live at
the catalog level; engine primitives are tested in their owning crates. Artwork
titles, descriptions and fixed catalog or page counts are not separate test contracts.

```sh
CARGO_INCREMENTAL=0 cargo test --locked --offline -j 2 -p motion-gallery --lib
MOTION_TEST_FONT=/path/to/font.ttf CARGO_INCREMENTAL=0 cargo test --locked --offline -j 2 -p motion-gallery --lib every_demo_renders_its_subject_and_replays_exactly -- --ignored --nocapture
MOTION_TEST_FONT=/path/to/font.ttf CARGO_INCREMENTAL=0 cargo test --locked --offline -j 2 -p motion-gallery --all-features --lib films_match_cpu_gpu_and_replay -- --ignored --nocapture
MOTION_TEST_FONT=/path/to/font.ttf cargo test --release --locked --offline -p motion-gallery --lib particle_features:: -- --include-ignored --nocapture
MOTION_TEST_FONT=/path/to/font.ttf cargo test --release --locked --offline -p motion-gallery --lib capability_atlases -- --include-ignored --nocapture
MOTION_TEST_FONT=/path/to/font.ttf cargo test --release --locked --offline -p motion-gallery --features gpu --lib capability_atlases_match_cpu_gpu_and_replay -- --ignored --nocapture
cargo test --release --locked --offline -p motion-gallery --lib audio_atlas:: -- --nocapture
cargo test --release --locked --offline -p motion-gallery --lib noise_atlas:: -- --nocapture
cargo test --release --locked --offline -p motion-gallery --lib typography_atlas:: -- --include-ignored --nocapture
cargo test --release --locked --offline -p motion-gallery --lib caption_atlas:: -- --nocapture
MOTION_TEST_FONT=/path/to/font.ttf cargo test --release --locked --offline -p motion-gallery --all-features --lib default_full_hd_output_preserves_demo_layouts -- --ignored --nocapture
MOTION_TEST_FONT=/path/to/Georgia.ttf cargo test --release --locked --offline -p motion-gallery --features gpu --lib type_in_motion::tests::readable_columns_fit_and_frames_replay -- --ignored --nocapture
MOTION_TEST_FONT=/path/to/Menlo.ttc cargo test --release --locked --offline -p motion-gallery --features gpu --lib signal_garden::tests::readable_hud_fits_and_frames_replay -- --ignored --nocapture
```

Particle Features has separate CPU checks for visible subjects, motion, viewport isolation
and exact replay on all 47 pages, plus worker and time-mapping state checks. Its authored
studies are not exhaustive acceptance of every particle parameter combination or backend.

The ten capability atlases have separate CPU checks for visible subjects, motion,
viewport isolation and exact replay on all 193 pages. The typography and caption
cases explicitly discover system fonts, including Chinese coverage; the other eight
use `MOTION_TEST_FONT`. Catalog checks exercise one- and eight-second durations;
atlas adapter and page-rendering checks also cover sixty-second timelines.
Audio checks cover finite deterministic
PCM, real DSP changes, limiting and audible delay/reverb tails. These sampled
checks do not establish full-resolution GPU parity or physical audio-device behavior.
The separate native GPU check samples two times on each of the 193 pages at
288x162, requires at most two channel values of CPU/GPU difference and verifies
exact GPU replay.
Typography checks also cover grapheme integrity, stable layout and cool/warm
gradient pixels. Caption checks exercise exact cue boundaries, overlap, SRT
interchange, marker progress and shared retimed boundaries.
