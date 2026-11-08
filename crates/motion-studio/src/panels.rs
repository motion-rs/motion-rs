//! Studio export panel and timeline panel hosting, plus the `eframe::App` update loop that lays
//! out the sidebars, toolbar and status bar.

use crate::app::{StudioApp, tool};
use crate::preview::JobState;
use crate::theme;
use eframe::egui::{self, RichText};
use egui_phosphor::regular as icons;
use motion_encode::ExportPhase;
use std::time::{Duration, Instant};

fn sidebar_defaults(width: f32) -> (bool, bool) {
    (false, width >= 1120.0)
}

fn sidebar_toggle(ui: &mut egui::Ui, icon: &str, label: &str, selected: bool) -> bool {
    theme::icon_toggle_sized(ui, icon, label, selected, egui::vec2(30.0, 30.0))
}

fn preview_status(
    ui: &mut egui::Ui,
    backend: &str,
    frame: Option<u32>,
    pending: bool,
) -> egui::Response {
    let (area, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::hover());
    let mut status = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("preview-status")
            .max_rect(area),
    );
    let counter = egui::Rect::from_min_max(egui::pos2(area.right() - 128.0, area.top()), area.max);
    let busy = egui::Rect::from_center_size(
        egui::pos2(counter.left() - 18.0, area.center().y),
        egui::vec2(16.0, 16.0),
    );
    let details = egui::Rect::from_min_max(area.min, egui::pos2(busy.left() - 8.0, area.bottom()));
    let counter = status.put(
        counter,
        egui::Label::new(
            RichText::new(
                frame.map_or_else(|| "Frame ---".into(), |frame| format!("Frame {frame:03}")),
            )
            .monospace(),
        )
        .truncate(),
    );
    status
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(details)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| ui.add(egui::Label::new(RichText::new(backend).small()).truncate()),
        )
        .inner
        .on_hover_text(backend);
    if pending {
        status.put(busy, egui::Spinner::new().size(16.0));
    }
    counter
}

impl StudioApp {
    pub(crate) fn timeline(&mut self, context: &egui::Context) {
        self.timeline_panel(context);
    }

    pub(crate) fn export_panel(&mut self, context: &egui::Context) {
        let defaults = sidebar_defaults(context.input(|input| input.screen_rect().width()));
        if !self.inspection.output_visible.unwrap_or(defaults.1) {
            return;
        }
        egui::SidePanel::right("output")
            .default_width(252.0)
            .width_range(240.0..=340.0)
            .frame(
                egui::Frame::side_top_panel(&context.style())
                    .inner_margin(egui::Margin::symmetric(12, 10)),
            )
            .show(context, |ui| self.export_contents(ui, context));
    }

