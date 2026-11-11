//! Preview worker threads that render queued frames and publish results to the coordinator.

use super::*;
use motion_core::sync::{CondvarExt, MutexExt};

pub(crate) struct PreviewWorker {
    shared: Arc<(Mutex<PreviewRequestQueue>, Condvar)>,
}

impl PreviewWorker {
    pub(super) fn spawn(
        source: StudioScene,
        context: Context,
        calibration: backend::Calibration,
        #[cfg(feature = "gpu")] textures: textures::TextureRing,
        notify_completed: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> io::Result<Self> {
        let shared = Arc::new((Mutex::new(PreviewRequestQueue::default()), Condvar::new()));
        let worker = shared.clone();
        let _thread = thread::Builder::new()
            .name("motion-preview".into())
            .spawn(move || {
                run_preview_worker(
                    source,
                    context,
                    calibration,
                    #[cfg(feature = "gpu")]
                    textures,
                    worker,
                    notify_completed,
                )
            })?;
        Ok(Self { shared })
    }

    pub fn request_with_settings(
        &self,
        key: PreviewKey,
        settings: PreviewSettings,
        #[cfg(feature = "gpu")] reservation: Option<textures::TextureReservation>,
    ) {
        let (mutex, wake) = &*self.shared;
        let mut mailbox = mutex.lock_unpoisoned();
        mailbox.request_with_settings(key, settings);
        #[cfg(feature = "gpu")]
        {
            mailbox.texture_reservation = reservation;
        }
        wake.notify_one();
    }

    pub fn take_reply(&self) -> Option<PreviewReply> {
        self.shared.0.lock_unpoisoned().reply.take()
    }
}

fn run_preview_worker(
    source: StudioScene,
    context: Context,
    calibration: backend::Calibration,
    #[cfg(feature = "gpu")] textures: textures::TextureRing,
    worker: Arc<(Mutex<PreviewRequestQueue>, Condvar)>,
    notify_completed: Option<Arc<dyn Fn() + Send + Sync>>,
) {
    let mut cache: Option<(Renderer, PreviewSettings, PreviewRenderer)> = None;
    let mut inflight = std::collections::VecDeque::<(
        PreviewRequest,
        motion_core::Result<PreviewSubmission>,
        Vec<Arc<motion_core::Error>>,
        u64,
        std::time::Instant,
    )>::new();
    let mut inflight_bytes = 0u64;
    loop {
        let mut completion = None;
        if let Some(front) = inflight.front_mut() {
            let mailbox = worker.0.lock_unpoisoned();
            if mailbox.stopping {
                break;
            }
            let next_bytes = mailbox
                .pending
                .as_ref()
                .map_or(0, |_| source.composition.raster_limits().max_bytes);
            let ready = front
                .1
                .as_mut()
                .map_or(true, |submission| submission.is_ready().unwrap_or(true));
            if ready
                || mailbox.pending.is_none()
                || inflight.len() >= 2
                || inflight_bytes.saturating_add(next_bytes) > 2 * 1024 * 1024 * 1024
            {
                drop(mailbox);
                let completed = inflight.pop_front().expect("oldest pending preview");
                inflight_bytes -= completed.3;
                completion = Some(completed);
            }
        }
        #[cfg(feature = "gpu")]
        let mut texture_reservation = None;
        let request = if let Some(completed) = &completion {
            completed.0
        } else {
            let (mutex, wake) = &*worker;
            let mut mailbox = mutex.lock_unpoisoned();
            while mailbox.pending.is_none() && !mailbox.stopping {
                mailbox = wake.wait_unpoisoned(mailbox);
            }
            if mailbox.stopping {
                break;
            }
            #[cfg(feature = "gpu")]
            {
                texture_reservation = mailbox.texture_reservation.take();
            }
            mailbox.pending.take().expect("pending preview")
        };
        // Drop the mailbox lock before font/GPU work so UI requests remain nonblocking.
        let started = completion
            .as_ref()
            .map_or_else(std::time::Instant::now, |entry| entry.4);
        let result = if let Some((_, submission, diagnostics, _, _)) = completion {
            catch_unwind(AssertUnwindSafe(|| {
                submission
                    .and_then(PreviewSubmission::finish)
                    .map(|image| (image, diagnostics))
            }))
        } else {
            let submitted = catch_unwind(AssertUnwindSafe(|| {
                submit_preview(
                    &source,
                    request,
                    &mut cache,
                    &calibration,
                    #[cfg(feature = "gpu")]
                    &textures,
                )
            }));
            #[cfg(feature = "gpu")]
            drop(texture_reservation);
            match submitted {
                Ok(Ok((mut submission, diagnostics))) => {
                    let ready = submission.is_ready();
                    if matches!(ready, Ok(false)) {
                        let bytes = source.composition.raster_limits().max_bytes;
                        inflight_bytes = inflight_bytes.saturating_add(bytes);
                        inflight.push_back((request, Ok(submission), diagnostics, bytes, started));
                        continue;
                    }
                    catch_unwind(AssertUnwindSafe(|| {
                        ready
                            .and_then(|_| submission.finish())
                            .map(|image| (image, diagnostics))
                    }))
                }
                Ok(Err(error)) => Ok(Err(error)),
                Err(payload) => Err(payload),
            }
        };
        let (image, diagnostics) = preview_result(result, &source, request, &mut cache);
        let image = image.map_err(|error| error.at_frame(u64::from(request.key.frame)));
        let published = publish_preview(&worker, request, image, diagnostics, started);
        if published {
            context.request_repaint();
            if let Some(notify_completed) = &notify_completed {
                notify_completed();
            }
        }
    }
}

type PreviewResult =
    std::thread::Result<motion_core::Result<(PreviewImage, Vec<Arc<motion_core::Error>>)>>;

fn preview_result(
    result: PreviewResult,
    source: &StudioScene,
    request: PreviewRequest,
    cache: &mut Option<(Renderer, PreviewSettings, PreviewRenderer)>,
) -> (
    motion_core::Result<PreviewImage>,
    Vec<Arc<motion_core::Error>>,
) {
    match result {
        Ok(Ok((image, diagnostics))) => (Ok(image), diagnostics),
        Ok(Err(error)) => {
            if error.kind() == motion_core::ErrorKind::DeviceLost {
                *cache = None;
            }
            (Err(error), Vec::new())
        }
        Err(payload) => {
            *cache = None;
            (
                Err(motion_core::Error::scene_panic(
                    &source.name,
                    u64::from(request.key.frame),
                    payload,
                )),
                Vec::new(),
            )
        }
    }
}

fn submit_preview(
    source: &StudioScene,
    request: PreviewRequest,
    cache: &mut Option<(Renderer, PreviewSettings, PreviewRenderer)>,
    calibration: &backend::Calibration,
    #[cfg(feature = "gpu")] textures: &textures::TextureRing,
) -> motion_core::Result<(PreviewSubmission, Vec<Arc<motion_core::Error>>)> {
    let key = request.key;
    if cache
        .as_ref()
        .is_none_or(|cached| (cached.0, cached.1) != (key.renderer, request.settings))
    {
        *cache = None;
        let mut scene = source.clone();
        {
            let size = scene.composition.output_size();
            scene.composition = scene
                .composition
                .with_output_size(motion_core::PixelSize::new(
                    ((u64::from(size.width) * u64::from(request.settings.scale)) / 100).max(1)
                        as u32,
                    ((u64::from(size.height) * u64::from(request.settings.scale)) / 100).max(1)
                        as u32,
                ))?;
            if let Some(preference) = request.settings.backend {
                scene.composition = scene
                    .composition
                    .configure(|options| options.backend(preference));
            }
            scene.composition = scene
                .composition
                .configure(|options| options.quality(request.settings.quality));
        }
        let choice = if matches!(
            request.settings.backend,
            Some(motion_render::BackendPreference::Automatic { .. })
        ) {
            Some(calibration.choose(source, request.settings, key.frame))
        } else {
            None
        };
        if let Some(choice) = &choice {
            scene.composition = scene
                .composition
                .configure(|options| options.backend(choice.backend));
        }
        let mut renderer = PreviewRenderer::new(&scene, key.renderer)?;
        renderer.automatic = choice.is_some();
        renderer.selection_reason = choice.map(|choice| choice.reason);
        #[cfg(feature = "gpu")]
        let renderer = {
            let mut renderer = renderer;
            renderer.textures = textures.clone();
            renderer
        };
        *cache = Some((key.renderer, request.settings, renderer));
    }
    let renderer = &mut cache
        .as_mut()
        .ok_or_else(|| io::Error::other("Preview renderer was not initialized"))?
        .2;
    let mut snapshot = renderer.begin_preview(source, key.frame, request.settings)?;
    if let Some(reason) = &renderer.selection_reason {
        snapshot.annotate(reason);
    }
    let diagnostics = renderer.diagnostics();
    Ok((snapshot, diagnostics))
}

fn publish_preview(
    worker: &Arc<(Mutex<PreviewRequestQueue>, Condvar)>,
    request: PreviewRequest,
    image: motion_core::Result<PreviewImage>,
    diagnostics: Vec<Arc<motion_core::Error>>,
    started: std::time::Instant,
) -> bool {
    let mut mailbox = worker.0.lock_unpoisoned();
    let backend = image
        .as_ref()
        .map(|image| image.backend.clone())
        .unwrap_or_default();
    let nodes = image
        .as_ref()
        .map(|image| image.nodes.clone())
        .unwrap_or_default();
    let stats = image.as_ref().ok().and_then(|image| image.stats);
    let timings = image.as_ref().ok().and_then(|image| image.timings);
    #[cfg(feature = "gpu")]
    let texture = image.as_ref().ok().and_then(|image| image.texture.clone());
    if let Some(reply) = mailbox.publish_reply(request, image.map(|image| image.image)) {
        reply.elapsed = started.elapsed();
        reply.stats = stats;
        reply.timings = timings;
        reply.nodes = nodes;
        reply.diagnostics = diagnostics;
        reply.backend = backend;
        #[cfg(feature = "gpu")]
        {
            reply.texture = texture;
        }
        true
    } else {
        false
    }
}

impl Drop for PreviewWorker {
    fn drop(&mut self) {
        let (mutex, wake) = &*self.shared;
        let mut mailbox = mutex.lock_unpoisoned();
        mailbox.stopping = true;
        mailbox.pending = None;
        #[cfg(feature = "gpu")]
        {
            mailbox.texture_reservation = None;
        }
        wake.notify_one();
    }
}
