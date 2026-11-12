//! The look: the palette of the original tools on a flat, rounded, card-based layout.

use egui::{Color32, CornerRadius, Frame, Margin, Response, RichText, Stroke, Ui, Vec2};

pub const BG_DARK: Color32 = Color32::from_rgb(0x12, 0x17, 0x3a);
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
pub const TEXT: Color32 = Color32::from_rgb(0xd6, 0xed, 0xff);
pub const MUTED: Color32 = Color32::from_rgb(0x7f, 0xb2, 0xd6);
/// Hairlines around cards and fields: a lighter step of the card blue.
pub const LINE: Color32 = Color32::from_rgb(0x2b, 0x3a, 0x63);

/// Text on the coloured buttons: light yellow (never white).
pub const ON_COLOR: Color32 = Color32::from_rgb(0xff, 0xf1, 0xa8);
/// Light tint the hover colours move towards (never white).
const GLOW: Color32 = Color32::from_rgb(0xb8, 0xdc, 0xf5);
/// Deep navy the dark tints move towards (never black).
const DEEP: Color32 = Color32::from_rgb(0x05, 0x0b, 0x2a);

/// Corner radius of cards.
pub const CARD_RADIUS: u8 = 14;
/// Corner radius of buttons and fields.
pub const CONTROL_RADIUS: u8 = 10;

pub fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG_DARK;
    v.window_fill = BG_LIGHT;
    v.extreme_bg_color = FIELD_BG;
    v.faint_bg_color = Color32::from_rgb(0x1b, 0x36, 0x5a);
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = TEAL;
    v.selection.stroke.color = ON_COLOR;
    v.window_corner_radius = CornerRadius::same(CARD_RADIUS);
    v.menu_corner_radius = CornerRadius::same(CONTROL_RADIUS);
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_shadow = egui::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_rgba_unmultiplied(0x05, 0x0b, 0x2a, 110),
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
    w.hovered.fg_stroke = Stroke::new(1.5, ON_COLOR);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = lighten(FIELD_BG, 0.22);
    w.active.weak_bg_fill = lighten(FIELD_BG, 0.22);
    w.active.bg_stroke = Stroke::new(1.0, TEAL);
    w.active.fg_stroke = Stroke::new(2.0, ON_COLOR);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;
    w.open.bg_fill = lighten(FIELD_BG, 0.12);
    w.open.weak_bg_fill = lighten(FIELD_BG, 0.12);
    w.open.fg_stroke = text;
    w.open.corner_radius = radius;
    colorize(&mut v, GLOW, Color32::from_rgb(0x0f, 0x2a, 0x4a));
    v
}