    fn export_contents(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(index) = theme::segmented_control(
                ui,
                &[
                    (icons::EXPORT, "Export"),
                    (icons::TREE_STRUCTURE, "Node"),
                    (icons::WARNING_CIRCLE, "Diagnostics"),
                    (icons::CHART_BAR, "Stats"),
                ],
                self.inspection.inspector_tab,
            ) {
                self.inspection.inspector_tab = index;
            }
            ui.add_space(4.0);
            ui.separator();
            if self.inspection.inspector_tab != 0 {
                self.inspector(ui);
                return;
            }
            self.export_specification(ui);
            self.export_encoding(ui);
            self.export_destination_controls(ui, context);
            self.export_progress(ui);
            self.export_queue_controls(ui);
            self.export_results(ui);
        });
    }

    fn export_specification(&self, ui: &mut egui::Ui) {
        theme::section(ui, "Output");
        let frame = self.clock;
        egui::Grid::new("specification")
            .spacing([16.0, 8.0])
            .show(ui, |ui| {
                ui.weak("Resolution");
                ui.label(format!("{} x {}", frame.width(), frame.height()));
                ui.end_row();
                ui.weak("Frame rate");
                let rate = frame.rate;
                ui.label(format!("{}/{} FPS", rate.numerator(), rate.denominator()));
                ui.end_row();
                ui.weak("Duration");
                ui.label(format!("{} frames", frame.frame_count()));
                ui.end_row();
                ui.weak("Format");
                let audio = &self.audio;
                ui.label(if !self.scene.supports_video_export() {
                    "Preview only"
                } else if audio.enabled && audio.clips.iter().any(|clip| clip.enabled) {
                    "Video + audio"
                } else {
                    self.exports.export_format.label()
                });
                ui.end_row();
            });
        ui.add_space(6.0);
    }

    fn export_encoding(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "Encode");
        {
            let old = (self.exports.export_png, self.exports.export_format);
            egui::ComboBox::from_id_salt("output-format")
                .selected_text(if self.exports.export_png {
                    "PNG"
                } else {
                    self.exports.export_format.label()
                })
                .show_ui(ui, |ui| {
                    for format in crate::export::ExportFormat::ALL {
                        if ui
                            .selectable_label(
                                !self.exports.export_png && self.exports.export_format == format,
                                format.label(),
                            )
                            .clicked()
                        {
                            self.exports.export_png = false;
                            self.exports.export_format = format;
                        }
                    }
                    ui.selectable_value(&mut self.exports.export_png, true, "PNG");
                });
            if old != (self.exports.export_png, self.exports.export_format) {
                self.exports.export_destination =
                    std::path::Path::new(&self.exports.export_destination)
                        .with_extension(if self.exports.export_png {
                            "png"
                        } else {
                            self.exports.export_format.extension().unwrap_or("")
                        })
                        .to_string_lossy()
                        .into_owned();
            }
            egui::ComboBox::from_id_salt("export-preset")
                .selected_text("Preset")
                .show_ui(ui, |ui| {
                    for (label, format, size) in [
                        (
                            "1080p H.264",
                            crate::export::ExportFormat::H264,
                            motion_core::PixelSize::new(1920, 1080),
                        ),
                        (
                            "ProRes 4444 alpha",
                            crate::export::ExportFormat::ProRes,
                            self.scene.composition.configured_output_size(),
                        ),
                        (
                            "GIF preview",
                            crate::export::ExportFormat::Gif,
                            motion_core::PixelSize::new(640, 360),
                        ),
                    ] {
                        if ui.button(label).clicked() {
                            self.exports.export_format = format;
                            self.exports.export_png = false;
                            self.exports.export_size = size;
                            self.exports.export_destination =
                                std::path::Path::new(&self.exports.export_destination)
                                    .with_extension(format.extension().unwrap_or(""))
                                    .to_string_lossy()
                                    .into_owned();
                        }
                    }
                });
        }
        self.export_dimensions(ui);
        self.export_range_controls(ui);
    }

    fn export_dimensions(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.exports.export_size.width)
                    .range(1..=16384)
                    .prefix("W "),
            );
            ui.add(
                egui::DragValue::new(&mut self.exports.export_size.height)
                    .range(1..=16384)
                    .prefix("H "),
            );
        });
        ui.horizontal(|ui| {
            if ui.small_button("Design").clicked() {
                self.exports.export_size = self.scene.composition.configured_output_size();
            }
            for height in [720, 1080, 2160] {
                if ui.small_button(format!("{height}p")).clicked() {
                    let size = self.scene.composition.configured_output_size();
                    self.exports.export_size = motion_core::PixelSize::new(
                        (f64::from(size.width) * f64::from(height) / f64::from(size.height))
                            .round()
                            .clamp(1.0, 16384.0) as u32,
                        height,
                    );
                }
            }
        });
    }

    fn export_range_controls(&mut self, ui: &mut egui::Ui) {
        let frame = self.clock;
        ui.horizontal(|ui| {
            let old_range = self.exports.export_range;
            ui.add(
                egui::DragValue::new(&mut self.exports.export_range.0)
                    .range(0..=self.exports.export_range.1 - 1)
                    .prefix("In "),
            );
            ui.add(
                egui::DragValue::new(&mut self.exports.export_range.1)
                    .range(self.exports.export_range.0 + 1..=frame.frame_count())
                    .prefix("Out "),
            );
            if old_range != self.exports.export_range {
                self.pause();
            }
        });
        ui.checkbox(&mut self.exports.export_motion_blur, "Motion blur");
    }

    fn export_destination_controls(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        let frame = self.clock;
        ui.add_enabled_ui(
            !self.close_when_idle
                && self.exports.export_queue.len() < 16
                && self.scene.supports_video_export(),
            |ui| {
                theme::section(ui, "Destination");
                ui.add(
                    egui::TextEdit::singleline(&mut self.exports.export_destination)
                        .desired_width(f32::INFINITY),
                );
                if tool(ui, icons::FOLDER_OPEN, "Choose destination") {
                    let extension = if self.exports.export_png {
                        Some("png")
                    } else {
                        self.exports.export_format.extension()
                    };
                    let mut dialog = rfd::FileDialog::new();
                    if let Some(extension) = extension {
                        dialog = dialog.add_filter("Output", &[extension]);
                    }
                    if let Some(path) = dialog
                        .set_file_name(
                            extension
                                .map_or("frames".into(), |extension| format!("render.{extension}")),
                        )
                        .save_file()
                    {
                        self.exports.export_destination = path.to_string_lossy().into_owned();
                    }
                }
                ui.add_space(8.0);
                if matches!(
                    self.exports.export_format,
                    crate::export::ExportFormat::H264
                        | crate::export::ExportFormat::H265
                        | crate::export::ExportFormat::Vp9
                ) && !self.exports.export_png
                {
                    ui.label("Quality / CRF");
                    ui.add(egui::Slider::new(
                        &mut self.exports.export_quality,
                        0..=if self.exports.export_format == crate::export::ExportFormat::Vp9 {
                            63
                        } else {
                            51
                        },
                    ));
                }
                ui.checkbox(&mut self.exports.overwrite, "Replace existing file");
                theme::section(ui, "Audio");
                let duration = f64::from(frame.frame_count()) / frame.fps();
                let previous_audio = self.audio.clone();
                let response = ui.push_id("audio", |ui| self.audio.show(ui, duration));
                if previous_audio != self.audio {
                    self.pause();
                    self.playback.audio_error = None;
                }
                if let Some(message) = response.inner {
                    self.exports.export_message = Some(message);
                }
                ui.add_space(12.0);
                if theme::primary_button(
                    ui,
                    format!("{}  Export", icons::EXPORT),
                    egui::vec2(ui.available_width(), 36.0),
                )
                .clicked()
                {
                    self.start_export(context);
                }
            },
        );
    }

    fn export_progress(&self, ui: &mut egui::Ui) {
        if let Some(job) = &self.exports.export {
            ui.separator();
            ui.label(format!("Exporting {}", job.label));
            if let JobState::Running(progress) = job.state() {
                let fraction = progress.map_or(0.0, |progress| {
                    progress.completed_frames as f32 / progress.total_frames as f32
                });
                ui.add(egui::ProgressBar::new(fraction).show_percentage());
                if let Some(progress) = progress {
                    let elapsed = job.started.elapsed().as_secs_f64();
                    let fps = progress.completed_frames as f64 / elapsed.max(0.001);
                    let remaining = progress
                        .total_frames
                        .saturating_sub(progress.completed_frames)
                        as f64
                        / fps.max(0.001);
                    ui.label(format!("{fps:.1} fps / ~{remaining:.0}s remaining"));
                }
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(if job.cancelling() {
                        "Cancelling..."
                    } else if progress
                        .is_some_and(|progress| progress.phase == ExportPhase::Finalizing)
                    {
                        "Finalizing..."
                    } else {
                        "Rendering frames..."
                    });
                });
            }
            if ui
                .add_enabled(
                    !job.cancelling(),
                    egui::Button::new(format!("{}  Cancel export", icons::X)),
                )
                .clicked()
            {
                job.cancel();
            }
        }
    }

    fn export_queue_controls(&mut self, ui: &mut egui::Ui) {
        let mut remove = None;
        for (index, task) in self.exports.export_queue.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add(
                    egui::Label::new(format!("{}: {}", index + 1, task.path.display())).truncate(),
                );
                if tool(ui, icons::X, "Remove queued task") {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            self.exports.export_queue.remove(index);
        }
    }

    fn export_results(&self, ui: &mut egui::Ui) {
        let colors = theme::palette_for(ui);
        for (path, error, message) in &self.exports.completed_exports {
            ui.separator();
            ui.add(
                egui::Label::new(egui::RichText::new(message).color(if *error {
                    colors.error
                } else {
                    colors.success
                }))
                .wrap(),
            );
            if !error && tool(ui, icons::FOLDER_OPEN, "Open output folder") {
                let folder = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or(std::path::Path::new("."));
                #[cfg(target_os = "macos")]
                let opener = "open";
                #[cfg(target_os = "windows")]
                let opener = "explorer";
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                let opener = "xdg-open";
                let _ = std::process::Command::new(opener).arg(folder).spawn();
            }
        }
        if let Some((error, message)) = &self.exports.export_message {
            ui.separator();
            ui.add(
                egui::Label::new(RichText::new(message).color(if *error {
                    colors.error
                } else {
                    colors.success
                }))
                .wrap(),
            );
        }
    }
}

