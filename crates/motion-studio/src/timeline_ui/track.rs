//! Timeline track view: painting and hit testing for the ruler, clip lanes, audio
//! waveform lane, cache lane, export range, cue flags and playhead, plus hover thumbnails.

use crate::app::StudioApp;
use crate::theme;
use eframe::egui::{self, Color32};

use super::{
    CACHE_HEIGHT, LABEL_WIDTH, ROW_HEIGHT, RULER_HEIGHT, WAVE_HEIGHT, boundary_at, frame_at,
    tick_label, tick_step, timecode,
};

impl StudioApp {
    fn thumbnail_for(
        &mut self,
        context: &egui::Context,
        frame: u32,
    ) -> Option<egui::TextureHandle> {
        self.preview.request_thumbnail(crate::preview::PreviewKey {
            frame,
            renderer: motion_render::Renderer::Cpu,
        });
        if self.thumbnail.as_ref().is_none_or(|entry| entry.0 != frame)
            && let Some((_, image)) = self.cached_previews.iter().find(|entry| entry.0 == frame)
        {
            let longest = image.size[0].max(image.size[1]).max(160);
            let width = (image.size[0] * 160 / longest).max(1);
            let height = (image.size[1] * 160 / longest).max(1);
            let mut preview = egui::ColorImage::filled([width, height], Color32::TRANSPARENT);
            for row in 0..height {
                for column in 0..width {
                    preview.pixels[row * width + column] =
                        image.pixels[(row * image.size[1] / height) * image.size[0]
                            + column * image.size[0] / width];
                }
            }
            let texture =
                context.load_texture("cache-thumbnail", preview, egui::TextureOptions::LINEAR);
            self.thumbnail = Some((frame, texture));
        }
        self.thumbnail
            .as_ref()
            .filter(|entry| entry.0 == frame)
            .map(|entry| entry.1.clone())
    }

