# 16 Declarative Documents

## What This Chapter Covers

Describe a whole film as JSON data instead of Rust closures, load it into a `Timeline`,
render it through `Project` or the command line, and build the same documents in code.
Enable the `declarative` feature (on by default). Text, images and SVG elements also
need `text`, `image` and `svg`. The format contract is in
[Declarative documents](../design/documents.md).

## A Minimal Document

Save this document in `examples/documents/` so its relative `$schema` path resolves
to the [committed schema](../../schemas/motion.schema.json).

```json
{
  "$schema": "../../schemas/motion.schema.json",
  "version": 1,
  "size": [640, 360],
  "fps": 30,
  "background": "#0c1311",
  "tracks": [{
    "items": [
      {"scene": {"duration": 3, "content": {
        "circle": {"center": [0, 0], "radius": 28},
        "fill": "#14a082",
        "position": {"tween": {"from": [120, 180], "to": [520, 180], "duration": 3,
                               "easing": "cubic_in_out"}}
      }}},
      {"transition": {"effect": "cross_fade", "duration": 0.5}},
      {"scene": {"duration": 2, "content": {
        "text": {"content": "Hello", "family": "Noto Sans", "size": 48},
        "fill": "white",
        "position": [320, 180]
      }}}
    ]
  }],
  "fonts": ["NotoSans-Regular.ttf"]
}
```

`version` is required and must be `1`. Times are seconds and angles are clockwise
degrees. Each element has at most one content key (`children`, `rect`, `circle`,
`ellipse`, `line`, `arc`, `polygon`, `polyline`, `regular_polygon`, `star`, `ring`,
`arrow`, `spline`, `path`, `text`, `image` or `svg`) plus optional style, transform and
effect keys. [`examples/documents/intro.json`](../../examples/documents/intro.json) is a
complete geometry-only film.

## Rendering a Document

`load` reads a file and resolves relative font, image, SVG and audio paths against the
file's directory. Apply `render_options` so the document's look reaches the output.

```rust,no_run
use motion::prelude::*;

fn main() -> motion::Result {
    let film = motion::declarative::load("examples/documents/intro.json")?;
    let options = film.render_options(RenderOptions::default());
    render_still(film.timeline(), 1.seconds().after_origin(), &options)?.save_png("document.png")
}
```

Register documents next to code timelines with `Project::document`. Any `Project`
command line also accepts a `.json` path in place of a timeline name, and `schema`
prints the JSON Schema for editor completion:

```sh
cargo run -p motion-cli -- still examples/documents/intro.json --time 1.5
cargo run -p motion-cli -- schema > schemas/motion.schema.json
```

```rust,no_run
use motion::prelude::*;

fn main() -> std::process::ExitCode {
    Project::new("films")
        .document("intro", "examples/documents/intro.json")
        .run()
}
```

## Animated Properties

Any scalar, point or color property accepts a fixed value or one motion object:

| Motion | JSON | Notes |
| --- | --- | --- |
| Tween | `{"tween": {"from": 0, "to": 1, "duration": 2, "easing": "back_out"}}` | Optional `delay`, `repeat`, `ping_pong` |
| Keyframes | `{"keyframes": {"keys": [{"time": 0, "value": 0}, {"time": 1, "value": 5, "easing": "sine_out"}]}}` | Times strictly increase; easing belongs to the arriving key |
| Oscillate | `{"oscillate": {"frequency": 2, "amplitude": 0.5, "offset": 1}}` | Numbers only; `waveform` is `sine`, `triangle`, `square` or `sawtooth` |
| Noise | `{"noise": {"seed": 7, "frequency": 1, "amplitude": 4}}` | Numbers only; seeded and deterministic |

Unit easings are strings such as `"cubic_in_out"`; parameterized easings are objects
such as `{"steps": 4}` or `{"cubic_bezier": {"x1": 0.2, "y1": 0, "x2": 0, "y2": 1}}`.

Additional motion objects describe closed-form springs, path components and composition:

```json
{"spring": {"from": 0, "to": 1, "stiffness": 121, "damping": 9}}
```

```json
{"clock": {"source": {"spring": {"from": 0, "to": 1}}, "scale": 2, "offset": -0.2}}
```

```json
{"map": {"source": {"tween": {"from": 0, "to": 1, "duration": 1}}, "expr": "t * t"}}
```

