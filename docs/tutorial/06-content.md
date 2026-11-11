# 06 Content and Resources

## What This Chapter Covers

Add images, SVG and text to scenes with a consistent resource, font and cache environment. Enable features as needed; you do not need every content capability at once.

| Content | Feature |
| --- | --- |
| Images and animated images | `image` |
| SVG | `svg` |
| Text and fonts | `text` |
| VectorTree and Morph | `vector` |
| Brush strokes and textured fills | `ink` |
| Charts and annotations | `diagram` |
| Subtitles and cues | `captions` |

## Minimal Example: Explicit Fonts

Enable `text`. From the repository root, this example uses the bundled Noto font. In a standalone project, replace the path with an explicitly supplied font file.

```rust,no_run
use motion::prelude::*;

fn main() -> motion::Result {
    let mut fonts = FontLibrary::new();
    fonts.load_file("tests/fixtures/fonts/NotoSans-Regular.ttf")?;
    let assets = Assets::new(".")?.with_fonts(fonts);
    let options = RenderOptions::default().resources(assets);
    let timeline = Timeline::new(Size::new(480.0, 270.0), 30).track(|track| {
        track.scene(2.seconds(), |context| {
            Text::new("Motion, one frame at a time.")
                .family("Noto Sans")
                .size(26.0)
                .fill(Color::rgb(20, 150, 125))
                .write(context.progress())
                .translate(32.0, 100.0)
        })
    }).build()?;
    render_still(&timeline, 1.seconds().after_origin(), &options)?
        .save_png("tutorial-content.png")
}
```

`FontLibrary::new` creates an empty environment without scanning the system. `load_file` reads and registers explicit bytes; `Assets::new` can fail, while `with_fonts` derives a font snapshot. Cache identity includes the font environment; existing snapshots and handles retain their own lifetimes.

## Images and Resource Owners

`Image::open("photo.png")` retains a source, resolved through the caller's Assets during compilation. `ImageAsset` is the eager representation of fixed content; `Image` controls presentation, fit, size, corners and sampling.

Enable `image` for this example; no external image file is needed:

```rust,no_run
use motion::prelude::*;
use std::sync::Arc;

fn main() -> motion::Result {
    let assets = Assets::new(".")?;
    let pixels = ImageAsset::from_rgba8(32, 32, vec![255; 32 * 32 * 4])?;
    assets.preload("tile.png", Arc::new(pixels))?;
    let options = RenderOptions::default().resources(assets);
    let timeline = Timeline::new(Size::new(160.0, 90.0), 30).track(|track| {
        track.scene(1.seconds(), |_| {
            Image::open("tile.png").translate(64.0, 29.0)
        })
    }).build()?;
    render_still(&timeline, Time::ZERO, &options)?
        .save_png("tutorial-preloaded-image.png")
}
```

Preloading and lazy loading use the same owner. Extensions such as `AssetsExt::image` return the current `Handle<A>`. Creating a handle does not finish loading; `get()` requests the actual content.

PNG/JPEG/WebP decode to shared linear-premultiplied mips. Sampling, image mapping and edge extension do not require copying the pixels before drawing. `AnimatedImageAsset` stores composited GIF/APNG or explicit frame sequences; `AnimatedImage` selects frames by ticks. Loop/PingPong do not duplicate endpoint durations.

## SVG and VectorTree

SVG parses into the shared VectorTree rather than silently generating a bitmap through another SVG raster backend. `Svg::open` uses the same resource owner as images. SVG text and image content also require the appropriate fonts and assets.

| API | Purpose |
| --- | --- |
| `SvgAsset` / `Svg` | Parsed SVG and its scene presentation node |
| `VectorTree` | A vector model of shared paths, groups, images and masks |
| `Morph` | Content-level vector morphing |
| `WriteOptions` | Explicitly prepared vector writing effects |
| `ImagePattern` / `Sampling` | Image mapping and reconstruction |

Static SVG content is supported. Read diagnostics or errors for unsupported features; do not assume browser SVG animation, scripting or CSS behavior is available.

## Text Is a Paragraph, Not Per-Character Textures

`Text` and `TextSpan` support families, weights, sizes, line height, letter spacing, width, alignment, rich text and outlines. Layout includes bidi, fallback and graphemes; UTF-8 bytes or individual code points cannot predict a character's width.

