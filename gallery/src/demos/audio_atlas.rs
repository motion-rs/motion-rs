//! Audible synthesis and DSP studies whose dry and stereo waveforms use the actual prepared PCM.

use super::atlas::{self, Page};
use crate::{DemoMetadata, DemoParameters};
use motion::audio::effects::{Effect, EqKind};
use motion::audio::synth::{Adsr, Sound, Synth, Wave};
use motion::audio::{Duck, Master, MixedAudio, PcmBuffer};
use motion::extend::AudioSource;
use motion::prelude::*;
use std::sync::Arc;

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "audio-atlas",
    "Audio Atlas",
    "Audio",
    &[
        "synthesis",
        "audio-effects",
        "sidechain-ducking",
        "stereo",
        "master-limiter",
        "pcm-waveforms",
    ],
);

/// Native synthesis and mix rate in frames per second.
const RATE: u32 = 48_000;

/// Waveform inspection window in native audio frames, independent of output resolution.
const WINDOW_FRAMES: usize = 2048;

/// Fixed peak-envelope bins per audio page, shared by all three waveform lanes.
const OVERVIEW_COLUMNS: usize = 216;

/// Waveform start in viewport units, clear of the 12-unit track labels.
const TRACK_LEFT: f64 = 64.0;

/// Width of one overview column in viewport units; 216 columns end before the meters.
const TRACK_COLUMN: f64 = 1.8;

/// Waveform end in viewport units.
const TRACK_RIGHT: f64 = TRACK_LEFT + OVERVIEW_COLUMNS as f64 * TRACK_COLUMN;

const SOUNDS: [Sound; 11] = [
    Sound::Whoosh,
    Sound::Click,
    Sound::Pop,
    Sound::Tick,
    Sound::Riser,
    Sound::Impact,
    Sound::SubDrop,
    Sound::Kick,
    Sound::Snare,
    Sound::Hat,
    Sound::Ambience,
];

pub(super) const PAGES: &[Page] = &[
    Page {
        title: "Sine oscillator",
        lines: [
            "A pure sinusoidal tone",
            "has one fundamental with",
            "an analytically sampled phase.",
        ],
    },
    Page {
        title: "Saw oscillator",
        lines: [
            "A band-edge-corrected saw",
            "adds a rich harmonic series",
            "above the fundamental.",
        ],
    },
    Page {
        title: "Square oscillator",
        lines: [
            "A corrected square wave",
            "emphasizes odd harmonics",
            "with a hollow timbre.",
        ],
    },
    Page {
        title: "Triangle oscillator",
        lines: [
            "A triangle retains odd",
            "harmonics with a gentler",
            "high-frequency rolloff.",
        ],
    },
    Page {
        title: "White noise",
        lines: [
            "Seeded independent samples",
            "form a repeatable broadband",
            "source without audio files.",
        ],
    },
    Page {
        title: "Pink noise",
        lines: [
            "An octave-shaped source",
            "approximates equal energy",
            "across frequency octaves.",
        ],
    },
    Page {
        title: "Brown noise",
        lines: [
            "A seeded low-frequency",
            "noise approximation creates",
            "a darker continuous bed.",
        ],
    },
    Page {
        title: "ADSR envelope",
        lines: [
            "Attack, decay, sustain and",
            "release shape one finite note",
            "on its local source clock.",
        ],
    },
    Page {
        title: "Frequency chirp",
        lines: [
            "Integrated oscillator phase",
            "sweeps smoothly from low",
            "to high frequency.",
        ],
    },
    Page {
        title: "Drum machine",
        lines: [
            "A sixteen-step bit mask",
            "sequences synthesized kick,",
            "snare and hat voices.",
        ],
    },
    Page {
        title: "Low-pass EQ",
        lines: [
            "An RBJ low-pass removes",
            "upper harmonics from the",
            "same dry saw oscillator.",
        ],
    },
    Page {
        title: "Compression",
        lines: [
            "A threshold and ratio",
            "reduce peaks using explicit",
            "attack and release times.",
        ],
    },
    Page {
        title: "Noise gate",
        lines: [
            "A smoothed gate attenuates",
            "quiet portions of a source",
            "with a varying envelope.",
        ],
    },
    Page {
        title: "De-essing",
        lines: [
            "A high-band compressor",
            "reduces bright noise while",
            "retaining lower frequencies.",
        ],
    },
    Page {
        title: "Feedback delay",
        lines: [
            "Repeated echoes extend",
            "a short synthetic pluck",
            "inside a bounded tail.",
        ],
    },
    Page {
        title: "Reverb",
        lines: [
            "Damped combs and diffusion",
            "spread an impulse into a",
            "finite synthetic room tail.",
        ],
    },
    Page {
        title: "Sidechain ducking",
        lines: [
            "A kick trigger reduces",
            "the tone's level with real",
            "offline gain reduction.",
        ],
    },
    Page {
        title: "Animated pan",
        lines: [
            "Constant-power mono pan",
            "moves a sound between the",
            "left and right channels.",
        ],
    },
    Page {
        title: "Stereo width",
        lines: [
            "Mid/side width reshapes",
            "a genuine stereo source",
            "without altering its mid.",
        ],
    },
    Page {
        title: "Master limiter",
        lines: [
            "A boosted mix passes through",
            "lookahead peak limiting",
            "with a safe output ceiling.",
        ],
    },
    Page {
        title: "Sound presets",
        lines: [
            "Eleven generated accents",
            "play in a fixed sequence",
            "with no external recordings.",
        ],
    },
];

