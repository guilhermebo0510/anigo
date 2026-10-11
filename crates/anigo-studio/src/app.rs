//! `eframe::App` do ANIGO Studio: barra de workspaces, shelf de ferramentas, inspector,
//! viewport (Workbench wgpu) e Biblioteca.

use crate::cli::Cli;
use crate::hot::{dev_shader_path, ShaderWatcher};
use crate::i18n::{self, tr, Lang};
use crate::icons;
use crate::library::{self, AssetRecord};
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
mod gizmo;
mod inspector;
pub mod operators;
mod modifiers_ui;
mod normals_ui;
mod sculpt_ui;
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

#[derive(Default)]
pub(crate) struct LibraryState {
    pub assets: Vec<AssetRecord>,
    pub category: Option<&'static str>,
    pub query: String,
    pub selected: Option<usize>,
    pub toast: Option<String>,
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
    pub(crate) library: LibraryState,
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
    pub(crate) box_start: Option<glam::Vec2>,
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

        let mut library = LibraryState::default();
        library.assets = library_scan();

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
            library,
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
            box_start: None,
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
                    let can_redo = self.modeling.can_redo();
                    let can_undo = self.modeling.can_undo();
                    if ui
                        .add_enabled(can_redo, egui::Button::new(icons::ARROW_CLOCKWISE))
                        .on_hover_text(tr("action.redo"))
                        .clicked()
                    {
                        self.modeling.redo();
                    }
                    if ui
                        .add_enabled(can_undo, egui::Button::new(icons::ARROW_COUNTER_CLOCKWISE))
                        .on_hover_text(tr("action.undo"))
                        .clicked()
                    {
                        self.modeling.undo();
                    }
                });
            });
        });
    }

    pub(crate) fn switch_workspace(&mut self, w: WorkspaceId) {
        if self.modeling.modal_active() {
            self.modeling.cancel_modal();
        }
        self.ws = w;
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("statusbar").exact_size(24.0).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(8.0);
                let msg = if !self.modeling.status.is_empty() {
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
        if self.ws == WorkspaceId::Modelagem {
            self.modeling.status = format!("Ferramenta: {}", tr(&format!("tool.{id}")));
            if let Some(kind) = operators::tool_to_op(id) {
                self.run_op(kind);
            }
        }
    }

    // ------------------------------------------------------------- Biblioteca

    fn library_ui(&mut self, ui: &mut egui::Ui) {
        // categorias (esquerda do conteúdo)
        egui::Panel::left("lib_cats").default_size(190.0).show(ui, |ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(tr("lib.categories")).strong());
            ui.add_space(4.0);
            if ui.selectable_label(self.library.category.is_none(), "Todos").clicked() {
                self.library.category = None;
            }
            for c in library::CATEGORIES {
                let n = self.library.assets.iter().filter(|a| a.category == c).count();
                let label = format!("{c}  ({n})");
                if ui.selectable_label(self.library.category == Some(c), label).clicked() {
                    self.library.category = Some(c);
                }
            }
        });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(icons::MAGNIFYING_GLASS);
                ui.add(
                    egui::TextEdit::singleline(&mut self.library.query)
                        .hint_text(tr("lib.search_hint"))
                        .desired_width(320.0),
                );
            });
            ui.separator();
            let authored: Vec<AssetRecord> = self
                .modeling
                .doc
                .objects
                .iter()
                .map(|o| library::authored_record(&o.name, &o.mesh))
                .collect();
            let all: Vec<&AssetRecord> = self
                .library
                .assets
                .iter()
                .chain(authored.iter())
                .filter(|a| a.matches(self.library.category, &self.library.query))
                .collect();
            if all.is_empty() {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Nenhum ativo nesta categoria").color(theme::TEXT_DIM));
                });
                return;
            }
            let card_w = 190.0;
            let cols = ((ui.available_width() / (card_w + 10.0)).floor() as usize).max(1);
            let mut toast = None;
            let mut edit_request: Option<AssetRecord> = None;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                egui::Grid::new("asset_grid").spacing([10.0, 10.0]).show(ui, |ui| {
                    for (i, a) in all.iter().enumerate() {
                        ui.allocate_ui(EVec2::new(card_w, 170.0), |ui| {
                            egui::Frame::group(ui.style()).show(ui, |ui| {
                                ui.set_width(card_w - 16.0);
                                ui.vertical(|ui| {
                                    let (rect, _) = ui.allocate_exact_size(EVec2::new(card_w - 16.0, 74.0), egui::Sense::hover());
                                    ui.painter().rect_filled(rect, 4.0, theme::BG_DEEP);
                                    ui.painter().text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        category_icon(a.category),
                                        egui::FontId::proportional(34.0),
                                        theme::ACCENT,
                                    );
                                    ui.label(RichText::new(&a.name).strong());
                                    let meta = match (a.tris, a.verts) {
                                        (Some(t), Some(v)) => format!("{} · {} tris · {} vértices", a.format, t, v),
                                        _ => a.format.to_string(),
                                    };
                                    ui.label(RichText::new(meta).size(11.0).color(theme::TEXT_DIM));
                                    ui.horizontal_wrapped(|ui| {
                                        if ui.small_button(tr("lib.use_in_scene")).clicked() {
                                            toast = Some(format!("\"{}\": a colocação em cena entra com o workspace Cenário nativo", a.name));
                                        }
                                        if ui.small_button("Editar na Modelagem").clicked() {
                                            edit_request = Some((*a).clone());
                                        }
                                        if ui.small_button(tr("lib.inspect")).clicked() {
                                            self.library.selected = Some(i);
                                        }
                                    });
                                });
                            });
                        });
                        if (i + 1) % cols == 0 {
                            ui.end_row();
                        }
                    }
                });
            });
            if let Some(a) = edit_request {
                self.edit_library_asset(&a);
            } else if let Some(t) = toast {
                self.library.toast = Some(t);
            }
            if let Some(t) = &self.library.toast {
                ui.separator();
                ui.label(RichText::new(t).color(theme::TEXT_DIM).size(12.0));
            }
        });
    }

    /// "Editar na Modelagem": importa o GLB (ou ativa o objeto autoral) e muda de workspace.
    fn edit_library_asset(&mut self, a: &AssetRecord) {
        if a.authored {
            let name = a.name.clone();
            if let Some(i) = self.modeling.doc.objects.iter().position(|o| o.name == name) {
                self.modeling.set_active(i);
            }
            self.switch_workspace(WorkspaceId::Modelagem);
            return;
        }
        let Some(path) = a.path.clone() else {
            return;
        };
        match library::import_glb_mesh(&path) {
            Ok(mesh) => {
                let name = mesh.name.clone();
                self.modeling.add_object_from_mesh(&name, mesh, glam::Vec3::ZERO);
                self.modeling.status = format!("Importado: {name}");
                self.switch_workspace(WorkspaceId::Modelagem);
                self.frame_selected_or_all();
            }
            Err(e) => self.library.toast = Some(format!("Falha ao importar {}: {e}", path.display())),
        }
    }

    /// Exporta o objeto ativo como GLB na pasta de ativos e atualiza o catálogo.
    pub(crate) fn export_active_to_library(&mut self) {
        let Some(obj) = self.modeling.active_object().cloned() else {
            self.modeling.status = "Nenhum objeto ativo".into();
            return;
        };
        match library::export_object_glb(&obj, &library::studio_assets_dir()) {
            Ok(path) => {
                self.modeling.status = format!("Exportado para a Biblioteca: {}", path.display());
                self.library.assets = library_scan();
            }
            Err(e) => self.modeling.status = format!("Exportar: {e}"),
        }
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
        if !self.screenshot_requested && self.frame_count >= self.cli.frames as u64 {
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

/// Bases canônicas do repositório + ativos do usuário (`Documents/ANIGO/Assets`).
pub(crate) fn library_scan() -> Vec<AssetRecord> {
    let mut v = library::scan_glb_dir(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models"));
    v.extend(library::scan_glb_dir(&library::studio_assets_dir()));
    v
}

fn category_icon(cat: &str) -> &'static str {
    match cat {
        "Personagens" => icons::USER,
        "Roupas" => icons::T_SHIRT,
        "Penteados" => icons::SCISSORS,
        "Acessórios" => icons::CUBE,
        "Poses" => icons::PERSON_SIMPLE_RUN,
        "Materiais" => icons::PALETTE,
        "Ambientes" => icons::CLOUD,
        _ => icons::MOUNTAINS,
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
        if self.ws == WorkspaceId::Modelagem {
            self.inspector_modeling(ui);
            if self.show_uv_editor && self.modeling.mode == Mode::Edit {
                egui::Panel::bottom("uv_editor")
                    .default_size(300.0)
                    .size_range(160.0..=700.0)
                    .show(ui, |ui| self.uv_editor_panel(ui));
            }
        }

        if self.ws.has_viewport() {
            if self.ws == WorkspaceId::Modelagem {
                self.viewport_ui(ui, frame);
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
    }
}

pub(crate) fn shading_label(s: ViewShading) -> &'static str {
    match s {
        ViewShading::Solid => tr("viewport.solid"),
        ViewShading::Wireframe => tr("viewport.wireframe"),
        ViewShading::Cel => tr("viewport.cel"),
    }
}
