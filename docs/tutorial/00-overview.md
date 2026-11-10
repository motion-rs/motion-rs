# 00 Overview and Pipeline

## What This Chapter Covers

Decide whether motion-rs fits your project and distinguish authoring, rendering and output. This chapter's example needs no optional features.

motion-rs is a programmable motion graphics library for Rust. Scene functions describe images at specific times, timelines combine content, and output produces images or video. It is not a browser animation framework and requires neither a window nor a playback clock.

## Minimal Example

```rust,no_run
use motion::prelude::*;
use motion::render::{DisplayList, ValidatedDisplayList, Bitmap, FrameBuffer, PreparedFrame};
use motion::extend::{Encoder};
use motion::vector::{Morph};
use motion::particles::{Particle};

fn main() -> motion::Result {
    let timeline = Timeline::new(Size::new(320.0, 180.0), 30).track(|track| {
        track.scene(1.seconds(), |_| {
            Circle::new(Point::new(160.0, 90.0), 24.0)
                .fill(Color::rgb(20, 150, 125))
        })
    }).build()?;
    let bitmap = render_still(&timeline, Time::ZERO, &RenderOptions::default())?;
    bitmap.save_png("tutorial-overview.png")
}
```

This program has no animation clock. `render_still` requests time zero, and the scene computes only the image needed at that time.

## Pipeline

From the application author's perspective:

```text
SceneContext -> scene function -> Node
    -> record and validate drawing commands -> ValidatedDisplayList
    -> PreparedFrame -> CPU/GPU -> Bitmap/FrameBuffer/texture/Encoder
```

`DisplayList` is an editable low-level command container. Compiling an ordinary `Node` directly produces a `ValidatedDisplayList`; applications need not construct every intermediate object manually.

| Object | Responsibility | Does Not Imply |
| --- | --- | --- |
| `SceneContext` | Local/global time, logical size, resources, randomness, authoring cache | A playback clock or GPU command queue |
| `Node` | Scene structure, transforms, layers, content | Already-rendered pixels |
| `ValidatedDisplayList` | Immutable, structurally validated drawing commands | GPU availability or sufficient budget |
| `PreparedFrame` | Device-space coverage, paint sources, clip and filter data | A task already submitted to the GPU |
| `Bitmap` | Transparent image output | A video encoder or opaque window frame |
| `FrameBuffer` | Frame data with an explicit pixel format | Identical memory layouts for all formats |
| `Encoder` | Consume ordered frames and complete the output transaction | Scene construction or user interface |

## Design Principles

### Evaluate by Time

Treat an image as a function of time, parameters, assets and seeds. Seeking to frame 100 should not require playing the preceding 99 frames. Particle simulations may have internal state, but their explicit simulation and sampling contracts must preserve replayability.

### Explicit Environments

The geometry path needs no fonts, GPU or window. Text and SVG require an identifiable font environment; deterministic projects should register fixed font files or bytes. Resource loading needs an explicit root, not arbitrary relative reads treated as global defaults.

### Determinism Has a Scope

CPU evaluation of identical inputs and GPU replay on the same device have their own guarantees. CPU/GPU pixel tolerances and differences between devices, fonts or toolchains are separate concerns. An identical random seed does not imply byte-identical output across platforms.

### Keep Output Boundaries Distinct

GPU working images and Studio previews can remain on the GPU. Returning a `Bitmap` or CPU-readable frame involves a memory-output boundary; encoding also has format, synchronization and lifetime requirements. Using the GPU does not eliminate all CPU work from the application.

## Feature Selection

| Feature | Capabilities and Dependencies |
| --- | --- |
| No optional features | Geometry, timelines, CPU drawing, basic output |
| `vector` | Vector content and Morph without enabling image decoding or fonts |
| `image` | Static/animated images; includes `vector` |
| `text` | Text, fonts, paragraphs; includes `image` |
| `svg` | SVG import; includes `text`, `image` |
| `particles` | Trajectories, simulations, instances; includes `image` |
| `ink` | Brush strokes and textured fills; includes `image` |
| `diagram` | Charts, ticks, annotations; includes `text` |
| `captions` | Subtitles and cues; includes `text` |
| `audio` | Decoding, mixing, analysis; no playback device required |
| `gpu` | wgpu composition and external texture output; does not select a backend |
| `typst` | Trusted single-page Typst typesetting |
| `formula` | Direct math formulas, selection, writing, Morph; includes `typst` |
| `ffmpeg` | Native video/audio encoding; includes `audio` |
| `studio` | Desktop workbench; includes `ffmpeg` |

The current defaults enable `gpu,image,svg,text,particles,ffmpeg,studio,ink,diagram,captions`.
This is a convenient full-authoring profile, not a minimal dependency set. Enable `formula` and `typst` separately.

## Common Pitfalls

- Enabling `gpu` does not select GPU rendering. Choose the backend through render configuration.
- Structural validation does not satisfy every execution condition. Resources, budgets, devices and encoders can still fail later.
- Reading system time or unseeded randomness per frame breaks seeking and replay.
- Ordinary 2D scenes are not a 3D engine. model3d import and rendering are not currently provided.
- Do not recreate the entire environment for every frame. Share fixed assets and fonts, and reuse rendering state through sessions.

## Next Steps

Next chapter: [01 Quick Start](01-quick-start.md).
Contracts and source entry points: [architecture](../design/architecture.md), [package boundaries](../design/architecture.md#packages-and-features), [minimal Project example](../../examples/motion.rs).