struct Study {
    dry: Arc<MixedAudio>,
    processed: Arc<MixedAudio>,
    duration: Time,
    envelopes: [Vec<f32>; 3],
}

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    let duration = parameters.seconds.seconds();
    let seed = parameters.seed as u32 ^ (parameters.seed >> 32) as u32;
    let mut studies = Vec::new();
    let mut clips = Vec::new();
    for page in 0..PAGES.len() {
        let start = Time::from_ticks(duration.ticks() * page as i64 / PAGES.len() as i64);
        let end = Time::from_ticks(duration.ticks() * (page + 1) as i64 / PAGES.len() as i64);
        let study = prepare(page, end - start, seed)?;
        clips.push(
            AudioClip::source(study.processed.clone())
                .start(start)
                .duration(end - start),
        );
        studies.push(study);
    }
    let mut timeline = atlas::builder(parameters, METADATA, PAGES, move |page, progress, _| {
        Ok(waveforms(&studies[page], progress))
    });
    for clip in clips {
        timeline = timeline.audio(clip);
    }
    timeline.build()
}

fn prepare(page: usize, duration: Time, seed: u32) -> motion::Result<Study> {
    let length = duration.as_seconds();
    let wave = match page {
        0 => Wave::Sine,
        2 => Wave::Square,
        3 => Wave::Triangle,
        4 | 12 | 13 => Wave::White,
        5 => Wave::Pink,
        6 => Wave::Brown,
        _ => Wave::Saw,
    };
    let source = Synth::new(wave, 220.0, RATE, seed)?.gain_db(-9.0)?;
    let mut dry = AudioClip::source(source)
        .duration(duration)
        .volume(match page {
            5 => 3.0,
            6 => 5.0,
            _ => 0.7,
        });
    if page == 8 {
        dry = AudioClip::source(
            Synth::new(Wave::Sine, 110.0, RATE, seed)?
                .chirp(1760.0, duration)?
                .gain_db(-9.0)?,
        )
        .duration(duration);
    } else if page == 9 {
        dry = AudioClip::source(Synth::drum_machine(
            120.0,
            [1, 0, 4, 0, 6, 0, 4, 0, 1, 0, 4, 4, 6, 0, 4, 4],
            duration,
            seed,
        )?)
        .duration(duration)
        .volume(0.45);
    } else if page == 14 || page == 15 {
        let note = (length * 0.2).seconds();
        dry = AudioClip::source(Synth::sound(Sound::Pop, note, seed)?)
            .duration(note)
            .volume(0.5);
    } else if page == 18 {
        dry = AudioClip::source(stereo(duration, seed)?)
            .duration(duration)
            .volume(0.5);
    }
    if page == 11 || page == 12 {
        dry = dry.volume_envelope(move |time: Time| {
            0.05 + 0.95 * (0.5 + 0.5 * math::sin(time.as_seconds() / length * 18.0)).powi(2)
        });
    }
    dry = dry
        .fade_in((length * 0.01).seconds())
        .fade_out((length * 0.02).seconds());
    let mut processed = dry.clone();
    match page {
        7 => {
            processed = processed.volume_envelope(Adsr::new(
                (length * 0.2).seconds(),
                (length * 0.2).seconds(),
                0.35,
                (length * 0.25).seconds(),
                duration,
            )?)
        }
        10 => {
            processed = processed.effect(Effect::Eq {
                kind: EqKind::LowPass,
                frequency: 500.0,
                q: 0.707,
                gain_db: 0.0,
            })
        }
        11 => {
            processed = processed.effect(Effect::Compressor {
                threshold_db: -24.0,
                ratio: 8.0,
                attack: 2.milliseconds(),
                release: 40.milliseconds(),
            })
        }
        12 => {
            processed = processed.effect(Effect::Gate {
                threshold_db: -20.0,
                attack: 2.milliseconds(),
                release: 15.milliseconds(),
            })
        }
        13 => {
            processed = processed.effect(Effect::DeEsser {
                frequency: 1800.0,
                threshold_db: -28.0,
                ratio: 10.0,
            })
        }
        14 => {
            processed = processed.effect(Effect::Delay {
                delay: (length * 0.18).seconds(),
                feedback: 0.55,
                wet: 0.65,
                tail: (length * 0.7).seconds(),
            })
        }
        15 => {
            processed = processed.effect(Effect::Reverb {
                room: 0.8,
                damping: 0.45,
                wet: 0.65,
                tail: (length * 0.7).seconds(),
            })
        }
        17 => {
            processed = processed.pan_animation(move |time: Time| {
                math::sin(
                    time.as_seconds() / length * std::f64::consts::TAU
                        - std::f64::consts::FRAC_PI_2,
                )
            })
        }
        18 => processed = processed.stereo_width(2.0),
        19 => processed = processed.volume(8.0),
        _ => {}
    }
    let blank = || Timeline::new(Size::new(960.0, 540.0), 30).duration(duration);
    let mut dry_timeline = blank().audio(dry);
    let mut processed_timeline = blank().audio(processed);
    if page == 16 {
        let trigger = AudioClip::source(Synth::drum_machine(
            120.0,
            [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0],
            duration,
            seed,
        )?)
        .duration(duration)
        .volume(0.5);
        let tone = AudioClip::source(Synth::new(Wave::Sine, 220.0, RATE, seed)?.gain_db(-10.0)?)
            .duration(duration);
        dry_timeline = blank().audio(tone.clone()).audio(trigger.clone());
        processed_timeline = blank()
            .audio(tone.ducked_by(
                &trigger,
                Duck::new(-35.0, 18.0).timing(2.milliseconds(), 150.milliseconds()),
            ))
            .audio(trigger);
    } else if page == 19 {
        processed_timeline = processed_timeline.master(Master::safe());
    } else if page == 20 {
        processed_timeline = blank();
        for (index, sound) in SOUNDS.into_iter().enumerate() {
            let start = Time::from_ticks(duration.ticks() * index as i64 / SOUNDS.len() as i64);
            let end = Time::from_ticks(duration.ticks() * (index + 1) as i64 / SOUNDS.len() as i64);
            processed_timeline = processed_timeline.audio(
                AudioClip::source(Synth::sound(
                    sound,
                    end - start,
                    seed.wrapping_add(index as u32),
                )?)
                .start(start)
                .duration(end - start)
                .volume(0.4),
            );
        }
        dry_timeline = processed_timeline.clone();
    }
    let dry = Arc::new(MixedAudio::from_timeline(&dry_timeline.build()?, RATE)?);
    let processed = Arc::new(MixedAudio::from_timeline(
        &processed_timeline.build()?,
        RATE,
    )?);
    let envelopes = overview(&dry, &processed);
    Ok(Study {
        dry,
        processed,
        duration,
        envelopes,
    })
}

