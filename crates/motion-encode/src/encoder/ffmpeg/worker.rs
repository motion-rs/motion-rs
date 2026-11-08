//! Bounded frame transfer to a thread-confined transactional native encoder.

use super::{
    Codec, ExportOptions, ExportReport, FrameBuffer, FrameRate, ProgressSink, Result, native,
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const QUEUE_CAPACITY: usize = 2;

const CANCEL_INTERVAL: Duration = Duration::from_millis(10);

enum Message {
    Frame(FrameBuffer),
    Finish,
}

enum Event {
    Ready,
    Finished(Result<ExportReport>),
}

pub(super) struct EncodingWorker {
    sender: Option<SyncSender<Message>>,
    events: Receiver<Event>,
    cancelled: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

fn failure(message: impl Into<String>) -> motion_core::Error {
    motion_core::Error::Encoder(Box::new(std::io::Error::other(message.into())))
}

impl EncodingWorker {
    pub(super) fn spawn(
        options: ExportOptions,
        rate: FrameRate,
        expected: u64,
        audio: Option<crate::MixedAudio>,
        progress: &dyn ProgressSink,
    ) -> Result<Self> {
        let mut worker = Self::launch(move |receiver, events, cancelled| {
            encode(options, rate, expected, audio, receiver, events, cancelled)
        })?;
        match worker.receive(progress)? {
            Event::Ready => Ok(worker),
            Event::Finished(result) => Err(result
                .err()
                .unwrap_or_else(|| failure("Encoder finished before opening"))),
        }
    }

    fn launch(
        task: impl FnOnce(Receiver<Message>, &SyncSender<Event>, &AtomicBool) -> Result<ExportReport>
        + Send
        + 'static,
    ) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        let (events, replies) = mpsc::sync_channel(2);
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let thread = std::thread::Builder::new()
            .name("motion-encoder".into())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| task(receiver, &events, &flag)))
                    .unwrap_or_else(|payload| {
                        let message = payload
                            .downcast_ref::<String>()
                            .map(String::as_str)
                            .or_else(|| payload.downcast_ref::<&str>().copied())
                            .unwrap_or("non-string panic");
                        Err(failure(format!("Encoding worker panicked: {message}")))
                    });
                let _ = events.send(Event::Finished(result));
            })?;
        Ok(Self {
            sender: Some(sender),
            events: replies,
            cancelled,
            thread: Some(thread),
        })
    }

    fn check_cancelled(&self, progress: &dyn ProgressSink) -> Result<()> {
        if progress.is_cancelled() {
            self.cancelled.store(true, Ordering::Release);
            Err(motion_core::Error::Cancelled)
        } else {
            Ok(())
        }
    }

    fn receive(&mut self, progress: &dyn ProgressSink) -> Result<Event> {
        loop {
            self.check_cancelled(progress)?;
            match self.events.recv_timeout(CANCEL_INTERVAL) {
                Ok(event) => return Ok(event),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(failure("Encoding worker disconnected"));
                }
            }
        }
    }

    fn send(&mut self, mut message: Message, progress: &dyn ProgressSink) -> Result<()> {
        loop {
            self.check_cancelled(progress)?;
            match self.events.try_recv() {
                Ok(Event::Finished(result)) => {
                    return Err(result.err().unwrap_or_else(|| {
                        failure("Encoder finished before frame submission completed")
                    }));
                }
                Ok(Event::Ready) | Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(failure("Encoding worker disconnected"));
                }
            }
            match self
                .sender
                .as_ref()
                .ok_or_else(|| failure("Encoder input is closed"))?
                .try_send(message)
            {
                Ok(()) => return Ok(()),
                Err(TrySendError::Full(returned)) => {
                    message = returned;
                    match self.events.recv_timeout(CANCEL_INTERVAL) {
                        Ok(Event::Finished(result)) => {
                            return Err(result.err().unwrap_or_else(|| {
                                failure("Encoder finished before submission completed")
                            }));
                        }
                        Ok(Event::Ready) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            return Err(failure("Encoding worker disconnected"));
                        }
                    }
                }
                Err(TrySendError::Disconnected(_)) => {
                    return match self.receive(progress)? {
                        Event::Finished(result) => Err(result
                            .err()
                            .unwrap_or_else(|| failure("Encoder input disconnected"))),
                        Event::Ready => Err(failure("Encoder input disconnected after opening")),
                    };
                }
            }
        }
    }

    pub(super) fn write(&mut self, frame: &FrameBuffer, progress: &dyn ProgressSink) -> Result<()> {
        self.send(Message::Frame(frame.clone()), progress)
    }

    pub(super) fn finish(mut self, progress: &dyn ProgressSink) -> Result<ExportReport> {
        self.send(Message::Finish, progress)?;
        self.sender.take();
        match self.receive(progress)? {
            Event::Finished(result) => {
                if let Some(thread) = self.thread.take() {
                    thread
                        .join()
                        .map_err(|_| failure("Encoding worker join failed"))?;
                }
                result
            }
            Event::Ready => Err(failure("Unexpected encoder readiness during finalization")),
        }
    }
}

