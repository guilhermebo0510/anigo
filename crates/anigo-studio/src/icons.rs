//! Ícones vetoriais (Phosphor, fonte embutida) — regra do projeto: nada de emoji.

pub use egui_phosphor::regular::*;

/// Registra a fonte Phosphor nas fontes do egui.
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    // variante preenchida (estrela/coração ativos) como família separada: `FontFamily::Name("phosphor-fill")`
    egui_phosphor::add_font_bytes_as_family(&mut fonts, "phosphor-fill", egui_phosphor::Variant::Fill.font_bytes());
    ctx.set_fonts(fonts);
}
