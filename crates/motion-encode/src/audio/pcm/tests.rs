//! Unit tests for PCM buffers, timeline mixing, bus metering and resampling.

use super::*;
use motion_core::TimeLiteral;

#[test]
fn nested_audio_preparation_depth_fails_without_poisoning_following_mixes() -> motion_core::Result {
    use motion_core::{Size, Timeline};
    let mut timeline = Timeline::new(Size::new(8.0, 8.0), 30)
        .audio(AudioClip::source(PcmBuffer::new(8000, 1, vec![0.1; 8])?))
        .build()?;
    for _ in 0..17 {
        timeline = Timeline::new(Size::new(8.0, 8.0), 30)
            .audio(AudioClip::composition(timeline).duration(1.milliseconds()))
            .build()?;
    }
    assert!(MixedAudio::from_timeline(&timeline, 8000).is_err());
    assert!(
        MixedAudio::new(
            &[AudioClip::source(PcmBuffer::new(8000, 1, vec![0.1; 8])?)],
            8000,
            1.milliseconds()
        )
        .is_ok()
    );
    Ok(())
}

#[test]
fn bus_tails_meter_windows_and_spatial_controls_are_preserved() -> motion_core::Result {
    use motion_core::{AudioBus, PanLaw, Point, Size, Timeline};
    let pcm = PcmBuffer::new(
        8000,
        2,
        (0..8000)
            .flat_map(|frame| if frame < 4000 { [0.1, 0.1] } else { [0.5, 0.5] })
            .collect(),
    )?;
    let clip = AudioClip::source(pcm).bus("echo");
    let mut bus = AudioBus::new("echo");
    bus.effects
        .push(Arc::new(super::super::effects::Effect::Delay {
            delay: 100.milliseconds(),
            feedback: 0.0,
            wet: 0.5,
            tail: 200.milliseconds(),
        }));
    let timeline = Timeline::new(Size::new(8.0, 8.0), 30)
        .audio(clip.clone())
        .audio_bus(bus.clone())
        .build()?;
    assert_eq!(timeline.duration(), 1200.milliseconds());
    assert_eq!(
        Timeline::new(Size::new(8.0, 8.0), 30)
            .audio_bus(bus)
            .audio(clip)
            .build()?
            .duration(),
        1200.milliseconds()
    );
    let mixed = MixedAudio::from_timeline(&timeline, 8000)?;
    let mut tail = [0.0; 2];
    mixed.read(8400, &mut tail);
    assert!(tail[0] > 0.0);
    let expected = mixed.bus_meters()[0].level(625.milliseconds());
    let cropped = mixed.window(625.milliseconds()..1125.milliseconds())?;
    assert_eq!(cropped.bus_meters()[0].level(Time::ZERO), expected);
    assert_eq!(
        cropped.bus_meters()[0].level(500.milliseconds()),
        [-96.0; 2]
    );
    let stereo = PcmBuffer::new(8000, 2, vec![0.4, -0.4])?;
    let mut mono = [1.0; 2];
    MixedAudio::new(
        &[AudioClip::source(stereo).stereo_width(0.0)],
        8000,
        1.milliseconds(),
    )?
    .read(0, &mut mono);
    assert_eq!(mono, [0.0; 2]);
    for law in [PanLaw::Linear, PanLaw::ConstantPower, PanLaw::Minus4_5dB] {
        let source = PcmBuffer::new(8000, 2, vec![0.25; 2])?;
        let mut value = [0.0; 2];
        MixedAudio::new(
            &[AudioClip::source(source).pan_law(law)],
            8000,
            1.milliseconds(),
        )?
        .read(0, &mut value);
        assert_eq!(value, [0.25; 2]);
    }
    let panned = |clip: AudioClip| -> motion_core::Result<[f32; 2]> {
        let mut value = [0.0; 2];
        MixedAudio::new(&[clip.pan(0.5)], 8000, 1.milliseconds())?.read(0, &mut value);
        Ok(value)
    };
    let stereo = || PcmBuffer::new(8000, 2, vec![0.25; 2]);
    assert_eq!(
        panned(AudioClip::source(stereo()?))?,
        panned(AudioClip::source(stereo()?).pan_law(PanLaw::ConstantPower))?,
        "Stereo balance defaults to constant power"
    );
    assert_ne!(
        panned(AudioClip::source(stereo()?))?,
        panned(AudioClip::source(stereo()?).pan_law(PanLaw::Linear))?
    );
    let source = PcmBuffer::new(8000, 1, vec![0.5; 8])?;
    let spatial = AudioClip::source(source).spatial(
        motion_core::Constant::new(Point::new(100.0, 0.0)),
        Point::new(0.0, 0.0),
        100.0,
        0.0,
    )?;
    let mut value = [0.0; 2];
    MixedAudio::new(&[spatial], 8000, 1.milliseconds())?.read(0, &mut value);
    assert!(value[0].abs() < 1e-6);
    assert_eq!(value[1], 0.5);
    Ok(())
}

