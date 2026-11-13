# Architecture

The rendering path is `SceneContext -> Scene -> Node -> ValidatedDisplayList ->
FrameIr -> backend preparation -> CPU/GPU -> output`. CPU preparation produces
`PreparedCpuFrame`; GPU preparation produces its own geometry and composition records.
A scene is a deterministic function of exact time and fixed
resources, not an object advanced by a playback clock.

## Evaluation and Ownership

SceneContext supplies signed local/global ticks, rational frame rate, logical bounds,
a shared `Evaluation` (quality, resources, diagnostics), keyed random streams and a bounded cache. Node snapshots
are immutable and copy-on-write. Compilation measures layout, resolves placement,
expands `DeferredContent` into ordinary nodes, records Drawable commands and validates all branches, including invisible content.

Node compilation validates while recording and returns ValidatedDisplayList directly.
DisplayList is an editable low-level API. ValidatedDisplayList freezes a checked Arc snapshot and
retains structural requirements for later allowance checks; it is not proof of
device availability or sufficient raster budgets. Image paints reference immutable
pixels; prepared frames have no deferred image-production phase.

The internal FrameIr normalizes ordered scopes, paint and clip references, filter
domains, padded dimensions and the output window before coverage allocation.
It retains f64 device-space curves, conservative Wang segment upper bounds and
admitted resource bytes, without coverage. A local-geometry recipe preserves CPU
transform arithmetic and coverage-cache identities. Both preparations consume
the same immutable semantics, not each other's results. CPU preparation emits
coverage; GPU preparation flattens device curves independently for f32 device
coverage and retains explicit CPU coverage for ineligible paths and clips.
Normalization storage and work contribute to preparation statistics; the live
IR overlaps backend preparation scratch and therefore increases its admitted peak.
Both renderers preserve command semantics and resource contracts without
requiring identical coverage values. Outputs are typed FrameBuffer (straight RGBA8 for transparent PNG), borrowed
external RenderTarget (opaque white-backed frames are straight RGBA8 FrameBuffers) or an ordered Encoder.

RenderOptions stores policy; RenderSession owns reusable caches, backend state and
request diagnostics. FrameRequest addresses a frame by index or exact time and supplies rational rate, duration,
background, target and pixel format. FrameRenderer wraps a session with Composition's
frame-index clock. Preview may coalesce requests and export may render every frame
without defining separate scene/render pipelines.

