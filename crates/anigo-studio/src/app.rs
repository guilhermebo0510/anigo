//! `eframe::App` do ANIGO Studio: barra de workspaces, shelf de ferramentas, inspector,
//! viewport (Workbench wgpu) e Biblioteca.

use crate::cli::Cli;
use crate::hot::{dev_shader_path, ShaderWatcher};
use crate::i18n::{self, tr, Lang};
use crate::icons;
use crate::library;
use crate::library_ui::{HostAction, LibraryWorkspace};
use crate::modeling::{Mode, ModelingSession};
use crate::session::{SessionState, SESSION_VERSION};
use crate::theme;
use crate::viewport::OrbitCamera;
use crate::celview::CelPreview;
use crate::workbench::{GpuMesh, ViewSettings, Workbench};
use crate::workspace::WorkspaceId;
use egui::{Color32, RichText, Vec2 as EVec2};
use std::collections::HashMap;
use std::path::PathBuf;

mod automation;
mod character_ui;
mod gizmo;
mod hair_ui;
mod lighting_ui;
pub(crate) mod inspector;
pub mod operators;
mod modifiers_ui;
mod normals_ui;
mod sculpt_ui;
mod scene_ui;
pub use scene_ui::ScenePrefs;
mod render_ui;
pub use render_ui::RenderPrefs;
mod uv_ui;
mod viewport_ui;

pub(crate) struct CachedObject {
    pub id: u32,
    pub key: u64,
    pub mesh: GpuMesh,
}

/// Modo de sombreamento do viewport (UI). `Cel` usa o renderer canônico; os overlays seguem no Workbench.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewShading {
    #[default]
    Solid,
    Wireframe,
    Cel,
}

pub(crate) struct Gpu {
    pub wb: Workbench,
    pub tex: Option<egui::TextureId>,
    pub cel: Option<CelPreview>,
    pub cel_tex: Option<egui::TextureId>,
    pub cache: Vec<CachedObject>,
    pub adapter_name: String,
    pub backend: String,
}

pub struct StudioApp {
    pub(crate) cli: Cli,
    pub(crate) ws: WorkspaceId,
    pub(crate) tool_idx: HashMap<WorkspaceId, usize>,
    pub(crate) modeling: ModelingSession,
    pub(crate) camera: OrbitCamera,
    pub(crate) settings: ViewSettings,
    pub(crate) view_shading: ViewShading,
    pub(crate) gpu: Option<Gpu>,
    pub(crate) shader_watcher: Option<ShaderWatcher>,
    pub(crate) shader_error: Option<String>,
    /// Workspace Biblioteca (aberto sob demanda, na primeira visita ou no primeiro export).
    pub(crate) lib_ws: Option<LibraryWorkspace>,
    /// Biblioteca temporária do cenário `biblioteca-*` (mantida viva até o fim).
    pub(crate) lib_scenario_dir: Option<std::path::PathBuf>,
    pub(crate) frame_count: u64,
    pub(crate) screenshot_requested: bool,
    pub(crate) session_path: Option<PathBuf>,
    pub(crate) overlay_wire: bool,
    pub(crate) last_mouse: egui::Pos2,
    pub(crate) viewport_hovered: bool,
    pub(crate) gizmo_drag: bool,
    pub(crate) op_params: operators::OpParams,
    pub(crate) last_op: Option<operators::LastOp>,
    pub(crate) frame_pending: bool,
    /// Editor 2D de UV visível (Modo Edição).
    pub(crate) show_uv_editor: bool,
    /// Estado do editor de UV (vista 2D, parâmetros do Inspector, cache de métricas).
    pub(crate) uv: uv_ui::UvState,
    pub(crate) box_start: Option<glam::Vec2>,
    /// Estado do painel de normais (overlay, transferência, relatório, pincel de atributo).
    pub(crate) normals_ui: normals_ui::NormalsUi,
    /// Ferramenta "Pintar atributo" ativa: LMB no viewport pinta o canal NPR (Modo Edição).
    pub(crate) paint_attr_active: bool,
    /// Estado do Modo Escultura (pincéis, contexto/BVH do objeto ativo, traço).
    pub(crate) sculpt: crate::modeling::sculpt::SculptState,
    /// Estado do Modo Cabelo (ferramenta, grupo/mecha ativos, traço, pente).
    pub(crate) hair: crate::modeling::hair::HairState,
    /// Sessão canônica do núcleo (projeto + histórico) do workspace Personagem.
    pub(crate) core: anigo_session::CoreSession,
    pub(crate) char_ui: character_ui::CharacterUi,
    /// Estado do workspace Cenário (câmera, snap, seleção, caches); o projeto vive em `core`.
    pub(crate) scene_ui: scene_ui::SceneUi,
    /// Estado do workspace Iluminação (câmera, gizmo da luz, edição ao vivo); o projeto vive em `core`.
    pub(crate) light_ui: lighting_ui::LightingUi,
    /// Estado do workspace Render (câmera de render, saída, fila); o projeto vive em `core`.
    pub(crate) render_ui: render_ui::RenderUi,
}

