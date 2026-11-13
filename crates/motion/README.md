# motion-rs

A programmable 2D video engine in Rust. A scene is a pure function of explicit time, so the same
prepared frame feeds both a CPU backend and a GPU backend and any frame can be rendered, seeked
to or replayed exactly.

```toml
[dependencies]
motion-rs = "0.2"
```

The package is `motion-rs`; the library is imported as `motion`:

```rust,ignore
use motion::prelude::*;
```

## System requirements

The default features include `ffmpeg` (native video export) and `studio` (the desktop
workbench). Building them needs:

- `pkg-config` and the FFmpeg development libraries, found through `pkg-config`;
- libclang, which `bindgen` loads at build time.

See [motion-ffmpeg](https://crates.io/crates/motion-ffmpeg) for the environment variables its
build script reads. Rendering and encoding never run an `ffmpeg` executable.

To build without them, disable the default features and pick what you need:

```toml
[dependencies]
motion-rs = { version = "0.2", default-features = false, features = ["image", "text"] }
```

## Features

Default: `gpu`, `image`, `svg`, `text`, `particles`, `ffmpeg`, `studio`, `ink`, `diagram`,
`captions` and `declarative`. Optional: `audio`, `typst` and `formula` (Typst-based formulas).
The `ffmpeg` feature implies `audio`, and `studio` implies `ffmpeg`. docs.rs documents the build
without the FFmpeg-dependent features.

## Documentation

The tutorial, design notes and command-line guide live in the
[repository](https://github.com/motion-rs/motion-rs/tree/master/docs). API documentation is on
[docs.rs](https://docs.rs/motion-rs).

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
