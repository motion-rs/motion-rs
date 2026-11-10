# 10 Audio

## What This Chapter Covers

Attach sound to a timeline and understand source offsets, occurrences, mixing and analysis without speakers or a playback device.
File decoding and analysis require `audio`; native MP4/MOV audio encoding requires `ffmpeg`. PCM data alone needs no window.

## Minimal Example: Synthesis, Analysis and Visuals

Enable `audio`. This example reads no audio files and requests no playback device.

```rust,no_run
use motion::prelude::*;
use motion::extend::{AudioSource};
use motion::encode::{AudioAnalysis, AudioSignal, MixedAudio, PcmBuffer};
use std::sync::Arc;

fn main() -> motion::Result {
    let samples = (0..48_000)
        .map(|index| {
            let phase = index as f64 * 440.0 * std::f64::consts::TAU / 48_000.0;
            (math::sin(phase) * 0.2) as f32
        })
        .collect();
    let pcm = Arc::new(PcmBuffer::new(48_000, 1, samples)?);
    let loudness = AudioAnalysis::from_pcm(pcm.clone()).loudness()?;
    let sound = AudioClip::source(pcm)
        .fade_in(100.milliseconds())
        .fade_out(100.milliseconds());
    let timeline = Timeline::new(Size::new(320.0, 180.0), 30)
        .audio(sound)
        .track(|track| {
            track.scene(1.seconds(), move |context| {
                let radius = 20.0 + context.sample(&loudness) * 80.0;
                Circle::new(context.center(), radius).fill(Color::rgb(20, 150, 125))
            })
        }).build()?;
    render_still(&timeline, 500.milliseconds(), &RenderOptions::default())?
        .save_png("tutorial-audio-reactive.png")
}
```

RMS comes from the original PCM; clip fades are timeline envelopes. To drive visuals from mixed audio, analyze the prepared mix rather than treating source analysis as final-track analysis.

## Sources and Occurrences

`AudioSource` is the extension protocol for random-access audio data. `PcmBuffer` is immutable, finite mono/stereo f32 PCM. Clones share samples, not a file cursor.

`AudioClip` is one timeline occurrence. Sources can be shared, while each placement has its own start, trim, duration, volume, pan and envelope.

| Operation | Meaning |
| --- | --- |
| `AudioClip::source` | Wrap explicit PCM or a custom source |
| `AudioClip::file` | Retain a lazy file reference without decoding during authoring |
| `start` | Start time on the timeline |
| `offset` | Time at which reading begins in the source |
| `duration` | Duration used by the occurrence |
| `volume` / `pan` | Gain and stereo positioning |
| `fade_in` / `fade_out` | Occurrence fades |
| `volume_envelope` | Explicit time-animated gain |

`start(1.seconds())` does not trim the source's first second; that is `offset`'s role. Multiple placements of one source need multiple explicit clips.

## Shared Narration Clock

With `captions`, `CueSheet` combines a CaptionTrack, a bounded positive
`std::time::Duration` and unique named markers. Cues retain nanosecond boundaries;
sampling compares them with Time ticks through integer cross-products. SRT
interchange still requires representable millisecond timestamps.

`elapsed(time, marker)` is absent before the marker or at/after the sheet end;
it does not stop at the next marker. `progress(time, start, end)` clamps and rejects
equal or reversed marker endpoints. Retiming uses exact integer ratios and nearest
nanoseconds, rejects collapsed cues and requires rebuilding derived layouts. It
does not stretch audio or update cached work automatically.

`audio_clip` places an explicit source interval at a cue and rejects intervals
longer than that cue. Source bounds use Time. There is no automatic speech
generation, truncation, pitch change or ducking; ordinary AudioClip/MixedAudio
provides preview and export.

## File Audio and Mixing

This example requires `audio` and the runtime files `voice.wav` and `music.mp3`:

```rust,no_run
use motion::prelude::*;
use motion::encode::MixedAudio;

fn main() -> motion::Result {
    let narration = AudioClip::file("voice.wav")
        .start(1.seconds())
        .duration(4.seconds())
        .fade_in(100.milliseconds());
    let music = AudioClip::file("music.mp3")
        .offset(2.seconds())
        .duration(6.seconds())
        .volume(0.12);
    let timeline = Timeline::new(Size::new(320.0, 180.0), 30)
        .audio(narration)
        .audio(music)
        .duration(6.seconds()).build()?;
    let resolved = motion::audio::prepare_timeline(&timeline)?;
    let _mixed = MixedAudio::from_timeline(&resolved, 48_000)?;
    Ok(())
}
```

Preparation resolves file metadata and required windows. A file source remains a random-access file, not permanently frozen PCM. Keep inputs unchanged during execution. Use explicit eager PCM decoding for a snapshot independent of the file's lifetime.

