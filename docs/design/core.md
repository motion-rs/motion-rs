# Core Authoring Contracts

Core owns time, geometry, scene snapshots, layout and the drawing protocol. It does
not initialize fonts, rendering devices, codecs or playback clocks. See the
[tutorial](../tutorial/README.md) for runnable authoring examples.

## Numeric Boundaries

Authoring geometry, angles, interpolation progress and easing parameters use f64.
Raster integration retains f64 areas before u8 coverage quantization. Color stores
finite extended encoded-sRGB f32 channels; shading and linear-premultiplied working
pixels use f32. Image storage and output formats have independent precision
contracts. There is no blanket f64 promise for every public numeric parameter.

Logical coordinates have a top-left origin, x right and y down. Rect stores
opposite corners and uses half-open containment. Invalid geometry is not silently
normalized. Rotation uses radians; degree and turn literals convert to radians.
The root uniformly contains design dimensions in the output viewport. Stroke and
effect distances follow that mapping; flattening tolerance is in device pixels.

Time (a point on a timeline) and Duration (a span) are distinct types over signed ticks
at 705600000 ticks per second: `Time - Time` is a Duration, `Time ± Duration` is a
Time, and `Time + Time` does not compile. Literals such as `2.seconds()` are
Durations; `Duration::after_origin` and `Time::since_origin` convert explicitly.
FrameRate is rational; convert each frame boundary
independently. Interval tests use ticks, not accumulated floating seconds. Seconds
are a view for animation mathematics, not mutable playback state. Frame indices,
sample indices and time points are distinct inputs even when conversion is exact.

## Stateless Animation

`Signal` samples a value at an explicit elapsed Duration and has no end. `FiniteAnimation`
extends it with a nonnegative `duration()`; only finite animations offer `delay`, `then`,
`repeat` and `ping_pong`, so composition cannot invent an end time for an unbounded
signal. `Interval { start, duration }` is the single definition of clamped progress and
easing over a time range; `SceneContext::progress_in`, `ease_in` and `Tween` use it. Interpolate
accepts f64 progress and permits deliberate extrapolation. Oklab color interpolation
can extrapolate RGB while clamping alpha. Transform interpolation accounts for
translation, rotation, scale and skew rather than interpolating pixels.

Keyframe destinations are absolute local times; holds are relative durations.
The arriving point owns easing. Equal or decreasing times are errors, not sorted
inputs. Built-in easing pins endpoints; Custom clamps its input but preserves its
callback output. Springs use a conservative settling envelope rounded to ticks
(`settle_duration`), not frame-by-frame state; they, like Noise and Oscillator, are
`Signal` only. Path following is a finite animation.

Combinators keep timing independent of seek order and thread scheduling. Finite-only
operations, invalid factors and overflow have checked boundaries, but convenience
methods may panic. A checked input/output contract is not a custom-callback sandbox.
See [animation](../tutorial/03-animation.md) for composition and dynamic-input use.

## Scenes and Snapshots

The canonical Scene returns a Node for a SceneContext. IntoSceneOutput adapters accept
drawables, heterogeneous tuples or supported Result outputs. Failures retain their
original category, source and requested frame, including in hidden or memoized
subtrees. A failed scene is not successful empty artwork.

SceneContext supplies local/global time, rate, clip duration, design bounds,
resource snapshots, keyed randomness and bounded computation caches. Shifting,
embedding and subframe sampling establish explicit clock boundaries. Random keys
and cache keys must cover all result-defining inputs; cache hits do not justify
side effects or mutable frame history.

Nodes are immutable copy-on-write snapshots. Shared leaves and memoized subtrees
avoid rebuilding static content without freezing dynamic callbacks. Siblings draw
in stable z-index order, preserving insertion order on ties. z-index is not global
depth sorting and does not change layout order. Higher timeline tracks compose
above lower ones.

Spatial/compositing modifiers belong to nodes; content builders describe geometry,
style and assets. The default pivot is the measured center. Placement positions
the pivot, while translation offsets existing local coordinates. Group opacity
applies once to the completed subtree, not independently to each overlapping child.

