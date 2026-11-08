# 12 Studio

## What This Chapter Covers

Inspect one compiled scene in a desktop workbench and diagnose preview, playback, audio and export issues.
This requires `studio` and a native desktop environment; the feature includes `ffmpeg`. Scene GPU rendering also requires `gpu`, but CPU scenes can display in the window too.

## Minimal Example

```rust,no_run
use motion::prelude::*;
use motion::render::Composition;

fn main() -> motion::Result {
    let timeline = Timeline::new(Size::new(640.0, 360.0), FrameRate::NTSC).track(|track| {
        track.scene(4.seconds(), |context| {
            RoundedRect::new(Rect::new(180.0, 100.0, 460.0, 260.0), 12.0)
                .fill(Color::rgb(35, 155, 130))
                .rotate(context.seconds() * 0.6)
        })
    });
    motion::studio::run(motion::studio::StudioScene::timeline(
        "tutorial",
        Composition::new(640, 360),
        timeline,
    )?)
}
```

Running opens one scene, not a scene catalog. Rust edits require recompilation and there is no parameter editing. A declarative document opened with `studio film.json` is the exception: saving the document, or a file it uses, reloads the scene in place and keeps the playhead (see [Hot reload](../studio.md#hot-reload)).

Open the corresponding repository entry directly:

```sh
cargo run --release --features studio --example studio
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo chromatic-tide
```

## Workbench Areas

| Area | Inspect or Control |
| --- | --- |
| Header / Viewer | Backend, render scale, Mute, theme, zoom, pan |
| Timeline | Clips, transition overlaps, playhead, In/Out, cues |
| Export | Output formats, dimensions, range, queue, cancellation |
| Node | Occurrence paths, actual layout, z-order, transforms, local clips |
| Diagnostics | Resource, inspection and execution errors; select entries to seek to their frames |
| Stats | Worker latency, presented/target fps, cache, skipped frames, logical work/peak |

To inspect a segment, pause and seek first, enable Bounds or Pixels, then examine diagnostics and statistics. Bounds provides approximate visual-bounds picking, not exact raster coverage.

## Render Scale and Viewer Zoom

25/50/100% render scale changes actual output pixels; Fit/50/100/200% Viewer zoom changes display size. Enlarging a low-resolution preview does not produce higher-quality output.

Backgrounds, checkerboards, safe areas and grids are Viewer presentation, not changes to the final film. Normal previews retain transparent frames, so background changes do not rebuild the renderer.

Playback and timeline dragging use Draft. Dragging renders at 25%; release restores the selected scale before requesting Final after 150 ms idle. Draft explicitly approximates flattening, shutter sampling and blur. Final output still uses Final.

## GPU, Pixels and A/B

Normal native GPU preview registers completed textures with the same-device UI, avoiding full-frame CPU readback and upload.
The texture ring reserves slots before dispatching workers. When no slot is available, dispatch pauses rather than silently switching to a CPU-pixel path.

Pixels explicitly requests memory output and readback. It inspects sRGB8 and decoded SDR linear values, not the unclipped HDR working buffer. After capturing an A/B reference, disable Pixels if desired; retaining the reference alone does not force continued CPU readback of current frames.

## Real-Time Playback Is Not Per-Frame Acceptance

Play starts a clock without preparing the entire film. When rendering falls behind, targets advance and obsolete results are not published. Seeking pauses playback and invalidates old generations. Executing callbacks can finish but cannot replace results for newer requests.

Target fps, recently presented fps, cache hits, skipped frames and worker time are separate metrics. Smooth cached playback does not prove the same cold-rendering fps.

Automatic samples CPU and GPU completion latency for the current scene and settings. It is a runtime heuristic affected by order, first-time setup and shared caches, not a fair AB/BA benchmark. Explicit GPU failures retain typed errors.

## Caches and Sound

The preview LRU retains 256 MiB of logical payload by default. Set `MOTION_STUDIO_CACHE_MIB=0..1024` to adjust it; 0 disables retention and neighbor prefetch. This is not an RSS or total GPU-memory limit.

Thumbnails use a separate smaller CPU/Draft cache, neither occupying main GPU texture slots nor replacing the main preview. Renderer coverage, layout and material caches are separate from the full-frame LRU.

With sound, audible samples drive the visual clock. Preparation holds the current image; underruns, device or timestamp failures pause rather than silently switching to wall time. Empty mixes open no device, and Mute only affects listening.

## Export and Sessions

Set dimensions, In/Out, backend, shutter and codec before queuing. Tasks retain their own parameter snapshots and run serially. Preview frame skipping does not affect per-frame export requests.

Cancellation and window closing use cooperative cancellation and wait for cleanup. Existing files are replaced only after successful finalization with overwrite permission. Preview and export are separate tasks; preview success does not prove audio encoding or final commit success.

Session settings live in `.motion/studio.json` without automatic replay or export. Audio configuration does not embed media files or form an executable project.

## Common Pitfalls

- Inferring Release performance from Debug playback latency.
- Mistaking Pixels-induced CPU readback for normal GPU preview.
- Treating target fps as presented fps or cache hits as newly rendered frames.
- Treating Mute as removal of the export audio track.
- Assuming that opening a window validates audio devices, drivers, display scaling and recovery on every platform.

## Next Steps

Next chapter: [13 Performance and Reliability](13-performance-and-reliability.md).
Full workflows and limitations: [Studio](../studio.md), [Studio example](../../examples/studio.rs).
Films: [Gallery guide](appendix-gallery.md).