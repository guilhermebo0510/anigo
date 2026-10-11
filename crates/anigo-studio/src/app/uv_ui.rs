//! Editor de UV (painel 2D) do Modo Edição. Hook chamado por `viewport_ui.rs`/`app.rs`.
use super::StudioApp;

impl StudioApp {
    /// Painel do Inspector (seção UV): costuras, unwrap, pack, projeções, métricas.
    pub(crate) fn inspector_uv(&mut self, _ui: &mut egui::Ui) {}

    /// Painel inferior/lateral com o editor 2D de UV (desenha ilhas, seleção sincronizada). Devolve a altura usada.
    pub(crate) fn uv_editor_panel(&mut self, _ui: &mut egui::Ui) {}
}
