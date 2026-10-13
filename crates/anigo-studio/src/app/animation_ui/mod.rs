//! Workspace Animação nativo: timeline com régua/timecode, dope sheet, curvas (Graph), reprodução, auto-key sobre a
//! edição do Posing, onion skin, "animar em 2s/3s", clips múltiplos, clips na Biblioteca, importar/exportar
//! glTF/VRMA/BVH, física secundária no playback e fila de render de sequências.
//!
//! O projeto é o `ProjectState` canônico: toda edição de animação é UM `Command::EditAnimation` (ou um `Batch`, como o
//! auto-key `Batch[SetPose, InsertKeys]`) pela MESMA `CoreSession` dos demais workspaces, com undo/redo exatos.
//! Scrub e reprodução NÃO são edições: o clip é avaliado no playhead e a amostra é aplicada ao projeto fora do histórico
//! (`CoreSession::apply_animation_sample`), como o Blender/Sequencer; arrastar chaves usa `preview` + `commit_preview`.
//!
//! * `timeline` — régua, dope sheet, curvas, clips (painel inferior);
//! * `panels` — barra de transporte e inspetor (direita);
//! * `viewport` — viewport do Posing + onion skin + overlay da física + HUD de quadro;
//! * `script` — cenários de captura (`--scenario animacao-*`).
//!
//! A matemática mora em `crate::anim` (testada sem egui) e em `anigo_core::{animation, retarget, bvh, clip_asset}`.

mod panels;
mod script;
mod timeline;
mod viewport;

use super::StudioApp;
use crate::anim::keys::{KeyClipboard, KeyId};
use crate::anim::playback::{Span, Transport};
use crate::anim::secondary::{SecondaryConfig, SecondarySim};
use crate::i18n::tr;
use crate::library_ui::clip_bridge::ClipCard;
use anigo_core::animation::{AnimEdit, ChannelGroup, ChannelTarget, Interp};
use anigo_core::command::Command;
use anigo_core::ids::AnimationClipId;
use anigo_core::project::AnimationClip;
use std::collections::BTreeSet;

pub(crate) use script::is_animation_scenario;
#[allow(unused_imports)]
pub use script::ANIMATION_SCENARIOS;

/// Editor do painel inferior (escolhido na tool shelf).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Editor {
    #[default]
    Dope,
    Curves,
    Clips,
}

impl Editor {
    pub fn from_tool_index(i: usize) -> Editor {
        match i {
            1 => Editor::Curves,
            2 => Editor::Clips,
            _ => Editor::Dope,
        }
    }
}

/// Aba do inspetor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SideTab {
    #[default]
    Clip,
    Bone,
}

/// Interpolação padrão das chaves novas (o que o artista escolhe na barra).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum InterpPick {
    Step,
    Linear,
    #[default]
    Ease,
    Bezier,
}

impl InterpPick {
    pub const ALL: [InterpPick; 4] = [InterpPick::Step, InterpPick::Linear, InterpPick::Ease, InterpPick::Bezier];

    pub fn interp(self) -> Interp {
        match self {
            InterpPick::Step => Interp::Step,
            InterpPick::Linear => Interp::Linear,
            InterpPick::Ease => Interp::Ease,
            InterpPick::Bezier => Interp::bezier_linear(),
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            InterpPick::Step => "anim.interp.step",
            InterpPick::Linear => "anim.interp.linear",
            InterpPick::Ease => "anim.interp.ease",
            InterpPick::Bezier => "anim.interp.bezier",
        }
    }
}

/// Fantasmas de onion skin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Onion {
    pub enabled: bool,
    pub before: u8,
    pub after: u8,
    /// Espaço entre fantasmas, em quadros (ignorado no modo "chaves").
    pub step: u8,
    /// Fantasmas nas chaves vizinhas em vez de quadros vizinhos.
    pub on_keys: bool,
}

impl Default for Onion {
    fn default() -> Self {
        Onion { enabled: false, before: 2, after: 2, step: 3, on_keys: false }
    }
}

/// Vista horizontal compartilhada pelo dope sheet e pelo Graph.
#[derive(Debug, Clone, Copy)]
pub(crate) struct View {
    /// Quadro na borda esquerda da área de chaves.
    pub x0: f32,
    /// Pixels por quadro.
    pub ppf: f32,
    pub scroll_y: f32,
    /// Faixa vertical do Graph.
    pub y_lo: f32,
    pub y_hi: f32,
    pub fit_y: bool,
    pub fitted: bool,
}

impl Default for View {
    fn default() -> Self {
        View { x0: -4.0, ppf: 11.0, scroll_y: 0.0, y_lo: -1.0, y_hi: 1.0, fit_y: true, fitted: false }
    }
}

