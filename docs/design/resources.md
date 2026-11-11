# Resources and Fonts

This page records who owns resource bytes, how long they live, and what makes a
cached value reusable. Nothing is read from disk, scanned from the system or
decoded on a device unless the author asked for it. The tutorial walkthrough is
[content](../tutorial/06-content.md); content-specific decisions are in
[content](content.md), admission limits in [reliability](reliability.md) and
[rendering](rendering.md#resources-and-content).

## Ownership Model

| Object | Owns | Lifetime |
| --- | --- | --- |
| `Assets` | Typed entry table, `LoadContext` (absolute root, optional `FontEnvironment`, namespace) | Clones share registrations; `snapshot()` freezes resolved entries and isolates later edits |
| `ResourceStore` | Logical byte counter and weak entry index | Shared by clones; a new `Assets` owner starts with its own store unless `with_store` is used |
| `Handle<A>` | One shared entry: source, sticky result, charged bytes | Until the last handle/owner drops; dropping releases the store charge |
| Decoded `Arc<A>` | Immutable decoded value | Can outlive every owner and handle; it is then no longer store-accounted |
| `FontLibrary` | Font database, shaping engine, glyph/layout caches, usage log | Cloned libraries share one frozen snapshot |
| `RenderCache` | Compute cache: seven independently bounded partitions | Shared by clones, sessions and export workers until `clear` |
| `SceneCache` | Compute cache (LRU) of pure author computations keyed by call site, type and key | Per cache; entries are LRU-evicted |

`ResourceStore` keeps only `Weak` references: it does not keep decoded data alive.
`Assets::remove` unregisters a source without invalidating existing handles, and
retained snapshots or handles keep their payload charged until dropped. Sources
are [`asset.rs`](../../crates/motion-core/src/asset.rs),
[`handle.rs`](../../crates/motion-core/src/asset/handle.rs) and
[`store.rs`](../../crates/motion-core/src/asset/store.rs).

## Explicit-IO Policy

- `Assets::new(root)` captures an absolute root (`std::path::absolute`) and
  performs no read or decode. `Assets::default()` has an empty root.
- A relative `Source::File` with an empty root is a failed handle ("require an
  explicit project root"); an empty owner does not authorize working-directory
  access. A relative path containing `..` is rejected; absolute paths are accepted
  as given.
- `Assets::load`, `AssetsExt::image`, `Image::open` and `Svg::open` only retain a
  source. The first `Handle::get` (or compilation of the scene that draws it)
  reads and decodes.
- File reads require a regular file of at most 64 MiB (`Asset::decode` receives
  bytes, never a path). `Source::Bytes` keys must be 1..=1024 bytes without
  control characters; the bytes take part in cache identity, so reusing a name with
  different bytes cannot return the wrong asset.
- `FontLibrary::new()` is empty and never touches the filesystem. System fonts
  are available only through the explicit `load_system_fonts` and
  `load_system_family` calls; they index metadata and read font files on first
  use. Deterministic projects should call `load_file`, `load_dir` or `load_data`
  and freeze those bytes instead.
- SVG rejects non-`data:` external `href`s, scripts, `foreignObject` and
  animation elements; SVG text requires a non-empty `FontLibrary`.
- `FormulaCompiler::new` and `TypstCompiler::new` use bundled fonts, no system
  discovery, and fail on imports, external files and packages.

## Fonts

A `FontLibrary` is copy-on-write: every mutation (`load_*`, `default_family`)
produces a new `snapshot_id` and resets the shaping engine, layout cache and usage
log. Clones share the previous snapshot. `load_data` admits at most 4096 faces and
256 MiB of source bytes per library and is atomic: an invalid payload changes
nothing. `load_dir` sorts entries, does not follow symlinked directories and
visits at most 4096 entries. `load_file` accepts regular files up to 256 MiB, while
the lazy `FontFace` asset (`AssetsExt::font`) is capped at `MAX_FONT_FACE_BYTES`
(32 MiB). System metadata indexing is capped at 65,536 faces. The first loaded
face becomes the default sans-serif, serif and monospace family.

Core never initializes a font engine. It carries an opaque `FontEnvironment`
(`Arc<dyn Any + Send + Sync>` plus `snapshot_id`); only `motion-content` can
downcast it. `Assets::with_fonts` derives a new owner view with an empty entry
table, so previously resolved SVGs are not reused with a different font set.
A `Text` bound with `Text::fonts` overrides the `DrawContext` environment; layout
needs one or the other. See [`font.rs`](../../crates/motion-content/src/font.rs).

## Cache Identity

| Cache | Key includes |
| --- | --- |
| `Assets` entry | asset type, absolute root, font `snapshot_id`, snapshot namespace, `Source` (bytes included) |
| Paragraph layout | text/style/width plus font `snapshot_id` |
| Glyph outline | font `snapshot_id`, face id, weight, glyph |
| Formula | source, size, family, fill, caller `revision`, `FORMULA_BACKEND_VERSION`, compiler `environment` |
| `SceneCache` | call site, key type, key equality (after hashing) |

Anything that changes decoding or layout must be in the key. Pure scene values
should put resource revisions in the author key. `SceneCache` builders may run
more than once (concurrent misses, eviction, `clear`, disabled cache), so they must
be pure. Sharing a `ResourceStore` never lets owners with different roots, sources
or font snapshots reuse each other's entries.

## Limits and Accounting

All byte limits are **logical** payload estimates, not process RSS: allocator
overhead, parser scratch and external `Arc` pins are excluded.

| Scope | Default | Notes |
| --- | --- | --- |
| Encoded asset input | 64 MiB | Checked on file metadata, bytes and after the read |
| `ResourceStore` | 256 MiB, 4096 entries | `with_limits(ResourceLimits { max_bytes, max_entries })` requires positive bytes and 1..=4096 entries; it is independent of the compute caches' `CacheLimits`; at least the encoded length is charged |
| Image decode | 16384 px per side, 16,777,216 pixels | Static PNG/JPEG/WebP only; stored as linear-premultiplied RGBA16F with mips |
| SVG source | 8 MiB, XML depth 64, 100,000 nodes | DTDs disallowed |
| `RenderCache` partitions | Coverage 2048/16 MiB, Paragraph 256/8 MiB, Glyph 2048/16 MiB, Gradient 64/8 MiB, Formula 32/32 MiB, Ink 64/32 MiB, Scene 256/8 MiB | `RenderCache::with_limits` applies one limit to all; `fork_partition` isolates one |
| `SceneCache::new` | entries only | Byte accounting requires `with_limits` plus `cache_with_cost`; oversized values are returned without retention |
| `FormulaCompiler` | 32 entries, 64 MiB | Source up to 8192 bytes |
| `RasterLimits::authoring()` | 16,777,216 px, 256 MiB live bytes, no work cap | `sandbox()` adds 200,000,000 work units |

`Budget` counts admitted raster pixels, live bytes (with peak) and work for one
operation; callers release transient bytes, work is never refunded. These limits
bound preparation, not Rust callbacks or Typst evaluation time. See
[`budget.rs`](../../crates/motion-core/src/budget.rs) and
[`cache.rs`](../../crates/motion-core/src/authoring/context/cache.rs).

## Deferred and Eager Content

| Deferred (retains source/options) | Eager (does the work now) |
| --- | --- |
| `Assets::load`, `Image::open`, `Svg::open`, `Svg::new(handle)` | `ImageAsset::from_bytes`, `SvgAsset::from_bytes_with_fonts`, `Assets::preload` |
| `Formula::typst` | `Formula::compile`, `FormulaCompiler::compile` |
| `Text` shaping at compilation | `Text::layout(&fonts)` for inspection |

`Handle::state()` returns an `AssetState` without loading; it moves `Unloaded` → `Loading` → `Ready(Arc<A>)` or `Failed(Arc<Error>)`. Failure is
sticky and never retried implicitly. Concurrent
callers join one decode; a panicking decoder becomes a sticky asset error.
Recursive resolution or nesting deeper than 64 fails. `Formula` without an
explicit compiler uses a process-wide bundled-font `FormulaCompiler`, so it does
not see fonts registered through `Assets::with_fonts`; use `FormulaCompiler::with_fonts`
for custom math fonts.

## Threading and Poisoning

`Asset`, `FontEnvironment`, `Assets`, `Handle` and `RenderCache` are `Send + Sync`
(or require it of their contents), so workers share them freely. Policy by
object:

- `Assets`, `ResourceStore`, `RequestDiagnostics`, `FontUsage` and the font usage
  log **recover** from a poisoned lock and continue.
- `SceneCache` treats a poisoned lock as a bug and panics.
- A poisoned font engine returns a `ValidationError` ("Font engine is poisoned");
  a poisoned formula cache returns `FormulaCompileError`; a poisoned sound
  declaration lock returns a validation error.
- Computation runs outside cache locks, so a slow builder does not block other
  entries.

## Failure and Diagnostics

Failures carry the asset kind and source name. `Handle::get_with` writes into an
explicit `RequestDiagnostics`, deduplicated by `Arc` identity, which is safe to
pass across threads; there is no ambient per-frame context. Production
rendering returns the first resource diagnostic as an error tagged with the frame
unless `RenderOptions::allow_missing_assets(true)` is set (Studio's preview does
this); `Image` then draws a magenta placeholder and records the diagnostic.
`RenderOptions::mark_invalid_geometry(true)` is the matching policy for geometry:
a leaf whose geometry fails validation is drawn as a magenta square at its local
origin instead of failing the frame. Studio's preview enables it; stills, export
and the CLI keep the strict default.
`ExportReport.fonts` lists the faces actually selected, with non-cryptographic
source hashes, ordered independently of worker completion.

## Supplying Assets from Studio and the CLI

Both entry points consume the `RenderOptions` registered on the `Project`
(`Project::render_options`), which carry `resources: Option<Assets>`, the
`RenderCache` and `RasterLimits`. The CLI has no assets-root or font flag (see
`--help` in [`cli.rs`](../../crates/motion/src/project/cli.rs)), and `studio`
builds its composition from the same options, so a project chooses its root and
fonts in code:

```rust,no_run
use motion::prelude::*;

fn main() -> motion::Result {
    let mut fonts = FontLibrary::new();
    fonts.load_file("assets/project.ttf")?;
    let assets = Assets::new("assets")?.with_fonts(fonts);
    let _options = RenderOptions::default().resources(assets);
    Ok(())
}
```

Compositions cloned from these options share the `Assets` owner (and its resource store);
sessions keep separate targets and caches.

## Related

- [Tutorial: content](../tutorial/06-content.md)
- [Content contracts](content.md), [reliability](reliability.md),
  [rendering](rendering.md#resources-and-content)
- [`asset.rs`](../../crates/motion-core/src/asset.rs),
  [`context.rs`](../../crates/motion-core/src/authoring/context.rs),
  [`partition.rs`](../../crates/motion-core/src/authoring/context/partition.rs)
- [`font.rs`](../../crates/motion-content/src/font.rs),
  [`options.rs`](../../crates/motion-render/src/options.rs),
  [`compiler.rs`](../../crates/motion-formula/src/compiler.rs)
