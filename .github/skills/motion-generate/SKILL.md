---
name: motion-generate
description: 'Generate Motion declarative JSON scene and video files from natural-language content descriptions. Use when asked to turn an idea, storyboard, animation, typography, particle effect, diagram or motion design into a .json file, edit an existing Motion JSON film, or explain how to preview, hot-reload, render stills and export video directly from JSON with motion-cli.'
argument-hint: 'Describe the scene, duration, dimensions, text, motion and desired output format'
user-invocable: true
---

# Motion JSON Authoring

Turn the user's content description into a complete, independently compilable Motion
JSON document. Create or edit the actual file, then return its location, verification
results and concrete commands for preview and export. Do not stop at a storyboard or
return only a JSON snippet unless the user asks for a proposal or an explanation.

## Establish the Brief

1. Identify the subject, exact on-screen copy, composition, visual style, motion,
   scene order, duration, aspect ratio, resource needs and delivery format.
2. Ask only about missing choices that materially change the result. Otherwise
   state conservative assumptions and proceed: 1920x1080 design canvas, 30 fps,
   eight seconds, explicit deterministic seeds and H.264 MP4 delivery.
3. Follow the user's language for visible copy and the final response. Do not
   invent additional captions, branding or unsupported factual claims.
4. Use an existing user-designated document when supplied. Otherwise create a
   descriptive kebab-case file in `examples/documents/`, without overwriting an
   existing file. Use `gallery/assets/` only for a requested Gallery integration.
   Resolve every relative path from the document's directory.

## Consult the Current Format

Read repository instructions before authoring. Load only references needed by the
requested content; the current checked-in schema and implementations take precedence
over remembered APIs.

- [JSON Schema](../../../schemas/motion.schema.json): accepted fields,
  enum names and structural constraints. Inspect relevant definitions rather than
  reading the entire schema.
- [Declarative tutorial](../../../docs/en/tutorial/16-declarative.md): geometry,
  animation, text, resources, bindings and reusable definitions.
- [Document contracts](../../../docs/en/design/documents.md): determinism, budgets,
  compilation, parameters and validation boundaries.
- [CLI reference](../../../docs/en/cli.md): exact command grammar, codecs and errors.
- [Studio guide](../../../docs/en/design/studio.md): document hot reload and preview behavior.
- [Intro document](../../../examples/documents/intro.json) and
  [Orbit document](../../../examples/documents/orbit.json): small working examples.
- [Animation Atlas](../../../gallery/assets/animation-atlas.json),
  [Particle Features](../../../gallery/assets/particle-features.json),
  [Filter Atlas](../../../gallery/assets/filter-atlas.json) and
  [Vector Atlas](../../../gallery/assets/vector-atlas.json): reusable examples for
  those domains; inspect only the relevant scenes and definitions.

For an uncertain field, animation or effect, inspect its owning types in
`crates/motion-scene/src/` before using it. Do not invent properties, codecs,
expression functions or a format version. Do not change the engine, dependencies,
CI or generated schema merely to support an imagined feature. If a requested
effect is not expressible, explain the limitation and obtain approval for an
alternative or an engine extension.

## Bundled Scripts

Use Node.js 18.3 or newer; these scripts have no package dependencies. Run them
from the workspace root as shown, or use absolute script paths from another directory.
Input/output paths and `--motion` resolve from the caller's working directory.

- [create.cjs](./scripts/create.cjs) creates a blank, single-scene document with
   explicit design size, frame rate and duration, plus a computed local `$schema`
   link. The output parent directory must exist; it never overwrites, including
   symlinks. It does not generate the artwork or launch the engine.
- [check.cjs](./scripts/check.cjs) parses JSON, checks a declared local schema file
   and literal font/image/SVG/SVG-morph/caption/audio file paths, then runs
   `motion-cli validate`. It does not implement JSON Schema constraint validation,
   calculate expressions or substitute parameters. The engine owns those semantic
   checks; unused parameter values are not scanned as resources.

