//! Workspace Posing nativo: posar o personagem 3D (FK, IK, olhar, mãos, pés no chão) com undo/redo completos.
//!
//! Toda mudança persistente é um `Command::SetPose` do núcleo (`StudioApp::core`, a MESMA `CoreSession` do
//! Personagem e do Cenário): o personagem renderizado é o do renderer canônico, deformado pela paleta de
//! skinning que a sessão calcula a partir da pose. Um arrasto usa `CoreSession::preview` (ao vivo, fora do
//! histórico) e vira UM passo de desfazer ao soltar (`commit_preview`).
//!
//! * `viewport` — navegação, seleção de juntas, gizmo FK, handles de IK, overlays e render;
//! * `panels` — árvore de ossos, painel de osso, IK/olhar/pés/equilíbrio, presets, mãos e Biblioteca de poses;
//! * `script` — cenários de captura (`--scenario posing-*`).
//!
//! A matemática (edição de pose, IK, picking, limites) mora em `crate::posing` e em `anigo_core::pose` /
//! `anigo_ik::pose_ik`, com testes numéricos.

mod panels;
mod script;
mod viewport;

use super::StudioApp;
use crate::library_ui::pose_bridge::PoseCard;
use crate::posing::edit::EditOpts;
use crate::posing::GestureTracker;
use crate::scene::gizmo::GizmoDrag;
use crate::viewport::OrbitCamera;
use anigo_core::command::Command;
use anigo_core::pose::{bone, Limb, LimitProfile, PoseOp, PoseRig, PoseState, BONE_COUNT};
use glam::{Mat4, Vec2, Vec3};
use std::collections::BTreeSet;

pub(crate) use script::{is_posing_scenario, temp_library};
pub(crate) use viewport::PoseView;
#[allow(unused_imports)]
pub use script::POSING_SCENARIOS;

/// O que está sob o cursor / sendo arrastado.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Hot {
    /// Anel do gizmo FK do osso ativo (eixo 0/1/2).
    Ring(crate::scene::gizmo::Axis3),
    Effector(Limb),
    Pole(Limb),
    Hips,
    Gaze,
    /// Cabeça arrastando a coluna (FABRIK).
    Head,
}

/// Arrasto em curso.
pub(crate) enum DragKind {
    Fk { bone: usize },
    Effector { limb: Limb, pole: Vec3, start_target: Vec3 },
    Pole { limb: Limb, target: Vec3, start_pole: Vec3 },
    Hips { whole_body: bool },
    Gaze { start_target: Vec3 },
    Head { start_target: Vec3 },
}

pub(crate) struct PoseDrag {
    pub kind: DragKind,
    pub gizmo: GizmoDrag,
    pub tracker: GestureTracker,
    /// Eixo travado pelas teclas X/Y/Z durante o arrasto de translação.
    pub axis_lock: Option<usize>,
}

/// Retângulo de seleção em andamento.
pub(crate) struct Marquee {
    pub start: Vec2,
    pub cur: Vec2,
    pub additive: bool,
}

