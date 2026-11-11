//! The synthesized Signal Garden mix: kick, hat, bass and lead buses with sidechain
//! ducks, bus and clip effects and a mastering chain, plus the bus-meter readings
//! the visuals use.

use super::{BPM, KICK_DEPTH_DB, NOTES, RATE};
use motion::audio::effects::{AnimatedEq, Effect, EqKind};
use motion::audio::pcm::BusMeter;
use motion::audio::synth::{Adsr, Synth, Wave};
use motion::audio::{AudioBus, Duck, DuckTrigger, Loudness, Master, MixedAudio};
use motion::prelude::*;
use std::f64::consts::TAU;
use std::sync::Arc;

/// Meter hop of `BusMeter`, in seconds.
const METER_HOP: f64 = 0.05;

pub(super) struct Soundtrack {
    buses: Vec<AudioBus>,
    clips: Vec<AudioClip>,
}

fn kick_duck(depth_db: f64) -> Duck {
    Duck::new(-30.0, depth_db)
        .ratio(100.0)
        .timing(5.milliseconds(), 240.milliseconds())
        .lookahead(5.milliseconds())
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

fn kick(duration: Duration, seed: u32) -> motion::Result<AudioClip> {
    let mut steps = [0; 16];
    for step in [0, 4, 8, 12] {
        steps[step] = 1;
    }
    Ok(
        AudioClip::source(Synth::drum_machine(BPM, steps, duration, seed)?)
            .bus("kick")
            .volume(0.9)
            .fade_out(100.milliseconds()),
    )
}

/// The pad's low-pass opens over the film, so the garden "blooms" as it plays.
fn bass_bus(duration: Duration, ducked: Option<&AudioClip>) -> AudioBus {
    let length = duration.as_seconds();
    let mut bus = AudioBus::new("bass");
    bus.effects.push(Arc::new(AnimatedEq::new(
        EqKind::LowPass,
        move |time: Time| 420.0 + 1100.0 * math::smoothstep(0.0, length, time.as_seconds()),
        0.8,
        0.0,
    )));
    match ducked {
        Some(kick) => bus.ducked_by(
            DuckTrigger::Clip(Arc::new(kick.clone())),
            kick_duck(KICK_DEPTH_DB),
        ),
        None => bus,
    }
}

/// Sub sine plus detuned saw pairs panned apart and widened, all on the bass bus.
fn bass_clips(duration: Duration, seed: u32) -> motion::Result<Vec<AudioClip>> {
    let mut clips = vec![
        AudioClip::source(Synth::new(Wave::Sine, 55.0, RATE, seed)?.gain_db(-3.0)?)
            .duration(duration)
            .bus("bass")
            .volume(0.5)
            .fade_in(30.milliseconds())
            .fade_out(100.milliseconds()),
    ];
    for (index, frequency) in [55.0, 82.41, 110.0, 138.59].into_iter().enumerate() {
        for (side, pan, detune) in [(0, -0.8, 0.997), (1, 0.8, 1.003)] {
            let source = Synth::new(
                Wave::Saw,
                frequency * detune,
                RATE,
                seed + 10 + index as u32 * 2 + side,
            )?
            .gain_db(-9.0)?;
            clips.push(
                AudioClip::source(source)
                    .duration(duration)
                    .bus("bass")
                    .volume(0.2)
                    .pan(pan)
                    .stereo_width(1.6)
                    .fade_in(30.milliseconds())
                    .fade_out(100.milliseconds()),
            );
        }
    }
    Ok(clips)
}

/// The complete mix. `ducked: false` keeps only the bass bus without its sidechain,
/// the reference level used to measure the real duck gain.
pub(super) fn soundtrack(
    duration: Duration,
    ducked: bool,
    seed: u32,
) -> motion::Result<Soundtrack> {
    if !ducked {
        return Ok(Soundtrack {
            buses: vec![bass_bus(duration, None)],
            clips: bass_clips(duration, seed)?,
        });
    }
    let kick = kick(duration, seed)?;
    let mut lead_bus =
        AudioBus::new("lead").ducked_by(DuckTrigger::Bus("kick".to_owned()), kick_duck(6.0));
    lead_bus.effects = vec![
        Arc::new(eq(EqKind::LowPass, 3600.0, 0.7, 0.0)),
        Arc::new(Effect::Delay {
            delay: 375.milliseconds(),
            feedback: 0.4,
            wet: 0.3,
            tail: 1500.milliseconds(),
        }),
        Arc::new(Effect::Reverb {
            room: 0.6,
            damping: 0.5,
            wet: 0.25,
            tail: 1500.milliseconds(),
        }),
    ];
    let buses = vec![
        AudioBus::new("kick"),
        AudioBus::new("hats"),
        bass_bus(duration, Some(&kick)),
        lead_bus,
    ];

    let mut hats = [0; 16];
    for step in [2, 6, 10, 13, 14, 15] {
        hats[step] = 4;
    }
    let mut clips = vec![
        kick,
        AudioClip::source(Synth::drum_machine(BPM, hats, duration, seed + 1)?)
            .bus("hats")
            .volume(0.3)
            .pan_animation(|time: Time| 0.6 * math::sin(TAU * time.as_seconds() / 2.0))
            .fade_out(100.milliseconds()),
    ];
    clips.extend(bass_clips(duration, seed)?);

    // Eighth-note plucks walk the pentatonic cycle; a faint air tone drifts across the field.
    let note = 250.milliseconds();
    let count = (duration.as_seconds() * BPM / 30.0).round() as usize;
    for step in 0..count {
        let frequency = NOTES[step % NOTES.len()];
        let start = step as f64 * 0.25;
        clips.push(
            AudioClip::source(
                Synth::new(Wave::Triangle, frequency, RATE, seed)?
                    .chirp(frequency, note)?
                    .gain_db(-8.0)?,
            )
            .start(Time::from_seconds(start))
            .bus("lead")
            .volume(0.6)
            .volume_envelope(Adsr::new(
                4.milliseconds(),
                120.milliseconds(),
                0.25,
                80.milliseconds(),
                note,
            )?)
            .pan(0.35 * math::sin(TAU * start / 4.0)),
        );
    }
    clips.push(
        AudioClip::source(Synth::new(Wave::Sine, 1320.0, RATE, seed)?.gain_db(-30.0)?)
            .duration(duration)
            .bus("lead")
            .pan_animation(|time: Time| 0.8 * math::sin(TAU * time.as_seconds() / 6.0))
            .stereo_width(1.4)
            .fade_in(300.milliseconds())
            .fade_out(100.milliseconds()),
    );
    Ok(Soundtrack { buses, clips })
}

pub(super) fn audio_timeline(track: &Soundtrack, duration: Duration) -> TimelineBuilder {
    let mut timeline = Timeline::new(Size::new(960.0, 540.0), motion::fps!(30));
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
    let at = |position: f64| meter.level(Duration::from_seconds(position.max(0.0)))[0];
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