```sh
node .github/skills/motion-generate/scripts/create.cjs examples/documents/my-film.json --name "My Film" --size 1920x1080 --fps 30 --duration 8 --seed 17

# After replacing the blank scene with the requested content:
node .github/skills/motion-generate/scripts/check.cjs examples/documents/my-film.json

# No build or engine invocation; only JSON and literal local-file preflight:
node .github/skills/motion-generate/scripts/check.cjs examples/documents/my-film.json --static-only

# Use an existing CLI executable instead of Cargo, optionally rendering CPU stills:
node .github/skills/motion-generate/scripts/check.cjs examples/documents/my-film.json --motion target/debug/motion --render-dir my-film-checks --times 0,2,6 --size 960x540
```

The generator defaults to 1920x1080, 30 fps and eight seconds. `--fps` also accepts
`30000/1001`. An optional `--seed` accepts integers from zero to 9007199254740991
and only declares `params.seed`; it is not a global engine seed. Bind it through
`{"$param": "seed"}` to the authored random inputs, or remove it if unused. The
blank scene is a scaffold, never the finished deliverable. The generator sets both
film and scene duration; update the film duration when restructuring scenes.

The checker defaults to `cargo run -p motion-cli -- validate`, which may compile.
Use `--static-only` when engine execution is unavailable or not authorized, and
report that engine preflight and pixels remain unverified. A missing `$schema` is
reported rather than silently supplied. URLs are not fetched; referenced files
must be local. Document and schema inputs are limited to 16 MiB each, and static
inspection is bounded to 128 nested levels and 100000 JSON values.

Frame rendering requires both `--render-dir` and `--times`, up to 60 explicitly
chosen times inside the actual film. Choose meaningful sample times instead of
inferring duration by summing scene/transition lengths or parsing human CLI output.
The render directory must be new and its parent must exist. Failed rendering may
leave partial outputs for inspection; subsequent runs need a new directory. The
checker never passes `--overwrite`, opens Studio or exports video. Inspect rendered
images separately; a successful command does not establish visual quality.

Use `--help` for each script's options. Run their focused regression checks with:

```sh
node --test .github/skills/motion-generate/scripts/scripts.test.cjs
```

## Author the Document

1. Create valid JSON without comments or trailing commas. Include `version: 1`,
   a descriptive `name`, `size`, `fps` and `tracks`. Set `background` or explicitly
   design for transparency. Add a valid document-relative `$schema` link.
2. Represent the actual content, not a generic demo substituted for the brief.
   Use intentional hierarchy, readable text, coherent layout and motion that
   supports the subject. Keep text and animated geometry within their intended
   bounds at the start, middle, end and transition points.
3. Use native JSON geometry, paints, text, images, SVG, particles and animations.
   Prefer `defs` / `use` for repeated components and `params` / `$param` for
   genuinely useful controls. Use `$calc` for load-time numeric calculations and
   expressions for explicit scene-local time. Keep references acyclic and ids
   unique. Do not add parameters that the document does not actually consume.
4. Preserve exact timing semantics: scene clocks are local, transitions overlap
   scenes, and sampled content may need a separate source clock. Align discrete
   cuts to the frame grid and avoid zero-duration scenes. Do not assume that
   summing scene and transition durations gives the final film duration.
5. All randomness uses explicit seeds. Bound particle counts, sampled points,
   material frames and effects; validate before allocating untrusted sizes.
   Avoid gratuitous full-frame blur, Bloom or excessive populations.
6. Gradients default to object-bounding-box coordinates. Use `user_space` for
   pixel/design-coordinate gradients; include stops at zero and one. Check text
   gradients against glyph-coordinate behavior rather than assuming page space.
7. Declare font files in `fonts` for text and glyph emission. Use user-supplied
   fonts or an explicitly selected existing font with the required glyph coverage.
   Do not scan fonts or assume a machine-specific family is available. Do not
   redistribute a system or downloaded font without permission. Fixture fonts
   may serve verification, but disclose them and do not imply production suitability.