/// Gesto sobre chaves em andamento.
#[derive(Debug, Clone)]
pub(crate) enum DragKind {
    Move,
    Duplicate,
    Scale { pivot: f32 },
    /// Alça Bézier do Graph (qual alça, chave de origem).
    Handle { which: crate::anim::graph::Handle, key: KeyId, comp: usize },
}

pub(crate) struct KeyDrag {
    pub kind: DragKind,
    /// Quadro (fracionário) sob o ponteiro quando o arrasto começou.
    pub start_frame: f32,
    pub base_clip: AnimationClip,
    pub base_sel: BTreeSet<KeyId>,
    pub last: Option<AnimEdit>,
}

/// Retângulo de seleção.
pub(crate) struct Marquee {
    pub start: egui::Pos2,
    pub cur: egui::Pos2,
    pub additive: bool,
}

/// Física secundária (overlay de depuração e dado exportável).
pub(crate) struct PhysUi {
    pub enabled: bool,
    pub cfg: SecondaryConfig,
    pub sim: Option<SecondarySim>,
    /// Relógio contínuo da simulação (s de animação), avança com o playback.
    pub time: f32,
    /// Último relógio do transporte visto (detecta saltos).
    pub last_raw: Option<f32>,
    pub status: String,
}

impl Default for PhysUi {
    fn default() -> Self {
        PhysUi { enabled: false, cfg: SecondaryConfig::default(), sim: None, time: 0.0, last_raw: None, status: String::new() }
    }
}

pub(crate) struct AnimUi {
    pub transport: Transport,
    pub autokey: bool,
    pub interp: InterpPick,
    pub onion: Onion,
    pub side_tab: SideTab,
    pub view: View,
    pub selection: BTreeSet<KeyId>,
    pub only_selected_bones: bool,
    pub collapsed: BTreeSet<String>,
    /// Canais visíveis no Graph.
    pub graph_visible: BTreeSet<ChannelTarget>,
    pub drag: Option<KeyDrag>,
    pub marquee: Option<Marquee>,
    pub scrubbing: bool,
    pub clipboard: Option<KeyClipboard>,
    pub message: String,
    /// Edição numérica em andamento (vira UM passo ao soltar o ponteiro).
    pub gesture: Option<Command>,
    // ---- amostragem do clip no playhead
    pub synced_raw: Option<f32>,
    pub synced_clip: Option<AnimationClipId>,
    pub cached: Option<anigo_core::project::AnimationState>,
    /// A pose foi editada à mão e difere do clip no quadro atual (mudança sem chave).
    pub dirty_pose: bool,
    // ---- clips
    pub rename_buf: String,
    pub rename_for: Option<AnimationClipId>,
    /// Renomear a partir da lista de clips (editor "Clips"); independente do campo do inspetor.
    pub list_rename_buf: String,
    pub list_rename_for: Option<AnimationClipId>,
    /// Nó da cena escolhido para receber chaves de transformação.
    pub scene_node: Option<anigo_core::NodeId>,
    pub lib_name: String,
    pub cards: Vec<ClipCard>,
    pub cards_loaded: bool,
    // ---- arquivo
    pub io_path: String,
    pub io_status: String,
    // ---- física
    pub phys: PhysUi,
    // ---- sequência de render
    pub seq_start: i32,
    pub seq_end: i32,
    pub seq_step: u32,
    pub seq_custom: bool,
    pub seq_template: String,
    pub seq_job: Option<u64>,
    /// Mudar o fps reescala os quadros das chaves (a duração em segundos se mantém).
    pub retime: bool,
    /// Seção do inspetor a abrir à força (cenários de captura); consumida a cada quadro.
    pub force_section: Option<&'static str>,
    // ---- layout (smoke/trace)
    pub canvas: egui::Rect,
    pub ruler_h: f32,
    pub row_h: f32,
    pub play_btn: egui::Pos2,
    pub key_btn: egui::Pos2,
    pub rows_geom: Vec<(ChannelTarget, f32)>,
    pub timeline_hovered: bool,
    // ---- cenários / smoke
    pub script: Option<script::Script>,
    pub trace: bool,
    pub last_trace: String,
}

