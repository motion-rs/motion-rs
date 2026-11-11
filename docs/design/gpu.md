# GPU Rendering

## Selection and Device Ownership

BackendPreference::Cpu uses the scalar renderer. Gpu requires a supported native
wgpu adapter; Automatic may choose CPU during startup policy checks and reports why.
An explicit GPU request does not silently fall back after failure or device loss.
`Gpu` and `Automatic` carry `allow_software`; software GPU adapters require it to be true.
`Renderer` only reports the backend that was actually used.

GpuContext owns the device, queue, pipelines, device-loss monitor and submission
serialization. Clones share that identity. Import an existing compatible device for
Studio or external textures rather than constructing one device per frame.

Context construction creates texture/particle and all supported output pipelines,
including packing, finite checks and RGBA16F/RGBA8/BGRA8 presentation. It completes
one empty submission under validation/OOM scopes before returning the context.
Clones reuse those pipelines; worker sessions do not repeat warm-up. This does not
prove that all driver first-use costs disappear. Automatic still uses startup
policy and a shared visual self-check, not a frame-latency calibration benchmark.
CPU and unsupported-precision early returns remain device-free.

The once-only self-check prepares CPU reference coverage and GPU edge records
independently. Its small fixtures cover spatial paints, group blending, curves,
self-intersections under both fill rules, subpixel contours, nested geometric
clips, filter guard bands and large-coordinate CPU coverage. Geometry fixtures
must actually contain GPU coverage. Comparisons check alpha and linear-light
compositing on black and white using fixture-specific RMS allowances; mutually
transparent padding cannot dilute errors. Replay uses the same visual comparison,
not byte equality. Success is shared by context clones but never hides later device loss.

## Shared Data and Output

FrameIr admits f64 device curves, segment upper bounds, immutable resources and
normalized composition semantics before either backend prepares coverage.
GPU preparation owns device edge records separately from public CPU PreparedFrame
values. FrameRenderer's explicit GPU preparation and packed export workers do not
construct a CPU prepared frame or initialize a device. Low-level APIs can still
consume an explicitly prepared CPU frame and upload its coverage.
GPU planning rejects unsupported precision or budgets before initialization. Ordered
draw/compute operations use finite linear-premultiplied pixels in RGBA32F working
textures. Particle states and ordered tile metadata remain storage buffers, as do
coverage/paint uploads and the final output boundary. These payloads are charged
explicitly. Eligible device curves retain statically Wang-sized descriptors.
A GPU pass expands those descriptors into the edge records consumed by coverage;
the host does not flatten or upload those curve edges. Line-only uploads remain
compatible. The GPU computes f32 coverage before composition, without an
intermediate u8 quantization. All other coverage is still generated on the CPU and uploaded.
Where contours cross inside a pixel the GPU sums signed areas rather than reproducing
the exact CPU sweep, allowing localized antialiasing differences. CPU rendering
prepares its own coverage with unchanged reference arithmetic. An Automatic startup
rejection prepares CPU coverage independently from the validated source before
rendering; both preparation attempts count toward the request's work allowance.
An already selected GPU never changes backends after a frame failure.

