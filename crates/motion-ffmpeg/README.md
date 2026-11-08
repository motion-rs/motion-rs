# motion-ffmpeg

Native FFmpeg bindings with thread-confined codec and mixer resources, used by motion-rs for
video export. It links FFmpeg through its libraries and never runs an `ffmpeg` executable.

## Requirements

Building this crate needs:

- `pkg-config` and the FFmpeg development libraries, found through `pkg-config`;
- libclang, which `bindgen` loads at build time (set `LIBCLANG_PATH` if it is not found).

The build script reads `PKG_CONFIG_PATH`, `PKG_CONFIG_LIBDIR`, `PKG_CONFIG_SYSROOT_DIR`,
`LIBCLANG_PATH` and `BINDGEN_EXTRA_CLANG_ARGS`. Without the libraries the build fails.
docs.rs cannot build this crate for the same reason.

This crate is part of [motion-rs](https://github.com/motion-rs/motion-rs). Most applications
should depend on the `motion-rs` facade crate instead.