impl eframe::App for StudioApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_workers(context);
        self.poll_reload(context);
        if context.input(|input| input.viewport().close_requested()) {
            self.pause();
            self.exports.export_queue.clear();
            if let Some(export) = &self.exports.export {
                self.close_when_idle = true;
                export.cancel();
                context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else {
                return;
            }
        }
        if self.close_when_idle && self.exports.export.is_none() {
            context.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        self.advance_transport();
        self.shortcuts(context);
        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::side_top_panel(&context.style())
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(context, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let width = context.input(|input| input.screen_rect().width());
                    let defaults = sidebar_defaults(width);
                    let output = self.inspection.output_visible.unwrap_or(defaults.1);
                    if sidebar_toggle(
                        ui,
                        icons::SIDEBAR_SIMPLE,
                        if output {
                            "Hide sidebar"
                        } else {
                            "Show sidebar"
                        },
                        output,
                    ) {
                        self.inspection.output_visible = Some(!output);
                    }
                    ui.with_layout(
                        egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
                        |ui| {
                            let colors = theme::palette_for(ui);
                            ui.label(
                                RichText::new("Motion Studio")
                                    .strong()
                                    .size(15.0)
                                    .color(colors.accent_text),
                            );
                            if let Some((error, message)) = &self.reload_status {
                                let colors = theme::palette_for(ui);
                                ui.label(RichText::new(message).color(if *error {
                                    colors.error
                                } else {
                                    colors.success
                                }));
                            }
                            ui.separator();
                            let old = self.preview_settings;
                            ui.weak("Preview");
                            egui::ComboBox::from_id_salt("preview-scale")
                                .width(64.0)
                                .selected_text(format!("{}%", self.preview_settings.scale))
                                .show_ui(ui, |ui| {
                                    for scale in [25, 50, 100] {
                                        ui.selectable_value(
                                            &mut self.preview_settings.scale,
                                            scale,
                                            format!("{scale}%"),
                                        );
                                    }
                                });
                            self.preview_settings.transparent =
                                self.inspection.viewer.background != 3;
                            self.preview_settings.inspect = self.inspection.viewer.show_bounds;
                            self.preview_settings.pixels =
                                self.inspection.viewer.show_pixel_inspector;
                            ui.weak("Backend");
                            egui::ComboBox::from_id_salt("native-backend")
                                .width(88.0)
                                .selected_text(format!(
                                    "{:?}",
                                    self.preview_settings.backend.unwrap_or_default()
                                ))
                                .show_ui(ui, |ui| {
                                    for backend in [
                                        motion_render::BackendPreference::Automatic,
                                        motion_render::BackendPreference::Cpu,
                                        motion_render::BackendPreference::Gpu,
                                    ] {
                                        ui.add_enabled_ui(
                                            backend == motion_render::BackendPreference::Cpu
                                                || self.gpu_available,
                                            |ui| {
                                                ui.selectable_value(
                                                    &mut self.preview_settings.backend,
                                                    Some(backend),
                                                    format!("{backend:?}"),
                                                );
                                            },
                                        );
                                    }
                                });
                            ui.separator();
                            if theme::icon_toggle(
                                ui,
                                if self.playback.preview_muted {
                                    icons::SPEAKER_SLASH
                                } else {
                                    icons::SPEAKER_HIGH
                                },
                                if self.playback.preview_muted {
                                    "Unmute preview audio"
                                } else {
                                    "Mute preview audio"
                                },
                                self.playback.preview_muted,
                            ) {
                                self.playback.preview_muted = !self.playback.preview_muted;
                                let playing = self.playback.transport.anchor.is_some();
                                let starting = self.playback.transport.starting;
                                self.pause();
                                if playing {
                                    self.playback.transport.anchor =
                                        Some((Instant::now(), self.playback.transport.frame));
                                    self.start_audio();
                                } else {
                                    self.playback.transport.starting = starting;
                                }
                            }
                            let dark = context.style().visuals.dark_mode;
                            if theme::icon_toggle(
                                ui,
                                if dark { icons::SUN } else { icons::MOON },
                                if dark {
                                    "Switch to light theme"
                                } else {
                                    "Switch to dark theme"
                                },
                                false,
                            ) {
                                theme::apply(context, !dark);
                            }
                            if old.for_interaction(false, false, Duration::from_secs(1))
                                != self.preview_settings.for_interaction(
                                    false,
                                    false,
                                    Duration::from_secs(1),
                                )
                            {
                                self.pause();
                                self.playback.last_interaction = Instant::now();
                                self.preview_state.cancel_request();
                            }
                            if self.close_when_idle {
                                ui.label("Waiting for export cleanup...");
                            }
                        },
                    );
                });
            });
        egui::TopBottomPanel::bottom("status")
            .exact_height(28.0)
            .frame(
                egui::Frame::side_top_panel(&context.style())
                    .inner_margin(egui::Margin::symmetric(8, 3)),
            )
            .show(context, |ui| {
                preview_status(
                    ui,
                    &self.observations.backend_status,
                    self.preview_state.displayed.map(|key| key.frame),
                    self.preview_state.pending,
                );
            });
        self.timeline(context);
        self.export_panel(context);
        self.preview_panel(context);
        self.request_preview();
        if self.playback.transport.playing() {
            context.request_repaint_after(Duration::from_millis(16));
        } else if self.playback.scrub_dragging
            || self.playback.last_interaction.elapsed() < Duration::from_millis(150)
        {
            context.request_repaint_after(Duration::from_millis(20));
        } else if self.exports.export.is_some() {
            context.request_repaint_after(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_status(
        context: &egui::Context,
        width: f32,
        frame: Option<u32>,
        pending: bool,
        pointer: Option<egui::Pos2>,
    ) -> (egui::Response, egui::FullOutput) {
        let mut response = None;
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 240.0),
                )),
                events: pointer.into_iter().map(egui::Event::PointerMoved).collect(),
                ..Default::default()
            },
            |context| {
                egui::TopBottomPanel::bottom("status")
                    .exact_height(28.0)
                    .frame(
                        egui::Frame::side_top_panel(&context.style())
                            .inner_margin(egui::Margin::symmetric(8, 3)),
                    )
                    .show(context, |ui| {
                        response = Some(preview_status(
                            ui,
                            "GPU (Apple M3) with a deliberately long backend diagnostic",
                            frame,
                            pending,
                        ));
                    });
            },
        );
        (response.unwrap(), output)
    }

    #[test]
    fn sidebar_defaults_keep_preview_space_at_narrow_widths() {
        assert_eq!(sidebar_defaults(640.0), (false, false));
        assert_eq!(sidebar_defaults(900.0), (false, false));
        assert_eq!(sidebar_defaults(1280.0), (false, true));
        for width in [900.0, 1119.0, 1120.0, 1240.0, 1440.0] {
            let (scenes, output) = sidebar_defaults(width);
            let remaining =
                width - if scenes { 184.0 } else { 0.0 } - if output { 252.0 } else { 0.0 } - 24.0;
            assert!(remaining >= 640.0);
        }
    }

    #[test]
    fn status_counter_rect_and_id_stay_stable_during_playback() {
        for width in [360.0, 800.0, 1440.0] {
            let context = egui::Context::default();
            let mut fonts = egui::FontDefinitions::default();
            egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
            context.set_fonts(fonts);
            let (first, _) = render_status(&context, width, None, false, None);
            for (frame, pending) in [
                (None, false),
                (Some(9), true),
                (Some(10), false),
                (Some(999), true),
                (Some(1000), false),
                (Some(u32::MAX), true),
            ] {
                let (actual, output) =
                    render_status(&context, width, frame, pending, Some(first.rect.center()));
                assert_eq!(
                    (actual.rect, actual.id),
                    (first.rect, first.id),
                    "width {width}, pending {pending}"
                );
                assert_eq!(actual.rect.size(), egui::vec2(128.0, 22.0));
                assert!(actual.hovered());
                for shape in output.shapes {
                    if let egui::Shape::Text(text) = shape.shape {
                        let bounds = text.visual_bounding_rect();
                        assert!(
                            bounds.left() >= 0.0 && bounds.right() <= width,
                            "text outside status bar at width {width}: {bounds:?}"
                        );
                        if !actual.rect.contains(bounds.center()) {
                            assert!(
                                !bounds.intersects(actual.rect),
                                "text {:?} at {bounds:?} overlaps frame counter {:?}",
                                text.galley.text(),
                                actual.rect
                            );
                        }
                    }
                }
            }
        }
    }
}