impl Default for AnimUi {
    fn default() -> Self {
        AnimUi {
            transport: Transport::default(),
            autokey: false,
            interp: InterpPick::Ease,
            onion: Onion::default(),
            side_tab: SideTab::Clip,
            view: View::default(),
            selection: BTreeSet::new(),
            only_selected_bones: false,
            collapsed: BTreeSet::new(),
            graph_visible: BTreeSet::new(),
            drag: None,
            marquee: None,
            scrubbing: false,
            clipboard: None,
            message: String::new(),
            gesture: None,
            synced_raw: None,
            synced_clip: None,
            cached: None,
            dirty_pose: false,
            rename_buf: String::new(),
            rename_for: None,
            list_rename_buf: String::new(),
            list_rename_for: None,
            scene_node: None,
            lib_name: String::new(),
            cards: Vec::new(),
            cards_loaded: false,
            io_path: String::new(),
            io_status: String::new(),
            phys: PhysUi::default(),
            seq_start: 0,
            seq_end: 0,
            seq_step: 1,
            seq_custom: false,
            seq_template: "{project}_%04d".to_string(),
            seq_job: None,
            retime: true,
            force_section: None,
            canvas: egui::Rect::NOTHING,
            ruler_h: 22.0,
            row_h: 20.0,
            play_btn: egui::Pos2::ZERO,
            key_btn: egui::Pos2::ZERO,
            rows_geom: Vec::new(),
            timeline_hovered: false,
            script: None,
            trace: std::env::var_os("ANIGO_TRACE_ANIMATION").is_some(),
            last_trace: String::new(),
        }
    }
}

/// Descrição de comando do núcleo no idioma da UI (histórico/status).
pub(crate) fn describe_anim(desc: &str) -> String {
    let (undo, redo) = (desc.strip_prefix("Undo "), desc.strip_prefix("Redo "));
    if let Some(rest) = undo {
        return format!("{}: {}", tr("char.undone"), describe_anim(rest));
    }
    if let Some(rest) = redo {
        return format!("{}: {}", tr("char.redone"), describe_anim(rest));
    }
    let count = |s: &str| s.split_whitespace().next().and_then(|n| n.parse::<usize>().ok());
    if let Some(rest) = desc.strip_prefix("Insert ") {
        return format!("{} ({})", tr("anim.h.insert"), count(rest).unwrap_or(0));
    }
    if let Some(rest) = desc.strip_prefix("Delete ") {
        return format!("{} ({})", tr("anim.h.delete"), count(rest).unwrap_or(0));
    }
    if let Some(rest) = desc.strip_prefix("Move ") {
        return format!("{} ({})", tr("anim.h.move"), count(rest).unwrap_or(0));
    }
    if desc.starts_with("Batch of") {
        return tr("anim.h.batch").to_string();
    }
    if desc.starts_with("Interpolation") {
        return tr("anim.h.interp").to_string();
    }
    if desc.starts_with("Animation clip") || desc.starts_with("Remove animation clip") || desc.starts_with("Active animation clip") || desc.starts_with("Rename animation clip") {
        return tr("anim.h.clip").to_string();
    }
    if desc.starts_with("Animation range") {
        return tr("anim.h.range").to_string();
    }
    if desc.starts_with("Animation playback") {
        return tr("anim.h.playback").to_string();
    }
    if desc.starts_with("Frame rate") {
        return tr("anim.h.fps").to_string();
    }
    if desc.starts_with("Animation markers") {
        return tr("anim.h.markers").to_string();
    }
    super::posing_ui::describe_pose(desc)
}

impl StudioApp {
    // ============================================================ acesso ao clip ativo

    pub(crate) fn anim_fps(&self) -> f32 {
        self.core.project().animation.fps
    }

    pub(crate) fn anim_clip_id(&self) -> Option<AnimationClipId> {
        let a = &self.core.project().animation;
        a.active_clip.clone().filter(|id| a.clips.contains_key(id))
    }

    pub(crate) fn anim_clip(&self) -> Option<&AnimationClip> {
        let a = &self.core.project().animation;
        a.active_clip.as_ref().and_then(|id| a.clips.get(id))
    }

    pub(crate) fn anim_span(&self) -> Span {
        Span::of(self.anim_clip(), self.anim_fps())
    }

    /// Quadro inteiro em que o usuário está (onde as chaves são inseridas).
    pub(crate) fn anim_frame(&self) -> i32 {
        self.anim_ui.transport.current_frame(self.anim_span())
    }

    /// Quadro em que uma chave nova cai: o atual, encaixado na grade do "animar em Ns".
    pub(crate) fn anim_key_frame(&self) -> i32 {
        let f = self.anim_frame();
        match self.anim_clip() {
            Some(c) if c.hold_step > 1 => crate::anim::keys::snap_to_step(f, c.effective_range(self.anim_fps()).0, c.hold_step as i32),
            _ => f,
        }
    }

    pub(crate) fn anim_say(&mut self, msg: impl Into<String>) {
        self.anim_ui.message = msg.into();
    }

