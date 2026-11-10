# 04 Scenes and Timeline

## What This Chapter Covers

Combine animated values into measurable, layout-aware scenes, then organize a film with tracks. The basic example needs no optional features; text layout requires `text` and an explicit font environment.

## Minimal Example

```rust,no_run
use motion::prelude::*;

fn main() -> motion::Result {
    let first = scene(|context| -> motion::Result<_> {
        let progress = context.sample(&Tween::new(0.0, 1.0, 2.seconds()));
        Ok(Node::group((
            Rect::new(40.0, 40.0, 440.0, 230.0)
                .fill(Color::rgb(20, 80, 70)),
            Circle::new(Point::new(120.0 + 240.0 * progress, 135.0), 24.0)?
                .fill(Color::WHITE)
                .z_index(1),
        ))
        .opacity(0.9))
    });
    let second = scene(|context| -> motion::Result<_> {
        Ok(Circle::new(context.center(), 40.0)?.fill(Color::rgb(210, 80, 105)))
    });
    let timeline = Timeline::new(Size::new(480.0, 270.0), 30).track(|track| {
        track
            .push(first.with_duration(2.seconds()))
            .push(second.with_duration(1.seconds()))
    }).build()?;
    render_still(&timeline, 2500.milliseconds(), &RenderOptions::default())?
        .save_png("tutorial-timeline.png")
}
```

The second scene starts at global time two seconds. At global time 2.5 seconds, its local time is 0.5 seconds.

## scene, SceneContext and Node

`scene` provides context inference for closures. `Scene` describes a scene through `render(&SceneContext) -> Node`; ordinary authoring closures can return shapes, Nodes, heterogeneous tuples or supported `Result` outputs.

`Node` is an immutable snapshot. Modifying a branch after cloning does not force the other branch to change. Shapes, text, images and custom Drawables all participate in the same scene tree.

| API | Purpose |
| --- | --- |
| `Node::group(tuple_or_iterator)` | Combine a heterogeneous tuple or homogeneous iterator |
| `group!` | Concisely convert multiple children into Nodes |
| `Node::push` | Append a child to a copy-on-write group |
| `translate` / `scale` / `rotate` | Local geometric transforms |
| `opacity` / `blend_mode` | Layer opacity and composition |
| `z_index` | Sibling drawing order |
| `clip` / `clip_with` | Coverage or geometric clipping |
| `filter` | Ordered subtree filters |

Siblings draw in stable z-index and insertion order. z-index does not change layout order or provide global depth sorting across ancestors.

Invalid hidden branches still validate; opacity does not provide a culling shortcut.
SceneContext keeps local/global clocks, resource snapshots and cache identity
explicit. `shifted`, `during` and `embed` have distinct clock boundaries, not an
implicit mutable playback cursor.

## Group and Child Opacity Differ

Applying `.opacity(0.5)` to a group draws its children first, then applies opacity to the combined result. Applying `.opacity(0.5)` separately to overlapping children blends them with one another, potentially changing the overlap.

For a whole section fading in together, usually apply opacity to the group. Apply it separately when elements should appear independently.

## Layout

`Row` and `Column` arrange transformed measured bounds in insertion order. Text uses a real baseline; geometry has its own baseline fallback contract.

`context.measure`, `next_to` and `align_to` use a consistent font and drawing environment. Relative layout depends on the reference Node's measurement, not guessed text widths or measurements from another font environment.

Place fixed decoration in `context.memo(key, builder)` to reuse static subtrees while evaluating animated parameters at the current time. Keys must distinguish every result-defining input, including asset versions, fonts, colors and geometry. Eviction or contention can rerun the builder; do not rely on once-only side effects.

## Timeline and Track

`Timeline::new(size, rate)` starts a `TimelineBuilder`; `build()` freezes it into an immutable, validated `Timeline` that holds design dimensions, a rational frame rate, tracks, a `Soundtrack` and `diagnostics()`. Ordinary clips within a track are sequential; different tracks contribute to composition at the same time.

| Operation | Use |
| --- | --- |
| `track.scene(duration, callback)` | A simple fixed-duration scene |
| `track.push(scene.with_duration(duration))` | Reuse an existing scene |
| `track.gap(duration)` | Insert an explicit gap |
| `track.place(start, scene)` | Place a scene at a specific time |
| `track.transition(effect, duration)` | Arrange a transition between adjacent clips |
| `TimelineBuilder::audio` | Attach an audio occurrence |
| `TimelineBuilder::duration` | Set the final film duration explicitly; otherwise it is the end of the tracks and audio |

Transitions allow only two adjacent clips to participate simultaneously; arbitrary triple overlaps do not replace transition design. `CrossFade`, `Slide`, `Push`, `Wipe`, `Iris` and `ZoomBlur` provide different composition methods. Both sides keep their own local clocks during a transition.

Non-`try_` track and builder methods accumulate authoring errors that `TimelineBuilder::build` reports together. Use `try_track`, `try_push` and similar methods for programmatic input requiring immediate failure. Do not ignore retained errors.

Clip boundaries align independently to rational frame ticks and retain alignment
warnings in `Timeline::diagnostics()`; an explicit duration shorter than the content adds a `Truncated` warning and cuts only the picture. The half-open end is not the index of the final rendered frame.
Animated-image source-time selection is separate from timeline placement; trimmed
or reversed views share source frames rather than duplicating endpoint durations.

## Nesting, Cameras, Skeletons and Spaces

`Timeline` is not a `Scene`. Place a child timeline with `track.nest(child)` (or `nest_for(duration, child)` / `place_timeline(start, child)`): its picture clips the child viewport before a centered contain mapping, and its audio joins the parent's soundtrack once, at build time. Inside a scene, `context.embed(&child.visual(), offset)` embeds the picture only, so dropping the sound is visible at the call site. A different child frame rate does not implicitly reset the parent clock or automatically retime all content.

`camera(context, content, view)` applies an inverse view transform. Keep HUD content outside the camera node and use `ClipMode::Geometric` for a fixed viewport. `Camera` world/screen conversion excludes external ancestor transforms and requires a finite, invertible mapping.

`Skeleton`, `Bone`, `BonePose`, `Pose` and `BoneTransform` connect local channels to ordinary Node artwork through hierarchical f64 transforms. This is 2D hierarchical animation, not model3d. Define local bone coordinates before mapping animated channels to artwork; do not apply the same transform to both artwork and bones.

## The Project Boundary

`Project` registers film timelines and CLI entry points; it does not replace `Timeline`. Export, stills and Studio select the same film definition, keeping preview and export aligned.

## Common Pitfalls

- Using global time as local time misaligns animation when a clip moves.
- Hiding invalid nodes does not avoid validation; invisible branches are checked too.
- z-index controls sibling drawing order, not layout order.
- A fixed HUD inside a camera moves, scales or clips with the view.
- Authoring caches should retain pure results, not act as arbitrary side-effect executors.

## Next Steps

Next chapter: [05 Drawing](05-drawing.md).
Further reading: [scene guide](../design/core.md#scenes-and-snapshots), [timeline implementation](../../crates/motion-core/src/authoring/timeline.rs), [Project example](../../examples/motion.rs).