#[test]
fn master_gain_identity_and_random_access() -> motion_core::Result {
    let clip = AudioClip::source(PcmBuffer::new(1000, 2, vec![0.25; 200])?);
    let mixed = MixedAudio::new(&[clip], 1000, 100.milliseconds())?;
    let mut original = [0.0; 200];
    mixed.read(0, &mut original);
    let mut identity = [0.0; 200];
    mixed.clone().gain_db(0.0)?.read(0, &mut identity);
    assert_eq!(identity, original);
    let gained = mixed.gain_db(-6.0)?;
    let mut whole = [0.0; 200];
    gained.read(0, &mut whole);
    let mut window = [0.0; 18];
    gained.read(31, &mut window);
    assert_eq!(window, whole[62..80]);
    assert!((whole[0] - 0.1252968).abs() < 1e-6);
    assert!(gained.gain_db(f64::NAN).is_err());
    Ok(())
}

#[test]
#[cfg(feature = "audio")]
fn buses_nested_master_and_local_sound_declarations_are_preserved() -> motion_core::Result {
    use motion_core::{AudioBus, Master, Node, Scene, Size, Timeline};
    let pcm = PcmBuffer::new(8000, 2, vec![0.1; 16000])?;
    let mut bus = AudioBus::new("voice");
    bus.gain_db = -6.0;
    let child = Timeline::new(Size::new(8.0, 8.0), 30)
        .audio(AudioClip::source(pcm.clone()).bus("voice"))
        .audio_bus(bus)
        .master(Master {
            gain_db: -6.0,
            ..Default::default()
        })
        .build()?;
    let parent = Timeline::new(Size::new(8.0, 8.0), 30)
        .track(|track| track.push(child.clone()))
        .master(Master {
            gain_db: -6.0,
            ..Default::default()
        })
        .build()?;
    let nested = MixedAudio::from_timeline(&parent, 8000)?;
    let expected = 0.1 * super::super::effects::amplitude(-18.0);
    let mut samples = [0.0; 2];
    nested.read(4000, &mut samples);
    assert!((f64::from(samples[0]) - expected).abs() < 1e-6);
    let window = MixedAudio::from_window(&parent, 500.milliseconds()..750.milliseconds(), 8000)?;
    let mut seek = [0.0; 2];
    window.read(0, &mut seek);
    assert_eq!(samples, seek);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = calls.clone();
    let scene = motion_core::scene_with_sound(
        move |context| -> motion_core::Result<Node> {
            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            context.sound(AudioClip::source(pcm.clone()).start(100.milliseconds()))?;
            Ok(Node::empty())
        },
        1.seconds(),
        Size::new(8.0, 8.0),
        30,
    );
    assert_eq!(scene.audio_clips()?.len(), 1);
    assert_eq!(scene.audio_clips()?.len(), 1);
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    let timeline = Timeline::new(Size::new(8.0, 8.0), 30)
        .track(|track| track.gap(1.seconds()).push(scene))
        .build()?;
    let mix = MixedAudio::from_timeline(&timeline, 8000)?;
    mix.read(4000, &mut samples);
    assert_eq!(samples, [0.0; 2]);
    mix.read(9600, &mut samples);
    assert_eq!(samples, [0.1; 2]);
    let many = vec![AudioClip::source(PcmBuffer::new(4, 1, vec![0.01])?); 64];
    assert!(MixedAudio::new(&many, 4, 1.seconds()).is_ok());
    Ok(())
}

