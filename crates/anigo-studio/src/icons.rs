//! Ícones vetoriais (Phosphor, fonte embutida) — regra do projeto: nada de emoji.

pub use egui_phosphor::regular::*;

/// Ícones PRÓPRIOS do ANIGO (workspaces e ferramentas), gerados por `brand/gen_icons.py` na fonte `AnigoIcons.ttf`.
pub use crate::icons_anigo as ani;

const ANIGO_ICONS_TTF: &[u8] = include_bytes!("../assets/fonts/AnigoIcons.ttf");

/// Registra a fonte Phosphor nas fontes do egui.
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    // variante preenchida (estrela/coração ativos) como família separada: `FontFamily::Name("phosphor-fill")`
    egui_phosphor::add_font_bytes_as_family(&mut fonts, "phosphor-fill", egui_phosphor::Variant::Fill.font_bytes());
    // fonte própria: pontos de código F100–F1FF (o Phosphor ocupa E000–EE82), como fallback da família proporcional
    fonts.font_data.insert("anigo-icons".to_owned(), std::sync::Arc::new(egui::FontData::from_static(ANIGO_ICONS_TTF)));
    fonts.families.entry(egui::FontFamily::Proportional).or_default().push("anigo-icons".to_owned());
    fonts.families.entry(egui::FontFamily::Name("anigo-icons".into())).or_default().push("anigo-icons".to_owned());
    ctx.set_fonts(fonts);
}