const STORAGE_KEY: &str = "anigo_session_v1";

impl StudioApp {
    pub fn new(cc: &eframe::CreationContext<'_>, cli: Cli) -> Self {
        theme::apply(&cc.egui_ctx);
        icons::install(&cc.egui_ctx);

        let mut ws = WorkspaceId::default();
        let mut lang = Lang::default();
        let mut camera = OrbitCamera::default();
        let mut modeling = ModelingSession::with_default_cube();
        let mut restored = false;
        let mut core = anigo_session::CoreSession::new();
        let mut char_ui = character_ui::CharacterUi::default();
        let mut scene_ui = scene_ui::SceneUi::default();
        let mut render_ui = render_ui::RenderUi::default();

        let session_path = cli.session_file.as_ref().map(PathBuf::from);
        if !cli.no_session {
            let loaded = match &session_path {
                Some(p) if p.exists() => SessionState::load_from(p)
                    .map_err(|e| tracing::warn!("sessão ignorada ({}): {e}", p.display()))
                    .ok(),
                Some(_) => None,
                None => cc
                    .storage
                    .and_then(|s| eframe::get_value::<String>(s, STORAGE_KEY))
                    .and_then(|raw| {
                        SessionState::from_json(&raw)
                            .map_err(|e| tracing::warn!("sessão salva ignorada: {e}"))
                            .ok()
                    }),
            };
            if let Some(s) = loaded {
                ws = s.workspace;
                lang = s.lang;
                camera = s.camera;
                if !s.document.objects.is_empty() {
                    modeling.doc = s.document;
                }
                if let Some(doc) = &s.character_document {
                    if let Err(e) = core.load_document(doc) {
                        tracing::warn!("projeto do personagem da sessão ignorado: {e}");
                    }
                }
                if let Some(c) = s.character_camera {
                    char_ui.camera = c;
                    char_ui.framed = true;
                }
                if let Some(c) = s.scene_camera {
                    scene_ui.camera = c;
                    scene_ui.framed = true;
                }
                if let Some(p) = &s.scene_prefs {
                    scene_ui.apply_prefs(p);
                }
                if let Some(p) = &s.render_prefs {
                    render_ui.apply_prefs(p);
                }
                restored = true;
            }
        }
        if let Some(w) = cli.workspace {
            ws = w;
        }
        if let Some(l) = cli.lang {
            lang = l;
        }
        i18n::set_lang(lang);
        tracing::info!("sessão restaurada: {restored}");

        if !restored {
            let (lo, hi) = (glam::Vec3::new(-1.0, 0.0, -1.0), glam::Vec3::new(1.0, 2.0, 1.0));
            camera.frame_bounds(lo, hi);
            camera.distance *= 1.9;
        }
        let shader_watcher = dev_shader_path().and_then(|p| ShaderWatcher::new(p).ok());

        // cenários da Biblioteca: monta uma biblioteca temporária SINTÉTICA antes do primeiro quadro
        let mut lib_ws = None;
        let mut lib_scenario_dir = None;
        if let Some(name) = cli.scenario.as_deref().filter(|n| crate::library_ui::scenario::is_library_scenario(n)) {
            if cli.workspace.is_none() {
                ws = WorkspaceId::Biblioteca;
            }
            match crate::library_ui::scenario::build(name) {
                Ok(built) => {
                    let mut w = LibraryWorkspace::with_library(built.lib.clone());
                    w.start_scenario(name, built.assets.clone());
                    lib_scenario_dir = Some(built.dir.clone());
                    lib_ws = Some(w);
                }
                Err(e) => eprintln!("cenário {name}: {e}"),
            }
        }

        // cenários do Cenário: biblioteca temporária SINTÉTICA + GLBs reais gerados pelo studio
        if let Some(name) = cli.scenario.as_deref().filter(|n| scene_ui::is_scene_scenario(n)) {
            if cli.workspace.is_none() {
                ws = WorkspaceId::Cenario;
            }
            let n = std::env::var("ANIGO_SCENARIO_N").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
            match crate::library_ui::scenario::build_n(name, n) {
                Ok(built) => {
                    lib_ws = Some(LibraryWorkspace::with_library(built.lib.clone()));
                    lib_scenario_dir = Some(built.dir.clone());
                    scene_ui.script_assets = Some(built.assets.clone());
                }
                Err(e) => eprintln!("cenário {name}: {e}"),
            }
        }

        // cenários do Iluminação: o cenário é montado por comandos do núcleo (sem Biblioteca)
        if cli.scenario.as_deref().is_some_and(lighting_ui::is_light_scenario) && cli.workspace.is_none() {
            ws = WorkspaceId::Iluminacao;
        }

        // cenários do Render: a mesma biblioteca temporária SINTÉTICA + GLBs reais do Cenário
        if let Some(name) = cli.scenario.as_deref().filter(|n| render_ui::is_render_scenario(n)) {
            if cli.workspace.is_none() {
                ws = WorkspaceId::Render;
            }
            let n = std::env::var("ANIGO_SCENARIO_N").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
            match crate::library_ui::scenario::build_n(name, n) {
                Ok(built) => {
                    lib_ws = Some(LibraryWorkspace::with_library(built.lib.clone()));
                    lib_scenario_dir = Some(built.dir.clone());
                    scene_ui.script_assets = Some(built.assets.clone());
                }
                Err(e) => eprintln!("cenário {name}: {e}"),
            }
        }

        Self {
            cli,
            ws,
            tool_idx: HashMap::new(),
            modeling,
            camera,
            settings: ViewSettings::default(),
            view_shading: ViewShading::default(),
            gpu: None,
            shader_watcher,
            shader_error: None,
            lib_ws,
            lib_scenario_dir,
            frame_count: 0,
            screenshot_requested: false,
            session_path,
            overlay_wire: false,
            last_mouse: egui::Pos2::ZERO,
            viewport_hovered: false,
            gizmo_drag: false,
            op_params: operators::OpParams::default(),
            last_op: None,
            frame_pending: false,
            show_uv_editor: false,
            uv: uv_ui::UvState::default(),
            box_start: None,
            normals_ui: normals_ui::NormalsUi::default(),
            paint_attr_active: false,
            sculpt: crate::modeling::sculpt::SculptState::new(),
            hair: crate::modeling::hair::HairState::new(),
            core,
            char_ui,
            scene_ui,
            light_ui: lighting_ui::LightingUi::default(),
            render_ui,
        }
    }