    // ============================================================ comandos

    /// Comando discreto (botão): vai direto ao histórico.
    pub(crate) fn anim_apply_cmd(&mut self, cmd: Command) -> bool {
        self.anim_ui.gesture = None;
        match self.core.apply(cmd) {
            Ok(o) => {
                self.anim_ui.message = describe_anim(&o.description);
                true
            }
            Err(e) => {
                if !(e.contains("no effect") || e.contains("already")) {
                    self.anim_ui.message = format!("{}: {e}", tr("char.error"));
                }
                false
            }
        }
    }

    pub(crate) fn anim_apply(&mut self, edit: AnimEdit) -> bool {
        self.anim_apply_cmd(Command::EditAnimation { edit })
    }

    /// Edição contínua (campo numérico, alça): preview ao vivo e UM passo de desfazer ao soltar o ponteiro.
    pub(crate) fn anim_live(&mut self, edit: AnimEdit) {
        let cmd = Command::EditAnimation { edit };
        self.core.cancel_preview();
        self.core.begin_preview();
        match self.core.preview(cmd.clone()) {
            Ok(()) => self.anim_ui.gesture = Some(cmd),
            Err(e) => {
                self.core.cancel_preview();
                if !(e.contains("no effect") || e.contains("already")) {
                    self.anim_ui.message = format!("{}: {e}", tr("char.error"));
                }
                self.anim_ui.gesture = None;
            }
        }
    }

    pub(crate) fn anim_finish_gesture(&mut self, ctx: &egui::Context) {
        if self.anim_ui.gesture.is_none() || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        if let Some(cmd) = self.anim_ui.gesture.take() {
            match self.core.commit_preview(cmd) {
                Ok(o) => self.anim_ui.message = describe_anim(&o.description),
                Err(e) => {
                    if !(e.contains("no effect") || e.contains("already")) {
                        self.anim_ui.message = format!("{}: {e}", tr("char.error"));
                    }
                }
            }
        }
    }

    // ============================================================ clips

    /// Garante um clip ativo; devolve o id (cria "Clip N" pelo comando, um passo de histórico).
    pub(crate) fn anim_ensure_clip(&mut self) -> Option<AnimationClipId> {
        if let Some(id) = self.anim_clip_id() {
            return Some(id);
        }
        let name = self.anim_new_clip_name();
        let id = crate::anim::autokey::fresh_clip_id(self.core.project(), &name);
        let clip = AnimationClip::new(id.clone(), name);
        let batch = Command::Batch {
            commands: vec![Command::EditAnimation { edit: AnimEdit::PutClip { clip } }, Command::EditAnimation { edit: AnimEdit::SetActiveClip { clip_id: Some(id.clone()) } }],
        };
        if self.anim_apply_cmd(batch) {
            Some(id)
        } else {
            None
        }
    }

    pub(crate) fn anim_new_clip_name(&self) -> String {
        format!("{} {}", tr("anim.clip.default_name"), self.core.project().animation.clips.len() + 1)
    }

    /// "+ Clip": novo clip vazio, já ativo (um passo).
    pub(crate) fn anim_new_clip(&mut self) {
        let name = self.anim_new_clip_name();
        let id = crate::anim::autokey::fresh_clip_id(self.core.project(), &name);
        let clip = AnimationClip::new(id.clone(), name);
        let batch = Command::Batch {
            commands: vec![Command::EditAnimation { edit: AnimEdit::PutClip { clip } }, Command::EditAnimation { edit: AnimEdit::SetActiveClip { clip_id: Some(id) } }],
        };
        self.anim_apply_cmd(batch);
        self.anim_ui.selection.clear();
        self.anim_ui.transport.goto(0.0, true);
    }

    /// Duplica um clip (novo id, nome "… cópia"), já ativo (um passo).
    pub(crate) fn anim_duplicate_clip(&mut self, id: &AnimationClipId) {
        let Some(src) = self.core.project().animation.clips.get(id).cloned() else { return };
        let name = format!("{} {}", src.name, tr("anim.clip.copy_suffix"));
        let new_id = crate::anim::autokey::fresh_clip_id(self.core.project(), &name);
        let mut clip = src;
        clip.clip_id = new_id.clone();
        clip.name = name;
        let batch = Command::Batch {
            commands: vec![Command::EditAnimation { edit: AnimEdit::PutClip { clip } }, Command::EditAnimation { edit: AnimEdit::SetActiveClip { clip_id: Some(new_id) } }],
        };
        self.anim_apply_cmd(batch);
    }

