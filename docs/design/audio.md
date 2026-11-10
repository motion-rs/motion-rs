# Audio

Audio is split into a pure authoring description and an offline, deterministic
preparation step. Nothing in either layer needs a playback device. Real-time playback
exists only in Studio, as a separate consumer. Exact signatures are in Rustdoc;
this page records ownership, limits, ordering and failure behavior. Asset-level
context is in [Content](content.md#authored-tools-and-audio) and container/commit
rules are in [Rendering and export](rendering.md#video-and-audio).

## Ownership Split

| Layer | Crate | Owns |
| --- | --- | --- |
| Description | [`motion-core`](../../crates/motion-core/src/audio.rs) | `AudioSource`, `AudioClip`, `AudioBus`, `Master`, `Duck`/`Ducking`, `PanLaw`, `AudioEffect`, `AudioMetadata`; validation only |
| Preparation | [`motion-encode`](../../crates/motion-encode/src/audio.rs) | `PcmBuffer`, `MixedAudio`, `BusMeter`, resampling, built-in effects, master processing, analysis, synthesis, WAV export |
| Preview | [`motion-studio`](../../crates/motion-studio/src/audio/preview.rs) | `cpal` playback, ring buffer, readout model |

- An `AudioClip` is an immutable occurrence over a shared `AudioSource`. Start, source
  offset, trim, volume envelope and fades have independent clocks; a pan animation
  is sampled on the occurrence clock.
- `AudioSource` is a pure, concurrent, random-access interleaved f32 source. Repeated
  reads must not depend on earlier reads; short reads and EOF leave the rest of the
  buffer silent; returned samples must be finite. Mixing supports mono and stereo.
- File and nested-timeline sources are deferred. `Timeline::resolve_audio` returns a
  `ResolvedAudio` (clips plus the output duration they need) without changing the timeline. `resolve_clip` and
  `prepare_timeline` turn them into resolved sources at an execution boundary
  (export, `MixedAudio`), never during scene evaluation. File sources are decoded
  lazily in windows and must not change while executing; explicit `PcmBuffer` data is
  frozen. Nested compositions are limited to 16 levels.

## Limits

| Limit | Value | Where checked |
| --- | --- | --- |
| Source sample rate / channels | 1..=192000 Hz, mono or stereo | `AudioClip::resolved_duration` |
| Clips per prepared mix, file export or Studio list | 256 (`MAX_CLIPS`) | `MixedAudio::new_with`, Studio audio settings |
| Buses / effects per chain / ducking per clip | 16 / 16 / 4 | `Timeline` validation, `AudioClip` |
| Effect tail | 0..=30 s; added to the clip duration | `AudioClip::resolved_duration` |
| Declared span | at most 24 h (`MAX_SECONDS = 86_400`) | clip and mix validation |
| Volume / pan / stereo width | 0..=16 / -1..=1 / 0..=2 | `AudioClip::resolved_duration` |
| Retained PCM | 32 Mi f32 samples = 128 MiB (`MAX_SAMPLES`) | `PcmBuffer`, mixes, bus submixes, snapshots, resampling scratch |
| Encoded input | 64 MiB (`MAX_ENCODED_BYTES`) | `PcmBuffer::from_bytes`/`open` |
| Bus meter traces | 16 MiB | `process_buses` |
| Metadata | 64 tags, 4096 chapters, 4096-byte text | `AudioMetadata::validate` |
| Mix rate | 1..=192000 Hz; video AAC requires 44.1/48/96 kHz | `MixedAudio`, `NativeOptions::audio_mix` |

These are allocation/work limits, not a process memory cap. Oversized requests are
rejected rather than truncated.

## Preparation Order

Per clip (in `MixedAudio::new_with`): read the source window → effect chain at the
**source rate** → resample to the mix rate → volume, envelope and fades → stereo
width and pan → clip-level ducking → store as a `PreparedClip`.

Then, once for the whole mix:

1. Buses (`process_buses`), in declaration order. Each bus sums its clips, runs its
   effects, applies static gain, then its ducking. Unbussed clips bypass buses. A clip
   naming an undeclared bus fails.
2. Bus meters record 50 ms RMS and sample-peak traces after effects, gain and ducking.
3. Master (`MixedAudio::master`): static gain → optional loudness normalization →
   optional lookahead limiter → a reconstructed-peak correction below the ceiling.
   `Master::default()` is identity; `Master::safe()` is a -1 dB limiter without
   normalization.

Ducking triggers are rendered completely before the target is touched. A bus trigger
must be declared earlier than the ducked bus; a clip trigger is rendered standalone
at its own gain and pan and its own ducking is ignored. Detector, attack, release
and lookahead run offline on the full trigger signal.

Panning: `PanLaw::ConstantPower` is the default; it and `Minus4_5dB` are normalized
at center, while `Linear` attenuates only the opposite channel; mono sources always pan with constant power.

## Effects: Statefulness, Tails, Random Access

`AudioEffect::process` receives whole interleaved buffers from a zero initial
state. Filter/dynamics state exists only during preparation; the mixer processes
each occurrence from its origin once and retains the result, so later reads never
warm up IIR state. Built-in `Effect` variants are EQ, compressor, gate, de-esser,
delay and reverb; `AnimatedEq` samples automation on the clip's native clock.
Delay and reverb declare an explicit `tail` that truncates the decay; `tail()` extends
the occurrence by that much silent input. Custom effects must be deterministic and
implement `validate` without processing PCM.

Consequence: `MixedAudio::from_window` is only a direct window when there are no
effects, ducking, buses or master settings; otherwise it prepares the full timeline
and crops, so effect and normalization clocks never restart at the window edge.
`MixedAudio::window` likewise crops prepared output.

## Determinism and Seek-Replay

- Prepared reads (`MixedAudio::read`) sum prepared clips in fixed order in f64
  and never touch sources or resampler state; results do not depend on query order.
- Source reads, effects, ducking and master processing are pure functions of the
  timeline and options. User sources and envelopes are trusted pure callbacks; panics
  propagate.
- The resampler is chosen explicitly: `ResampleQuality::Fast` (FFT, default,
  needs `audio`) or `Sinc` (windowed sinc, 32 zero crossings per side).
- Synthesis (`Synth`) integrates phase analytically from the frame index; noise is a
  hash of frame and seed, so seek-and-replay reproduces the same samples for a
  fixed seed. Time-to-frame conversion is integer arithmetic. Sinusoids read a fixed
  4096-segment table with linear interpolation (error below `3e-7`, about -130 dB)
  instead of evaluating `sin` per sample.
- Analysis signals are frozen traces, so sampling them inside a frame callback has
  no I/O or FFT cost.

## Analysis Signals

`AudioAnalysis` (feature `audio`) wraps one immutable `PcmBuffer`, or freezes a bounded
interval of any pure source via `from_source`. It yields `AudioSignal`
(a stateless `Animation<f64>`, linear interpolation, 0 outside its duration):
`loudness` (linear RMS, not LUFS), `band`, `pitch`, `onset_strength`;
`AudioSpectrum` (`spectrum`, `chroma`); `AudioMarkers` (`onsets`, JSON read/write);
and `BeatGrid`. Traces are computed with forward Hann windows, cached per PCM
(16 traces / 64 MiB) and capped at one million time points. Analysis of a source
is not analysis of the final mix; analyze a prepared mix when visuals should follow
the delivered track. `WaveformPyramid` builds peak envelopes (at most 1M base bins).

## Export

- Video: `export` resolves audio, builds one `MixedAudio::from_window_with` for the
  selected range at `ExportOptions.audio_sample_rate`, and passes it to
  `Encoder::open` before any frame is written.
- Standalone: `export_audio` writes WAV (RIFF, or RF64 for large data) through a
  temporary file beside the destination. Existing destinations survive unless
  `overwrite` is set; failure leaves no partial file; the destination may not be an
  input source. Rates 8000..=192000; chapters are rejected for WAV.
- Loudness (`MixedAudio::loudness_report`) is gated 400 ms integrated LUFS plus
  a reconstructed true peak; outputs shorter than a block have no integrated value.

## Studio Preview vs Export

Studio plays audio through a worker thread, `cpal` and a lock-free ring buffer; it
is not an export path. It requires 48 kHz stereo f32 output and mixes in 512-frame
blocks, preparing `from_timeline` for processed timelines or short 8192-frame
`from_window` ranges otherwise. A request is cancelled by `stop`; underruns, device
loss and mix errors surface as `AudioStatus::Failed` and never alter export.
`AudioReadout` reports integrated LUFS, sample/true peak, correlation and bus
meters from a prepared mix without a device. Audio drafts load without opening
media sources. See [Studio guide](../studio.md).

## Features and Failure

| Feature | Effect |
| --- | --- |
| `audio` (default in `motion-encode`) | Decoding (WAV/MP3/AAC/FLAC/Vorbis), FFT resampling, analysis |
| `ffmpeg` (implies `audio`) | Native AAC muxing into video |
| neither | Explicit PCM and same-rate mixing work; file audio and rate conversion fail with explicit errors |

Failures are `motion_core::Error` values: invalid parameters are field-tagged
validation errors (`audio.<field>`), unreadable or changed files are asset errors,
and nonfinite PCM from a source, gain or master is rejected rather than stored.
Preparation errors surface before video encoding starts.

## Tutorial Coverage

[Chapter 10](../tutorial/10-audio.md) teaches authoring: sources, occurrences, narration
clock, mixing, buses, ducking, synthesis, analysis-driven visuals and pitfalls. This
page owns the contracts behind it: layer split, limits, processing order and
determinism.

## Related

- [Content](content.md#authored-tools-and-audio) · [Rendering and export](rendering.md#video-and-audio) · [Reliability](reliability.md)
- [Studio guide](../studio.md) · [`audio_export` example](../../examples/audio_export.rs)
- [`mix.rs`](../../crates/motion-encode/src/audio/pcm/mix.rs) · [`master.rs`](../../crates/motion-encode/src/audio/master.rs) · [`effects.rs`](../../crates/motion-encode/src/audio/effects.rs)
