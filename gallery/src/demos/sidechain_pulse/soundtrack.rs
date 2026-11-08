//! The synthesized Sidechain Pulse program: kick, drums, bass, music and voice buses with
//! sidechain ducks and a mastering chain, plus the bus-meter readings the visuals use.

use super::{
    BPM, KICK_DEPTH_DB, LEAD_NOTES, METER_HOP, PROGRAM_SECONDS, RATE, VOICE_DEPTH_DB, seconds,
};
use motion::audio::effects::{Effect, EqKind};
use motion::audio::pcm::BusMeter;
use motion::audio::synth::{Adsr, Synth, Wave};
use motion::audio::{AudioBus, Duck, DuckTrigger, Loudness, Master, MixedAudio};
use motion::prelude::*;
use std::f64::consts::{PI, TAU};

pub(super) struct Soundtrack {
    buses: Vec<AudioBus>,
    clips: Vec<AudioClip>,
}

fn kick_duck() -> Duck {
    Duck::new(-30.0, KICK_DEPTH_DB)
        .ratio(100.0)
        .timing(5.milliseconds(), 240.milliseconds())
        .lookahead(5.milliseconds())
}

fn voice_duck() -> Duck {
    Duck::new(-45.0, VOICE_DEPTH_DB)
        .timing(60.milliseconds(), 320.milliseconds())
        .lookahead(40.milliseconds())
}

fn master() -> Master {
    let mut master = Master::safe();
    master.normalize = Some(Loudness {
        target_lufs: -16.0,
        true_peak_db: -1.0,
    });
    master
}

fn eq(kind: EqKind, frequency: f64, q: f64, gain_db: f64) -> Effect {
    Effect::Eq {
        kind,
        frequency,
        q,
        gain_db,
    }
}

