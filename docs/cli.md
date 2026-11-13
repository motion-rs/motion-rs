# Project Command Line

A binary becomes a command line by registering timelines on a `Project` and calling
`run()`. The same clap grammar serves `examples/motion.rs`, the gallery and any film
crate. This page is the reference for the subcommands, flags and failure behavior
implemented in [`project.rs`](../crates/motion/src/project.rs),
[`args.rs`](../crates/motion/src/project/args.rs) and
[`cli.rs`](../crates/motion/src/project/cli.rs). Terms follow the
[design contracts](design/README.md): a *still* is one rendered frame, a *film* an
exported video file.

## Making a Binary a CLI

```rust,no_run
use motion::prelude::*;

fn main() -> std::process::ExitCode {
    Project::new("motion")
        .timeline("motion", || {
            Timeline::new(Size::new(640.0, 360.0), fps!(30)).track(|track| {
                track.scene(3.seconds(), |context| -> motion::Result<_> {
                    let x = context.sample(&Tween::new(80.0, 560.0, 3.seconds()));
                    Ok(Circle::new(Point::new(x, 180.0), 32.0)?.fill(Color::rgb(20, 160, 130)))
                })
            })
        })
        .run()
}
```

This is [`examples/motion.rs`](../examples/motion.rs). `Project::run` prints
errors with `Display` to stderr and returns `ExitCode::FAILURE`, else `SUCCESS`.
`try_run` returns the `ProjectOutcome` (`Help`, `Listed`, `Schema` with feature
`declarative`, `Validated`, `Still`, `Exported`, `StudioClosed`) and `cli(args)` parses an explicit argument list (without the
program name); `print_outcome` prints a result. The gallery
([`gallery/src/main.rs`](../gallery/src/main.rs)) embeds `ProjectCommand` in its own
clap parser (adding `--font` and the `--demo` alias of `--timeline`) and calls `execute`.

## Registration Rules

- `timeline(name, build)` registers a lazy factory; `timeline_with::<P>` and
  `try_timeline_with::<P>` register typed parameters. Registration and `list`
  never call a builder or touch fonts, devices or encoders.