/// Alvo do olhar (arrastável).
#[derive(Debug, Clone, Copy)]
pub(crate) struct GazeUi {
    pub enabled: bool,
    /// Posição no espaço do personagem.
    pub target: Vec3,
    pub weight: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HandSide {
    Left,
    Right,
    Both,
}

pub(crate) struct PosingUi {
    // ---- câmera e visão
    pub camera: OrbitCamera,
    pub framed: bool,
    pub view_size: Vec2,
    pub show_mesh: bool,
    pub show_skeleton: bool,
    pub show_balance: bool,
    pub show_labels: bool,
    // ---- seleção
    pub selection: BTreeSet<usize>,
    pub active: Option<usize>,
    pub hover_bone: Option<usize>,
    pub hover_hot: Option<Hot>,
    pub marquee: Option<Marquee>,
    // ---- edição
    pub space_local: bool,
    pub snap_deg: Option<f32>,
    pub symmetry: bool,
    pub profile: LimitProfile,
    pub show_limits: bool,
    pub limb_ik: [bool; 4],
    pub foot_lock: bool,
    pub auto_ground: bool,
    /// A cabeça vira um handle de IK de cadeia longa (FABRIK sobre a coluna).
    pub spine_ik: bool,
    pub gaze: GazeUi,
    pub drag: Option<PoseDrag>,
    /// Edição numérica / botões em andamento (commit ao soltar o ponteiro).
    pub num_gesture: Option<GestureTracker>,
    pub clipboard: Option<(PoseState, BTreeSet<usize>)>,
    // ---- biblioteca / presets
    pub hand_side: HandSide,
    pub lib_name: String,
    pub lib_only_selected: bool,
    pub poses: Vec<PoseCard>,
    pub poses_loaded: bool,
    pub blend: f32,
    pub apply_only_selected: bool,
    pub preset_status: String,
    // ---- árvore
    pub tree_filter: String,
    // ---- textos
    pub message: String,
    pub hud: String,
    pub message_seen: (String, Option<std::time::Instant>),
    // ---- cenários / smoke
    pub script: Option<script::Script>,
    pub trace: bool,
    pub last_trace: String,
}

impl Default for PosingUi {
    fn default() -> Self {
        Self {
            camera: OrbitCamera { target: Vec3::new(0.0, 0.9, 0.0), yaw: 0.5, pitch: 0.1, distance: 3.6, ..OrbitCamera::default() },
            framed: false,
            view_size: Vec2::new(900.0, 700.0),
            show_mesh: true,
            show_skeleton: true,
            show_balance: false,
            show_labels: true,
            selection: BTreeSet::new(),
            active: None,
            hover_bone: None,
            hover_hot: None,
            marquee: None,
            space_local: false,
            snap_deg: None,
            symmetry: false,
            profile: LimitProfile::Human,
            show_limits: true,
            limb_ik: [false; 4],
            foot_lock: false,
            auto_ground: false,
            spine_ik: false,
            gaze: GazeUi { enabled: false, target: Vec3::new(0.0, 1.55, 1.0), weight: 1.0 },
            drag: None,
            num_gesture: None,
            clipboard: None,
            hand_side: HandSide::Both,
            lib_name: String::new(),
            lib_only_selected: false,
            poses: Vec::new(),
            poses_loaded: false,
            blend: 1.0,
            apply_only_selected: false,
            preset_status: String::new(),
            tree_filter: String::new(),
            message: String::new(),
            hud: String::new(),
            message_seen: (String::new(), None),
            script: None,
            trace: std::env::var_os("ANIGO_TRACE_POSING").is_some(),
            last_trace: String::new(),
        }
    }
}

pub(crate) fn limb_index(l: Limb) -> usize {
    Limb::ALL.iter().position(|x| *x == l).unwrap_or(0)
}

impl PosingUi {
    pub fn is_ik(&self, l: Limb) -> bool {
        self.limb_ik[limb_index(l)]
    }

    pub fn edit_opts(&self) -> EditOpts {
        EditOpts { profile: self.profile, symmetry: self.symmetry, snap_deg: self.snap_deg }
    }

    pub fn select_one(&mut self, b: usize) {
        self.selection.clear();
        self.selection.insert(b);
        self.active = Some(b);
    }

