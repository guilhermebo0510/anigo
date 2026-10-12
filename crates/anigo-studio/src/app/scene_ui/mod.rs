//! Workspace Cenário nativo (estilo editor de níveis do Unreal): monta cenários com ativos da
//! Biblioteca — posicionar, mover, girar, escalar, duplicar, apagar, agrupar — com SNAP e
//! desfazer/refazer completos.
//!
//! Toda mudança persistente é um `Command` do núcleo (`StudioApp::core`, a MESMA `CoreSession` do
//! Personagem); a viewport só guarda estado transitório (câmera, seleção, arrasto). Um arrasto de gizmo
//! usa `CoreSession::preview` (ao vivo, fora do histórico) e vira UM passo de desfazer ao soltar.
//!
//! * `viewport` — navegação, seleção, gizmo, soltar da Biblioteca, render e overlays;
//! * `panels` — Outliner, Histórico, Details, Colocar atores, Content Drawer;
//! * `actions` — operações sobre a seleção (duplicar, apagar, agrupar, copiar/colar…);
//! * `script` — cenários de captura (`--scenario cenario-*`).
//!
//! A matemática (snap, gizmo, raycast, colocação) mora em `crate::scene` e tem testes unitários.

mod actions;
mod panels;
mod script;
mod viewport;

use super::StudioApp;
use crate::scene::camview::SceneViewKind;
use crate::scene::geometry::{LoadedGeometry, MeshCache};
use crate::scene::gizmo::{GizmoMode, GizmoSpace, NodeXf};
use crate::scene::outliner::Expansion;
use crate::scene::runtime::SceneRuntime;
use crate::scene::selection::Selection;
use crate::scene::snap::SnapSettings;
use crate::viewport::OrbitCamera;
use crate::workbench::GpuMesh;
use anigo_core::command::Command;
use anigo_core::ids::NodeId;
use anigo_core::level::ClipboardNodes;
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) use script::is_scene_scenario;
#[allow(unused_imports)]
pub use script::SCENE_SCENARIOS;

/// Modo de sombreamento do Cenário.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SceneShading {
    /// Cel canônico (o mesmo renderer do render final) + overlays do Workbench.
    #[default]
    Cel,
    Solid,
    Wire,
}

/// Guias de composição sobre a viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guides {
    pub thirds: bool,
    pub center: bool,
    pub safe: bool,
    pub frame: bool,
}

impl Default for Guides {
    fn default() -> Self {
        Self { thirds: true, center: false, safe: false, frame: true }
    }
}

/// Preferências do Cenário que sobrevivem ao reinício (vão em `session.json`; não são do projeto).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenePrefs {
    pub snap: SnapSettings,
    pub tool: GizmoMode,
    pub space: GizmoSpace,
    pub shading: SceneShading,
    pub guides: Guides,
    pub game_view: bool,
    pub speed: f32,
    pub drawer_open: bool,
    pub place_open: bool,
}

/// Alvo do painel direito (aba de cima).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RightTab {
    #[default]
    Outliner,
    History,
}

/// Arrastar da lista "Colocar atores".
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PlaceKind {
    Primitive(anigo_core::PrimitiveShape),
    Camera,
    Group,
}

#[derive(Debug, Clone)]
pub(crate) struct PlaceDragPayload(pub PlaceKind);

/// Arrastar de linhas do Outliner (reparentar / mover para pasta).
#[derive(Debug, Clone)]
pub(crate) struct OutlinerDragPayload(pub Vec<NodeId>);

/// Arrasto de gizmo em andamento.
pub(crate) struct GizmoSession {
    pub drag: crate::scene::gizmo::GizmoDrag,
    pub xf: Vec<NodeXf>,
    pub pivot: Vec3,
    /// Comando de duplicação (Alt+arrastar) que entra junto no passo final.
    pub dup: Option<Command>,
    pub last: Option<Command>,
    pub label: String,
    pub moved: bool,
    pub start_sel: Vec<NodeId>,
    /// Caixa mundial da seleção no início (surface snapping: a base da caixa pousa na superfície).
    pub start_bounds: Option<crate::scene::bvh::Aabb>,
}

/// Retângulo de seleção em andamento.
pub(crate) struct Marquee {
    pub start: Vec2,
    pub cur: Vec2,
    pub additive: bool,
}

/// Prévia do soltar (fantasma) sobre a viewport.
#[derive(Clone)]
pub(crate) struct Ghost {
    pub items: Vec<GhostItem>,
}

#[derive(Clone)]
pub(crate) struct GhostItem {
    pub geom: Option<Arc<LoadedGeometry>>,
    pub transform: anigo_core::Transform,
    pub name: String,
    pub loading: bool,
    pub error: Option<String>,
}

