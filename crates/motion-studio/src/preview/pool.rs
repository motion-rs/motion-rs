//! Preview coordinator that schedules requests across the worker pool, prefetches playback
//! frames, tracks render latency and caches thumbnails.

use super::*;
use motion_core::sync::{CondvarExt, MutexExt};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Largest thumbnail width; images shrink by one integer step until both bounds fit.
const THUMBNAIL_WIDTH: usize = 160;

/// Largest thumbnail height.
const THUMBNAIL_HEIGHT: usize = 90;

/// Byte budget of the thumbnail cache, at four bytes per pixel.
const THUMBNAIL_BUDGET: usize = 16 * 1024 * 1024;

/// Maximum number of cached thumbnails.
const MAX_THUMBNAILS: usize = 128;

struct Desired {
    request: PreviewRequest,
    playback: bool,
    idle: bool,
}

struct Entry {
    key: PreviewKey,
    settings: PreviewSettings,
    image: PreviewImage,
    bytes: usize,
}

#[derive(Default)]
struct CoordinatorState {
    thumbnail: Option<PreviewKey>,
    generation: u64,
    desired: Option<Desired>,
    reply: Option<PreviewReply>,
    stopping: bool,
    prefetching: bool,
    delivered: bool,

    thumbnails: ThumbnailCache,
}

pub(crate) struct PreviewCoordinator {
    shared: Arc<(Mutex<CoordinatorState>, Condvar)>,
}

#[derive(Default)]
struct ThumbnailCache {
    entries: VecDeque<(PreviewKey, Arc<ColorImage>)>,
    bytes: usize,
}

impl ThumbnailCache {
    fn insert(&mut self, key: PreviewKey, image: &ColorImage) {
        if image.size == [1, 1] {
            return;
        }
        let step = image.size[0]
            .div_ceil(THUMBNAIL_WIDTH)
            .max(image.size[1].div_ceil(THUMBNAIL_HEIGHT))
            .max(1);
        let width = (image.size[0] / step).max(1);
        let height = (image.size[1] / step).max(1);
        let cost = width * height * 4;
        if cost > THUMBNAIL_BUDGET {
            return;
        }
        if let Some(position) = self.entries.iter().position(|entry| entry.0 == key) {
            let old = self.entries.remove(position).expect("thumbnail cache");
            self.bytes -= old.1.pixels.len() * 4;
        }
        while self.entries.len() >= MAX_THUMBNAILS || self.bytes + cost > THUMBNAIL_BUDGET {
            let Some(old) = self.entries.pop_front() else {
                break;
            };
            self.bytes -= old.1.pixels.len() * 4;
        }
        let mut thumbnail = ColorImage::filled([width, height], eframe::egui::Color32::TRANSPARENT);
        for row in 0..height {
            for column in 0..width {
                thumbnail.pixels[row * width + column] =
                    image.pixels[(row * image.size[1] / height) * image.size[0]
                        + column * image.size[0] / width];
            }
        }
        self.bytes += cost;
        self.entries.push_back((key, Arc::new(thumbnail)));
    }
}

fn same_stream(first: PreviewRequest, second: PreviewRequest) -> bool {
    first.key.renderer == second.key.renderer && first.settings == second.settings
}

/// Recent render latency per preview settings. Draft playback, Final idle frames and other scales
/// differ several-fold, so one shared average would plan playback from unrelated renders.
#[derive(Default)]
struct Latencies(VecDeque<(PreviewSettings, Duration)>);

impl Latencies {
    /// Zero until a render with these settings has completed.
    fn get(&self, settings: PreviewSettings) -> Duration {
        self.0
            .iter()
            .find(|entry| entry.0 == settings)
            .map_or(Duration::ZERO, |entry| entry.1)
    }

    fn forget(&mut self, settings: PreviewSettings) {
        self.0.retain(|entry| entry.0 != settings);
    }

    fn observe(&mut self, settings: PreviewSettings, elapsed: Duration) {
        if let Some(entry) = self.0.iter_mut().find(|entry| entry.0 == settings) {
            entry.1 = (entry.1 * 3 + elapsed) / 4;
        } else {
            if self.0.len() == 8 {
                self.0.pop_front();
            }
            self.0.push_back((settings, elapsed));
        }
    }
}

fn lead_frames(elapsed: Duration, rate: motion_core::FrameRate) -> u32 {
    (elapsed.as_secs_f64() * f64::from(rate.numerator()) / f64::from(rate.denominator()))
        .ceil()
        .clamp(0.0, f64::from(u32::MAX)) as u32
}