    pub fn toggle(&mut self, b: usize) {
        if !self.selection.remove(&b) {
            self.selection.insert(b);
            self.active = Some(b);
        } else if self.active == Some(b) {
            self.active = self.selection.iter().next_back().copied();
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
        self.active = None;
    }

    /// Ossos que a seleção "afeta" para copiar/colar/salvar: a seleção, ou todos se vazia.
    pub fn mask_or_all(&self) -> BTreeSet<usize> {
        if self.selection.is_empty() {
            (0..BONE_COUNT).collect()
        } else {
            self.selection.clone()
        }
    }
}

/// Transformação do nó do personagem no mundo (a viewport posa o personagem onde ele está no projeto).
pub(crate) fn character_xf(core: &anigo_session::CoreSession) -> Mat4 {
    core.project().character_node().map(|n| Mat4::from_scale_rotation_translation(n.transform.scale, n.transform.rotation, n.transform.translation)).unwrap_or(Mat4::IDENTITY)
}

/// Descrição de comando do núcleo no idioma da UI (histórico/status).
pub(crate) fn describe_pose(desc: &str) -> String {
    use crate::i18n::tr;
    let (undo, redo) = (desc.strip_prefix("Undo "), desc.strip_prefix("Redo "));
    if let Some(rest) = undo {
        return format!("{}: {}", tr("char.undone"), describe_pose(rest));
    }
    if let Some(rest) = redo {
        return format!("{}: {}", tr("char.redone"), describe_pose(rest));
    }
    if let Some(op) = desc.strip_prefix("Pose ") {
        let key = format!("pose.op.{op}");
        let label = tr(&key);
        if label != key {
            return format!("{}: {}", tr("pose.history.pose"), label);
        }
        // "Pose LeftElbow": nome do osso
        let bkey = format!("pose.bone.{op}");
        let b = tr(&bkey);
        if b != bkey {
            return format!("{}: {}", tr("pose.history.pose"), b);
        }
    }
    super::character_ui::describe(desc)
}

impl StudioApp {
    /// Descrição de histórico no idioma da UI; na Animação também alimenta a barra de status do workspace.
    pub(crate) fn pose_describe(&mut self, desc: &str) -> String {
        if self.ws == crate::workspace::WorkspaceId::Animacao {
            let d = super::animation_ui::describe_anim(desc);
            self.anim_ui.message = d.clone();
            return d;
        }
        describe_pose(desc)
    }

    /// Pose atual (cópia barata: poucos ossos).
    pub(crate) fn pose_now(&self) -> PoseState {
        self.core.pose().clone()
    }

    /// Rig atual (cache da sessão); em caso de erro, o canônico.
    pub(crate) fn pose_rig(&mut self) -> PoseRig {
        self.core.rig().unwrap_or_default()
    }

    pub(crate) fn pose_say(&mut self, msg: impl Into<String>) {
        self.pose_ui.message = msg.into();
    }

    /// "Sem efeito" não é erro para o usuário; o resto aparece na barra de status.
    pub(crate) fn pose_report(&mut self, e: String) {
        if !e.contains("no effect") && !e.contains("already") && !e.contains("matches") {
            self.pose_ui.message = format!("{}: {e}", crate::i18n::tr("char.error"));
        }
    }

    /// Comando discreto (botão): vai direto ao histórico.
    pub(crate) fn pose_apply(&mut self, cmd: Command) {
        // Animação com auto-key: a edição de pose vira `Batch[SetPose, InsertKeys]` (um passo de histórico)
        let cmd = self.anim_wrap_pose_cmd(cmd);
        match self.core.apply(cmd) {
            Ok(o) => self.pose_ui.message = self.pose_describe(&o.description),
            Err(e) => self.pose_report(e),
        }
    }

    /// Planeja e aplica `target` a partir da pose atual como UM passo de histórico.
    pub(crate) fn pose_commit_to(&mut self, target: PoseState, op: PoseOp) {
        let now = self.pose_now();
        if let Some(cmd) = anigo_core::pose::plan_pose_change(&now, &target, op) {
            self.pose_apply(cmd);
        }
    }

    /// Passo de edição contínua que não vem do mouse no viewport (painel numérico, sliders): preview ao vivo e
    /// UM passo de desfazer quando o ponteiro é solto (`posing_finish_gesture`).
    pub(crate) fn pose_live(&mut self, target: PoseState, op: PoseOp) {
        if self.pose_ui.num_gesture.is_none() {
            self.core.begin_preview();
            self.pose_ui.num_gesture = Some(GestureTracker::begin(self.pose_now(), op));
        }
        let now = self.pose_now();
        let Some(g) = self.pose_ui.num_gesture.as_mut() else { return };
        if let Some(cmd) = g.step(&now, target) {
            if let Err(e) = self.core.preview(cmd) {
                self.pose_report(e);
            }
        }
    }

