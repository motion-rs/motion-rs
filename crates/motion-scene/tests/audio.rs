//! Audio buses, effects, ducking, pan and volume automation in declarative documents.
#![cfg(feature = "audio")]

use motion_core::{AudioSource, ErrorKind, ValidationError};
use motion_encode::audio::MixedAudio;
use motion_scene::*;
use std::path::Path;

const RATE: u32 = 8000;

fn write_tone(path: &Path, seconds: f64, amplitude: f32) {
    let frames = (f64::from(RATE) * seconds) as usize;
    let mut data = Vec::new();
    for frame in 0..frames {
        let phase = frame as f32 / RATE as f32 * 440.0 * std::f32::consts::TAU;
        data.extend_from_slice(&((phase.sin() * amplitude * 32767.0) as i16).to_le_bytes());
    }
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    std::fs::write(path, wav).unwrap();
}

struct Files(tempfile::TempDir);

impl Files {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        write_tone(&directory.path().join("voice.wav"), 1.0, 0.5);
        write_tone(&directory.path().join("music.wav"), 2.0, 0.5);
        Self(directory)
    }

    fn compile(&self, document: &Document) -> motion_core::Result<CompiledDocument> {
        document.compile(&CompileOptions::default().base_dir(self.0.path()))
    }
}

fn base() -> Document {
    Document::new([8.0, 8.0], 10)
        .track(TrackSpec::new().scene(SceneSpec::new(2.0, Element::default())))
}

/// Stereo RMS of `[start, end)` seconds, as `(left, right)`.
fn rms(mix: &MixedAudio, start: f64, end: f64) -> (f64, f64) {
    let first = (start * f64::from(RATE)) as u64;
    let frames = ((end - start) * f64::from(RATE)) as usize;
    let mut buffer = vec![0.0_f32; frames * 2];
    let read = mix.read(first, &mut buffer);
    assert_eq!(read, frames);
    let energy = |channel: usize| {
        let sum: f64 = buffer
            .chunks(2)
            .map(|frame| f64::from(frame[channel]).powi(2))
            .sum();
        (sum / frames as f64).sqrt()
    };
    (energy(0), energy(1))
}

fn mix(film: &CompiledDocument) -> MixedAudio {
    MixedAudio::from_timeline(film.timeline(), RATE).unwrap()
}

fn level(mix: &MixedAudio, start: f64, end: f64) -> f64 {
    let (left, right) = rms(mix, start, end);
    left.hypot(right)
}

#[test]
fn clips_duck_under_earlier_clips() {
    let files = Files::new();
    let plain = base()
        .audio(AudioSpec::new("voice.wav").id("voice"))
        .audio(AudioSpec::new("music.wav"));
    let ducked = base().audio(AudioSpec::new("voice.wav").id("voice")).audio(
        AudioSpec::new("music.wav").ducked_by(DuckSpec {
            depth_db: Some(24.0),
            attack: Some(0.01),
            release: Some(0.05),
            ..DuckSpec::new(DuckTriggerSpec::Clip("voice".into()))
        }),
    );
    let plain = mix(&files.compile(&plain).unwrap());
    let ducked = mix(&files.compile(&ducked).unwrap());
    let (during, after) = ((0.3, 0.9), (1.4, 1.9));
    assert!(
        level(&ducked, during.0, during.1) < level(&plain, during.0, during.1) * 0.8,
        "music is lowered while the voice plays"
    );
    let ratio = level(&ducked, after.0, after.1) / level(&plain, after.0, after.1);
    assert!(
        (ratio - 1.0).abs() < 0.05,
        "music recovers after the voice: {ratio}"
    );
}

#[test]
fn buses_apply_gain_and_effects() {
    let files = Files::new();
    let direct = base().audio(AudioSpec::new("music.wav"));
    let routed = base()
        .bus(BusSpec {
            gain_db: Some(-12.0),
            effects: vec![EffectSpec::Eq {
                kind: EqKindSpec::LowPass,
                frequency: Animated::expr("2000"),
                q: None,
                gain_db: None,
            }],
            ..BusSpec::new("music")
        })
        .master(MasterSpec {
            limiter: Some(LimiterSpec {
                ceiling_db: -1.0,
                lookahead: None,
                release: None,
            }),
            ..MasterSpec::default()
        })
        .audio(AudioSpec::new("music.wav").bus("music"));
    let routed = files.compile(&routed).unwrap();
    assert_eq!(routed.timeline().soundtrack().buses().len(), 1);
    assert!(routed.timeline().soundtrack().master().limiter.is_some());
    let ratio =
        level(&mix(&routed), 0.5, 1.5) / level(&mix(&files.compile(&direct).unwrap()), 0.5, 1.5);
    assert!(
        (ratio - 0.251).abs() < 0.03,
        "-12 dB is a 0.25 gain: {ratio}"
    );

    let ducked_bus = base()
        .audio(AudioSpec::new("voice.wav").id("voice"))
        .bus(BusSpec::new("dialogue"))
        .bus(BusSpec {
            ducked_by: vec![DuckSpec::new(DuckTriggerSpec::Bus("dialogue".into()))],
            ..BusSpec::new("music")
        })
        .audio(AudioSpec::new("music.wav").bus("music"));
    assert!(files.compile(&ducked_bus).is_ok());
}

