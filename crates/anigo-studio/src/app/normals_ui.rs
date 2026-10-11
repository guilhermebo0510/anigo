//! Painel de Normais (cel shading anime): transferência de normais de proxy, sharp/seam, contorno.
use super::StudioApp;

impl StudioApp {
    /// Seção "Normais" do Inspector no modo Edição.
    pub(crate) fn inspector_normals(&mut self, _ui: &mut egui::Ui) {}
}
