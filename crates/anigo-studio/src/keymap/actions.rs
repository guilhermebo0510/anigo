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
    a("modelagem.toggle_edit", M_ALL, &["Tab"]),
    a("modelagem.grab", M_NOT_HAIR, &["G"]),
    a("modelagem.rotate", M_NOT_HAIR, &["R"]),
    a("modelagem.scale", M_NOT_HAIR, &["S"]),
    a("modelagem.select_all", M_NOT_HAIR, &["A"]),
    a("modelagem.select_none", M_NOT_HAIR, &["Alt+A"]),
    a("modelagem.select_invert", M_NOT_HAIR, &["Ctrl+I"]),
    a("modelagem.select_more", M_NOT_HAIR, &["Ctrl+Plus"]),
    a("modelagem.select_less", M_NOT_HAIR, &["Ctrl+Minus"]),
    a("modelagem.xray", M_ALL, &["Alt+Z"]),
    a("modelagem.shading_toggle", M_NOT_HAIR, &["Shift+Z"]),
    a("modelagem.frame", M_ALL, &["Period", "Home"]),
    a("modelagem.object.delete", M_OBJECT, &["X", "Delete"]),
    a("modelagem.object.duplicate", M_OBJECT, &["Shift+D"]),
    a("modelagem.edit.vertex_mode", M_EDIT, &["1"]),
    a("modelagem.edit.edge_mode", M_EDIT, &["2"]),
    a("modelagem.edit.face_mode", M_EDIT, &["3"]),
    a("modelagem.edit.select_linked", M_EDIT, &["L"]),
    a("modelagem.edit.proportional", M_EDIT, &["O"]),
    a("modelagem.edit.extrude", M_EDIT, &["E"]),
    a("modelagem.edit.inset", M_EDIT, &["I"]),
    a("modelagem.edit.bevel", M_EDIT, &["Ctrl+B"]),
    a("modelagem.edit.loop_cut", M_EDIT, &["Ctrl+R"]),
    a("modelagem.edit.bisect", M_EDIT, &["K"]),
    a("modelagem.edit.delete", M_EDIT, &["X", "Delete"]),
    a("modelagem.edit.merge", M_EDIT, &["M"]),
    a("modelagem.edit.fill", M_EDIT, &["F"]),
    a("modelagem.edit.triangulate", M_EDIT, &["Ctrl+T"]),
    a("modelagem.edit.recalc_normals", M_EDIT, &["Ctrl+N"]),
    a("modelagem.edit.reveal", M_EDIT, &["Alt+H"]),
    a("modelagem.edit.hide", M_EDIT, &["H"]),
    a("modelagem.sculpt.resize_radius", M_SCULPT, &["F"]),
    a("modelagem.sculpt.resize_strength", M_SCULPT, &["Shift+F"]),
    a("modelagem.sculpt.radius_smaller", M_SCULPT, &["OpenBracket"]),
    a("modelagem.sculpt.radius_larger", M_SCULPT, &["CloseBracket"]),
    a("modelagem.hair.tool_draw", M_HAIR, &["D"]),
    a("modelagem.hair.tool_edit", M_HAIR, &["V"]),
    a("modelagem.hair.tool_comb", M_HAIR, &["C"]),
    a("modelagem.hair.duplicate", M_HAIR, &["Shift+D"]),
    a("modelagem.hair.delete_hard", M_HAIR, &["Shift+X"]),
    a("modelagem.hair.delete", M_HAIR, &["X", "Delete"]),
    a("modelagem.hair.flip_group", M_HAIR, &["Shift+M"]),
    a("modelagem.hair.symmetry", M_HAIR, &["M"]),
    a("modelagem.hair.comb_smaller", M_HAIR, &["OpenBracket"]),
    a("modelagem.hair.comb_larger", M_HAIR, &["CloseBracket"]),
    // ---------------------------------------------------------------- Cenário (Unreal-like)
    a("cenario.tool.select", CEN, &["Q"]),
    a("cenario.tool.move", CEN, &["W"]),
    a("cenario.tool.rotate", CEN, &["E"]),
    a("cenario.tool.scale", CEN, &["R"]),
    a("cenario.tool.cycle", CEN, &["Space"]),
    a("cenario.space_toggle", CEN, &["Ctrl+Backtick"]),
    a("cenario.drawer_toggle", CEN, &["Ctrl+Space"]),
    a("cenario.delete", CEN, &["Delete", "Backspace"]),
    a("cenario.duplicate", CEN, &["Ctrl+W"]),
    a("cenario.select_all", CEN, &["Ctrl+A"]),
    a("cenario.select_invert", CEN, &["Ctrl+I"]),
    a("cenario.select_same", CEN, &["Shift+E"]),
    a("cenario.unhide_all", CEN, &["Ctrl+H"]),
    a("cenario.hide", CEN, &["H"]),
    a("cenario.ungroup", CEN, &["Ctrl+Shift+G", "Shift+G"]),
    a("cenario.group", CEN, &["Ctrl+G"]),
    a("cenario.copy", CEN, &["Ctrl+C"]),
    a("cenario.cut", CEN, &["Ctrl+X"]),
    a("cenario.paste", CEN, &["Ctrl+V"]),
    a("cenario.deselect", CEN, &["Escape"]),
    a("cenario.rename", CEN, &["F2"]),
    a("cenario.snap_origin", CEN, &["Ctrl+End"]),
    a("cenario.snap_floor", CEN, &["End"]),
    a("cenario.snap_surface", CEN, &["Z"]),
    a("cenario.snap_vertex", CEN, &["V"]),
    a("cenario.grid_coarser", CEN, &["CloseBracket"]),
    a("cenario.grid_finer", CEN, &["OpenBracket"]),
    a("cenario.rot_coarser", CEN, &["Shift+CloseBracket"]),
    a("cenario.rot_finer", CEN, &["Shift+OpenBracket"]),
    a("cenario.focus", CEN, &["F"]),
    a("cenario.game_view", CEN, &["G"]),
    a("cenario.view_perspective", CEN, &["Alt+G"]),
    a("cenario.view_front", CEN, &["Alt+H"]),
    a("cenario.view_top", CEN, &["Alt+J"]),
    a("cenario.view_right", CEN, &["Alt+K"]),
    a("cenario.shading_wire", CEN, &["Alt+2"]),
    a("cenario.shading_solid", CEN, &["Alt+3"]),
    a("cenario.shading_cel", CEN, &["Alt+4"]),
    // ---------------------------------------------------------------- Iluminação / Shading
    a("iluminacao.frame", LIGHT_VP, &["F"]),
    a("material.copy", SHD, &["Ctrl+C"]),
    a("material.paste", SHD, &["Ctrl+V"]),
    a("material.frame", SHD, &["F"]),
    // ---------------------------------------------------------------- Posing (e Animação fora da timeline)
    a("posing.copy", POSE, &["Ctrl+C"]),
    a("posing.paste", POSE, &["Ctrl+V"]),
    a("posing.paste_mirrored", POSE, &["Ctrl+Shift+V"]),
    a("posing.select_all", POSE, &["Ctrl+A"]),
    a("posing.reset_selection", POSE, &["Alt+R"]),
    a("posing.reset_all", POSE, &["Alt+Shift+R"]),
    a("posing.mirror", POSE, &["M"]),
    a("posing.frame", POSE, &["F"]),
    // ---------------------------------------------------------------- Animação
    a("animacao.play", ANIM, &["Space"]),
    a("animacao.step_next", ANIM, &["Right"]),
    a("animacao.step_prev", ANIM, &["Left"]),
    a("animacao.step_next_10", ANIM, &["Shift+Right"]),
    a("animacao.step_prev_10", ANIM, &["Shift+Left"]),
    a("animacao.home", ANIM, &["Home"]),
    a("animacao.end", ANIM, &["End"]),
    a("animacao.key_next", ANIM, &["Up"]),
    a("animacao.key_prev", ANIM, &["Down"]),
    a("animacao.key_insert", ANIM, &["I"]),
    a("animacao.key_delete_here", ANIM, &["Alt+I"]),
    a("animacao.keys_delete", ANIM_TL, &["Delete", "X"]),
    a("animacao.keys_copy", ANIM_TL, &["Ctrl+C"]),
    a("animacao.keys_paste", ANIM_TL, &["Ctrl+V"]),
    a("animacao.keys_paste_mirrored", ANIM_TL, &["Ctrl+Shift+V"]),
    a("animacao.keys_mirror", ANIM_TL, &["Shift+M"]),
    a("animacao.keys_select_all", ANIM_TL, &["Ctrl+A"]),
    a("animacao.marker", ANIM_TL, &["M"]),
    a("animacao.view_frame", ANIM_TL, &["F"]),
];

pub fn def(id: &str) -> Option<&'static ActionDef> {
    ACTIONS.iter().find(|d| d.id == id)
}
