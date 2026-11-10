//! Hot reload of file-backed scenes: polls watched files, rebuilds the scene on a worker
//! and swaps it into the running workbench without losing the playhead or settings.

use super::{StudioApp, StudioScene};
use crate::app::scene::ReloadSource;
use crate::preview::PreviewCoordinator;
use eframe::egui;
use motion_core::Result;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant, SystemTime};

/// Time between checks of the watched files.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Quiet time after the last observed change before the scene is rebuilt, so an editor
/// that writes a file in several steps triggers one reload.
const SETTLE: Duration = Duration::from_millis(200);

/// Largest accepted reload error text shown in the header.
const MAX_MESSAGE_CHARS: usize = 600;

/// Watch state of one reloadable scene.
pub(crate) struct ReloadState {
    source: Arc<ReloadSource>,
    stamps: Vec<Option<SystemTime>>,
    last_poll: Instant,
    changed_at: Option<Instant>,
    running: Option<Receiver<Result<StudioScene>>>,
    /// A change arrived while a rebuild was running.
    dirty: bool,
}

impl ReloadState {
    pub(crate) fn new(source: Arc<ReloadSource>) -> Self {
        let stamps = source.watch.iter().map(stamp).collect();
        Self {
            source,
            stamps,
            last_poll: Instant::now(),
            changed_at: None,
            running: None,
            dirty: false,
        }
    }

    /// Whether the watched files differ from the last observation.
    fn poll(&mut self, now: Instant) -> bool {
        if now.saturating_duration_since(self.last_poll) < POLL_INTERVAL {
            return false;
        }
        self.last_poll = now;
        let current: Vec<_> = self.source.watch.iter().map(stamp).collect();
        if current == self.stamps {
            return false;
        }
        self.stamps = current;
        true
    }
}

fn stamp(path: &PathBuf) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
}

impl StudioApp {
    /// Starts watching when the scene was registered as reloadable.
    pub(crate) fn watch_scene(&mut self) {
        self.reload = self.scene.reload.clone().map(ReloadState::new);
    }

    /// Advances the reload state machine; called once per frame.
    pub(crate) fn poll_reload(&mut self, context: &egui::Context) {
        let Some(state) = &mut self.reload else {
            return;
        };
        let now = Instant::now();
        if state.poll(now) {
            if state.running.is_some() {
                state.dirty = true;
            } else {
                state.changed_at = Some(now);
            }
        }
        if let Some(receiver) = &state.running {
            match receiver.try_recv() {
                Ok(result) => {
                    state.running = None;
                    if state.dirty {
                        state.dirty = false;
                        state.changed_at = Some(now);
                    }
                    self.finish_reload(context, result);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    state.running = None;
                    self.reload_status = Some((true, "Reload worker stopped".into()));
                }
            }
        }
        let Some(state) = &mut self.reload else {
            return;
        };
        if state.running.is_none()
            && state
                .changed_at
                .is_some_and(|at| now.saturating_duration_since(at) >= SETTLE)
        {
            state.changed_at = None;
            let (sender, receiver) = std::sync::mpsc::channel();
            let source = state.source.clone();
            let wake = context.clone();
            let spawned = std::thread::Builder::new()
                .name("motion-studio-reload".into())
                .spawn(move || {
                    let result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (source.load)()))
                            .unwrap_or_else(|_| {
                                Err(motion_core::Error::Parameters(Box::new(
                                    std::io::Error::other("The reload function panicked"),
                                )))
                            });
                    let _ = sender.send(result);
                    wake.request_repaint();
                });
            match spawned {
                Ok(_) => {
                    state.running = Some(receiver);
                    self.reload_status = Some((false, "Reloading…".into()));
                }
                Err(error) => self.reload_status = Some((true, error.to_string())),
            }
        }
        context.request_repaint_after(POLL_INTERVAL);
    }

    fn finish_reload(&mut self, context: &egui::Context, result: Result<StudioScene>) {
        let applied = result.and_then(|scene| self.replace_scene(context, scene));
        self.reload_status = Some(match applied {
            Ok(()) => (false, "Reloaded".into()),
            Err(error) => {
                let mut message: String =
                    error.to_string().chars().take(MAX_MESSAGE_CHARS).collect();
                message.insert_str(0, "Reload failed, showing the previous version: ");
                self.observations
                    .diagnostics
                    .push_back((self.playback.transport.frame, message.clone()));
                while self.observations.diagnostics.len() > 128 {
                    self.observations.diagnostics.pop_front();
                }
                (true, message)
            }
        });
    }

    /// Swaps in a rebuilt scene, keeping the playhead, cues and settings.
    ///
    /// The new scene is fully validated and its preview workers are started before
    /// anything is replaced, so a failure leaves the running scene untouched.
    pub(crate) fn replace_scene(
        &mut self,
        context: &egui::Context,
        mut scene: StudioScene,
    ) -> Result<()> {
        let clock = super::validate_scene(&scene)?;
        let preview = PreviewCoordinator::new(scene.clone(), context.clone())?;
        scene.reload = self.scene.reload.clone();
        self.pause();
        let mut view = crate::timeline_ui::TimelineView::new(&scene, context, clock);
        view.cues = std::mem::take(&mut self.inspection.timeline_view.cues)
            .into_iter()
            .filter(|(frame, _)| *frame < clock.frame_count())
            .collect();
        self.inspection.timeline_view = view;
        let previous = self.clock;
        self.preview = preview;
        self.observations.diagnostics = super::alignment_diagnostics(&scene, clock);
        self.exports.export_size = scene.composition.output_size();
        let (start, end) = self.exports.export_range;
        self.exports.export_range = if end >= previous.frame_count() || end > clock.frame_count() {
            (
                start.min(clock.frame_count().saturating_sub(1)),
                clock.frame_count(),
            )
        } else {
            (start, end)
        };
        self.scene = scene;
        self.clock = clock;
        self.playback.transport.frame = self
            .playback
            .transport
            .frame
            .min(clock.frame_count().saturating_sub(1));
        self.cached_frames.clear();
        self.cached_previews.clear();
        self.cached_bytes = 0;
        self.thumbnail = None;
        self.inspection.nodes = Arc::from([]);
        self.preview_state = Default::default();
        Ok(())
    }
}