/// Instâncias do Workbench por nó (compartilham os buffers do ativo).
#[derive(Default)]
pub(crate) struct SceneGpu {
    pub base: HashMap<String, GpuMesh>,
    pub inst: HashMap<NodeId, (String, GpuMesh)>,
    pub ghost: Vec<GpuMesh>,
    pub placeholder: Option<GpuMesh>,
}

pub(crate) struct SceneDialog {
    pub save: bool,
    pub path: String,
    pub error: Option<String>,
}

pub(crate) struct SceneUi {
    // ---- câmera e visão
    pub camera: OrbitCamera,
    pub framed: bool,
    pub shading: SceneShading,
    pub game_view: bool,
    pub camera_view: bool,
    pub guides: Guides,
    pub speed: f32,
    pub fly_active: bool,
    pub view_kind: SceneViewKind,
    // ---- ferramenta e snap
    pub tool: GizmoMode,
    pub space: GizmoSpace,
    pub snap: SnapSettings,
    // ---- seleção e interação
    pub selection: Selection,
    pub session: Option<GizmoSession>,
    pub marquee: Option<Marquee>,
    pub hover_handle: Option<crate::scene::gizmo::Handle>,
    pub ghost: Option<Ghost>,
    /// Fantasma fixado (cenários de captura): a viewport não o apaga quando não há arrasto.
    pub ghost_pinned: bool,
    // ---- derivados
    pub cache: MeshCache,
    pub runtime: SceneRuntime,
    pub char_geom: Option<(u64, Arc<LoadedGeometry>)>,
    pub gpu: SceneGpu,
    pub view_size: Vec2,
    // ---- painéis
    pub right_tab: RightTab,
    pub query: String,
    pub expansion: Expansion,
    pub rename: Option<(NodeId, String, bool)>,
    pub drawer_open: bool,
    pub place_open: bool,
    pub clipboard: Option<ClipboardNodes>,
    pub tag_input: String,
    pub scale_lock: bool,
    pub dialog: Option<SceneDialog>,
    pub last_path: String,
    pub message: String,
    pub hud: String,
    pub message_seen: (String, Option<std::time::Instant>),
    /// Edição numérica em andamento (Details): último comando ao vivo.
    pub gesture: Option<Command>,
    pub new_folder: String,
    // ---- cenários de captura
    pub script: Option<script::Script>,
    pub script_assets: Option<crate::library_ui::synth::ScenarioAssets>,
    pub history_limit_set: bool,
    pub pending_focus: bool,
    /// Última linha de ANIGO_TRACE_SCENE (só imprime mudanças).
    pub trace: Option<String>,
    /// Direção de luz que SUBSTITUI a do projeto só no preview (luz presa à câmera do workspace Iluminação).
    pub light_dir_override: Option<[f32; 3]>,
}

impl Default for SceneUi {
    fn default() -> Self {
        Self {
            camera: OrbitCamera { target: Vec3::new(0.0, 1.0, 0.0), yaw: 0.55, pitch: 0.38, distance: 9.0, far: 600.0, ..OrbitCamera::default() },
            framed: false,
            shading: SceneShading::Cel,
            game_view: false,
            camera_view: false,
            guides: Guides::default(),
            speed: 1.0,
            fly_active: false,
            view_kind: SceneViewKind::Perspective,
            tool: GizmoMode::Translate,
            space: GizmoSpace::World,
            snap: SnapSettings::default(),
            selection: Selection::default(),
            session: None,
            marquee: None,
            hover_handle: None,
            ghost: None,
            ghost_pinned: false,
            cache: MeshCache::new(),
            runtime: SceneRuntime::default(),
            char_geom: None,
            gpu: SceneGpu::default(),
            view_size: Vec2::new(900.0, 600.0),
            right_tab: RightTab::Outliner,
            query: String::new(),
            expansion: Expansion::default(),
            rename: None,
            drawer_open: true,
            place_open: true,
            clipboard: None,
            tag_input: String::new(),
            scale_lock: true,
            dialog: None,
            last_path: "cenario.anigo".to_string(),
            message: String::new(),
            hud: String::new(),
            message_seen: (String::new(), None),
            gesture: None,
            new_folder: String::new(),
            script: None,
            script_assets: None,
            history_limit_set: false,
            pending_focus: false,
            trace: None,
            light_dir_override: None,
        }
    }
}

impl SceneUi {
    pub fn prefs(&self) -> ScenePrefs {
        ScenePrefs {
            snap: self.snap,
            tool: self.tool,
            space: self.space,
            shading: self.shading,
            guides: self.guides,
            game_view: self.game_view,
            speed: self.speed,
            drawer_open: self.drawer_open,
            place_open: self.place_open,
        }
    }

