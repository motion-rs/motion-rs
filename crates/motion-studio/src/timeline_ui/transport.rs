//! Timeline transport bar: playback buttons, timecode and frame field, In/Out range
//! editing, cue markers and the audio report menu.

use crate::app::{StudioApp, tool};
use crate::theme;
use eframe::egui::{self, RichText};
use egui_phosphor::regular as icons;

use super::timecode;

impl StudioApp {
    pub(super) fn transport_bar(&mut self, ui: &mut egui::Ui) {
        let clock = self.clock;
        let total = clock.frames;
        let colors = theme::palette_for(ui);
        ui.horizontal_wrapped(|ui| {
            let old_range = self.exports.export_range;
            if tool(ui, icons::SKIP_BACK, "First frame (Home)") {
                self.seek(0);
            }
            if tool(ui, icons::CARET_LEFT, "Previous frame (Left)") {
                self.seek(self.playback.transport.frame.saturating_sub(1));
            }
            let playing = self.playback.transport.playing();
            let play = ui
                .add_enabled_ui(
                    self.exports.export.is_none()
                        && self.exports.export_queue.is_empty()
                        && !self.close_when_idle,
                    |ui| {
                        theme::primary_button(
                            ui,
                            (if playing { icons::PAUSE } else { icons::PLAY }).to_string(),
                            egui::vec2(40.0, 28.0),
                        )
                        .on_hover_text("Play / pause (Space)")
                    },
                )
                .inner;
            if play.clicked() {
                self.toggle_playback();
            }
            if tool(ui, icons::CARET_RIGHT, "Next frame (Right)") {
                self.seek(self.playback.transport.frame.saturating_add(1));
            }
            if tool(ui, icons::SKIP_FORWARD, "Last frame (End)") {
                self.seek(total - 1);
            }
            if theme::icon_toggle(
                ui,
                icons::REPEAT,
                "Loop playback (L)",
                self.playback.transport.looping,
            ) {
                self.playback.transport.looping = !self.playback.transport.looping;
            }
            ui.separator();
            let mut frame = self.playback.transport.frame;
            let seconds = motion_core::Time::from_frame(i64::from(frame), clock.rate).as_seconds();
            ui.label(
                RichText::new(timecode(seconds))
                    .monospace()
                    .size(16.0)
                    .strong()
                    .color(colors.accent_text),
            );
            if ui
                .add(
                    egui::DragValue::new(&mut frame)
                        .range(0..=total - 1)
                        .prefix("Frame "),
                )
                .changed()
            {
                self.seek(frame);
            }
            ui.weak(format!(
                "{}/{} fps",
                clock.rate.numerator(),
                clock.rate.denominator()
            ));
            ui.separator();
            if ui
                .small_button("Mark In")
                .on_hover_text("Set In at playhead")
                .clicked()
            {
                self.exports.export_range.0 = frame.min(self.exports.export_range.1 - 1);
            }
            if ui
                .small_button("Mark Out")
                .on_hover_text("Set Out after playhead")
                .clicked()
            {
                self.exports.export_range.1 = frame
                    .saturating_add(1)
                    .max(self.exports.export_range.0 + 1)
                    .min(total);
            }
            ui.add(
                egui::DragValue::new(&mut self.exports.export_range.0)
                    .range(0..=self.exports.export_range.1 - 1)
                    .prefix("In "),
            );
            ui.add(
                egui::DragValue::new(&mut self.exports.export_range.1)
                    .range(self.exports.export_range.0 + 1..=total)
                    .prefix("Out "),
            );
            if old_range != self.exports.export_range {
                self.pause();
            }
            ui.separator();
            ui.add_sized(
                [80.0, 24.0],
                egui::TextEdit::singleline(&mut self.inspection.timeline_view.cue_name_draft),
            );
            if self.inspection.timeline_view.cues.len() < 256
                && tool(ui, icons::PLUS, "Add cue at playhead")
            {
                self.inspection.timeline_view.cues.push((
                    frame,
                    self.inspection
                        .timeline_view
                        .cue_name_draft
                        .chars()
                        .take(128)
                        .collect(),
                ));
                self.inspection.timeline_view.cues.sort_by_key(|cue| cue.0);
            }
            ui.menu_button(icons::BOOKMARK, |ui| {
                let mut remove = None;
                if self.inspection.timeline_view.cues.is_empty() {
                    ui.weak("No cues yet");
                }
                for (index, (frame, name)) in self.inspection.timeline_view.cues.iter().enumerate()
                {
                    ui.horizontal(|ui| {
                        ui.label(format!("{frame}: {name}"));
                        if tool(ui, icons::TRASH, "Remove cue") {
                            remove = Some(index);
                        }
                    });
                }
                if let Some(index) = remove {
                    self.inspection.timeline_view.cues.remove(index);
                }
            })
            .response
            .on_hover_text("Cue markers");
            ui.menu_button(icons::WAVEFORM, |ui| {
                let waveform = self
                    .inspection
                    .timeline_view
                    .waveform_result
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if let Some(Ok(overview)) = waveform.as_ref() {
                    if let Some(grid) = overview.beat {
                        ui.label(format!(
                            "{:.1} BPM · {:.0}%",
                            grid.bpm,
                            grid.confidence * 100.0
                        ));
                    }
                    if let Some(report) = overview.report {
                        ui.label(format!(
                            "48 kHz integrated: {} LUFS",
                            report
                                .integrated_lufs
                                .map_or_else(|| "Silence".into(), |value| format!("{value:.1}"))
                        ));
                        ui.label(format!(
                            "True peak: {:.1} dBTP",
                            20.0 * report.true_peak.max(1e-12).log10()
                        ));
                        if let Some(correlation) = report.correlation {
                            ui.label(format!("Stereo correlation: {correlation:.2}"));
                        }
                    }
                    let time = motion_core::Duration::from_frame(
                        u64::from(self.playback.transport.frame),
                        self.clock.rate,
                    );
                    for meter in &overview.meters {
                        let [rms, peak] = meter.level(time);
                        ui.label(format!("{}: {rms:.1} / {peak:.1} dBFS", meter.name));
                        ui.add(
                            egui::ProgressBar::new(((rms + 96.0) / 96.0).clamp(0.0, 1.0) as f32)
                                .desired_width(180.0),
                        );
                    }
                    if let Some(timeline) = &self.scene.definition.audio_timeline {
                        let master = timeline.soundtrack().master();
                        if let Some(normalize) = master.normalize {
                            ui.label(format!("Target: {:.1} LUFS", normalize.target_lufs));
                        }
                        if let Some(limiter) = master.limiter {
                            ui.label(format!("Limiter: {:.1} dBTP", limiter.ceiling_db));
                        }
                    }
                    if let Some(error) = &overview.analysis_error {
                        ui.colored_label(colors.error, error);
                    }
                }
                drop(waveform);
                ui.checkbox(
                    &mut self.inspection.timeline_view.snap_audio,
                    "Snap to beats",
                );
            })
            .response
            .on_hover_text("Audio report");
        });
    }
}