fn overview(dry: &MixedAudio, processed: &MixedAudio) -> [Vec<f32>; 3] {
    let frames = processed.frames() as usize;
    let mut original = vec![0.0; frames * 2];
    let mut output = vec![0.0; frames * 2];
    dry.read(0, &mut original);
    processed.read(0, &mut output);
    let mut envelopes = std::array::from_fn(|_| vec![0.0_f32; OVERVIEW_COLUMNS]);
    for (row, envelope) in envelopes.iter_mut().enumerate() {
        for (column, peak) in envelope.iter_mut().enumerate() {
            let start = column * frames / OVERVIEW_COLUMNS;
            let end = (column + 1) * frames / OVERVIEW_COLUMNS;
            for frame in start..end {
                let sample = if row == 0 {
                    (original[frame * 2] + original[frame * 2 + 1]) * 0.5
                } else {
                    output[frame * 2 + row - 1]
                };
                *peak = peak.max(sample.abs());
            }
        }
    }
    envelopes
}

fn stereo(duration: Time, seed: u32) -> motion::Result<PcmBuffer> {
    let frames = (duration.as_seconds() * f64::from(RATE)).ceil() as usize;
    let mut left = vec![0.0; frames];
    let mut right = vec![0.0; frames];
    Synth::new(Wave::Sine, 220.0, RATE, seed)?
        .gain_db(-9.0)?
        .read(0, &mut left);
    Synth::new(Wave::Sine, 330.0, RATE, seed)?
        .gain_db(-9.0)?
        .read(0, &mut right);
    let samples = left
        .into_iter()
        .zip(right)
        .flat_map(|(left, right)| [left, right])
        .collect();
    Ok(PcmBuffer::new(RATE, 2, samples)?)
}

