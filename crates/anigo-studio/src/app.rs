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

mod animation_ui;
mod automation;
mod bridge_host;
mod bridge_anim;
mod bridge_app;
mod bridge_char;
mod bridge_scene;
mod bridge_anim2;
mod bridge_hair;
mod bridge_input;
mod bridge_meshdata;
mod bridge_light;
mod bridge_model;
mod bridge_pose;
mod bridge_render;
mod bridge_shading;
mod bridge_shading2;
mod bridge_weights;
mod bridge_ui;
mod multiview;
pub use multiview::MultiViewState;
#[cfg(test)]
pub(crate) fn multiview_for_tests() -> MultiViewState {
    multiview::MultiView::default().to_state(&crate::viewport::OrbitCamera::default())
}
mod props_tabs;
mod character_ui;
mod dock_ui;
mod gizmo;
mod hair_ui;
mod lighting_ui;
mod menu_ui;
pub(crate) mod inspector;
pub mod operators;
mod modifiers_ui;
mod normals_ui;
mod posing_ui;
mod sculpt_ui;
mod scene_ui;
pub use scene_ui::ScenePrefs;
mod render_ui;
pub use render_ui::RenderPrefs;
mod shading_ui;
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
    /// Texturas das vistas 1..=3 do multi-vista (a vista 0 usa `tex`).
    pub view_tex: [Option<egui::TextureId>; 3],
    /// Previews cel das vistas 1..=3 (criados na primeira vez que a vista pede Cel) e as suas texturas no egui.
    pub view_cel: [Option<CelPreview>; 3],
    pub view_cel_tex: [Option<egui::TextureId>; 3],
    pub cache: Vec<CachedObject>,
    pub adapter_name: String,
    pub backend: String,
    device: eframe::wgpu::Device,
    queue: eframe::wgpu::Queue,
    adapter: eframe::wgpu::AdapterInfo,
}

impl Gpu {
    /// Cria o preview cel da vista `slot` se ainda não existe (a vista 0 usa `cel`, criado com a GPU).
    pub(crate) fn ensure_cel(&mut self, slot: usize) {
        if slot == 0 || self.view_cel[slot - 1].is_some() {
            return;
        }
        match CelPreview::new(self.device.clone(), self.queue.clone(), &self.adapter) {
            Ok(c) => self.view_cel[slot - 1] = Some(c),
            Err(e) => tracing::warn!("preview cel da vista {slot} indisponível: {e}"),
        }
    }

    /// Preview cel da vista `slot`.
    pub(crate) fn cel_slot(&mut self, slot: usize) -> Option<&mut CelPreview> {
        if slot == 0 {
            self.cel.as_mut()
        } else {
            self.view_cel[slot - 1].as_mut()
        }
    }

    pub(crate) fn cel_tex_of(&self, slot: usize) -> Option<egui::TextureId> {
        if slot == 0 {
            self.cel_tex
        } else {
            self.view_cel_tex[slot - 1]
        }
    }

    pub(crate) fn has_cel(&self, slot: usize) -> bool {
        if slot == 0 {
            self.cel.is_some()
        } else {
            self.view_cel[slot - 1].is_some()
        }
    }

    /// Registra (ou re-registra, se o alvo foi recriado) a textura do preview cel da vista no egui.
    pub(crate) fn refresh_cel_tex(&mut self, slot: usize, recreated: bool, rs: &eframe::egui_wgpu::RenderState) {
        let (cel, tex) = if slot == 0 { (&self.cel, &mut self.cel_tex) } else { (&self.view_cel[slot - 1], &mut self.view_cel_tex[slot - 1]) };
        if !(recreated || tex.is_none()) {
            return;
        }
        let mut renderer = rs.renderer.write();
        if let Some(old) = tex.take() {
            renderer.free_texture(&old);
        }
        if let Some(view) = cel.as_ref().and_then(|c| c.view()) {
            *tex = Some(renderer.register_native_texture(&rs.device, view, eframe::wgpu::FilterMode::Linear));
        }
    }
}

