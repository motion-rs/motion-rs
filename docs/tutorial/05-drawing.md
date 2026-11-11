# 05 Drawing

## What This Chapter Covers

Learn how shapes become commands, gradients and strokes are sampled, and clips and filters form layers. Understand when to use Canvas. This chapter needs no optional features; chapter 06 covers loading image-pattern assets.

## Minimal Example

```rust,no_run
use motion::prelude::*;
use motion::extend::{Canvas, Drawable};
use motion::render::{DisplayList, FilterOperation, ValidatedDisplayList};

fn main() -> motion::Result {
    let timeline = Timeline::new(Size::new(480.0, 270.0), fps!(30)).track(|track| {
        track.scene(1.seconds(), |_| -> motion::Result<_> {
            Ok(Node::group((
                Rect::new(60.0, 60.0, 420.0, 210.0)
                    .fill(Color::rgb(20, 150, 125))
                    .filter(Filter::DropShadow {
                        offset: Vector2::new(0.0, 6.0),
                        sigma: 3.0,
                        color: Color::BLACK.with_alpha(0.35),
                    }),
                Circle::new(Point::new(240.0, 135.0), 72.0)?
                    .fill(Color::rgb(220, 90, 115))
                    .clip_with(Rect::new(180.0, 80.0, 300.0, 190.0), ClipMode::Geometric),)
            ))
        })
    }).build()?;
    render_still(&timeline, Time::ZERO, &RenderOptions::default())?
        .save_png("tutorial-drawing.png")
}
```

Shadows are ordered layer effects; geometric clipping intersects geometry first. Neither simply rewrites the original shape bounds as final pixel bounds.

## Paint and Stroke

| API | Contract |
| --- | --- |
| `Paint::Solid` / `Color` | Solid-color drawing |
| `LinearGradient` | Linear gradients |
| `RadialGradient` | Radial and focal gradients |
| `SweepGradient` | Angular gradients around a center |
| `ImagePattern` | Immutable image data, mapping, extension, sampling |
| `GradientUnits` | Coordinate units used by the gradient |
| `GradientSpace` | Color interpolation space |
| `GradientExtend` | Interval extension such as Pad, Repeat and Reflect |
| `StrokeStyle` | Width, caps, joins, miter limit |

Do not confuse gradient coordinates with color interpolation space. Resizing a node can produce different results for local absolute gradients and gradients mapped to object bounds.

Strokes expand into bounded, budgeted outlines. Both the input path and generated outline have costs; zero-width strokes do not bypass parameter validation. `LineCap`, `LineJoin` and the miter limit directly affect appearance and conservative bounds.

## Canvas Records Commands

Ordinary applications should prefer shape `.fill` and `.stroke`. Custom content implements `Drawable` and uses the caller-provided Canvas in `draw`. Do not create a global recorder outside the scene budget.

| Canvas Operation | Purpose |
| --- | --- |
| `fill_rect(bounds, paint)` | Retain the direct rectangle-fill path |
| `fill(shape, paint, rule)` | Record a shape with an explicit fill rule |
| `fill_path(arc_path, transform, paint, rule)` | Record a shared path and local mapping |
| `stroke(shape, style, paint)` | Generate and record a stroke outline |
| `instances(batch, transform)` | Record evaluated instances without expanding them into paths |

The first Canvas error fails the recording. Even if a custom callback ignores the return value, subsequent valid commands cannot be treated as a complete successful image.

`FillRule::NonZero` and `EvenOdd` define holes and self-intersections differently. Path topology and fill rules are scene inputs, not choices for CPU and GPU to guess independently.

## Clips, Masks and Blending

`clip(shape)` multiplies independently rasterized coverage. `clip_with(shape, ClipMode::Geometric)` intersects geometry before quantization, with a specialized path for rectangles.

A later geometric clip on the same Node replaces the earlier one. Use parent and child nodes for multiple intersections. Content masks use alpha or linear-premultiplied Rec.709 luminance; they are not another spelling of path clipping.

