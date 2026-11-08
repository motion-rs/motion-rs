# motion-encode

Transactional encoding and stateless audio preparation for motion-rs: PCM preparation, mixing,
effects, analysis, synthesis and export. Native video encoding is behind the `ffmpeg` feature,
which needs the FFmpeg development libraries described in
[motion-ffmpeg](https://crates.io/crates/motion-ffmpeg).

This crate is part of [motion-rs](https://github.com/motion-rs/motion-rs). Most applications
should depend on the `motion-rs` facade crate instead, which assembles the public authoring API.
