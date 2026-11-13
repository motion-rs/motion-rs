# Particle Instance Contracts

Use the [particle tutorial](../tutorial/07-particles.md) for trajectories, simulation,
styles and examples. This document owns CPU/GPU instance protocols and admission.
Particle is a 72-byte state record with finite linear-premultiplied color, distinct
from the 48-byte upload primitive and the independent 48-byte GPU support metadata.
There is no second Layer-based runtime or GPU trajectory-program interpreter.

## CPU Composition

Built-in batches scatter conservative per-instance supports into 16x16 device
tiles. Each tile retains ascending original instance indices; sampling uses the
same scalar coverage/compositor and preserves zero-source identity and nonzero
order. Glow's existing three-radius cutoff, rotated sprites, line/trail extents,
antialiasing and numeric padding remain intact. Bounds that cannot be handled
reliably fall back to the full-batch path. Other InstanceBatch implementations
use that path unless they explicitly supply SpatialInstances.

Tile metadata and index entries count toward logical live/peak bytes. Work charges
cover bin construction and tile-instance pixel visits, rather than every instance
at every output pixel. This changes algorithm-work admission, not the configured
budget. Failed renders do not commit a partially updated public output frame.
No CPU/GPU equivalence or real-time guarantee follows from reduced work totals.

## GPU State Preparation

The legacy upload path supplies Rust-evaluated primitive positions, radius,
color, rotation, kind and independently sampled segment endpoints through
`Canvas::instances` and `InstanceBatch`. It remains the boundary for arbitrary
custom trajectories and CPU fixed-step simulations.

Eligible built-in analytic trajectories additionally supply admitted explicit
seed/time/model descriptions. A GPU pass evaluates those descriptions into the
48-byte primitive records that are then consumed by support preparation,
binning and composition. It is not a disconnected diagnostic calculation:
eligible analytic rendering does not upload CPU-evaluated primitive positions.
Precision or unsupported-model cases retain diagnosed CPU materialization,
including an analytic fallback count, without changing the selected backend.
The CPU reference and keyed streams remain unchanged.

Analytic uploads admit at most 1,048,576 instances with
`0 <= age < lifetime <= 3600` seconds. Numeric magnitudes are bounded by one
million and temporal/noise phases by 8192 for the f32 device path. Precision
rejection uses the `Unsupported` diagnostic at `instances.analytic.precision`;
the batch's `analytic_fallback` observation and preparation's
`analytic_cpu_fallbacks` count distinguish it from a backend change.

A separate 64-thread compute pass derives conservative local support in a
48-byte metadata record from either form of primitive storage.
Batches below 256 instances retain the full-loop path to avoid fixed
binning/synchronization overhead. Larger batches use ordered 16x16 tiles. When
the bitset layout and its conservative work bound fit the budgets, one 64-thread
compute pass assigns an invocation to each particle. It transforms the
particle's conservative support to device space and atomically sets that
particle's source-order bit in every overlapped tile. Composition enumerates
set bits from low to high, preserving source order without a GPU summary
readback. If the bitset is too large or cannot be admitted, the exact-size
count/prefix/fill list path remains available.

`InstanceUpload::new` accepts typed `InstanceRecord` values: Disc, Glow, Sprite or
Segment. The upload is a four-word header (record stride in the low five bits of word three,
the batch `InstanceBlend` above them) followed by twelve words (48 bytes) per
primitive and optional shared linear RGBA sprite texels. Each record contains
position, segment start, color, radius, rotation, segment width and kind; sprites
sample the full shared image. Records contain no age, velocity, seed, model or
simulation parameters. Count, dimensions, finite values, texel count and checked
allocation are validated. Planning checks the exact declared byte charge and
instance count. CPU-only batches default to no upload and are rejected by explicit
GPU consumers rather than silently falling back.