    pub(crate) fn lang(&self) -> Lang {
        i18n::lang()
    }

    fn session_state(&self) -> SessionState {
        SessionState {
            version: SESSION_VERSION,
            workspace: self.ws,
            lang: i18n::lang(),
            camera: self.camera,
            document: self.modeling.doc.clone(),
            character_document: self.core.document().ok(),
            character_camera: Some(self.char_ui.camera),
            scene_camera: Some(self.scene_ui.camera),
            scene_prefs: Some(self.scene_ui.prefs()),
            render_prefs: Some(self.render_ui.prefs.clone()),
        }
    }

    /// Desfazer/refazer roteados pelo workspace ativo: Personagem fala com a sessão do núcleo,
    /// os demais com a `ModelingSession`.
    pub(crate) fn can_undo_redo(&self) -> (bool, bool) {
        character_ui::can_undo_redo(self.ws, &self.core, &self.modeling)
    }

    pub(crate) fn undo_redo(&mut self, undo: bool) {
        if self.ws == WorkspaceId::Personagem {
            self.char_ui.gesture = None;
        }
        if self.ws == WorkspaceId::Cenario {
            self.scene_ui.session = None;
            self.scene_ui.gesture = None;
        }
        if self.ws == WorkspaceId::Iluminacao {
            self.light_ui.gesture = None;
            self.light_ui.drag = None;
        }
        if self.ws == WorkspaceId::Render {
            self.render_ui.gesture = None;
            self.render_ui.nav = None;
            self.render_ui.wheel = None;
        }
        if let Some(msg) = character_ui::route_undo_redo(self.ws, &mut self.core, &mut self.modeling, undo) {
            if self.ws == WorkspaceId::Cenario {
                self.scene_ui.message = msg;
            } else if self.ws == WorkspaceId::Iluminacao {
                self.light_ui.message = msg;
            } else if self.ws == WorkspaceId::Render {
                self.render_ui.message = msg;
            } else {
                self.char_ui.message = msg;
            }
        }
    }

