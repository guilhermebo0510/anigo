//! Tema escuro de estúdio (referência: Blender 4, Unreal 5, DaVinci Resolve).

use egui::{Color32, CornerRadius, Stroke};

pub const ACCENT: Color32 = Color32::from_rgb(0x8a, 0x6c, 0xff);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x5a, 0x48, 0xa8);
pub const BG_DEEP: Color32 = Color32::from_rgb(0x14, 0x14, 0x17);
pub const BG_PANEL: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x23);
pub const BG_WIDGET: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x31);
pub const BG_HOVER: Color32 = Color32::from_rgb(0x36, 0x36, 0x40);
pub const BORDER: Color32 = Color32::from_rgb(0x38, 0x38, 0x42);
pub const TEXT: Color32 = Color32::from_rgb(0xd8, 0xd8, 0xe0);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8c, 0x8c, 0x99);
/// Laranja de seleção (padrão Blender).
pub const SELECT: Color32 = Color32::from_rgb(0xff, 0x9a, 0x2e);

pub fn apply(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG_PANEL;
    v.window_fill = BG_PANEL;
    v.extreme_bg_color = BG_DEEP;
    v.faint_bg_color = Color32::from_rgb(0x23, 0x23, 0x29);
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;
    let r = CornerRadius::same(4);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = r;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.bg_fill = BG_WIDGET;
    v.widgets.inactive.weak_bg_fill = BG_WIDGET;
    v.widgets.hovered.bg_fill = BG_HOVER;
    v.widgets.hovered.weak_bg_fill = BG_HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT_DIM);
    v.widgets.active.bg_fill = ACCENT_DIM;
    v.widgets.active.weak_bg_fill = ACCENT_DIM;
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    v.window_corner_radius = CornerRadius::same(6);
    ctx.set_visuals(v);

    ctx.global_style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(6.0, 5.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style.spacing.slider_width = 150.0;
    });
}
