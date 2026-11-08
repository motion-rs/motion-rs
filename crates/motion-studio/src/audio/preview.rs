//! Real-time audio preview: a worker thread mixes the scene's audio and plays it through cpal,
//! keeping the playhead in sync with the studio timeline.

use std::io;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use eframe::egui;

use crate::AudioOptions;
use crate::audio::settings::AudioForm;
use motion_core::AudioClip;
use motion_core::Scene;
use motion_encode::audio;

/// Stereo sample frames mixed per producer step.
const BLOCK: usize = 512;

/// Stereo sample frames held by the playback ring buffer and by each unprocessed mix window.
const CAPACITY: usize = 8192;

#[derive(Default)]
struct Shared {
    cancelled: AtomicBool,
    silent: AtomicBool,
    ready: AtomicBool,
    fault: AtomicU8,
    played: AtomicU64,
    error: Mutex<Option<String>>,
}

struct Request {
    clips: Vec<AudioClip>,
    native: Option<Arc<motion_core::Timeline>>,
    start: u64,
    total: u64,
    shared: Arc<Shared>,
}

#[derive(Default)]
struct PreviewRequestQueue {
    pending: Option<Request>,
    closing: bool,
}

type Inbox = Arc<(Mutex<PreviewRequestQueue>, Condvar)>;

struct Session {
    shared: Arc<Shared>,
    start: u64,
    length: u64,
}

pub(crate) enum AudioStatus {
    Preparing,
    Playing(u64),
    Finished,
    Failed(String),
}

pub(crate) struct AudioPreview {
    inbox: Inbox,
    session: Option<Session>,
    worker: Option<JoinHandle<()>>,
}

impl AudioPreview {
    pub(crate) fn new(context: egui::Context) -> io::Result<Self> {
        let inbox = Inbox::default();
        let queue = inbox.clone();
        let worker = thread::Builder::new()
            .name("motion-audio-preview".into())
            .spawn(move || worker(queue, context))?;
        Ok(Self {
            inbox,
            session: None,
            worker: Some(worker),
        })
    }

    pub(crate) fn start_at_rate(
        &mut self,
        settings: &AudioForm,
        rate: motion_core::FrameRate,
        frames: u32,
        position: u32,
    ) -> io::Result<()> {
        self.stop();
        if position >= frames {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Audio preview frame is out of range",
            ));
        }
        let options = settings.apply(AudioOptions::default())?;
        if options.audio_clips().is_empty() {
            return Ok(());
        }
        let total = sample_at_frame(frames, rate);
        let start = sample_at_frame(position, rate);
        let shared = Arc::new(Shared::default());
        let request = Request {
            clips: options.audio_clips().to_vec(),
            native: None,
            start,
            total,
            shared: shared.clone(),
        };
        self.session = Some(Session {
            shared,
            start,
            length: total - start,
        });
        let mut mailbox = self
            .inbox
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        mailbox.pending = Some(request);
        self.inbox.1.notify_one();
        Ok(())
    }

    pub(crate) fn start_timeline(
        &mut self,
        settings: &AudioForm,
        timeline: Arc<motion_core::Timeline>,
        position: u32,
    ) -> io::Result<()> {
        self.stop();
        let options = settings.apply(AudioOptions::default())?;
        let request = timeline_request(timeline, options.audio_clips().to_vec(), position)?;
        self.session = Some(Session {
            shared: request.shared.clone(),
            start: request.start,
            length: request.total - request.start,
        });
        self.inbox
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pending = Some(request);
        self.inbox.1.notify_one();
        Ok(())
    }

    pub(crate) fn stop(&mut self) {
        if let Some(session) = self.session.take() {
            session.shared.cancelled.store(true, Ordering::Relaxed);
        }
        let mut mailbox = self
            .inbox
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        mailbox.pending = None;
        self.inbox.1.notify_one();
    }

    pub(crate) fn status(&self) -> Option<AudioStatus> {
        let session = self.session.as_ref()?;
        let shared = &session.shared;
        if shared.silent.load(Ordering::Acquire) {
            return None;
        }
        let fault = shared.fault.load(Ordering::Acquire);
        Some(if fault != 0 {
            AudioStatus::Failed(
                shared
                    .error
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone()
                    .unwrap_or_else(|| match fault {
                        1 => "Audio preview buffer underrun".into(),
                        2 => "Audio output device failed or disconnected".into(),
                        _ => "Audio output has no usable playback timestamp".into(),
                    }),
            )
        } else if !shared.ready.load(Ordering::Acquire) {
            AudioStatus::Preparing
        } else {
            let played = shared.played.load(Ordering::Relaxed);
            if played >= session.length {
                AudioStatus::Finished
            } else {
                AudioStatus::Playing(session.start + played)
            }
        })
    }
}