Native timeline output defaults to 48 kHz stereo; `ExportOptions::audio_sample_rate` also accepts 44.1 and 96 kHz for AAC. Different source rates are resampled. Preserve exact timing with integer ticks and sample boundaries rather than rounding rational video rates before calculating audio length.

## Master, Buses and Effects

Master processing is opt-in. Identity settings preserve the old floating-point mix; `Master::safe()` enables a -1 dB ceiling with 5 ms lookahead and 100 ms release. Normalization measures K-weighted, gated integrated loudness before selecting a single gain, constrained by the requested reconstructed peak ceiling.

```rust,no_run
use motion::prelude::*;
use motion::audio::{MixedAudio, synth::{Synth, Wave}, effects::{Effect, EqKind}};
use std::sync::Arc;

fn main() -> motion::Result {
    let tone = Synth::new(Wave::Sine, 440.0, 48_000, 17)?;
    let mut music = AudioBus::new("music");
    music.gain_db = -6.0;
    music.effects.push(Arc::new(Effect::Eq { kind: EqKind::LowPass, frequency: 8_000.0, q: 0.707, gain_db: 0.0 }));
    let timeline = Timeline::new(Size::new(320.0, 180.0), 30)
        .audio_bus(music)
        .audio(AudioClip::source(tone).duration(2.seconds()).bus("music").fade_in(50.milliseconds()))
        .master(Master::safe()).build()?;
    let mixed = MixedAudio::from_timeline(&timeline, 48_000)?;
    let _report = mixed.loudness_report()?;
    let _levels = mixed.bus_meters()[0].level(500.milliseconds());
    Ok(())
}
```

`Effect` provides RBJ EQ, compression, gating, de-essing, delay and Schroeder reverb. `AnimatedEq` samples frequency/Q/gain on the native clip clock every 64 frames and ramps coefficients. Effects run before occurrence gain/pan; bus effects run before bus gain and master. Declared delay/reverb tails extend inferred output, while explicit timeline duration deliberately crops them.

Stateful DSP is baked from the clip origin once, not approximated with a finite IIR warmup. Immutable prepared reads and output windows then replay exactly, including master normalization. Prepared clones share cached loudness reports. A bounded global report cache additionally keys measurements by sample rate and PCM content hash, verifying sample bits on hits to reject hash collisions.

`PanLaw` selects stereo balance; `stereo_width` uses M/S. `spatial` maps screen positions to pan and inverse-distance gain, with width measured from listener to screen edge, not HRTF. The report's correlation is a whole-output mono-compatibility indicator.

## Ducking

`clip.ducked_by(&narration, Duck::new(-45.0, 12.0))` lowers a clip while the trigger is audible; `AudioBus::ducked_by(DuckTrigger::Bus("voice".into()), duck)` ducks a whole submix (the trigger bus must be declared earlier). The trigger is rendered first, followed by a 5 ms peak-hold detector, a threshold/`ratio` gain computer capped by `depth_db`, and attack/release smoothing in dB. `lookahead` shifts the reduction earlier so the music is already low when speech starts. Everything is baked offline, so output windows replay exactly. Clip triggers must be clips; bus triggers are bus-level only. A clip trigger's own ducking is ignored, up to four triggers per clip, and nested scenes relocate triggers together with the clip.

## Procedural Sound and Editing

`audio::synth` supplies oscillators, seeded white/pink/brown noise approximations, analytic chirps, `Adsr`, eleven `Sound` presets, metronomes and a sixteen-step drum machine (mask bits 1/2/4: kick/snare/hat). No third-party sound assets are bundled.

Import `motion::audio::AudioClipExt` for `speed`, `reverse`, `loop_until`, `crossfade` and `pitch_shift`. Transforms freeze the trimmed source before clip effects. Speed uses windowed sinc interpolation; independent pitch/tempo uses bounded WSOLA, not formant-preserving voice processing. Speed scales clip fades but leaves explicitly authored envelope/pan animations on their original time clock. Loop seams use equal-power crossfades; clip crossfade convenience uses occurrence fades.

`scene_with_sound(build, duration, size, rate)` opts into one cached declaration pass at local time zero. Call `context.sound(clip)?` unconditionally during that pass; do not hide declarations in time-dependent branches. Normal visual snapshots remain silent. Scene nesting relocates/crops sounds and preserves child master/bus ownership.

## Analysis Without Playback

`AudioAnalysis::from_pcm` shares decoded data; `from_source` freezes an explicit interval. Analysis traces are reusable `AudioSignal` values whose time sampling repeats neither I/O nor FFT work.

`loudness()` returns linear RMS, not LUFS or dB. `band(low, high)` returns RMS energy in the selected band, with Hann windows and Nyquist-range checks. It does not apply a filter to the original track.