pub fn apply(ctx: &egui::Context) {
    // Always the dark look, whatever the system theme says (a light system theme used to turn
    // the labels dark on the dark panels).
    ctx.set_theme(egui::ThemePreference::Dark);
    let v = visuals();
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

/// `c` mixed towards a light blue tint by `t` (0..1).
pub fn lighten(c: Color32, t: f32) -> Color32 {
    c.lerp_to_gamma(GLOW, t.clamp(0.0, 1.0))
}

/// `c` mixed towards deep navy by `t` (0..1).
pub fn darken(c: Color32, t: f32) -> Color32 {
    c.lerp_to_gamma(DEEP, t.clamp(0.0, 1.0))
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
            egui::Button::new(text.color(ON_COLOR).strong())
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

/// Bianco e grigio non devono esistere: ogni colore neutro (anche quelli che egui mette di
/// default, come il testo dei pulsanti, le ombre o i bordi) prende una tinta.
/// I chiari vanno verso `light`, gli scuri verso `dark`.
pub fn colorize(v: &mut egui::Visuals, light: Color32, dark: Color32) {
    fn fix(c: &mut Color32, light: Color32, dark: Color32) {
        if !is_neutral(*c) {
            return;
        }
        let [r, g, b, a] = c.to_srgba_unmultiplied();
        let lum = (u32::from(r) * 3 + u32::from(g) * 6 + u32::from(b)) / 10;
        let (tone, t) = if lum >= 128 {
            (light, 0.85)
        } else {
            (dark, 0.9)
        };
        let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
        let [tr, tg, tb, _] = tone.to_srgba_unmultiplied();
        *c = Color32::from_rgba_unmultiplied(mix(r, tr), mix(g, tg), mix(b, tb), a);
    }
    for c in [
        &mut v.panel_fill,
        &mut v.window_fill,
        &mut v.extreme_bg_color,
        &mut v.faint_bg_color,
        &mut v.code_bg_color,
        &mut v.window_stroke.color,
        &mut v.window_shadow.color,
        &mut v.popup_shadow.color,
        &mut v.selection.bg_fill,
        &mut v.selection.stroke.color,
        &mut v.hyperlink_color,
        &mut v.text_cursor.stroke.color,
        &mut v.warn_fg_color,
        &mut v.error_fg_color,
    ] {
        fix(c, light, dark);
    }
    for c in [
        &mut v.override_text_color,
        &mut v.weak_text_color,
        &mut v.text_edit_bg_color,
    ]
    .into_iter()
    .flatten()
    {
        fix(c, light, dark);
    }
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        fix(&mut w.bg_fill, light, dark);
        fix(&mut w.weak_bg_fill, light, dark);
        fix(&mut w.bg_stroke.color, light, dark);
        fix(&mut w.fg_stroke.color, light, dark);
    }
}

/// Neutro = bianco, grigio o quasi (i canali RGB differiscono di meno di 24).
pub fn is_neutral(c: Color32) -> bool {
    let [r, g, b, a] = c.to_srgba_unmultiplied().map(i32::from);
    // Un colore del tutto trasparente non si vede: non è né bianco né grigio.
    a > 0 && r.max(g).max(b) - r.min(g).min(b) < 24
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_white_or_gray_anywhere_in_the_visuals() {
        let v = visuals();
        let mut named: Vec<(&str, Color32)> = vec![
            ("panel", v.panel_fill),
            ("window", v.window_fill),
            ("faint", v.faint_bg_color),
            ("extreme", v.extreme_bg_color),
            ("code", v.code_bg_color),
            ("window_stroke", v.window_stroke.color),
            ("window_shadow", v.window_shadow.color),
            ("popup_shadow", v.popup_shadow.color),
            ("selection", v.selection.bg_fill),
            ("selection_stroke", v.selection.stroke.color),
            ("hyperlink", v.hyperlink_color),
            ("cursor", v.text_cursor.stroke.color),
            ("warn", v.warn_fg_color),
            ("error", v.error_fg_color),
        ];
        for (n, c) in [
            ("text", v.override_text_color),
            ("weak_text", v.weak_text_color),
            ("text_edit", v.text_edit_bg_color),
        ] {
            if let Some(c) = c {
                named.push((n, c));
            }
        }
        for (n, w) in [
            ("noninteractive", v.widgets.noninteractive),
            ("inactive", v.widgets.inactive),
            ("hovered", v.widgets.hovered),
            ("active", v.widgets.active),
            ("open", v.widgets.open),
        ] {
            named.push((n, w.bg_fill));
            named.push((n, w.weak_bg_fill));
            named.push((n, w.bg_stroke.color));
            named.push((n, w.fg_stroke.color));
        }
        for (n, c) in named {
            assert!(!is_neutral(c), "{n}: {c:?} è bianco o grigio");
        }
    }

    #[test]
    fn palette_constants_are_never_white_or_gray() {
        for (n, c) in [
            ("BG_DARK", BG_DARK),
            ("BG_LIGHT", BG_LIGHT),
            ("LIST_BG", LIST_BG),
            ("FIELD_BG", FIELD_BG),
            ("LINE", LINE),
            ("PRIMARY", PRIMARY),
            ("TEXT", TEXT),
            ("MUTED", MUTED),
            ("ON_COLOR", ON_COLOR),
        ] {
            assert!(!is_neutral(c), "{n}: {c:?}");
        }
    }
}
