# 14 Architecture and Extensions

## What This Chapter Covers

Choose the right extension layer and understand ownership of values, content, rendering, encoding and the workbench without pulling devices or I/O back into core.
The minimal `Drawable` needs no optional features. Select features for content and native extensions through their owning crates.

## Minimal Example: Custom Drawable

```rust,no_run
use motion::prelude::*;
use motion::extend::{Canvas, DrawContext, Drawable, ValidationError};

struct Badge;

impl Drawable for Badge {
    fn bounds(&self, _context: &DrawContext) -> Result<Rect, ValidationError> {
        Ok(Rect::new(0.0, 0.0, 160.0, 60.0))
    }

    fn draw(
        &self,
        context: &DrawContext,
        canvas: &mut Canvas<'_>,
    ) -> Result<(), ValidationError> {
        canvas.fill_rect(self.bounds(context)?, Color::rgb(20, 150, 125))?;
        canvas.fill_rect(Rect::new(20.0, 20.0, 140.0, 40.0), Color::WHITE)
    }
}

fn main() -> motion::Result {
    let timeline = Timeline::new(Size::new(320.0, 180.0), 30).track(|track| {
        track.scene(1.seconds(), |_| Node::from(Badge))
    }).build()?;
    render_still(&timeline, Time::ZERO, &RenderOptions::default())?
        .save_png("tutorial-drawable.png")
}
```

Drawable returns local bounds and emits commands to a checked Canvas. It does not create devices, write files or return an image task with a separate backend lifetime. `bounds` is fallible, so a Drawable reports invalid geometry as a `ValidationError`. It is only for true leaves. Content that expands into other nodes at the current time, such as animated text, SVG and formulas, implements `DeferredContent::resolve` and is wrapped with `Node::deferred`. Text, SVG and formulas share one pipeline for this: a `CompileCache` created from an immutable compile key, a compiled vector tree, `VectorSelection`, and `VectorTree::reveal`; reuse it instead of clearing caches by hand.

## Crate Ownership

| Crate | Responsibility / Dependency Direction |
| --- | --- |
| `motion-core` | Values, animation, authoring, commands, resources, audio occurrences; no upstream engine dependency |
| `motion-render` | Coverage, CPU/GPU, sessions, frame targets; depends on core |
| `motion-content` | Images, layout, SVG, vectors, particles, gated content; depends on core |
| `motion-formula` | Typst and formula operations; depends on core/content |
| `motion-encode` | PCM, streaming, transactional encoding; depends on core/render, optional FFmpeg |
| `motion-scene` | Declarative JSON documents and schema; depends on core/render, optional content |
| `motion-studio` | Desktop preview, playback, export orchestration; depends on core/render/encode |
| `motion-rs` | Direct facade, Project/CLI; feature-selected reexports |
| `motion-ffmpeg` | Native ABI and thread-confined resource ownership |
| `motion-bench` / `motion-gallery` | Measurements and films, not core engine implementations |

Each type is defined in one owning crate; lower layers do not depend on higher-level convenience namespaces. Core image, instance and font-snapshot protocols do not initialize content compilers, devices or windows.

## Five Import Categories

| Namespace | Purpose |
| --- | --- |
| `motion::prelude` | Authoring values and expressions only; no encoders, progress sinks or render intermediates |
| `motion::extend` | Protocols such as `Drawable`, `Asset`, `AudioSource`, `RenderPrepare`, `Encoder` |
| `motion::render` | Explicit low-level commands, budgets, sessions, formats, GPU |
| `motion::encode` | Encoders and factories, progress sinks, `ExportReport`, `ExportPhase`, `NoProgress` |
| `motion::eager` | Precompiled vectors/formulas and related operations |

The facade directly reexports shared types without another wrapper layer. Internal modules do not chain imports through public preludes, and lower-level crates do not reexport every upstream convenience type.

## Choosing Extension Points

| Extension Point | Contract |
| --- | --- |
| `Drawable` | Measure and produce checked local commands without partial successful output |
| `Asset` / `AssetsExt` | Immutable snapshots, lazy typed handles, retained bytes, explicit resource owners |
| `AudioSource` | Pure random-access samples, rate/channels, bounded read counts |
| `RenderPrepare` | Prepare checked display data into renderer-consumable records |
| `Encoder` | Declare formats/capabilities, ordered open/write/finish, own transaction cleanup |
| `ProgressSink` | Thread-safe notifications and cooperative cancellation; frame counts do not imply commit |
| Instance protocols | Bounded random-access instance samples and style data |

`AssetsExt` is a current content-crate convenience trait providing typed access under image/svg/text features. The actual handle type is `Handle<A>`, not the old design draft's `AssetHandle`.

When preloading `Arc<A>`, report `Asset::retained_bytes` correctly. During execution, pass the diagnostic sink from `DrawContext` to captured handles through `get_with`, rather than relying on the removed ambient frame thread-local.

## Keep GPU Ownership Explicit

`RenderContext` holds assets and structural policy. Inject GPU state through `RenderOptions.gpu_context` or explicit context APIs. Complete pending texture-output checks before handing results to the UI; external holders remain separate from request and cache budgets.

There is currently no `ImageProducer`, `ProducedImage`, native-image input registry or model3d extension pipeline. Do not reintroduce dependencies on removed interfaces from historical reviews. Image paints consume immutable `ImagePixels`; external textures are a rendering-output contract.

## Contribution Boundaries

Keep implementations in the modules that directly control behavior. Tests follow implementations, with larger suites in adjacent test modules. Start with a minimal feature consumer and check non-test compilation so test-only imports or workspace feature unification do not hide omissions.

Select `vector`, `particles`, `ink`, `diagram`, `captions`, `typst` and `formula` independently instead of checking only default or all features. Native FFmpeg, GPU and windows have additional environment requirements; feature availability does not imply runtime health.

The structure check does not compile:

```sh
node tools/check-structure.cjs
```

See [development checks](../development.md) for compilation, testing, Rustdoc and real-device gates. Do not bury a local behavior change in repository-wide formatting, unrelated refactoring or new facade wrappers.

## Common Pitfalls

- Putting rendering devices or Typst internals into core's public contract.
- Using facade or public preludes to aggregate internal imports.
- Relying only on all-features tests to validate independent feature consumers.
- Expecting the library to protect destinations when a custom Encoder omits failure/drop cleanup.
- Copying removed interfaces from old designs or reviews without checking current source.

## Next Steps

Apply these ideas to films: [Gallery guide](appendix-gallery.md). Return to the [tutorial index](README.md).
Full rules: [architecture and packages](../design/architecture.md), [design principles](../design/README.md).
Protocols: [Drawable](../../crates/motion-core/src/drawing/context.rs), [Encoder](../../crates/motion-encode/src/export.rs).