    pub(crate) fn anim_select_clip(&mut self, id: &AnimationClipId) {
        if self.anim_clip_id().as_ref() != Some(id) {
            self.anim_apply(AnimEdit::SetActiveClip { clip_id: Some(id.clone()) });
            self.anim_ui.selection.clear();
        }
    }

    // ============================================================ chaves

    /// Alvos que "Inserir chave" grava: ossos selecionados (+ translação do quadril/raiz), ou o corpo inteiro.
    pub(crate) fn anim_key_targets(&self) -> Vec<ChannelTarget> {
        use anigo_core::pose::{bone, BONE_NAMES};
        let sel = &self.pose_ui.selection;
        if sel.is_empty() {
            return anigo_core::animation::full_pose_targets();
        }
        let mut out = Vec::new();
        for b in sel {
            out.push(ChannelTarget::bone(BONE_NAMES[*b]));
            match *b {
                bone::HIPS => out.push(ChannelTarget::Hips),
                bone::ROOT => out.push(ChannelTarget::Root),
                bone::L_WRIST => out.push(ChannelTarget::Hand { side: anigo_core::animation::HandSide::Left }),
                bone::R_WRIST => out.push(ChannelTarget::Hand { side: anigo_core::animation::HandSide::Right }),
                _ => {}
            }
        }
        out
    }

    /// "I": insere chaves no quadro atual com o valor vivo (ossos selecionados, ou o corpo inteiro).
    pub(crate) fn anim_insert_key(&mut self) {
        self.anim_insert_targets(self.anim_key_targets());
    }

    pub(crate) fn anim_insert_targets(&mut self, targets: Vec<ChannelTarget>) {
        let frame = self.anim_key_frame();
        let interp = self.anim_ui.interp.interp();
        // sem clip: o clip nasce no MESMO passo de histórico que as primeiras chaves (Batch[PutClip, SetActive, InsertKeys])
        let existing = self.anim_clip_id();
        let (id, fresh) = match &existing {
            Some(id) => (id.clone(), None),
            None => {
                let name = self.anim_new_clip_name();
                let id = crate::anim::autokey::fresh_clip_id(self.core.project(), &name);
                (id.clone(), Some(AnimationClip::new(id, name)))
            }
        };
        let clip = match &fresh {
            Some(c) => Some(c.clone()),
            None => self.core.project().animation.clips.get(&id).cloned(),
        };
        let Some(edit) = crate::anim::keys::plan_insert(self.core.project(), &id, clip.as_ref(), frame, &targets, interp) else {
            self.anim_say(tr("anim.key.nothing"));
            return;
        };
        let cmd = match fresh {
            Some(c) => Command::Batch {
                commands: vec![
                    Command::EditAnimation { edit: AnimEdit::PutClip { clip: c } },
                    Command::EditAnimation { edit: AnimEdit::SetActiveClip { clip_id: Some(id.clone()) } },
                    Command::EditAnimation { edit },
                ],
            },
            None => Command::EditAnimation { edit },
        };
        if self.anim_apply_cmd(cmd) {
            self.anim_ui.selection = targets.iter().filter(|t| self.anim_clip().and_then(|c| c.channel(t)).is_some_and(|c| c.key_at(frame).is_some())).map(|t| (t.clone(), frame)).collect();
        }
    }

    /// Alt+I: apaga as chaves dos alvos no quadro atual.
    pub(crate) fn anim_delete_key_at_frame(&mut self) {
        let (Some(id), Some(clip)) = (self.anim_clip_id(), self.anim_clip().cloned()) else { return };
        let frame = self.anim_key_frame();
        if let Some(e) = crate::anim::keys::plan_delete_at(&id, &clip, frame, &self.anim_key_targets()) {
            self.anim_apply(e);
            self.anim_ui.selection.clear();
        }
    }

    pub(crate) fn anim_delete_selected(&mut self) {
        let (Some(id), Some(clip)) = (self.anim_clip_id(), self.anim_clip().cloned()) else { return };
        if let Some(e) = crate::anim::keys::plan_delete(&id, &clip, &self.anim_ui.selection) {
            self.anim_apply(e);
            self.anim_ui.selection.clear();
        }
    }

    pub(crate) fn anim_set_interp_selected(&mut self, interp: Interp) {
        let (Some(id), Some(clip)) = (self.anim_clip_id(), self.anim_clip().cloned()) else { return };
        if let Some(e) = crate::anim::keys::plan_interp(&id, &clip, &self.anim_ui.selection, interp) {
            self.anim_apply(e);
        }
    }

