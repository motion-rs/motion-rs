# 01 Quick Start

## What This Chapter Covers

Set up a minimal environment, generate your first PNG, then use the same scene for video, Gallery and Studio.
The first still needs no optional features. MP4 requires `ffmpeg`; the workbench requires `studio`.

## Environment and Dependencies

Use a toolchain supporting Rust edition 2024. Reference the checked-out facade crate in a standalone application's Cargo configuration:

```toml
[package]
name = "motion-tutorial"
version = "0.1.0"
edition = "2024"

[dependencies]
motion-rs = { path = "/absolute/path/to/motion-rs/crates/motion", default-features = false }
```

Replace `path` with your checkout location, not the workspace root.
Later chapters add dependency options such as `features = ["text"]` as needed.

Try the minimal example from the motion-rs repository:

```sh
cargo run --no-default-features --example motion -- list
cargo run --no-default-features --example motion -- still motion --time 1 --output tutorial-first.png
```

## Your First Still

This is a complete standalone application:

```rust,no_run
use motion::prelude::*;
use motion::render::{Bitmap};

fn main() -> motion::Result {
    let timeline = Timeline::new(Size::new(640.0, 360.0), 30).track(|track| {
        track.scene(3.seconds(), |context| {
            let horizontal = context.sample(&Tween::new(80.0, 560.0, 3.seconds()));
            (
                Rect::new(0.0, 0.0, 640.0, 360.0).fill(Color::rgb(245, 248, 246)),
                Circle::new(Point::new(horizontal, 180.0), 32.0)
                    .fill(Color::rgb(20, 160, 130)),
            )
        })
    });
    let bitmap = render_still(
        &timeline,
        1500.milliseconds(),
        &RenderOptions::default(),
    )?;
    bitmap.save_png("tutorial-first.png")
}
```

The background is an ordinary rectangle. Without a background, the image retains transparent regions.
`1500.milliseconds()` is a time point, not a frame index. At 30 fps it falls on frame 45's boundary.

## Use the Same Scene Through the CLI

Register the timeline with `Project` instead of writing argument parsing or a playback loop:

```rust,no_run
use motion::prelude::*;

fn main() -> std::process::ExitCode {
    Project::new("tutorial")
        .timeline("intro", || {
            Timeline::new(Size::new(640.0, 360.0), 30).track(|track| {
                track.scene(3.seconds(), |context| {
                    let horizontal =
                        context.sample(&Tween::new(80.0, 560.0, 3.seconds()));
                    Circle::new(Point::new(horizontal, 180.0), 32.0)
                        .fill(Color::rgb(20, 160, 130))
                })
            })
        })
        .run()
}
```

Run these commands in the standalone project:

```sh
cargo run -- list
cargo run -- still intro --time 1.5 --output tutorial-cli.png
cargo run -- --help
```

Every command and flag is listed in the [command-line reference](../cli.md).

`Project::run` returns `ExitCode`, reporting argument or export failures through the CLI exit status. Do not use a `motion::Result` return type for `main` when calling it.

## Your First Video

Enable `ffmpeg` in the dependency. It uses native FFmpeg development libraries, not an external `ffmpeg` command to assemble frames.

On macOS:

```sh
brew install ffmpeg pkg-config llvm
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"
```

The libraries require available libx264/AAC support, pkg-config and libclang. FFmpeg 8.x has been verified.
Linux also needs the corresponding development headers and libraries. Installing a command-line player alone does not ensure the bindings can build.

Use the repository example:

```sh
cargo run --release --no-default-features --features ffmpeg --example motion -- export motion --output tutorial-motion.mp4
```

For the standalone Project application, use `export intro`. Video and stills share the timeline; only the output workflow changes.
Export protects existing destinations by default. Check the current CLI's `--help` and explicitly choose overwrite when needed.

## Gallery and Studio

```sh
cargo run -p motion-gallery --bin motion-gallery -- list
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo type-in-motion
```

Gallery is a separate package with its own feature selection. `--features studio` does not mean every scene has selected GPU rendering.
Studio opens one scene at a time. See [12 Studio](12-studio.md) for preview and export workflows.

## API Reference

| API | Purpose |
| --- | --- |
| `Timeline::new(size, rate)` | Create a timeline with explicit logical size and frame rate |
| `track.scene(duration, callback)` | Add a scene to a track |
| `context.sample(&animation)` | Sample at the current scene-local time |
| `render_still` | Request a transparent image at one time point |
| `Bitmap::save_png` | Save a PNG |
| `Project::timeline` / `run` | Register a film and provide a CLI |

## Common Pitfalls

- `Rect::new(left, top, right, bottom)` is not `(x, y, width, height)`.
- Default features include native dependencies. Use `default-features = false` when learning geometry alone.
- Debug performance does not represent final export performance. Use `--release` for films, but still measure the actual scene.
- Font, audio and image paths do not trigger asset downloads. The tutorial explicitly lists required files.
- A still does not require Studio or a manually created GPU. Headless output works independently.

## Next Steps

Next chapter: [02 Foundations](02-foundations.md).
Runnable repository programs: [motion example](../../examples/motion.rs), [audio export example](../../examples/audio_export.rs), [Gallery guide](../../gallery/README.md).