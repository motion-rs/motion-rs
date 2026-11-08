//! Asset-free sidechain study: a synthesized four-on-the-floor mix whose visuals are
//! read back from the prepared audio (spectrum, onsets, beat grid and the measured
//! duck gain of the pumping bass and music buses).

use crate::{DemoMetadata, DemoParameters};
use motion::audio::pcm::BusMeter;
use motion::audio::{AudioAnalysis, AudioSignal, MixedAudio};
use motion::prelude::*;
use panels::{
    backdrop, bus_panel, captions, duck_trace, master_sweep, rings, spectrum, timeline_strip,
};
use soundtrack::{audio_timeline, duck_db, meter, rms_db, soundtrack};
use std::sync::Arc;

mod panels;
mod soundtrack;

const RATE: u32 = 48_000;

/// Length of the synthesized program, independent of the visual duration.
const PROGRAM_SECONDS: f64 = 8.0;

const BPM: f64 = 120.0;

const CENTER: Point = Point::new(480.0, 236.0);

const INK: u32 = 0x07080f;

const NAVY: u32 = 0x121632;

const CORAL: u32 = 0xff5a7a;

const CYAN: u32 = 0x38d9ff;

const GOLD: u32 = 0xffc857;

const VIOLET: u32 = 0xa88bff;

const MINT: u32 = 0x5dffc4;

const PAPER: u32 = 0xe9ecff;

const MONO: &str = "Menlo";

const SPECTRUM_BANDS: usize = 32;

const KICK_DEPTH_DB: f64 = 16.0;

const VOICE_DEPTH_DB: f64 = 11.0;

/// Meter hop of `BusMeter`, in seconds.
const METER_HOP: f64 = 0.05;

/// Sampling interval of the duck-gain trace.
const CURVE_STEP: f64 = 0.04;

/// Floor of the duck-gain trace, in dB.
const CURVE_FLOOR_DB: f64 = -24.0;

/// Each feature enters once; the chip then stays on screen as a legend.
const FEATURES: [(f64, &str, &str, u32); 6] = [
    (0.0, "KICK + CLICK", "4-on-the-floor / metronome", CORAL),
    (1.0, "SIDECHAIN", "pad ducked by the kick clip", CYAN),
    (2.0, "HI-HAT GRID", "16th steps / pan animation", GOLD),
    (4.0, "LEAD", "delay + reverb / stereo width", VIOLET),
    (5.5, "VOICE DUCK", "synth voice lowers music bus", PAPER),
    (7.0, "MASTER", "limiter + loudness normalize", MINT),
];

const LEAD_NOTES: [f64; 16] = [
    440.0, 523.25, 659.25, 783.99, 659.25, 523.25, 587.33, 659.25, 880.0, 783.99, 659.25, 523.25,
    440.0, 523.25, 659.25, 440.0,
];

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "sidechain-pulse",
    "Sidechain Pulse",
    "Audio",
    &[
        "sidechain-ducking",
        "audio-buses",
        "synthesis",
        "audio-effects",
        "pan-and-width",
        "audio-analysis",
        "master-limiter",
    ],
);

fn seconds(value: f64) -> Time {
    Time::from_seconds(value)
}

struct Rig {
    spectrum: Vec<AudioSignal>,
    beats: Vec<f64>,
    onsets: Vec<f64>,
    kick: BusMeter,
    drums: BusMeter,
    voice: BusMeter,
    bass: BusMeter,
    bass_dry: BusMeter,
    music: BusMeter,
    music_dry: BusMeter,
    curve: Vec<(f64, f64)>,
    lufs: f64,
    true_peak_db: f64,
    end: f64,
}

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    let duration = i64::from(parameters.seconds).seconds();
    let seed = parameters.seed as u32;
    // Analysis always sees the whole program so every bus meter exists, even for short renders.
    let program = seconds(PROGRAM_SECONDS);
    let wet = audio_timeline(&soundtrack(true, seed)?, program);
    let dry = audio_timeline(&soundtrack(false, seed)?, program);
    let mix = MixedAudio::from_timeline(&wet, RATE)?;
    let dry_mix = MixedAudio::from_timeline(&dry, RATE)?;

    let span = seconds(PROGRAM_SECONDS.min(duration.as_seconds()));
    let analysis = AudioAnalysis::from_source(&mix, span)?;
    let spectrum = analysis
        .spectrum(SPECTRUM_BANDS, 40.0, 12_000.0)?
        .bands()
        .iter()
        .map(|band| {
            band.log_scale()
                .attack_release(10.milliseconds(), 140.milliseconds())
        })
        .collect::<motion::Result<Vec<_>>>()?;
    // A hand-set grid replaces the estimate when it picks a harmonic of the tempo.
    let grid = match analysis.beat_grid() {
        Ok(grid) if (grid.bpm - BPM).abs() < 2.0 => grid,
        _ => motion::audio::BeatGrid::new(BPM, Time::ZERO)?,
    };
    let beats = grid
        .markers(span)?
        .times()
        .iter()
        .map(|time| time.as_seconds())
        .collect();
    let onsets = analysis
        .onsets()?
        .times()
        .iter()
        .map(|time| time.as_seconds())
        .collect();
    let report = mix.loudness_report()?;
    let (bass, bass_dry) = (meter(&mix, "bass")?, meter(&dry_mix, "bass")?);
    let (music, music_dry) = (meter(&mix, "music")?, meter(&dry_mix, "music")?);
    let curve = (0..=(PROGRAM_SECONDS / CURVE_STEP) as usize)
        .map(|step| {
            let time = step as f64 * CURVE_STEP;
            (
                duck_db(&bass, &bass_dry, time),
                duck_db(&music, &music_dry, time),
            )
        })
        .collect();
    let rig = Arc::new(Rig {
        spectrum,
        beats,
        onsets,
        kick: meter(&mix, "kick")?,
        drums: meter(&mix, "drums")?,
        voice: meter(&mix, "voice")?,
        bass,
        bass_dry,
        music,
        music_dry,
        curve,
        lufs: report.integrated_lufs.unwrap_or(-70.0),
        true_peak_db: 20.0 * report.true_peak.max(1.0e-6).log10(),
        end: duration.as_seconds(),
    });
    let output = audio_timeline(&soundtrack(true, seed)?, duration);
    Ok(output.track(|track| track.scene(duration, move |context| frame(context, &rig))))
}

