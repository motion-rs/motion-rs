# 02 Foundations

## What This Chapter Covers

Distinguish time, logical coordinates, color and pixels so that scenes do not depend on one frame rate or resolution. This chapter needs no optional features.

## Minimal Example

```rust,no_run
use motion::prelude::*;

fn main() -> motion::Result {
    let timeline = Timeline::new(Size::new(480.0, 270.0), FrameRate::NTSC)
        .track(|track| {
            track.scene(2.seconds(), |context| {
                let horizontal = 48.0 + context.random("marker-position") * 300.0;
                let mut path = Path2D::new();
                path.move_to(Point::new(48.0, 200.0))
                    .line_to(Point::new(240.0, 48.0))
                    .line_to(Point::new(432.0, 200.0))
                    .close();
                (
                    path.fill(Color::from_hex(0x168C78)),
                    Circle::new(Point::new(horizontal, 210.0), 8.0)
                        .fill(Color::WHITE),
                )
            })
        });
    let time = Time::from_frame(30, FrameRate::NTSC);
    render_still(&timeline, time, &RenderOptions::default())?
        .save_png("tutorial-foundations.png")
}
```

The random marker's position is determined by its key. Repeated evaluation and backward seeks do not move it. Time is calculated from the rational frame rate rather than rounding it to 30 first.

## Time Is Not an Accumulator of Floating-Point Seconds

`Time` and `Duration` use signed ticks, with `705600000` ticks per second. Seconds are a convenient view, not internal playback state.

| API | Purpose |
| --- | --- |
| `1.seconds()` / `250.milliseconds()` | Construct tick values from time literals |
| `Time::ZERO` | Time zero |
| `Time::from_frame(index, rate)` | Calculate a frame boundary independently |
| `FrameRate::NTSC` | Rational frame rate of `30000/1001` |
| `context.time()` | Scene-local time |
| `context.seconds()` | Local time in seconds for physics and geometry |

Convert each frame boundary independently instead of repeatedly adding `1.0 / fps`. Timelines and captions use half-open intervals: the start is included and the end is excluded. Negative time is meaningful; do not clamp it to zero at every input boundary.

## Logical Geometry and Device Pixels

`Point`, `Vector2`, `Size`, `Rect` and `Transform` use f64. Scene dimensions describe design coordinates; output pixel dimensions use `PixelSize`. Changing preview scale should not require rewriting design coordinates.

```rust,no_run
use motion::prelude::*;

fn main() {
    let bounds = Rect::new(20.0, 30.0, 120.0, 90.0);
    let center = Point::new(70.0, 60.0);
    let displacement = Vector2::new(12.5, -4.0);
    let _ = (bounds, center, displacement, Transform::IDENTITY);
}
```

The rectangle is 100 wide and 60 high. `Rect::new` takes opposite corners, not width and height. Negative dimensions or invalid geometry are not automatically normalized just because they resemble a rectangle.

The `Shape` abstraction provides bounds and paths. Shapes such as `Circle`, `Rect`, `Polygon` and `Path2D` become drawable content through `.fill` or `.stroke` and can then be placed in a `Node`.

## Color and Alpha

| API | Meaning |
| --- | --- |
| `Color::rgb(20, 150, 125)` | 8-bit RGB input |
| `Color::from_hex(0x168C78)` | Hexadecimal RGB |
| `Color::from_rgb_float(1.5, 0.4, 0.1)` | Finite extended-sRGB input, not limited to the display range |
| `color.with_alpha(0.5)` | Set alpha |
| `to_rgba8` | Clamp and quantize to display bytes; not HDR tone mapping |
| `to_linear_premultiplied` | Convert to the linear-premultiplied drawing representation |

`Color` accepts f64 input and retains finite f32 channels internally. Working images use linear-premultiplied color. Do not manually multiply ordinary drawing colors by alpha before the engine premultiplies them again.

Extended sRGB can represent finite values brighter than ordinary white. This does not mean PNG or ordinary video can preserve them, or that HDR display mapping happens automatically.

Oklab interpolation can extrapolate RGB while clamping alpha. Geometry retains signed
overshoot rather than converting values to integer pixels during authoring. Rect
containment is half-open.

The public `srgb_to_linear`, `linear_to_srgb`, `rec709_luminance`, `premultiply`
and `unpremultiply` helpers operate on f32 values. Relative luminance expects linear
RGB; zero-alpha unpremultiplication clears hidden RGB. These helpers do not own
surfaces, validate every input or perform tone mapping.

## Transforms and Paths

`NodeModifiers` attaches translation, scale, rotation, pivot and opacity to content. The default pivot is the measured center; choose it explicitly to rotate around a particular point. Transforms are ordered: scaling then translating is not interchangeable with translating then scaling.

`Path2D` retains segments and curves rather than treating them as integer pixels. Its chained methods take a `Point`, such as `move_to(Point::new(...))`, not two separate numeric arguments.

Paths also support SVG path data, stroke outlines, trimming and dashes. Choose
`FillRule::NonZero` or `EvenOdd` explicitly for holes and self-intersections.

For path data from a network or editor, use the corresponding `try_` methods to handle errors immediately. Chained authoring methods retain error state; appending valid segments cannot erase an earlier failure during validation.

## Deterministic Randomness

`context.random(key)` returns a stable value in `[0, 1)`. `context.random_generator(key)` returns an independent stream starting from that key. Give objects stable identifiers rather than incidental indices that change during frame traversal.

Successive values from one stream depend on sampling order. Use different keys for independent elements. Stable randomness does not compensate for changing external files, system time or mutable global state.

## Common Pitfalls

- Do not mix `Duration`, time points and frame indices. Establish what an interface expects before converting.
- Approximating NTSC as 30.0 and accumulating seconds causes drift in long outputs and audio/video boundaries.
- Do not hard-code preview pixel widths into scenes. Use logical dimensions or geometry from `context`.
- Infinity, NaN and noninvertible mappings need input validation, not an expectation of silent repair.
- Identical seeds do not guarantee identical pixels across all platforms and backends. Fonts, library versions and devices remain environment inputs.

## Next Steps

Next chapter: [03 Animation](03-animation.md).
Further reading: [time and animation](../design/core.md#stateless-animation), [color implementation](../../crates/motion-core/src/color/math.rs), [path implementation](../../crates/motion-core/src/path2d.rs).