8. Reference only actual resources whose paths and usage rights are known. Keep
   requested audio, imagery and fonts explicit. Never write secrets into JSON.
9. Keep the document independent of Gallery Rust builders. Do not generate a Rust
   entry point or add a catalog registration just to preview a JSON file.

## Validate and Inspect

All commands below run from the `motion-rs` workspace root. Substitute the actual
document path, sample time, dimensions and output paths; quote paths containing spaces.
Use an existing built `motion` executable when appropriate, or Cargo as shown.

1. Run the bundled checker for JSON and resource preflight; its default invocation
   also performs the owning CLI validation below. Do not repeat that validation
   unless inputs changed. `--static-only` does not perform engine validation.
2. Run the owning CLI validation:

   ```sh
   cargo run -p motion-cli -- validate examples/documents/my-film.json
   ```

   This compiles the document and preflights first, middle and last frames without
   encoding or opening a window. It does not evaluate every intermediate frame.
   Repair local diagnostics and rerun the same check before widening scope.
3. Render a representative CPU still, plus additional frames at meaningful cuts,
   peak motion, text reveals or particle lifecycle events. Inspect the actual images
   for blank output, clipping, text overlap and missing resources:

   ```sh
   cargo run -p motion-cli -- still examples/documents/my-film.json --backend cpu --time 2 --size 960x540 --output my-film-check.png
   ```

   Pick a time inside the real duration. Do not overwrite an existing file without
   authorization. Temporary verification outputs need not become repository assets.
4. For seeded or stateful-looking content, replay the same time after seeking to
   other times and compare same-backend pixels. Explicitly state any tolerance.
5. Verify CLI syntax against current `--help` when uncertain. Check process exit
   codes and report actual results. JSON parsing, preflight, a rendered still,
   interactive Studio acceptance and video encoding are different checks.
6. Generate and inspect a short encoded sample only when export is requested or
   authorized. Do not launch Studio automatically unless the user requests a window.
   If builds, resources, codecs, disk space or a display prevent a check, disclose
   the specific unverified step and still provide verified command syntax.

## Return Preview and Export Commands

Finish concisely in the user's language with a clickable link to the generated
document, its duration/design size/fps, important assumptions or resource requirements,
checks that actually ran, and commands containing the real paths. Do not leave
`my-film.json` placeholders in the final response.

Include these primary workflows, adapted to the user's requested format:

```sh
# Interactive preview; saving the JSON or referenced resources reloads it in place.
cargo run --release -p motion-cli -- studio examples/documents/my-film.json

# One frame, with an explicit output resolution and sample time.
cargo run --release -p motion-cli -- still examples/documents/my-film.json --time 2 --size 1920x1080 --output my-film.png

# Full film, H.264 MP4.
cargo run --release -p motion-cli -- export examples/documents/my-film.json --codec h264 --size 1920x1080 --output my-film.mp4
```

Do not add `mode: "json"`: a `.json` path already selects document loading. Such
a mode parameter belongs to Gallery entries, not arbitrary standalone documents.
Use `--params` only for keys declared by this file. JSON relative resources resolve
from its directory, whereas output paths resolve from the command's working directory.
Output parent directories must already exist; create a requested new directory using
the available directory tool or show `mkdir -p` before the commands. Append
`--overwrite` only when replacement is authorized.

The current `motion-cli` uses the facade's default features, including declarative
documents, Studio, GPU and FFmpeg. MP4 export still requires the native FFmpeg
libraries and an available codec; Studio requires an interactive display. For a
headless preview use `still`. For an approved non-FFmpeg delivery use `--codec gif`
or `--codec png`, selecting a `.gif` destination or a new frame-directory destination.
Do not pass `--quality` to GIF, PNG sequence or ProRes. For a partial film, use
`export --range START:END`, not `--time`. Alpha delivery uses the supported
VP9/ProRes or image workflows, not H.264 MP4.