`clock` changes the nested animation's time, while `map` treats `t` as its sampled
numeric value. `components` combines scalar animations into a point or rectangle;
`sum` adds scalar sources in order. `follow_path` supplies `x`, `y` or tangent
`angle` in degrees from SVG path data with a duration and optional easing.

For a live readout, use `text.number` instead of fixed text:

```json
{"text": {"number": {"value": {"expr": "t"}, "precision": 2, "prefix": "t ", "suffix": " s"}, "size": 14}, "fill": "white"}
```

See the [format contract](../design/documents.md#numeric-text-readouts) for numeric
readout limits. Sequence different pages as scenes in a track; each scene has its
own local time and all scenes are validated during compilation.

## Styles, Transforms and Effects

| Key | Meaning |
| --- | --- |
| `fill` | Animated color or `{"linear" \| "radial" \| "sweep": {…, "stops": […]}}` |
| `stroke` | `{"width", "paint", "cap", "join", "dash", "dash_offset"}`; `join` is `{"miter": {"limit": 4}}`, `"round"` or `"bevel"` |
| `trim`, `fill_rule` | Drawn arc-length range and interior rule for geometry |
| `position`, `pivot` | Place the pivot (bounds center by default) at a parent point; `pivot` is a name such as `"top_left"`, `{"relative": [0.5, 0.5]}` or `{"point": [x, y]}` |
| `translate`, `rotate`, `scale`, `scale_xy` | Animated transforms; rotation in degrees |
| `opacity`, `blend`, `z_index`, `hidden` | Compositing and sibling order |
| `filters` | Ordered `blur`, `drop_shadow`, `glow`, `bloom`, `color_adjust`, `cdl`, `halation` |
| `clip`, `mask` | `{"shape": {"rect" \| "circle" \| "path": …}, "mode"}` and `{"element", "mode"}` |

Track items are `{"scene": …}`, `{"gap": SECONDS}` and `{"transition": {"effect", "duration"}}`
with effects `cross_fade`, `zoom_blur`, `{"slide": {"direction"}}`, `{"push": {"direction"}}`,
`{"wipe": {"angle", "feather"}}` and `{"iris": {"shape"}}`. A scene with `start` is
placed at that time. Document-level `look` is `"clean"`, `"cinematic"` or
`{"film": {"seed": N}}`, and `audio` lists file occurrences with `start`, `offset`,
`duration`, `volume`, `fade_in`, `fade_out` and `pan`.

## Parameters

Declare defaults in `params` and refer to them with `{"$param": "name"}` anywhere a value
can appear, including colors, text and whole lists of children. Overrides replace the
whole default and unknown names are rejected:

```json
{
  "version": 1, "size": [320, 180], "fps": 30,
  "params": { "radius": 40, "tint": "#14a082" },
  "tracks": [{"items": [{"scene": {"duration": 2, "content": {
    "circle": {"center": [160, 90], "radius": {"$param": "radius"}},
    "fill": {"$param": "tint"}
  }}}]}]
}
```

```rust
use motion::declarative::{CompileOptions, Document};
use serde_json::json;

let text = r##"{"version": 1, "size": [64, 64], "fps": 10,
    "params": {"radius": 8},
    "tracks": [{"items": [{"scene": {"duration": 1, "content": {
        "circle": {"center": [32, 32], "radius": {"$param": "radius"}},
        "fill": "white"}}}]}]}"##;
let document = Document::from_json_with(text, Some(&json!({"radius": 20})))?;
assert_eq!(document.params["radius"], 20);
document.compile(&CompileOptions::default())?;
assert!(Document::from_json_with(text, Some(&json!({"size": 1}))).is_err());
# Ok::<(), motion::Error>(())
```

From the command line pass `--params '{"radius": 20}'` after a `.json` path. The loaded
document keeps the effective values and no `$param` objects.

Use `$calc` for numeric values that should be computed once after parameter
overrides, rather than animated each frame:

```json
{
  "seconds": 8,
  "fps": 30,
  "pages": 43,
  "page_1_frame": {"$calc": "ceil($seconds * $fps / $pages)"},
  "page_1_start": {"$calc": "$page_1_frame / $fps"}
}
```

Put these declarations inside root `params`. Overriding `seconds` recalculates both
derived values, regardless of declaration order. A `$calc` object can also appear
directly in a numeric field or reusable definition. It uses the same arithmetic
and functions as `expr`, but cannot read time `t` or element bindings; cycles and
non-finite results are errors. Arithmetic uses f64, while an untouched integer
parameter such as a seed keeps its original precision. Use `expr` for values that
must change with frame time.

## Reusable definitions

Declare shared values, styles and component templates in `defs`. A `use` reference
expands while loading; sibling fields replace whole fields from the definition:

```json
{
  "version": 1, "size": [320, 180], "fps": 30,
  "params": {"tint": "#14a082"},
  "defs": {
    "style": {"fill": {"$param": "tint"}},
    "dot": {"use": "style", "circle": {"center": [0, 0], "radius": 20}}
  },
  "tracks": [{"items": [{"scene": {"duration": 2, "content": {
    "children": [
      {"use": "dot", "id": "left", "position": [100, 90]},
      {"use": "dot", "id": "right", "position": [220, 90], "fill": "white"}
    ]
  }}}]}]
}
```

Definitions can reference other definitions and contain parameter references.
Array and scalar definitions use `{"use": "name"}` without sibling fields. An override
of `circle`, for example, must supply the whole circle specification. Cycles and
unknown names fail at the referring path. Loaded documents serialize their expanded
values without `defs`; ids remain exactly as authored.

## Expressions and Bindings

A number, point or rectangle can be `{"expr": "..."}`; a point takes one expression per
component. `t` is scene-local seconds and `$name` reads a numeric parameter. Give an
element an `id` and other elements in the same scene can read its properties with
`@id.property`:

```json
{"id": "planet", "circle": {"center": [0, 0], "radius": 10}, "fill": "#6ad6b8",
 "position": {"expr": ["320 + $radius * cos(t)", "180 + $radius * sin(t)"]}},
{"circle": {"center": [0, 0], "radius": 4}, "fill": "white",
 "position": {"expr": ["@planet.position.x + 28 * cos(t * 4)", "@planet.position.y + 28 * sin(t * 4)"]}}
```

Functions are `sin cos tan asin acos atan atan2 sqrt abs floor ceil round fract exp ln
sign radians degrees min max pow mod clamp lerp smoothstep step if`; comparisons give 1
or 0. A binding reads the other element's own property value, so it does not include
parent transforms. Cycles, unknown ids and non-numeric paths are errors.
[`examples/documents/orbit.json`](../../examples/documents/orbit.json) combines both.

## More Content

| Key | Feature | What it draws |
| --- | --- | --- |
| `morph` | none | `{"from": "M…", "to": "M…", "progress": …}`, styled by the element fill and stroke |
| `svg_morph` | `svg` | Two SVG files matched by id, then by position and shape |
| `particles` | `particles` | `seed`, `emitters` (`point`, `disk`, `rect`, `path`, `burst`), `modifiers`, `style`, `blend` |
| `trajectory` | `particles` | A closed-form `ballistic`, `orbit` or `noise_drift` burst |
| `ruler`, `callout`, `line_chart` | `diagram` | Axes, annotations and charts; the chart derives axis ranges from `bounds` |
| `captions` | `captions` | Inline `cues` or an `srt` file, laid out with the document `fonts` |
| `formula` | `formula` | Typst math, with optional `write` or `morph_to` and `progress` |

Particle colors are authored sRGB. Rulers, callouts and charts draw text through the
render font environment, so render with `CompiledDocument::render_options` (or `Project`
or the command line) when the document declares `fonts`.

## Rich Text

`text` accepts either `content` or `spans`, a list of runs with their own `family`,
`size`, `weight`, `italic` and `fill`. `split` selects characters, words or lines;
`visible_parts` reveals that many leading parts, so `{"expr": "floor(t * 12)"}` is a
typewriter. `parts` staggers `offset`, `scale`, `rotate` and `opacity` by `delay` seconds
per part, and each part sees its own delayed time:

```json
{"text": {"content": "Staggered", "family": "Noto Sans", "size": 40,
          "split": "characters",
          "parts": {"delay": 0.08, "offset": {"tween": {"from": [0, 16], "to": [0, 0], "duration": 0.4}},
                    "opacity": {"tween": {"from": 0, "to": 1, "duration": 0.4}}}},
 "fill": "white", "position": [320, 180]}
```

## Audio

```json
{
  "buses": [{"name": "music", "gain_db": -6, "effects": [{"reverb": {"room": 0.3}}]}],
  "master": {"limiter": {"ceiling_db": -1}},
  "audio": [
    {"id": "voice", "source": "voice.wav", "pan": {"tween": {"from": -0.5, "to": 0.5, "duration": 4}}},
    {"source": "music.wav", "bus": "music", "volume": {"expr": "1 - t / 60"},
     "ducked_by": [{"trigger": {"clip": "voice"}, "depth_db": 18}]}
  ]
}
```

Animated `volume`, `pan` and `spatial.position` are sampled in seconds since the clip starts.
Effects are `eq` (animatable frequency and gain), `compressor`, `gate`, `de_esser`,
`delay` and `reverb`, and need feature `audio`. A clip ducks under earlier clips named by
`id`; a bus can duck under an earlier bus.

## Errors, Validation and Hot Reload

Documents loaded from a file report where the problem is and suggest fixes for typos:

```text
tracks[0].items[0].scene.content.fil: unknown field `fil`; 45 names are valid here. (line 4, column 4) Did you mean `fill`?
```

`validate` checks a document without rendering anything. It builds the timeline, requires
every referenced file, and preflights the first, middle and last frames:

```sh
cargo run -p motion-cli -- validate examples/documents/intro.json
cargo run -p motion-cli -- validate orbit.json --params '{"radius": 40}'
```

In Rust, `Project::check` returns a `ValidationReport`. To preview while you edit, open the
document in Studio; saving it, or a file it uses, reloads the scene in place and keeps the
playhead. A reload that fails keeps the previous version on screen and shows the error in
the header:

```sh
cargo run --release -p motion-cli -- studio examples/documents/orbit.json
```

## Building Documents in Code

The JSON types are also a builder API. Documents round-trip: writing and reading a
document returns an equal value.

```rust
use motion::declarative::{
    Animated, CompileOptions, Document, EasingSpec, Element, SceneSpec, TrackSpec, TweenSpec,
};
use motion::prelude::*;

let document = Document::new([320.0, 180.0], 30).track(TrackSpec::new().scene(
    SceneSpec::new(
        2.0,
        Element::circle([160.0, 90.0], 24.0)
            .fill(Color::WHITE)
            .scale(TweenSpec::new(0.0, 1.0, 1.0).easing(EasingSpec::BackOut { overshoot: 1.7 }))
            .opacity(Animated::tween(0.0, 1.0, 0.5)),
    ),
));
let json = document.to_json()?;
assert_eq!(Document::from_json(&json)?, document);
let film = document.compile(&CompileOptions::default())?;
assert_eq!(film.timeline().frame_count(), 60);
# Ok::<(), motion::Error>(())
```

## Diagnostics

Parsing reports the JSON path of the first structural error. Compilation checks every
property and reports all failures together, each as a `ValidationError` whose `path`
follows the document, such as `tracks[0].items[0].scene.content.children[1].circle.radius`.

```rust
use motion::declarative::Document;
use motion::prelude::*;

let error = Document::from_json(
    r#"{"version": 1, "size": [64, 64], "fps": 24, "tracks": [{"items": [
        {"scene": {"duration": 1, "content": {"circle": {"center": [8, 8], "radius": 4}}}}]}]}"#,
)?
.compile(&Default::default())
.unwrap_err();
let diagnostic = error.errors()[0].downcast_ref::<ValidationError>().unwrap();
assert_eq!(diagnostic.path, "tracks[0].items[0].scene.content");
assert_eq!(diagnostic.field, "fill");
# Ok::<(), motion::Error>(())
```

## Common Pitfalls

- Geometry without `fill` or `stroke` is an error rather than an invisible shape.
- Keyframe `easing` shapes the segment arriving at that key; the first key's easing is ignored.
- Paths containing `..` are rejected, and relative paths resolve against the document's
  directory only when loaded with `load` or a base directory.
- Content kinds whose feature is off (`text`, `image`, `svg`, `particles`, `diagram`,
  `captions`, `formula`, audio effects) still parse and serialize but fail to compile.
- Bindings are scoped to one scene and read the other element's own value, not its
  world transform.
- `$param` substitutes before types are checked: a parameter holding the wrong type
  fails at the referring path.
- The schema checks structure; value ranges and timing are checked by compilation.

## Next Steps

Read [Declarative documents](../design/documents.md) for limits and contracts and the
[command line](../cli.md#declarative-documents) for the `schema` command.