pub struct StudioApp {
    pub(crate) cli: Cli,
    pub(crate) ws: WorkspaceId,
    pub(crate) tool_idx: HashMap<WorkspaceId, usize>,
    pub(crate) modeling: ModelingSession,
    pub(crate) camera: OrbitCamera,
    /// Multi-vista do viewport da Modelagem (layout, câmeras das vistas não ativas, vista ativa).
    pub(crate) mview: multiview::MultiView,
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
    /// Identidade visual (texturas da marca) e tela de abertura.
    pub(crate) brand: crate::brand::Brand,
    pub(crate) splash: crate::brand::Splash,
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
    /// Estado do workspace Shading (alvo, preview, paleta, presets); o material vive no núcleo/Modelagem.
    pub(crate) shading_ui: shading_ui::ShadingUi,
    /// Estado do workspace Posing (seleção de ossos, IK, gizmo, Biblioteca de poses); a pose vive em `core`.
    pub(crate) pose_ui: posing_ui::PosingUi,
    /// Bridge local de automação do `anigo-mcp` (T-2.13); `None` = desligada (padrão).
    pub(crate) bridge: Option<bridge_host::BridgeHost>,
    /// Estado do workspace Animação (timeline, clips, auto-key, onion, física); o clip vive no projeto de `core`.
    pub(crate) anim_ui: animation_ui::AnimUi,
    /// Layout dockável por workspace (árvores do `egui_tiles`, layouts salvos, presets).
    pub(crate) dock: dock_ui::DockState,
    /// Aba atual do Propriedades da Modelagem, por modo (`Mode as u8`); estado de apresentação, não persiste.
    pub(crate) prop_tabs: HashMap<u8, props_tabs::PropTab>,
    /// Abas que cada painel mostrou no último quadro (a faixa do Cenário depende da seleção).
    pub(crate) tabs_shown: HashMap<u8, Vec<props_tabs::PropTab>>,
    /// Entrada sintética enfileirada pela bridge (`INPUT`); drenada em `raw_input_hook`, um passo por quadro.
    pub(crate) input: bridge_input::InputQueue,
    /// Atalhos em vigor (padrão + delta do usuário); `Arc` para os handlers consultarem sem travar `self`.
    pub(crate) keymap: std::sync::Arc<crate::keymap::Keymap>,
    pub(crate) keymap_path: Option<PathBuf>,
    /// Estado dos menus, diálogos e da janela de atalhos.
    pub(crate) menu: menu_ui::MenuState,
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
        let mut saved_views: HashMap<String, MultiViewState> = HashMap::new();
        let mut core = anigo_session::CoreSession::new();
        let mut char_ui = character_ui::CharacterUi::default();
        let mut scene_ui = scene_ui::SceneUi::default();
        let mut render_ui = render_ui::RenderUi::default();
        let pose_ui = posing_ui::PosingUi::default();
        // layouts: arquivo do usuário (padrão; `--no-session` não lê nem grava), sessão por cima, `--layout-file` explícito por cima de tudo
        let mut layout_warnings: Vec<String> = Vec::new();
        let mut layouts = crate::layout::LayoutStore::default();
        let layout_path: Option<PathBuf> = match (&cli.layout_file, cli.no_session) {
            (Some(p), _) => Some(PathBuf::from(p)),
            (None, false) => Some(crate::layout::default_layouts_path()),
            (None, true) => None,
        };
        if cli.layout_file.is_none() {
            if let Some(p) = &layout_path {
                let l = crate::layout::LayoutStore::load_file(p);
                layout_warnings.extend(l.warnings);
                layouts = l.store;
            }
        }

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
                if let Some(l) = s.layouts {
                    layouts.merge_over(l);
                }
                saved_views = s.multiviews;
                restored = true;
            }
        }
        // atalhos: arquivo do usuário (`--no-session` não lê nem grava, salvo `--keymap-file` explícito)
        let keymap_path: Option<PathBuf> = match (&cli.keymap_file, cli.no_session) {
            (Some(p), _) => Some(PathBuf::from(p)),
            (None, false) => Some(crate::keymap::default_keymap_path()),
            (None, true) => None,
        };
        let (keymap, keymap_warnings) = keymap_path.as_deref().map(crate::keymap::Keymap::load_file).unwrap_or_default();
        for w in &keymap_warnings {
            tracing::warn!("{w}");
        }
        if let (Some(_), Some(p)) = (&cli.layout_file, &layout_path) {
            let l = crate::layout::LayoutStore::load_file(p);
            layout_warnings.extend(l.warnings);
            layouts.merge_over(l.store);
        }
        for w in &layout_warnings {
            tracing::warn!("{w}");
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

        // cenários `layout-<workspace>`: só abrem o workspace (o layout padrão ou o do `--layout-file`)
        if let Some(w) = cli.scenario.as_deref().and_then(StudioApp::layout_scenario_workspace) {
            if cli.workspace.is_none() {
                ws = w;
            }
        }

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

        // cenários do Shading: biblioteca temporária VAZIA (presets salvos pelo próprio cenário)
        if let Some(name) = cli.scenario.as_deref().filter(|n| shading_ui::is_shading_scenario(n)) {
            if cli.workspace.is_none() {
                ws = WorkspaceId::Shading;
            }
            let dir = std::env::var_os("ANIGO_SCENARIO_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::temp_dir().join(format!("anigo-shading-scenario-{}-{}", std::process::id(), name.trim_start_matches("shading-"))));
            let _ = std::fs::remove_dir_all(&dir);
            match anigo_assets::Library::open(dir.join("lib")) {
                Ok(lib) => {
                    lib_ws = Some(LibraryWorkspace::with_library(lib));
                    lib_scenario_dir = Some(dir.clone());
                }
                Err(e) => eprintln!("cenário {name}: {e}"),
            }
            // paletas do cenário isoladas das do usuário
            std::env::set_var("ANIGO_SHADING_DIR", dir.join("shading"));
        }

        // cenários do Posing: abrem o workspace; o da Biblioteca de poses usa uma biblioteca temporária vazia
        if let Some(name) = cli.scenario.as_deref().filter(|n| posing_ui::is_posing_scenario(n)) {
            if cli.workspace.is_none() {
                ws = WorkspaceId::Posing;
            }
            if name == "posing-biblioteca" {
                match posing_ui::temp_library() {
                    Ok((w, dir)) => {
                        lib_ws = Some(w);
                        lib_scenario_dir = Some(dir);
                    }
                    Err(e) => eprintln!("cenário {name}: {e}"),
                }
            }
        }

        let bridge = bridge_host::BridgeHost::start(&cli, &cc.egui_ctx);
        // cenários da Animação: abrem o workspace; os que salvam/aplicam clips usam uma biblioteca temporária vazia
        if let Some(name) = cli.scenario.as_deref().filter(|n| animation_ui::is_animation_scenario(n)) {
            if cli.workspace.is_none() {
                ws = WorkspaceId::Animacao;
            }
            if name == "animacao-clips" || name == "animacao-sequencia" {
                match posing_ui::temp_library() {
                    Ok((w, dir)) => {
                        lib_ws = Some(w);
                        lib_scenario_dir = Some(dir);
                    }
                    Err(e) => eprintln!("cenário {name}: {e}"),
                }
            }
        }

        // splash só no uso normal: captura, cenário e bridge (automação) começam direto na interface
        let cli_automated = cli.screenshot.is_some() || cli.scenario.is_some() || cli.bridge;
        let mut app = Self {
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
            brand: crate::brand::Brand::default(),
            splash: crate::brand::Splash::new(crate::brand::splash_mode(cli_automated)),
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
            shading_ui: shading_ui::ShadingUi::default(),
            pose_ui,
            bridge,
            anim_ui: animation_ui::AnimUi::default(),
            dock: dock_ui::DockState::new(layout_path, layouts, layout_warnings),
            prop_tabs: HashMap::new(),
            tabs_shown: HashMap::new(),
            input: Default::default(),
            mview: multiview::MultiView::default(),
            keymap: std::sync::Arc::new(keymap),
            keymap_path,
            menu: menu_ui::MenuState { keymap_warnings, ..Default::default() },
        };
        app.restore_multiviews(&saved_views);
        app
    }

    pub(crate) fn lang(&self) -> Lang {
        i18n::lang()
    }

    pub(crate) fn session_state(&self) -> SessionState {
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
            layouts: Some(self.dock.snapshot()),
            multiviews: self.multiview_states(),
        }
    }

    /// Desfazer/refazer roteados pelo workspace ativo: Personagem fala com a sessão do núcleo,
    /// os demais com a `ModelingSession`.
    pub(crate) fn can_undo_redo(&self) -> (bool, bool) {
        if self.ws == WorkspaceId::Shading {
            return self.shading_can_undo_redo();
        }
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
        if self.ws == WorkspaceId::Shading {
            if let Some(msg) = self.shading_undo_redo(undo) {
                self.shading_ui.message = msg;
            }
            return;
        }
        if self.ws == WorkspaceId::Posing {
            self.pose_reset_transients();
        }
        if self.ws == WorkspaceId::Animacao {
            // Ctrl+Z/Y vão para a MESMA sessão; o clip/pose amostrados são reavaliados no quadro seguinte
            self.anim_reset_transients();
        }
        if let Some(msg) = character_ui::route_undo_redo(self.ws, &mut self.core, &mut self.modeling, undo) {
            if self.ws == WorkspaceId::Cenario {
                self.scene_ui.message = msg;
            } else if self.ws == WorkspaceId::Iluminacao {
                self.light_ui.message = msg;
            } else if self.ws == WorkspaceId::Render {
                self.render_ui.message = msg;
            } else if self.ws == WorkspaceId::Posing {
                self.pose_ui.message = msg;
            } else if self.ws == WorkspaceId::Animacao {
                self.anim_ui.message = msg;
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
                    view_tex: [None; 3],
                    view_cel: [None, None, None],
                    view_cel_tex: [None; 3],
                    cache: Vec::new(),
                    adapter_name: info.name.clone(),
                    backend: format!("{:?}", info.backend),
                    device: rs.device.clone(),
                    queue: rs.queue.clone(),
                    adapter: info,
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
                self.top_bar_logo(ui);
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

    /// Marca + nome no canto da barra superior (o lockup na resolução certa para o DPI).
    fn top_bar_logo(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let h = 20.0;
        let v = crate::brand::Variant::of(&ctx);
        if let Some(tex) = self.brand.lockup(&ctx, v, h * ctx.pixels_per_point()) {
            let w = h * crate::brand::lockup_aspect(&tex);
            ui.add(egui::Image::new(&tex).fit_to_exact_size(egui::vec2(w, h)));
        } else {
            ui.label(RichText::new("ANIGO").strong().size(15.0).color(theme::ACCENT));
        }
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
        if self.ws == WorkspaceId::Shading && w != WorkspaceId::Shading {
            self.shading_commit_now();
        }
        if self.ws == WorkspaceId::Animacao && w != WorkspaceId::Animacao {
            self.anim_leave();
        }
        if w == WorkspaceId::Animacao && self.ws != WorkspaceId::Animacao {
            self.anim_enter();
        }
        if w == WorkspaceId::Shading && self.ws != WorkspaceId::Shading {
            self.shading_on_enter(self.ws);
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
                } else if self.ws == WorkspaceId::Shading && !self.shading_ui.message.is_empty() {
                    self.shading_ui.message.clone()
                } else if self.ws == WorkspaceId::Posing && !self.pose_ui.message.is_empty() {
                    self.pose_ui.message.clone()
                } else if self.ws == WorkspaceId::Animacao && !self.anim_status_text().is_empty() {
                    self.anim_status_text()
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
                self.bridge_status_label(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(8.0);
                    // GPU/backend e métricas do cel são diagnóstico: ficam em Ajuda > Sobre, não na barra de status
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

    pub(crate) fn on_tool_selected(&mut self, id: &str) {
        if self.ws == WorkspaceId::Cenario {
            self.on_scene_tool_selected(id);
        }
        if self.ws == WorkspaceId::Iluminacao {
            self.on_light_tool_selected(id);
        }
        if self.ws == WorkspaceId::Shading {
            self.on_shading_tool_selected(id);
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

    /// Ações que a Biblioteca devolve ao app (abrir na Modelagem, status).
    fn library_actions(&mut self, actions: Vec<HostAction>) {
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

    /// O corpo do workspace atual: lógica de início de quadro (roteiros, atalhos), o dock com os painéis e a viewport,
    /// e a lógica de fim de quadro (diálogos, fechamento de gestos).
    fn workspace_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        match self.ws {
            WorkspaceId::Personagem => self.character_shortcuts(&ctx),
            WorkspaceId::Cenario => {
                self.scene_script_tick();
                self.scene_shortcuts(&ctx);
                self.scene_frame_begin();
            }
            WorkspaceId::Iluminacao => {
                self.light_script_tick();
                self.light_shortcuts(&ctx);
            }
            WorkspaceId::Render => {
                self.render_script_tick(&ctx);
                self.render_shortcuts(&ctx);
                self.render_frame_begin(&ctx);
            }
            WorkspaceId::Shading => {
                self.shading_script_tick();
                self.shading_shortcuts(&ctx);
                self.shading_frame_begin();
            }
            WorkspaceId::Posing => {
                self.posing_script_tick();
                self.posing_shortcuts(&ctx);
                self.pose_frame_begin();
            }
            WorkspaceId::Animacao => {
                self.anim_script_tick(&ctx);
                self.anim_tick(&ctx);
                self.animation_shortcuts(&ctx);
                self.anim_frame_begin();
            }
            WorkspaceId::Modelagem => {}
            WorkspaceId::Biblioteca => {
                self.lib_workspace().dock_begin(&ctx);
                self.lib_workspace().dock_strip(ui);
            }
        }
        self.dock_ui(ui, frame);
        match self.ws {
            WorkspaceId::Personagem => self.char_frame_end(&ctx),
            WorkspaceId::Cenario => self.scene_frame_end(&ctx),
            WorkspaceId::Shading => self.shading_frame_end(&ctx),
            WorkspaceId::Biblioteca => {
                let actions = self.lib_workspace().dock_end(&ctx);
                self.library_actions(actions);
            }
            _ => {}
        }
        // diálogo de abrir/salvar projeto (Personagem e menu Arquivo); o Cenário tem o próprio
        if self.ws != WorkspaceId::Cenario {
            self.char_project_dialog(&ctx);
        }
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.cli.screenshot.clone() else {
            return;
        };
        let library_busy = (self.ws == WorkspaceId::Biblioteca && self.frame_count < self.cli.frames as u64 + 3000 && self.lib_ws.as_ref().is_some_and(|w| !w.scenario_done()))
            || (self.ws == WorkspaceId::Cenario && self.frame_count < self.cli.frames as u64 + 3000 && self.scene_ui.script.as_ref().is_some_and(|s| !s.is_done()))
            || (self.ws == WorkspaceId::Iluminacao && self.frame_count < self.cli.frames as u64 + 3000 && self.light_ui.script.as_ref().is_some_and(|s| !s.is_done()))
            || (self.ws == WorkspaceId::Render && self.frame_count < self.cli.frames as u64 + 3000 && self.render_busy())
            || (self.ws == WorkspaceId::Shading && self.frame_count < self.cli.frames as u64 + 3000 && self.shading_ui.script.as_ref().is_some_and(|s| !s.is_done()))
            || (self.ws == WorkspaceId::Posing && self.frame_count < self.cli.frames as u64 + 3000 && self.pose_ui.script.as_ref().is_some_and(|s| !s.is_done()))
            || (self.ws == WorkspaceId::Animacao && self.frame_count < self.cli.frames as u64 + 3000 && self.anim_ui.script.as_ref().is_some_and(|s| !s.is_done()));
        if !self.screenshot_requested && self.frame_count >= self.cli.frames as u64 && !library_busy {
            self.screenshot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let shots: Vec<_> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    // capturas pedidas pela bridge (user_data presente) não são do `--screenshot`
                    egui::Event::Screenshot { image, user_data, .. } if user_data.data.is_none() => Some(image.clone()),
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
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.input.next_events(raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.frame_count += 1;
        self.ensure_gpu(frame);
        self.poll_hot_reload();
        self.run_scenario();
        self.bridge_tick(&ui.ctx().clone());

        self.menu_bar(ui);
        self.top_bar(ui);
        self.status_bar(ui);
        if self.ws.has_viewport() {
            self.tool_shelf(ui);
        }
        self.workspace_ui(ui, frame);
        let ctx = ui.ctx().clone();
        self.handle_screenshot(&ctx);
        self.splash.show(&ctx, &mut self.brand, tr("splash.loading"), &format!("v{}", env!("CARGO_PKG_VERSION")));
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
        self.anim_cleanup();
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

/// Chaves de i18n das abas de origem do Shading (para o teste de tradução).
#[cfg(test)]
pub(crate) fn shading_source_keys() -> Vec<&'static str> {
    shading_ui::Source::ALL.iter().map(|s| s.key()).collect()
}