`BlendSpace::LinearRgb` is the default. Explicit sRGB composition crosses the corresponding encoding and decoding boundaries. Group opacity applies to the group result; individual child alpha is not an automatic substitute.

## Filter Order

`Filter` describes operations such as Blur, DropShadow, Glow and ColorAdjust. Lens dispersion and vignetting are `Filter::Optics(Optics::Lens { .. })` and `Optics::LensVignette { .. }`; seeded scene-linear noise is `Filter::grain(seed, amplitude)` (display-domain film grain is `FilmGrain`). Multiple `.filter` calls execute in order. Blurring before color adjustment need not match the reverse order.

Child effects complete before parent effects. Filter distances use local design units; preparation derives conservative device capture regions. Layout bounds and the pixel regions affected by filters are different.

The completed source is filtered, masked/clipped, then composited with group
opacity and blending. Blur uses normalized bounded box approximations with
fractional small-sigma weights. Transparent borders do not introduce hidden color.
Custom `FilterOperation` values reference their saved input explicitly; ordering
is not inferred from which source appears nearby.

Motion blur averages linear-premultiplied captures from replayable subframe samples; it is not a trail from the previous frame. Keep captions or HUD content outside the sampled subtree when they should remain sharp.

Empty shutter samples retain their weight. Sampling callbacks must use the supplied
subframe context rather than a previously rendered frame. GPU isolated working
layers use RGBA32F (16 bytes per pixel); external RGBA16F quantizes only at the
declared output boundary. Admission charges working precision, not output precision.

## DisplayList, Validation and Budget

`Node::compile` records and validates an immutable `ValidatedDisplayList`. Low-level `DisplayList` is editable, but edits require revalidation. Use `into_editable` for explicit changes or `with_background` to change only the background.

Structural validation checks nodes, depth, commands, paths and parameters. Raster budgets check pixels, logical bytes and work. Structural success does not guarantee later resource, GPU or budget availability.

`Budget` charges before growth. Limits describe logical costs of supported paths, not a hard sandbox for third-party callbacks, drivers, encoders or process RSS. Custom Drawables are trusted code.

Prepared image paints directly reference immutable pixels; there is no separate deferred image-producer execution protocol.

## Extended Ink Brushes

Enable `ink` and use `motion::ink`. `BrushStroke::builder(path, width, paint)` starts a `BrushStrokeBuilder` over the single bounded multistroke geometry and full-material implementation; setters such as `pressure_curve`, `caps`, `bristles` and `tiled_material` refine it at every render quality, and `build()` validates it and returns an immutable `BrushStroke`. `BrushStroke::new(path, width, paint)` builds the default stroke directly. The color argument is any `Into<Paint>`: a `Color` still works, and gradients and image patterns are accepted. The baked material is cached on the built stroke only, so build once, retain the stroke and clone it for each reveal rather than reconstructing it in every callback.

```rust,no_run
use motion::prelude::*;
use motion::ink::{BrushStroke, NibMode, StrokeFont};

fn main() -> motion::Result {
    let path = Path2D::builder()
        .move_to(Point::new(20.0, 30.0))
        .line_to(Point::new(120.0, 70.0))
        .build()?;
    let stroke = BrushStroke::builder(path, 8.0, Color::rgb(20, 150, 125))
        .pressure_curve(vec![(0.0, 0.2), (0.5, 1.5), (1.0, 0.1)])
        .nib_mode(NibMode::FollowPath(0.0))
        .bristles(5, 0.4, 0.1, 17)
        .parallel_material(true)
        .build()?;
    let _revealed = Node::from(stroke.clone().write(0.5));
    let _cursor = stroke.write(0.5).tip_node(Circle::new(Point::new(0.0, 0.0), 2.0)?.fill(Color::WHITE))?;
    let _letters = StrokeFont::new(20.0)?.strokes("MOTION", 1.0, Color::WHITE)?;
    Ok(())
}
```

