# Export and FFmpeg

This page records the contracts of the export path: how a timeline frame becomes
pixels, how pixels reach an encoder, which codecs exist, and how output is committed
or discarded. [Rendering and export](rendering.md) owns coverage, caches and budgets;
[Reliability](reliability.md) owns the general failure model. The
[tutorial chapter](../tutorial/11-export.md) teaches the workflow (PNG sequence
example, `ExportOptions`, the encoder lifecycle, progress, pitfalls); this page states
the invariants behind it and does not repeat the walkthrough.

## Pipeline

```text
Timeline frame index -> Time::from_frame -> FrameRenderer::render_output_at
  -> FrameBuffer (in Encoder::input_format) -> Encoder::write -> Encoder::finish
```

[`export`](../../crates/motion-encode/src/export.rs) owns the
orchestration for built-in and custom encoders (`ExportOptions::encoder`); it runs on the caller's thread and always write frames in order.

| Stage | Contract |
| --- | --- |
| Preparing | Audio is prepared, options are validated and normalized (`selected`), destination and capability checks run before any output exists. |
| Rendering | Frame `n` is evaluated at `Time::from_frame(n, rate)`; the encoder's `input_format()` decides the pixel layout and it is never reinterpreted. |
| Finalizing | `Encoder::finish` flushes, then commits. `ExportReport` describes committed output only. |

`ExportPhase` notifications are ordered (`Preparing`, `Rendering`, `Finalizing`,
`Completed`); `Completed` is reported only after `finish` returns. `frame_completed`
counts frames handed to the encoder, not committed output.

## Native API only

Video and audio are encoded by calling libav* through
[`motion-ffmpeg`](../../crates/motion-ffmpeg/src/lib.rs) in process. No `ffmpeg`
executable is launched and there is no subprocess fallback; media inspection used by
tests ([`inspect.rs`](../../crates/motion-ffmpeg/src/inspect.rs)) is native as well.
PNG sequence and GIF encoders are pure Rust and do not need FFmpeg.

## Frame range and rate

| Rule | Source |
| --- | --- |
| `range` is half-open and must satisfy `0 <= start < end <= duration`. | [`render.rs`](../../crates/motion-encode/src/export/render.rs) |
| Start and end round up to the next frame timestamp; a range with no frame is rejected. The report carries the normalized range. | same |
| Output rate is the timeline rate, except `Codec::Gif`, which samples at its own `frame_rate`. | same |
| `workers` must be in `1..=64` and `max_parallel_bytes` positive. | same |

## Codec matrix

Verified in [`options.rs`](../../crates/motion-encode/src/export/options.rs),
[`settings.rs`](../../crates/motion-ffmpeg/src/settings.rs) and
[`ffmpeg.rs`](../../crates/motion-encode/src/encoder/ffmpeg.rs).

| `Codec` | Container / library | Input format | Alpha | Notes |
| --- | --- | --- | --- | --- |
| `H264 { quality, preset }` | MP4 / `libx264` | `Yuv420` 8-bit | no | CRF `0..=51` |
| `H265 { quality, preset, bit_depth, params }` | MP4 / `libx265` | `Yuv420` 8 or 10-bit | no | `BitDepth::Ten` selects Main10 (`yuv420p10le`); optional `params` x265 string (at most 4096 bytes, no NUL) |
| `Vp9 { quality, alpha }` | WebM / `libvpx-vp9` | `Yuv420` or straight `SrgbRgba8` when `alpha` | optional | quality `0..=63`; timeline audio is rejected |
| `ProRes4444` | MOV / `prores_ks` | straight `SrgbRgba8` (`yuva444p10le`) | yes | fixed quality, no tuning |
| `PngSequence` | directory of PNG frames | encoder-defined | straight | no audio, no container metadata |
| `Gif { frame_rate, colors }` | GIF | encoder-defined | transparent entry | `2..=256` colors; delays must fit `1..=65535` centiseconds |

Further constraints: encoder-specific settings live only on the codec variants that
use them (`preset` for H.264/H.265, `bit_depth` and `params` for H.265), so
`ExportOptions` carries none; `preset` is `medium`, `slow` or `slower`; the native encoder
requires even dimensions, at most 16,384 per side (`EncoderCapabilities`), at least
one and at most `u32::MAX` frames, and at most 86,400 seconds. Audio is AAC at
44,100, 48,000 (default) or 96,000 Hz. Selecting an alpha-capable codec, PNG or GIF
through `ExportOptions::codec` clears the default opaque black background; other
codecs flatten onto black. A codec whose library is absent from the linked FFmpeg
fails at `open` with `ErrorKind::Unsupported` (`VideoCodec::available`).

Color metadata: native video is tagged BT.709 primaries and matrix, limited range;
transfer is sRGB (IEC 61966-2-1) by default or BT.709 when `gamma24` is set
([`codec.rs`](../../crates/motion-ffmpeg/src/encoder/codec.rs)). MP4 output uses
`+faststart`, and the movie timescale and chapters use 705,600,000 ticks per second.
Container tags (at most 64) and chapters (at most 4096, ordered, non-overlapping) are
validated in [`encoder.rs`](../../crates/motion-ffmpeg/src/encoder.rs).

## Pixels

