# 09 Rendering

## What This Chapter Covers

Choose a CPU/GPU backend, reuse preparation and caches, and distinguish transparent images, explicitly formatted memory frames and external GPU textures.
The CPU example needs no optional features. Native GPU and external texture support require `gpu`.

## Minimal Example: Reuse a Session

```rust,no_run
use motion::prelude::*;
use motion::render::{
    FrameAt, FrameBuffer, FrameRequest, PixelFormat, RasterLimits, RenderSession, RenderTarget,
};

fn main() -> motion::Result {
    let size = PixelSize::new(320, 180);
    let options = RenderOptions::default().size(size);
    let mut session = RenderSession::new(Size::new(320.0, 180.0), options)?;
    let mut output = FrameBuffer::new(size, RasterLimits::default())?;
    session.render(
        FrameRequest {
            at: FrameAt::Time(250.milliseconds().after_origin()),
            frame_rate: FrameRate::NTSC,
            duration: 2.seconds(),
            background: None,
            target: RenderTarget::Memory(&mut output),
            format: PixelFormat::SrgbRgba8 { premultiplied: false },
        },
        |context| Node::from(context.bounds().fill(Color::rgb(20, 150, 125))),
    )?;
    Ok(())
}
```

Output is written to a memory target in the declared format. Another request can reuse the same session instead of recreating devices and all caches for every time point.

## Choosing an API Level

| API | Suitable Use |
| --- | --- |
| `render_still(timeline, time, options)` | A single transparent image |
| `RenderOptions` | Shared backend, assets, quality, output dimensions and budget configuration |
| `RenderSession` + `FrameRequest` | Repeated requests with explicit time, target and format |
| `Composition` + `FrameRenderer` | Integer-indexed frame requests for one composition |
| `RenderPrepare` / `PreparedFrame` | Extensions controlling command preparation and low-level consumption |
| `GpuContext` | Import or share native devices, queues and submission lifetimes |

`FrameAt::Time` does not automatically snap to the frame grid; `FrameAt::Index` converts exactly with the request's rational frame rate. `FrameRenderer` wraps the same session for indices of one composition, so their clocks must remain distinct.

## Backend Selection

```rust,no_run
use motion::prelude::*;

fn main() {
    let cpu = RenderOptions::default().backend(BackendPreference::Cpu);
    let automatic = RenderOptions::default().backend(BackendPreference::automatic());
    let strict_gpu = RenderOptions::default().backend(BackendPreference::gpu());
    let ci_gpu = RenderOptions::default().backend(BackendPreference::Gpu {
        allow_software: true,
    });
    let _ = (cpu, automatic, strict_gpu, ci_gpu);
}
```

This only configures policy; it does not initialize a GPU. Actual GPU requests require `gpu` and a supported native adapter.

- `Cpu` is the library default and does not initialize a composition GPU.
- `Automatic` can select CPU during startup policy and admission checks, retaining the reason.
- `Gpu` explicitly requires GPU rendering. Device loss, unsupported formats and execution failures do not silently switch to CPU.
- Software GPU adapters require explicit permission. Their measurements do not represent physical GPU performance.

`RenderContext` manages Assets and structural policy, not a GPU. `RenderOptions.gpu_context` explicitly shares a device but does not select the backend.

## FrameBuffer

`FrameBuffer` is the single stored image type; in straight `SrgbRgba8` it provides transparent images and `save_png`. `Snapshot` is explicit low-level opaque RGB output. Its PPM writer writes directly to a file and does not provide the same protection contract as transactional exports.

| `PixelFormat` | Key Requirements |
| --- | --- |
| `SrgbRgba8` | Explicit straight or premultiplied alpha |
| Linear RGBA16F | Linear-premultiplied; output is limited by half-float range |
| `Yuv420` 8/10-bit | Limited-range BT.709; even width and height; no transparent pixels |

Do not infer plane strides from format names or reinterpret bytes with different alpha representations as the same frame. YUV is not a synonym for an arbitrary RGBA buffer, and HDR has no automatic tone mapping.

## PreparedFrame and Caches

Preparation generates curves, coverage, paints, clips and filter data on the CPU. GPU composition is not GPU path rasterization; it consumes the same supported prepared data.

`ValidatedDisplayList` already has structural measurements, so normal preparation does not rescan a successful command stream. Prepared images reference immutable `ImagePixels` directly, without another deferred-producer execution stage.

`RenderCache` has separately limited coverage, paragraph, glyph, gradient and formula partitions. Cloned RenderOptions can share these pure caches. `SceneCache` caches authoring computations; Studio's final-preview-frame cache is another layer.

Fixed resources, fonts and caches can be shared, while each request retains its own diagnostics, target and allowance boundaries. Cache hits neither skip parameter validation nor imply completed rendering.

## GPU Texture Output

GPU working color images remain in RGBA32F textures. Staging uploads, instance metadata and finite-state checks still involve CPU and buffer work. Texture display does not mean zero CPU work.

External textures must belong to the same device and satisfy the declared format's 2D, single-layer, single-mip, single-sample render-attachment contract. RGBA8/BGRA8 are encoded output; linear RGBA16F has a separate output contract.

`FrameRenderer::submit_to_texture` / `GpuContext::submit_texture` return `PendingTextureFrame`. Complete `finish` checks for submission, finite values, validation/OOM and device health before publishing the target. `is_ready` polls its own readiness. Dropping unfinished work must not treat in-flight resources as idle pool entries.

Studio registers textures on the same device as eframe, avoiding full-frame CPU readback and re-upload. Finite-state checks can still involve small readbacks. Requests returning CPU-readable Bitmaps or memory frames cross their own readback and format boundaries.

## Budgets and Failures

Memory targets are replaced only after success; their existing payload also contributes to the active request's relevant costs. External textures do not have the same rollback guarantee after GPU failure, making it especially important to keep them unpublished until completion checks pass.

Quality, pixel dimensions, filter halos, target formats and shared holders all affect cost. `RenderQuality::Draft` is an explicit approximation policy, not a silent reduction of Final defaults.

## Common Pitfalls

- Creating GpuContext per frame loses device, pool and pipeline reuse.
- Identical adapter models do not mean external textures belong to the same device.
- Displaying or overwriting a texture immediately after submission bypasses completion checks.
- Memory output is not zero-readback texture output.
- Pure caches, active frames, idle pools, external holders and RSS are distinct quantities.

## Next Steps

Next chapter: [10 Audio](10-audio.md). Without audio, skip to [11 Export](11-export.md).
Full contracts: [rendering and output](../design/rendering.md), [GPU](../design/gpu.md), [FrameRenderer](../../crates/motion-render/src/composition/frame_renderer.rs).