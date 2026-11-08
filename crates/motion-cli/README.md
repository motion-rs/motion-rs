# motion-cli

The `motion` program: loads declarative JSON documents and validates, renders, exports or
previews them.

```sh
cargo install motion-cli
motion validate intro.json
motion still intro.json --time 1.5 --output frame.png
```

It is built on `motion-rs` with default features, which include native video export and the
Studio workbench. Building it needs `pkg-config`, the FFmpeg development libraries and libclang;
see [motion-ffmpeg](https://crates.io/crates/motion-ffmpeg). See the
[command-line guide](https://github.com/motion-rs/motion-rs/blob/master/docs/cli.md) for every
command and flag.