[`output.rs`](../../crates/motion-render/src/output.rs) defines the `PixelFormat`
contract shared by the CPU and GPU backends: `LinearRgba16Float`, `LinearRgba32Float`, `SrgbRgba8
{ premultiplied }` and `Yuv420 { bit_depth }` (8 or 10, even dimensions, BT.709
limited range, 10-bit as little-endian `u16`). `FrameBuffer` owns tightly packed
planes with byte strides. [`yuv.rs`](../../crates/motion-render/src/output/yuv.rs)
does 4:2:0 block conversion; the display-transform path applies a 4x4 ordered
dither when quantizing. An opaque `SrgbRgba8` `FrameBuffer` ([`rgb.rs`](../../crates/motion-render/src/output/rgb.rs))
writes `write_ppm` (P6) and `write_rgb`; they do not use FFmpeg
or the video overwrite policy and overwrite their destination unconditionally.

## Streaming and threads

Export never buffers the whole film. The native encoder runs on a dedicated
`motion-encoder` thread fed by a bounded queue of 2 frames
([`worker.rs`](../../crates/motion-encode/src/encoder/ffmpeg/worker.rs)); libav
objects stay confined to that thread, and a worker panic becomes `Error::Encoder`.

CPU workers use a private Rayon pool of `workers` slots
([`parallel.rs`](../../crates/motion-encode/src/export/parallel.rs)). Slot `i` renders
frames `i, i+workers, ...`; the export thread consumes results strictly by frame
index, so output order and bytes equal the serial result (covered by
`parallel_export_matches_serial_pixels_ntsc_ranges_and_progress`). The first frame is
rendered serially to measure its peak; the worker count is capped by frames and
`max_parallel_bytes`. If a later frame exceeds a slot's byte budget, only that
`max_bytes` structural failure triggers a serial resume from that frame; other errors
and panics propagate. Scene callbacks must therefore be repeatable.

The GPU path ([`gpu.rs`](../../crates/motion-encode/src/export/gpu.rs), feature
`gpu`) applies when the backend is not CPU, more than one frame is exported, the
format can be packed on the GPU, and a GPU context was actually selected. CPU
preparation slots feed a bounded `FrameQueue` whose results are completed and
encoded in order. An Automatic request that resolves to CPU keeps the CPU path.

## Transactions and overwrite

| Output | Mechanism |
| --- | --- |
| Video, GIF | Temporary file `.motion-video-*` / `.motion-gif-*` in the destination directory, committed with `persist` (overwrite) or `persist_noclobber`. |
| PNG sequence | Temporary directory renamed into place with `renameat_with`: `NOREPLACE`, or `EXCHANGE` when overwriting; overwrite requires a real directory. |
| Custom encoder | Owns its own transaction; dropping after an error must discard partial output. |

Without `overwrite`, an existing destination fails with `AlreadyExists` before the encoder
opens or any frame is rendered. With `overwrite`, the old file is untouched until the final
commit. An existing destination that is also a timeline audio source is rejected even
with `overwrite`. Cancellation, errors and drops remove the temporary output
([`native.rs`](../../crates/motion-encode/src/encoder/ffmpeg/native.rs)).

## Cancellation and errors

`ProgressSink::is_cancelled` is polled before work, between frames, in queue waits
(every 10 ms in the worker) and during flush; it yields `Error::Cancelled` and no
commit. Render errors carry the frame via `at_frame`; panics on worker slots and the
first parallel frame become scene-panic errors. Encoder failures surface as
`Error::Encoder` wrapping `io::Error` with the FFmpeg message and code.
`Encoder::write` rejects out-of-order indices, mismatched size or wrong format, and
`finish` rejects an incomplete count.

## Features and platform

| Requirement | Detail |
| --- | --- |
| `ffmpeg` (`motion-encode`, `motion`) | Enables native video/audio encoding; without it, video codecs fail at `open` with `Unsupported`, while PNG/GIF still work. Default in `motion`. |
| `gpu` | Enables the GPU export path. |
| Build host | `pkg-config`, FFmpeg development libraries (libavformat, libavcodec, libavfilter, libswscale, libswresample, libavutil) and libclang for bindgen ([`build.rs`](../../crates/motion-ffmpeg/build.rs)). |
| Runtime | The linked FFmpeg must contain the needed encoders (`libx264`, `libx265`, `libvpx-vp9`, `prores_ks`, AAC). Libraries are found through rpath on Unix. |

## Where unsafe lives

All FFI is in `motion-ffmpeg` (`lib.rs`, `settings.rs`, `audio.rs`, `inspect.rs`,
`encoder.rs` and its `codec`/`mux` modules). The workspace sets
`unsafe_op_in_unsafe_fn = "deny"` and `clippy::undocumented_unsafe_blocks = "warn"`:
every `unsafe` block carries a `// SAFETY:` comment naming the ownership or lifetime
invariant. `Encoder` exclusively owns its libav pointers, frees them in `Drop`, and
`Drop` closes output without committing. Bindgen allowlists only the functions the
boundary uses; `motion-encode` and the rest of the workspace see safe Rust only.

## Testing

Tests verify decoded output rather than call sequences: `inspect` reads stream
info, frame counts, AAC audio and 10-bit luma back from the written file; tests
assert codec, bit depth, color tags, preserved destinations and removed temporaries
([`export_tests.rs`](../../crates/motion-encode/src/encoder/ffmpeg/export_tests.rs),
[`export/tests.rs`](../../crates/motion-encode/src/export/tests.rs)). Custom-encoder
tests use a recording `Encoder` to check ordering, cancellation and capabilities.

## Related

- [Tutorial: Export](../tutorial/11-export.md)
- [Rendering and export](rendering.md), [Reliability](reliability.md), [GPU](gpu.md)
- [Development checks](../development.md)
- [`export.rs`](../../crates/motion-encode/src/export.rs),
  [`options.rs`](../../crates/motion-encode/src/export/options.rs),
  [`motion-ffmpeg`](../../crates/motion-ffmpeg/src/lib.rs)