    /// Fecha a edição numérica quando o ponteiro é solto.
    pub(crate) fn posing_finish_gesture(&mut self, ctx: &egui::Context) {
        if self.pose_ui.num_gesture.is_none() || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        if let Some(g) = self.pose_ui.num_gesture.take() {
            match g.finish().map(|c| self.anim_wrap_pose_cmd(c)) {
                Some(cmd) => match self.core.commit_preview(cmd) {
                    Ok(o) => self.pose_ui.message = self.pose_describe(&o.description),
                    Err(e) => self.pose_report(e),
                },
                None => self.core.cancel_preview(),
            }
        }
    }

    /// Ctrl+Z / Ctrl+Y / Esc / Delete e atalhos do Posing (fora de campos de texto).
    pub(crate) fn posing_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        use egui::{Key, Modifiers};
        let redo = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || i.consume_key(Modifiers::COMMAND, Key::Y));
        if redo {
            self.undo_redo(false);
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Z)) {
            self.undo_redo(true);
            return;
        }
        // Ctrl+C / Ctrl+V chegam como eventos de área de transferência no egui; Shift junto = colar espelhado
        let (copy, paste, shift) = ctx.input(|i| {
            (i.events.iter().any(|e| matches!(e, egui::Event::Copy)), i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))), i.modifiers.shift)
        });
        if copy || ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::C)) {
            self.pose_copy();
        }
        if paste || ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::V)) {
            self.pose_paste(shift);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::V)) {
            self.pose_paste(true);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::A)) {
            self.pose_ui.selection = (0..BONE_COUNT).filter(|b| *b != bone::ROOT).collect();
            self.pose_ui.active = self.pose_ui.selection.iter().next().copied();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::ALT, Key::R)) {
            self.pose_reset_selection();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::ALT | Modifiers::SHIFT, Key::R)) {
            self.pose_reset_all();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::M)) {
            self.pose_mirror();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F)) {
            self.pose_ui.framed = false;
        }
    }

    // ------------------------------------------------------------------ ações

    pub(crate) fn pose_reset_selection(&mut self) {
        let sel = self.pose_ui.mask_or_all();
        let target = crate::posing::edit::reset_bones(&self.pose_now(), &sel);
        self.pose_commit_to(target, PoseOp::Reset);
    }

    pub(crate) fn pose_reset_all(&mut self) {
        self.pose_commit_to(PoseState::default(), PoseOp::Reset);
    }

    pub(crate) fn pose_reset_bone(&mut self, b: usize) {
        let sel: BTreeSet<usize> = [b].into_iter().collect();
        let mut target = crate::posing::edit::reset_bones(&self.pose_now(), &sel);
        if self.pose_ui.symmetry {
            let m = anigo_core::pose::mirror_bone(b);
            target.set_rotation(m, glam::Quat::IDENTITY);
        }
        self.pose_commit_to(target, PoseOp::Reset);
    }

    pub(crate) fn pose_mirror(&mut self) {
        let now = self.pose_now();
        let sel = self.pose_ui.selection.clone();
        let target = if sel.is_empty() {
            now.mirrored()
        } else {
            // espelha só a seleção: o lado oposto recebe o reflexo, o resto fica
            let m = now.mirrored();
            let eff: BTreeSet<usize> = sel.iter().flat_map(|b| [*b, anigo_core::pose::mirror_bone(*b)]).collect();
            now.overlay(&m, &eff)
        };
        self.pose_commit_to(target, PoseOp::Mirror);
    }

    pub(crate) fn pose_copy(&mut self) {
        let mask = self.pose_ui.mask_or_all();
        let n = mask.len();
        self.pose_ui.clipboard = Some((self.pose_now().masked(&mask), mask));
        self.pose_say(format!("{} ({n})", crate::i18n::tr("pose.copied")));
    }

    pub(crate) fn pose_paste(&mut self, flipped: bool) {
        let Some((clip, mask)) = self.pose_ui.clipboard.clone() else {
            self.pose_say(crate::i18n::tr("pose.clipboard_empty"));
            return;
        };
        let target = crate::posing::edit::paste_pose(&self.pose_now(), &clip, &mask, flipped);
        self.pose_commit_to(target, PoseOp::Paste);
    }

    /// Aplica um preset de corpo (T, A, em pé, sentado, andando, ação).
    pub(crate) fn pose_apply_body_preset(&mut self, key: &str) {
        let rig = self.pose_rig();
        let Some(preset) = anigo_core::pose::presets::body(key, &rig) else { return };
        // mantém as mãos da pose atual quando o preset não as define
        let mut target = preset;
        if target.hands.is_rest() {
            target.hands = self.pose_now().hands;
        }
        self.pose_commit_to(target, PoseOp::Preset);
    }

    pub(crate) fn pose_apply_hand_preset(&mut self, key: &str) {
        let Some(curls) = anigo_core::pose::presets::hand(key) else { return };
        let mut p = self.pose_now();
        let side = self.pose_ui.hand_side;
        if matches!(side, HandSide::Left | HandSide::Both) {
            p.hands.left = curls;
        }
        if matches!(side, HandSide::Right | HandSide::Both) {
            p.hands.right = curls;
        }
        self.pose_commit_to(p, PoseOp::Hand);
    }

    /// Apoia o pé mais baixo no chão agora.
    pub(crate) fn pose_ground_now(&mut self) {
        let rig = self.pose_rig();
        let t = rig.grounded(&self.pose_now(), 0.0);
        self.pose_commit_to(t, PoseOp::Ground);
    }

    /// Salva a pose atual (ou só os ossos selecionados) na Biblioteca.
    pub(crate) fn pose_save_to_library(&mut self) {
        let name = self.pose_ui.lib_name.trim().to_string();
        let name = if name.is_empty() { crate::i18n::tr("pose.lib.default_name").to_string() } else { name };
        let rig = self.pose_rig();
        let pose = self.pose_now();
        let mask = if self.pose_ui.lib_only_selected && !self.pose_ui.selection.is_empty() { self.pose_ui.selection.clone() } else { (0..BONE_COUNT).collect() };
        let asset = anigo_core::pose::PoseAsset::from_mask(name.clone(), &pose, &mask, &rig);
        let mut asset = asset;
        if mask.len() == BONE_COUNT {
            asset.bones.clear();
        }
        match self.lib_workspace().save_pose_asset(&asset) {
            Ok(msg) => {
                self.pose_ui.preset_status = msg.clone();
                self.pose_say(msg);
                self.pose_refresh_library();
            }
            Err(e) => {
                self.pose_say(format!("{}: {e}", crate::i18n::tr("pose.lib.error")));
            }
        }
    }

    /// Relê os ativos de pose da Biblioteca.
    pub(crate) fn pose_refresh_library(&mut self) {
        match self.lib_workspace().list_pose_assets(200) {
            Ok(list) => {
                self.pose_ui.poses = list;
                self.pose_ui.poses_loaded = true;
            }
            Err(e) => {
                self.pose_ui.poses.clear();
                self.pose_ui.poses_loaded = true;
                self.pose_say(format!("{}: {e}", crate::i18n::tr("pose.lib.error")));
            }
        }
    }

    /// Aplica o ativo `idx` da lista (espelhado, mistura, só selecionados).
    pub(crate) fn pose_apply_asset(&mut self, idx: usize, flipped: bool) {
        let Some(card) = self.pose_ui.poses.get(idx).cloned() else { return };
        let only = (self.pose_ui.apply_only_selected && !self.pose_ui.selection.is_empty()).then(|| self.pose_ui.selection.clone());
        let target = card.asset.apply_to(&self.pose_now(), self.pose_ui.blend, flipped, only.as_ref());
        self.pose_commit_to(target, PoseOp::Apply);
    }

    /// Estado do gesto/seleção invalidado por undo/redo/abrir projeto.
    pub(crate) fn pose_reset_transients(&mut self) {
        self.pose_ui.drag = None;
        self.pose_ui.num_gesture = None;
        self.pose_ui.marquee = None;
        self.pose_ui.hud.clear();
    }
}
