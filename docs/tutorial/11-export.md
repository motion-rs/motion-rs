# 11 Export

## What This Chapter Covers

Send the same timeline to an ordered encoder, choose image sequences, GIF or native video, and handle destination protection, progress and cancellation correctly.
PNG sequences and GIF need no `ffmpeg`; native MP4/MOV/WebM encoding requires `ffmpeg`.

## Minimal Example: PNG Sequence

```rust,no_run
use motion::encode::NoProgress;
use motion::prelude::*;
use motion::render::{FrameBuffer};
use motion::extend::{Encoder, ProgressSink};

fn main() -> motion::Result {
    let timeline = Timeline::new(Size::new(320.0, 180.0), fps!(30)).track(|track| {
        track.scene(1.seconds(), |context| -> motion::Result<_> {
            Ok(Circle::new(Point::new(40.0 + context.seconds() * 240.0, 90.0), 20.0)?
                .fill(Color::rgb(20, 150, 125)))
        })
    }).build()?;
    let options = ExportOptions::new("tutorial-frames").codec(Codec::PngSequence);
    let report = export(&timeline, &options, &NoProgress)?;
    println!("{} frames -> {}", report.frames, report.path.display());
    Ok(())
}
```

The destination is a sequence directory, not a single PNG file. `render_still` is better for one PNG.
An existing directory is protected from overwrite by default. Do not delete it manually to bypass the library's transaction boundary.

## MP4 and Format Selection

`ExportOptions::new("film.mp4")` defaults to H.264, CRF 18, the Medium preset and opaque black background. It requires native FFmpeg with libx264/AAC. Changing the file extension alone does not change the encoder.

| Codec | Output Constraints |
| --- | --- |
| `H264` / `H265` | MP4; limited-range BT.709; no transparent video output |
| `Vp9 { alpha, .. }` | WebM; optional alpha; WebM audio tracks are currently rejected |
| `ProRes4444` | MOV; alpha supported |
| `PngSequence` | PNG directory; full alpha; no audio track |
| `Gif` | 2–256 colors, 1-bit transparency, explicit frame rate; no audio; maximum 100 fps |

Encoder capabilities are checked before opening output. Even dimensions, maximum size, quality, audio support and input format are contractual requirements; file extensions cannot repair them automatically.

With `ffmpeg` enabled, export directly:

```rust,no_run
use motion::encode::NoProgress;
use motion::prelude::*;

fn main() -> motion::Result {
    let timeline = Timeline::new(Size::new(320.0, 180.0), fps!(30)).track(|track| {
        track.scene(2.seconds(), |context| -> motion::Result<_> {
            Ok(Circle::new(context.center(), 20.0 + context.progress() * 30.0)?
                .fill(Color::rgb(20, 150, 125)))
        })
    }).build()?;
    export(&timeline, &ExportOptions::new("tutorial-film.mp4"), &NoProgress)?;
    Ok(())
}
```

See the [audio_export example](../../examples/audio_export.rs) for a complete version with sound. MP4/MOV audio uses AAC; ignoring an audio track does not make PNG/GIF a successful audio export.

## ExportOptions

| Setting | Purpose |
| --- | --- |
| `codec` | Explicit container and encoding format |
| `size` | Output pixel dimensions, distinct from design dimensions |
| `range` | Half-open absolute time range |
| `backend` | CPU or GPU (the default with feature `gpu`) |
| `render` | Configure assets, caches, quality, background and budgets |
| `workers` | Requested render concurrency; 1 preserves serial evaluation |
| `overwrite` | Replace the destination only on successful commit |

The range maps to representable frame boundaries. Scene callbacks retain absolute scene time, while encoder frame indices and output timestamps start at zero. Do not reset all callback times to clip-relative time merely to export a segment.

## Encoder Lifecycle

`export` selects a built-in encoder; `ExportOptions::encoder` replaces it with a factory for your own `Encoder`, so `export` stays the only entry point.

1. Check configuration, dimensions, audio capabilities and the selected range.
2. Prepare the cropped mix, then call `Encoder::open`.
3. Call `write` with consecutive zero-based output indices, not out-of-order completion results.
4. After every frame succeeds and cancellation checks pass, call `finish` to finalize and commit.

`input_format` must match FrameBuffer's plane, alpha and color contracts. `capabilities` defaults are permissive; custom encoders must still declare and validate their own restrictions correctly.

Built-in native encoders are created, used and destroyed on a dedicated thread. Video frames pass through a bounded memory queue without a per-frame raw-video file spool. Audio still uses a temporary PCM WAV bridge. Containers write to a temporary file in the same directory before commit.

## Destination Protection and Transactions

A successful `ExportReport` confirms destination commit. Cancellation, drop, rendering errors, worker panics or encoding failures end the task and discard staged output, preserving the previously committed destination.

Custom Encoders own their output transactions and drop cleanup. Implementing `write` does not automatically provide file protection. Do not let an export destination replace an audio source currently in use.

## Progress and Cancellation

```rust,no_run
use motion::extend::ProgressSink;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
struct Progress {
    cancel: AtomicBool,
}

impl ProgressSink for Progress {
    fn frame_completed(&self, completed: u64, total: u64) {
        println!("{completed}/{total}");
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }
}

fn main() {
    let progress = Progress::default();
    progress.cancel.store(true, Ordering::Release);
    assert!(progress.is_cancelled());
}
```

Pass `&progress` to `export`. Another thread or UI can set the cancellation flag. Cancellation is cooperative and does not guarantee immediate interruption of a blocking native call.

`frame_completed` reports rendered-frame progress, not file commit. `ExportPhase::Completed` and a successful report mark completion.

## CPU Concurrency and GPU Queues

Explicit CPU export renders the first frame normally, then uses its measured logical peak, remaining frames, requested workers and `max_parallel_bytes` to select later concurrent slots. Larger later frames may retry serially under the original allowance, so callbacks must be pure and repeatable.

Only supported byte-admission failures trigger this retry. Invalid content, work-limit failures and panics remain errors. Writers and progress callbacks stay ordered; more workers do not guarantee faster overall export.

A GPU export retains one session. Supported GPU packed output can use CPU preparation slots and a bounded GPU queue, completing and encoding in order. The aggregate concurrency allowance does not include all shared caches, encoders, idle pools or RSS.

## Common Pitfalls

- Changing extensions without changing Codec, or treating a PNG sequence path as one image.
- Treating 100% rendering progress as a committed file.
- Assuming cancellation immediately interrupts blocking encoder calls.
- Introducing callback side effects to increase concurrency.
- Using an asset source file as the output destination.

## Next Steps

Next chapter: [12 Studio](12-studio.md).
Full contracts: [rendering and export](../design/rendering.md), [export API](../../crates/motion-encode/src/export.rs).
Native application: [audio export example](../../examples/audio_export.rs).