    pub(super) fn track_view(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        let total = self.clock.frames;
        let fps = (f64::from(self.clock.rate.numerator())
            / f64::from(self.clock.rate.denominator()))
        .round()
        .max(1.0) as u32;
        let colors = theme::palette_for(ui);
        let rows = self.inspection.timeline_view.tracks.len();
        let has_audio = self.scene.definition.audio_timeline.is_some()
            && !matches!(
                self.inspection.timeline_view
                    .waveform_result
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .as_ref(),
                Some(Ok(values)) if values.peaks.is_empty()
            );
        let tracks_top = RULER_HEIGHT;
        let wave_top = tracks_top + rows as f32 * ROW_HEIGHT;
        let cache_top = wave_top + if has_audio { WAVE_HEIGHT } else { 0.0 };
        let height = cache_top + CACHE_HEIGHT + 4.0;
        let (area, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), height),
            egui::Sense::hover(),
        );
        let left = area.left() + LABEL_WIDTH;
        let right = (area.right() - 10.0).max(left + 1.0);
        let ruler = egui::Rect::from_min_max(
            egui::pos2(left, area.top()),
            egui::pos2(right, area.top() + RULER_HEIGHT),
        );
        let position = |frame: u32| left + ruler.width() * frame as f32 / total as f32;
        let painter = ui.painter().clone();

        let seek = ui.interact(
            egui::Rect::from_min_max(ruler.min, egui::pos2(area.right(), area.bottom())),
            ui.scope_id().with("playhead"),
            egui::Sense::click_and_drag(),
        );
        self.playback.scrub_dragging = seek.dragged();
        if seek.drag_stopped() {
            self.playback.last_interaction = std::time::Instant::now();
        }
        if (seek.clicked() || seek.dragged())
            && let Some(point) = seek.interact_pointer_pos()
        {
            let mut frame = frame_at(ruler, point, total);
            if self.inspection.timeline_view.snap_audio {
                let waveform = self
                    .inspection
                    .timeline_view
                    .waveform_result
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if let Some(Ok(overview)) = waveform.as_ref()
                    && let Some(grid) = overview.beat
                {
                    let time = motion_core::Time::from_frame(i64::from(frame), self.clock.rate);
                    let index = ((time - grid.offset).as_seconds() * grid.bpm / 60.0)
                        .round()
                        .max(0.0);
                    let snapped =
                        grid.offset + motion_core::Duration::from_seconds(index * 60.0 / grid.bpm);
                    frame = (snapped.to_frame(self.clock.rate).max(0) as u32).min(total - 1);
                }
            }
            self.seek(frame);
        }
        let mut range = self.exports.export_range;
        for (which, value) in [
            (0usize, self.exports.export_range.0),
            (1, self.exports.export_range.1),
        ] {
            let handle = egui::Rect::from_center_size(
                egui::pos2(position(value), ruler.center().y),
                egui::vec2(12.0, ruler.height()),
            );
            let response = ui
                .interact(
                    handle,
                    ui.scope_id().with(("range-handle", which)),
                    egui::Sense::drag(),
                )
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
                .on_hover_text(if which == 0 { "Drag In" } else { "Drag Out" });
            if response.dragged()
                && let Some(point) = response.interact_pointer_pos()
            {
                let boundary = boundary_at(ruler, point, total);
                if which == 0 {
                    range.0 = boundary.min(range.1 - 1);
                } else {
                    range.1 = boundary.clamp(range.0 + 1, total);
                }
            }
        }
        if range != self.exports.export_range {
            self.exports.export_range = range;
            self.pause();
        }
        let hovered = if seek.hovered() && !seek.dragged() {
            context
                .input(|input| input.pointer.hover_pos())
                .map(|point| (point, frame_at(ruler, point, total)))
        } else {
            None
        };
        let thumbnail = hovered.and_then(|(_, frame)| self.thumbnail_for(context, frame));

        let step = tick_step(total, fps, ruler.width());
        let body_bottom = area.top() + cache_top + CACHE_HEIGHT;
        for tick in (0..total).step_by(step as usize) {
            let x = position(tick);
            painter.line_segment(
                [
                    egui::pos2(x, ruler.bottom() - 7.0),
                    egui::pos2(x, body_bottom),
                ],
                egui::Stroke::new(1.0_f32, colors.ruler_tick.gamma_multiply(0.35)),
            );
            if x + 40.0 <= area.right() {
                painter.text(
                    egui::pos2(x + 3.0, ruler.top() + 2.0),
                    egui::Align2::LEFT_TOP,
                    tick_label(tick, fps, step),
                    egui::FontId::monospace(10.0),
                    colors.ruler_text,
                );
            }
        }

        for index in 0..rows {
            let top = area.top() + tracks_top + index as f32 * ROW_HEIGHT;
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(left, top + 1.0),
                    egui::pos2(right, top + ROW_HEIGHT - 1.0),
                ),
                4.0,
                colors.lane,
            );
            painter.text(
                egui::pos2(area.left(), top + ROW_HEIGHT / 2.0),
                egui::Align2::LEFT_CENTER,
                format!("Track {}", index + 1),
                egui::FontId::proportional(11.0),
                colors.ruler_text,
            );
            for (start, end, id, transition) in &self.inspection.timeline_view.tracks[index] {
                let rect = egui::Rect::from_min_max(
                    egui::pos2(position(*start), top + 2.0),
                    egui::pos2(
                        position(*end).max(position(*start) + 2.0),
                        top + ROW_HEIGHT - 2.0,
                    ),
                );
                let response = ui.interact(
                    rect,
                    ui.scope_id().with(("clip", index, id)),
                    egui::Sense::hover(),
                );
                let fill = colors.clip[index % colors.clip.len()];
                painter.rect_filled(
                    rect,
                    4.0,
                    if response.hovered() {
                        fill.lerp_to_gamma(Color32::WHITE, 0.14)
                    } else {
                        fill
                    },
                );
                if *transition > 0 {
                    let overlap = egui::Rect::from_min_max(
                        rect.min,
                        egui::pos2(
                            position(start.saturating_add(*transition)).min(rect.right()),
                            rect.bottom(),
                        ),
                    );
                    painter.rect_filled(overlap, 4.0, colors.transition);
                    painter.line_segment(
                        [overlap.left_bottom(), overlap.right_top()],
                        egui::Stroke::new(1.0_f32, Color32::WHITE),
                    );
                }
                painter.with_clip_rect(rect).text(
                    rect.left_center() + egui::vec2(8.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    if self.inspection.timeline_view.single_scene {
                        "Scene".to_string()
                    } else {
                        format!("Clip {id}")
                    },
                    egui::FontId::proportional(11.5),
                    Color32::WHITE,
                );
                response.on_hover_text(format!(
                    "Track {}, clip {id}\nFrames {start}..{end}\nTransition {transition} frames",
                    index + 1
                ));
            }
        }

        if has_audio {
            let lane = egui::Rect::from_min_max(
                egui::pos2(left, area.top() + wave_top + 1.0),
                egui::pos2(right, area.top() + wave_top + WAVE_HEIGHT - 1.0),
            );
            painter.rect_filled(lane, 4.0, colors.lane);
            painter.text(
                egui::pos2(area.left(), lane.center().y),
                egui::Align2::LEFT_CENTER,
                "Audio",
                egui::FontId::proportional(11.0),
                colors.ruler_text,
            );
            let waveform = self
                .inspection
                .timeline_view
                .waveform_result
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match waveform.as_ref() {
                Some(Ok(overview)) if !overview.peaks.is_empty() => {
                    let values = &overview.peaks;
                    let bars = (lane.width() / 2.0).floor().max(1.0) as usize;
                    let mut mesh = egui::Mesh::default();
                    for bar in 0..bars {
                        let from = bar * values.len() / bars;
                        let to = ((bar + 1) * values.len() / bars).clamp(from + 1, values.len());
                        let level = values[from..to]
                            .iter()
                            .fold(0.0_f32, |peak, value| peak.max(value.clamp(0.0, 1.0)));
                        let bar_height = (level * (lane.height() - 6.0)).max(1.0);
                        mesh.add_colored_rect(
                            egui::Rect::from_center_size(
                                egui::pos2(lane.left() + (bar as f32 + 0.5) * 2.0, lane.center().y),
                                egui::vec2(1.4, bar_height),
                            ),
                            colors.waveform,
                        );
                    }
                    painter.add(egui::Shape::mesh(mesh));
                    if !overview.curve.is_empty() {
                        let points: Vec<_> = (0..bars)
                            .map(|bar| {
                                let value = overview.curve[bar * overview.curve.len() / bars]
                                    .clamp(0.0, 1.0);
                                egui::pos2(
                                    lane.left() + lane.width() * bar as f32 / bars as f32,
                                    lane.bottom() - 3.0 - value * (lane.height() - 6.0),
                                )
                            })
                            .collect();
                        painter.add(egui::Shape::line(
                            points,
                            egui::Stroke::new(1.0_f32, colors.ruler_text),
                        ));
                    }
                    let mut last_x = f32::NEG_INFINITY;
                    for time in &overview.markers {
                        let x = position(time.to_frame(self.clock.rate).max(0) as u32);
                        if x - last_x >= 2.0 && x <= lane.right() {
                            painter.line_segment(
                                [egui::pos2(x, lane.top()), egui::pos2(x, lane.top() + 7.0)],
                                egui::Stroke::new(1.0_f32, colors.cue),
                            );
                            last_x = x;
                        }
                    }
                }
                Some(Ok(_)) => {
                    painter.text(
                        lane.center(),
                        egui::Align2::CENTER_CENTER,
                        "No audio clips",
                        egui::FontId::proportional(11.0),
                        colors.ruler_text,
                    );
                }
                Some(Err(error)) => {
                    painter.text(
                        lane.left_center() + egui::vec2(8.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        "Audio analysis failed",
                        egui::FontId::proportional(11.0),
                        colors.error,
                    );
                    ui.interact(
                        lane,
                        ui.scope_id().with("waveform-error"),
                        egui::Sense::hover(),
                    )
                    .on_hover_text(error);
                    if !self.inspection.timeline_view.waveform_reported {
                        if self.observations.diagnostics.len() == 128 {
                            self.observations.diagnostics.pop_front();
                        }
                        self.observations
                            .diagnostics
                            .push_back((0, format!("Audio analysis: {error}")));
                        self.inspection.timeline_view.waveform_reported = true;
                    }
                }
                None => {
                    painter.text(
                        lane.left_center() + egui::vec2(8.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        "Analyzing audio...",
                        egui::FontId::proportional(11.0),
                        colors.ruler_text,
                    );
                }
            }
        }

        let cache = egui::Rect::from_min_max(
            egui::pos2(left, area.top() + cache_top + 2.0),
            egui::pos2(right, area.top() + cache_top + CACHE_HEIGHT - 2.0),
        );
        painter.text(
            egui::pos2(area.left(), cache.center().y),
            egui::Align2::LEFT_CENTER,
            "Cache",
            egui::FontId::proportional(10.0),
            colors.ruler_text,
        );
        painter.rect_filled(cache, 2.0, colors.lane);
        for (start, end) in &self.inspection.timeline_view.cache_runs {
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(position(*start), cache.top()),
                    egui::pos2(position(*end).max(position(*start) + 2.0), cache.bottom()),
                ),
                2.0,
                colors.cache,
            );
        }
        let cache_response = ui.interact(
            cache,
            ui.scope_id().with("cache-lane"),
            egui::Sense::hover(),
        );
        if let Some(point) = cache_response.hover_pos() {
            let frame = frame_at(ruler, point, total);
            if self.inspection.timeline_view.cache_contains(frame) {
                let texture = self.thumbnail_for(context, frame);
                cache_response.on_hover_ui(|ui| {
                    ui.label(format!("Cached frame {frame}"));
                    if let Some(texture) = &texture {
                        ui.image((texture.id(), texture.size_vec2()));
                    }
                });
            }
        }

        let dim = Color32::from_black_alpha(if ui.visuals().dark_mode { 90 } else { 40 });
        let body = egui::Rect::from_min_max(
            egui::pos2(left, ruler.bottom()),
            egui::pos2(right, body_bottom),
        );
        let (start, end) = (
            position(self.exports.export_range.0),
            position(self.exports.export_range.1),
        );
        painter.rect_filled(
            egui::Rect::from_min_max(body.min, egui::pos2(start, body.bottom())),
            0.0,
            dim,
        );
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(end, body.top()), body.max),
            0.0,
            dim,
        );
        for (boundary, x) in [(0, start), (1, end)] {
            painter.line_segment(
                [
                    egui::pos2(x, ruler.top() + 4.0),
                    egui::pos2(x, body.bottom()),
                ],
                egui::Stroke::new(1.5_f32, colors.range),
            );
            let bracket = if boundary == 0 { 1.0 } else { -1.0 };
            painter.rect_filled(
                egui::Rect::from_two_pos(
                    egui::pos2(x, ruler.bottom() - 9.0),
                    egui::pos2(x + 5.0 * bracket, ruler.bottom() - 1.0),
                ),
                1.0,
                colors.range,
            );
        }

        for (frame, name) in &self.inspection.timeline_view.cues {
            let x = position(*frame);
            flag(
                &painter,
                egui::pos2(x, ruler.bottom() - 1.0),
                4.5,
                colors.cue,
            );
            ui.interact(
                egui::Rect::from_center_size(
                    egui::pos2(x, ruler.bottom() - 6.0),
                    egui::vec2(10.0, 10.0),
                ),
                ui.scope_id().with(("cue", frame, name)),
                egui::Sense::hover(),
            )
            .on_hover_text(format!("{name} / frame {frame}"));
        }

        if let Some((point, _)) = hovered
            && point.x >= left
            && point.x <= right
        {
            painter.line_segment(
                [
                    egui::pos2(point.x, ruler.top()),
                    egui::pos2(point.x, body_bottom),
                ],
                egui::Stroke::new(1.0_f32, colors.ruler_text.gamma_multiply(0.6)),
            );
        }
        let x = position(self.playback.transport.frame);
        painter.line_segment(
            [egui::pos2(x, ruler.top() + 6.0), egui::pos2(x, body_bottom)],
            egui::Stroke::new(1.5_f32, colors.playhead),
        );
        flag(
            &painter,
            egui::pos2(x, ruler.top() + 10.0),
            5.5,
            colors.playhead,
        );

        if let Some((_, frame)) = hovered {
            let rate = self.clock.rate;
            seek.on_hover_ui(|ui| {
                ui.label(format!(
                    "{}  /  frame {frame}",
                    timecode(motion_core::Time::from_frame(i64::from(frame), rate).as_seconds())
                ));
                if let Some(texture) = &thumbnail {
                    ui.image((texture.id(), texture.size_vec2()));
                }
            });
        }
    }
}

fn flag(painter: &egui::Painter, tip: egui::Pos2, half: f32, color: Color32) {
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(tip.x - half, tip.y - half * 1.4),
            egui::pos2(tip.x + half, tip.y - half * 1.4),
            tip,
        ],
        color,
        egui::Stroke::NONE,
    ));
}