    pub(crate) fn ensure_gpu(&mut self, frame: &eframe::Frame) -> bool {
        if self.gpu.is_some() {
            return true;
        }
        let Some(rs) = frame.wgpu_render_state() else {
            return false;
        };
        match Workbench::new(rs.device.clone(), rs.queue.clone()) {
            Ok(wb) => {
                let info = rs.adapter.get_info();
                let cel = CelPreview::new(rs.device.clone(), rs.queue.clone(), &info)
                    .map_err(|e| tracing::warn!("preview cel indisponível: {e}"))
                    .ok();
                self.gpu = Some(Gpu {
                    wb,
                    tex: None,
                    cel,
                    cel_tex: None,
                    cache: Vec::new(),
                    adapter_name: info.name,
                    backend: format!("{:?}", info.backend),
                });
                true
            }
            Err(e) => {
                self.shader_error = Some(format!("workbench: {e}"));
                false
            }
        }
    }

    fn poll_hot_reload(&mut self) {
        let (Some(w), Some(gpu)) = (&self.shader_watcher, &mut self.gpu) else {
            return;
        };
        if w.take_changed() {
            match w.read().and_then(|src| gpu.wb.reload_shaders(&src)) {
                Ok(()) => {
                    self.shader_error = None;
                    tracing::info!("shader recarregado");
                }
                Err(e) => {
                    self.shader_error = Some(e);
                }
            }
        }
    }