impl Drop for AudioPreview {
    fn drop(&mut self) {
        self.stop();
        self.inbox
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .closing = true;
        self.inbox.1.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn worker(inbox: Inbox, context: egui::Context) {
    loop {
        let request = {
            let mut mailbox = inbox.0.lock().unwrap_or_else(|error| error.into_inner());
            while mailbox.pending.is_none() && !mailbox.closing {
                mailbox = inbox
                    .1
                    .wait(mailbox)
                    .unwrap_or_else(|error| error.into_inner());
            }
            if mailbox.closing {
                return;
            }
            mailbox.pending.take().expect("pending audio request")
        };
        let shared = request.shared.clone();
        if shared.cancelled.load(Ordering::Relaxed) {
            continue;
        }
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| play(request, &inbox)))
                .unwrap_or_else(|_| Err(io::Error::other("Audio preview worker panicked")));
        if let Err(error) = result
            && !shared.cancelled.load(Ordering::Relaxed)
        {
            *shared
                .error
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(error.to_string());
            shared.fault.store(4, Ordering::Release);
            context.request_repaint();
        }
        context.request_repaint();
    }
}

fn play(request: Request, inbox: &Inbox) -> io::Result<()> {
    let mut mixer = PreviewMix::prepare(&request)?;
    if request.shared.cancelled.load(Ordering::Relaxed) {
        return Ok(());
    }
    if request.total == request.start || mixer.timeline.is_none() {
        request.shared.silent.store(true, Ordering::Release);
        return Ok(());
    }
    let device = cpal::default_host()
        .default_output_device()
        .ok_or_else(|| io::Error::other("No audio output device"))?;
    let config = device
        .supported_output_configs()
        .map_err(io::Error::other)?
        .find(|config| {
            config.channels() == 2
                && config.sample_format() == cpal::SampleFormat::F32
                && config.min_sample_rate().0 <= audio::SAMPLE_RATE as u32
                && config.max_sample_rate().0 >= audio::SAMPLE_RATE as u32
        })
        .ok_or_else(|| io::Error::other("Audio preview requires 48 kHz stereo float output"))?
        .with_sample_rate(cpal::SampleRate(audio::SAMPLE_RATE as u32))
        .config();
    let (mut producer, consumer) = rtrb::RingBuffer::new(CAPACITY);
    let length = request.total - request.start;
    let shared = request.shared;
    let mut output = Output {
        consumer,
        shared: shared.clone(),
        length,
        cursor: 0,
        submitted: 0,
    };
    let failed = shared.clone();
    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                let timestamp = info.timestamp();
                output.write(data, timestamp.playback.duration_since(&timestamp.callback));
            },
            move |_| {
                failed.fault.store(2, Ordering::Release);
            },
            None,
        )
        .map_err(io::Error::other)?;
    let mut buffer = [[0.0; 2]; BLOCK];
    let mut produced = 0;
    let mut started = false;
    while !shared.cancelled.load(Ordering::Relaxed) && shared.fault.load(Ordering::Acquire) == 0 {
        while producer.slots() >= BLOCK && produced < length {
            if shared.cancelled.load(Ordering::Relaxed) {
                return Ok(());
            }
            let count =
                mixer.read(&mut buffer[..(length - produced).min(BLOCK as u64) as usize])?;
            if count == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Audio preview ended early",
                ));
            }
            for sample in &buffer[..count] {
                producer
                    .push(*sample)
                    .map_err(|_| io::Error::other("Audio preview queue overflow"))?;
            }
            produced += count as u64;
        }
        if !started {
            shared.ready.store(true, Ordering::Release);
            stream.play().map_err(io::Error::other)?;
            started = true;
        }
        let mailbox = inbox.0.lock().unwrap_or_else(|error| error.into_inner());
        if shared.cancelled.load(Ordering::Relaxed) {
            return Ok(());
        }
        let _guard = inbox
            .1
            .wait_timeout(mailbox, Duration::from_millis(4))
            .unwrap_or_else(|error| error.into_inner());
    }
    Ok(())
}

fn sample_at_frame(frame: u32, rate: motion_core::FrameRate) -> u64 {
    let time = motion_core::Time::from_frame(i64::from(frame), rate);
    (time.ticks() as u128 * u128::from(audio::SAMPLE_RATE))
        .div_ceil(motion_core::TICKS_PER_SECOND as u128) as u64
}

fn timeline_request(
    timeline: Arc<motion_core::Timeline>,
    clips: Vec<AudioClip>,
    position: u32,
) -> io::Result<Request> {
    let frames = u32::try_from(timeline.frame_count()).map_err(io::Error::other)?;
    if position >= frames {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Audio preview frame is out of range",
        ));
    }
    let rate = timeline.frame_rate();
    let total = (timeline.total_duration().ticks() as u128 * u128::from(audio::SAMPLE_RATE))
        .div_ceil(motion_core::TICKS_PER_SECOND as u128) as u64;
    Ok(Request {
        clips,
        native: Some(timeline),
        start: sample_at_frame(position, rate),
        total,
        shared: Arc::new(Shared::default()),
    })
}

pub(crate) fn frame_at_sample(samples: u64, rate: motion_core::FrameRate) -> u32 {
    (u128::from(samples) * u128::from(rate.numerator())
        / (u128::from(audio::SAMPLE_RATE) * u128::from(rate.denominator())))
    .min(u128::from(u32::MAX)) as u32
}