    pub(crate) fn anim_duplicate_selected(&mut self, delta: i32) {
        let (Some(id), Some(clip)) = (self.anim_clip_id(), self.anim_clip().cloned()) else { return };
        if let Some(e) = crate::anim::keys::plan_duplicate(&id, &clip, &self.anim_ui.selection, delta) {
            let sel: BTreeSet<KeyId> = self.anim_ui.selection.iter().map(|(t, f)| (t.clone(), f + delta)).collect();
            if self.anim_apply(e) {
                self.anim_ui.selection = sel;
            }
        }
    }

    pub(crate) fn anim_mirror_selected(&mut self) {
        let (Some(id), Some(clip)) = (self.anim_clip_id(), self.anim_clip().cloned()) else { return };
        if let Some(e) = crate::anim::keys::plan_mirror(&id, &clip, &self.anim_ui.selection) {
            let sel: BTreeSet<KeyId> = self.anim_ui.selection.iter().filter_map(|(t, f)| t.mirrored().map(|m| (m, *f))).collect();
            if self.anim_apply(e) {
                self.anim_ui.selection = sel;
            }
        } else {
            self.anim_say(tr("anim.key.mirror_none"));
        }
    }

    pub(crate) fn anim_copy_keys(&mut self) {
        let Some(clip) = self.anim_clip() else { return };
        let cb = KeyClipboard::copy(clip, &self.anim_ui.selection);
        let n = cb.keys.len();
        if n > 0 {
            self.anim_ui.clipboard = Some(cb);
            self.anim_say(format!("{} ({n})", tr("anim.key.copied")));
        }
    }

    pub(crate) fn anim_paste_keys(&mut self, mirrored: bool) {
        let Some(cb) = self.anim_ui.clipboard.clone() else { return };
        let Some(id) = self.anim_ensure_clip() else { return };
        let at = self.anim_key_frame();
        if let Some(e) = cb.paste(&id, at, mirrored) {
            let sel: BTreeSet<KeyId> = cb
                .keys
                .iter()
                .map(|(t, r, k)| {
                    let t2 = if mirrored { anigo_core::animation::mirror_key(t, k).map(|(a, _)| a).unwrap_or_else(|| t.clone()) } else { t.clone() };
                    (t2, at + r)
                })
                .collect();
            if self.anim_apply(e) {
                self.anim_ui.selection = sel;
            }
        }
    }

    /// Quadros inteiros de chaves selecionadas (para mostrar no inspetor).
    pub(crate) fn anim_selected_existing(&self) -> Vec<KeyId> {
        match self.anim_clip() {
            Some(c) => crate::anim::keys::existing(c, &self.anim_ui.selection),
            None => Vec::new(),
        }
    }

    // ============================================================ playback