fn make_preview_reply(
    request: PreviewRequest,
    image: PreviewImage,
    hit: bool,
    elapsed: Duration,
    entries: &VecDeque<Entry>,
    bytes: usize,
) -> PreviewReply {
    PreviewReply {
        key: request.key,
        image: Ok(image.image),
        nodes: image.nodes,
        #[cfg(feature = "gpu")]
        texture: image.texture,
        backend: image.backend,
        cached_frames: entries
            .iter()
            .filter(|entry| entry.settings == request.settings)
            .map(|entry| entry.key.frame)
            .collect(),
        cached_bytes: bytes,
        elapsed,
        cache_hit: hit,
        stats: image.stats.filter(|_| !hit),
        timings: image.timings.filter(|_| !hit),
        diagnostics: Vec::new(),
    }
}

impl PreviewCoordinator {
    pub fn new(scene: StudioScene, context: Context) -> io::Result<Self> {
        let value = std::env::var("MOTION_STUDIO_CACHE_MIB")
            .map(Some)
            .or_else(|error| {
                if matches!(error, std::env::VarError::NotPresent) {
                    Ok(None)
                } else {
                    Err(io::Error::new(io::ErrorKind::InvalidInput, error))
                }
            })?;
        Self::with_cache_bytes(scene, context, preview_cache_bytes(value.as_deref())?)
    }

    pub(super) fn with_cache_bytes(
        scene: StudioScene,
        context: Context,
        bytes: usize,
    ) -> io::Result<Self> {
        let cpus = thread::available_parallelism()
            .map_or(1, usize::from)
            .saturating_sub(1)
            .clamp(1, 6);
        let largest = scene.composition.raster_limits().max_bytes.max(1);
        let workers = cpus.min((2 * 1024 * 1024 * 1024_u64 / largest).max(1) as usize);
        Self::with_slots(scene, context, bytes, workers)
    }

    pub(super) fn with_slots(
        mut scene: StudioScene,
        context: Context,
        cache_bytes: usize,
        workers: usize,
    ) -> io::Result<Self> {
        let clock =
            crate::playback::StudioClock::new(&scene.composition).map_err(io::Error::other)?;
        let cache = scene
            .composition
            .studio_render_options(motion_render::BackendPreference::default())
            .cache
            .fork_partition(motion_core::CachePartition::Scene);
        scene.composition = scene
            .composition
            .clone()
            .configure(|options| options.cache(cache));
        let shared = Arc::new((Mutex::new(CoordinatorState::default()), Condvar::new()));
        #[cfg(feature = "gpu")]
        let textures = textures::TextureRing::default();
        let mut slots = Vec::new();
        for _ in 0..workers.clamp(1, 6) {
            let wake = shared.clone();
            let slot_context = Context::default();
            let completed = Arc::new(move || {
                let _guard = wake.0.lock_unpoisoned();
                wake.1.notify_one();
            });
            slots.push(worker::PreviewWorker::spawn(
                scene.clone(),
                slot_context,
                #[cfg(feature = "gpu")]
                textures.clone(),
                Some(completed),
            )?);
        }
        let coordinator = shared.clone();
        thread::Builder::new()
            .name("motion-preview-coordinator".into())
            .spawn(move || {
                CoordinatorRuntime::new(
                    scene,
                    context,
                    cache_bytes,
                    slots,
                    clock,
                    #[cfg(feature = "gpu")]
                    textures,
                )
                .run(coordinator)
            })?;
        Ok(Self { shared })
    }

    #[cfg(test)]
    pub fn request(&self, key: PreviewKey) {
        self.request_with_settings(key, PreviewSettings::default());
    }

    pub fn request_with_settings(&self, key: PreviewKey, settings: PreviewSettings) {
        self.enqueue(key, settings, false);
    }

    pub fn request_playback(&self, key: PreviewKey, settings: PreviewSettings) {
        self.enqueue(key, settings, true);
    }

    pub fn request_idle(&self, key: PreviewKey, mut settings: PreviewSettings) {
        settings.idle = true;
        self.enqueue(key, settings, false);
    }

    fn enqueue(&self, key: PreviewKey, mut settings: PreviewSettings, playback: bool) {
        let idle = settings.idle;
        settings.idle = false;
        let mut state = self.shared.0.lock_unpoisoned();
        let next = PreviewRequest {
            key,
            settings,
            generation: state.generation,
            ahead: 0,
        };
        let same = playback
            && state.desired.as_ref().is_some_and(|old| {
                old.playback && same_stream(old.request, next) && key.frame >= old.request.key.frame
            });
        if !same {
            state.generation = state.generation.wrapping_add(1);
            state.thumbnail = None;
        }
        state.desired = Some(Desired {
            request: PreviewRequest {
                generation: state.generation,
                ..next
            },
            playback,
            idle,
        });
        state.reply = None;
        state.delivered = false;
        state.prefetching = playback || idle;
        self.shared.1.notify_one();
    }

