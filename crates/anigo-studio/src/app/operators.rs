//! Painel de operadores do modo Edição (ligados ao motor `anigo-modeling`).
use super::StudioApp;
use egui::RichText;

impl StudioApp {
    pub(crate) fn edit_hotkeys(&mut self, _ui: &mut egui::Ui) {}

    pub(crate) fn inspector_edit_operators(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(RichText::new("Operadores").strong())
            .default_open(true)
            .show(ui, |ui| {
                ui.label(RichText::new("(operadores ligados após a integração dos módulos do motor)").size(11.5));
            });
    }
}
