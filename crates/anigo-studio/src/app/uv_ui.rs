//! Editor de UV do Modo Edição: painel 2D inferior (`uv_editor_panel`) e seção UV do Inspector
//! (`inspector_uv`). A lógica pura vive em `uv_ui/*` (testável); aqui fica o estado e a cola com a sessão.

mod actions;
mod cache;
mod canvas;
mod geom;
mod inspector;

pub(crate) use actions::{UvAction, UvParams};
pub(crate) use cache::UvCache;
pub(crate) use geom::{UvView, Xform};

use super::StudioApp;
use crate::modeling::{Mode, SnapshotPub};
use glam::Vec2;

/// Como as faces são coloridas no editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    /// Uma cor por ilha.
    #[default]
    Islands,
    /// Mapa de estiramento anisotrópico (`face_stretch`).
    Distortion,
    /// Densidade de texels relativa à média.
    Density,
}

/// Textura de fundo procedural para enxergar a cobertura/estiramento.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CheckerMode {
    #[default]
    Off,
    C8,
    C64,
}

impl CheckerMode {
    pub fn cells(self) -> Option<usize> {
        match self {
            CheckerMode::Off => None,
            CheckerMode::C8 => Some(8),
            CheckerMode::C64 => Some(64),
        }
    }
}

/// Transformação G/R/S (ou arrasto) em andamento no editor de UV.
pub(crate) struct UvModal {
    pub xf: Xform,
    /// `(face, canto, UV original)` de cada canto afetado.
    pub orig: Vec<(u32, usize, Vec2)>,
    pub before: SnapshotPub,
    /// Arrasto com o botão pressionado: confirma ao soltar.
    pub drag: bool,
    pub num_buf: String,
    /// Objeto e hash da geometria no início; se mudarem, a operação é descartada.
    pub obj_id: u32,
    pub geo_hash: u64,
}

pub(crate) struct UvState {
    pub view: UvView,
    pub view_inited: bool,
    pub fit_request: bool,
    pub color_mode: ColorMode,
    pub checker: CheckerMode,
    pub show_overlaps: bool,
    pub show_grid: bool,
    pub params: UvParams,
    /// Proporção (largura/altura) do viewport 3D, para "Projetar da vista".
    pub view_aspect: f32,
    pub modal: Option<UvModal>,
    pub box_start: Option<Vec2>,
    pub cache: Option<UvCache>,
    pub tex_checker: [Option<egui::TextureHandle>; 2],
    /// Última mensagem de erro de ação (mostrada no Inspector).
    pub last_error: Option<String>,
    /// Face sob o cursor no editor (tooltip e destaque).
    pub hover_face: Option<u32>,
    /// Última linha de rastreio (ANIGO_TRACE_UV), para só imprimir mudanças.
    pub trace_last: String,
}

impl Default for UvState {
    fn default() -> Self {
        Self {
            view: UvView::default(),
            view_inited: false,
            fit_request: false,
            color_mode: ColorMode::default(),
            checker: CheckerMode::default(),
            show_overlaps: true,
            show_grid: true,
            params: UvParams::default(),
            view_aspect: 1.6,
            modal: None,
            box_start: None,
            cache: None,
            tex_checker: [None, None],
            last_error: None,
            hover_face: None,
            trace_last: String::new(),
        }
    }
}

impl StudioApp {
    /// Executa uma ação de UV com mensagem amigável na barra de status. Devolve se teve sucesso.
    pub(crate) fn run_uv_action(&mut self, a: UvAction) -> bool {
        if self.modeling.mode != Mode::Edit {
            self.modeling.status = "UV: entre no Modo Edição (Tab)".into();
            return false;
        }
        let view_proj = self.modeling.active_object().map(|o| {
            let aspect = self.uv.view_aspect.max(0.1);
            self.camera.view_proj(aspect) * o.model_matrix()
        });
        let params = self.uv.params;
        match actions::run_action(&mut self.modeling, a, &params, view_proj) {
            Ok(msg) => {
                self.modeling.status = format!("{}: {msg}", a.label());
                self.uv.last_error = None;
                true
            }
            Err(e) => {
                let text = format!("{}: {e}", a.label());
                self.modeling.status = text.clone();
                self.uv.last_error = Some(text);
                false
            }
        }
    }

    /// Marca as costuras de cruz (complemento de uma árvore geradora) como seleção de arestas — usado
    /// por cenários e como atalho de teste.
    pub(crate) fn uv_seed_tree_seams(&mut self) {
        let Some(mesh) = self.modeling.active_object().map(|o| o.mesh.clone()) else {
            return;
        };
        let edges = actions::spanning_tree_seams(&mesh);
        self.modeling.set_select_mode(anigo_modeling::SelectMode::Edge);
        self.modeling.set_selection(anigo_modeling::Selection::from_edges(edges), None);
        self.run_uv_action(UvAction::MarkSeam);
    }

    /// Painel do Inspector (seção UV): costuras, unwrap, pack, projeções, métricas.
    pub(crate) fn inspector_uv(&mut self, ui: &mut egui::Ui) {
        self.inspector_uv_impl(ui);
    }

    /// Painel inferior com o editor 2D de UV (ilhas, estiramento, seleção sincronizada).
    pub(crate) fn uv_editor_panel(&mut self, ui: &mut egui::Ui) {
        self.uv_editor_impl(ui);
    }
}
