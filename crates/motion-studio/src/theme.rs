//! Shared Studio look: one accent, layered neutral surfaces and semantic colors.

use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke};

/// Colors used by custom-painted regions that egui visuals do not cover.
#[derive(Clone, Copy)]
pub(crate) struct Palette {
    pub accent: Color32,
    pub accent_text: Color32,
    pub canvas: Color32,
    pub lane: Color32,
    pub ruler_text: Color32,
    pub ruler_tick: Color32,
    pub clip: [Color32; 3],
    pub transition: Color32,
    pub playhead: Color32,
    pub range: Color32,
    pub cache: Color32,
    pub waveform: Color32,
    pub cue: Color32,
    pub error: Color32,
    pub success: Color32,
    pub overlay: Color32,
}

pub(crate) const ACCENT: Color32 = Color32::from_rgb(10, 132, 255);

pub(crate) fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            accent: ACCENT,
            accent_text: Color32::from_rgb(64, 156, 255),
            canvas: Color32::BLACK,
            lane: Color32::from_rgb(28, 28, 30),
            ruler_text: Color32::from_rgb(152, 152, 157),
            ruler_tick: Color32::from_rgb(72, 72, 74),
            clip: [
                Color32::from_rgb(48, 209, 88),
                Color32::from_rgb(10, 132, 255),
                Color32::from_rgb(191, 90, 242),
            ]
            .map(|color| color.gamma_multiply(0.7)),
            transition: Color32::from_rgba_unmultiplied(255, 159, 10, 190),
            playhead: Color32::from_rgb(255, 69, 58),
            range: Color32::from_rgb(255, 159, 10),
            cache: Color32::from_rgb(48, 209, 88),
            waveform: Color32::from_rgb(94, 92, 230),
            cue: Color32::from_rgb(255, 214, 10),
            error: Color32::from_rgb(255, 69, 58),
            success: Color32::from_rgb(48, 209, 88),
            overlay: Color32::from_black_alpha(190),
        }
    } else {
        Palette {
            accent: Color32::from_rgb(0, 122, 255),
            accent_text: Color32::from_rgb(0, 122, 255),
            canvas: Color32::from_rgb(229, 229, 234),
            lane: Color32::from_rgb(242, 242, 247),
            ruler_text: Color32::from_rgb(110, 110, 115),
            ruler_tick: Color32::from_rgb(199, 199, 204),
            clip: [
                Color32::from_rgb(52, 199, 89),
                Color32::from_rgb(0, 122, 255),
                Color32::from_rgb(175, 82, 222),
            ]
            .map(|color| color.gamma_multiply(0.85)),
            transition: Color32::from_rgba_unmultiplied(255, 149, 0, 200),
            playhead: Color32::from_rgb(255, 59, 48),
            range: Color32::from_rgb(255, 149, 0),
            cache: Color32::from_rgb(52, 199, 89),
            waveform: Color32::from_rgb(88, 86, 214),
            cue: Color32::from_rgb(255, 204, 0),
            error: Color32::from_rgb(255, 59, 48),
            success: Color32::from_rgb(36, 138, 61),
            overlay: Color32::from_black_alpha(170),
        }
    }
}

pub(crate) fn palette_for(ui: &egui::Ui) -> Palette {
    palette(ui.visuals().dark_mode)
}

fn rounded(radius: u8) -> CornerRadius {
    CornerRadius::same(radius)
}

/// Default text fonts with the regular Phosphor font as the first fallback.
pub(crate) fn font_definitions() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "phosphor".into(),
        egui::FontData::from_static(egui_phosphor::Variant::Regular.font_bytes()).into(),
    );
    let proportional = fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default();
    proportional.insert(proportional.len().min(1), "phosphor".into());
    let named = proportional.clone();
    fonts
        .families
        .insert(egui::FontFamily::Name("phosphor".into()), named);
    fonts
}