#[test]
fn envelope_ticks_round_trip_at_fractional_sample_boundaries_and_hard_pan_is_exact()
-> motion_core::Result {
    let boundary = Time::from_frame(1, motion_core::FrameRate::from(8192));
    let envelope = motion_core::Tween::new(0.0, 1.0, boundary);
    let clip = AudioClip::source(PcmBuffer::new(8192, 1, vec![1.0; 8])?)
        .pan(1.0)
        .volume_envelope(envelope);
    let mixed = MixedAudio::new(&[clip], 8192, 1.milliseconds())?;
    let mut frame = [7.0; 2];
    mixed.read(1, &mut frame);
    assert_eq!(frame, [0.0, 1.0]);
    assert!(
        MixedAudio::new(
            &std::iter::repeat_n(AudioClip::source(PcmBuffer::new(4, 1, vec![1.0])?), 257)
                .collect::<Vec<_>>(),
            4,
            1.seconds()
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn overlapping_clips_add_without_normalization_and_faulty_sources_fail() -> motion_core::Result {
    let source = PcmBuffer::new(4, 2, [1.0, 0.0].repeat(4))?;
    let mixed = MixedAudio::new(
        &[
            AudioClip::source(source.clone()),
            AudioClip::source(source)
                .start(500.milliseconds())
                .duration(500.milliseconds())
                .volume(0.5),
        ],
        4,
        1.seconds(),
    )?;
    let mut output = [0.0; 8];
    mixed.read(0, &mut output);
    assert_eq!(output, [1.0, 0.0, 1.0, 0.0, 1.5, 0.0, 1.5, 0.0]);

    struct Fault;

    impl AudioSource for Fault {
        fn sample_rate(&self) -> u32 {
            4
        }

        fn channels(&self) -> u16 {
            1
        }

        fn duration(&self) -> Option<Duration> {
            Some(1.seconds())
        }

        fn read(&self, _: u64, output: &mut [f32]) -> usize {
            output.len() + 1
        }
    }
    assert!(MixedAudio::new(&[AudioClip::source(Fault)], 4, 1.seconds()).is_err());
    let bad_envelope = AudioClip::source(PcmBuffer::new(4, 1, vec![1.0])?)
        .volume_envelope(motion_core::Constant::new(f64::NAN));
    assert!(MixedAudio::new(&[bad_envelope], 4, 1.seconds()).is_err());
    Ok(())
}

#[test]
fn mix_positions_offsets_fades_envelopes_pan_eof_and_replay_are_exact() -> motion_core::Result {
    let source = PcmBuffer::new(4, 2, vec![1.0, 0.5, 0.8, 0.4, 0.6, 0.3, 0.4, 0.2])?;
    let clip = AudioClip::source(source)
        .start(500.milliseconds())
        .offset(250.milliseconds())
        .duration(1.seconds())
        .volume(0.5)
        .pan(0.5)
        .pan_law(motion_core::PanLaw::Linear)
        .fade_in(500.milliseconds())
        .fade_out(500.milliseconds())
        .volume_envelope(motion_core::Constant::new(0.5));
    let mix = MixedAudio::new(&[clip], 4, 2.seconds())?;
    let mut all = [0.0; 16];
    assert_eq!(mix.read(0, &mut all), 8);
    assert_eq!(&all[..6], &[0.0; 6]);
    assert!((all[6] - 0.0375).abs() < 1e-7 && (all[7] - 0.0375).abs() < 1e-7);
    assert!((all[8] - 0.05).abs() < 1e-7 && (all[9] - 0.05).abs() < 1e-7);
    assert_eq!(&all[10..], &[0.0; 6]);
    for start in [4, 0, 3, 2, 4] {
        let mut value = [0.0; 2];
        mix.read(start, &mut value);
        assert_eq!(value, all[start as usize * 2..start as usize * 2 + 2]);
    }
    let mut wave = Vec::new();
    mix.write_wav(&mut wave, 8)?;
    #[cfg(feature = "audio")]
    {
        let decoded = PcmBuffer::from_bytes(&wave)?;
        assert_eq!(decoded.samples(), all);
    }
    #[cfg(not(feature = "audio"))]
    assert!(PcmBuffer::from_bytes(&wave).is_err());
    let mono = AudioClip::source(PcmBuffer::new(4, 1, vec![1.0; 4])?).pan(-1.0);
    let mono = MixedAudio::new(&[mono], 4, 1.seconds())?;
    let mut value = [0.0; 2];
    mono.read(0, &mut value);
    assert_eq!(value, [1.0, 0.0]);
    assert!(
        MixedAudio::new(
            &[AudioClip::source(PcmBuffer::new(4, 1, vec![0.0; 4])?).pan(f64::NAN)],
            4,
            1.seconds()
        )
        .is_err()
    );
    Ok(())
}

#[test]
#[cfg(feature = "audio")]
fn rubato_resampling_preserves_tone_and_chunk_independent_queries() -> motion_core::Result {
    let input = (0..4410)
        .map(|index| {
            (libm::sin(std::f64::consts::TAU * 1000.0 * f64::from(index) / 44_100.0) * 0.5) as f32
        })
        .collect();
    let clip = AudioClip::source(PcmBuffer::new(44_100, 1, input)?).pan(-1.0);
    let mix = MixedAudio::new(&[clip], 48_000, 100.milliseconds())?;
    assert_eq!(mix.frames(), 4800);
    let mut output = vec![0.0; 9600];
    mix.read(0, &mut output);
    let error = (200..4600)
        .map(|index| {
            (f64::from(output[index * 2])
                - libm::sin(std::f64::consts::TAU * 1000.0 * index as f64 / 48_000.0) * 0.5)
                .abs()
        })
        .fold(0.0, f64::max);
    assert!(error < 0.002, "resampler tone error={error}");
    let mut slice = vec![0.0; 258];
    mix.read(503, &mut slice);
    assert_eq!(slice, output[1006..1264]);
    Ok(())
}

#[test]
fn pcm_random_reads_eof_and_invalid_inputs_preserve_frame_contract() {
    let pcm = PcmBuffer::new(48_000, 2, vec![1.0, -1.0, 0.5, -0.5, 0.25, -0.25]).unwrap();
    for start in [2, 0, 1, 2, 3, u64::MAX] {
        let mut output = [7.0; 5];
        let frames = pcm.read(start, &mut output);
        assert_eq!(frames, (3u64.saturating_sub(start) as usize).min(2));
        if frames != 0 {
            assert_eq!(
                &output[..frames * 2],
                &pcm.samples()[start as usize * 2..(start as usize + frames) * 2]
            );
        }
        assert!(output[frames * 2..].iter().all(|value| *value == 0.0));
    }
    assert!(PcmBuffer::new(0, 1, vec![0.0]).is_err());
    assert!(PcmBuffer::new(48_000, 2, vec![0.0]).is_err());
    assert!(PcmBuffer::new(48_000, 1, vec![f32::NAN]).is_err());
    assert!(PcmBuffer::new(48_000, 1, vec![]).is_err());
    let clone = pcm.clone();
    assert!(Arc::ptr_eq(&pcm.samples, &clone.samples));
}

#[test]
fn procedural_audio_requires_duration_and_is_frozen_before_random_mix_reads() -> motion_core::Result
{
    struct Tone(Arc<std::sync::atomic::AtomicUsize>);

    impl AudioSource for Tone {
        fn sample_rate(&self) -> u32 {
            48_000
        }

        fn channels(&self) -> u16 {
            1
        }

        fn duration(&self) -> Option<Duration> {
            None
        }

        fn read(&self, start: u64, output: &mut [f32]) -> usize {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            for (index, value) in output.iter_mut().enumerate() {
                *value = libm::sin(
                    std::f64::consts::TAU * 137.0 * (start + index as u64) as f64 / 48_000.0,
                ) as f32;
            }
            output.len()
        }
    }
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let clip = AudioClip::source(Tone(calls.clone()));
    assert!(MixedAudio::new(std::slice::from_ref(&clip), 48_000, 1.seconds()).is_err());
    let mixed = MixedAudio::new(
        &[clip
            .duration(50.milliseconds())
            .offset(25.milliseconds())
            .pan(-1.0)],
        48_000,
        50.milliseconds(),
    )?;
    let count = calls.load(std::sync::atomic::Ordering::Relaxed);
    assert!(count > 0);
    for start in [100, 0, 1000, 100] {
        let mut output = [0.0; 2];
        mixed.read(start, &mut output);
        let expected =
            libm::sin(std::f64::consts::TAU * 137.0 * (1200 + start) as f64 / 48_000.0) as f32;
        assert_eq!(output, [expected, 0.0]);
    }
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), count);
    std::thread::scope(|scope| {
        let tasks = (0..3)
            .map(|_| {
                scope.spawn(|| {
                    let mut output = [0.0; 64];
                    mixed.read(200, &mut output);
                    output
                })
            })
            .collect::<Vec<_>>();
        let expected = tasks
            .into_iter()
            .map(|task| task.join().unwrap())
            .collect::<Vec<_>>();
        assert!(expected.windows(2).all(|pair| pair[0] == pair[1]));
    });
    Ok(())
}

#[test]
#[cfg(feature = "audio")]
fn wav_decode_and_lazy_asset_handles_freeze_exact_pcm() -> motion_core::Result {
    let mut bytes = Vec::new();
    bytes.extend(b"RIFF");
    bytes.extend(44u32.to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(3u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(24_000u32.to_le_bytes());
    bytes.extend(96_000u32.to_le_bytes());
    bytes.extend(4u16.to_le_bytes());
    bytes.extend(32u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(8u32.to_le_bytes());
    for value in [0.25f32, -0.5] {
        bytes.extend(value.to_le_bytes());
    }
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("tone.wav");
    std::fs::write(&path, &bytes)?;
    let pcm = PcmBuffer::open(&path)?;
    std::fs::remove_file(path)?;
    assert_eq!(pcm.sample_rate(), 24_000);
    assert_eq!(pcm.channels(), 1);
    assert_eq!(pcm.samples(), [0.25, -0.5]);
    let assets = motion_core::Assets::new(directory.path())?;
    let source = motion_core::Source::Bytes {
        key: "tone".into(),
        data: bytes.into(),
    };
    let first = assets.load::<PcmBuffer>(source.clone());
    let second = assets.load::<PcmBuffer>(source);
    assert_eq!(first.status(), motion_core::AssetStatus::Unloaded);
    assert!(Arc::ptr_eq(&first.get().unwrap(), &second.get().unwrap()));
    assert!(PcmBuffer::from_bytes(b"not audio").is_err());
    Ok(())
}

#[test]
#[cfg(feature = "ffmpeg")]
#[ignore = "requires native FFmpeg/libx264/AAC; no devices, fonts or media executables"]
fn timeline_audio_exports_decodes_aac_and_protects_existing_output() -> motion_core::Result {
    use motion_core::authoring::Scene as _;
    let input = (0..4410)
        .map(|index| {
            (libm::sin(std::f64::consts::TAU * 1000.0 * f64::from(index) / 44_100.0) * 0.25) as f32
        })
        .collect();
    let clip = AudioClip::source(PcmBuffer::new(44_100, 1, input)?)
        .start(25.milliseconds())
        .pan(-1.0)
        .fade_in(5.milliseconds())
        .fade_out(5.milliseconds());
    let timeline = motion_core::Timeline::try_new(
        motion_core::Size::new(8.0, 4.0),
        motion_core::FrameRate::NTSC,
    )?
    .try_audio(clip)?
    .try_duration(Time::from_frame(7, motion_core::FrameRate::NTSC))?
    .build()?;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("native-audio.mp4");
    crate::export(
        &timeline,
        &crate::ExportOptions::new(&path).workers(1),
        &crate::NoProgress,
    )?;
    let metadata = motion_ffmpeg::inspect::media(&path)?;
    assert!(metadata.h264 && metadata.aac);
    assert_eq!(metadata.sample_rate, 48_000);
    assert_eq!(metadata.channels, 2);
    assert_eq!(motion_ffmpeg::inspect::video_frames(&path, 7)?, 7);
    let expected = crate::audio::video_samples(7, motion_core::FrameRate::NTSC)?;
    assert!((metadata.audio_seconds - expected as f64 / 48_000.0).abs() <= 1.0 / 48_000.0);
    let samples = motion_ffmpeg::inspect::audio_samples(&path, expected as usize + 2048)?;
    let energy = samples.iter().fold([0.0f64; 2], |sum, value| {
        [
            sum[0] + f64::from(value[0]).powi(2),
            sum[1] + f64::from(value[1]).powi(2),
        ]
    });
    assert!(energy[0] > 10.0 && energy[1] < energy[0] * 0.01);
    let decoded = PcmBuffer::open(&path)?;
    assert_eq!(decoded.sample_rate(), 48_000);
    assert_eq!(decoded.channels(), 2);
    assert!(decoded.samples().iter().any(|value| value.abs() > 0.1));
    assert_eq!(decoded.frames(), expected);
    let previous = std::fs::read(&path)?;

    struct Invalid;

    impl AudioSource for Invalid {
        fn sample_rate(&self) -> u32 {
            48_000
        }

        fn channels(&self) -> u16 {
            1
        }

        fn duration(&self) -> Option<Duration> {
            Some(1.seconds())
        }

        fn read(&self, _: u64, output: &mut [f32]) -> usize {
            output.fill(f32::NAN);
            output.len()
        }
    }
    let invalid = motion_core::Timeline::try_new(timeline.size(), timeline.frame_rate())?
        .try_audio(AudioClip::source(Invalid))?
        .build()?;
    assert!(
        crate::export(
            &invalid,
            &crate::ExportOptions::new(&path).workers(1).overwrite(true),
            &crate::NoProgress,
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&path)?, previous);
    assert_eq!(directory.path().read_dir()?.count(), 1);
    assert_eq!(timeline.audio_clips()?.len(), 1);
    println!(
        "audio export: frames=7 AAC=48000/stereo expected_samples={expected} decoded={} symphonia_frames={} pan/resample/fades/failure/cleanup=pass",
        samples.len(),
        decoded.frames()
    );
    Ok(())
}

fn ducking_fixture() -> motion_core::Result<(AudioClip, AudioClip)> {
    let rate = 8000;
    let voice: Vec<f32> = (0..rate)
        .flat_map(|frame| {
            let value = 0.5 * libm::sin(frame as f64 * 0.3) as f32;
            [value, value]
        })
        .collect();
    let music = PcmBuffer::new(rate, 2, vec![0.4; rate as usize * 8])?;
    let voice = AudioClip::source(PcmBuffer::new(rate, 2, voice)?).start(1.seconds());
    Ok((voice, AudioClip::source(music)))
}

fn peak(mix: &MixedAudio, from: f64, to: f64) -> f32 {
    let mut samples = vec![0.0; ((to - from) * 8000.0) as usize * 2];
    mix.read((from * 8000.0) as u64, &mut samples);
    samples
        .iter()
        .fold(0.0, |peak, value| value.abs().max(peak))
}

#[test]
fn clip_ducking_lowers_music_under_the_trigger_and_recovers() -> motion_core::Result {
    use motion_core::{Duck, Size, Timeline};
    let (voice, music) = ducking_fixture()?;
    let duck = Duck::new(-40.0, 12.0)
        .timing(20.milliseconds(), 100.milliseconds())
        .lookahead(10.milliseconds());
    let timeline = Timeline::new(Size::new(8.0, 8.0), 30)
        .audio(music.ducked_by(&voice, duck))
        .audio(voice.clone())
        .build()?;
    let mix = MixedAudio::from_timeline(&timeline, 8000)?;
    assert!((peak(&mix, 0.2, 0.8) - 0.4).abs() < 1e-6);
    let mid = f64::from(peak(&mix, 1.3, 1.6));
    let voice_only = f64::from(peak(
        &MixedAudio::from_timeline(
            &Timeline::new(Size::new(8.0, 8.0), 30)
                .audio(voice.clone())
                .build()?,
            8000,
        )?,
        1.3,
        1.6,
    ));
    assert!(mid < voice_only + 0.4 * 0.3, "{mid} {voice_only}");
    assert!((peak(&mix, 3.0, 3.5) - 0.4).abs() < 1e-4);
    let window = MixedAudio::from_window(&timeline, 900.milliseconds()..1400.milliseconds(), 8000)?;
    let (mut whole, mut cropped) = ([0.0; 2], [0.0; 2]);
    mix.read(7200 + 2000, &mut whole);
    window.read(2000, &mut cropped);
    assert_eq!(whole, cropped);
    Ok(())
}

#[test]
fn bus_ducking_uses_earlier_buses_and_rejects_forward_or_unknown_triggers() -> motion_core::Result {
    use motion_core::{AudioBus, Duck, DuckTrigger, Size, Timeline};
    let (voice, music) = ducking_fixture()?;
    let duck = Duck::new(-40.0, 20.0).timing(0.milliseconds(), 50.milliseconds());
    let build = |trigger: &str| {
        Timeline::new(Size::new(8.0, 8.0), 30)
            .audio(voice.clone().bus("voice"))
            .audio(music.clone().bus("music"))
            .audio_bus(AudioBus::new("voice"))
            .audio_bus(AudioBus::new("music").ducked_by(DuckTrigger::Bus(trigger.to_owned()), duck))
            .build()
            .unwrap()
    };
    let mix = MixedAudio::from_timeline(&build("voice"), 8000)?;
    let ducked = f64::from(peak(&mix, 1.3, 1.6));
    assert!(ducked < 0.5 + 0.4 * 0.11);
    assert!((peak(&mix, 0.2, 0.8) - 0.4).abs() < 1e-6);
    assert!(MixedAudio::from_timeline(&build("music"), 8000).is_err());
    assert!(MixedAudio::from_timeline(&build("missing"), 8000).is_err());
    let invalid_duck = Duck::new(5.0, 1.0);
    assert!(invalid_duck.validate().is_err());
    assert!(
        Timeline::new(Size::new(8.0, 8.0), 30)
            .audio(music.clone().ducked_by(&voice, invalid_duck))
            .build()
            .is_err()
    );
    Ok(())
}

#[test]
fn ducking_triggers_follow_relocation_and_sinc_mixing_is_selectable() -> motion_core::Result {
    use motion_core::{Duck, DuckTrigger};
    let (voice, music) = ducking_fixture()?;
    let ducked = music.ducked_by(&voice, Duck::default());
    let moved = ducked
        .relocated(2.seconds(), 10.seconds())?
        .expect("placed");
    let DuckTrigger::Clip(trigger) = &moved.parameters().ducking[0].trigger else {
        panic!("clip trigger");
    };
    assert_eq!(trigger.parameters().start, 3.seconds());
    let cropped = ducked
        .relocated(0.seconds(), 500.milliseconds())?
        .expect("placed");
    assert!(cropped.parameters().ducking.is_empty());
    let source = PcmBuffer::new(44_100, 1, vec![0.25; 4410])?;
    let timeline = motion_core::Timeline::new(motion_core::Size::new(8.0, 8.0), 30)
        .audio(AudioClip::source(source))
        .build()?;
    let sinc = MixedAudio::from_timeline_with(&timeline, 48_000, ResampleQuality::Sinc)?;
    let mut samples = [0.0; 2];
    sinc.read(2400, &mut samples);
    let expected = 0.25 * std::f32::consts::FRAC_1_SQRT_2;
    assert!((samples[0] - expected).abs() < 2e-3, "{samples:?}");
    Ok(())
}