    pub fn apply_prefs(&mut self, p: &ScenePrefs) {
        self.snap = p.snap;
        self.tool = p.tool;
        self.space = p.space;
        self.shading = p.shading;
        self.guides = p.guides;
        self.game_view = p.game_view;
        self.speed = p.speed.clamp(0.1, 16.0);
        self.drawer_open = p.drawer_open;
        self.place_open = p.place_open;
    }

    /// Pivô do gizmo: mediana das posições mundiais dos nós selecionados (ou a do ativo).
    pub fn pivot(&self, ids: &[NodeId]) -> Option<Vec3> {
        if ids.is_empty() {
            return None;
        }
        let mut sum = Vec3::ZERO;
        for id in ids {
            sum += self.runtime.position_of(id);
        }
        Some(sum / ids.len() as f32)
    }
}

impl StudioApp {
    /// Troca a ferramenta (Q/W/E/R) e mantém o shelf em sincronia.
    pub(crate) fn set_scene_tool(&mut self, tool: GizmoMode) {
        self.scene_ui.tool = tool;
        let idx = match tool {
            GizmoMode::Select => 0,
            GizmoMode::Translate => 1,
            GizmoMode::Rotate => 2,
            GizmoMode::Scale => 3,
        };
        self.tool_idx.insert(crate::workspace::WorkspaceId::Cenario, idx);
    }

    pub(crate) fn on_scene_tool_selected(&mut self, id: &str) {
        let tool = match id {
            "select_box" => GizmoMode::Select,
            "move" => GizmoMode::Translate,
            "rotate" => GizmoMode::Rotate,
            "scale" => GizmoMode::Scale,
            _ => return,
        };
        self.set_scene_tool(tool);
    }

    /// Mensagem de status curta do workspace.
    pub(crate) fn scene_say(&mut self, msg: impl Into<String>) {
        self.scene_ui.message = msg.into();
    }

    /// Aplica um comando do núcleo e reporta o resultado na barra de status. `true` se foi aceito.
    pub(crate) fn scene_apply(&mut self, cmd: Command) -> bool {
        match self.core.apply(cmd) {
            Ok(o) => {
                self.scene_ui.message = describe_scene(&o.description);
                true
            }
            Err(e) => {
                if !e.contains("no effect") && !e.contains("already") {
                    self.scene_ui.message = format!("{}: {e}", crate::i18n::tr("char.error"));
                }
                false
            }
        }
    }

    /// Entrada no workspace: profundidade de histórico própria do editor de cenário e Biblioteca aberta.
    pub(crate) fn scene_enter(&mut self) {
        if !self.scene_ui.history_limit_set {
            self.core.set_history_limit(400);
            self.scene_ui.history_limit_set = true;
        }
        let tool = self.scene_ui.tool;
        self.set_scene_tool(tool);
        let w = self.lib_workspace();
        w.ensure_open();
        w.set_drawer_defaults();
    }

    /// Recalcula o estado derivado (matrizes, BVH, geometria) quando o projeto ou o cache mudou.
    pub(crate) fn scene_refresh(&mut self) {
        let lib = self.lib_ws.as_ref().and_then(|w| w.library().cloned());
        let su = &mut self.scene_ui;
        if let Some(w) = self.lib_ws.as_ref().map(|w| w.wake.clone()) {
            su.cache.set_wake(w);
        }
        su.cache.poll();
        let key = (
            self.core.dynamic_revision(),
            self.core.static_revision(),
            self.core.history().revision(),
            su.cache.generation,
            lib.is_some() as u64 + su.cache.loading_count() as u64 * 2,
        );
        if su.runtime.key == Some(key) {
            return;
        }
        let ckey = {
            let c = format!("{:?}", self.core.project().character);
            anigo_core::fnv1a64(&c) ^ self.core.static_revision()
        };
        if su.char_geom.as_ref().map(|c| c.0) != Some(ckey) {
            match self.core.deformed_mesh() {
                Ok(mesh) => {
                    if let Some(g) = LoadedGeometry::new(format!("char:{ckey:x}"), mesh) {
                        su.char_geom = Some((ckey, g));
                    }
                }
                Err(e) => su.message = format!("personagem: {e}"),
            }
        }
        su.runtime.rebuild(self.core.project(), &mut su.cache, lib.as_ref(), su.char_geom.as_ref().map(|c| &c.1));
        su.runtime.key = Some(key);
        su.selection.retain_existing(self.core.project());
        if !su.framed {
            if let Some(b) = su.runtime.bounds {
                crate::scene::camview::frame_box(&mut su.camera, b.min, b.max);
                su.camera.distance = (su.camera.distance * 0.85).clamp(4.0, 60.0);
                su.framed = true;
            }
        }
    }

    /// Ids selecionados que ainda existem e não estão bloqueados (alvo de operações de edição).
    pub(crate) fn scene_editable_selection(&self) -> Vec<NodeId> {
        let p = self.core.project();
        self.scene_ui.selection.ids().iter().filter(|i| p.scene.node(i).is_some_and(|n| !n.locked)).cloned().collect()
    }