impl Drop for EncodingWorker {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn encode(
    options: ExportOptions,
    rate: FrameRate,
    expected: u64,
    audio: Option<crate::MixedAudio>,
    receiver: Receiver<Message>,
    events: &SyncSender<Event>,
    cancelled: &AtomicBool,
) -> Result<ExportReport> {
    let started = Instant::now();
    let quality = match options.codec {
        Codec::H264 { quality, .. } | Codec::H265 { quality } | Codec::Vp9 { quality, .. } => {
            quality
        }
        Codec::ProRes4444 => 18,
        _ => return Err(failure("Unsupported native codec")),
    };
    let range = options
        .range
        .clone()
        .ok_or_else(|| failure("Encoder requires normalized range"))?;
    let size = options
        .render
        .size
        .ok_or_else(|| failure("Encoder requires normalized size"))?;
    let mut settings = crate::NativeOptions::default()
        .codec(options.codec)
        .encoding(&options)
        .crf(quality)
        .overwrite(options.overwrite);
    if let Some(audio) = audio {
        settings = settings.audio_mix(audio)?;
    }
    if cancelled.load(Ordering::Acquire) {
        return Err(motion_core::Error::Cancelled);
    }
    let mut encoder = native::Encoder::new_with_frame_rate(
        &options.path,
        (size.width as usize, size.height as usize),
        rate,
        expected as u32,
        &settings,
    )?;
    events
        .send(Event::Ready)
        .map_err(|_| motion_core::Error::Cancelled)?;
    let mut written = 0;
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(motion_core::Error::Cancelled);
        }
        match receiver.recv().map_err(|_| motion_core::Error::Cancelled)? {
            Message::Frame(frame) => {
                if written >= expected {
                    return Err(failure("Too many native video frames"));
                }
                encoder.write_buffer(&frame).map_err(|error| {
                    error.at_frame((range.start.to_frame(rate) as u64) + written)
                })?;
                written += 1;
            }
            Message::Finish => {
                if written != expected {
                    return Err(failure("Incomplete native video frame count"));
                }
                if !encoder.finish_with_check(|| cancelled.load(Ordering::Acquire))? {
                    return Err(motion_core::Error::Cancelled);
                }
                return Ok(ExportReport {
                    fonts: Vec::new(),
                    path: options.path.clone(),
                    frames: written,
                    range,
                    elapsed_seconds: started.elapsed().as_secs_f64(),
                    bytes: std::fs::metadata(&options.path)?.len(),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_is_bounded_and_worker_panics_are_recoverable() -> Result<()> {
        let (release, gate) = mpsc::sync_channel(1);
        let mut worker = EncodingWorker::launch(move |_receiver, events, _| {
            events.send(Event::Ready).unwrap();
            gate.recv().unwrap();
            Err(motion_core::Error::Cancelled)
        })?;
        assert!(matches!(worker.receive(&crate::NoProgress)?, Event::Ready));
        let sender = worker.sender.as_ref().unwrap();
        for _ in 0..QUEUE_CAPACITY {
            assert!(sender.try_send(Message::Finish).is_ok());
        }
        assert!(matches!(
            sender.try_send(Message::Finish),
            Err(TrySendError::Full(_))
        ));
        release.send(()).unwrap();
        drop(worker);
        let mut panicking = EncodingWorker::launch(|_, _, _| panic!("expected encoding panic"))?;
        match panicking.receive(&crate::NoProgress)? {
            Event::Finished(Err(error)) => {
                assert!(error.to_string().contains("expected encoding panic"))
            }
            _ => panic!("worker panic must be reported"),
        }
        Ok(())
    }

    #[test]
    fn dropping_before_readiness_is_consumed_joins_the_worker() -> Result<()> {
        let (opened, ready) = mpsc::sync_channel(1);
        let worker = EncodingWorker::launch(move |_receiver, events, _| {
            events.send(Event::Ready).unwrap();
            opened.send(()).unwrap();
            Err(motion_core::Error::Cancelled)
        })?;
        ready.recv().unwrap();
        drop(worker);
        Ok(())
    }
}