GPU planning separates immutable sprite atlas texels from the per-draw header
and records, for both ordinary and extended strides. Binding 11 consumes the
context-resident atlas, so changing time, record positions or draw transforms does
not upload its pixels again on a cache hit. The raw `InstanceUpload` protocol
still includes optional trailing texels; CPU consumers are unchanged.
Atlas host storage, worst-case device upload and comparison work are admitted
even on a hit. Device storage limits apply independently of cache occupancy.
See [resource residency](resources.md#limits-and-accounting).

The analytic protocol is a bounded set of supported models, not an arbitrary GPU
program interpreter. Custom trajectories need not implement it and retain their
existing Rust-evaluated boundary. Fixed-step simulation is not executed on the GPU.
`MAX_INSTANCE_COUNT` permits up to 1,048,576 uploaded primitives or analytic
trajectory states, replacing the old 65,536 protocol ceiling. Memory, work,
device-storage and index checks still apply; simulation and per-step emitter
quotas remain separate. Plexus uses deterministic spatial hashing up to 65,536 states, with pair and logical-work bounds.
A native regression renders and replays 65,537 instances. The 48-byte GPU metadata
record is separate charged storage, not a second name for the upload record.

Bit positions encode the original indices, so atomic writes do not introduce
unordered compositing. The complete state and bitset storage, worst-case
particle-to-tile build work and conservative maximum composition work are
admitted before execution. The fallback uses separate count, prefix and fill
passes to retain original instance order; it reads back a 16-byte summary before
allocating exact index storage and admitting actual tile-instance pixel work.
Each fallback reference has an independent 16-byte storage record to avoid
concurrent read/modify/write races between packed vector components. Metadata,
scan work, duplicate state storage and references all count toward logical
budgets. Index, device-storage or work overflow fails the frame before public
output commits; indices are never silently truncated. Returned frame stats
include path-specific work and storage; the internal GPU plan statistics
contain static admission charges only.

Tile composition has a separate shader-specialized pipeline. Ordinary scenes
and small batches do not execute the tile lookup. Scalar WGSL coverage, blend
order, numeric-error reporting and existing CPU/GPU pixel tolerances are unchanged.
Default-budget admission and replay checks are separate from throughput acceptance
and Gallery budget reduction. Current measured workloads and their provenance
belong to [Benchmarks](../benchmarks.md), not historical timings embedded in this contract.

## Extended Uploads

Original uploads keep header word three zero and twelve-word records unchanged. new_extended uses sixteen words with header word three equal to 16 + (blend_id << 5), exactly represented as f32. Consumers derive stride from the low five bits and compositing from the remaining bits; texture data starts after count*stride.

Extra words contain a UV cell, tapered start RGBA or analytic shape parameters. Tapered width is the start diameter; base radius/color are the endpoint. SpriteSheet stores stretch aspect in base width. New kinds do not renumber Disc/Glow/Sprite/Segment. Geometry, finite values, UVs and endpoint alpha are validated; HDR RGB is not clipped. Shape evaluation, CPU binning, upload and GPU storage retain independent budget gates. SourceOver/Plus/Screen/Multiply are batch policies; without an explicit blend, `GlowDots` uses Plus and every other style uses SourceOver.

Historical segments share one sample and ID lookup per time. One render queries those times in ascending order through a forward cursor held in the admitted working buffers: a query continues stepping from the previous one when it lies between the nearest retained snapshot and the target, and otherwise restarts from that snapshot. Fixed steps are deterministic, so results equal independent queries; the cursor is dropped with the render and never carries state between frames. A conservative 64 MiB logical history/index bound rejects excess storage without silently reducing segment counts. Interval/subdivision/count limits bound record generation. CPU tiled work uses declared per-instance sampling cost. Extended styles retain the 72-byte Particle and 48-byte GPU metadata. Style operations not represented by an admitted analytic upload retain their Rust-evaluated path; neither upload path moves fixed-step simulation to the GPU.

## Replay Extensions

Defaults keep old snapshots and keyed streams. Event/population/time/worker/intensity features select enhanced snapshots with birth-color/depth metadata. Nonrecycled IDs, ordered event queues and per-rule random keys make events independent of worker count. Logical snapshot admission includes events and working buffers; both loop phases share query work. Custom callback panics propagate, including joined worker panics.

SDF grids retain signed samples and exact cached boundary guards. Resolution/budgets are explicit, with speed*step below a grid cell. Cache keys include geometry identity, bounds, resolution and preparation budgets. Boundary guards protect resolved thin geometry but cannot recover curves omitted by declared surface flattening tolerance. Shared external pins follow existing SceneCache cost semantics.

Parallelism and reduced logical work do not certify real-time throughput. These contracts are covered by functional tests separately from timing benchmarks.