    pub fn stop_prefetch(&self) {
        let mut state = self.shared.0.lock_unpoisoned();
        state.prefetching = false;
        state.thumbnail = None;
        self.shared.1.notify_one();
    }

    pub fn take_reply(&self) -> Option<PreviewReply> {
        self.shared.0.lock_unpoisoned().reply.take()
    }

    pub fn request_thumbnail(&self, key: PreviewKey) {
        let mut state = self.shared.0.lock_unpoisoned();
        let key = PreviewKey {
            renderer: BackendKind::Cpu,
            ..key
        };
        if state.thumbnails.entries.iter().any(|entry| entry.0 == key) {
            return;
        }
        state.thumbnail = Some(key);
        self.shared.1.notify_one();
    }

    pub fn thumbnails(&self) -> Vec<(u32, Arc<ColorImage>)> {
        let state = self.shared.0.lock_unpoisoned();
        state
            .thumbnails
            .entries
            .iter()
            .filter(|_| state.desired.is_some())
            .map(|entry| (entry.0.frame, entry.1.clone()))
            .collect()
    }
}

struct CandidatePlan {
    candidates: Vec<PreviewRequest>,
    playback: bool,
    ahead: u32,
}

struct CoordinatorRuntime {
    scene: StudioScene,
    context: Context,
    cache_bytes: usize,
    slots: Vec<worker::PreviewWorker>,
    busy: Vec<Option<PreviewRequest>>,
    entries: VecDeque<Entry>,
    bytes: usize,
    latency: Latencies,
    planned: VecDeque<PreviewKey>,
    planned_generation: u64,
    started: Vec<Instant>,
    clock: crate::playback::StudioClock,
    #[cfg(feature = "gpu")]
    textures: textures::TextureRing,
}

impl CoordinatorRuntime {
    fn new(
        scene: StudioScene,
        context: Context,
        cache_bytes: usize,
        slots: Vec<worker::PreviewWorker>,
        clock: crate::playback::StudioClock,
        #[cfg(feature = "gpu")] textures: textures::TextureRing,
    ) -> Self {
        let workers = slots.len();
        Self {
            scene,
            context,
            cache_bytes,
            slots,
            busy: vec![None; workers],
            entries: VecDeque::new(),
            bytes: 0,
            latency: Latencies::default(),
            planned: VecDeque::new(),
            planned_generation: 0,
            started: vec![Instant::now(); workers],
            clock,
            #[cfg(feature = "gpu")]
            textures,
        }
    }

    fn run(mut self, coordinator: Arc<(Mutex<CoordinatorState>, Condvar)>) {
        loop {
            let mut state = coordinator.0.lock_unpoisoned();
            if state.stopping {
                break;
            }
            let mut changed = self.complete_replies(&mut state);
            if let Some(plan) = self.candidates(&mut state) {
                changed |= self.dispatch(plan);
            }
            changed |= self.thumbnail(&mut state);
            if !changed {
                state = coordinator.1.wait_unpoisoned(state);
            }
            drop(state);
        }
    }

    fn complete_replies(&mut self, state: &mut CoordinatorState) -> bool {
        let mut changed = false;
        for index in 0..self.slots.len() {
            let Some(result) = self.slots[index].take_reply() else {
                continue;
            };
            let Some(mut request) = self.busy[index].take() else {
                continue;
            };
            changed = true;
            let elapsed = self.started[index].elapsed();
            if request.ahead == 2 {
                if request.generation == state.generation
                    && result.diagnostics.is_empty()
                    && let Ok(image) = &result.image
                {
                    state.thumbnails.insert(request.key, image);
                }
                self.context.request_repaint();
                continue;
            }
            self.latency.observe(request.settings, elapsed);
            let Some(desired) = &state.desired else {
                continue;
            };
            if desired.request.key == request.key && same_stream(desired.request, request) {
                request.generation = desired.request.generation;
            }
            if desired.request.generation != request.generation
                || !same_stream(desired.request, request)
            {
                continue;
            }
            if desired.playback && request.key.frame < desired.request.key.frame {
                continue;
            }
            let publish = desired.request.key == request.key && !state.delivered;
            self.publish_reply(state, request, result, elapsed, publish);
        }
        changed
    }

