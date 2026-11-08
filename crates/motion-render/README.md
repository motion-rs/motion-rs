# motion-render

Shared bounded raster preparation and CPU/GPU rendering for motion-rs: coverage, composition,
filters, render sessions and output buffers. The optional `gpu` feature enables the GPU backend.

This crate is part of [motion-rs](https://github.com/motion-rs/motion-rs). Most applications
should depend on the `motion-rs` facade crate instead, which assembles the public authoring API.
