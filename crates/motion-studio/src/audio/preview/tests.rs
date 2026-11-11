use super::*;
use std::time::Duration;

#[test]
fn preview_uses_fractional_frame_sample_boundaries_and_same_mix() -> motion_core::Result {
    use motion_core::{AudioSource, TimeLiteral};
    let rate = motion_core::FrameRate::NTSC;
    let pcm = motion_encode::PcmBuffer::new(
        48_000,
        1,
        (0..96_096)
            .map(|index| (index % 64) as f32 / 256.0)
            .collect(),
    )?;
    let timeline = Arc::new(
        motion_core::Timeline::try_new(motion_core::Size::new(16.0, 8.0), rate)?
            .try_audio(motion_core::AudioClip::source(pcm))?
            .try_duration(motion_core::Duration::from_frame(60, rate))?
            .build()?,
    );
    let request = timeline_request(timeline.clone(), Vec::new(), 30)?;
    assert_eq!(request.start, 48_048);
    assert_eq!(request.total, 96_096);
    assert_eq!(frame_at_sample(request.start, rate), 30);
    assert_eq!(frame_at_sample(request.start - 1, rate), 29);
    let mut prepared = PreviewMix::prepare(&request)?;
    let mut actual = [[0.0; 2]; 64];
    assert_eq!(prepared.read(&mut actual)?, 64);
    let expected =
        motion_encode::MixedAudio::new(&timeline, motion_encode::MixOptions::new(48_000))?;
    let mut reference = [[0.0; 2]; 64];
    expected.read(request.start, reference.as_flattened_mut());
    assert_eq!(actual, reference);
    assert!(actual.iter().flatten().any(|sample| *sample != 0.0));
    assert!(timeline_request(timeline, Vec::new(), 60).is_err());
    let silent = Arc::new(
        motion_core::Timeline::try_new(motion_core::Size::new(16.0, 8.0), rate)?
            .try_duration(1.seconds())?
            .build()?,
    );
    let request = timeline_request(silent, Vec::new(), 0)?;
    let prepared = PreviewMix::prepare(&request)?;
    assert!(prepared.timeline.is_none());
    Ok(())
}

