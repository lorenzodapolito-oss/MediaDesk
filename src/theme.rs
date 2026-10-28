//! Colours of the original tools, applied to egui.

use egui::Color32;

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

pub fn apply(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG_DARK;
    v.window_fill = BG_LIGHT;
    v.extreme_bg_color = FIELD_BG;
    v.faint_bg_color = Color32::from_rgb(0x1b, 0x36, 0x5a);
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = TEAL;
    v.selection.stroke.color = Color32::WHITE;
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
    });
}

/// A filled button in one of the palette colours.
pub fn button(ui: &mut egui::Ui, text: &str, fill: Color32) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).color(Color32::WHITE).strong())
            .fill(fill)
            .corner_radius(8.0),
    )
}
