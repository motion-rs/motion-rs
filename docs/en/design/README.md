# Design Contracts

These documents describe the current workspace's decisions, ownership and execution
contracts. They replace the monolithic design draft, not Rustdoc's exact signatures.
This index and its owning topics are the source of truth for current design
contracts. Historical reviews and unsynchronized translations are not alternative
contracts; proposed changes are not implemented capabilities.
Learn authoring through the [tutorial](../tutorial/README.md); consult
[Rustdoc source](../../../crates/motion/src/lib.rs) for current APIs.

## Topics

| Document | Responsibility |
| --- | --- |
| [Architecture](architecture.md) | Evaluation pipeline, package ownership, dependency direction and features |
| [Core authoring](core.md) | Numeric rules, time, animation, snapshots, layout and recording |
| [Content](content.md) | Assets, images, vectors, text, Typst and audio preparation |
| [Rendering and export](rendering.md) | Coverage, GPU device ownership, scheduling and completion, output formats, caches, budgets, concurrency and encoding; [Chinese translation](../../zh-CN/design/rendering.md) |
| [Export and FFmpeg](export.md) | Codec matrix, native encoder boundary, transactional output, cancellation and ordering |
| [Audio](audio.md) | Clips, buses, effects, master chain, analysis, synthesis and offline preparation |
| [Resources](resources.md) | Asset and font ownership, explicit IO, caches, limits and failure behavior |
| [Particles](particles.md) | Evaluated-instance protocol, ordered tiling and admission |
| [Reliability](reliability.md) | Failure, determinism, ownership and transaction guarantees |
| [Documents](documents.md) | Declarative JSON format, versioning, resource paths, compilation and diagnostics |

Maintenance commands belong to [development checks](../development.md), measurements
to [Benchmarks](../benchmarks.md), and remaining work to
[the development plan](../plan.md) (including its [performance section](../plan.md#performance)).
The [Studio guide](../studio.md) documents workbench operation and preview behavior.

## Glossary

The documentation uses these terms consistently; Rustdoc defines the exact types.

| Term | Meaning |
| --- | --- |
| Timeline | Tracks of scenes at a frame rate and logical size (`Timeline`) |
| Scene | A pure function of the scene context that returns a node for one time |
| Node | One element of an immutable authoring snapshot; modifiers return new nodes |
| Project | The facade registry (`Project`) that names timelines and provides the command line |
| Composition | Low-level fixed-output render configuration (`Composition`), used with a scene or a timeline |
| Display list | Flat validated drawing commands (`DisplayList`) compiled from a node tree |
| Frame IR | Internal admitted device-space curves, ordered scopes, paints, clips, filter domains and output window, without coverage (`FrameIr`) |
| Prepared frame | CPU coverage, paints and preparation statistics (`PreparedCpuFrame`); GPU preparation has its own result |
| Backend | The CPU backend or the GPU backend; GPU sessions may compute fill coverage independently |
| Still | One rendered frame, saved as an image or returned as a bitmap |
| Film | An exported video file, with or without an audio mix |
| Look | A reusable recipe (`Look`) pairing scene-side `SceneEffects` (filters, shutter, gate drift) with an output `DisplayTransform` |
| Document | A versioned declarative JSON description of a timeline (`motion::declarative::Document`) |
| Studio | The desktop workbench for previewing one compiled scene (capitalized) |

## Goals and Non-Goals

Motion is a programmable 2D video engine: evaluate exact times independently,
compose animated geometry/content, and produce stills or ordered encoded output.
Preview and export use the same authored timeline. Logical design coordinates are
independent of output resolution. Optional features expose capabilities without
implicitly choosing devices, discovering fonts or opening windows.

There is no 3D asset renderer, browser runtime, general-purpose UI layout engine or
interactive keyframe editor. Studio previews compiled Rust scenes; source changes
still require rebuilding, except that a declarative document reloads in place. It opens
one scene per launch, with no in-window scene switching. Declarative [documents](documents.md) describe data-only timelines; they do not
bind properties to Rust callbacks. `Transform` stays affine. Planar 2.5D perspective is
limited to `Perspective`, which projects path geometry; raster layers and images are not
perspective-mapped.

## Design Principles

1. Visuals are a pure function of explicit time and fixed inputs. Bounded caches
   retain complete-key results, not mutable playback history.
2. One authoring model produces immutable Node snapshots. Shared leaves reuse
   static preparation without freezing custom callbacks.
3. Traits serve open extension points such as Scene, Drawable, DeferredContent, Asset, AudioSource
   and Encoder. Closed command/backend protocols use exhaustive enums.
4. Backend-neutral frame facts precede backend-aware preparation. Eligible GPU
   fills use device-flattened curves or line uploads and GPU-generated coverage; other coverage stays
   on the CPU. Independent GPU work does not imply automatic speedup.
5. SVG, text and formulas share vector machinery, effects and coverage rather than
   separate rendering engines.
6. Numerical and resource contracts have explicit boundaries. Same-environment
   replay is not a promise of identical pixels on another platform or device.
7. Typed errors retain causes and frames. Successful file commit, memory-target
   replacement and in-place external textures have different guarantees.

Public APIs use complete descriptive names, keeping established acronyms such as
GPU, SVG, RGBA and YUV. Asset types distinguish decoded data from drawable content.
Combinators stay with their capability where practical; shared objects are normally
Send + Sync. The facade assembles public namespaces, not another implementation layer.

## Risks and Open Questions

| Risk | Boundary or response |
| --- | --- |
| CPU preparation limits GPU throughput | Profile preparation separately; cache proposals and arithmetic fast paths require their own correctness and timing evidence |
| Repeated node/path construction | Retain complete-key immutable preparation; do not replace dynamic scenes with rendered-frame history |
| Extreme stroke/curve geometry | Preserve structural limits, reference checks and deterministic failure |
| Font, compiler or driver changes | Freeze inputs, report environment identity and rerun applicable fixture gates |
| Typst dependency churn | Encapsulate compiler internals and bounded environment-aware caches |
| Native codec availability | Require native development libraries/codecs; PNG/GIF are distinct outputs, not subprocess fallbacks |
| Preview iteration and audio devices | Bound worker lifetimes; physical window/audio acceptance is separate from offline fixtures |

Historical milestone goals are not current timing guarantees. This design does not
claim universal GPU speedup, real-time 1080p rendering, bounded RSS or cross-device
acceptance. Current evidence lives in the benchmark report; missing gates stay in
the plans.

## Maintenance

Before every 0.x release, reconcile contracts with owning sources, feature forwarding,
removed symbols and migration notes. Validate local links and examples separately
from runtime/backend acceptance. Changes to pixels, arithmetic, scheduling or
lifetimes need the applicable replay and native fixtures, not just a documentation
scan. Keep each contract in its owning topic and link to it rather than duplicating
API declarations, benchmark tables or completion ledgers here.
