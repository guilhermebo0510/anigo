//! Registro central de AÇÕES com atalho (T-2.12): id estável, contextos onde valem e atalhos PADRÃO — exatamente os que os
//! handlers (`*_shortcuts`, `handle_hotkeys`, `scene_hotkeys`…) já usavam antes do registro. Mudar um padrão aqui muda o
//! comportamento padrão do app; por isso os testes travam: nenhum conflito entre padrões, todo id é traduzido e todo id é
//! consultado em algum handler (e todo id consultado existe).
//!
//! Teclas MODAIS (Enter/Esc/X/Y/Z/dígitos durante um G/R/S, Esc de cancelar gesto, WASD do voo da câmera) e a navegação
//! de grade da Biblioteca ficam FORA do registro: são parte do gesto, não comandos (lista em `LAYOUT_ATALHOS.md`).

use crate::workspace::WorkspaceId;

/// Área da janela onde a ação vale (os handlers atuais só disparam com o ponteiro no viewport, ou na timeline).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Area {
    Any,
    /// Ponteiro sobre o viewport (no Animação: fora da timeline).
    Viewport,
    Timeline,
}

/// Um contexto: workspace (None = todos) + modo (só Modelagem; None = todos) + área.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ctx {
    pub ws: Option<WorkspaceId>,
    pub mode: Option<&'static str>,
    pub area: Area,
}

impl Ctx {
    /// Duas ações com atalhos iguais só conflitam se algum contexto de uma pode estar ATIVO junto com o da outra.
    pub fn overlaps(&self, other: &Ctx) -> bool {
        let ws_ok = match (self.ws, other.ws) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        };
        let mode_ok = match (self.mode, other.mode) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        };
        let area_ok = self.area == Area::Any || other.area == Area::Any || self.area == other.area;
        ws_ok && mode_ok && area_ok
    }

    /// O contexto `self` está ativo no estado `a`?
    pub fn active_in(&self, a: &Active) -> bool {
        self.ws.is_none_or(|w| w == a.ws) && self.mode.is_none_or(|m| Some(m) == a.mode) && (self.area == Area::Any || self.area == a.area)
    }
}

/// Estado de contexto num instante (workspace, modo de Modelagem, área sob o ponteiro).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Active {
    pub ws: WorkspaceId,
    pub mode: Option<&'static str>,
    pub area: Area,
}

#[derive(Debug, Clone, Copy)]
pub struct ActionDef {
    pub id: &'static str,
    pub ctxs: &'static [Ctx],
    /// Atalhos padrão em texto canônico (`"Ctrl+Shift+Z"`).
    pub defaults: &'static [&'static str],
}

impl ActionDef {
    /// Chave de i18n do rótulo.
    pub fn label_key(&self) -> String {
        format!("act.{}", self.id)
    }
}

const fn a(id: &'static str, ctxs: &'static [Ctx], defaults: &'static [&'static str]) -> ActionDef {
    ActionDef { id, ctxs, defaults }
}

const fn ws(w: WorkspaceId) -> Ctx {
    Ctx { ws: Some(w), mode: None, area: Area::Any }
}
const fn ws_area(w: WorkspaceId, area: Area) -> Ctx {
    Ctx { ws: Some(w), mode: None, area }
}
const fn mode(m: &'static str) -> Ctx {
    Ctx { ws: Some(WorkspaceId::Modelagem), mode: Some(m), area: Area::Viewport }
}

const GLOBAL: Ctx = Ctx { ws: None, mode: None, area: Area::Any };

const G: &[Ctx] = &[GLOBAL];
const CEN: &[Ctx] = &[ws(WorkspaceId::Cenario)];
const SHD: &[Ctx] = &[ws(WorkspaceId::Shading)];
const LIGHT_VP: &[Ctx] = &[ws_area(WorkspaceId::Iluminacao, Area::Viewport)];
/// Posing: sempre; Animação: só fora da timeline.
const POSE: &[Ctx] = &[ws(WorkspaceId::Posing), ws_area(WorkspaceId::Animacao, Area::Viewport)];
const ANIM: &[Ctx] = &[ws(WorkspaceId::Animacao)];
const ANIM_TL: &[Ctx] = &[ws_area(WorkspaceId::Animacao, Area::Timeline)];
const M_ALL: &[Ctx] = &[mode("object"), mode("edit"), mode("sculpt"), mode("hair")];
const M_NOT_HAIR: &[Ctx] = &[mode("object"), mode("edit"), mode("sculpt")];
const M_OBJECT: &[Ctx] = &[mode("object")];
const M_EDIT: &[Ctx] = &[mode("edit")];
const M_SCULPT: &[Ctx] = &[mode("sculpt")];
const M_HAIR: &[Ctx] = &[mode("hair")];

