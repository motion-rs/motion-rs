//! Timeline panel: frame ruler, playhead, cached-frame runs, audio overview and markers.

use crate::app::{StudioApp, StudioScene};
use eframe::egui;
use std::sync::{Arc, Mutex};
use waveform::Waveform;

#[cfg(test)]
mod tests;
mod track;
mod transport;
mod waveform;

const LABEL_WIDTH: f32 = 64.0;

const RULER_HEIGHT: f32 = 26.0;

const ROW_HEIGHT: f32 = 30.0;

const WAVE_HEIGHT: f32 = 36.0;

const CACHE_HEIGHT: f32 = 16.0;

const MIN_TICK_SPACING: f32 = 72.0;

pub(crate) struct TimelineView {
    tracks: Vec<Vec<(u32, u32, usize, u32)>>,
    single_scene: bool,
    cache_runs: Vec<(u32, u32)>,
    pub cues: Vec<(u32, String)>,
    pub cue_name_draft: String,

    waveform_result: Waveform,
    waveform_reported: bool,
    snap_audio: bool,
}

impl TimelineView {
    pub fn new(
        scene: &StudioScene,
        context: &egui::Context,
        clock: crate::playback::StudioClock,
    ) -> Self {
        let rate = clock.rate;
        let tracks = scene
            .definition
            .audio_timeline
            .as_ref()
            .map(|timeline| {
                timeline
                    .tracks()
                    .iter()
                    .map(|track| {
                        track
                            .clips()
                            .iter()
                            .map(|clip| {
                                (
                                    clip.start().to_frame(rate) as u32,
                                    clip.end().to_frame(rate) as u32,
                                    clip.id(),
                                    clip.transition_in()
                                        .map_or(0, |(_, duration)| duration.to_frame(rate) as u32),
                                )
                            })
                            .collect()
                    })
                    .collect()
            })
            .unwrap_or_else(|| vec![vec![(0, clock.frames, 0, 0)]]);
        let waveform_result = Arc::new(Mutex::new(None));
        if let Some(timeline) = scene.definition.audio_timeline.clone() {
            waveform::spawn_analysis(&waveform_result, timeline, context);
        }
        Self {
            tracks,
            single_scene: scene.definition.audio_timeline.is_none(),
            cache_runs: Vec::new(),
            cues: Vec::new(),
            cue_name_draft: "Cue".into(),
            waveform_result,
            waveform_reported: false,
            snap_audio: false,
        }
    }

    /// Merges cached frame numbers into contiguous runs so painting and hit testing
    /// cost one shape per run rather than one widget per frame.
    pub fn set_cache(&mut self, frames: &[u32]) {
        let mut sorted = frames.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        self.cache_runs = merge_runs(&sorted);
    }

    fn cache_contains(&self, frame: u32) -> bool {
        let index = self.cache_runs.partition_point(|run| run.1 <= frame);
        self.cache_runs
            .get(index)
            .is_some_and(|run| run.0 <= frame && frame < run.1)
    }
}

fn merge_runs(sorted: &[u32]) -> Vec<(u32, u32)> {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for &frame in sorted {
        match runs.last_mut() {
            Some(run) if run.1 == frame => run.1 = frame.saturating_add(1),
            _ => runs.push((frame, frame.saturating_add(1))),
        }
    }
    runs
}

pub(crate) fn frame_at(rect: egui::Rect, point: egui::Pos2, total: u32) -> u32 {
    (((point.x - rect.left()) / rect.width()).clamp(0.0, 1.0) * total as f32)
        .floor()
        .min(total.saturating_sub(1) as f32) as u32
}

/// Nearest frame boundary under the pointer, used by the In/Out handles.
fn boundary_at(rect: egui::Rect, point: egui::Pos2, total: u32) -> u32 {
    (((point.x - rect.left()) / rect.width()).clamp(0.0, 1.0) * total as f32).round() as u32
}

fn timecode(seconds: f64) -> String {
    let minutes = (seconds / 60.0).floor();
    format!("{:02}:{:06.3}", minutes as u64, seconds - minutes * 60.0)
}

/// Picks the smallest round interval whose labels stay readable.
fn tick_step(total: u32, fps: u32, width: f32) -> u32 {
    let fps = fps.max(1);
    let mut candidates = vec![1, 2, 5, 10];
    candidates.extend(
        [1, 2, 5, 10, 30, 60, 300, 600, 1800, 3600]
            .into_iter()
            .map(|seconds| fps.saturating_mul(seconds)),
    );
    candidates.sort_unstable();
    candidates.dedup();
    let per_frame = width / total.max(1) as f32;
    candidates
        .iter()
        .copied()
        .find(|step| *step as f32 * per_frame >= MIN_TICK_SPACING)
        .unwrap_or_else(|| *candidates.last().expect("candidates"))
}

fn tick_label(frame: u32, fps: u32, step: u32) -> String {
    if step >= fps.max(1) {
        let seconds = frame / fps.max(1);
        format!("{}:{:02}", seconds / 60, seconds % 60)
    } else {
        frame.to_string()
    }
}

impl StudioApp {
    pub fn timeline_panel(&mut self, ui: &mut egui::Ui) {
        let context = &ui.ctx().clone();
        let rows = self.inspection.timeline_view.tracks.len() as f32;
        let audio = if self.scene.definition.audio_timeline.is_some() {
            WAVE_HEIGHT
        } else {
            0.0
        };
        let content = RULER_HEIGHT + rows * ROW_HEIGHT + audio + CACHE_HEIGHT + 4.0 + 56.0;
        egui::Panel::bottom("timeline")
            .resizable(true)
            .default_size(content.clamp(120.0, 380.0))
            .size_range(120.0..=380.0)
            .frame(
                egui::Frame::side_top_panel(&context.global_style())
                    .inner_margin(egui::Margin::symmetric(12, 8)),
            )
            .show(ui, |ui| {
                self.transport_bar(ui);
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.track_view(ui, context));
            });
    }
}