fn waveforms(study: &Study, progress: f64) -> Node {
    let center = (study.duration.as_seconds() * progress * f64::from(RATE)) as u64;
    let start = center.saturating_sub(WINDOW_FRAMES as u64 / 2);
    let mut dry = vec![0.0; WINDOW_FRAMES * 2];
    let mut processed = vec![0.0; WINDOW_FRAMES * 2];
    study.dry.read(start, &mut dry);
    study.processed.read(start, &mut processed);
    let peak = study
        .envelopes
        .iter()
        .flatten()
        .copied()
        .fold(0.0_f32, f32::max);
    let gain = (0.8 / f64::from(peak).max(0.001)).clamp(0.5, 8.0);
    let mut nodes = vec![atlas::text(
        &format!("{gain:.1}x / 48 kHz"),
        11.0,
        0xb9cec5,
        TRACK_LEFT,
        12.0,
    )];
    for (row, label) in ["Dry", "Left", "Right"].into_iter().enumerate() {
        let top = 60.0 + row as f64 * 82.0;
        let color = Color::from_hex([0x8eaaa0, atlas::MINT, atlas::GOLD][row]);
        for (column, peak) in study.envelopes[row].iter().enumerate() {
            let left = TRACK_LEFT + column as f64 * TRACK_COLUMN;
            let height = (f64::from(*peak) * gain).min(1.0) * 32.0;
            nodes.push(
                Rect::new(left, top - height, left + TRACK_COLUMN, top + height)
                    .fill(color.with_alpha(0.18))
                    .into(),
            );
        }
        let mut path = Path2D::new();
        for column in 0..OVERVIEW_COLUMNS {
            let frame = column * (WINDOW_FRAMES - 1) / (OVERVIEW_COLUMNS - 1);
            let sample = if row == 0 {
                (dry[frame * 2] + dry[frame * 2 + 1]) * 0.5
            } else {
                processed[frame * 2 + row - 1]
            };
            let point = Point::new(
                TRACK_LEFT + column as f64 * TRACK_COLUMN,
                top - f64::from(sample) * gain * 32.0,
            );
            if column == 0 {
                path.move_to(point);
            } else {
                path.line_to(point);
            }
        }
        nodes.push(Node::from(path.stroke(1.7, color)));
        nodes.push(atlas::text(label, 13.0, 0xb9cec5, 12.0, top - 7.0));
        let data = if row == 0 { &dry } else { &processed };
        let peak = data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|frame| {
                if row == 0 {
                    ((frame[0] + frame[1]) * 0.5).abs()
                } else {
                    frame[row - 1].abs()
                }
            })
            .fold(0.0_f32, f32::max)
            .min(1.0);
        nodes.push(
            Rect::new(
                476.0,
                top + 25.0 - f64::from(peak) * 50.0,
                486.0,
                top + 25.0,
            )
            .fill(color)
            .into(),
        );
    }
    nodes.push(
        Rect::new(TRACK_LEFT, 280.0, TRACK_RIGHT, 282.0)
            .fill(Color::from_hex(0x424e47))
            .into(),
    );
    nodes.push(
        Circle::new(
            Point::new(TRACK_LEFT + progress * (TRACK_RIGHT - TRACK_LEFT), 281.0),
            4.0,
        )
        .fill(Color::from_hex(atlas::BLUE))
        .into(),
    );
    Node::group(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audible_studies_are_finite_replayable_and_use_real_dsp() -> motion::Result {
        for page in 0..PAGES.len() {
            let study = prepare(page, 0.6.seconds(), 17)?;
            let mut first = vec![0.0; 28_800 * 2];
            let mut replay = first.clone();
            let mut dry = first.clone();
            study.processed.read(0, &mut first);
            study.processed.read(100, &mut replay[..400]);
            study.processed.read(0, &mut replay);
            study.dry.read(0, &mut dry);
            assert_eq!(first, replay, "audio replay page {page}");
            assert_eq!(study.envelopes, overview(&study.dry, &study.processed));
            for envelope in &study.envelopes {
                assert_eq!(envelope.len(), OVERVIEW_COLUMNS);
                assert!(envelope.iter().all(|peak| peak.is_finite() && *peak >= 0.0));
                assert!(
                    envelope.iter().any(|peak| *peak > 0.001),
                    "empty envelope page {page}"
                );
            }
            assert!(
                first.iter().all(|sample| sample.is_finite()),
                "nonfinite page {page}"
            );
            assert!(
                first.iter().any(|sample| sample.abs() > 0.001),
                "silent page {page}"
            );
            if matches!(page, 7 | 10..=19) {
                assert!(
                    first
                        .iter()
                        .zip(&dry)
                        .any(|(wet, dry)| (wet - dry).abs() > 0.0001),
                    "DSP did not alter page {page}"
                );
            }
            if page == 19 {
                assert!(
                    first.iter().all(|sample| sample.abs() <= 0.9),
                    "limiter ceiling"
                );
            }
            if page == 14 || page == 15 {
                let tail_start = RATE as usize / 3 * 2;
                assert!(
                    dry[tail_start..]
                        .iter()
                        .all(|sample| sample.abs() < 0.00001),
                    "dry note did not finish"
                );
                assert!(
                    first[tail_start..]
                        .iter()
                        .any(|sample| sample.abs() > 0.00001),
                    "missing effect tail on page {page}"
                );
            }
        }
        Ok(())
    }
}
