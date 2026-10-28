//! The look: the palette of the original tools on a flat, rounded, card-based layout.

use egui::{Color32, CornerRadius, Frame, Margin, Response, RichText, Stroke, Ui, Vec2};

pub const BG_DARK: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x2e);
pub const BG_LIGHT: Color32 = Color32::from_rgb(0x16, 0x21, 0x3e);
pub const PRIMARY: Color32 = Color32::from_rgb(0x0f, 0x4c, 0x75);
pub const ACCENT: Color32 = Color32::from_rgb(0xe9, 0x45, 0x60);
pub const TEAL: Color32 = Color32::from_rgb(0x00, 0xb7, 0xc2);
pub const LIST_BG: Color32 = Color32::from_rgb(0x1f, 0x40, 0x68);
pub const FIELD_BG: Color32 = Color32::from_rgb(0x22, 0x31, 0x4f);
pub const PURPLE: Color32 = Color32::from_rgb(0x6a, 0x09, 0x7d);
pub const BLUE: Color32 = Color32::from_rgb(0x34, 0x98, 0xdb);
pub const VIOLET: Color32 = Color32::from_rgb(0x9b, 0x59, 0xb6);
pub const ORANGE: Color32 = Color32::from_rgb(0xd3, 0x54, 0x00);
pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe6, 0xe6);
pub const MUTED: Color32 = Color32::from_rgb(0x9a, 0xa5, 0xb1);
/// Hairlines around cards and fields: a lighter step of the card blue.
pub const LINE: Color32 = Color32::from_rgb(0x2b, 0x3a, 0x63);

/// Corner radius of cards.
pub const CARD_RADIUS: u8 = 14;
/// Corner radius of buttons and fields.
pub const CONTROL_RADIUS: u8 = 10;

pub fn apply(ctx: &egui::Context) {
    // Always the dark look, whatever the system theme says (a light system theme used to turn
    // the labels dark on the dark panels).
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG_DARK;
    v.window_fill = BG_LIGHT;
    v.extreme_bg_color = FIELD_BG;
    v.faint_bg_color = Color32::from_rgb(0x1b, 0x36, 0x5a);
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = TEAL;
    v.selection.stroke.color = Color32::WHITE;
    v.window_corner_radius = CornerRadius::same(CARD_RADIUS);
    v.menu_corner_radius = CornerRadius::same(CONTROL_RADIUS);
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_shadow = egui::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(110),
    };
    v.handle_shape = egui::style::HandleShape::Circle;
    v.slider_trailing_fill = true;

    let text = Stroke::new(1.0, TEXT);
    let radius = CornerRadius::same(CONTROL_RADIUS);
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    w.noninteractive.fg_stroke = text;
    w.noninteractive.corner_radius = radius;
    w.inactive.bg_fill = FIELD_BG;
    w.inactive.weak_bg_fill = FIELD_BG;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = text;
    w.inactive.corner_radius = radius;
    w.hovered.bg_fill = lighten(FIELD_BG, 0.12);
    w.hovered.weak_bg_fill = lighten(FIELD_BG, 0.12);
    w.hovered.bg_stroke = Stroke::new(1.0, TEAL);
    w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = lighten(FIELD_BG, 0.22);
    w.active.weak_bg_fill = lighten(FIELD_BG, 0.22);
    w.active.bg_stroke = Stroke::new(1.0, TEAL);
    w.active.fg_stroke = Stroke::new(2.0, Color32::WHITE);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;
    w.open.bg_fill = lighten(FIELD_BG, 0.12);
    w.open.weak_bg_fill = lighten(FIELD_BG, 0.12);
    w.open.fg_stroke = text;
    w.open.corner_radius = radius;

    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        ctx.set_visuals_of(theme, v.clone());
        ctx.style_mut_of(theme, |s| {
            s.spacing.button_padding = egui::vec2(14.0, 7.0);
            s.spacing.item_spacing = egui::vec2(10.0, 8.0);
            s.spacing.interact_size.y = 30.0;
            s.spacing.slider_width = 150.0;
            s.spacing.slider_rail_height = 6.0;
            s.spacing.combo_width = 120.0;
            s.spacing.scroll.floating = true;
            s.spacing.scroll.bar_width = 8.0;
            for (style, size) in [
                (egui::TextStyle::Body, 14.5),
                (egui::TextStyle::Button, 14.5),
                (egui::TextStyle::Small, 12.0),
                (egui::TextStyle::Heading, 20.0),
            ] {
                if let Some(f) = s.text_styles.get_mut(&style) {
                    f.size = size;
                }
            }
        });
    }
}

/// `c` mixed towards white by `t` (0..1).
pub fn lighten(c: Color32, t: f32) -> Color32 {
    c.lerp_to_gamma(Color32::WHITE, t.clamp(0.0, 1.0))
}

/// `c` mixed towards black by `t` (0..1).
pub fn darken(c: Color32, t: f32) -> Color32 {
    c.lerp_to_gamma(Color32::BLACK, t.clamp(0.0, 1.0))
}

/// A rounded panel on the dark background: the building block of both tabs.
pub fn card() -> Frame {
    Frame::new()
        .fill(BG_LIGHT)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::same(14))
}

/// A card for a list (the lighter blue of the original lists).
pub fn list_card() -> Frame {
    card()
        .fill(LIST_BG)
        .stroke(Stroke::new(1.0, lighten(LIST_BG, 0.12)))
        .inner_margin(Margin::same(8))
}

/// The frame of a side/top/bottom panel: the page background with a gutter around its cards.
pub fn page(margin: i8) -> Frame {
    Frame::new()
        .fill(BG_DARK)
        .inner_margin(Margin::same(margin))
}

/// A card heading: a small teal bar, then the title.
pub fn section(ui: &mut Ui, title: &str) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::new(4.0, 18.0), egui::Sense::hover());
        ui.painter().rect_filled(r, 2.0, TEAL);
        ui.label(RichText::new(title).strong().size(15.5));
    });
}

fn styled_button(ui: &mut Ui, text: RichText, fill: Color32, min: Vec2) -> Response {
    ui.scope(|ui| {
        let w = &mut ui.visuals_mut().widgets;
        w.inactive.weak_bg_fill = fill;
        w.inactive.bg_fill = fill;
        w.inactive.bg_stroke = Stroke::NONE;
        w.hovered.weak_bg_fill = lighten(fill, 0.14);
        w.hovered.bg_fill = lighten(fill, 0.14);
        w.hovered.bg_stroke = Stroke::new(1.0, lighten(fill, 0.4));
        w.active.weak_bg_fill = darken(fill, 0.15);
        w.active.bg_fill = darken(fill, 0.15);
        w.active.bg_stroke = Stroke::NONE;
        ui.add(
            egui::Button::new(text.color(Color32::WHITE).strong())
                .min_size(min)
                .corner_radius(CornerRadius::same(CONTROL_RADIUS)),
        )
    })
    .inner
}

/// A filled button in one of the palette colours (lighter under the pointer).
pub fn button(ui: &mut Ui, text: &str, fill: Color32) -> Response {
    styled_button(ui, RichText::new(text), fill, Vec2::new(0.0, 32.0))
}

/// A button of a given size.
pub fn button_sized(ui: &mut Ui, size: Vec2, text: RichText, fill: Color32) -> Response {
    styled_button(ui, text, fill, size)
}

/// A full-width button.
pub fn wide_button(ui: &mut Ui, text: &str, fill: Color32) -> Response {
    let w = ui.available_width();
    styled_button(ui, RichText::new(text), fill, Vec2::new(w, 36.0))
}