    // ------------------------------------------------------------------- barras

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("topbar").exact_size(38.0).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(8.0);
                ui.label(RichText::new("ANIGO").strong().size(15.0).color(theme::ACCENT));
                ui.add_space(14.0);
                for w in WorkspaceId::ALL {
                    let label = format!("{}  {}", w.icon(), tr(&format!("workspace.{}", w.key())));
                    let resp = ui.selectable_label(self.ws == w, RichText::new(label).size(13.5));
                    if resp.clicked() && self.ws != w {
                        self.switch_workspace(w);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(8.0);
                    let lang_label = match self.lang() {
                        Lang::PtBr => "PT",
                        Lang::En => "EN",
                    };
                    if ui.button(lang_label).clicked() {
                        i18n::set_lang(match self.lang() {
                            Lang::PtBr => Lang::En,
                            Lang::En => Lang::PtBr,
                        });
                    }
                    let (can_undo, can_redo) = self.can_undo_redo();
                    if ui
                        .add_enabled(can_redo, egui::Button::new(icons::ARROW_CLOCKWISE))
                        .on_hover_text(tr("action.redo"))
                        .clicked()
                    {
                        self.undo_redo(false);
                    }
                    if ui
                        .add_enabled(can_undo, egui::Button::new(icons::ARROW_COUNTER_CLOCKWISE))
                        .on_hover_text(tr("action.undo"))
                        .clicked()
                    {
                        self.undo_redo(true);
                    }
                });
            });
        });
    }

    pub(crate) fn switch_workspace(&mut self, w: WorkspaceId) {
        if self.modeling.modal_active() {
            self.modeling.cancel_modal();
        }
        // a luz presa à câmera é só preview do Iluminação: nunca vaza para os outros workspaces
        self.scene_ui.light_dir_override = None;
        if self.ws == WorkspaceId::Render && w != WorkspaceId::Render {
            self.render_leave();
        }
        self.ws = w;
        if w == WorkspaceId::Iluminacao {
            self.light_enter();
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("statusbar").exact_size(24.0).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(8.0);
                let msg = if self.ws == WorkspaceId::Biblioteca && self.lib_ws.is_some() {
                    self.lib_ws.as_ref().map(|w| w.status_text()).unwrap_or_default()
                } else if self.ws == WorkspaceId::Personagem && !self.char_ui.message.is_empty() {
                    self.char_ui.message.clone()
                } else if self.ws == WorkspaceId::Cenario && !self.scene_ui.message.is_empty() {
                    self.scene_ui.message.clone()
                } else if self.ws == WorkspaceId::Iluminacao && !self.light_ui.message.is_empty() {
                    self.light_ui.message.clone()
                } else if self.ws == WorkspaceId::Render && !self.render_ui.message.is_empty() {
                    self.render_ui.message.clone()
                } else if !self.modeling.status.is_empty() {
                    self.modeling.status.clone()
                } else {
                    tr("status.ready").to_string()
                };
                ui.label(RichText::new(msg).color(theme::TEXT_DIM).size(12.0));
                if let Some(err) = &self.shader_error {
                    ui.separator();
                    ui.label(RichText::new(format!("shader: {}", err.lines().next().unwrap_or(""))).color(Color32::from_rgb(0xff, 0x70, 0x70)).size(12.0));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(8.0);
                    if let Some(g) = &self.gpu {
                        if let Some(m) = g.cel.as_ref().and_then(|c| c.metrics.as_ref()) {
                            if self.view_shading == ViewShading::Cel {
                                ui.label(
                                    RichText::new(format!("cel: {} draws · {} descartados · {} tris", m.draw_calls, m.culled_draw_calls, m.triangle_count))
                                        .color(theme::TEXT_DIM)
                                        .size(11.5),
                                );
                                ui.separator();
                            }
                        }
                        ui.label(RichText::new(format!("{} · {}", g.adapter_name, g.backend)).color(theme::TEXT_DIM).size(11.5));
                    }
                    if self.ws == WorkspaceId::Modelagem {
                        let s = self.modeling.stats();
                        ui.separator();
                        ui.label(
                            RichText::new(format!(
                                "V {}  F {}  Tris {}   sel V {} F {}",
                                s.verts, s.faces, s.tris, s.selected_verts, s.selected_faces
                            ))
                            .color(theme::TEXT_DIM)
                            .size(11.5),
                        );
                    }
                });
            });
        });
    }

    fn tool_shelf(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("toolshelf")
            .exact_size(46.0)
            .resizable(false)
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.vertical_centered(|ui| {
                    let tools = self.shelf_tools();
                    let cur = *self.tool_idx.get(&self.ws).unwrap_or(&0);
                    for (i, t) in tools.iter().enumerate() {
                        let selected = cur == i;
                        let btn = egui::Button::new(RichText::new(t.icon).size(20.0))
                            .min_size(EVec2::new(34.0, 34.0))
                            .selected(selected);
                        if ui.add(btn).on_hover_text(tr(t.label_key)).clicked() {
                            self.tool_idx.insert(self.ws, i);
                            self.on_tool_selected(t.id);
                        }
                    }
                });
            });
    }

    /// Ferramentas do shelf: no workspace Modelagem dependem do modo.
    pub(crate) fn shelf_tools(&self) -> Vec<crate::workspace::ToolDef> {
        if self.ws == WorkspaceId::Modelagem {
            return crate::app::inspector::modeling_tools(self.modeling.mode).to_vec();
        }
        self.ws.tools().to_vec()
    }

    fn on_tool_selected(&mut self, id: &str) {
        if self.ws == WorkspaceId::Cenario {
            self.on_scene_tool_selected(id);
        }
        if self.ws == WorkspaceId::Iluminacao {
            self.on_light_tool_selected(id);
        }
        if self.ws == WorkspaceId::Modelagem {
            let label_key = crate::app::inspector::modeling_tools(self.modeling.mode)
                .iter()
                .find(|t| t.id == id)
                .map(|t| t.label_key)
                .unwrap_or("");
            self.modeling.status = format!("Ferramenta: {}", tr(label_key));
            if let Some(kind) = operators::tool_to_op(id) {
                self.run_op(kind);
            }
        }
    }

    // ------------------------------------------------------------- Biblioteca

    /// Workspace Biblioteca (criado na primeira necessidade; abre a biblioteca em segundo plano).
    pub(crate) fn lib_workspace(&mut self) -> &mut LibraryWorkspace {
        self.lib_ws.get_or_insert_with(|| LibraryWorkspace::new(library::default_spec()))
    }

    fn library_ui(&mut self, ui: &mut egui::Ui) {
        let actions = self.lib_workspace().ui(ui);
        for a in actions {
            match a {
                HostAction::EditInModeling { path, format, name } => self.edit_library_asset(&path, &format, &name),
                HostAction::Status(s) => self.modeling.status = s,
                HostAction::PlaceAsset { .. } => {}
            }
        }
    }

    /// "Editar na Modelagem": importa o modelo (GLB/VRM/OBJ/FBX) como objeto editável e muda de workspace.
    fn edit_library_asset(&mut self, path: &std::path::Path, format: &str, name: &str) {
        match library::import_model_mesh(path, format) {
            Ok(mut mesh) => {
                mesh.name = name.to_string();
                self.modeling.add_object_from_mesh(name, mesh, glam::Vec3::ZERO);
                self.modeling.status = format!("Importado: {name}");
                self.switch_workspace(WorkspaceId::Modelagem);
                self.frame_selected_or_all();
            }
            Err(e) => {
                let msg = format!("Falha ao abrir {} na Modelagem: {e}", path.display());
                self.modeling.status = msg.clone();
                self.lib_workspace().report_error(msg);
            }
        }
    }

    /// Exporta o objeto ativo como GLB e o registra na Biblioteca como ativo autoral (nova revisão se já existir).
    pub(crate) fn export_active_to_library(&mut self) {
        let Some(obj) = self.modeling.active_object().cloned() else {
            self.modeling.status = "Nenhum objeto ativo".into();
            return;
        };
        let tmp = std::env::temp_dir().join(format!("anigo-export-{}", std::process::id()));
        let result = library::export_object_glb(&obj, &tmp).map_err(|e| format!("Exportar: {e}")).and_then(|glb| {
            let r = self.lib_workspace().import_authored(&obj.name, &glb).map_err(|e| format!("Biblioteca: {e}"));
            let _ = std::fs::remove_file(&glb);
            r
        });
        let _ = std::fs::remove_dir(&tmp);
        self.modeling.status = match result {
            Ok(msg) => format!("Exportado para a Biblioteca: {msg}"),
            Err(e) => e,
        };
    }

    fn placeholder_ui(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.35);
                ui.label(RichText::new(self.ws.icon()).size(48.0).color(theme::ACCENT));
                ui.label(RichText::new(tr(&format!("workspace.{}", self.ws.key()))).size(22.0).strong());
                ui.add_space(6.0);
                ui.label(RichText::new(tr("placeholder.not_ported")).color(theme::TEXT_DIM));
            });
        });
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.cli.screenshot.clone() else {
            return;
        };
        let library_busy = (self.ws == WorkspaceId::Biblioteca && self.frame_count < self.cli.frames as u64 + 3000 && self.lib_ws.as_ref().is_some_and(|w| !w.scenario_done()))
            || (self.ws == WorkspaceId::Cenario && self.frame_count < self.cli.frames as u64 + 3000 && self.scene_ui.script.as_ref().is_some_and(|s| !s.is_done()))
            || (self.ws == WorkspaceId::Iluminacao && self.frame_count < self.cli.frames as u64 + 3000 && self.light_ui.script.as_ref().is_some_and(|s| !s.is_done()))
            || (self.ws == WorkspaceId::Render && self.frame_count < self.cli.frames as u64 + 3000 && self.render_busy());
        if !self.screenshot_requested && self.frame_count >= self.cli.frames as u64 && !library_busy {
            self.screenshot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let shots: Vec<_> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
                .collect()
        });
        for img in shots {
            let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            match image::save_buffer(&path, &rgba, img.size[0] as u32, img.size[1] as u32, image::ColorType::Rgba8) {
                Ok(()) => eprintln!("screenshot salvo: {path} ({}x{})", img.size[0], img.size[1]),
                Err(e) => eprintln!("falha ao salvar screenshot: {e}"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }
}

impl eframe::App for StudioApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.frame_count += 1;
        self.ensure_gpu(frame);
        self.poll_hot_reload();
        self.run_scenario();

        self.top_bar(ui);
        self.status_bar(ui);
        if self.ws.has_viewport() {
            self.tool_shelf(ui);
        }
        if self.ws == WorkspaceId::Personagem {
            self.character_shortcuts(&ui.ctx().clone());
            self.inspector_character(ui);
        }
        if self.ws == WorkspaceId::Cenario {
            self.scene_script_tick();
            self.scene_shortcuts(&ui.ctx().clone());
            self.scene_panels(ui);
        }
        if self.ws == WorkspaceId::Iluminacao {
            self.light_script_tick();
            self.light_shortcuts(&ui.ctx().clone());
            self.light_panels(ui);
        }
        if self.ws == WorkspaceId::Render {
            let ctx = ui.ctx().clone();
            self.render_script_tick(&ctx);
            self.render_shortcuts(&ctx);
            self.render_panels(ui);
        }
        if self.ws == WorkspaceId::Modelagem {
            self.inspector_modeling(ui);
            if self.show_uv_editor && self.modeling.mode == Mode::Edit {
                egui::Panel::bottom("uv_editor")
                    .default_size(360.0)
                    .size_range(160.0..=700.0)
                    .show(ui, |ui| self.uv_editor_panel(ui));
            }
        }

        if self.ws.has_viewport() {
            if self.ws == WorkspaceId::Modelagem {
                self.viewport_ui(ui, frame);
            } else if self.ws == WorkspaceId::Personagem {
                self.character_viewport_ui(ui, frame);
            } else if self.ws == WorkspaceId::Cenario {
                self.scene_viewport_ui(ui, frame);
            } else if self.ws == WorkspaceId::Iluminacao {
                self.light_viewport_ui(ui, frame);
            } else if self.ws == WorkspaceId::Render {
                self.render_viewport_ui(ui, frame);
            } else {
                self.placeholder_ui(ui);
            }
        } else {
            self.library_ui(ui);
        }
        let ctx = ui.ctx().clone();
        self.handle_screenshot(&ctx);
        if self.shader_watcher.is_some() {
            // hot reload precisa de polling mesmo sem interação
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.cli.no_session {
            return;
        }
        let state = self.session_state();
        match (&self.session_path, state.to_json()) {
            (Some(p), Ok(_)) => {
                if let Err(e) = state.save_to(p) {
                    tracing::warn!("falha ao salvar sessão: {e}");
                }
            }
            (None, Ok(json)) => eframe::set_value(storage, STORAGE_KEY, &json),
            (_, Err(e)) => tracing::warn!("falha ao serializar sessão: {e}"),
        }
    }

    fn on_exit(&mut self) {
        self.render_cleanup();
        // a biblioteca temporária de um cenário não sobrevive ao app (ANIGO_SCENARIO_KEEP=1 preserva para inspeção)
        if let (Some(dir), None) = (self.lib_scenario_dir.take(), std::env::var_os("ANIGO_SCENARIO_KEEP")) {
            let _ = std::fs::remove_dir_all(dir);
        }
        if let (Some(p), false) = (&self.session_path, self.cli.no_session) {
            let _ = self.session_state().save_to(p);
        }
    }
}

pub(crate) fn mode_label(m: Mode) -> &'static str {
    match m {
        Mode::Object => tr("mode.object"),
        Mode::Edit => tr("mode.edit"),
        Mode::Sculpt => tr("mode.sculpt"),
        Mode::Hair => tr("mode.hair"),
    }
}

pub(crate) fn shading_label(s: ViewShading) -> &'static str {
    match s {
        ViewShading::Solid => tr("viewport.solid"),
        ViewShading::Wireframe => tr("viewport.wireframe"),
        ViewShading::Cel => tr("viewport.cel"),
    }
}