    fn publish_reply(
        &mut self,
        state: &mut CoordinatorState,
        request: PreviewRequest,
        result: PreviewReply,
        elapsed: Duration,
        publish: bool,
    ) {
        match result.image {
            Ok(image) => {
                let image = PreviewImage {
                    image,
                    backend: result.backend,
                    nodes: result.nodes,
                    stats: result.stats,
                    timings: result.timings,
                    #[cfg(feature = "gpu")]
                    texture: result.texture,
                };
                let cost = image.retained_bytes();
                if result.diagnostics.is_empty() && cost <= self.cache_bytes {
                    while self.bytes.saturating_add(cost) > self.cache_bytes {
                        if let Some(old) = self.entries.pop_front() {
                            self.bytes -= old.bytes;
                        } else {
                            break;
                        }
                    }
                    self.entries.push_back(Entry {
                        key: request.key,
                        settings: request.settings,
                        image: image.clone(),
                        bytes: cost,
                    });
                    self.bytes += cost;
                }
                if publish {
                    let mut published = make_preview_reply(
                        request,
                        image,
                        false,
                        elapsed,
                        &self.entries,
                        self.bytes,
                    );
                    published.diagnostics = result.diagnostics;
                    state.reply = Some(published);
                    state.delivered = true;
                    self.context.request_repaint();
                }
            }
            Err(error) => {
                if publish {
                    if matches!(
                        error.kind(),
                        motion_core::ErrorKind::ScenePanic | motion_core::ErrorKind::DeviceLost
                    ) {
                        self.scene
                            .composition
                            .studio_render_options(motion_render::BackendPreference::default())
                            .cache
                            .partition(motion_core::CachePartition::Scene)
                            .clear();
                        self.entries.clear();
                        self.bytes = 0;
                        state.generation = state.generation.wrapping_add(1);
                        let generation = state.generation;
                        if let Some(desired) = &mut state.desired {
                            desired.request.generation = generation;
                        }
                        state.prefetching = false;
                    }
                    state.reply = Some(PreviewReply {
                        key: request.key,
                        image: Err(error),
                        nodes: result.nodes,
                        #[cfg(feature = "gpu")]
                        texture: None,
                        backend: result.backend,
                        cached_frames: Vec::new(),
                        cached_bytes: self.bytes,
                        elapsed,
                        cache_hit: false,
                        stats: result.stats,
                        timings: result.timings,
                        diagnostics: result.diagnostics,
                    });
                    state.delivered = true;
                    self.context.request_repaint();
                }
            }
        }
    }

    fn candidates(&mut self, state: &mut CoordinatorState) -> Option<CandidatePlan> {
        let desired = state.desired.as_ref()?;

        let request = desired.request;
        if self.planned_generation != request.generation {
            self.planned.clear();
            self.planned_generation = request.generation;
            // A new playback stream is planned from its own renders; earlier ones with these
            // settings were often cold first frames and would push prefetch far ahead.
            if desired.playback {
                self.latency.forget(request.settings);
            }
        }
        self.planned
            .retain(|key| !desired.playback || key.frame > request.key.frame);
        let playback = desired.playback;
        let idle = desired.idle;
        let prefetch = state.prefetching;
        if !state.delivered
            && let Some(position) = self
                .entries
                .iter()
                .position(|entry| entry.key == request.key && entry.settings == request.settings)
        {
            let entry = self.entries.remove(position).expect("preview cache");
            let image = entry.image.clone();
            self.entries.push_back(entry);
            state.reply = Some(make_preview_reply(
                request,
                image,
                true,
                Duration::ZERO,
                &self.entries,
                self.bytes,
            ));
            state.delivered = true;
            self.context.request_repaint();
        }
        let done = state.delivered
            || self
                .entries
                .iter()
                .any(|entry| entry.key == request.key && entry.settings == request.settings);
        let queued = self
            .busy
            .iter()
            .flatten()
            .any(|job| job.key == request.key && same_stream(*job, request));
        let lead = lead_frames(self.latency.get(request.settings), self.clock.rate);
        // Playback discards frames that finish after the clock has passed them, so when a render
        // takes longer than a frame the current frame is wasted work; its slot goes to prefetch.
        let late = playback && prefetch && lead > 1;
        let mut candidates = Vec::new();
        if !done && !queued && !late {
            candidates.push(request);
        }
        let count = self.clock.frames;
        let estimate = (self.scene.composition.output_size().width as usize
            * self.scene.composition.output_size().height as usize
            * 4
            * request.settings.scale as usize
            * request.settings.scale as usize
            / 10000)
            .max(PreviewImage::metadata_bytes() + 1);
        // Prefetch may occupy every slot except the one kept for seeks, plus a little buffer of
        // finished frames waiting for the clock.
        let ahead = (self.cache_bytes / estimate.max(1))
            .saturating_sub(1)
            .min((self.slots.len() + 1).max(4)) as u32;
        #[cfg(feature = "gpu")]
        self.textures
            .set_capacity(self.slots.len() + ahead as usize + 2);
        if playback && prefetch {
            let advance = lead.max(1).min(count.saturating_sub(request.key.frame + 1));
            for distance in advance..advance.saturating_add(ahead) {
                if let Some(frame) = request
                    .key
                    .frame
                    .checked_add(distance)
                    .filter(|frame| *frame < count)
                {
                    candidates.push(PreviewRequest {
                        key: PreviewKey {
                            frame,
                            ..request.key
                        },
                        ahead: 1,
                        ..request
                    });
                }
            }
        } else if idle && prefetch && ahead > 0 {
            for offset in [-1_i64, 1, -2, 2] {
                let frame = i64::from(request.key.frame) + offset;
                if (0..i64::from(count)).contains(&frame) {
                    candidates.push(PreviewRequest {
                        key: PreviewKey {
                            frame: frame as u32,
                            ..request.key
                        },
                        ahead: 1,
                        ..request
                    });
                }
            }
        }

        Some(CandidatePlan {
            candidates,
            playback,
            ahead,
        })
    }

