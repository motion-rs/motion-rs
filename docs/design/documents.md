# Declarative Documents

A document is a versioned JSON description of a complete timeline: canvas size,
frame rate, background, tracks of timed scenes, transitions, audio occurrences, font
files and an optional film look. The `motion-scene` package owns the data model, its
JSON Schema and compilation to a `Timeline`; the facade re-exports it as
`motion::declarative` behind the `declarative` feature and adds `Project` and command
line entry points ([command line](../cli.md#declarative-documents)). Authoring
guidance is in the [tutorial](../tutorial/16-declarative.md); Rustdoc lists the types.

The document model reuses core value types. Plain enumerations (`BlendMode`, `FillRule`,
`MaskMode`, `ClipMode`, `LineCap`, `LineJoin`, `GradientSpace`, `GradientUnits`,
`GradientExtend`, `Waveform`, `ArrowHead`, `Sampling`, `Direction`, `IrisShape`, `Pivot`)
and the value types `Color`, `Point` and `Rect` implement serde and schema behind
`motion-core`'s `serde` and `schema` features, so the spec layer has no mirror types for
them. The spec layer keeps only what the core types cannot express: animated wrappers
(`Animated<Color>`, `Animated<Point>`, `Animated<Rect>`), parameter placeholders, gradients
written as `start`/`end`/`stops` objects with degrees, strokes with an animated width and
dashes, and vector shorthands such as sizes and offsets. Validation of the reused types
(for example the stroke miter limit) belongs to core; the compiler calls it instead of
repeating it.

The same rule applies to content policies: image fit, paragraph alignment and splitting,
vector write options, particle preset names, color-field coordinates, collision responses,
event kinds and inheritance reuse `motion-content` types. Particle blending reuses core's
`InstanceBlend`. Document fields use the original type names; duplicate `*Spec` names are
removed rather than retained as compatibility aliases. Content's optional `serde` and
`schema` features expose the JSON contracts; plain policies are available without text or
particle preparation. Disabling a document
content feature therefore still permits parsing and serialization, but compilation reports
the unsupported capability. Resource descriptions, animated properties and particle shape
parameters that retain f64 authoring precision remain document-specific structures.

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
  parsed by `Color::parse`; writing a document emits `#rrggbb[aa]`, or `rgb()` with
  fractional channels when 8 bits cannot represent the color. Stroke joins are
  `"round"`, `"bevel"` or `{"miter": {"limit": 4}}`, and a pivot is a name such as
  `"top_left"`, `{"relative": [x, y]}` or `{"point": [x, y]}`.
- An animated property is either a fixed value or an object with exactly one motion
  key: `tween`, `keyframes`, `oscillate`, `noise`, `expr`, `spring`, `clock`, `map`,
  `follow_path`, `components` or `sum`. Fixed values are never JSON
  objects, so the two forms cannot be confused. Oscillators and noise apply to
  numeric properties only. Easing is a closed serializable set; custom callbacks are
  not representable.

## Animation composition

`spring` uses the core closed-form Spring with `from`, `to` and optional `mass`,
`stiffness`, `damping`, `initial_velocity` and `settling_threshold`. Omitted physical
parameters keep the core defaults; invalid configurations and settling durations
over 1e6 seconds fail during compilation. `follow_path` samples core arc-length
motion from SVG `data`, `duration`, `easing` and a numeric `component`: `x`, `y` or
`angle` in clockwise degrees. It does not introduce a separate path approximation.

`clock` takes a nested `source`, finite `scale` and signed seconds `offset`, each
with magnitude at most 1e6. It samples the source at
`clamp(t * scale + offset, -1e6, 1e6)` seconds, rounded to engine ticks. Negative
scales reverse time; expressions and their bindings read the remapped clock.
`map` takes a numeric `source` and arithmetic `expr`. Only inside that mapping,
`t` is the source value, not seconds. Mapping expressions accept numeric document
parameters but reject element bindings.

`components` takes one numeric animation for each component of a number, point or
rectangle. `sum` takes 1..=32 numeric sources and adds them in order. Combined with
`map`, it describes paired numeric signals without callbacks. Recursive motion
composition is limited to 32 levels. `Animated<f64>::compile` validates an independent
sampler without parameters, bindings or IO; Rust document builders can use that
same definition to precompute static sampled curves.

## Subtree clipping

An element's `clip.shape` selects `rect`, `rounded_rect`, `circle` or `path` in
element-local coordinates. `rounded_rect` reuses rectangle content's `bounds` and
optional `corner_radius` fields; both accept animations, and the radius defaults
to zero design units. The compiler lowers it to the core rounded rectangle, using
the same coverage or geometric clip mode as other shapes. Invalid sampled geometry
returns a validation error rather than silently dropping the clip.

## Numeric text readouts

Text accepts exactly one of `content`, `spans` or `number`. A numeric readout has
an animated `value`, decimal `precision` in 0..=16 and optional fixed `prefix` and
`suffix`, whose combined length is at most 4096 bytes. It reuses the same paragraph
configuration and explicit font environment as ordinary text. Nonfinite expression
results use Rust's numeric spelling; no unbounded user formatting program runs.

## Parameters

A root `params` object declares named default values: numbers, strings, colors,
arrays or whole subtrees. Any object `{"$param": "name"}` outside `params` is
replaced by that value while the document loads, before types are checked, so a
parameter can stand in for a radius, a color, a text string or a list of children.
Overrides come from `Document::from_json_with`, `load_with`, `Project::document` and the
command line's `--params`: a JSON object whose keys must be declared, each replacing the
whole default value. Unknown keys and unknown `$param` names are errors with the
referring path.

`{"$calc": "ceil($seconds * $fps / $pages) / $fps"}` computes a finite number
once during loading, after overrides merge and before `$param` substitution and
definition expansion. Calculations work inside parameter values and elsewhere in
the document. They use the numeric expression grammar and f64 arithmetic, reading
numeric or boolean parameters as `$name`; calculated parameters resolve their
dependencies regardless of declaration order. Time `t`, element bindings, cycles
and non-finite results are rejected. Numeric results still pass through the
destination field's normal type and range checks. Integer-valued results within
the exact f64 integer range also work in integer-only fields.

Each calculation accepts at most 4096 source bytes, 1024 expression nodes and 48
expression nesting levels; parameter resolution permits 32 nested dependencies.
Raw parameter values, including full-width integer seeds, are not converted to
f64 unless used in arithmetic.

The loaded `Document` holds the effective values in `params`, with references
outside `params` resolved, so writing it back records the resolved film. Each parameter,
declared or overridden, is limited to 64 KiB, a document to 256 parameters, and the
document after substitution to the document size limit: each `$param` reference charges
its value once, and declarations are not charged separately.
Numeric parameters are also readable in expressions as `$name`.
`schema()` accepts `$param` and `$calc` objects wherever a value is allowed.

## Reusable definitions

A root `defs` object declares named values, styles or component subtrees. Any object
`{"use": "name"}` outside root `params` is replaced by the named definition during
load, after parameter substitution and before type checking. Definitions may refer
to other definitions, and `$param` references inside definitions use the effective
document parameters. References introduced by a parameter value also expand at the
use site; stored parameter values are not changed by definition expansion.
`use`, `$param` and `$calc` are reserved keys: no document field takes these names,
so an object carrying one is always read as a reference where the rules above apply.

Sibling fields override an object definition's fields, replacing each whole field
rather than merging nested objects or arrays. Scalar and array definitions accept
no sibling overrides. A definition can itself inherit another definition with `use`.
Unknown names, non-string references and cycles report the referring document path;
unknown names suggest a nearby declared name. Unused definitions are size-checked
but are not expanded or type-checked. The loaded document contains expanded values
and no root `defs`, so serialization records the resolved film, not its templates.
These are load-time JSON shorthands, not retained fields in the Rust builder model.

Each document admits at most 256 definitions, each limited to 64 KiB after parameter
substitution and before definition expansion. Reference chains are limited to 32
and expanded JSON nesting to 128 levels; the element nesting limit still applies.
The resolved input plus the cumulative serialized bytes of cloned definitions must
fit the 16 MiB document limit, checked before each clone. This conservative admission
can reject expansions whose final JSON is smaller after overrides. Counting serialized
bytes does not allocate a temporary JSON buffer. Instance ids are not renamed, so
authors override ids when reusing a component in the same scene.

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

## Vector paint and effects

Linear gradient endpoints, radial centers/radii/focal points and sweep centers/angles
accept animated properties. Stops and interpolation options remain fixed. Sweep angles
use degrees; a full turn tolerates only floating-point conversion roundoff, not an
authored span above 360 degrees. Gradient geometry is validated initially and again
when each frame is prepared.

Polygon and polyline sources accept exactly one of `points`, `sample` or `animated`.
`animated` is a bounded list of independently animated `[x, y]` points, sampled at the
scene clock rather than baked over a range. Polygons require at least three points and
polylines at least two; the same 65536-point cap applies to all source forms.

Filters include `blur_xy`, seeded `grain` and the shared `optics` operations: lens
flare, lens distortion/dispersion, lens vignette, bokeh, radial blur, anamorphic
streaks and relighting. Spatial and strength properties supported by these operations
are animated; Bloom intensity and halation strength are animated too. Bokeh polygon
rotation uses degrees. The compiler delegates optical validation to the core filter
model, preserving its tap, aperture and finite-direction limits. Grain keeps the full
unsigned 64-bit seed and introduces no hidden random state. Every filter is validated
at compilation and again for each frame, so an animated parameter that only becomes
invalid later fails that frame with its `filters[i]` path instead of reaching a backend.

## Content kinds and features

| Content key | Feature | Notes |
| --- | --- | --- |
| geometry, `path`, `morph` | none | `morph` pairs two path data strings once and interpolates by progress |
| `text` | `text` | Runs (`spans`), `split`, `visible_parts` and per-part `parts` animation |
| `image`, `svg`, `svg_morph` | `image`, `svg` | Files resolve against the document directory; SVG also accepts embedded markup |
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

SVG `source` and SVG morph `from` / `to` accept a file path string or `{"data": "<svg ...>...</svg>"}`.
Sources are limited to 16 MiB and parsed once during compilation. Embedded markup adds
no watched resource. SVG `write_options` sets the positive outline `width` and
`outline_fraction` in `0..=1`; omitted options preserve the core handwriting defaults.

## Particle simulations

`particles` accepts either explicit emitters, modifiers and style, or a `preset`.
Presets delegate to the same core recipes as code-authored simulations: `sparks`,
`embers`, `dust`, `motes`, `snow`, `rain`, `smoke`, `fire`, `confetti`, `fireworks`,
`fireflies`, `logo_dissolve` and `logo_assemble`. A preset supplies `kind` and local
`bounds`, with optional `density`, `speed` and `intensity` multipliers. It cannot be
combined with explicit emitters, modifiers, style or blend; `step` and `max_particles`
remain available. Presets require an explicit simulation `duration` in `(0, 3600]`
seconds and retain the core recipe's validation and admission limits.

Logo presets require fixed colored vector paths in `preset.logo`, using the preset
bounds. Each path supplies SVG `data` and a fixed `fill`, including gradients; animated
source paints are rejected. At most 256 paths are accepted before compilation allocates
the source tree. No file loading or raster substitution is implied.

An optional animated `sample_time` selects simulation time independently of scene
time, in finite `0..=1e6` seconds. It is sampled at the scene clock and applied through
an explicitly shifted context; a supplied simulation duration also becomes the sampled
context's clip duration. Omitted sampling preserves scene-local behavior. Compilation
retains each simulation once, and seeking does not depend on prior frame evaluation.

Explicit sources also support painted vector surfaces, inline RGBA8 images weighted by
alpha or luminance, animated path progress and glyph outlines from supplied fonts.
Glyph simulations prepare lazily in the scene cache and retain their complete immutable
source even when preparation fails. No font discovery occurs during compilation or sampling.
`premultiplied_color` supplies finite linear birth energy without an sRGB conversion.

`configuration` describes prewarm, a two-phase loop, explicit query remapping, reverse
opt-in, workers and work admission, plus trail, stretch, parallax and flip policies.
Ordered `operator` modifiers add population forces, swept colliders, signed-distance
grids, color fields and stable-ID custom slots 0 to 2. `sub_emitters` declare events and
inheritance; `root_only` reserves custom slot three as a nonrecursive child marker, so
`custom_slot` operators cannot write it. Curl noise and curl flow seeds take 32-bit
values and larger seeds are rejected instead of truncated.
Preset recipes reject extra event rules and presentation overrides, retaining their
own complete recipes. Generated sprite atlases take frame sides 2 to 256 and admit
cumulative construction bytes before generating frames. Data-only `node_circles`
reproduce age-dependent vector presentation without executing authored callbacks.

`trajectory` supports Ballistic, Orbit, NoiseDrift and Converge, with an optional
explicit `sample_time`. Converge and Seek target sets reuse bounded compile-time
point sampling; per-frame animated target sets are rejected.

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
keeps the previous scene ([Studio hot reload](../studio.md#hot-reload)). Each successful
reload updates the watch set to the rebuilt document's dependencies. A result superseded
by another watched change is discarded and rebuilt after the next quiet interval.

## Versioning

`version` must equal `FORMAT_VERSION` (currently `1`). Other versions fail before
compilation with an unsupported-value diagnostic; there is no implicit migration.
A `$schema` key is accepted for editor tooling and ignored by the engine. The schema
from `motion::declarative::schema()` (draft 2020-12) checks structure, types and selected
static ranges: format version, document and scene times, and particle recipe inputs.
Reference alternatives share a `ValueRef` definition instead of repeating three branches
at every value position. Animated results, complete timing rules and resource availability
remain compilation or evaluation checks.
[`schemas/motion.schema.json`](../../schemas/motion.schema.json) is the generated schema and is tested to stay
in sync.

Widening a field to accept new JSON forms keeps the format version when every previous
JSON form still parses with the same meaning. The Rust spec types still change, which
is a source break for code that builds documents directly: gradient geometry and
angles, Bloom `intensity` and halation `strength` are `Animated` values, SVG `source`
and SVG morph `from` / `to` are `SvgSourceSpec`, and the curl noise `seed` is `u64`.
Plain values convert with `.into()`.

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

`CompileCache` is a caller-owned single-entry cache for file documents. `Project` shares
one instance across its registered documents, direct JSON paths and clones; plain Rust
factories are never cached. A hit requires the absolute document path, complete source,
parameter overrides and dependency contents to match. Loads still read the source and
bounded dependency snapshots, so same-length resource edits with unchanged modification
times invalidate the entry. No global mutable cache or frame history is introduced.

Only successful compilations replace the entry. Snapshot admission is limited to 16 MiB
of source, serialized overrides and dependency bytes, and at most 256 dependencies;
larger or unreadable dependency sets compile normally without caching. These are retained
input limits, separate from the compiled timeline's own resource limits. A failed reload
does not replace either the last successful cache entry or the running Studio scene.

## Resources

Relative font, image, SVG and audio paths resolve against `CompileOptions::base_dir`;
loading a file uses the document's own directory. Paths containing `..` are
rejected. Fonts listed in `fonts` are read during compilation and bound to every text
element, overriding the render font environment. Images are opened lazily through the
render `Assets` owner; SVG sources are read and parsed during compilation, and audio
files are decoded at export, under the
same rules as code-authored content ([resources](resources.md)). Without a base
directory, images resolve against the assets root and SVG and audio paths against the
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
| Definitions per document, size of one definition | 256, 64 KiB |
| Definition reference chain, expanded JSON nesting | 32, 128 levels |
| Expression source, terms, nesting | 4096 bytes, 1024, 48 levels |
| Motion composition nesting, sources in one sum | 32 levels, 32 |
| Numeric text precision, combined affix bytes | 0..=16, 4096 |
| Binding chain, distinct bindings per scene | 32, 1024 |
| Rich text spans | 256 |
| Particles requested, burst or trajectory count | 1000000 |

Engine limits for paths, strokes, filters and rasterization still apply at render time.

## Diagnostics

Diagnostics from loaded files carry source positions. `load`, `Project` and the command
line locate each diagnostic's JSON path and failing field in the original text and append
`(line L, column C)`; when the exact key is missing, for example because a `$param`
object stands in for the value, the deepest existing key is reported. An unknown key
points at the key itself and drops serde's list of valid names when it has more than
eight. Unknown keys and variants, parameters, element ids, audio clip and bus names, and
expression functions suggest the closest name within edit distance three. Positions come
from the original file, so documents parsed from strings with `Document::from_json`
report positions only for parse errors and compile errors have none.

Parse failures are a `ValidationError` whose path follows the JSON structure, for
example `tracks[0].items[1].scene.content`. Compilation reports a single
`ValidationError`, or an `ErrorReport` holding one per invalid property, with paths
such as `tracks[0].items[0].scene.content.children[2].circle.radius` and the failing
field. Parameter and expression errors name the failing property. Timeline construction failures, such as an overlapping transition, are
reported at their track item path. Text, image and SVG content parse without their
features but fail to compile with an unsupported-feature diagnostic.