/// The complete mix. `ducked: false` keeps every clip and bus but removes both
/// sidechains, giving the reference level used to measure the real duck gain.
pub(super) fn soundtrack(ducked: bool, seed: u32) -> motion::Result<Soundtrack> {
    let total = seconds(PROGRAM_SECONDS);
    let mut voice_bus = AudioBus::new("voice");
    voice_bus.gain_db = -1.0;
    let mut bass_bus = AudioBus::new("bass");
    let mut music_bus = AudioBus::new("music");
    if ducked {
        let trigger = DuckTrigger::Bus("voice".to_owned());
        bass_bus = bass_bus.ducked_by(trigger.clone(), voice_duck());
        music_bus = music_bus.ducked_by(trigger, voice_duck());
    }
    let buses = vec![
        AudioBus::new("kick"),
        AudioBus::new("drums"),
        voice_bus,
        bass_bus,
        music_bus,
    ];

    let mut steps = [0; 16];
    for step in [0, 4, 8, 12] {
        steps[step] = 1;
    }
    let kick = AudioClip::source(Synth::drum_machine(BPM, steps, total, seed)?)
        .bus("kick")
        .volume(0.9)
        .fade_out(300.milliseconds());
    let mut clips = vec![kick.clone()];

    clips.push(
        AudioClip::source(Synth::metronome(BPM, 4, total)?)
            .duration(2.seconds())
            .bus("drums")
            .volume(0.35)
            .fade_out(150.milliseconds()),
    );

    let mut hats = [0; 16];
    for step in [2, 6, 10, 13, 14, 15] {
        hats[step] = 4;
    }
    clips.push(
        AudioClip::source(Synth::drum_machine(BPM, hats, total, seed + 1)?)
            .start(2.seconds())
            .offset(2.seconds())
            .duration(seconds(PROGRAM_SECONDS - 2.0))
            .bus("drums")
            .volume(0.32)
            .pan_animation(|time: Time| 0.6 * math::sin(TAU * time.as_seconds() / 2.0))
            .fade_out(300.milliseconds()),
    );

    let duck = |clip: AudioClip| {
        if ducked {
            clip.ducked_by(&kick, kick_duck())
        } else {
            clip
        }
    };
    clips.push(duck(
        AudioClip::source(Synth::new(Wave::Sine, 55.0, RATE, seed)?.gain_db(-3.0)?)
            .start(1.seconds())
            .offset(1.seconds())
            .duration(seconds(PROGRAM_SECONDS - 1.0))
            .bus("bass")
            .volume(0.5)
            .fade_in(300.milliseconds())
            .fade_out(300.milliseconds()),
    ));
    // Detuned saw pairs panned apart give the pad a side signal; the second half
    // continues the same oscillator phase but opens the stereo width.
    for (index, frequency) in [55.0, 82.41, 110.0, 130.81].into_iter().enumerate() {
        for (side, pan, detune) in [(0, -0.8, 0.997), (1, 0.8, 1.003)] {
            for (start, end, width) in [(1.0, 4.1, 0.25), (4.0, PROGRAM_SECONDS, 1.8)] {
                let source = Synth::new(
                    Wave::Saw,
                    frequency * detune,
                    RATE,
                    seed + 10 + index as u32 * 2 + side,
                )?
                .gain_db(-9.0)?;
                let mut clip = AudioClip::source(source)
                    .start(seconds(start))
                    .offset(seconds(start))
                    .duration(seconds(end - start))
                    .bus("bass")
                    .volume(0.2)
                    .pan(pan)
                    .stereo_width(width)
                    .effect(eq(EqKind::LowPass, 700.0, 0.8, 0.0));
                clip = if start < 2.0 {
                    clip.fade_in(300.milliseconds())
                        .fade_out(100.milliseconds())
                } else {
                    clip.fade_in(100.milliseconds())
                        .fade_out(300.milliseconds())
                };
                clips.push(duck(clip));
            }
        }
    }

    for (index, frequency) in LEAD_NOTES.into_iter().enumerate() {
        let start = 4.0 + index as f64 * 0.25;
        let note = 250.milliseconds();
        let source = Synth::new(Wave::Square, frequency, RATE, seed)?
            .chirp(frequency, note)?
            .gain_db(-14.0)?;
        clips.push(
            AudioClip::source(source)
                .start(seconds(start))
                .bus("music")
                .volume(0.5)
                .volume_envelope(Adsr::new(
                    4.milliseconds(),
                    70.milliseconds(),
                    0.45,
                    70.milliseconds(),
                    note,
                )?)
                .pan_animation(move |time: Time| {
                    0.7 * math::sin(TAU * (start + time.as_seconds()) / 4.0)
                })
                .effect(eq(EqKind::LowPass, 3200.0, 0.7, 0.0))
                .effect(Effect::Delay {
                    delay: 375.milliseconds(),
                    feedback: 0.45,
                    wet: 0.35,
                    tail: 1500.milliseconds(),
                })
                .effect(Effect::Reverb {
                    room: 0.55,
                    damping: 0.5,
                    wet: 0.22,
                    tail: 1500.milliseconds(),
                }),
        );
    }

    let phrase = |time: Time| {
        let local = time.as_seconds();
        if (0.0..1.8).contains(&local) {
            math::pow(math::sin(PI * (local / 0.45).fract()).max(0.0), 0.6)
        } else {
            0.0
        }
    };
    clips.push(
        AudioClip::source(
            Synth::new(Wave::Saw, 150.0, RATE, seed)?
                .chirp(190.0, 1750.milliseconds())?
                .gain_db(-9.0)?,
        )
        .start(seconds(5.5))
        .bus("voice")
        .volume(0.8)
        .volume_envelope(phrase)
        .effect(eq(EqKind::Peak, 650.0, 2.0, 10.0))
        .effect(eq(EqKind::Peak, 1200.0, 3.0, 6.0))
        .effect(eq(EqKind::LowPass, 3800.0, 0.7, 0.0))
        .effect(Effect::Compressor {
            threshold_db: -24.0,
            ratio: 3.0,
            attack: 5.milliseconds(),
            release: 80.milliseconds(),
        }),
    );
    Ok(Soundtrack { buses, clips })
}

pub(super) fn audio_timeline(track: &Soundtrack, duration: Time) -> Timeline {
    let mut timeline = Timeline::new(Size::new(960.0, 540.0), 30);
    for bus in &track.buses {
        timeline = timeline.audio_bus(bus.clone());
    }
    for clip in &track.clips {
        timeline = timeline.audio(clip.clone());
    }
    timeline.master(master()).duration(duration)
}

pub(super) fn meter(mix: &MixedAudio, name: &str) -> motion::Result<BusMeter> {
    mix.bus_meters()
        .iter()
        .find(|meter| meter.name == name)
        .cloned()
        .ok_or_else(|| std::io::Error::other(format!("Missing bus meter {name}")).into())
}

/// Bus RMS in dBFS, interpolated between the 50 ms meter hops.
pub(super) fn rms_db(meter: &BusMeter, time: f64) -> f64 {
    let base = (time / METER_HOP).floor() * METER_HOP;
    let amount = (time - base) / METER_HOP;
    let at = |position: f64| meter.level(seconds(position.max(0.0)))[0];
    at(base) + (at(base + METER_HOP) - at(base)) * amount
}

/// Measured gain reduction in dB (zero or negative) between a ducked bus and its dry twin.
pub(super) fn duck_db(ducked: &BusMeter, dry: &BusMeter, time: f64) -> f64 {
    let reference = rms_db(dry, time);
    if reference < -70.0 {
        0.0
    } else {
        (rms_db(ducked, time) - reference).min(0.0)
    }
}