    /// Desfazer/refazer/atalhos do Cenário (fora de campos de texto).
    pub(crate) fn scene_shortcuts(&mut self, ctx: &egui::Context) {
        self.character_shortcuts(ctx);
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        self.scene_hotkeys(ctx);
    }
}

/// Descrição de um comando do núcleo no idioma da UI (histórico e status).
pub(crate) fn describe_scene(desc: &str) -> String {
    use crate::i18n::tr;
    let (undo, redo) = (desc.strip_prefix("Undo "), desc.strip_prefix("Redo "));
    if let Some(rest) = undo {
        return format!("{}: {}", tr("char.undone"), describe_scene(rest));
    }
    if let Some(rest) = redo {
        return format!("{}: {}", tr("char.redone"), describe_scene(rest));
    }
    let key = |k: &str| tr(k).to_string();
    if let Some(rest) = desc.strip_prefix("Transform ") {
        if let Some(n) = rest.strip_suffix(" nodes") {
            return format!("{} ({n})", key("scene.cmd.transform"));
        }
        return format!("{}: {}", key("scene.cmd.transform"), rest.trim_start_matches("nod_"));
    }
    if desc.starts_with("Duplicate ") {
        return key("scene.cmd.duplicate");
    }
    if desc.starts_with("Remove node") {
        return key("scene.cmd.delete");
    }
    if desc.starts_with("Add node") {
        return key("scene.cmd.add");
    }
    if desc.starts_with("Rename ") {
        return key("scene.cmd.rename");
    }
    if desc.starts_with("Lock ") {
        return key("scene.cmd.lock");
    }
    if desc.starts_with("Properties ") {
        return key("scene.cmd.properties");
    }
    if desc.starts_with("Parent ") || desc.starts_with("Unparent ") {
        return key("scene.cmd.reparent");
    }
    if desc == "Visibility" {
        return key("scene.cmd.visibility");
    }
    if desc.starts_with("Replace asset") {
        return key("scene.cmd.replace");
    }
    if desc.starts_with("Assets ") {
        return key("scene.cmd.assets");
    }
    if desc == "Camera" {
        return key("scene.cmd.camera");
    }
    if desc == "Lighting" {
        return key("scene.cmd.light");
    }
    if desc == "Background" {
        return key("scene.cmd.background");
    }
    if desc.starts_with("Batch") {
        return key("scene.cmd.batch");
    }
    super::character_ui::describe(desc)
}

/// Matriz de uma lista de translação/rotação/escala em linhas legíveis (HUD do arrasto).
pub(crate) fn fmt_vec3(v: Vec3) -> String {
    format!("{:.2}, {:.2}, {:.2}", v.x, v.y, v.z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::level::{self, PrimitiveShape};
    use anigo_core::Transform;
    use std::collections::BTreeSet;

    /// Todo comando que o Cenário emite tem descrição traduzida (nenhum texto cru em inglês no Histórico).
    #[test]
    fn history_descriptions_of_real_scene_commands_are_localised() {
        let mut core = anigo_session::CoreSession::new();
        let (add, id) = level::plan_place_primitive(core.project(), PrimitiveShape::Cube, "Caixa", Transform::default(), &BTreeSet::new());
        core.apply(add).unwrap();
        let (add2, id2) = level::plan_place_primitive(core.project(), PrimitiveShape::Sphere, "Esfera", Transform::default(), &BTreeSet::new());
        core.apply(add2).unwrap();
        let steps: Vec<Command> = vec![
            level::plan_set_locked(core.project(), &[id.clone()], true).unwrap(),
            level::plan_set_visible(core.project(), &[id.clone()], false).unwrap(),
            level::plan_duplicate(core.project(), &[id.clone()], None).unwrap().0,
            level::plan_group(core.project(), &[id.clone(), id2.clone()]).unwrap().0,
        ];
        for c in steps {
            let raw = c.description();
            let out = describe_scene(&raw);
            assert_ne!(out, raw, "descrição sem tradução: {raw}");
            assert!(!out.is_empty());
        }
        let del = level::plan_delete(core.project(), &[id2]).unwrap();
        let raw = del.description();
        assert_ne!(describe_scene(&raw), raw, "descrição sem tradução: {raw}");
    }

    #[test]
    fn undo_and_redo_messages_wrap_the_translated_description() {
        let out = describe_scene("Undo Rename nod_x");
        assert!(out.contains(&*crate::i18n::tr("char.undone")), "{out}");
        let out = describe_scene("Redo Duplicate 2 nodes");
        assert!(out.contains(&*crate::i18n::tr("char.redone")), "{out}");
    }
}