#[test]
fn cancelled_request_does_not_read_audio_or_open_device() -> motion_core::Result {
    use motion_core::TimeLiteral;

    struct Source(Arc<AtomicU64>);

    impl motion_core::AudioSource for Source {
        fn sample_rate(&self) -> u32 {
            48_000
        }

        fn channels(&self) -> u16 {
            1
        }

        fn duration(&self) -> Option<motion_core::Duration> {
            Some(1.seconds())
        }

        fn read(&self, _: u64, output: &mut [f32]) -> usize {
            self.0.fetch_add(1, Ordering::Relaxed);
            output.fill(0.0);
            output.len()
        }
    }
    let reads = Arc::new(AtomicU64::new(0));
    let timeline = Arc::new(
        motion_core::Timeline::try_new(motion_core::Size::new(16.0, 8.0), motion_core::fps!(30))?
            .try_audio(motion_core::AudioClip::source(Source(reads.clone())))?
            .build()?,
    );
    let request = timeline_request(timeline, Vec::new(), 0)?;
    request.shared.cancelled.store(true, Ordering::Relaxed);
    let prepared = PreviewMix::prepare(&request)?;
    assert!(prepared.native.is_none());
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn replacing_playback_cancels_the_old_session_and_preserves_seek_origin() -> motion_core::Result {
    use motion_core::TimeLiteral;
    let timeline = Arc::new(
        motion_core::Timeline::try_new(
            motion_core::Size::new(16.0, 8.0),
            motion_core::FrameRate::NTSC,
        )?
        .try_duration(2.seconds())?
        .build()?,
    );
    let mut preview = AudioPreview {
        inbox: Inbox::default(),
        session: None,
        worker: None,
    };
    preview.start_timeline(&AudioForm::default(), timeline.clone(), 0)?;
    let old = preview.session.as_ref().unwrap().shared.clone();
    preview.start_timeline(&AudioForm::default(), timeline, 30)?;
    assert!(old.cancelled.load(Ordering::Relaxed));
    assert_eq!(preview.session.as_ref().unwrap().start, 48_048);
    assert!(matches!(preview.status(), Some(AudioStatus::Preparing)));
    let active = preview.session.as_ref().unwrap().shared.clone();
    preview.stop();
    assert!(active.cancelled.load(Ordering::Relaxed));
    assert!(preview.status().is_none());
    assert!(preview.inbox.0.lock().unwrap().pending.is_none());
    Ok(())
}

#[test]
fn silent_timeline_finishes_preparation_without_an_output_device() -> motion_core::Result {
    use motion_core::TimeLiteral;
    let context = egui::Context::default();
    let (sender, receiver) = std::sync::mpsc::channel();
    context.set_request_repaint_callback(move |_| {
        let _ = sender.send(());
    });
    let timeline = Arc::new(
        motion_core::Timeline::try_new(motion_core::Size::new(16.0, 8.0), motion_core::fps!(30))?
            .try_duration(1.seconds())?
            .build()?,
    );
    let mut preview = AudioPreview::new(context)?;
    preview.start_timeline(&AudioForm::default(), timeline, 0)?;
    receiver
        .recv_timeout(Duration::from_secs(5))
        .map_err(io::Error::other)?;
    assert!(preview.status().is_none());
    assert!(
        preview
            .session
            .as_ref()
            .unwrap()
            .shared
            .silent
            .load(Ordering::Acquire)
    );
    Ok(())
}

#[test]
fn timeline_and_file_preview_sources_are_mixed_in_order() -> motion_core::Result {
    use motion_core::TimeLiteral;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("preview.wav");
    let pcm = motion_encode::PcmBuffer::new(48_000, 1, vec![0.1; 4800])?;
    let mix = motion_encode::MixedAudio::new(
        motion_encode::MixSource::clips(
            &[motion_core::AudioClip::source(pcm.clone())],
            100.milliseconds(),
        ),
        motion_encode::MixOptions::new(48_000),
    )?;
    mix.write_wav(&mut std::fs::File::create(&path)?, 4800)?;
    let clip = motion_core::AudioClip::file(path).duration(100.milliseconds());
    let timeline = Arc::new(
        motion_core::Timeline::try_new(
            motion_core::Size::new(16.0, 8.0),
            motion_core::FrameRate::NTSC,
        )?
        .try_audio(motion_core::AudioClip::source(pcm))?
        .try_duration(motion_core::Duration::from_frame(
            3,
            motion_core::FrameRate::NTSC,
        ))?
        .build()?,
    );
    let mut request = timeline_request(timeline, vec![clip], 0)?;
    let mut combined = PreviewMix::prepare(&request)?;
    let mut actual = [[0.0; 2]; 64];
    combined.read(&mut actual)?;
    let mut native = [[0.0; 2]; 64];
    let mut file_samples = [[0.0; 2]; 64];
    let clips = std::mem::take(&mut request.clips);
    let mut source = PreviewMix::prepare(&request)?;
    source.read(&mut native)?;
    request.clips = clips;
    request.native = None;
    let mut source = PreviewMix::prepare(&request)?;
    source.read(&mut file_samples)?;
    for ((actual, native), file_samples) in actual.iter().zip(native).zip(file_samples) {
        for channel in 0..2 {
            assert!((actual[channel] - (native[channel] + file_samples[channel])).abs() < 1e-6);
            assert!(actual[channel] > native[channel]);
        }
    }
    Ok(())
}

#[test]
fn file_preview_windows_preserve_offsets_fades_and_export_crop() -> motion_core::Result {
    use motion_core::{AudioSource, TimeLiteral};
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("window.wav");
    let pcm = motion_encode::PcmBuffer::new(
        48_000,
        2,
        (0..96_000)
            .map(|index| (index % 128) as f32 / 512.0)
            .collect(),
    )?;
    motion_encode::MixedAudio::new(
        motion_encode::MixSource::clips(&[motion_core::AudioClip::source(pcm)], 1.seconds()),
        motion_encode::MixOptions::new(48_000),
    )?
    .write_wav(&mut std::fs::File::create(&path)?, 48_000)?;
    let clip = motion_core::AudioClip::file(path)
        .offset(50.milliseconds())
        .start(20.milliseconds().after_origin())
        .duration(900.milliseconds())
        .volume(0.5)
        .fade_in(400.milliseconds())
        .fade_out(400.milliseconds());
    let timeline = Arc::new(
        motion_core::Timeline::new(motion_core::Size::new(1.0, 1.0), motion_core::fps!(100))
            .audio(clip)
            .duration(1.seconds())
            .build()?,
    );
    let request = timeline_request(timeline.clone(), vec![], 10)?;
    let mut preview = PreviewMix::prepare(&request)?;
    let expected = motion_encode::MixedAudio::new(
        &timeline,
        motion_encode::MixOptions::new(48_000).range(100.milliseconds()..1.seconds()),
    )?;
    let mut position = 0;
    while preview.cursor < preview.total {
        let mut actual = [[0.0; 2]; BLOCK];
        let count = preview.read(&mut actual)?;
        let mut reference = [[0.0; 2]; BLOCK];
        assert_eq!(
            expected.read(position, reference[..count].as_flattened_mut()),
            count
        );
        for (actual, reference) in actual[..count]
            .iter()
            .flatten()
            .zip(reference[..count].iter().flatten())
        {
            assert!((actual - reference).abs() < 1e-6);
        }
        assert!(preview.window_end - preview.window_start <= CAPACITY as u64);
        position += count as u64;
    }
    assert_eq!(position, 43_200);
    Ok(())
}

fn make_output(length: u64, count: usize) -> Output {
    let (mut producer, consumer) = rtrb::RingBuffer::new(CAPACITY);
    for _ in 0..count {
        producer.push([0.25, -0.5]).unwrap();
    }
    let shared = Arc::new(Shared::default());
    shared.ready.store(true, Ordering::Release);
    Output {
        consumer,
        shared,
        length,
        cursor: 0,
        submitted: 0,
    }
}

#[test]
fn audio_clock_counts_audible_samples_and_drains_final_latency() {
    let mut output = make_output(960, 960);
    let mut buffer = [0.0; 960];
    let delay = Some(Duration::from_millis(10));
    output.write(&mut buffer, delay);
    assert_eq!(&buffer[..2], &[0.25, -0.5]);
    assert_eq!(output.shared.played.load(Ordering::Relaxed), 0);
    output.write(&mut buffer, delay);
    assert_eq!(output.shared.played.load(Ordering::Relaxed), 0);
    output.write(&mut buffer, delay);
    assert_eq!(output.shared.played.load(Ordering::Relaxed), 480);
    assert!(buffer.iter().all(|sample| *sample == 0.0));
    output.write(&mut buffer, delay);
    assert_eq!(output.shared.played.load(Ordering::Relaxed), 960);
    assert_eq!(output.shared.fault.load(Ordering::Acquire), 0);
}

#[test]
fn cancelled_sessions_are_silent_and_do_not_advance() {
    let mut output = make_output(960, 960);
    output.shared.cancelled.store(true, Ordering::Relaxed);
    let mut buffer = [1.0; 960];
    output.write(&mut buffer, Some(Duration::ZERO));
    assert!(buffer.iter().all(|sample| *sample == 0.0));
    assert_eq!(output.submitted, 0);
    assert_eq!(output.shared.played.load(Ordering::Relaxed), 0);
}

#[test]
fn underruns_and_missing_timestamps_fail_instead_of_running_a_silent_clock() {
    let mut output = make_output(960, 0);
    let mut buffer = [1.0; 960];
    output.write(&mut buffer, Some(Duration::ZERO));
    assert_eq!(output.shared.fault.load(Ordering::Acquire), 1);
    assert!(buffer.iter().all(|sample| *sample == 0.0));
    let mut output = make_output(960, 960);
    output.write(&mut buffer, None);
    assert_eq!(output.shared.fault.load(Ordering::Acquire), 3);
    assert!(buffer.iter().all(|sample| *sample == 0.0));
}

#[test]
fn callbacks_before_preroll_are_silent_without_consuming_or_failing() {
    let mut output = make_output(960, 960);
    output.shared.ready.store(false, Ordering::Release);
    let mut buffer = [1.0; 960];
    output.write(&mut buffer, None);
    assert!(buffer.iter().all(|sample| *sample == 0.0));
    assert_eq!(output.submitted, 0);
    assert_eq!(output.cursor, 0);
    assert_eq!(output.shared.fault.load(Ordering::Acquire), 0);
}

#[test]
fn stop_invalidates_the_session_and_clears_the_pending_request() -> io::Result<()> {
    let shared = Arc::new(Shared::default());
    let inbox = Inbox::default();
    inbox.0.lock().unwrap().pending = Some(Request {
        clips: Vec::new(),
        native: None,
        start: 0,
        total: 960,
        shared: shared.clone(),
    });
    let mut preview = AudioPreview {
        inbox,
        session: Some(Session {
            shared: shared.clone(),
            start: 0,
            length: 960,
        }),
        worker: None,
    };
    preview.stop();
    assert!(shared.cancelled.load(Ordering::Relaxed));
    assert!(preview.status().is_none());
    assert!(preview.inbox.0.lock().unwrap().pending.is_none());
    Ok(())
}

#[test]
fn changing_device_latency_cannot_move_the_audio_clock_backwards() {
    let mut output = make_output(8192, 8192);
    let mut buffer = [0.0; 960];
    output.write(&mut buffer, Some(Duration::ZERO));
    output.write(&mut buffer, Some(Duration::ZERO));
    assert_eq!(output.shared.played.load(Ordering::Relaxed), 480);
    output.write(&mut buffer, Some(Duration::from_millis(20)));
    assert_eq!(output.shared.played.load(Ordering::Relaxed), 480);
    output.write(&mut buffer, Some(Duration::ZERO));
    assert_eq!(output.shared.played.load(Ordering::Relaxed), 1440);
}
