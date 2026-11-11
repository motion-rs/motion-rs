//! Native FFmpeg encoder validation, streaming, audio and cleanup tests.

use super::*;

#[cfg(feature = "ffmpeg")]
fn frame(
    width: usize,
    height: usize,
    background: motion_core::Color,
) -> motion_render::FrameBuffer {
    motion_render::Composition::new(width, height, motion_render::FrameClock::new(30, 150))
        .frame_renderer(motion_core::Size::new(width as f64, height as f64))
        .unwrap()
        .render_output_at(
            motion_core::Time::ZERO,
            Some(background),
            motion_render::PixelFormat::Yuv420 { bit_depth: 8 },
            |_| motion_core::Node::empty(),
        )
        .unwrap()
}

#[test]
fn fractional_frame_rate_validation_preserves_audio_duration() -> Result {
    let duration = motion_core::Time::from_seconds(1.0);
    let clip = motion_core::AudioClip::source(crate::PcmBuffer::new(48_000, 1, vec![0.0])?)
        .duration(duration.since_origin());
    let options = NativeOptions::default().audio_mix(crate::MixedAudio::new(
        crate::MixSource::clips(&[clip], duration.since_origin()),
        crate::MixOptions::new(48_000),
    )?)?;
    let rate = motion_core::FrameRate::NTSC;
    let frames =
        (u64::from(rate.numerator()) * audio::MAX_SECONDS / u64::from(rate.denominator())) as u32;
    assert!(validate_at_rate((2, 2), rate, frames, &options).is_ok());
    assert!(validate_at_rate((2, 2), rate, frames + 1, &options).is_err());
    assert!(
        validate_at_rate(
            (2, 2),
            motion_core::FrameRate::new(1, u32::MAX),
            1,
            &NativeOptions::default()
        )
        .is_err()
    );
    assert!(validate((2, 2), 0, 1, &NativeOptions::default()).is_err());
    Ok(())
}

#[test]
fn encoder_and_cancellation_error_diagnostics() -> Result {
    let directory = tempfile::tempdir()?;
    let destination = directory.path().join("existing.mp4");
    std::fs::write(&destination, b"committed")?;
    let error = Encoder::new(&destination, (2, 2), 30, 2, &NativeOptions::default())
        .err()
        .unwrap();
    assert_eq!(error.kind(), motion_core::ErrorKind::Io);
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(std::fs::read(&destination)?, b"committed");
    #[cfg(not(feature = "ffmpeg"))]
    {
        let error = Encoder::new(
            &directory.path().join("new.mp4"),
            (2, 2),
            30,
            2,
            &NativeOptions::default(),
        )
        .err()
        .unwrap();
        assert_eq!(error.kind(), motion_core::ErrorKind::Encoder);
        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::Unsupported
        );
    }
    assert_eq!(directory.path().read_dir()?.count(), 1);
    Ok(())
}

#[test]
#[cfg(feature = "ffmpeg")]
#[ignore = "requires native FFmpeg with libx264; no fonts or window"]
fn encoder_error_preserves_cause_and_cleanup() -> Result {
    let directory = tempfile::tempdir()?;
    let destination = directory.path().join("existing.mp4");
    std::fs::write(&destination, b"committed")?;
    let mut encoder = Encoder::new(
        &destination,
        (2, 2),
        30,
        1,
        &NativeOptions::default().overwrite(true),
    )?;
    let error = encoder
        .write_buffer(&frame(4, 4, motion_core::Color::BLACK))
        .unwrap_err()
        .at_frame(6);
    assert_eq!(error.kind(), motion_core::ErrorKind::Encoder);
    assert_eq!(error.frame(), Some(6));
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(
        error
            .to_string()
            .contains("Unexpected YUV planes/count or failed encoder")
    );
    drop(encoder);
    assert_eq!(std::fs::read(&destination)?, b"committed");
    assert_eq!(directory.path().read_dir()?.count(), 1);
    Ok(())
}

#[test]
fn invalid_encoding_settings_are_rejected_before_opening_resources() {
    let options = NativeOptions::default();
    assert!(validate((640, 360), 30, 60, &options).is_ok());
    for size in [(0, 360), (641, 360), (640, 361)] {
        assert!(validate(size, 30, 60, &options).is_err());
    }
    assert!(validate((640, 360), 0, 60, &options).is_err());
    assert!(validate((640, 360), 30, 0, &options).is_err());
    assert!(validate((640, 360), 30, 60, &options.crf(52)).is_err());
}

