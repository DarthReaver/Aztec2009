//! Maya 2009 inspired look: classic light-gray chrome, black text, sunken input fields.

use eframe::egui::{self, Color32, FontFamily, FontId, CornerRadius, Stroke, TextStyle};

pub const CHROME: Color32 = Color32::from_rgb(0xb4, 0xb4, 0xb4);
pub const CHROME_DARK: Color32 = Color32::from_rgb(0x96, 0x96, 0x96);
pub const CHROME_LIGHT: Color32 = Color32::from_rgb(0xd2, 0xd2, 0xd2);
pub const FIELD: Color32 = Color32::from_rgb(0xdc, 0xdc, 0xdc);
pub const LIST_BG: Color32 = Color32::from_rgb(0xc4, 0xc4, 0xc4);
pub const TEXT: Color32 = Color32::from_rgb(0x10, 0x10, 0x10);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x55, 0x55, 0x55);
pub const SELECT: Color32 = Color32::from_rgb(0x4f, 0x7f, 0xc4);
pub const KEYED: Color32 = Color32::from_rgb(0xd9, 0x8a, 0x7a);
pub const CONNECTED: Color32 = Color32::from_rgb(0xe0, 0xd0, 0x70);
pub const BORDER: Color32 = Color32::from_rgb(0x5a, 0x5a, 0x5a);
pub const VIEW_BG: [f32; 3] = [0.40, 0.40, 0.40];

pub fn apply(ctx: &egui::Context) {
    let mut v = egui::Visuals::light();
    v.panel_fill = CHROME;
    v.window_fill = CHROME;
    v.extreme_bg_color = FIELD;
    v.faint_bg_color = CHROME_LIGHT;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = SELECT;
    v.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    v.window_corner_radius = CornerRadius::same(2);
    v.menu_corner_radius = CornerRadius::same(0);
    v.window_shadow = egui::epaint::Shadow { offset: [2, 3], blur: 6, spread: 0, color: Color32::from_black_alpha(70) };
    v.popup_shadow = v.window_shadow;
    let r = CornerRadius::same(2);
    v.widgets.noninteractive.bg_fill = CHROME;
    v.widgets.noninteractive.weak_bg_fill = CHROME;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, CHROME_DARK);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.bg_fill = Color32::from_rgb(0xc9, 0xc9, 0xc9);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0xc9, 0xc9, 0xc9);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(0x70, 0x70, 0x70));
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.bg_fill = Color32::from_rgb(0xe2, 0xe2, 0xe2);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xe2, 0xe2, 0xe2);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(0x30, 0x30, 0x30));
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, Color32::BLACK);
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.bg_fill = Color32::from_rgb(0x9a, 0x9a, 0x9a);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(0x9a, 0x9a, 0x9a);
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, Color32::BLACK);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::BLACK);
    v.widgets.active.corner_radius = r;
    v.widgets.open = v.widgets.active;
    ctx.set_visuals(v);

    let mut st = (*ctx.style()).clone();
    st.spacing.item_spacing = egui::vec2(4.0, 3.0);
    st.spacing.button_padding = egui::vec2(6.0, 2.0);
    st.spacing.interact_size.y = 18.0;
    st.spacing.menu_margin = egui::Margin::symmetric(2, 2);
    st.spacing.window_margin = egui::Margin::same(6);
    st.spacing.scroll.bar_width = 8.0;
    st.text_styles = [
        (TextStyle::Small, FontId::new(10.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(12.5, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(12.5, FontFamily::Proportional)),
        (TextStyle::Heading, FontId::new(15.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
    ]
    .into();
    ctx.set_style(st);
}
