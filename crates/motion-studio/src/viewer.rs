//! Preview viewer state (zoom, pan, background, overlays), the inspector panel, and the preview
//! canvas with its comparison, guide, node-inspection and error overlays.

use crate::audio::preview::AudioStatus;
use crate::theme;
use eframe::egui::{self, Color32, RichText};
use egui_phosphor::regular as icons;
use std::sync::Arc;

mod stats;

pub(crate) struct Viewer {
    pub zoom: Option<f32>,
    pub pan: egui::Vec2,
    pub background: usize,
    pub show_safe_area: bool,
    pub show_grid: bool,
    pub show_bounds: bool,
    pub show_pixel_inspector: bool,
    pub selected: Option<usize>,
    pub reference: Option<Arc<egui::ColorImage>>,
    pub comparison_split: f32,

    pub reference_texture: Option<egui::TextureHandle>,

    pub diagnostic_focus: Option<String>,

    checker: Option<(bool, egui::TextureHandle)>,
}

impl Default for Viewer {
    fn default() -> Self {
        Self {
            zoom: None,
            pan: egui::Vec2::ZERO,
            background: 0,
            show_safe_area: false,
            show_grid: false,
            show_bounds: false,
            show_pixel_inspector: false,
            selected: None,
            reference: None,
            comparison_split: 0.5,
            reference_texture: None,
            diagnostic_focus: None,
            checker: None,
        }
    }
}

impl Viewer {
    pub fn tools(
        &mut self,
        ui: &mut egui::Ui,
        context: &egui::Context,
        pixels: Option<&Arc<egui::ColorImage>>,
    ) {
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("view-zoom")
                .width(64.0)
                .selected_text(
                    self.zoom
                        .map_or("Fit".into(), |zoom| format!("{:.0}%", zoom * 100.0)),
                )
                .show_ui(ui, |ui| {
                    if ui.selectable_value(&mut self.zoom, None, "Fit").changed() {
                        self.pan = egui::Vec2::ZERO;
                    }
                    for zoom in [0.5, 1.0, 2.0] {
                        ui.selectable_value(
                            &mut self.zoom,
                            Some(zoom),
                            format!("{:.0}%", zoom * 100.0),
                        );
                    }
                });
            egui::ComboBox::from_id_salt("view-background")
                .width(104.0)
                .selected_text(["Checkerboard", "Dark", "White", "Scene"][self.background])
                .show_ui(ui, |ui| {
                    for (index, label) in ["Checkerboard", "Dark", "White", "Scene"]
                        .into_iter()
                        .enumerate()
                    {
                        ui.selectable_value(&mut self.background, index, label);
                    }
                });
            ui.separator();
            for (icon, label, value) in [
                (
                    icons::FRAME_CORNERS,
                    "Title and action safe areas",
                    &mut self.show_safe_area,
                ),
                (icons::GRID_FOUR, "Grid", &mut self.show_grid),
                (icons::BOUNDING_BOX, "Node bounds", &mut self.show_bounds),
                (
                    icons::EYEDROPPER,
                    "Pixel inspector",
                    &mut self.show_pixel_inspector,
                ),
            ] {
                if theme::icon_toggle(ui, icon, label, *value) {
                    *value = !*value;
                }
            }
            if ui
                .add_enabled(
                    self.show_pixel_inspector && pixels.is_some_and(|image| image.size != [1, 1]),
                    egui::Button::new(icons::COPY),
                )
                .on_hover_text("Capture reference frame")
                .clicked()
            {
                self.reference = pixels.cloned();
                self.reference_texture = self.reference.as_ref().map(|image| {
                    context.load_texture(
                        "reference-frame",
                        image.clone(),
                        egui::TextureOptions::LINEAR,
                    )
                });
            }
            if self.reference.is_some() {
                ui.add(
                    egui::Slider::new(&mut self.comparison_split, 0.0..=1.0)
                        .text("A/B")
                        .show_value(false),
                );
                if ui
                    .button(icons::X)
                    .on_hover_text("Clear reference")
                    .clicked()
                {
                    self.reference = None;
                    self.reference_texture = None;
                }
            }
        });
    }

    pub fn fitted(area: egui::Rect, size: egui::Vec2) -> f32 {
        (area.width() / size.x).min(area.height() / size.y)
    }

    pub fn bounds(&self, area: egui::Rect, size: egui::Vec2) -> egui::Rect {
        egui::Rect::from_center_size(
            area.center() + self.pan,
            size * self.zoom.unwrap_or_else(|| Self::fitted(area, size)),
        )
    }

    /// Applies pinch/Ctrl-wheel zoom around the pointer, keeping the image point
    /// under the cursor fixed, and trackpad scrolling as panning once zoomed.
    pub fn navigate(
        &mut self,
        area: egui::Rect,
        size: egui::Vec2,
        pointer: Option<egui::Pos2>,
        zoom_delta: f32,
        scroll: egui::Vec2,
    ) {
        if zoom_delta != 1.0
            && let Some(pointer) = pointer
        {
            let current = self.zoom.unwrap_or_else(|| Self::fitted(area, size));
            let next = (current * zoom_delta).clamp(0.05, 16.0);
            let ratio = next / current;
            let center = area.center() + self.pan;
            self.pan = pointer - (pointer - center) * ratio - area.center();
            self.zoom = Some(next);
        } else if self.zoom.is_some() && scroll != egui::Vec2::ZERO {
            self.pan += scroll;
        }
    }

    /// A tiny repeating texture whose cells are drawn once by the GPU instead of as
    /// hundreds of rectangles per frame.
    fn checker(&mut self, context: &egui::Context, dark: bool) -> egui::TextureId {
        if self.checker.as_ref().is_none_or(|entry| entry.0 != dark) {
            let (a, b) = if dark {
                (Color32::from_gray(48), Color32::from_gray(60))
            } else {
                (Color32::from_gray(214), Color32::from_gray(232))
            };
            self.checker = Some((
                dark,
                context.load_texture(
                    "checkerboard",
                    egui::ColorImage::new([2, 2], vec![a, b, b, a]),
                    egui::TextureOptions::NEAREST_REPEAT,
                ),
            ));
        }
        self.checker.as_ref().expect("checker texture").1.id()
    }
}