struct PreviewMix {
    timeline: Option<motion_core::Timeline>,
    native: Option<motion_encode::MixedAudio>,
    window_start: u64,
    window_end: u64,
    cursor: u64,
    total: u64,
}

impl PreviewMix {
    fn prepare(request: &Request) -> io::Result<Self> {
        if request.shared.cancelled.load(Ordering::Relaxed) {
            return Ok(Self {
                timeline: None,
                native: None,
                window_start: request.start,
                window_end: request.start,
                cursor: request.start,
                total: request.total,
            });
        }
        let mut timeline = request.native.as_deref().cloned().unwrap_or_else(|| {
            motion_core::Timeline::new(
                motion_core::Size::new(1.0, 1.0),
                motion_core::FrameRate::NTSC,
            )
        });
        for clip in &request.clips {
            timeline = timeline.try_audio(clip.clone()).map_err(io::Error::other)?;
        }
        let timeline = if timeline.audio_clips().map_err(io::Error::other)?.is_empty() {
            None
        } else {
            Some(timeline)
        };
        let processed = timeline.as_ref().is_some_and(|timeline| {
            timeline.audio_master() != motion_core::Master::default()
                || !timeline.audio_buses().is_empty()
                || timeline.audio_clips().is_ok_and(|clips| {
                    clips
                        .iter()
                        .any(|clip| !clip.parameters().effects.is_empty())
                })
        });
        let prepared = if let Some(timeline) = timeline.as_ref().filter(|_| processed) {
            Some(
                motion_encode::MixedAudio::from_timeline(timeline, audio::SAMPLE_RATE as u32)
                    .map_err(io::Error::other)?,
            )
        } else {
            None
        };
        Ok(Self {
            timeline,
            native: prepared,
            window_start: if processed { 0 } else { request.start },
            window_end: if processed {
                request.total
            } else {
                request.start
            },
            cursor: request.start,
            total: request.total,
        })
    }

    fn read(&mut self, output: &mut [[f32; 2]]) -> io::Result<usize> {
        use motion_core::AudioSource;
        let count = (self.total.saturating_sub(self.cursor) as usize).min(output.len());
        output.fill([0.0; 2]);
        if let Some(timeline) = &self.timeline {
            let mut written = 0;
            while written < count {
                let position = self.cursor + written as u64;
                if position >= self.window_end {
                    self.window_start = position;
                    self.window_end = (position + CAPACITY as u64).min(self.total);
                    let time = |sample: u64| {
                        motion_core::Time::from_ticks(
                            (sample as i64 * motion_core::TICKS_PER_SECOND)
                                / audio::SAMPLE_RATE as i64,
                        )
                    };
                    self.native = Some(
                        motion_encode::MixedAudio::from_window(
                            timeline,
                            time(position)..time(self.window_end),
                            audio::SAMPLE_RATE as u32,
                        )
                        .map_err(io::Error::other)?,
                    );
                }
                let length = (count - written).min((self.window_end - position) as usize);
                let native = self.native.as_ref().expect("prepared audio window");
                let read = native.read(
                    position - self.window_start,
                    output[written..written + length].as_flattened_mut(),
                );
                if read != length {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "audio preview ended early",
                    ));
                }
                written += length;
            }
        }
        if output[..count]
            .iter()
            .flatten()
            .any(|sample| !sample.is_finite())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Audio preview mix contains non-finite samples",
            ));
        }
        self.cursor += count as u64;
        Ok(count)
    }
}

struct Output {
    consumer: rtrb::Consumer<[f32; 2]>,
    shared: Arc<Shared>,
    length: u64,
    cursor: u64,
    submitted: u64,
}

impl Output {
    fn write(&mut self, data: &mut [f32], latency: Option<Duration>) {
        data.fill(0.0);
        if self.shared.cancelled.load(Ordering::Relaxed)
            || !self.shared.ready.load(Ordering::Acquire)
            || self.shared.fault.load(Ordering::Acquire) != 0
        {
            return;
        }
        let Some(latency) = latency else {
            self.shared.fault.store(3, Ordering::Release);
            return;
        };
        let delay = ((latency.as_nanos() * u128::from(audio::SAMPLE_RATE)) / 1_000_000_000)
            .min(u128::from(u64::MAX)) as u64;
        self.shared.played.fetch_max(
            self.cursor.saturating_sub(delay).min(self.length),
            Ordering::Relaxed,
        );
        let count = (self.length - self.submitted).min((data.len() / 2) as u64) as usize;
        for frame in data.as_chunks_mut::<2>().0.iter_mut().take(count) {
            let Ok(sample) = self.consumer.pop() else {
                self.shared.fault.store(1, Ordering::Release);
                return;
            };
            frame[0] = sample[0].clamp(-1.0, 1.0);
            frame[1] = sample[1].clamp(-1.0, 1.0);
            self.submitted += 1;
        }
        self.cursor = self.cursor.saturating_add((data.len() / 2) as u64);
    }
}

#[cfg(test)]
mod tests;