`pressure`, `pressure_curve` and `pressure_from` use last-builder-wins semantics. `write(progress)` reveals from the start by original arc length and reuses the cached material; it differs from `Styled::trim`, which clips an interval of an ordinary stroke outline. `BrushCap` (`Round`, `Flat`, `Tapered`) shapes the pressure profile at the ends of a variable-width brush stroke, whereas `LineCap` (`Butt`, `Round`, `Square`) chooses the end geometry of a uniform `StrokeStyle` outline; they are independent. `InputSample` records finite local positions, pressure 0..=4, tilt azimuth in radians and strictly increasing `Time` starting at zero (2..=512 samples). `BrushStrokeBuilder::from_samples` fits Catmull-Rom geometry; `write_with(time)` uses fitted arc/time mapping, preserving stationary pen pauses. `BrushTiming`/`write_timed` provide a monotone speed curve and pen-up interval for independent subpaths.

`BrushPreset` provides ten styles. `.stroke(path,width,paint)` returns an editable `BrushStrokeBuilder`; `.node` builds it and additionally applies the intended Multiply/opacity/Bloom composition. `paint` replaces the stroke's color with a color, gradient or image paint, `bristles` creates seeded fine tips, and `wobble`, frame-seeded `boil` and analytic `try_wetness` remain replayable. Ink-load profiles stay on the builder, while the material effects `build_up`, `edge_anisotropy`, `edge_darkening` and `height_map` are `Ink` settings. `tip`/`tip_node` expose the final subpath endpoint/tangent for cursor or emitter placement; `erase(content)` uses an inverted-alpha mask without a new backend blend mode.

One `Paper` enum serves both `BrushStroke` and `InkFill` through `Ink::paper`: `Paper::Procedural { seed, strength, scale }` is seeded noise, and `Paper::Texture { texture, grain_scale, absorbency }` uses a resolved caller-owned image whose repeating height samples attenuate coverage. Paper defaults to untransformed design coordinates; `Ink::paper_transform` must explicitly match enclosing node transforms for canvas-fixed paper. `InkFill` uses dryness, bleed, roughness, granulation, paper, pooling and `edge_darkening`; fibers, `build_up`, `edge_anisotropy` and `height_map` need stroke geometry and apply only to strokes. `Ink::height_map` makes a stroke emit a separate grayscale ridge/load drawable for `Optics::Relight` with `HeightFrom::Luma`; it does not add an independent height input to colored Relight. Composite the lighting layer with the colored brush when both are needed.

`PigmentMix::KubelkaMunk.painting(strokes)` mixes only covered intersections using a six-band hand-authored reflectance approximation; it is not calibrated physical pigment or fluid simulation. `StrokeFont` uses original fourteen-segment uppercase/digit/plus/minus centerlines, maps lowercase to uppercase, supports spaces/newlines and rejects other glyphs. It performs no font outline skeleton extraction and needs no SVG/font resource.

Extended geometry admits 64 subpaths, 512 sample intervals per path and 64 bristles, additionally constrained by the global outline-verb budget. Material tiles admit 16..=2048 edges (default 2048), at most 256 tiles, a 16384-edge/16-million-texel total and bounded distance work. Pigment composition separately admits 32 strokes, 2048-edge/262144-texel output and 32 million coverage evaluations. Serial/parallel tile arithmetic is identical; CPU/GPU consume the same baked material. Draft/Final extended brushes share this path.

## Common Pitfalls

- Coverage and geometric clipping are not byte-equivalent; quantization order differs.
- Do not infer layout from filter-expanded pixel bounds. Layout and capture regions serve different purposes.
- Returning success after ignoring a Canvas error does not clear the recording's error state.
- Do not reorder effects arbitrarily. Subtree isolation and order are visible behavior.
- Budget numbers are not total GPU-memory or RSS caps; shared resources and external holders may be excluded.

## Next Steps

Next chapter: [06 Content and Resources](06-content.md).
Low-level contracts: [effects and color](../design/core.md#layers-and-sampling), [reliability](../design/reliability.md), [Canvas source](../../crates/motion-core/src/drawing/canvas.rs).
See [14 Architecture and Extensions](14-architecture-and-extending.md) for a complete custom Drawable example.