impl crate::app::StudioApp {
    pub(crate) fn inspector(&mut self, ui: &mut egui::Ui) {
        match self.inspection.inspector_tab {
            1 => {
                if let Some(node) = self
                    .inspection
                    .viewer
                    .selected
                    .and_then(|index| self.inspection.nodes.get(index))
                {
                    ui.label(egui::RichText::new(&node.path).strong());
                    ui.label(format!("Bounds: {:?}", node.bounds));
                    ui.label(format!("Transform: {:?}", node.transform));
                    ui.label(format!(
                        "Clipped: {} / {:?}",
                        node.clipped, node.clip_bounds
                    ));
                    ui.label(format!("Masked: {}", node.masked));
                    for filter in &node.filters {
                        ui.label(format!("{filter:?}"));
                    }
                }
                if self.inspection.nodes.is_empty() {
                    ui.weak("No nodes in this frame.");
                } else if self.inspection.viewer.selected.is_none() {
                    ui.weak("Turn on Node bounds in the viewer toolbar, then click a node.");
                }
                for (index, node) in self.inspection.nodes.iter().enumerate() {
                    if ui
                        .add(
                            egui::Button::new(&node.path)
                                .selected(self.inspection.viewer.selected == Some(index))
                                .wrap(),
                        )
                        .clicked()
                    {
                        self.inspection.viewer.selected = Some(index);
                    }
                }
            }
            2 => {
                let mut seek = None;
                if inspector_action(ui, icons::TRASH, "Clear diagnostics") {
                    self.observations.diagnostics.clear();
                }
                if self.observations.diagnostics.is_empty() {
                    ui.weak("No diagnostics.");
                }
                for (frame, message) in &self.observations.diagnostics {
                    if ui.button(format!("Frame {frame}")).clicked() {
                        seek = Some((*frame, message.clone()));
                    }
                    ui.add(egui::Label::new(message).wrap());
                    ui.separator();
                }
                if let Some((frame, message)) = seek {
                    self.seek(frame);
                    self.inspection.viewer.show_bounds = true;
                    self.inspection.viewer.diagnostic_focus = Some(message);
                }
            }
            _ => self.stats_panel(ui),
        }
    }
}

fn inspector_action(ui: &mut egui::Ui, icon: &str, label: &str) -> bool {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            theme::icon_toggle(ui, icon, label, false)
        })
        .inner
    })
    .inner
}

