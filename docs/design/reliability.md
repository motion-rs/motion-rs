# Reliability

## Validation and Failure

Composition preflight checks dimensions/timing without evaluating content or creating
a device. Node measurement and compilation validate structural properties; preparation
and rendering apply separate raster work/live-byte limits. Invalid hidden content is
not silently skipped. Arbitrary user callbacks are trusted code, not sandboxed work.

Default raster admission uses `RasterLimits::authoring()` without a work ceiling;
`RasterLimits::sandbox()` preserves the opt-in 200,000,000-unit cap. Pixel/live-byte
limits and explicit work limits remain enforced. Export concurrency uses the first
frame's measured logical peak and bounded slots, with serial retry for larger later
frames. This is neither a deadline nor a process memory sandbox.

Error retains category, optional frame and original source. Inspect `kind`, `frame`
and `downcast_ref` rather than parsing messages. Synchronous callback panics propagate;
worker boundaries convert them into scene-panic diagnostics. A failed immutable frame
can be retried with healthy inputs without accepting partial output. Library mutexes
guard plain data that is valid after every statement, so a lock poisoned by a worker
panic is recovered through `motion_core::sync` rather than turned into a second panic.
The font engine is the exception: its third-party shaping state is not known to be
valid after a panic, so a poisoned engine reports a validation error.

ValidatedDisplayList retains a checked immutable command snapshot, not backend or
raster admission. Image paints reference immutable pixel data without deferred
production callbacks. Preflight cannot prove arbitrary user callbacks are device-free
or bounded.

## Who Returns What

| Type | Returned by | Meaning |
| --- | --- | --- |
| `ValidationError` | Constructors, `try_*` methods, `build` of value builders (`Spring`, `Oscillator`, `Path2D`) | One invalid property with its structural path; no frame context |
| `ErrorReport` | `TimelineBuilder::build`, `ProjectBuilder::build` | Every failure collected while assembling a timeline or project; fatal |
| `Diagnostics` | `Timeline::diagnostics` and scene evaluation | Non-fatal warnings and recorded errors that stay attached to a built value |
| `Error` | Rendering, export and I/O (`motion::Result`) | Category (`kind`), optional frame and the original source; wraps the three above |

Scene closures may return `Result<Node, ValidationError>` (any `IntoSceneOutput`), so `?` on a shape
constructor flows into the frame error without an infallible twin.

## Ownership and Transactions

Resources are explicit immutable snapshots. Cache identity includes the environment
that changes decoding/layout. Byte/entry limits describe logical retention, not hard
process RSS. Font/device discovery is not performed implicitly during scene callbacks.

Each new Assets owner has an isolated ResourceStore unless sharing is explicit.
RenderSession owns caches and installs a fresh RequestDiagnostics sink per request;
Handle::get_with propagates resource failures to an explicit cross-thread sink.
DrawContext passes the sink explicitly; FrameAssets and the frame thread-local bridge
have been removed. Captured handles use get_with rather than relying on ambient state.
RenderOptions.gpu_context is the explicit GPU injection point; RenderContext stores
assets and structural policy. External GPU output textures must match the selected
device and validated target contract. Failed submissions do not publish partial previews.

System font metadata is indexed lazily; deterministic projects should register
explicit immutable font bytes. ExportReport.fonts audits observed text source hashes,
not platform-independent glyph rasterization or a cryptographic file identity.

Exports write temporary files and commit once after codec finalization. Cancellation
before that point preserves existing output and drops staging files. Cancellation
does not interrupt a blocking call immediately. Streaming inputs must remain
available; output aliases of source files are rejected.

Memory render targets are replaced only after success, with the old payload included
in logical admission. External GPU textures are written in place and do not provide
the file export transaction's rollback guarantee.

## Determinism

Environment-scoped replay is the selected determinism policy, not platform-independent
byte identity. The environment includes the build, numerical dependencies, platform,
immutable resources and, for GPU execution, device and driver. Quality, output format
and all authored inputs must also remain fixed.

| Scope | Guarantee |
| --- | --- |
| CPU replay | Exact replay with the same build/platform, resources, inputs, quality and format |
| GPU replay | Exact seek replay with the same build/platform/device/driver, resources, inputs, quality and format |
| GPU versus CPU | Shared coverage; fixture tolerance of at most 2 LSB in straight RGBA8, with strictly fewer than 0.1% of pixels differing by more than 1 LSB |
| Other platforms/devices | No blanket byte identity or tolerance acceptance; run explicit fixtures |

All four channels and transparent pixels count in the per-frame GPU/CPU gate;
frames are not averaged. Stricter exact kernel/conversion fixtures remain intact.
Changing arithmetic, compiler, device or driver requires new applicable checks.
Controlled libm arithmetic alone is not cross-platform proof.

The CPU/GPU tolerance does not permit nonfinite working pixels, unchecked alpha
handling or publishing before submission errors are checked. RGBA16F working
textures and reduced error checking are not current capabilities merely because
the output gate allows a tolerance.

Use seeded randomness, explicit immutable resources and complete cache keys.
Ordered drawing cannot depend on HashMap iteration or ambient clock/discovery.
Preserve reference operation order when optimizing; externally pinned Arcs and
logical cache allowances are not RSS bounds. Callback purity is a project contract,
not an enforced sandbox.

See [development checks](../development.md) for property/reference/replay tests,
feature matrices and native fixtures. Compilation, zero-test filters and ignored
fixtures do not establish runtime, UI, codec or cross-platform acceptance.