Mesh and atmosphere paints upload admitted control/parameter descriptions rather
than CPU-baked texels. CPU and WGSL independently evaluate patch geometry, Oklab
color interpolation, seeded atmospheric noise and envelopes. Explicit baking APIs
remain separate and preserve their prior arithmetic; selecting analytic mesh
paint can differ from a baked image's tessellation and sampling. See
[procedural paints](content.md#procedural-paints).

### Coverage tiles and preparation diagnostics

Coverage uses 16-by-16 pixel tiles. GPU passes count intersecting edges, prefix
the counts within each fill's disjoint index reservation, scatter edge indices,
prefix signed row heights and evaluate tile-local trapezoid areas. An edge enters
each tile intersected by its tile-band-clipped x range. Signed heights propagate
only after the edge's last tile; empty interior tiles therefore retain winding,
including holes and offscreen-left contours. Nonzero and even-odd fill rules use
the same unquantized f32 samples as composition. Edge uploads are relative to the
fill origin; area evaluation subtracts the tile origin for local arithmetic.

Before selecting GPU coverage, host preparation admits conservative tile counts,
index capacity and work from line endpoints or curve control-point hulls and
Wang segment upper bounds. One extra tile on each
horizontal side covers interpolation rounding at seams. Each tile admits at most
4096 edge references. A fill admits at most 2^26 tile work units and the frame at
most 2^28, 2^24 bounding-box pixels and 32768 fills. Coordinates outside the local
8192-pixel precision range, excessive tile density or these work/frame ceilings
choose CPU coverage for that path before submission. Exhausting the caller's
byte/work allowance remains a validation error, not an uncharged retry.

Planning reserves host/upload/device descriptors, tile counters and offsets,
the full conservative index capacity and 16 row carries per tile. The count,
prefix and scatter passes never request growth or a readback. Device buffer and
two-dimensional dispatch limits are checked before encoding. Per-fill/frame work
ceilings bound each coverage dispatch. Pooled counters are explicitly cleared
before every count pass; each row carry and output sample has one writer.

`PreparedGpuFrame::gpu_stats` exposes `GpuPreparationStats`: logical preparation
totals, retained GPU segments, admitted tiles/index capacity, geometry-upload
bytes and `GpuFallbackCounts`. Upload bytes count only binning uniforms and
fill/line/curve descriptors, not CPU coverage, paints, textures or zeroed scratch.
Fallback counts distinguish coordinate range, tile capacity, work/frame ceilings,
decal paint and disabled device coverage (including encoded-sRGB blending).
Analytic rectangles and CPU clip coverage are not fallback paths.
`RenderSession::report().gpu_preparation` exposes the same completed preparation
attempt for memory, texture and preparation-only requests, including partial
execution failure and Automatic startup rejection. A new request clears it.
The report's separate backend decision identifies an actual whole-frame CPU
selection; per-path counts do not imply a backend switch.

Each composition dispatch uses a 256-byte `#[repr(C)]`/Pod `GpuUniforms` rather than a raw
word array. Frame, bounds, operation, value rows, source rectangles and flags
start at byte offsets 0, 16, 32, 64, 192 and 240 respectively. The WGSL Settings
struct has the same aligned layout; ABI tests check size, offsets and Naga
validation. Shader assembly generates operation and blend constants from the
Rust discriminants and blend mapping, so WGSL switches do not duplicate numeric
tables. Value-row arithmetic and operation-specific detail/extent meanings remain
unchanged.

Instance uploads use checked renderer-independent Disc, Glow, Sprite and Segment
records: a 16-byte header, 48 bytes per primitive and optional shared sprite texels.
Rust-evaluated uploads remain compatible. Eligible built-in analytic trajectories
instead upload explicit seeds, times and model parameters; a GPU pass generates
the primitive records actually consumed by support, ordered tiles and composition.
Unsupported or imprecise analytic cases retain diagnosed CPU materialization,
not a backend switch. Upload, generated state and metadata storage are charged
separately. Fixed-step simulation remains on the CPU. See
[particles](particles.md) for count, work and replay constraints.

Coverage uploads retain a local integer rectangle rather than expanding every draw
to the entire canvas. Fills with strips refer to absolute ranges in one per-frame
upload of the original strip, span and packed u8 alpha arrays. Hardware Fill fragments
index their record directly from the quad instance, so they are charged like sparse
fills; a fill that must run as a compute pass, and any clip of at most 16 strips and
spans, scans the referenced ranges per pixel and is charged for that scan. Larger
clips use dense packed coverage, whose lookup does not grow with the record count; a
1080p transition with a few hundred clip records would otherwise exceed the work
budget. Fully opaque and span-only payloads
retain compact constants or sparse/packed records. Decals retain CPU f64 rejection
and compact local payloads. Decals keep CPU coverage; eligible GPU fills bypass
the CPU coverage cache.
Each six-word strip record includes a storage discriminator: CPU records address
packed u8 alpha bytes, while device records address absolute f32 words in the same
buffer. GPU coverage writes one sample per tile invocation; large frames use two-dimensional
dispatch. Coverage storage and conservative tile/edge work are admitted before submission.
CPU alpha decoding retains the original f32 values; device coverage stays f32 through
composition. Geometric clip ratios
retain their original f32 operation order. Conditional clips retain an eight-word
bounded header and three independent alpha/child/ancestor payloads, which can
themselves refer to the shared arrays, instead of CPU-expanded float pixels.
The shader evaluates `alpha * min(child / ancestor, 1)` with zero for a zero
denominator. Sparse offsets remain local to each embedded payload. Shared
uploads, staging and record scans are admitted explicitly; each pending frame
retains its own upload until completion. Record scans and conditional arithmetic
are conservatively charged per output pixel.
Spatial paint payloads follow the encoded coverage region; decal rejection retains
original CPU mapping.

External textures must belong to the same device, be single-layer/mip/sample 2D
render attachments, and use the declared supported output format. RGBA8/BGRA8 are
encoded outputs; RGBA16F is linear. Output conversion retains original alpha and
the shared deterministic dither policy. There is no CPU upload fallback for an
unsupported external target. Texture output reads a finite-status word, not full pixels.

GpuContext::submit_texture and FrameRenderer::submit_to_texture return a
PendingTextureFrame without waiting for completion. Call finish before publishing
the target: it checks its own submission, finite status, validation/OOM scopes and
device health. is_ready polls without a global queue wait. Synchronous texture
APIs wait on the same implementation. Dropped pending frames do not return their
in-flight resources to a pool.

Memory output performs synchronous readback; external texture output keeps pixels
on the GPU and reads only completion status.

RenderOptions.gpu_context is the explicit injection point; RenderContext owns
assets and structural policy, not a device. Attaching a device does not select the
composition backend. RenderSession owns backend decisions
and request-local diagnostics while GpuContext clones retain shared device lifetime.

## Working Textures And Batches

Native contexts query Rgba32Float attachment/blending capabilities and request
wgpu 24's adapter-specific format feature only when supported. All working color
images are pooled RGBA32F textures, including isolated layers, masks and retained
filter inputs. The old buffer-backed color-image executor and per-Fill copyback
path have been removed.

Eligible consecutive linear SourceOver Fills and SourceOver/Plus layer composites
with identical output bounds and no saved intermediate consumers share one output
texture and one ordered render pass. The pass initializes the backdrop from its
input texture, then shades instanced quads or layer rectangles with hardware
blending. When the backdrop's last consumer is this batch, bounds match, alpha is
not deferred, and no source aliases it, the batch loads and accumulates into that
same texture instead. A read-only compute pass first checks every logical backdrop
pixel for finite values; skipping the backdrop copy does not skip validation.
Intermediate results within that batch do not require separate texture
slots. Eligibility is classified once during planning; conservative source-payload
bounds of 65536 keep unbounded HDR sources on the checked texture compute path.
RGBA32F precision is retained rather than substituting half precision.

Hardware Plus adds RGB and alpha. A batch containing Plus marks its result for
deferred alpha saturation: texture input reads and final output conversion cap
only that marked alpha at 1, without rescaling HDR RGB. Unmarked compute outputs are
not unconditionally clamped. This preserves the alpha-saturating Plus contract
while allowing ordered SourceOver/Plus draws to share a pass.

Masks, filters, non-separable blends, non-linear blend spaces and saved intermediate
consumers form compute or batch boundaries, not texture-to-buffer boundaries.
These operations read working textures directly and write a distinct output
texture using the shared composition arithmetic. No layer recapture or intermediate
resolve is required, including when a filter later reuses its original source.
Devices without the hardware blend capability and contexts imported with
`from_existing` use a non-aliasing, texture-backed compute schedule. This is a
native GPU path, not a silent CPU fallback.

Only linear readback resolves the final working image to a linear output buffer.
Packed RGBA8, supported 8/10-bit YUV420 and external-texture output instead read the
final RGBA32F texture directly, applying the same finite-value check and deferred
alpha correction before output conversion. This avoids the full linear buffer
and its resolve pass without changing crop origins, quantization, readback,
completion checks or ordered queue behavior. Cropped packing also checks finite
values outside the crop in its packing dispatch. Cropped external-texture output
uses a separate parallel finite check outside the crop; fragments validate the
crop itself. Uncropped present needs no extra check pass. This preserves the
original whole-image error contract and the RGBA16F half-float range check.
YUV packing requires width divisible by 8 and even height; other valid even
dimensions retain linear readback and CPU conversion. Both depths use an 8x2-tile
dispatch with exclusive word ownership. Ten-bit YUV stores two little-endian u16
samples per u32, with limited-range Y in 64..=940 and U/V in 64..=960. Transfer,
grading and grain run before quantization; dither remains below half a sample
rather than scaling up from 8-bit. The three tightly packed planes total 3 bytes
per pixel and use byte strides. Packed/readback/owned storage is charged at the
selected depth; FFmpeg still consumes host planes, not zero-copy GPU surfaces.
There are no per-Fill
texture-to-buffer copies or row-alignment copyback passes. Coverage, paint,
uniforms, vertices and working texture capacities remain admitted; output plans
conservatively retain the old output-boundary byte/work allowance. Reported
logical peaks are therefore unchanged and are not actual allocation or RSS savings.

CPU/GPU comparisons require visually equivalent drawing with fixture-specific
tolerances, not a global LSB threshold or byte-identical GPU replay. Color, contours,
alpha and compositing on different backgrounds matter; local antialiasing and
floating-point differences are allowed. Missing geometry, clip leakage and visible
replay flicker remain defects. Existing stricter fixtures are local regression checks,
not restrictions on new GPU algorithms. The native batch
fixture separately checks linear/HDR output, sparse gradients, boundaries,
pool sharing, packed RGBA/YUV and ordered three-frame completion. Texture-layer
fixtures cover all four mask modes, local origins, HDR, filtered and saved sources,
Plus/Hue boundaries and exact replay. The full-texture fixture checks zero working color-image buffers,
actual hardware Plus execution, ordered batching and the compute-only path.
Offline scheduling tests cover saved sources, two-dimensional capacities and
distinct compute input/output slots. CPU determinism is unchanged. These checks
are not an FPS, RSS or cross-device performance result.

Intermediate images retain a global integer origin and bounds but use local row
strides and dispatch extents. Input reads outside each retained image return
transparent black, including blur taps and fractional displacement. Paint, noise,
clip, guard-band and output coordinates remain global. Fill/composite/mask passes
preserve the live backdrop region; the final image remains full-sized.

Dependency-aware last-use scheduling chooses two-dimensional texture slots before
submission. Free slots are selected by capacity and can grow independently in
width and height. Inputs, saved filter sources and final output remain live until
their last read. A batch output never aliases its live backdrop or source textures;
compute outputs likewise use distinct slots. Simultaneously live slots receive
independent leases even when their dimensions and usage match. Static admission
charges the larger total texture capacity of the batched and compute-only schedules,
not both totals or merely the current image area.

Pooled capacities can exceed a logical image's extent. Render viewport and scissor
match that logical extent, as do compute dispatch guards and global-to-local reads,
so unused capacity does not affect pixels. Empty regions use a minimum valid
allocation.

Completed working textures and uniform/data/status/vertex uploads can be reused
across frames by exact size/format/usage compatibility. Leases remain owned by each
pending submission until successful completion. Idle payload is limited to one
quarter of the frame allowance and at most 64 MiB per shared context, with at most
512 buffers and 32 textures. Bind groups are not pooled; this is not a zero-allocation
guarantee. Texture handles, buffer lease metadata and batch metadata are included
in static admission.

Immutable image/mip/atlas words instead use a separate shared context LRU.
Binding 11 reads the immutable payload independently of per-draw parameters,
coverage and instance state; repeated frames do not upload those words on a
cache hit. Exact encoded content equality follows hashing, so changed pixels
cannot alias an older resource. The default 64-entry, 64-MiB limit includes
host comparison words and device payload, independently of the mutable pool.
Frame admission conservatively covers a miss and bounded lookup work even for a
hit. Resident buffers have dedicated in-flight leases and never enter the mutable
working-buffer pool; eviction cannot overwrite pending frames. Explicit limits,
clearing and cumulative residency observations are described in
[resources](resources.md#limits-and-accounting).

Hardware draw batches aggregate uploads in explicitly admitted
`MAP_WRITE | COPY_SRC` staging buffers. After successful submission completion,
staging is asynchronously remapped for writing and shares the same idle byte and
512-buffer limits with other working buffers. Only a successfully mapped buffer
of the exact requested size can be reused. Pending mappings are skipped, failed
mappings are discarded, and a cache miss creates fresh mapped staging without
waiting or adding device polling to the upload path. Dropped or failed submissions
do not return their in-flight staging to this cache. Bindings and draw order are
unchanged; this reuse does not establish a frame-latency or allocation-free claim.

Instance rectangles within a hardware batch share one vertex buffer. Each draw
binds its own contiguous byte slice and uses local instance indices, preserving
the original quad order and shader inputs. The backdrop retains its separate
rectangle buffer; parameter and coverage bindings remain per draw. Uploads write
directly into staging or the shared buffer without a concatenated host array.
Parameter records also share one buffer when their 256-byte stride satisfies the
device's minimum uniform offset alignment and the full payload fits one buffer.
Each binding exposes exactly one record through a static byte offset and size;
there are no dynamic offsets or changes to the uniform or shader layout. Larger
required alignment or an oversized payload retains separate per-draw uniforms,
without extra padding or a new byte/work charge. The backdrop keeps its own
uniform record. Tests check both vertex and uniform lease counts and exact-size,
empty-payload and incompatible-alignment fallback selection.
When the complete instance payload exceeds the device's buffer-size limit, the
batch retains separate per-draw vertex buffers. Working leases keep either form
alive until successful completion. Shader arithmetic, binding layouts, uniform
ABI and vertex/uniform payload charges remain unchanged. Native batch tests
verify the shared-buffer count together with output tolerance and exact replay;
these checks do not establish a measured timing gain.

The experimental operation-data path aggregates binding 4 separately only for
hardware runs with at least three draws. One- and two-draw runs retain per-draw
data, so an admitted aggregate removes at least two buffers and staging copies.
This is a topology-based trial gate, not a scene-name or payload-value special case.
Planning
admits an eight-byte size entry and four layout work units per step, then tries
each eligible batch transactionally. Record starts are aligned to 256 bytes; the
last record has no trailing padding. Each binding retains the original record
length and local payload offsets. Padding adds its full span to both the working
buffer and staging admission, with one work unit per padded f32 word for
initialization and another for transfer. Metadata allocation/admission failure
disables aggregation without committing its costs; failed padding admission
leaves that batch on per-draw buffers without partial padding charges. Device
alignment and buffer-size incompatibility also retain the per-draw path, and
ordinary per-record storage-binding validation still applies.
Mapped staging zeroes gaps and encodes one operation-data copy per shared batch.
Without staging, the queue still writes each original record separately. There
is no payload deduplication or special handling of `[1.0]`, and no dynamic offsets
or bind-group reuse. Shared data remains owned by the pending submission. Layout
metadata/work remain admitted even when every run is below the selection threshold;
short-run fallback avoids padding costs, not all planning overhead.

For an eligible batch with N draws and Q uploaded instance records, vertex sharing
reduces vertex buffers from N to one while retaining 16Q payload bytes. Uniform
sharing reduces uniform buffers from N to one while retaining 256N payload bytes.
Together they remove 2(N - 1) working-buffer leases and, with mapped staging,
2(N - 1) buffer-copy commands. Buffer acquisition calls decrease, but fresh
allocations depend on pool hits. The non-staging path still writes each original
record through the queue. Operation-data aggregation additionally removes
N - 1 data-buffer acquisitions/leases and staging copies for each admitted batch,
but increases its data/staging span by the alignment padding. Backdrop uploads,
per-draw bind groups, texture capacities and draw calls remain unchanged.

Working-lease reservation and per-draw binding metadata are unchanged; fewer
active leases do not imply smaller reserved host containers. Conservative
vertex/uniform charges are unchanged; operation-data layout and padding are
charged separately. Exact-size pooling can retain a different
mix of buffers under the same byte and object caps, so idle retained bytes need
not decrease. In-flight submissions still own independent buffers. Native tests
assert unchanged vertex/uniform/original-data payload totals, admitted padding,
reduced data-buffer/copy counts, unchanged bind-group counts, exact replay and
forced per-draw fallback parity. These are not measurements of driver allocation
overhead, physical GPU memory or process RSS.

The original unrestricted aligned-data cohort improves node-heavy GPU P50/P95
but regresses wide-layer GPU P50/P95 on Apple M3/Metal. A separate six-pair
diagnostics-disabled AB/BA cohort tests the three-draw gate against the same
original shared-vertex/uniform control, not against that regressed candidate.
Node GPU P50 improves by 47.00% and 66.56%; wide-layer P50/P95 intervals cross zero,
so the earlier wide-layer regression is not reproduced in this cohort. CPU 1k-node
P50 still regresses by 1.92%. The current policy remains experimental, not accepted
as a regression-free general optimization. Logical peak bytes/work still increase;
the wide-layer increase drops from 92,832 bytes/24,804 units to 6,432 bytes/3,204 units.
Both complete cohorts, including negative pairs and tail outliers, are retained
separately in [the performance history](../performance-runs.json). This does not
isolate the threshold's causal effect, prove all three-draw workloads profitable,
or establish cross-device, overall speedup or physical-memory benefits.

## Stage Observations

`RenderSession::enable_timings` enables separate host observations for evaluation,
compilation, coverage preparation, GPU planning, command encoding/submission,
completion/readback and output conversion. CPU banded output
reports composition and conversion as one fused stage rather than fabricating
independent durations. Command encoding is not video/audio encoder timing.

Supported devices negotiate `TIMESTAMP_QUERY` only: each timestamp is the start of a
marker compute pass, so no feature that writes between commands is needed. Apple GPUs that
sample only at encoder boundaries advertise `TIMESTAMP_QUERY_INSIDE_ENCODERS` through an
emulation in which a native frame with timestamps never completed, so it is not used.
Imported devices use only their existing features; unsupported devices keep rendering and
report no device timestamps.
An enabled submission owns six queries and two 48-byte resolve/readback payloads.
This fixed 96-byte diagnostic payload is reported separately from raster totals;
query objects, handles and driver allocation overhead are not physical memory
measurements. Disabling observation allocates no query or query-readback resources.

Actual GPU timestamps bracket composition/filter execution, final output
packing/resolve/presentation, and output/status readback copies. They do not measure
host waits, mapping callbacks, pixel extraction or hardware queue latency. Query
resolution/readback is outside those intervals. Existing submission completion
drives query mapping without another device wait. Per-submission handles isolate
in-flight results, including reverse completion; results are published only after
the owning frame passes its existing device, scoped-error, finite-value and output
checks. Dropped or failed frames publish no timestamps. Studio Stats separates
these device intervals from host submission/completion wall time.

The consistency choice retains RGBA32F working images and full runtime
error checks in every build. CPU/GPU comparisons and GPU replay use visual
equivalence with fixture-specific tolerances; they do not require byte identity.
CPU reference regressions retain their existing exact checks.
Observation does not resume paused benchmarks or establish latency, FPS, bandwidth
or speedup acceptance.

## Limits and Validation

`FrameQueue` reserves each submitted plan's logical peak, not its configured byte
ceiling. Packed output uses `OutputPlan::peak_bytes` to include output/readback resources
and validate work before submission. The plan itself is crate-private; callers plan through
`OutputPlan`, `FrameQueue::submit` and `Gpu::render_linear`. Queues still admit at most three frames and
reject excess aggregate bytes before executing a plan.

Dedicated packed plans reserve output resources before optional batch-data padding.
They omit unused RGBA32F mapped readback and CPU linear-frame reservations, but retain
actual RGBA32F working/output-boundary textures. Packed storage, mapped readback and
owned CPU output coexist, so all three payloads remain charged, alongside status and
output metadata. Output size/format are fixed by planning; submission cannot charge
them twice or reuse a packed plan for floating-point readback or external textures.
Conservative plans remain compatible with packed submission and retain their float
reservations. Both modes include the extra host plan metadata, not physical memory.

Memory output and GPU exports use dedicated packed planning. Export workers retain
their independent construction allowance and full submission limits. Required packed
resources cannot be displaced by optional batching; optional layout fallback remains
transactional. Six diagnostics-disabled pairs at 960x540 on Apple M3 reduce GPU logical
peak by 16,588,832 bytes per breadth case. Only the 10k-node GPU P50 interval excludes
zero (+6.09% [0.24,12.23]); all CPU P50 and all P95 intervals remain uncertain. The
unchanged particle-garden frame 203 now passes the default allowance and exact replay,
where the frozen control fails at `gpu.pack`. This is budget-capacity evidence, not
physical GPU-memory, RSS, general speed or export-throughput acceptance. Full results
and the separately validated post-freeze export wiring are retained in
[the performance history](../performance-runs.json).

GPU exports, explicit or an Automatic request whose startup selection chose the GPU,
prepare and submit the first selected frame normally. Its
preparation/planning maximum and packed-plan peak size separate CPU worker slots
and the GPU queue. Bounded workers prepare, build an `OutputPlan` and read its peak;
the ordered consumer only admits, submits and completes output for encoding.
Planning charges conservatively include device/staging reservations as well as
CPU data. `OutputPlan::with_planning_limit` limits construction independently of the
original submission allowance. Worker count is not fixed at two or derived from
the configured raster ceiling. Each completed plan is checked against in-flight
bytes; the oldest output is completed when the queue needs space. If a later frame
cannot fit preparation/planning in its slot or the queue, workers are joined and queued outputs
are drained before unwritten frames resume serially under the original allowance.
Plans keep the original GPU submission limit; the smaller slot limit bounds CPU
preparation and device-independent planning. Only slot byte-admission failures
trigger this retry; invalid content, work-limit errors and panics remain failures.
CPU/GPU policy, packing, frame order and transactional failure
handling are unchanged. Unsupported packed formats and an Automatic request that selects
the CPU retain the CPU memory-output path.

Raster limits count logical live payloads and conservative work, not physical VRAM
or process RSS. Local image/attachment capacities, metadata, uploads, staging and
output are charged before submission. Coverage/clip/paint uploads, final output,
readback and synchronization can still dominate large scenes. Further pooling
and lower-copy output work remains [planned](../plan.md#performance). Measured bounds,
paired gains, uncertain/negative results and verification scope are recorded only
in the unified [performance report](../benchmarks.md#current-measurements).

GPU fixtures compare against CPU and check visually stable replay with declared tolerances.
They cover clipping, masks, blending, filters, text/image/vector/particle output,
external textures and device failure. Font and hardware inputs are explicit;
compilation does not establish adapter availability or cross-platform pixel parity.