#[test]
fn pan_and_volume_can_be_automated() {
    let files = Files::new();
    let film = files
        .compile(
            &base().audio(
                AudioSpec::new("music.wav")
                    .pan(Animated::tween(-1.0, 1.0, 2.0))
                    .volume(Animated::expr("1 - t / 4")),
            ),
        )
        .unwrap();
    let mix = mix(&film);
    let (left_early, right_early) = rms(&mix, 0.0, 0.3);
    let (left_late, right_late) = rms(&mix, 1.7, 2.0);
    assert!(left_early > right_early * 3.0, "{left_early} {right_early}");
    assert!(right_late > left_late * 3.0, "{left_late} {right_late}");
}

#[test]
fn effects_and_spatialization_compile_and_mix() {
    let files = Files::new();
    let document = base().audio(AudioSpec {
        pan_law: Some(PanLawSpec::Minus4Point5Db),
        stereo_width: Some(1.5),
        effects: vec![
            EffectSpec::Eq {
                kind: EqKindSpec::Peak,
                frequency: Animated::tween(300.0, 900.0, 2.0),
                q: Some(1.0),
                gain_db: Some(Animated::expr("3 * sin(t)")),
            },
            EffectSpec::Compressor {
                threshold_db: -20.0,
                ratio: 3.0,
                attack: None,
                release: None,
            },
            EffectSpec::Delay {
                delay: 0.2,
                feedback: None,
                wet: Some(0.2),
                tail: None,
            },
            EffectSpec::Reverb {
                room: 0.4,
                damping: None,
                wet: None,
                tail: Some(0.5),
            },
        ],
        spatial: Some(SpatialSpec {
            position: Animated::tween([0.0, 0.0], [8.0, 0.0], 2.0),
            listener: [4.0, 4.0],
            width: 4.0,
            rolloff: 0.5,
        }),
        ..AudioSpec::new("music.wav")
    });
    let film = files.compile(&document).unwrap();
    assert!(level(&mix(&film), 0.2, 0.8) > 0.01);
}

#[test]
fn audio_documents_round_trip_and_report_errors() {
    let document = base()
        .master(MasterSpec {
            gain_db: Some(-3.0),
            normalize: Some(LoudnessSpec {
                target_lufs: -16.0,
                true_peak_db: -1.0,
            }),
            ..MasterSpec::default()
        })
        .audio(
            AudioSpec::new("a.wav")
                .id("a")
                .bus("mix")
                .pan(Animated::expr("sin(t)")),
        );
    let json = document.to_json().unwrap();
    assert_eq!(Document::from_json(&json).unwrap(), document);

    let files = Files::new();
    let broken = base()
        .audio(AudioSpec::new("voice.wav").id("voice"))
        .audio(AudioSpec::new("music.wav").id("voice"))
        .audio(
            AudioSpec::new("music.wav")
                .ducked_by(DuckSpec::new(DuckTriggerSpec::Clip("later".into()))),
        )
        .audio(
            AudioSpec::new("music.wav").ducked_by(DuckSpec::new(DuckTriggerSpec::Bus("b".into()))),
        )
        .audio(AudioSpec::new("../escape.wav"))
        .audio(AudioSpec {
            volume: Some(Animated::Fixed(40.0)),
            ..AudioSpec::new("music.wav")
        })
        .bus(BusSpec {
            ducked_by: vec![DuckSpec::new(DuckTriggerSpec::Bus("missing".into()))],
            ..BusSpec::new("late")
        });
    let error = files.compile(&broken).err().unwrap();
    assert_eq!(error.kind(), ErrorKind::Validation);
    let found: Vec<String> = error
        .build_errors()
        .iter()
        .map(|error| {
            error
                .downcast_ref::<ValidationError>()
                .unwrap()
                .message
                .clone()
        })
        .collect();
    for expected in [
        "Duplicate audio id",
        "No earlier audio clip with id `later`",
        "bus triggers apply to buses",
        "must not contain `..`",
        "0..=16",
        "No earlier bus named `missing`",
    ] {
        assert!(
            found.iter().any(|message| message.contains(expected)),
            "{expected}: {found:?}"
        );
    }
}