/// Applies colors, spacing and typography. Safe to call repeatedly for theme switches.
pub(crate) fn apply(context: &egui::Context, dark: bool) {
    let colors = palette(dark);
    context.set_theme(if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let (panel, window, field, faint, line, idle, hover, press) = if dark {
        (
            Color32::from_rgb(28, 28, 30),
            Color32::from_rgb(44, 44, 46),
            Color32::from_rgb(18, 18, 20),
            Color32::from_rgb(36, 36, 38),
            Color32::from_rgb(56, 56, 58),
            Color32::from_rgb(58, 58, 60),
            Color32::from_rgb(72, 72, 74),
            Color32::from_rgb(99, 99, 102),
        )
    } else {
        (
            Color32::from_rgb(242, 242, 247),
            Color32::WHITE,
            Color32::WHITE,
            Color32::from_rgb(235, 235, 240),
            Color32::from_rgb(209, 209, 214),
            Color32::from_rgb(229, 229, 234),
            Color32::from_rgb(216, 216, 220),
            Color32::from_rgb(199, 199, 204),
        )
    };
    visuals.panel_fill = panel;
    visuals.window_fill = window;
    visuals.extreme_bg_color = field;
    visuals.faint_bg_color = faint;
    visuals.window_stroke = Stroke::new(1.0_f32, line);
    visuals.window_corner_radius = rounded(12);
    visuals.menu_corner_radius = rounded(8);
    visuals.hyperlink_color = colors.accent_text;
    visuals.selection.bg_fill = colors.accent.gamma_multiply(if dark { 0.42 } else { 0.30 });
    visuals.selection.stroke = Stroke::new(1.0_f32, colors.accent_text);
    visuals.widgets.noninteractive.bg_fill = panel;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, line);
    for (state, fill) in [
        (&mut visuals.widgets.inactive, idle),
        (&mut visuals.widgets.hovered, hover),
        (&mut visuals.widgets.active, press),
        (&mut visuals.widgets.open, hover),
    ] {
        state.corner_radius = rounded(8);
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::NONE;
    }
    visuals.widgets.noninteractive.corner_radius = rounded(8);
    visuals.widgets.hovered.bg_stroke = Stroke::NONE;
    visuals.widgets.active.bg_stroke = Stroke::NONE;
    visuals.slider_trailing_fill = true;
    context.set_visuals(visuals);
    context.global_style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 4.0);
        style.spacing.interact_size = egui::vec2(36.0, 26.0);
        style.spacing.combo_width = 72.0;
        style.spacing.scroll = egui::style::ScrollStyle::floating();
        style.animation_time = 0.08;
        for (text, size) in [
            (egui::TextStyle::Small, 11.0),
            (egui::TextStyle::Body, 13.5),
            (egui::TextStyle::Button, 13.5),
            (egui::TextStyle::Monospace, 12.5),
            (egui::TextStyle::Heading, 17.0),
        ] {
            if let Some(font) = style.text_styles.get_mut(&text) {
                font.size = size;
            }
        }
    });
}

/// Small caps-style heading that separates groups of related controls.
pub(crate) fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(
        RichText::new(title.to_uppercase())
            .size(10.5)
            .strong()
            .color(ui.visuals().weak_text_color()),
    );
    ui.add_space(2.0);
}

/// Square icon button with a tooltip. Selected toggles are tinted with the accent.
pub(crate) fn icon_toggle(ui: &mut egui::Ui, icon: &str, label: &str, selected: bool) -> bool {
    icon_toggle_sized(ui, icon, label, selected, egui::vec2(30.0, 28.0))
}

/// [`icon_toggle`] with an explicit size, for rows of equal-width cells.
pub(crate) fn icon_toggle_sized(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    selected: bool,
    size: egui::Vec2,
) -> bool {
    let text = RichText::new(icon).size(17.0);
    let button = if selected {
        egui::Button::new(text.color(Color32::WHITE))
            .fill(palette_for(ui).accent)
            .corner_radius(rounded(8))
    } else {
        egui::Button::new(text)
    };
    ui.add_sized(size, button).on_hover_text(label).clicked()
}

/// Filled call-to-action button.
pub(crate) fn primary_button(ui: &mut egui::Ui, text: String, size: egui::Vec2) -> egui::Response {
    ui.add_sized(
        size,
        egui::Button::new(RichText::new(text).strong().color(Color32::WHITE))
            .fill(palette_for(ui).accent)
            .corner_radius(rounded(10)),
    )
}

/// iOS-style segmented control: a rounded track with a raised thumb under the selected icon.
/// Returns the newly clicked segment, if any.
pub(crate) fn segmented_control(
    ui: &mut egui::Ui,
    segments: &[(&str, &str)],
    selected: usize,
) -> Option<usize> {
    let dark = ui.visuals().dark_mode;
    let height = 32.0;
    let (track, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let track_fill = if dark {
        Color32::from_rgba_unmultiplied(118, 118, 128, 61)
    } else {
        Color32::from_rgba_unmultiplied(118, 118, 128, 31)
    };
    ui.painter().rect_filled(track, rounded(9), track_fill);
    let inset = 2.0;
    let inner = track.shrink(inset);
    let width = inner.width() / segments.len() as f32;
    let position = ui.ctx().animate_value_with_time(
        ui.scope_id().with("segment-thumb"),
        selected as f32,
        0.25,
    );
    let cell_at = |slot: f32| {
        egui::Rect::from_min_size(
            egui::pos2(inner.left() + width * slot, inner.top()),
            egui::vec2(width, inner.height()),
        )
    };
    let thumb = cell_at(position);
    ui.painter()
        .rect_filled(thumb, rounded(7), palette_for(ui).accent);
    let mut clicked = None;
    for (index, (icon, label)) in segments.iter().enumerate() {
        let cell = cell_at(index as f32);
        let response = ui
            .interact(
                cell,
                ui.scope_id().with(("segment", index)),
                egui::Sense::click(),
            )
            .on_hover_text(*label);
        if index != selected && response.hovered() {
            ui.painter().rect_filled(cell, rounded(7), track_fill);
        }
        let covered = (1.0 - (position - index as f32).abs()).clamp(0.0, 1.0);
        let idle_icon = if dark {
            Color32::from_rgb(174, 174, 178)
        } else {
            Color32::from_rgb(99, 99, 102)
        };
        let color = idle_icon.lerp_to_gamma(Color32::WHITE, covered);
        ui.painter().text(
            cell.center(),
            egui::Align2::CENTER_CENTER,
            icon,
            egui::FontId::proportional(17.0),
            color,
        );
        if response.clicked() {
            clicked = Some(index);
        }
    }
    clicked
}
