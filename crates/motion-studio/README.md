# motion-studio

Native desktop preview, playback and export workbench for motion-rs. Studio previews one
compiled scene, with seeking, playback, audio and export.

It depends on `motion-ffmpeg`, so building it needs `pkg-config`, the FFmpeg development
libraries and libclang; see [motion-ffmpeg](https://crates.io/crates/motion-ffmpeg). docs.rs
cannot build this crate for that reason.

This crate is part of [motion-rs](https://github.com/motion-rs/motion-rs). Most applications
should depend on the `motion-rs` facade crate and enable its `studio` feature instead.
