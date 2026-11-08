# Declarative Documents

A document is a versioned JSON description of a complete timeline: canvas size,
frame rate, background, tracks of timed scenes, transitions, audio occurrences, font
files and an optional film look. The `motion-scene` package owns the data model, its
JSON Schema and compilation to a `Timeline`; the facade re-exports it as
`motion::declarative` behind the `declarative` feature and adds `Project` and command
line entry points ([command line](../cli.md#declarative-documents)). Authoring
guidance is in the [tutorial](../tutorial/16-declarative.md); Rustdoc lists the types.

## Data Model

- Documents are plain data. Parsing and serialization never read resources, discover
  fonts or evaluate a frame. The same Rust types serve as a builder API, so a document
  built in code and one parsed from JSON are equal values.
- Every element selects at most one content key (`children`, `rect`, `circle`, `text`,
  `image`, `svg`, …). Two content keys, unknown keys and unknown enum names are parse
  errors; an element with no content key draws nothing.
- Units are seconds for time, clockwise degrees for angles, design units for lengths,
  `[x, y]` for points and `[left, top, right, bottom]` for rectangles. Frame rates are
  an integer or a `"NUMERATOR/DENOMINATOR"` string. Colors are CSS-style strings
  parsed by `Color::parse`; parsed colors keep their source text so writing a parsed
  document reproduces it.
- An animated property is either a fixed value or an object with exactly one motion
  key: `tween`, `keyframes`, `oscillate` or `noise`. Fixed values are never JSON
  objects, so the two forms cannot be confused. Oscillators and noise apply to
  numeric properties only. Easing is a closed serializable set; custom callbacks are
  not representable.

## Parameters

A root `params` object declares named default values: numbers, strings, colors,
arrays or whole subtrees. Any object `{"$param": "name"}` outside `params` is
replaced by that value while the document loads, before types are checked, so a
parameter can stand in for a radius, a color, a text string or a list of children.
Overrides come from `Document::from_json_with`, `load_with`, `Project::document` and the
command line's `--params`: a JSON object whose keys must be declared, each replacing the
whole default value. Unknown keys and unknown `$param` names are errors with the
referring path.

The loaded `Document` holds the effective values in `params` and no `$param` objects,
so writing it back records the resolved film. Each parameter is limited to 64 KiB, a
document to 256 parameters, and the document after substitution to the document
size limit. Numeric parameters are also readable in expressions as `$name`.
`schema()` accepts a `$param` object wherever a value is allowed.

## Expressions and bindings

A numeric, point or rectangle property may be `{"expr": "..."}`; a point takes one
expression per component, `{"expr": ["x", "y"]}`. Expressions are pure arithmetic on
numbers with `+ - * / % ^`, comparisons (yielding 1 or 0) and the functions `sin cos
tan asin acos atan atan2 sqrt abs floor ceil round fract exp ln sign radians degrees
min max pow mod clamp lerp smoothstep step if`. Names are `t` (scene-local seconds),
`pi`, `tau`, `$parameter` and `@id.path` bindings. Sources are limited to 4096 bytes,
1024 terms and 48 levels of nesting.

An element with an `id` can be read by expressions in the same scene as
`@id.property`, where the property path follows the element's JSON, for example
`@dot.rotate`, `@dot.position.x` or `@ring.circle.radius`. A binding reads the other
element's own property value, not its world transform, evaluated at the same
scene-local time. Dependencies may chain up to 32 deep, but a cycle, an unknown id,
a duplicate id or a path that is not numeric is a diagnostic. Ids are scoped to
their scene. Expressions never read frames, files or other global state, so
evaluation stays a pure function of time.

## Content kinds and features

| Content key | Feature | Notes |
| --- | --- | --- |
| geometry, `path`, `morph` | none | `morph` pairs two path data strings once and interpolates by progress |
| `text` | `text` | Runs (`spans`), `split`, `visible_parts` and per-part `parts` animation |
| `image`, `svg`, `svg_morph` | `image`, `svg` | Files resolve against the document directory |
| `particles`, `trajectory` | `particles` | Seeded simulations; read the scene clock, cache and duration |
| `ruler`, `callout`, `line_chart` | `diagram` | Labels use the render font environment |
| `captions` | `captions` | Laid out at compile time with the document `fonts` |
| `formula` | `formula` | Typst math, optionally written or morphed |

Documents using a kind whose feature is off still parse and serialize but fail to
compile with an unsupported-feature diagnostic. `ruler`, `callout` and `line_chart`
draw text through the render environment, so `CompiledDocument::render_options` (also
used by `Project` and the command line) installs the document `fonts` there; a caller
rendering `timeline()` with unrelated options must supply fonts itself. Particles are
rebuilt per frame from the scene context; compilation constructs everything else
eagerly so Typst, SVG and layout errors surface with their path.

## Audio

`audio` clips accept `volume`, `pan` and `spatial` position as animated properties,
sampled in seconds since the clip starts (animated `volume` becomes a volume envelope,
animated `pan` a pan animation). `bus` routes a clip to a declared entry of `buses`,
each with a gain, effects and ducking. `master` sets gain, limiter and loudness
normalization. `effects` (`eq`, `compressor`, `gate`, `de_esser`, `delay`, `reverb`)
require feature `audio`; an `eq` with animated frequency or gain becomes automated EQ.
A clip is lowered by earlier clips named by `id`: `ducked_by` lists triggers `{"clip":
id}`; bus triggers apply to buses and must be declared earlier. Engine limits apply at
validation and mixing: 256 clips, 16 buses, 16 effects per clip or bus and 4 ducks per
clip.

## Reload and validation

`CompiledDocument::resources()` lists the files a document refers to, resolved against the
base directory, without duplicates; `load` puts the document file first. `Project` uses the
list in two places. `Project::check` and the `validate` command require every listed file to
exist and preflight the first, middle and last frames without rendering. Studio watches the
list: a document opened there is rebuilt by calling the same loader, and a failed rebuild
keeps the previous scene ([Studio hot reload](../studio.md#hot-reload)). The watch set is
fixed when Studio starts.

## Versioning

`version` must equal `FORMAT_VERSION` (currently `1`). Other versions fail before
compilation with an unsupported-value diagnostic; there is no implicit migration.
A `$schema` key is accepted for editor tooling and ignored by the engine. The schema
from `motion::declarative::schema()` (draft 2020-12) checks structure and types; value
ranges, timing rules and resource availability are checked only by compilation.
`examples/documents/motion.schema.json` is the generated schema and is tested to stay
in sync.

## Compilation

Compilation validates every property first and collects diagnostics before building
the timeline, so one run reports every invalid property rather than only the first.

- Scene content is evaluated in scene-local time. Elements without animated
  properties are built once and their snapshot is shared by every frame; animated
  elements rebuild only their own subtree per time. Visuals depend only on the time
  and the document, so preview, stills and export agree.
- Track items keep the engine's sequential rules: scenes advance the cursor or use an
  explicit `start`, `gap` advances it and a `transition` overlaps the next scene with
  the previous one.
- A look applies its scene stage (shutter, spatial filters, gate weave) inside every
  scene and its display stage through `CompiledDocument::render_options`. `Project`
  applies the display stage to its stills, exports and Studio composition, so a
  document behaves the same through every entry point.

## Resources

Relative font, image, SVG and audio paths resolve against `CompileOptions::base_dir`;
loading a file uses the document's own directory. Paths containing `..` are
rejected. Fonts listed in `fonts` are read during compilation and bound to every text
element, overriding the render font environment. Images and SVG are opened lazily
through the render `Assets` owner, and audio files are decoded at export, under the
same rules as code-authored content ([resources](resources.md)). Without a base
directory, images and SVG resolve against the assets root and audio against the
working directory.

## Limits

| Limit | Value |
| --- | --- |
| Document size | 16 MiB (`MAX_DOCUMENT_BYTES`) |
| Times, durations, delays and repeated motion length | 0..=1e6 seconds |
| Design width and height | 1..=65536 |
| Element nesting, including masks | 64 levels |
| Polygon, polyline and spline points | 65536 |
| Regular-polygon sides and star points | 3..=4096 and 2..=4096 |
| Retained diagnostics per compilation | 256 |
| Parameters per document, size of one parameter | 256, 64 KiB |
| Expression source, terms, nesting | 4096 bytes, 1024, 48 levels |
| Binding chain, distinct bindings per scene | 32, 1024 |
| Rich text spans | 256 |
| Particles requested, burst or trajectory count | 1000000 |

Engine limits for paths, strokes, filters and rasterization still apply at render time.

## Diagnostics

Diagnostics from loaded files carry source positions. `load`, `Project` and the command
line locate each diagnostic's JSON path in the original text and append
`(line L, column C)`; when the exact key is missing, for example because a `$param`
object stands in for the value, the deepest existing key is reported. An unknown key
points at the key itself and drops serde's list of valid names when it has more than
eight. Unknown keys and variants, parameters, element ids, audio clip and bus names, and
expression functions suggest the closest name within edit distance three. Positions come
from the original file, so documents parsed from strings with `Document::from_json`
report positions only for parse errors and compile errors have none.

Parse failures are a `ValidationError` whose path follows the JSON structure, for
example `tracks[0].items[1].scene.content`. Compilation reports a single
`ValidationError`, or `Error::BuildErrors` holding one per invalid property, with paths
such as `tracks[0].items[0].scene.content.children[2].circle.radius` and the failing
field. Parameter and expression errors name the failing property. Timeline construction failures, such as an overlapping transition, are
reported at their track item path. Text, image and SVG content parse without their
features but fail to compile with an unsupported-feature diagnostic.