- With feature `declarative`, `document(name, path)` registers a JSON document that
  is read and compiled on each build; see [declarative documents](#declarative-documents).
- Names are 1..=128 bytes of ASCII letters, digits, `.`, `_` or `-`, unique, with
  at most 256 entries. The project name must be 1..=256 bytes.
- Registration errors from the infallible builder methods are deferred and reported
  together as one `ErrorReport` by the first `cli`/`build` call, before any
  argument is parsed.
- `Project::render_options` supplies the shared fonts/assets and defaults. The CLI
  always overrides the backend preference to the engine default (GPU when `gpu` is enabled) before applying `--backend`.

## Subcommands

| Command | Behavior | Needs |
| --- | --- | --- |
| `list` | Prints a pretty JSON array of `{name, defaults}`; takes no options | none |
| `still NAME` | Renders one frame to a PNG | none |
| `export NAME` | Encodes a film (or PNG sequence/GIF) | `ffmpeg` for H.264/H.265/VP9/ProRes |
| `studio [NAME]` | Opens the [Studio](studio.md) workbench for one timeline | `studio` |
| `validate NAME` | Builds and checks a film without rendering it; takes only `--params`/`--params-file` | none |
| `schema` | Prints the JSON Schema of declarative documents; takes no options | `declarative` |

No arguments, `--help`, `-h` or the `help` subcommand print usage and
exit 0 without loading content; `COMMAND --help` prints that command's flags. `still` and `export` require a positional `NAME`
(first argument after the command, not starting with `-`). `studio` takes `NAME`
or `--timeline NAME`, never both; `--timeline` exists only on `studio`.
Without the `studio` feature, `studio` fails with an error naming the feature
(after rejecting invalid options). Without `ffmpeg`, the FFmpeg encoder fails with
"export requires feature ffmpeg"; the PNG-sequence and GIF encoders are separate
code and do not use it. Default features enable both (`crates/motion/Cargo.toml`).

## Flags

Flags take a following value except `--overwrite`, `--alpha` and
`--allow-software-gpu`. Errors come from clap: a flag given twice, an unknown flag, a missing or invalid value
and conflicting flags (`--time` with `--frame`, `--params` with `--params-file`) fail
with a usage message. Flags a command does not define, such as `--range` on `still`,
are unknown to it. Programs with their own options embed `ProjectCommand` in a clap
parser and call `Project::execute`, as the gallery does.

| Flag | Applies to | Meaning |
| --- | --- | --- |
| `--params JSON` | all | Inline parameters; text over 1 MiB is rejected |
| `--params-file PATH` | all | Parameters from a regular file of at most 1 MiB |
| `--size WIDTHxHEIGHT` | all | Output pixel size (two `u32`, split on `x`) |
| `--backend cpu\|gpu` | all | Backend preference |
| `--allow-software-gpu` | all | Admits software adapters for `Gpu` in `RenderOptions::backend`; not in help text |
| `--output PATH` | still, export | Destination; default `NAME.png` or `NAME.EXT` |
| `--overwrite` | still, export | Replace an existing destination |
| `--time T` | still | Time of the frame; default `0` |
| `--frame N` | still | Frame index (`i64`, nonnegative), converted with the timeline frame rate |
| `--range A:B` or `A..B` | export | Half-open time range; requires `A < B` |
| `--codec NAME` | export | Default `h264` |
| `--quality N` | export | `u8`; default 18 |
| `--alpha` | export | Request alpha (VP9 only) |
| `--timeline NAME` | studio | Selection alternative to the positional name |

`--params` and `--params-file` are mutually exclusive. Combining `--time` with
`--frame` fails; `export` rejects `--time` and `--frame` (use `--range`); `still`
rejects `--range`, `--codec`, `--quality` and `--alpha`. `studio` accepts only
parameters, `--size`, `--backend`, `--timeline` and `--allow-software-gpu`; any
output/time/range/overwrite/frame/codec/quality/alpha flag is an error.

### Time syntax

`parse_time` accepts a decimal number with an optional suffix: `1.5`, `1.5s`
(seconds) or `1500ms`. The value must be finite and within `0..=86400` seconds.
`--range` splits on the first `..` or else the first `:`, so `2s..6s`, `1:3` and
`500ms..2s` all work.

### Codec names

`parse_codec` maps names to `Codec`; `--quality` is rejected for codecs without a
quality setting.

| Name | Codec | Default extension | Quality | Alpha |
| --- | --- | --- | --- | --- |
| `h264` | `Codec::H264` (Medium preset) | `.mp4` | yes | rejected |
| `h265`, `hevc` | `Codec::H265` (Medium preset, 8-bit) | `.mp4` | yes | rejected |
| `vp9` | `Codec::Vp9` | `.webm` | yes | `--alpha` |
| `prores`, `prores4444` | `Codec::ProRes4444` | `.mov` | rejected | no flag needed |
| `png`, `png-sequence` | `Codec::PngSequence` | `.frames` (a directory) | rejected | no flag needed |
| `gif` | `Codec::Gif` (256 colors, timeline frame rate) | `.gif` | rejected | no flag needed |

Other names fail with `Unknown codec`. The built codec is also run through
`Codec::validate`. Help lists only `h264|h265|vp9|prores4444|png|gif`; the aliases
above are accepted but undocumented there.

## Output Paths and Overwrite

- A still defaults to `NAME.png` and an export to `NAME.EXT` in the current
  directory, as in the table above. For a document path, `NAME` is the file stem. Paths are not otherwise normalized.
- **The output directory must already exist.** Both commands stage a temporary
  file beside the destination; a missing directory fails with
  `No such file or directory ... .motion-still-*.png` (or `.motion-video-*.mp4`).
  The CLI never creates directories.
- Without `--overwrite`, an existing destination fails: `still` checks first and
  reports `Still destination exists`; `export` reports `Export destination
  exists`. With it, the destination is replaced after the new output is complete.
- Both commit a staged file; see the transaction rules in
  [reliability](design/reliability.md) and [rendering](design/rendering.md).
- Success prints the destination path on stdout (the still path, or
  `ExportReport::path`). Encoder libraries may also log to stderr.
- Stills render with the project render options (transparent unless the project
  sets a background). Exports default the background to black when none is set.

## Parameters

Parameters are admitted before a builder runs ([`project.rs`](../crates/motion/src/project.rs)):

| Rule | Failure |
| --- | --- |
| Plain `timeline`/`try_timeline` takes none | `Timeline does not accept parameters` |
| `P::default()` must serialize to a JSON object of at most 1 MiB | registration error |
| Supplied JSON must be an object | `Parameters must be a JSON object` |
| Keys must exist in the serialized defaults | `Unknown parameter: KEY` |
| Merged value must deserialize into `P` | `invalid parameters:` serde message |
| Serialized parameters over 1 MiB | `Parameters exceed 1 MiB` |

The merge is **top-level only**: each supplied key replaces the default value
wholesale, including nested objects. `list` shows `defaults` as the discovery
surface (`null` for non-parameterized timelines). The built timeline is validated
before use. Defaults and deserializers should be deterministic and side-effect
free. The gallery's `DemoParameters` (`seconds`, `seed`, `count`) is an example.

## Declarative Documents

With feature `declarative` (a facade default), `still`, `export` and `studio` accept
a path ending in `.json` wherever a timeline name is expected, when no timeline is
registered under that name. The document is read and compiled for the command, its
relative resources resolve against its own directory, and its look's display stage
and fonts are applied to the render options. `--params` overrides the parameters the
document declares in `params`; unknown names fail, and `list` shows no defaults because
listing never reads files. `schema` prints the
format's JSON Schema. The `motion` program in
[`crates/motion-cli`](../crates/motion-cli) parses its arguments with clap and takes a
document path directly (`validate`, `still`, `export`, `studio`, `schema`; `--help` per
subcommand):

```sh
cargo run -p motion-cli -- still examples/documents/intro.json --time 1.5
cargo run -p motion-cli -- schema > examples/documents/motion.schema.json
```

Document errors print the failing JSON path, the field and, for loaded files, the
line and column, for example `tracks[0].items[0].scene.content.fill: Geometry requires
a fill or a stroke (line 4, column 5)`. Misspelled keys, enum names, parameters,
element ids, bus names and expression functions add `Did you mean `fill`?` when a close
name exists. See
[declarative documents](design/documents.md) for the format contract and the
[tutorial chapter](tutorial/16-declarative.md) for authoring.

### Validate

`validate NAME` (also `Project::check`) builds the timeline or document, validates it,
requires every file a document refers to, and then builds and preflights the first,
middle and last frames against the configured resources and limits. It renders and
encodes nothing and opens no device, so it is suitable for CI and editors. Success prints
one line such as `intro.json: valid (640x360, 30 fps, 132 frames, 4.4 s, 3 frames
preflighted, 1 files)`; any failure prints its diagnostic and exits with `1`. Frames
between the checked ones are not evaluated.

### Studio and documents

`studio NAME` on a `.json` document or a timeline registered with `Project::document`
opens the document reloadably: saving it, or a file it refers to, reloads the scene in
place. See [Studio hot reload](studio.md#hot-reload).

## Exit Codes and Errors

Exit code is `0` on success (including help) and `1` on any error; there are no
finer-grained codes. Argument and parameter faults print as
`invalid parameters: MESSAGE` (`Error::Parameters`); I/O, render and encoder errors
print their own `Display` text. Inspect `Error` kinds rather than messages when
using `try_run` programmatically ([reliability](design/reliability.md)).

## Gallery and Skill Consumers

The gallery wraps the same CLI ([`gallery/src/lib.rs`](../gallery/src/lib.rs)):

- No arguments run `studio` when its `studio` feature is on, else `list`.
- `--font PATH` is removed before parsing and loads only that font file; otherwise
  system fonts are loaded for every command except `list`.
- `studio --demo NAME` selects one demo (once, valid names only), conflicting with a
  positional name or `--timeline`; bare `studio` selects the first catalog entry.

Automation can drive `still` and `export` of a film binary with `--size`, `--time`,
`--range`, `--output` and `--overwrite`, and rely on exit code `0`. `VisualCheckPolicy` in
[`visual_check.rs`](../crates/motion/src/visual_check.rs) is optional registration metadata:
attach it with `ProjectBuilder::visual_check` and read it with `Project::visual_check`. It is exported
from the crate root but not the prelude, and the CLI, rendering and export never read it.

## Related

- [Studio guide](studio.md)
- [Export tutorial](tutorial/11-export.md)
- [Rendering and export contracts](design/rendering.md)
- [Reliability](design/reliability.md)
- [Development checks](development.md)
- [`cli.rs`](../crates/motion/src/project/cli.rs), [`args.rs`](../crates/motion/src/project/args.rs)