See [package ownership](architecture.md#packages-and-features) for implementation boundaries. Runtime modules
are divided by responsibility. Unit tests stay with their owning module, using
adjacent `tests.rs` files when large; cross-package tests live in the facade's
`tests/` directory. Only the facade assembles upstream public authoring APIs.
The engine forbids unsafe code; the first-party FFmpeg FFI crate owns codec unsafety.

GPU execution is split into `execute/submission`, `execute/textures`,
`execute/particles`, `execute/pending` and `execute/queue`: submission/packing,
texture computation and ordered blending, particle binning, completion/recycling
and bounded ordered scheduling stay independently readable.
The entry module only wires these children and its adjacent tests. Particle
style, batch/index and rendering remain separate, with protocol and raster tests
in `particles/render/`. The structure check guards these and the other completed
large-file splits at 800 implementation lines; pure test files are exempt.
Property tests and native fixtures are documented in [development checks](../development.md).
Criterion workloads and commands belong to [Benchmarks](../benchmarks.md#cpu-criterion-frames).

## Precision and Resources

Timing uses integer ticks and rational rates. Authoring decisions use f64; working
pixels are finite linear-sRGB premultiplied f32. Image mips use RGBA16F. All GPU
working color images use pooled RGBA32F textures, including masks and saved filter
sources; external RGBA16F output quantizes at its declared boundary. Only the final
working image resolves to a buffer for linear readback; packed and external-texture
outputs read that texture directly without the full linear buffer.
Particle metadata and coverage/paint uploads remain buffers. CPU determinism is
unchanged; GPU output is checked for visual equivalence with fixture-specific tolerances
in [GPU rendering](rendering.md), not byte identity, including replay on the same device and driver.

CPU memory output renders full-width adaptive regions with conservative filter
dependency halos. Preparation caches the maximum nested filter-chain halo;
independent siblings do not add their halos. Regions normally use 16 rows, grow
with the halo and can use a full-height linear intermediate when admitted. Tight
byte allowances shrink the region before retrying without refunding failed work.
See [rendering and export](rendering.md#execution-and-output-paths) for the policy.
Only the valid region is converted; final encoded planes stay owned until output Drop and then
return to a separately bounded shared pool. Layer/mask/filter scratch uses the
session pixel pool. Work is cumulative across regions; logical peaks include the
final output plus simultaneous prepared, capture and conversion payloads. Explicit
linear-frame and opaque-frame entry points remain available and are not implicit caches.

Paint preparation retains gradient geometry and pattern inverses. Row sampling
reuses vertical terms and advances exact bounded binary-grid products; unsupported
precision keeps direct scalar mapping. Interior coverage uses a canonical device
origin for ordinary paths and glyph contours, retaining fractional phase and full
geometry equality in cache keys. Clipping and unsafe coordinate ranges use raw keys.

`Canvas::instances` records an `InstanceBatch`; Rust evaluates all trajectory and
sprite-alignment mathematics. The optional checked `InstanceUpload` contains
generic 48-byte primitive records, not a GPU program or raw simulation state.
Consumers validate count and exact byte charge before device execution. GPU
support/tile metadata is independent storage and remains budgeted separately.

Structural quotas count occurrences, not unique shared pointers. Raster quotas
account for admitted logical work and live payloads, not allocator capacity,
driver memory, arbitrary callbacks or process RSS. All retained caches need separate
entry/byte bounds. Resource mutation creates a new snapshot identity.

Assets owns both preloaded values and lazy handles within one ResourceStore.
New resource stores isolate projects, while explicitly shared stores reuse matching sources and
accounting. RenderOptions.cache is a cloneable RenderCache shared by sessions and
export workers. Coverage, paragraph, glyph, gradient LUT, formula
and explicit author computation partitions each
have independent entry/byte limits and individual LRU eviction. SceneCache remains
the authoring computation store, shared through the Scene partition. Studio forks
that partition per registered scene while retaining other shared partitions. These caches are separate from decoded assets
and per-frame raster totals. Externally pinned Arcs and allocator overhead are not
hard-capped by these logical allowances.

## Execution Boundaries

Headless scene evaluation is synchronous and propagates callback panics. Studio
workers catch panics and retain typed errors until display. Preview requests coalesce;
exports render every selected frame. GPU contexts share a device and serialize scoped
submissions. Device loss aborts the frame rather than silently switching backends.

RequestDiagnostics can be shared explicitly with background resource reads through
Handle::get_with. DrawContext holds a shared immutable `Evaluation` (time, quality, resources, fonts, diagnostics and the RenderCache) plus device scale and validation limits; `SceneContext` shares one `Arc<Evaluation>` and keeps only local/global time, clip duration, frame rate, size and its `SceneCache`; `SceneContext::measure` hands that evaluation to a `DrawContext` with the local time. `cache_layout` always uses `CachePartition::Paragraph` of that single cache;
there is no FrameAssets or ambient frame diagnostic bridge. Request-owned Assets
views and their handles inherit the sink; older handles need get_with at the draw boundary.
RenderContext carries assets and structural limits; RenderOptions.gpu_context injects
a shared device. Studio can submit directly into same-device preview textures without
full-frame CPU readback. Output textures remain subject to device/target validation.

System font discovery indexes metadata; only selected sources are loaded lazily.
Explicit font files/bytes remain frozen at registration. ExportReport.fonts records
observed text face names and source-byte hashes, including cached glyph draws.
These noncryptographic hashes aid source auditing, not cross-toolchain raster identity.

Encoding commits a same-directory temporary output only after successful finalization
and cancellation checks. Timeline file audio is resolved separately into a
range-aware prepared mix; the native encoder receives that mix on its own thread.

## Packages and Features

The workspace root is a virtual manifest. Its default member is
[crates/motion](../../crates/motion/Cargo.toml): package `motion-rs`, library `motion`.
Root examples have explicit target paths. The unpublished benchmark package owns
its targets, fixtures and provenance build script, not a facade build script.

| Package | Responsibility | Engine dependencies |
| --- | --- | --- |
| motion-core | Time, geometry, animation, scene/layout/timeline, commands, resources, audio occurrences; optional `serde`/`schema` for plain value types | None |
| motion-render | Coverage, CPU/GPU composition, filters, sessions, output buffers, film looks | core |
| motion-content | Images, text, SVG, vectors, particles, optional ink/diagram/captions; feature-independent plain content policies with optional `serde`/`schema` | core |
| motion-formula | Typst document/math compilation and formula operations | core, content |
| motion-encode | PCM preparation, file streaming and transactional encoding | core, render; optional FFmpeg |
| motion-scene | Versioned JSON documents, their schema and compilation to timelines | core, render, content; optional content preparation features |
| motion-studio | Workbench, preview, playback and export orchestration | core, render, encode |
| motion-rs | Facade/prelude, Project/CLI and scene metadata | Feature-selected packages |
| motion-ffmpeg | Native FFmpeg bindings and thread-confined codec/mixer resources | None |
| motion-cli | The `motion` program for declarative JSON documents | facade |
| motion-bench | Engine/film benchmarks and provenance | facade, Gallery |
| motion-gallery | Authored short films | facade |

Only the facade assembles upstream public authoring APIs. Lower-level packages
import upstream types privately and must not depend on the facade or use public
preludes as internal import lists. Extension protocols stay at their owning crate
boundaries. Core carries immutable image/instance contracts and frozen font
snapshots, not content engines or devices. kurbo is a geometry dependency, not an
alternate renderer; Typst internals are isolated in the formula package.

The facade defaults to `gpu,image,svg,text,particles,ffmpeg,studio,ink,diagram,captions,declarative`.
Availability does not select GPU, discover fonts, open a window or play sound.
Gallery disables facade defaults and forwards its own selected capabilities.

| Feature relationship | Boundary |
| --- | --- |
| vector | Shared vector content and Morph without image decoding, fonts or particles |
| image | Includes vector; enables content image decoding |
| svg | Requires explicit image/text preparation for supported SVG content |
| particles | Depends on image, not text; independently selectable |
| diagram / captions | Depend on text; exposed as namespaced content tools |
| ink | Depends on image; exposed through vector::ink |
| typst / formula | Optional compiler / additional math authoring and morphing |
| audio / ffmpeg | Headless audio / native codecs and audio preparation |
| studio | Desktop workbench; enables FFmpeg, not automatic scene GPU selection |
| serde / schema | Core and content: serde and JSON Schema for plain value types and policies, enabled by motion-scene without enabling optional content engines |
| declarative | JSON documents; text, image and SVG elements follow the matching content features |

The facade's authoring, extension, rendering and eager namespaces have different
roles. Use the [tutorial](../tutorial/README.md) for current authoring and imports,
[development checks](../development.md) for isolated feature/CI validation and
[Benchmarks](../benchmarks.md) for benchmark target selection.

## Source Layout

Library entry modules declare children and expose public types. Formula and Studio
implementations live directly under src rather than another domain-named directory.
Pure test suites remain adjacent to their owner; large implementation splits are
guarded separately from test-file sizes.

- Core: authoring scene/node/modifiers/context/camera/skeleton, asset,
	color/math, drawing and shape. Path morphing belongs to path2d/morph.
- Render: prepared owns immutable records; raster owns shared preparation; cpu
	and gpu consume them. Composition timing, sessions and execution are separate.
- Formula: [compiler](../../crates/motion-formula/src/compiler.rs),
	[frame conversion](../../crates/motion-formula/src/frame.rs),
	[operations](../../crates/motion-formula/src/operations.rs) and
	[document compiler](../../crates/motion-formula/src/typst_asset.rs).
- Encode: [export orchestration](../../crates/motion-encode/src/export.rs) and
	[FFmpeg encoder](../../crates/motion-encode/src/encoder/ffmpeg.rs); the native
	transaction adapter is crate-private.
- Studio: app, panels, viewer, timeline, playback, session, preview worker and
	export modules; audio preview/settings/file preparation stay separate from core.

Performance work and feature extensions are prioritized in [the plan](../plan.md).
The [benchmark plan](../plan.md#performance) separates diagnostic engine cases from
real Gallery films. Their immutable samples feed one [performance report](../benchmarks.md#current-measurements)
with per-case previous-step and initial-baseline comparisons.