impl crate::app::StudioApp {
    pub(crate) fn preview_panel(&mut self, ui: &mut egui::Ui) {
        let context = &ui.ctx().clone();
        let colors = theme::palette(context.global_style().visuals.dark_mode);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::central_panel(&context.global_style())
                    .inner_margin(egui::Margin::symmetric(12, 8)),
            )
            .show(ui, |ui| {
                self.inspection
                    .viewer
                    .tools(ui, context, self.preview_pixels.as_ref());
                if let Some(error) = self
                    .preview_state
                    .error
                    .as_ref()
                    .filter(|_| !self.preview_state.displays_request())
                {
                    ui.add(egui::Label::new(RichText::new(error).color(colors.error)).wrap());
                }
                if matches!(self.audio_preview.status(), Some(AudioStatus::Preparing)) {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Preparing audio...");
                    });
                }
                if let Some(error) = &self.playback.audio_error {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("Audio: {error}")).color(colors.error),
                        )
                        .wrap(),
                    );
                }
                ui.add_space(8.0);
                self.preview_canvas(ui, context);
            });
    }

    fn preview_canvas(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        let colors = theme::palette(context.global_style().visuals.dark_mode);
        let available = ui.available_size().max(egui::vec2(1.0, 1.0));
        let (area, response) = ui.allocate_exact_size(available, egui::Sense::click_and_drag());
        ui.set_clip_rect(area.intersect(ui.clip_rect()));
        if response.dragged_by(egui::PointerButton::Middle)
            || (response.dragged() && context.input(|input| input.key_down(egui::Key::Space)))
        {
            self.inspection.viewer.pan += context.input(|input| input.pointer.delta());
        }
        let panning = response.dragged_by(egui::PointerButton::Middle)
            || (response.dragged() && context.input(|input| input.key_down(egui::Key::Space)));
        if panning {
            context.set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if response.hovered() && context.input(|input| input.key_down(egui::Key::Space)) {
            context.set_cursor_icon(egui::CursorIcon::Grab);
        }
        ui.painter().rect_filled(
            area,
            8.0,
            match self.inspection.viewer.background {
                1 => Color32::from_gray(18),
                2 => Color32::WHITE,
                _ => colors.canvas,
            },
        );
        let surface = self
            .texture
            .as_ref()
            .map(|texture| (texture.id(), texture.size_vec2()));
        #[cfg(feature = "gpu")]
        let surface = self
            .preview_only
            .as_ref()
            .map(|(id, texture)| {
                (
                    *id,
                    egui::vec2(texture.width() as f32, texture.height() as f32),
                )
            })
            .or(surface);
        if surface.is_none() {
            ui.put(
                egui::Rect::from_center_size(area.center(), egui::vec2(24.0, 24.0)),
                egui::Spinner::new(),
            );
            ui.painter().text(
                area.center() + egui::vec2(0.0, 28.0),
                egui::Align2::CENTER_CENTER,
                "Rendering first frame...",
                egui::FontId::proportional(12.0),
                ui.visuals().weak_text_color(),
            );
        }
        if let Some((id, size)) = surface {
            if response.hovered() {
                let (zoom, scroll) =
                    context.input(|input| (input.zoom_delta(), input.smooth_scroll_delta));
                self.inspection
                    .viewer
                    .navigate(area, size, response.hover_pos(), zoom, scroll);
            }
            let bounds = self.inspection.viewer.bounds(area, size);
            if self.inspection.viewer.background != 3 && self.inspection.viewer.background != 1 {
                let shadow = egui::Shadow {
                    offset: [0, 6],
                    blur: 24,
                    spread: 0,
                    color: Color32::from_black_alpha(if ui.visuals().dark_mode { 140 } else { 50 }),
                };
                ui.painter()
                    .with_clip_rect(area)
                    .add(shadow.as_shape(bounds, 2.0));
            }
            if self.inspection.viewer.background == 3 {
                let [red, green, blue, _] =
                    self.scene.composition.snapshot_background().to_rgba_float();
                ui.painter().rect_filled(
                    bounds,
                    0.0,
                    Color32::from_rgb(
                        (red * 255.0).round() as u8,
                        (green * 255.0).round() as u8,
                        (blue * 255.0).round() as u8,
                    ),
                );
            }
            if self.inspection.viewer.background == 0 && self.preview_settings.transparent {
                let cell = 16.0;
                let checker = self
                    .inspection
                    .viewer
                    .checker(context, ui.visuals().dark_mode);
                ui.painter().with_clip_rect(area).image(
                    checker,
                    bounds,
                    egui::Rect::from_min_max(
                        egui::Pos2::ZERO,
                        egui::pos2(
                            bounds.width() / (cell * 2.0),
                            bounds.height() / (cell * 2.0),
                        ),
                    ),
                    Color32::WHITE,
                );
            }
            ui.painter().with_clip_rect(area).image(
                id,
                bounds,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            self.preview_comparison(ui, area, bounds);
            self.preview_guides(ui, area, bounds);
            self.preview_node_inspection(ui, &response, size, bounds);
            self.preview_pixel_info(&response, bounds);
            self.preview_error_overlay(ui, bounds);
        }
    }

    fn preview_comparison(&self, ui: &mut egui::Ui, area: egui::Rect, bounds: egui::Rect) {
        if let Some(reference) = &self.inspection.viewer.reference_texture {
            let divide = bounds.left() + bounds.width() * self.inspection.viewer.comparison_split;
            let clipped = egui::Rect::from_min_max(bounds.min, egui::pos2(divide, bounds.bottom()))
                .intersect(area);
            ui.painter().with_clip_rect(clipped).image(
                reference.id(),
                bounds,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            ui.painter().line_segment(
                [
                    egui::pos2(divide, bounds.top()),
                    egui::pos2(divide, bounds.bottom()),
                ],
                egui::Stroke::new(1.0_f32, Color32::WHITE),
            );
        }
    }

    fn preview_guides(&self, ui: &mut egui::Ui, area: egui::Rect, bounds: egui::Rect) {
        let colors = theme::palette_for(ui);
        {
            let galley = ui.painter().layout_no_wrap(
                format!(
                    "{} x {}   {:.0}%",
                    self.clock.size.width,
                    self.clock.size.height,
                    bounds.width() / self.clock.size.width as f32 * 100.0
                ),
                egui::FontId::monospace(11.0),
                Color32::WHITE,
            );
            let chip = egui::Rect::from_min_size(
                egui::pos2(area.left() + 10.0, area.bottom() - galley.size().y - 18.0),
                galley.size() + egui::vec2(16.0, 8.0),
            );
            let painter = ui.painter().with_clip_rect(area);
            painter.rect_filled(chip, 6.0, colors.overlay);
            painter.galley(chip.min + egui::vec2(8.0, 4.0), galley, Color32::WHITE);
        }
        if self.inspection.viewer.show_safe_area {
            for fraction in [0.9, 0.8] {
                ui.painter().rect_stroke(
                    egui::Rect::from_center_size(bounds.center(), bounds.size() * fraction),
                    0.0,
                    egui::Stroke::new(1.0_f32, Color32::from_rgb(220, 185, 90)),
                    egui::StrokeKind::Inside,
                );
            }
        }
        if self.inspection.viewer.show_grid {
            for step in 1..10 {
                let fraction = step as f32 / 10.0;
                ui.painter().line_segment(
                    [
                        egui::pos2(bounds.left() + bounds.width() * fraction, bounds.top()),
                        egui::pos2(bounds.left() + bounds.width() * fraction, bounds.bottom()),
                    ],
                    egui::Stroke::new(0.5_f32, Color32::from_white_alpha(70)),
                );
                ui.painter().line_segment(
                    [
                        egui::pos2(bounds.left(), bounds.top() + bounds.height() * fraction),
                        egui::pos2(bounds.right(), bounds.top() + bounds.height() * fraction),
                    ],
                    egui::Stroke::new(0.5_f32, Color32::from_white_alpha(70)),
                );
            }
        }
    }

    fn preview_node_inspection(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        size: egui::Vec2,
        bounds: egui::Rect,
    ) {
        if self.inspection.viewer.show_bounds {
            let mapping = |rect: motion_core::Rect| {
                egui::Rect::from_min_max(
                    bounds.min
                        + egui::vec2(rect.left as f32 / size.x, rect.top as f32 / size.y)
                            * bounds.size(),
                    bounds.min
                        + egui::vec2(rect.right as f32 / size.x, rect.bottom as f32 / size.y)
                            * bounds.size(),
                )
            };
            for (index, node) in self.inspection.nodes.iter().enumerate() {
                let rect = mapping(node.bounds);
                if let Some(clip) = node.clip_bounds {
                    ui.painter().rect_stroke(
                        mapping(clip),
                        0.0,
                        egui::Stroke::new(1.0_f32, Color32::from_rgb(225, 185, 100)),
                        egui::StrokeKind::Inside,
                    );
                }
                ui.painter().rect_stroke(
                    rect,
                    0.0,
                    egui::Stroke::new(
                        if self.inspection.viewer.selected == Some(index) {
                            2.0_f32
                        } else {
                            0.5_f32
                        },
                        Color32::from_rgb(120, 220, 175),
                    ),
                    egui::StrokeKind::Inside,
                );
                if self.inspection.viewer.selected == Some(index) {
                    ui.painter().text(
                        rect.min,
                        egui::Align2::LEFT_TOP,
                        format!(
                            "{}{}{}",
                            node.path,
                            if node.masked { " / mask" } else { "" },
                            if node.filters.is_empty() {
                                ""
                            } else {
                                " / filters"
                            }
                        ),
                        egui::FontId::proportional(11.0),
                        Color32::WHITE,
                    );
                }
            }
            if response.clicked()
                && let Some(point) = response.interact_pointer_pos()
            {
                self.inspection.viewer.selected = self
                    .inspection
                    .nodes
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, node)| mapping(node.bounds).contains(point))
                    .map(|(index, _)| index);
                self.inspection.inspector_tab = 1;
                self.inspection.output_visible = Some(true);
            }
        }
    }

    fn preview_pixel_info(&self, response: &egui::Response, bounds: egui::Rect) {
        if self.inspection.viewer.show_pixel_inspector
            && let Some(point) = response.hover_pos().filter(|point| bounds.contains(*point))
            && let Some(image) = &self.preview_pixels
        {
            let relative = (point - bounds.min) / bounds.size();
            let column = ((relative.x * image.size[0] as f32) as usize).min(image.size[0] - 1);
            let row = ((relative.y * image.size[1] as f32) as usize).min(image.size[1] - 1);
            let rgba = image.pixels[row * image.size[0] + column].to_srgba_unmultiplied();
            let linear = rgba[..3]
                .iter()
                .map(|channel| motion_core::srgb_to_linear(f32::from(*channel) / 255.0))
                .collect::<Vec<_>>();
            response.clone().on_hover_text(format!(
                "({column}, {row})\nsRGB8 {rgba:?}\nDecoded linear {linear:.4?}"
            ));
        }
    }

    fn preview_error_overlay(&self, ui: &mut egui::Ui, bounds: egui::Rect) {
        if let Some(error) = self
            .preview_state
            .error
            .as_ref()
            .filter(|_| self.preview_state.displays_request())
        {
            let painter = ui.painter().with_clip_rect(bounds);
            let color = Color32::from_rgb(172, 45, 54);
            let galley = painter.layout(
                error.clone(),
                egui::FontId::proportional(13.0),
                color,
                (bounds.width() - 24.0).max(1.0),
            );
            let position = bounds.min + egui::vec2(8.0, 8.0);
            painter.rect_filled(
                egui::Rect::from_min_size(position, galley.size() + egui::vec2(8.0, 8.0)),
                3.0,
                Color32::from_rgba_unmultiplied(255, 235, 240, 240),
            );
            painter.galley(position + egui::vec2(4.0, 4.0), galley, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspector_actions_stay_at_the_top_right() {
        for width in [240.0, 340.0] {
            for (icon, label) in [
                (icons::COPY, "Copy stats to clipboard"),
                (icons::TRASH, "Clear diagnostics"),
            ] {
                let context = egui::Context::default();
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 800.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        egui::CentralPanel::default().show(ui, |ui| {
                            assert!(!inspector_action(ui, icon, label));
                        });
                    },
                );
                output.textures_delta.clear();
                let bounds = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == icon => {
                            Some(text.visual_bounding_rect())
                        }
                        _ => None,
                    })
                    .unwrap();
                assert!(bounds.left() > width - 40.0);
                assert!(bounds.right() <= width);
                assert!(bounds.bottom() < 50.0);
            }
        }
    }

    #[test]
    fn fit_zoom_and_pan_keep_pixel_coordinates_stable() {
        let mut viewer = Viewer::default();
        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 400.0));
        let size = egui::vec2(400.0, 200.0);
        assert_eq!(viewer.bounds(area, size), area);
        viewer.zoom = Some(1.0);
        viewer.pan = egui::vec2(10.0, 20.0);
        assert_eq!(viewer.bounds(area, size).size(), size);
        assert_eq!(
            viewer.bounds(area, size).center(),
            area.center() + viewer.pan
        );
    }
}
