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

pub(crate) const ACCENT: Color32 = Color32::from_rgb(31, 169, 140);

pub(crate) fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            accent: ACCENT,
            accent_text: Color32::from_rgb(124, 224, 197),
            canvas: Color32::from_gray(16),
            lane: Color32::from_gray(33),
            ruler_text: Color32::from_gray(150),
            ruler_tick: Color32::from_gray(78),
            clip: [
                Color32::from_rgb(45, 112, 94),
                Color32::from_rgb(52, 98, 140),
                Color32::from_rgb(122, 86, 150),
            ],
            transition: Color32::from_rgba_unmultiplied(232, 176, 64, 190),
            playhead: Color32::from_rgb(255, 99, 88),
            range: Color32::from_rgb(232, 176, 64),
            cache: Color32::from_rgb(88, 178, 142),
            waveform: Color32::from_rgb(142, 150, 214),
            cue: Color32::from_rgb(255, 214, 90),
            error: Color32::from_rgb(255, 130, 130),
            success: Color32::from_rgb(120, 216, 160),
            overlay: Color32::from_black_alpha(190),
        }
    } else {
        Palette {
            accent: Color32::from_rgb(17, 124, 103),
            accent_text: Color32::from_rgb(14, 110, 90),
            canvas: Color32::from_gray(226),
            lane: Color32::from_gray(238),
            ruler_text: Color32::from_gray(98),
            ruler_tick: Color32::from_gray(170),
            clip: [
                Color32::from_rgb(74, 160, 134),
                Color32::from_rgb(92, 142, 200),
                Color32::from_rgb(160, 124, 190),
            ],
            transition: Color32::from_rgba_unmultiplied(230, 160, 40, 200),
            playhead: Color32::from_rgb(220, 60, 52),
            range: Color32::from_rgb(200, 130, 20),
            cache: Color32::from_rgb(52, 150, 112),
            waveform: Color32::from_rgb(92, 100, 180),
            cue: Color32::from_rgb(214, 150, 0),
            error: Color32::from_rgb(190, 40, 52),
            success: Color32::from_rgb(17, 110, 87),
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

/// Applies colors, spacing and typography. Safe to call repeatedly for theme switches.
pub(crate) fn apply(context: &egui::Context, dark: bool) {
    let colors = palette(dark);
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let (panel, window, field, faint, line, idle, hover, press) = if dark {
        (
            Color32::from_gray(27),
            Color32::from_gray(34),
            Color32::from_gray(19),
            Color32::from_gray(33),
            Color32::from_gray(46),
            Color32::from_gray(44),
            Color32::from_gray(58),
            Color32::from_gray(70),
        )
    } else {
        (
            Color32::from_gray(246),
            Color32::from_gray(252),
            Color32::from_gray(255),
            Color32::from_gray(238),
            Color32::from_gray(218),
            Color32::from_gray(232),
            Color32::from_gray(222),
            Color32::from_gray(208),
        )
    };
    visuals.panel_fill = panel;
    visuals.window_fill = window;
    visuals.extreme_bg_color = field;
    visuals.faint_bg_color = faint;
    visuals.window_stroke = Stroke::new(1.0_f32, line);
    visuals.window_corner_radius = rounded(10);
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
        state.corner_radius = rounded(6);
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::NONE;
    }
    visuals.widgets.noninteractive.corner_radius = rounded(6);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, colors.accent.gamma_multiply(0.7));
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, colors.accent);
    visuals.slider_trailing_fill = true;
    context.set_visuals(visuals);
    context.style_mut(|style| {
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
    let mut text = RichText::new(icon).size(17.0);
    if selected {
        text = text.color(palette_for(ui).accent_text);
    }
    ui.add_sized(size, egui::Button::new(text).selected(selected))
        .on_hover_text(label)
        .clicked()
}

/// Filled call-to-action button.
pub(crate) fn primary_button(ui: &mut egui::Ui, text: String, size: egui::Vec2) -> egui::Response {
    ui.add_sized(
        size,
        egui::Button::new(RichText::new(text).strong().color(Color32::WHITE))
            .fill(palette_for(ui).accent)
            .corner_radius(rounded(7)),
    )
}