`ParagraphStyle` holds the paragraph description (text, style, width, alignment, fonts); `Text::paragraph` exposes it and `Text::from_paragraph` wraps one. Its `layout` produces a `ParagraphLayout` with actual glyphs, baselines, lines and source information, which every output path (static drawing, `write`, `animate_parts`, captions) reuses. Use these layout results for fixed-width or multiline content, not `character_count * font_size`.

`Text::write` and `animate_parts` can remain deferred effects using the inherited scene-local clock. Use explicit eager or context interfaces to inspect layout or prepare truncated content in advance. `fit_lines` is a precomputation, not work to repeat every frame.

Layout can be reused when fonts, content and layout parameters are unchanged. Color or position changes need not trigger shaping again. Do not conflate failures or different font environments through caching.

Text parts retain grapheme and source boundaries, visibility and baseline layout.
Compose decorations such as underlines from measured text and ordinary geometry,
not a second text renderer. A Text carries one reveal effect: calling `write` after
`animate_parts` (or the reverse) replaces the earlier one, so use separate Text nodes
for handwriting and part transforms together.

## Ink, Diagram and Captions

`BrushStroke` combines paths, independent tip dimensions, pressure, ink load and arc-length reveal; build it with `BrushStroke::builder(...).build()?`. Both `BrushStroke` and `InkFill` take a color, gradient or other `Paint`, and `InkFill` uses the same deterministic `Ink` materials and `Paper` (stroke-only effects such as fibers and build-up are ignored by fills). Dry marks, paper, grain, bleeding, roughness and pooling are procedural coverage, not a physical fluid simulation.

Retain the configured full stroke and animate only the necessary reveal. Splitting a continuous stroke into many pieces to evade budgets can change pressure and material continuity.

`motion::diagram` provides chart and annotation tools such as Scale, LineChart, Ruler and Callout. Units remain f64, and legends and labels need the correct font environment.

`motion::captions` provides CaptionTrack and CueSheet. Cues use exact half-open intervals. SRT is a millisecond format, so arbitrary ticks cannot be exported losslessly to SRT. Caption layout and display belong to content; later chapters cover audio occurrences and encoding.

### Prepared Caption Layout

`CaptionStyle` uses an opposite-corner safe Rect, Color and f64 sizes.
`layout(track, canvas_size, &fonts)` requires an explicit frozen FontLibrary and
precomputes paragraph/vector nodes. `CaptionLayout::node(Time)` selects half-open
cues and stacks overlaps bottom-up without reshaping every frame.

Keyword matching is literal, longest-first, insertion-stable and grapheme-safe;
tabs become four spaces. Input, cue, run, keyword and line budgets are bounded.
Safe-area overflow or excessive simultaneous cue height is an error, not accepted
clipped text. Inspect `cues` and `safe_area` to review measured preparation.

## Resource Stores and Diagnostics

Sharing `ResourceStore` is explicit. Separate Assets owners have isolated stores by default. Sharing a store does not allow arbitrary reuse across different roots, sources or font snapshots.

`Assets` cloning, snapshots, preloading and removal have distinct contracts. Removing a registration does not forcibly destroy data still held by handles or frames. Logical retained bytes are not RSS.

The scene's `DrawContext` shares an immutable `Evaluation` with time, quality, diagnostics, fonts, resources and the render cache (a single optional `RenderCache`; layout caching uses its paragraph partition). Cross-thread or previously captured handles use `get_with` to pass explicit diagnostics; there is no global current-frame context to rely on.

## Common Pitfalls

- An empty FontLibrary does not automatically gain system fallback. Register project fonts first.
- Latin-only fonts can leave Chinese glyphs missing at runtime even if the code compiles.
- Do not mix working-directory paths with asset-root paths. Define the root instead of relying on where the program happens to launch.
- Reuse owners, handles and prepared content rather than decoding assets or rebuilding compiler environments per frame.
- `allow_missing_assets` is not a normal production strategy. Placeholders and diagnostics do not mean assets loaded successfully.

## Next Steps

Next chapter: [07 Particles](07-particles.md). Without particles, skip to [08 Typst and Formulas](08-formula-typst.md) or [09 Rendering](09-rendering.md).
Contracts: [content preparation](../design/content.md), [shared vector content](../design/content.md#svg-and-vector-trees), [font implementation](../../crates/motion-content/src/font.rs).
Films: [Room to Move](../../gallery/src/demos/type_in_motion.rs), [The Long Way Home](../../gallery/src/demos/ink_journey.rs).