/// Todas as ações configuráveis. A ORDEM é a de exibição no painel de atalhos (agrupada por contexto).
pub const ACTIONS: &[ActionDef] = &[
    // ---------------------------------------------------------------- global
    a("edit.undo", G, &["Ctrl+Z"]),
    a("edit.redo", G, &["Ctrl+Shift+Z", "Ctrl+Y"]),
    // ---------------------------------------------------------------- Modelagem (Blender-like)
    a("modeling.toggle_edit", M_ALL, &["Tab"]),
    a("modeling.grab", M_NOT_HAIR, &["G"]),
    a("modeling.rotate", M_NOT_HAIR, &["R"]),
    a("modeling.scale", M_NOT_HAIR, &["S"]),
    a("modeling.select_all", M_NOT_HAIR, &["A"]),
    a("modeling.select_none", M_NOT_HAIR, &["Alt+A"]),
    a("modeling.select_invert", M_NOT_HAIR, &["Ctrl+I"]),
    a("modeling.select_more", M_NOT_HAIR, &["Ctrl+Plus"]),
    a("modeling.select_less", M_NOT_HAIR, &["Ctrl+Minus"]),
    a("modeling.xray", M_ALL, &["Alt+Z"]),
    a("modeling.shading_toggle", M_NOT_HAIR, &["Shift+Z"]),
    a("modeling.frame", M_ALL, &["Period", "Home"]),
    a("modeling.object.delete", M_OBJECT, &["X", "Delete"]),
    a("modeling.object.duplicate", M_OBJECT, &["Shift+D"]),
    a("modeling.edit.vertex_mode", M_EDIT, &["1"]),
    a("modeling.edit.edge_mode", M_EDIT, &["2"]),
    a("modeling.edit.face_mode", M_EDIT, &["3"]),
    a("modeling.edit.select_linked", M_EDIT, &["L"]),
    a("modeling.edit.proportional", M_EDIT, &["O"]),
    a("modeling.edit.extrude", M_EDIT, &["E"]),
    a("modeling.edit.inset", M_EDIT, &["I"]),
    a("modeling.edit.bevel", M_EDIT, &["Ctrl+B"]),
    a("modeling.edit.loop_cut", M_EDIT, &["Ctrl+R"]),
    a("modeling.edit.bisect", M_EDIT, &["K"]),
    a("modeling.edit.delete", M_EDIT, &["X", "Delete"]),
    a("modeling.edit.merge", M_EDIT, &["M"]),
    a("modeling.edit.fill", M_EDIT, &["F"]),
    a("modeling.edit.triangulate", M_EDIT, &["Ctrl+T"]),
    a("modeling.edit.recalc_normals", M_EDIT, &["Ctrl+N"]),
    a("modeling.edit.reveal", M_EDIT, &["Alt+H"]),
    a("modeling.edit.hide", M_EDIT, &["H"]),
    a("sculpt.resize_radius", M_SCULPT, &["F"]),
    a("sculpt.resize_strength", M_SCULPT, &["Shift+F"]),
    a("sculpt.radius_smaller", M_SCULPT, &["OpenBracket"]),
    a("sculpt.radius_larger", M_SCULPT, &["CloseBracket"]),
    a("hair.tool_draw", M_HAIR, &["D"]),
    a("hair.tool_edit", M_HAIR, &["V"]),
    a("hair.tool_comb", M_HAIR, &["C"]),
    a("hair.duplicate", M_HAIR, &["Shift+D"]),
    a("hair.delete_hard", M_HAIR, &["Shift+X"]),
    a("hair.delete", M_HAIR, &["X", "Delete"]),
    a("hair.flip_group", M_HAIR, &["Shift+M"]),
    a("hair.symmetry", M_HAIR, &["M"]),
    a("hair.comb_smaller", M_HAIR, &["OpenBracket"]),
    a("hair.comb_larger", M_HAIR, &["CloseBracket"]),
    // ---------------------------------------------------------------- Cenário (Unreal-like)
    a("scene.tool.select", CEN, &["Q"]),
    a("scene.tool.move", CEN, &["W"]),
    a("scene.tool.rotate", CEN, &["E"]),
    a("scene.tool.scale", CEN, &["R"]),
    a("scene.tool.cycle", CEN, &["Space"]),
    a("scene.space_toggle", CEN, &["Ctrl+Backtick"]),
    a("scene.drawer_toggle", CEN, &["Ctrl+Space"]),
    a("scene.delete", CEN, &["Delete", "Backspace"]),
    a("scene.duplicate", CEN, &["Ctrl+W"]),
    a("scene.select_all", CEN, &["Ctrl+A"]),
    a("scene.select_invert", CEN, &["Ctrl+I"]),
    a("scene.select_same", CEN, &["Shift+E"]),
    a("scene.unhide_all", CEN, &["Ctrl+H"]),
    a("scene.hide", CEN, &["H"]),
    a("scene.ungroup", CEN, &["Ctrl+Shift+G", "Shift+G"]),
    a("scene.group", CEN, &["Ctrl+G"]),
    a("scene.copy", CEN, &["Ctrl+C"]),
    a("scene.cut", CEN, &["Ctrl+X"]),
    a("scene.paste", CEN, &["Ctrl+V"]),
    a("scene.deselect", CEN, &["Escape"]),
    a("scene.rename", CEN, &["F2"]),
    a("scene.snap_origin", CEN, &["Ctrl+End"]),
    a("scene.snap_floor", CEN, &["End"]),
    a("scene.snap_surface", CEN, &["Z"]),
    a("scene.snap_vertex", CEN, &["V"]),
    a("scene.grid_coarser", CEN, &["CloseBracket"]),
    a("scene.grid_finer", CEN, &["OpenBracket"]),
    a("scene.rot_coarser", CEN, &["Shift+CloseBracket"]),
    a("scene.rot_finer", CEN, &["Shift+OpenBracket"]),
    a("scene.focus", CEN, &["F"]),
    a("scene.game_view", CEN, &["G"]),
    a("scene.view_perspective", CEN, &["Alt+G"]),
    a("scene.view_front", CEN, &["Alt+H"]),
    a("scene.view_top", CEN, &["Alt+J"]),
    a("scene.view_right", CEN, &["Alt+K"]),
    a("scene.shading_wire", CEN, &["Alt+2"]),
    a("scene.shading_solid", CEN, &["Alt+3"]),
    a("scene.shading_cel", CEN, &["Alt+4"]),
    // ---------------------------------------------------------------- Iluminação / Shading
    a("light.frame", LIGHT_VP, &["F"]),
    a("shading.copy", SHD, &["Ctrl+C"]),
    a("shading.paste", SHD, &["Ctrl+V"]),
    a("shading.frame", SHD, &["F"]),
    // ---------------------------------------------------------------- Posing (e Animação fora da timeline)
    a("pose.copy", POSE, &["Ctrl+C"]),
    a("pose.paste", POSE, &["Ctrl+V"]),
    a("pose.paste_mirrored", POSE, &["Ctrl+Shift+V"]),
    a("pose.select_all", POSE, &["Ctrl+A"]),
    a("pose.reset_selection", POSE, &["Alt+R"]),
    a("pose.reset_all", POSE, &["Alt+Shift+R"]),
    a("pose.mirror", POSE, &["M"]),
    a("pose.frame", POSE, &["F"]),
    // ---------------------------------------------------------------- Animação
    a("anim.play", ANIM, &["Space"]),
    a("anim.step_next", ANIM, &["Right"]),
    a("anim.step_prev", ANIM, &["Left"]),
    a("anim.step_next_10", ANIM, &["Shift+Right"]),
    a("anim.step_prev_10", ANIM, &["Shift+Left"]),
    a("anim.home", ANIM, &["Home"]),
    a("anim.end", ANIM, &["End"]),
    a("anim.key_next", ANIM, &["Up"]),
    a("anim.key_prev", ANIM, &["Down"]),
    a("anim.key_insert", ANIM, &["I"]),
    a("anim.key_delete_here", ANIM, &["Alt+I"]),
    a("anim.keys_delete", ANIM_TL, &["Delete", "X"]),
    a("anim.keys_copy", ANIM_TL, &["Ctrl+C"]),
    a("anim.keys_paste", ANIM_TL, &["Ctrl+V"]),
    a("anim.keys_paste_mirrored", ANIM_TL, &["Ctrl+Shift+V"]),
    a("anim.keys_mirror", ANIM_TL, &["Shift+M"]),
    a("anim.keys_select_all", ANIM_TL, &["Ctrl+A"]),
    a("anim.marker", ANIM_TL, &["M"]),
    a("anim.view_frame", ANIM_TL, &["F"]),
];

pub fn def(id: &str) -> Option<&'static ActionDef> {
    ACTIONS.iter().find(|d| d.id == id)
}