fn color(hex: u32) -> Color {
    Color::from_hex(hex)
}

fn text(value: &str, size: f64, fill: Color, bold: bool) -> Text {
    Text::new(value.to_owned())
        .family(MONO)
        .size(size)
        .weight(if bold {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        })
        .letter_spacing(1.0)
        .fill(fill)
}

fn polar(radius: f64, angle: f64) -> Point {
    Point::new(
        CENTER.x + math::cos(angle) * radius,
        CENTER.y + math::sin(angle) * radius,
    )
}

/// Maps a dBFS level to 0..1 over a 60 dB window.
fn meter_level(db: f64) -> f64 {
    ((db + 60.0) / 60.0).clamp(0.0, 1.0)
}

fn frame(context: &SceneContext<'_>, rig: &Rig) -> Node {
    let time = context.seconds();
    let kick = meter_level(rms_db(&rig.kick, time) + 6.0);
    let bass_duck = duck_db(&rig.bass, &rig.bass_dry, time);
    let music_duck = duck_db(&rig.music, &rig.music_dry, time);
    let fade =
        math::smoothstep(0.0, 0.3, time) * (1.0 - math::smoothstep(rig.end - 0.3, rig.end, time));

    let mut layers = vec![
        spectrum(context, rig),
        rings(context, bass_duck, music_duck, kick),
        master_sweep(time),
        bus_panel(rig, time, bass_duck, music_duck),
        duck_trace(rig, time),
        timeline_strip(rig, time),
    ];
    layers.extend(captions(rig, time));
    // The backdrop stays opaque so the fade ends on dark instead of the render background.
    Node::group([backdrop(context), Node::group(layers).opacity(fade)])
        .clip_with(context.bounds(), ClipMode::Geometric)
        .filter(Filter::Glow {
            threshold: 0.4,
            sigma: 7.0,
            intensity: 0.8,
        })
        .filter(Filter::grain(7, 0.006))
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion::audio::AudioSource;

    fn window(mix: &MixedAudio, start: u64) -> Vec<f32> {
        let mut output = vec![0.0; 2 * 4096];
        mix.read(start, &mut output);
        output
    }

    #[test]
    fn soundtrack_replays_pcm_and_ducked_bus_is_quieter_under_the_kick() -> motion::Result {
        let duration = 8.seconds();
        let wet = audio_timeline(&soundtrack(true, 7)?, duration);
        let dry = audio_timeline(&soundtrack(false, 7)?, duration);
        let mix = MixedAudio::from_timeline(&wet, RATE)?;
        let again = MixedAudio::from_timeline(&wet, RATE)?;
        let dry_mix = MixedAudio::from_timeline(&dry, RATE)?;

        // Random-order reads of the lead/voice region replay bit-exactly.
        let start = (5.6 * f64::from(RATE)) as u64;
        let first = window(&mix, start);
        let _ = window(&mix, 0);
        let _ = window(&mix, start / 2);
        assert_eq!(first, window(&mix, start));
        assert_eq!(first, window(&again, start));
        assert!(first.iter().any(|value| value.abs() > 0.001));
        assert_ne!(first, window(&dry_mix, start));

        // While the kick plays (1.0-4.0 s) the ducked bass bus sits below its dry twin,
        // and the dip is deepest just after each kick hit.
        let ducked = meter(&mix, "bass")?;
        let reference = meter(&dry_mix, "bass")?;
        let mean = |meter: &BusMeter| {
            (0..60)
                .map(|step| rms_db(meter, 1.5 + f64::from(step) * 0.025))
                .sum::<f64>()
                / 60.0
        };
        assert!(
            mean(&ducked) < mean(&reference) - 1.0,
            "ducked {} vs dry {}",
            mean(&ducked),
            mean(&reference)
        );
        let hit = duck_db(&ducked, &reference, 2.5 + 0.05);
        let recovered = duck_db(&ducked, &reference, 2.5 + 0.45);
        assert!(hit < -4.0 && recovered > hit + 3.0, "{hit} {recovered}");
        let report = mix.loudness_report()?;
        assert!(report.integrated_lufs.is_some_and(|lufs| lufs < -10.0));
        Ok(())
    }

    #[test]
    fn film_renders_deterministically() -> motion::Result {
        let timeline = timeline(DemoParameters::default())?;
        let options = crate::render_options(None)?.size(PixelSize::new(240, 135));
        let first = render_still(&timeline, 2.seconds(), &options)?;
        let again = render_still(&timeline, 2.seconds(), &options)?;
        let early = render_still(&timeline, 500.milliseconds(), &options)?;
        assert_eq!(first.bytes(), again.bytes());
        assert_ne!(first.bytes(), early.bytes());
        Ok(())
    }
}