    fn dispatch(&mut self, plan: CandidatePlan) -> bool {
        let CandidatePlan {
            candidates,
            playback,
            ahead,
        } = plan;
        let mut changed = false;
        for candidate in candidates {
            if playback && candidate.ahead > 0 && self.planned.len() >= ahead as usize {
                break;
            }
            if candidate.ahead > 0 && self.planned.contains(&candidate.key) {
                continue;
            }
            if self
                .entries
                .iter()
                .any(|entry| entry.key == candidate.key && entry.settings == candidate.settings)
                || self.busy.iter().flatten().any(|job| {
                    job.key == candidate.key
                        && job.settings == candidate.settings
                        && job.generation == candidate.generation
                })
            {
                continue;
            }
            if !playback && candidate.ahead > 0 && self.busy.iter().any(Option::is_some) {
                break;
            }
            let Some(index) = self.busy.iter().enumerate().position(|(index, job)| {
                job.is_none()
                    && (candidate.ahead == 0
                        || !playback
                        || self.slots.len() == 1
                        || index + 1 < self.slots.len())
            }) else {
                break;
            };
            #[cfg(feature = "gpu")]
            let reservation = if candidate.settings.pixels {
                None
            } else {
                let reserved = loop {
                    if let Some(reservation) = self.textures.reserve() {
                        break Some(reservation);
                    }
                    let Some(position) = self
                        .entries
                        .iter()
                        .position(|entry| entry.image.texture.is_some())
                    else {
                        break None;
                    };
                    let old = self
                        .entries
                        .remove(position)
                        .expect("preview texture cache");
                    self.bytes -= old.bytes;
                };
                let Some(reservation) = reserved else {
                    break;
                };
                Some(reservation)
            };
            self.slots[index].request_with_settings(
                candidate.key,
                candidate.settings,
                #[cfg(feature = "gpu")]
                reservation,
            );
            if candidate.ahead > 0 {
                if self.planned.len() == 16 {
                    self.planned.pop_front();
                }
                self.planned.push_back(candidate.key);
            }
            self.busy[index] = Some(candidate);
            self.started[index] = Instant::now();
            changed = true;
        }
        changed
    }

    fn thumbnail(&mut self, state: &mut CoordinatorState) -> bool {
        let mut changed = false;
        if self.busy.iter().all(Option::is_none)
            && state.delivered
            && let Some(key) = state.thumbnail.take()
            && !state.thumbnails.entries.iter().any(|entry| entry.0 == key)
        {
            let settings = PreviewSettings {
                scale: 25,
                quality: motion_core::RenderQuality::Draft,
                transparent: true,
                pixels: true,
                backend: Some(motion_render::BackendPreference::Cpu),
                ..PreviewSettings::default()
            };
            self.slots[0].request_with_settings(
                key,
                settings,
                #[cfg(feature = "gpu")]
                None,
            );
            self.busy[0] = Some(PreviewRequest {
                key,
                settings,
                generation: state.generation,
                ahead: 2,
            });
            self.started[0] = Instant::now();
            changed = true;
        }
        changed
    }
}

impl Drop for PreviewCoordinator {
    fn drop(&mut self) {
        self.shared.0.lock_unpoisoned().stopping = true;
        self.shared.1.notify_one();
    }
}

#[cfg(test)]
mod tests;