    /// Avança o relógio e amostra o clip. Chamado a cada quadro do workspace.
    pub(crate) fn anim_tick(&mut self, ctx: &egui::Context) {
        let dt = ctx.input(|i| i.stable_dt);
        let fps = self.anim_fps();
        let span = self.anim_span();
        let was_playing = self.anim_ui.transport.playing;
        // gesto de preview aberto (arrasto): o playback para (a amostra é recusada pela sessão)
        if self.core.is_previewing() {
            self.anim_ui.transport.playing = false;
        }
        let before = self.anim_ui.transport.raw;
        let moved = self.anim_ui.transport.advance(dt, fps, span);
        if moved && self.anim_ui.phys.enabled {
            self.anim_ui.phys.time += dt.min(0.25) * self.anim_ui.transport.speed.max(0.0);
        }
        if was_playing && !self.anim_ui.transport.playing && self.anim_ui.transport.raw == before {
            // parou por conta própria (fim do clip)
        }
        if self.anim_ui.transport.playing {
            ctx.request_repaint();
        }
        self.anim_sync();
        self.anim_phys_sync();
        if self.render_ui.running() {
            self.render_poll(ctx);
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
    }

    /// Avalia o clip ativo no playhead e aplica a amostra ao projeto FORA do histórico (só quando o tempo, o clip ou
    /// os dados do clip mudaram; uma pose editada à mão sem auto-key só é substituída ao mover o playhead).
    pub(crate) fn anim_sync(&mut self) {
        if self.core.is_previewing() {
            return;
        }
        let Some(id) = self.anim_clip_id() else {
            self.anim_ui.dirty_pose = false;
            self.anim_ui.synced_raw = None;
            return;
        };
        let fps = self.anim_fps();
        let raw = self.anim_ui.transport.eval_raw();
        let data_changed = self.anim_ui.cached.as_ref() != Some(&self.core.project().animation);
        let time_changed = self.anim_ui.synced_raw != Some(raw) || self.anim_ui.synced_clip.as_ref() != Some(&id);
        let Some(clip) = self.core.project().animation.clips.get(&id) else { return };
        let sample = clip.evaluate(fps, raw);
        if time_changed || data_changed {
            self.core.apply_animation_sample(&sample);
            let secs = self.anim_ui.transport.displayed(Span::of(self.anim_clip(), fps)) / fps;
            self.core.set_playhead_seconds(secs);
            self.anim_ui.synced_raw = Some(raw);
            self.anim_ui.synced_clip = Some(id);
            self.anim_ui.cached = Some(self.core.project().animation.clone());
            self.anim_ui.dirty_pose = false;
        } else {
            let pose = self.core.pose().clone();
            self.anim_ui.dirty_pose = sample.has_character() && sample.apply_to_pose(&pose) != pose;
        }
    }

    /// Esquece a amostra anterior (após undo/redo, abrir projeto…): a próxima `anim_sync` reamostra.
    pub(crate) fn anim_invalidate(&mut self) {
        self.anim_ui.synced_raw = None;
        self.anim_ui.cached = None;
    }

    /// Física secundária acompanhando o playhead (se ligada).
    pub(crate) fn anim_phys_sync(&mut self) {
        if !self.anim_ui.phys.enabled {
            return;
        }
        let Some(clip) = self.anim_clip().cloned() else { return };
        let fps = self.anim_fps();
        let raw = self.anim_ui.transport.raw;
        if self.anim_ui.phys.sim.is_none() {
            let rig = self.pose_rig();
            match SecondarySim::new(&rig, self.anim_ui.phys.cfg) {
                Ok(s) => self.anim_ui.phys.sim = Some(s),
                Err(e) => {
                    self.anim_ui.phys.status = e;
                    self.anim_ui.phys.enabled = false;
                    return;
                }
            }
            self.anim_ui.phys.last_raw = None;
        }
        let playing = self.anim_ui.transport.playing;
        let last = self.anim_ui.phys.last_raw;
        // reprodução contínua (relógio avançando): sem reinício; qualquer outro movimento do playhead é salto
        let contiguous = playing && last.is_some();
        if !playing {
            self.anim_ui.phys.time = raw / fps;
        } else if last.is_none() {
            self.anim_ui.phys.time = raw / fps;
        }
        if last != Some(raw) || playing {
            let t = self.anim_ui.phys.time;
            if let Some(sim) = self.anim_ui.phys.sim.as_mut() {
                sim.sync(&clip, fps, t, contiguous);
            }
            self.anim_ui.phys.last_raw = Some(raw);
        }
    }

    // ============================================================ atalhos

    pub(crate) fn animation_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        // desfazer/refazer (qualquer área)
        if self.hk(ctx, "edit.redo") {
            self.undo_redo(false);
            return;
        }
        if self.hk(ctx, "edit.undo") {
            self.undo_redo(true);
            return;
        }
        let span = self.anim_span();
        if self.hk(ctx, "animacao.play") {
            self.anim_ui.transport.toggle(span);
            self.anim_ui.phys.last_raw = None;
        }
        // com Shift antes do sem Shift (o egui ignora Shift extra e a primeira consome a tecla)
        if self.hk(ctx, "animacao.step_next_10") {
            self.anim_ui.transport.step(10, span);
        } else if self.hk(ctx, "animacao.step_next") {
            self.anim_ui.transport.step(1, span);
        }
        if self.hk(ctx, "animacao.step_prev_10") {
            self.anim_ui.transport.step(-10, span);
        } else if self.hk(ctx, "animacao.step_prev") {
            self.anim_ui.transport.step(-1, span);
        }
        if self.hk(ctx, "animacao.home") {
            self.anim_ui.transport.home(span);
        }
        if self.hk(ctx, "animacao.end") {
            self.anim_ui.transport.end(span);
        }
        if let Some(clip) = self.anim_clip().cloned() {
            if self.hk(ctx, "animacao.key_next") {
                self.anim_ui.transport.jump_key(&clip, span, true);
            }
            if self.hk(ctx, "animacao.key_prev") {
                self.anim_ui.transport.jump_key(&clip, span, false);
            }
        }
        if self.hk(ctx, "animacao.key_delete_here") {
            self.anim_delete_key_at_frame();
        }
        if self.hk(ctx, "animacao.key_insert") {
            self.anim_insert_key();
        }
        if self.anim_ui.timeline_hovered {
            if self.hk(ctx, "animacao.keys_delete") {
                self.anim_delete_selected();
            }
            if self.hk(ctx, "animacao.keys_copy") {
                self.anim_copy_keys();
            }
            if self.hk(ctx, "animacao.keys_paste_mirrored") {
                self.anim_paste_keys(true);
            } else if self.hk(ctx, "animacao.keys_paste") {
                self.anim_paste_keys(false);
            }
            if self.hk(ctx, "animacao.keys_mirror") {
                self.anim_mirror_selected();
            }
            if self.hk(ctx, "animacao.marker") {
                self.anim_add_marker();
            }
            if self.hk(ctx, "animacao.keys_select_all") {
                if let Some(c) = self.anim_clip() {
                    self.anim_ui.selection = c.channels.iter().flat_map(|ch| ch.keys.iter().map(|k| (ch.target.clone(), k.frame))).collect();
                }
            }
            if self.hk(ctx, "animacao.view_frame") {
                self.anim_frame_view();
            }
        } else {
            // fora da timeline valem os atalhos de pose do Posing (copiar/colar pose, espelhar, resetar, enquadrar)
            self.posing_shortcuts(ctx);
        }
    }

