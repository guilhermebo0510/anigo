//! Ícones vetoriais (Phosphor, fonte embutida) — regra do projeto: nada de emoji.

pub use egui_phosphor::regular::*;

/// Registra a fonte Phosphor nas fontes do egui.
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);
}