## Layout and Timelines

Row and Column measure transformed children in insertion order. Text uses its actual
baseline; non-text content has an explicit fallback. Relative positioning and
measurement use the same font environment and structural limits as compilation.
Deferred next-to/alignment is not a measurement made against another font snapshot.

Cameras apply an inverse view transform to an ordinary subtree. World/screen
conversion excludes ancestor transforms and requires finite invertible mappings.
Keep fixed HUD content outside the camera. Skeleton hierarchy transforms attach
ordinary Node artwork; they do not introduce a separate 3D engine.

Timeline is built once by TimelineBuilder and never changes afterwards. It owns design
dimensions, rational rate, tracks, a Soundtrack (audio occurrences, buses, master) and
its final duration. Clip boundaries align independently to frame ticks with warnings
in Diagnostics.
Transitions admit two neighboring clips, not arbitrary triple overlaps; both
endpoints keep their local clocks and share global resources. Scene is picture only
(render, duration, validate); Timeline is not a Scene. TrackBuilder::nest places a child
Timeline as a picture-only clip and appends its audio to the parent's Soundtrack. Nested timelines clip
their own viewport before centered contain mapping; child rates do not retime the
parent clock implicitly.

Ordinary builders retain bounded errors that build() reports together.
Explicit try_* methods provide immediate failure. Project registers lazy factories
and typed parameters; listing/help does not build timelines or discover resources.
Preview, stills and export select the same authored timeline.

## Geometry and Recording

Shape describes geometry; styling makes it drawable. Path2D retains curves, fill
rules, trimming, dashes and stroke outlines. Copy-on-write path edits preserve old
snapshots; a later valid edit cannot erase an earlier retained authoring error.
Source paths and generated stroke outlines both undergo structural admission.

Selected curve, stroke and arc operations use kurbo. Shared coverage remains
Motion-owned, not a complete third-party raster backend. Arc-length lookup, curve
splitting and geometric morphing operate before pixel execution; topology,
subpath pairing and winding remain explicit inputs. These dependencies are part
of the numerical environment, not proof of cross-platform byte identity.

Drawable uses the supplied DrawContext and checked Canvas. DeferredContent is a separate
node kind: it only resolves to an ordinary Node at the drawing time. Recording retains the
first error even if a custom callback ignores it. Compilation measures layout,
composes transforms and validates all branches, including invisible content.
It returns ValidatedDisplayList, an immutable checked command snapshot. Editable
DisplayList requires validation after mutation; structural validity does not prove
resource availability, device support or sufficient raster allowance.

## Layers and Sampling

Filters are ordered data over shared primitives, not arbitrary per-backend pixel
callbacks. Child effects complete before parent effects. The completed source is
filtered, masked/clipped, then composited with opacity and blending. Saved-source
operations reference their input explicitly. Layout bounds differ from device
capture bounds expanded for filters.

Coverage clips multiply independently rasterized coverage. Geometric clips
intersect before quantization; another geometric clip on one Node replaces the
previous one, while nesting intersects them. Content masks use alpha or Rec.709
linear-premultiplied luminance. LinearRgb is the default blending space; sRGB
composition is an explicit conversion boundary.

A Node is one snapshot and cannot be resampled as a scene. Motion blur invokes the
scene builder at repeatable subframe times and averages linear-premultiplied
captures. Empty samples retain their weight. Keep intentionally sharp overlays
outside the sampled subtree. Draft approximations are not Final export semantics.

## Owning Sources

- [Time](../../crates/motion-core/src/time.rs)
- [Animation](../../crates/motion-core/src/animation.rs) (`Signal`, `FiniteAnimation`, `Interval`)
- [Authoring](../../crates/motion-core/src/authoring.rs)
- [Paths](../../crates/motion-core/src/path2d.rs)
- [Drawing](../../crates/motion-core/src/drawing.rs)
- [Rendering and output](rendering.md)
- [Validation and failure](reliability.md)
