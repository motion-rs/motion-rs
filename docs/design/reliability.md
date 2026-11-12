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
| GPU replay | Visually stable seek replay with fixed resources, inputs, quality and format; numeric differences are allowed |
| GPU versus CPU | Equivalent drawing semantics and visual output with fixture-specific tolerances, not byte identity |
| Other platforms/devices | No blanket byte identity or tolerance acceptance; run explicit fixtures |

Changing arithmetic, compiler, device or driver requires new applicable checks.
Controlled libm arithmetic alone is not cross-platform proof.

### Visual acceptance

Pixel checks use one of four levels. A fixture names its level; it does not invent an
ad hoc metric.

| Level | Applies to | Gate |
| --- | --- | --- |
| Exact | CPU replay, CPU goldens, seeded sequences | Byte identity |
| Numeric | GPU kernels read back as working f32, packed RGBA and YUV planes | Per-channel absolute error declared by the fixture, such as 3e-4 linear or one output code |
| Visual | GPU output against the CPU backend | Multi-metric comparison with a named tier |
| Replay | GPU output against itself after a seek or resubmission | Visual comparison with the Replay tier |

The visual comparison follows the same ideas as Skia Gold's fuzzy, sample-area and
Sobel matchers and the Khronos CTS position-deviation comparison: a single global
number is never the only gate, localized error is not diluted by large images, and
edges may move by a pixel without hiding missing content. It is implemented in
[`visual.rs`](../../crates/motion-render/src/gpu/visual.rs) and runs on straight-alpha
sRGB RGBA8 images:

1. **Preconditions.** Both images have the same size and contain a visible pixel.
   Geometry fixtures also assert that the GPU produced its own coverage, so a blank or
   CPU-generated result cannot pass.
2. **Samples.** Each pixel contributes alpha and its display RGB composited over black
   and over white, normalized to 0..1 (one 8-bit code is 1/255). Color hidden by low
   alpha and alpha hidden by matching color are both visible. Pixels transparent in
   both images are excluded, so padding cannot dilute an error.
3. **Global RMS** over visible pixels bounds diffuse color, blend and gradient error.
4. **Tile RMS** is checked independently for every 16x16 tile with a visible pixel and
   catches localized dropouts that a global RMS hides.
5. **Neighborhood outliers.** A pixel is an outlier when any sample lies outside the
   range of the other image's 3x3 neighborhood plus a slack, checked in both
   directions. A one-pixel edge shift passes; holes, missing or dropped hairlines,
   clip leakage and inverted alpha fail. The tier bounds outliers as a fraction of
   visible pixels.

| Tier | Use | RMS | Tile RMS | Slack | Outlier ratio |
| --- | --- | --- | --- | --- | --- |
| Replay | Same backend, device and inputs | 0.002 | 0.004 | 2 codes | 0 |
| Paint | Solid and gradient paints, blending and group opacity on analytic or pixel-aligned coverage | 0.005 | 0.01 | 3 codes | 0 |
| Coverage | Antialiased paths and geometric clips without overlapping contours in a pixel | 0.01 | 0.03 | 4 codes | 0 |
| Overlap | Crossing or self-intersecting contours, whose f32 boundary intersections can differ locally from the CPU sweep | 0.05 | 0.12 | 8 codes | 3% |

A fixture may tighten any field of its tier; the startup self-check keeps a 0.02
global RMS for its self-intersecting geometry and 0.01 after filtering. Loosening a
tier, or a fixture beyond the Overlap tier, requires a stated reason and a change to
this page. The tiers were set from measurements on the Apple M1 Max adapter: across
the 48 fixed-seed crossing cases the largest values were 0.037 RMS, 0.080 tile RMS
and a 2.0% outlier ratio, while the other renderer coverage fixtures stayed below
0.003 RMS without outliers. Another adapter that fails a tier needs investigation,
not a wider tolerance.

There is no global per-channel LSB gate. No tolerance permits nonfinite working
pixels, missing geometry, wrong winding, clip leakage or visible flicker. Stricter
exact kernel/conversion fixtures remain local regression checks, not a restriction on
new backend algorithms.

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
