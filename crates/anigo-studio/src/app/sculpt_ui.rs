//! Escultura no viewport (Modo Escultura). Hooks chamados por `inspector.rs` e `viewport_ui.rs`.
use super::StudioApp;

impl StudioApp {
    /// Painel do Inspector no modo Escultura (pincéis, raio/força, simetria, falloff, máscara…).
    pub(crate) fn inspector_sculpt_panel(&mut self, _ui: &mut egui::Ui) {}

    /// Entrada do viewport no modo Escultura (traço do pincel, atalhos). Devolve `true` se consumiu a entrada
    /// (então o viewport não orbita/seleciona com o botão primário).
    pub(crate) fn sculpt_input(&mut self, _ui: &mut egui::Ui, _resp: &egui::Response, _rect: egui::Rect, _size: glam::Vec2) -> bool {
        false
    }

    /// Overlays do modo Escultura (círculo do pincel, máscara…).
    pub(crate) fn sculpt_overlay(&mut self, _ui: &mut egui::Ui, _rect: egui::Rect, _size: glam::Vec2) {}
}
