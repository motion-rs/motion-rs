# Content Contracts

Images, SVG, text, formulas and authored artwork use the same scene and rendering
pipeline. This document records preparation and ownership decisions, not a second
set of public API signatures. Start with [content](../tutorial/06-content.md),
[formulas](../tutorial/08-formula-typst.md) and [audio](../tutorial/10-audio.md).

## Assets and Images

Assets owns preloaded values and lazy typed handles in one explicit ResourcePool.
Clones share registrations; snapshots isolate later edits. Handles retain shared
loading/success/failure results and their logical charges. Removing a registration
does not invalidate existing handles. Relative files require a configured root;
an empty owner does not authorize implicit working-directory access.

Image::open and Svg::open retain sources without decoding at authoring time.
Compilation uses the session owner and font snapshot. Strict production output
rejects missing resources; explicit placeholder policy retains typed diagnostics.
Custom decoded data implements Asset, while pure non-resource computation belongs
in the scene cache. Pools, caches and external Arc holders have distinct lifetimes.
See [resource accounting](rendering.md#resources-and-content).

ImageAsset retains immutable linear-premultiplied image data and bounded mipmaps.
Fit, mapping, sampling and rounded presentation belong to the drawable, not a new
decoder. AnimatedImageAsset retains composed GIF/APNG or explicit timed frames.
Tick lookup, loop, ping-pong, trimming and reversal are stateless source-time
operations; endpoint durations are not duplicated. Image storage, GPU working
precision and output precision are separate boundaries.

## SVG and Vector Trees

SVG uses usvg's parsing layer, not resvg rendering. Supported paths, groups, images,
text, clipping, masks and filters convert into Motion's shared vector/command model.
Path types from parser dependencies are converted at the boundary. Unsupported
semantics retain diagnostics or fail; there is no hidden browser or bitmap renderer.

VectorTree retains group transforms, opacity, IDs, paths, paints and image content.
Element selection modifies ordinary Nodes. SVG, text and formulas share vector
writing, geometric morphing and surface sampling rather than each owning an effect
engine. Stroke writing uses arc length; fill reveal and temporary outline behavior
remain part of the chosen effect. Morphing needs explicit leaf/subpath correspondence
and must handle unmatched geometry rather than assuming compatible topology.

Seeded surface sampling uses filled geometry, not arbitrary bounding-box samples.
Sampling prepares particle origins/targets without introducing a second simulation
runtime. Caching a VectorTree does not cache rendered frames or bypass admission.

## Fonts and Text

FontLibrary owns an explicit font database shared with text and SVG. A new library
is empty. Project files/bytes are immutable registrations; system discovery is an
explicit metadata-indexing step, with selected sources loaded lazily. Deterministic
projects freeze their inputs and fallback choices rather than depend on discovery
inside scene callbacks.

One paragraph engine uses cosmic-text for shaping, wrapping, bidi and fallback;
skrifa supplies glyph outlines. Glyph artwork follows the shared coverage pipeline,
not a second size-dependent text renderer. Font identity and variation coordinates
are part of glyph/cache identity. Unsupported color-glyph formats are not implied
by ordinary outline support.

Paragraph layout retains measured lines, baselines, glyphs, grapheme boundaries and
source information. Complete text/style/width/font-snapshot keys govern layout
reuse. Position and paint changes need not reshape text. Per-part transforms reuse
layout and the inherited scene-local clock; visible parts must not split UTF-8 or
graphemes. Explicit eager APIs support inspection and truncation before rendering.
Unsupported combined effects retain diagnostics rather than silently dropping one.

ExportReport.fonts records actually observed faces and source-byte hashes, including
cached draws. These are auditing aids, not cryptographic identity or a promise of
identical glyph rasterization on another platform.

## Typst and Formulas

The typst feature compiles trusted single-page markup; formula adds math authoring,
selection, writing and morphing. Heavy compiler dependencies stay in motion-formula.
Formula::typst retains source/options without initializing a compiler or reading
fonts. `size` and `fill` create a new immutable compile key (source plus options) with its
own empty result cell, so a stale compilation is never reused. There is no LaTeX
conversion API or on-disk formula cache.

A frozen compiler environment produces vector glyphs and geometric marks while
retaining source ranges and rendered symbol occurrences in FormulaAsset. Source,
style and environment identity govern bounded in-memory reuse. Lazy rendering
uses the shared formula cache partition; eager compilation provides metadata and
precise selection before rendering. Failed compilation/selection keeps node paths
and frame diagnostics. FormulaMorph delegates geometry to the shared vector model.

Typst is trusted code, not a sandbox for unbounded external markup. Compilation,
font loading and cache lifetime are preparation responsibilities, not per-frame
I/O hidden inside an unrelated renderer.

## Authored Tools and Audio

Ink, diagrams and captions are optional content tools. Ink preparation retains
path, pressure, load and material continuity; procedural bleed/grain/pooling is not
a physical fluid simulation. Draft may use approximations confined to preview.
Charts, rulers and callouts keep f64 units and explicit fonts. Captions precompute
safe-area layout and use exact half-open cues; narration retiming does not stretch
audio or rebuild derived layouts automatically.

AudioClip is an occurrence over a shared random-access AudioSource. Start, source
offset, trim and envelopes have independent clocks. File sources remain lazy and
must stay unchanged during execution; explicit PCM freezes samples. Offline
decoding, resampling, mixing and analysis need no playback device. Prepared traces
avoid repeated I/O/FFT work; RMS is not LUFS or a causal microphone measurement.
Studio owns optional real-time playback separately from headless authoring.
Encoding, mix bounds and transactions are in [rendering](rendering.md#video-and-audio).

## Owning Sources

- [Assets](../../crates/motion-core/src/asset.rs)
- [Content](../../crates/motion-content/src/lib.rs)
- [Fonts](../../crates/motion-content/src/font.rs)
- [Formulas](../../crates/motion-formula/src/lib.rs)
- [Audio preparation](../../crates/motion-encode/src/audio.rs)
- [Particle instances](particles.md)
