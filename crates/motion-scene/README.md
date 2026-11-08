# motion-scene

Declarative JSON scene documents for motion-rs timelines: a versioned document format, its JSON
schema and compilation to timelines. Content kinds are enabled through Cargo features that
mirror those of `motion-rs`.

This crate is part of [motion-rs](https://github.com/motion-rs/motion-rs). Most applications
should depend on the `motion-rs` facade crate and enable its `declarative` feature instead.
