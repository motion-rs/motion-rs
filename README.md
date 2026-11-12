# motion-rs

Programmable motion graphics and deterministic video rendering in Rust.
The Cargo package is `motion-rs`; the library import is `motion`.

The pipeline is `SceneContext -> Scene -> Node -> DisplayList ->
ValidatedDisplayList -> PreparedCpuFrame -> CPU/GPU -> FrameBuffer/Encoder`. The headless core has
no browser, widget tree, playback clock or implicit font discovery. Optional
Motion Studio provides a desktop preview/export workbench.

## Quick Start

Requires a Rust toolchain supporting edition 2024. The default feature set includes
GPU support, content, native FFmpeg and Studio. Geometry-only headless builds use
`--no-default-features` and do not require FFmpeg, fonts, a GPU or a window.

```rust,no_run
use motion::prelude::*;

let timeline = Timeline::new(Size::new(640.0, 360.0), fps!(30))
    .track(|track| track.push(scene(|context| {
    let horizontal = context.sample(&Tween::new(32.0, 480.0, 3.seconds()));
    Node::from(Rect::new(horizontal, 64.0, horizontal + 96.0, 88.0)
        .fill(Color::rgb(20, 150, 125)))
    }).with_duration(3.seconds()))).build()?;
let bitmap = render_still(&timeline, 1500.milliseconds().after_origin(), &RenderOptions::default())?;
bitmap.save_png("frame.png")?;
# Ok::<(), motion::Error>(())
```

`Rect::new` takes opposite corners, not origin/width/height. Geometry and
animation progress use f64; time uses integer ticks (`Time` signed, `Duration` nonnegative) and rational `FrameRate`.
Colors retain finite extended sRGB channels; rendering uses linear-premultiplied
working storage. Straight RGBA8 `FrameBuffer` preserves transparency. `Composition` and its opaque
frame output remain explicit low-level rendering APIs, outside the authoring prelude.

```sh
cargo run --example motion -- list
cargo run --no-default-features --example motion -- list
cargo run --example motion -- still motion --time 1 --output motion.png
cargo run -p motion-cli -- still examples/documents/intro.json --time 1
cargo run --features ffmpeg --example motion -- export motion --output motion.mp4
cargo run -p motion-cli -- still examples/documents/intro.json --time 1
cargo run -p motion-cli -- validate examples/documents/intro.json
cargo run -p motion-gallery --bin motion-gallery -- list
cargo run -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo type-in-motion
```

The independent [Gallery](gallery/README.md) contains short films combining
multiple features. The short
[motion example](examples/motion.rs) uses Project/Timeline; it is not a second
application framework. Existing destinations are protected by export.

## Features

| Feature | Capability |
| --- | --- |
| Default | `gpu,image,svg,text,particles,ffmpeg,studio,ink,diagram,captions,declarative`; scene backend selection remains explicit |
| `vector` | Vector trees, Morph and Ink without enabling content image decoding or fonts |
| `image` | Static/animated image decoding and image content |
| `svg` | Static SVG vector import; includes explicit text/image preparation |
| `text` | Explicit font environments, shaping and paragraph content |
| `particles` | Deterministic simulation, trajectories and instance content |
| `ink` | Stylized vector strokes and textured fills; includes image support |
| `diagram` | Charts, scales, callouts and rulers; includes text support |
| `captions` | Timed caption tracks and cue sheets; includes text support |
| `declarative` | Versioned JSON film documents, their JSON Schema and CLI loading of `.json` paths |
| `audio` | PCM decoding, resampling, mixing and analysis without a playback device |
| `gpu` | Native wgpu composition and external texture output; the default backend, with no CPU fallback |
| `typst` | Trusted single-page markup to outlined SVG/Node |
| `formula` | Direct Typst math vectors, selection, writing and morphing |
| `ffmpeg` | Native H.264/AAC video encoding and transactional output |
| `studio` | egui workbench, workers, audio preview and export; enables FFmpeg |

FFmpeg requires development headers/libraries with libx264/AAC, pkg-config and
libclang for the first-party bindings in [crates/motion-ffmpeg](crates/motion-ffmpeg).
No ffmpeg/ffprobe executable or third-party FFmpeg wrapper is used. On macOS,
the prerequisites can be installed with `brew install ffmpeg pkg-config llvm`;
set `LIBCLANG_PATH` to the LLVM library directory when needed. FFmpeg 8.x is the
validated version. Studio needs a desktop.

Default features make GPU and Studio APIs available and select the GPU backend; they
do not open a window. Choose `BackendPreference::Cpu` explicitly for the reference
rasterizer. GPU selection never falls back to the CPU after initialization, an
unsupported path or device loss.

## Guides

- [Tutorial](docs/tutorial/README.md): first still, animation, scenes, content, audio and export
- [Documentation index](docs/README.md): learning, design and maintenance references
- [Design contracts](docs/design/README.md): architecture, rendering, export, audio, resources, GPU and ownership
- [Command line](docs/cli.md): `list`, `still`, `export`, `studio` and `schema` commands and flags
- [Studio](docs/studio.md): preview, playback and export workbench
- [Gallery](gallery/README.md): finished short films

Exact API contracts and executable examples live in [Rustdoc](crates/motion/src/lib.rs).
Generate them with `cargo doc --no-deps --all-features`.

## Development

Implementations live in focused crates; `crates/motion` assembles the public API.
Run ordinary tests with `cargo test --workspace --all-features --lib --bins --tests --examples`
and documentation tests with `cargo test --workspace --all-features --doc`.
Tests needing explicit fonts, a GPU or FFmpeg fixtures are selected separately;
ignored tests are not evidence of runtime support on an untested machine.

See [development checks](docs/development.md) for feature/fixture procedures,
[reliability](docs/design/reliability.md) for validation boundaries and the
[development plan](docs/plan.md) for remaining engineering and feature work.