#[cfg(not(feature = "ffmpeg"))]
#[test]
fn disabled_export_leaves_no_partial_output() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("out.mp4");
    let error = Encoder::new(&destination, (64, 64), 30, 1, &NativeOptions::default())
        .err()
        .unwrap();
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::Unsupported
    );
    assert_eq!(directory.path().read_dir().unwrap().count(), 0);
}

#[test]
fn existing_video_is_preserved_without_launching_the_encoder() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("out.mp4");
    std::fs::write(&destination, b"existing video").unwrap();
    let options = NativeOptions::default();
    assert!(Encoder::new(&destination, (64, 64), 30, 1, &options).is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), b"existing video");
    assert_eq!(directory.path().read_dir().unwrap().count(), 1);
}

#[test]
#[cfg(feature = "ffmpeg")]
#[ignore = "requires FFmpeg with libx264"]
fn streaming_video_commits_only_finished_outputs_and_cleans_up_on_drop() -> Result {
    let directory = tempfile::tempdir()?;
    let destination = directory.path().join("out.mp4");
    let options = NativeOptions::default();
    let mut encoder = Encoder::new(&destination, (64, 64), 24, 3, &options)?;
    for color in [
        motion_core::Color::from_hex(0x2d66e5),
        motion_core::Color::from_hex(0x118677),
        motion_core::Color::WHITE,
    ] {
        encoder.write_buffer(&frame(64, 64, color))?;
    }
    assert!(!destination.exists());
    encoder.finish()?;
    assert!(std::fs::metadata(&destination)?.len() > 100);
    let metadata = motion_ffmpeg::inspect::media(&destination)?;
    assert!(metadata.h264 && !metadata.aac);
    assert_eq!((metadata.width, metadata.height), (64, 64));
    assert_eq!(metadata.video_fps, 24.0);
    assert_eq!(metadata.video_frames, 3);
    assert_eq!(metadata.channels, 0);
    assert_eq!(motion_ffmpeg::inspect::video_frames(&destination, 3)?, 3);
    let interrupted = directory.path().join("interrupted.mp4");
    let mut encoder = Encoder::new(&interrupted, (64, 64), 24, 3, &options)?;
    encoder.write_buffer(&frame(64, 64, motion_core::Color::from_hex(0x2d66e5)))?;
    drop(encoder);
    assert!(!interrupted.exists());
    assert_eq!(directory.path().read_dir()?.count(), 1);
    Ok(())
}

#[cfg(feature = "ffmpeg")]
fn audio_fixture(directory: &Path) -> Result<PathBuf> {
    let source = directory.join("short voice with spaces.wav");
    write_tone(&source, 440.0, 0.25)?;
    Ok(source)
}

#[cfg(feature = "ffmpeg")]
pub(crate) fn write_tone(path: &Path, frequency: f64, seconds: f64) -> io::Result<()> {
    use std::io::Write;
    let count = (32_000.0 * seconds) as u32;
    let length = count * 2;
    let mut file = std::fs::File::create(path)?;
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + length).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16_u32.to_le_bytes())?;
    file.write_all(&1_u16.to_le_bytes())?;
    file.write_all(&1_u16.to_le_bytes())?;
    file.write_all(&32_000_u32.to_le_bytes())?;
    file.write_all(&64_000_u32.to_le_bytes())?;
    file.write_all(&2_u16.to_le_bytes())?;
    file.write_all(&16_u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&length.to_le_bytes())?;
    for index in 0..count {
        let sample = (4095.0
            * (std::f64::consts::TAU * frequency * f64::from(index) / 32_000.0).sin())
        .round() as i16;
        file.write_all(&sample.to_le_bytes())?;
    }
    Ok(())
}