    pub(crate) fn anim_add_marker(&mut self) {
        let Some(id) = self.anim_ensure_clip() else { return };
        let frame = self.anim_frame();
        let mut markers = self.anim_clip().map(|c| c.markers.clone()).unwrap_or_default();
        if markers.iter().any(|m| m.frame == frame) {
            markers.retain(|m| m.frame != frame);
        } else {
            markers.push(anigo_core::animation::Marker { frame, name: format!("{} {}", tr("anim.marker.default"), markers.len() + 1) });
        }
        self.anim_apply(AnimEdit::SetMarkers { clip_id: id, markers });
    }

    /// Enquadra a faixa do clip na largura da área de chaves.
    pub(crate) fn anim_frame_view(&mut self) {
        let (a, b) = self.anim_span_range();
        let w = self.anim_ui.canvas.width().max(200.0);
        let span = (b - a).max(4) as f32;
        self.anim_ui.view.ppf = ((w - 40.0) / (span + 4.0)).clamp(1.5, 80.0);
        self.anim_ui.view.x0 = a as f32 - 2.0;
        self.anim_ui.view.fitted = false;
    }

    pub(crate) fn anim_span_range(&self) -> (i32, i32) {
        let s = self.anim_span();
        (s.start, s.end)
    }

    // ============================================================ troca de workspace

    /// Entrada no workspace: histórico profundo, sem gesto aberto, amostra o clip no playhead salvo.
    pub(crate) fn anim_enter(&mut self) {
        if self.core.history().limit() < 400 {
            self.core.set_history_limit(400);
        }
        self.core.cancel_preview();
        self.pose_reset_transients();
        self.anim_ui.drag = None;
        self.anim_ui.gesture = None;
        self.anim_invalidate();
        // o playhead persistido no projeto vira o relógio, uma vez por sessão
        if self.anim_ui.synced_clip.is_none() && self.anim_ui.transport.raw == 0.0 {
            let secs = self.core.project().animation.playhead_seconds;
            let fps = self.anim_fps();
            self.anim_ui.transport.goto((secs * fps).round().max(0.0), true);
        }
    }

    pub(crate) fn anim_leave(&mut self) {
        self.core.cancel_preview();
        self.anim_ui.drag = None;
        self.anim_ui.gesture = None;
        self.anim_ui.transport.playing = false;
    }

    /// Estado transitório invalidado por undo/redo/abrir projeto.
    pub(crate) fn anim_reset_transients(&mut self) {
        self.anim_ui.drag = None;
        self.anim_ui.gesture = None;
        self.anim_ui.marquee = None;
        self.anim_invalidate();
        self.pose_reset_transients();
    }

    // ============================================================ interface com o Posing (auto-key)

    /// Embrulha uma edição de pose com as chaves do auto-key quando a Animação está ativa e o auto-key ligado.
    pub(crate) fn anim_wrap_pose_cmd(&mut self, cmd: Command) -> Command {
        if self.ws != crate::workspace::WorkspaceId::Animacao || !self.anim_ui.autokey {
            return cmd;
        }
        let frame = self.anim_key_frame();
        let interp = self.anim_ui.interp.interp();
        let name = self.anim_new_clip_name();
        crate::anim::autokey::wrap_pose_command(self.core.project(), cmd, frame, interp, &name)
    }

    /// Texto de status do workspace.
    pub(crate) fn anim_status_text(&self) -> String {
        if !self.anim_ui.message.is_empty() {
            return self.anim_ui.message.clone();
        }
        String::new()
    }

    /// Tipos de grupo na ordem de exibição.
    pub(crate) fn anim_group_order() -> [ChannelGroup; 4] {
        [ChannelGroup::Character, ChannelGroup::Nodes, ChannelGroup::Sun, ChannelGroup::Camera]
    }
}