Windows look forward for offline motion graphics; they are not equivalent to the causal latency of real-time microphone analysis. Opposite-phase stereo does not cancel energy through an initial sum of its channels.

`spectrum(bands, low, high)` computes 1..=64 logarithmic bands in one STFT pass. `chroma`, dominant spectral `pitch`, `onset_strength`, `onsets` and `beat_grid` reuse bounded shared traces. Pitch is not fundamental-frequency transcription; automatic tempo can select harmonics on complex music, so `BeatGrid::new` provides manual override. Signals support `attack_release`, `log_scale`, percentile `normalize` and `threshold`. `AudioMarkers::write_json/read_json` preserves integer ticks, validates version/clock and requires strictly ordered times.

## Export and Studio

Output-range cropping preserves the original occurrence's fade and envelope clock. Video encoders receive a prepared, cropped mix rather than reinterpreting a separate file-audio configuration.

`MixedAudio::from_timeline_with`/`from_window_with`/`new_with` and `ExportOptions::audio_resampler` select `ResampleQuality::Sinc` (32 zero-crossing Blackman-Harris windowed sinc, kernel stretched when downsampling) instead of the default FFT resampler. Delivery rates are 44100, 48000 and 96000 Hz. Sinc windows see silence beyond the requested interval, so the last few frames have a truncated kernel. Effects have no separate `latency`/`warmup` contract: they are baked from the clip origin, which already gives exact replay.

`export_audio(&timeline, &AudioExportOptions::new("mix.wav").format(AudioFormat::Wav24))` writes a standalone mix without video: 16/24-bit PCM or 32-bit float, mono or stereo, 8000..=192000 Hz, optional range, deterministic TPDF dither for 16-bit and `LIST INFO` tags (title, artist, album, comment, copyright, date, genre, software). Files above 4 GiB use an RF64 header; chapters are rejected. The destination is committed atomically and the report counts samples clipped by integer formats. FLAC/Opus/WebM audio are not offered because the FFmpeg bridge only exposes AAC.

Studio audio preview is an optional device consumer. Empty audio requests no device, and Mute affects preview listening without removing the output track. The playback audio clock and headless scene evaluation are separate layers.

The native adapter streams f32 stereo PCM directly into AAC, without an audio temporary WAV; video container commit remains transactional. `AudioMetadata` carries bounded tags (including language/R128/ReplayGain values) and nonoverlapping `AudioChapter` intervals. `ExportOptions::audio_metadata` supplies them; selected output ranges crop/rebase chapters. Loudness tags are caller-supplied, not inferred automatically.

`motion_studio::AudioReadout::from_mix(&mix, time)` gives a headless readout of integrated LUFS, sample/true peak, correlation and per-bus RMS/peak meters for UI or pre-export warnings.

Studio uses the same prepared effects/buses/master chain, multiresolution peak waveforms without the old 300-second cutoff, onset ticks, beat snapping, RMS curves and bus meters. Its report explicitly measures 48 kHz base timeline audio; draft extra-file tracks are not included. Long-input analysis failures are shown separately from waveform data.

## Bounds and Measurement Limits

Timeline preparation admits 256 clips, 16 buses, 16 effects per chain, 30-second per-effect tails, 16 composition levels and 24-hour declarations. Retained PCM is limited to 32 million f32 samples (128 MiB); full master/bus/effect preparation can reject long sparse outputs rather than silently truncate. Meter traces have a separate 16 MiB ceiling; analysis caches 16 groups/64 MiB, at most one million windows and bounded FFT work. The measurement cache separately retains at most 16 entries/64 MiB of verified PCM; larger measurements only use their prepared-instance cache. These are allocation/work limits, not a process RSS cap.

Integrated loudness uses 400 ms blocks/100 ms hops, -70 LUFS absolute and -10 LU relative gates. Outputs shorter than 400 ms have no integrated report. True peak is a four-times, 32-tap windowed-sinc estimate, not a certified ITU Annex implementation. Generated tone/ceiling/gating regressions do not replace official EBU fixture or listening acceptance. AAC consumes float PCM and needs no 16-bit dither.

## Common Pitfalls

- Timeline start and source offset are different.
- Do not decode or run FFTs in every frame callback.
- Do not modify files used by lazy sources or use input audio as the output destination.
- Do not label RMS directly as dB or LUFS.
- Enabling `audio` does not require speakers. Decoding, mixing and analysis are headless operations.

## Next Steps

Next chapter: [11 Export](11-export.md).
Complete application: [audio export example](../../examples/audio_export.rs).
Contracts: [PCM and mixing](../../crates/motion-encode/src/audio/pcm.rs), [analysis](../../crates/motion-encode/src/audio/analysis.rs).
Film: [Signal Garden](../../gallery/src/demos/signal_garden.rs).