#[test]
#[cfg(feature = "ffmpeg")]
#[ignore = "requires native FFmpeg audio filters"]
fn audio_mixer_streaming_seek_matches_the_export_graph_and_bounds_reads() -> Result {
    let directory = tempfile::tempdir()?;
    let source = audio_fixture(directory.path())?;
    let sources = [source];
    let graph = "anullsrc=r=48000:cl=stereo,aformat=sample_fmts=fltp,atrim=end_sample=48000[base];\
        [1:a:0]aresample=48000,aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo,\
        atrim=start_sample=0:end_sample=24000,asetpts=N/SR/TB,apad=whole_len=24000,\
        atrim=end_sample=24000,volume=0.5,afade=t=in:ss=0:ns=2400,afade=t=out:ss=19200:ns=4800,\
        adelay=12000S:all=1[a0];[base][a0]amix=inputs=2:duration=first:dropout_transition=0:normalize=0,\
        atrim=end_sample=48000,asetpts=N/SR/TB[motion_audio]";
    let open = || {
        motion_ffmpeg::Mixer::new(
            motion_ffmpeg::AudioMix {
                sources: &sources,
                filter_graph: graph,
            },
            48_000,
        )
    };
    let mut mixer = open()?;
    assert_eq!(
        mixer.read(&mut []).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(
        mixer.read(&mut [[0.0; 2]; 4097]).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    let mut buffer = [[0.0; 2]; 137];
    let mut full = Vec::new();
    loop {
        let count = mixer.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        full.extend_from_slice(&buffer[..count]);
    }
    assert_eq!(full.len(), 48_000);
    assert!(full.iter().flatten().all(|sample| sample.is_finite()));
    assert!(
        full[..12_000]
            .iter()
            .flatten()
            .all(|sample| sample.abs() < 0.00001)
    );
    assert!(
        full[14_400..24_000]
            .iter()
            .flatten()
            .any(|sample| sample.abs() > 0.01)
    );
    let mut seeked = open()?;
    seeked.skip(13_003, || false)?;
    let count = seeked.read(&mut buffer)?;
    for (actual, expected) in buffer[..count].iter().zip(&full[13_003..]) {
        assert!((actual[0] - expected[0]).abs() < 0.000001);
        assert!((actual[1] - expected[1]).abs() < 0.000001);
    }
    let mut cancelled = open()?;
    assert_eq!(
        cancelled.skip(1, || true).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(
        cancelled.skip(48_001, || false).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    cancelled.skip(48_000, || false)?;
    assert_eq!(cancelled.read(&mut buffer)?, 0);
    Ok(())
}

#[test]
#[cfg(feature = "ffmpeg")]
#[ignore = "requires native FFmpeg with libx264/AAC"]
fn audio_encoder_preserves_frame_count_and_encodes_delayed_padded_stereo_aac() -> Result {
    let directory = tempfile::tempdir()?;
    let source = audio_fixture(directory.path())?;
    let clip = crate::audio::resolve_clip(
        &motion_core::AudioClip::file(source)
            .duration(motion_core::Duration::from_seconds(0.5))
            .start(motion_core::Time::from_seconds(0.25))
            .volume(0.5)
            .fade_in(motion_core::Duration::from_seconds(0.05))
            .fade_out(motion_core::Duration::from_seconds(0.1)),
    )?;
    let options = NativeOptions::default().audio_mix(crate::MixedAudio::new(
        crate::MixSource::clips(&[clip], motion_core::Duration::from_seconds(1.0)),
        crate::MixOptions::new(48_000),
    )?)?;
    let destination = directory.path().join("output.mp4");
    let mut encoder = Encoder::new(&destination, (64, 64), 24, 24, &options)?;
    for _ in 0..24 {
        encoder.write_buffer(&frame(64, 64, motion_core::Color::WHITE))?;
    }
    encoder.finish()?;
    let metadata = motion_ffmpeg::inspect::media(&destination)?;
    assert!(metadata.h264 && metadata.aac);
    assert_eq!((metadata.width, metadata.height), (64, 64));
    assert_eq!(metadata.video_fps, 24.0);
    assert!((metadata.audio_seconds - 1.0).abs() < 1.0 / 48_000.0);
    assert_eq!(metadata.video_frames, 24);
    assert_eq!(motion_ffmpeg::inspect::video_frames(&destination, 24)?, 24);
    assert_eq!((metadata.sample_rate, metadata.channels), (48_000, 2));
    let decoded = motion_ffmpeg::inspect::audio_samples(&destination, 49_024)?;
    assert!(decoded.iter().flatten().all(|sample| sample.is_finite()));
    let samples: Vec<_> = decoded.iter().map(|pair| pair[0]).collect();
    assert!((48000..=49024).contains(&samples.len()));
    assert!(samples.iter().all(|sample| sample.is_finite()));
    let rms = |start: usize, end: usize| {
        (samples[start..end]
            .iter()
            .map(|sample| f64::from(*sample).powi(2))
            .sum::<f64>()
            / (end - start) as f64)
            .sqrt()
    };
    assert!(rms(2400, 7200) < 0.00001);
    assert!(rms(15360, 20160) > 0.01);
    assert!(rms(40800, 45600) < 0.00001);
    Ok(())
}

#[test]
#[cfg(feature = "ffmpeg")]
#[ignore = "requires native FFmpeg with libx264/AAC"]
fn pcm_delivery_rates_and_container_metadata_round_trip() -> Result {
    use motion_core::{AudioClip, AudioSource, TimeLiteral};
    let directory = tempfile::tempdir()?;
    for rate in [44100, 48000, 96000] {
        let source =
            crate::audio::synth::Synth::new(crate::audio::synth::Wave::Sine, 440.0, rate, 0)?
                .gain_db(-12.0)?;
        let options = NativeOptions::default().audio_mix(crate::MixedAudio::new(
            crate::MixSource::clips(
                &[AudioClip::source(source).duration(1.seconds())],
                1.seconds(),
            ),
            crate::MixOptions::new(rate),
        )?)?;
        let mut options = options;
        options
            .metadata
            .tags
            .insert("title".into(), "PCM rate regression".into());
        options
            .metadata
            .tags
            .insert("language".into(), "eng".into());
        options
            .metadata
            .tags
            .insert("R128_TRACK_GAIN".into(), "0".into());
        options
            .metadata
            .chapters
            .push(motion_core::audio::AudioChapter {
                start: motion_core::Time::ZERO,
                end: 1.seconds().after_origin(),
                title: "Opening".into(),
            });
        let destination = directory.path().join(format!("pcm-{rate}.mp4"));
        let mut encoder = Encoder::new(&destination, (32, 32), 24, 24, &options)?;
        for _ in 0..24 {
            encoder.write_buffer(&frame(32, 32, motion_core::Color::WHITE))?;
        }
        encoder.finish()?;
        let media = motion_ffmpeg::inspect::media(&destination)?;
        assert_eq!((media.sample_rate, media.channels), (rate as i32, 2));
        assert!((media.audio_seconds - 1.0).abs() <= 1.0 / f64::from(rate));
        assert_eq!(media.video_frames, 24);
        let decoded = crate::PcmBuffer::open(&destination)?;
        assert_eq!(decoded.sample_rate(), rate);
        assert_eq!(decoded.channels(), 2);
        assert!(decoded.samples().iter().any(|sample| sample.abs() > 0.01));
        let metadata = motion_ffmpeg::inspect::metadata(&destination)?;
        assert!(
            metadata
                .tags
                .iter()
                .any(|(key, value)| key == "title" && value == "PCM rate regression")
        );
        assert!(
            metadata
                .tags
                .iter()
                .any(|(key, value)| key == "language" && value == "eng")
        );
        assert_eq!(metadata.chapters.len(), 1);
        assert_eq!(metadata.chapters[0].title, "Opening");
        assert_eq!(metadata.chapters[0].start, 0);
        assert_eq!(metadata.chapters[0].end, 1.seconds().ticks());
    }
    Ok(())
}

#[test]
#[cfg(feature = "ffmpeg")]
#[ignore = "requires FFmpeg with libx264/AAC"]
fn audio_cancellation_preserves_existing_destination() -> Result {
    let directory = tempfile::tempdir()?;
    let destination = directory.path().join("output.mp4");
    std::fs::write(&destination, b"existing")?;
    let clip = crate::audio::resolve_clip(
        &motion_core::AudioClip::file(audio_fixture(directory.path())?)
            .duration(motion_core::Duration::from_seconds(1.0)),
    )?;
    let options = NativeOptions::default()
        .overwrite(true)
        .audio_mix(crate::MixedAudio::new(
            crate::MixSource::clips(&[clip], motion_core::Duration::from_seconds(1.0)),
            crate::MixOptions::new(48_000),
        )?)?;
    let mut encoder = Encoder::new(&destination, (64, 64), 24, 24, &options)?;
    for _ in 0..24 {
        encoder.write_buffer(&frame(64, 64, motion_core::Color::WHITE))?;
    }
    assert!(!encoder.finish_with_check(|| true)?);
    assert_eq!(std::fs::read(&destination)?, b"existing");
    assert!(directory.path().read_dir()?.all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".motion-video-")
    }));
    Ok(())
}
