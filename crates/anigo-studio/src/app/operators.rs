//! Operadores do modo Edição ligados ao motor `anigo-modeling`, com o painel
//! "Ajustar última operação" (equivalente ao Redo Panel do Blender): alterar um parâmetro
//! volta ao estado anterior e refaz a operação.

use super::StudioApp;
use crate::i18n::tr;
use crate::icons;
use crate::modeling::{Constraint, Mode, SnapshotPub, TransformKind};
use crate::theme;
use anigo_modeling::ops::{bevel, bisect, extrude, inset, loops, OpOutput};
use anigo_modeling::{EditMesh, ModelError, ModelResult, Selection};
use egui::RichText;
use glam::{Vec2, Vec3};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Extrude,
    ExtrudeIndividual,
    Inset,
    InsetIndividual,
    Bevel,
    LoopCut,
    Bisect,
}

impl OpKind {
    pub fn label(self) -> &'static str {
        match self {
            OpKind::Extrude => "Extrudar região",
            OpKind::ExtrudeIndividual => "Extrudar individual",
            OpKind::Inset => "Inset região",
            OpKind::InsetIndividual => "Inset individual",
            OpKind::Bevel => "Chanfro",
            OpKind::LoopCut => "Corte em loop",
            OpKind::Bisect => "Cortar por plano",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpParams {
    pub distance: f32,
    pub thickness: f32,
    pub depth: f32,
    pub outset: bool,
    pub even: bool,
    pub width: f32,
    pub segments: u32,
    pub profile: f32,
    pub cuts: u32,
    pub slide: f32,
    /// 0 = X, 1 = Y, 2 = Z (plano do bisect, em espaço local).
    pub plane_axis: usize,
    pub plane_offset: f32,
    pub clear_inner: bool,
    pub clear_outer: bool,
    pub fill: bool,
}

impl Default for OpParams {
    fn default() -> Self {
        Self {
            distance: 0.0,
            thickness: 0.15,
            depth: 0.0,
            outset: false,
            even: true,
            width: 0.1,
            segments: 2,
            profile: 0.5,
            cuts: 1,
            slide: 0.0,
            plane_axis: 1,
            plane_offset: 0.0,
            clear_inner: false,
            clear_outer: false,
            fill: true,
        }
    }
}

pub struct LastOp {
    pub kind: OpKind,
    pub params: OpParams,
    before: SnapshotPub,
    /// Tamanho do histórico logo após a operação (se mudou, o painel some).
    history_len: usize,
}

/// Aplica `kind` com `params` na malha. Puro: nenhuma dependência de UI (testável).
pub fn apply_kind(kind: OpKind, params: &OpParams, mesh: &mut EditMesh, sel: &Selection) -> ModelResult<OpOutput> {
    let faces: Vec<u32> = sel.faces.iter().copied().collect();
    match kind {
        OpKind::Extrude => {
            if !faces.is_empty() {
                if params.distance != 0.0 {
                    extrude::extrude_faces_region_along_normal(mesh, &faces, params.distance)
                } else {
                    extrude::extrude_faces_region(mesh, &faces)
                }
            } else if !sel.edges.is_empty() {
                let edges: Vec<_> = sel.edges.iter().copied().collect();
                extrude::extrude_edges(mesh, &edges)
            } else {
                let verts: Vec<u32> = sel.verts.iter().copied().collect();
                extrude::extrude_verts(mesh, &verts)
            }
        }
        OpKind::ExtrudeIndividual => extrude::extrude_faces_individual(mesh, &faces),
        OpKind::Inset => inset::inset_faces_region(mesh, &faces, params.thickness, params.depth, params.outset, true, params.even),
        OpKind::InsetIndividual => inset::inset_faces_individual(mesh, &faces, params.thickness, params.depth),
        OpKind::Bevel => {
            let edges: Vec<_> = sel.edges.iter().copied().collect();
            bevel::bevel_edges(mesh, &edges, params.width, params.segments, params.profile, true)
        }
        OpKind::LoopCut => {
            let Some(edge) = sel.edges.iter().next().copied() else {
                return Err(ModelError::EmptySelection("selecione uma aresta do anel a cortar"));
            };
            loops::loop_cut(mesh, edge, params.cuts, params.slide).map(Into::into)
        }
        OpKind::Bisect => {
            let mut normal = Vec3::ZERO;
            normal[params.plane_axis.min(2)] = 1.0;
            bisect::bisect(mesh, normal * params.plane_offset, normal, params.clear_inner, params.clear_outer, params.fill, 1e-5)
        }
    }
}

impl StudioApp {
    /// Executa o operador, registra o undo e prepara o painel de ajuste. Extrude inicia o grab ao longo da normal.
    pub(crate) fn run_op(&mut self, kind: OpKind) {
        if self.modeling.mode != Mode::Edit {
            self.modeling.status = "Entre no Modo Edição (Tab)".into();
            return;
        }
        let params = self.op_params.clone();
        self.run_op_with(kind, params, true);
    }

    fn run_op_with(&mut self, kind: OpKind, params: OpParams, start_modal: bool) {
        let before = self.modeling.capture();
        let p = params.clone();
        let result = self.modeling.edit_op(kind.label(), |mesh, sel| apply_kind(kind, &p, mesh, sel));
        match result {
            Ok(out) => {
                self.modeling.status = format!("{}: ok", kind.label());
                self.last_op = Some(LastOp {
                    kind,
                    params,
                    before,
                    history_len: self.modeling.history_labels().len(),
                });
                if start_modal && matches!(kind, OpKind::Extrude | OpKind::ExtrudeIndividual) {
                    self.begin_extrude_grab(&out);
                }
                self.frame_pending = false;
            }
            Err(e) => {
                self.modeling.status = format!("{}: {e}", kind.label());
            }
        }
    }

    fn begin_extrude_grab(&mut self, out: &OpOutput) {
        let Some(obj) = self.modeling.active_object() else {
            return;
        };
        let rot = obj.transform.rotation;
        let mut n = Vec3::ZERO;
        for f in &out.selection.faces {
            if (*f as usize) < obj.mesh.faces.len() {
                n += obj.mesh.face_normal(*f);
            }
        }
        let mouse = Vec2::new(self.last_mouse.x, self.last_mouse.y);
        if self.modeling.begin_modal(TransformKind::Grab, mouse) {
            if let Some(m) = self.modeling.modal_mut() {
                if n.length_squared() > 1e-8 {
                    m.constraint = Constraint::Direction(rot * n.normalize());
                }
            }
        }
    }

    pub(crate) fn edit_hotkeys(&mut self, ui: &mut egui::Ui) {
        use egui::{Key, Modifiers};
        let consume = |ui: &mut egui::Ui, m: Modifiers, k: Key| ui.input_mut(|i| i.consume_key(m, k));
        if consume(ui, Modifiers::NONE, Key::E) {
            self.run_op(OpKind::Extrude);
        } else if consume(ui, Modifiers::NONE, Key::I) {
            self.run_op(OpKind::Inset);
        } else if consume(ui, Modifiers::COMMAND, Key::B) {
            self.run_op(OpKind::Bevel);
        } else if consume(ui, Modifiers::COMMAND, Key::R) {
            self.run_op(OpKind::LoopCut);
        } else if consume(ui, Modifiers::NONE, Key::K) {
            self.run_op(OpKind::Bisect);
        }
    }

    pub(crate) fn inspector_edit_operators(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(RichText::new("Operadores").strong())
            .default_open(true)
            .show(ui, |ui| {
                let rows: [(OpKind, &str, &str); 7] = [
                    (OpKind::Extrude, icons::ARROW_CLOCKWISE, "E"),
                    (OpKind::ExtrudeIndividual, icons::ARROW_CLOCKWISE, ""),
                    (OpKind::Inset, icons::SQUARES_FOUR, "I"),
                    (OpKind::InsetIndividual, icons::SQUARES_FOUR, ""),
                    (OpKind::Bevel, icons::POLYGON, "Ctrl+B"),
                    (OpKind::LoopCut, icons::LINE_SEGMENTS, "Ctrl+R"),
                    (OpKind::Bisect, icons::KNIFE, "K"),
                ];
                ui.horizontal_wrapped(|ui| {
                    for (k, icon, key) in rows {
                        let mut btn = ui.button(format!("{icon} {}", k.label()));
                        if !key.is_empty() {
                            btn = btn.on_hover_text(key);
                        }
                        if btn.clicked() {
                            self.run_op(k);
                        }
                    }
                });
            });
        self.inspector_last_op(ui);
    }

    /// Painel "Ajustar última operação".
    fn inspector_last_op(&mut self, ui: &mut egui::Ui) {
        let hist = self.modeling.history_labels().len();
        if self.last_op.as_ref().is_some_and(|l| l.history_len != hist) {
            self.last_op = None;
        }
        let Some(last) = self.last_op.as_ref() else {
            return;
        };
        let kind = last.kind;
        let mut p = last.params.clone();
        let mut changed = false;
        egui::CollapsingHeader::new(RichText::new(format!("Ajustar: {}", kind.label())).strong().color(theme::ACCENT))
            .default_open(true)
            .show(ui, |ui| {
                let dv = |ui: &mut egui::Ui, label: &str, v: &mut f32, speed: f64| {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        ui.add(egui::DragValue::new(v).speed(speed).fixed_decimals(3)).changed()
                    })
                    .inner
                };
                match kind {
                    OpKind::Extrude => changed |= dv(ui, "Distância na normal", &mut p.distance, 0.01),
                    OpKind::ExtrudeIndividual => {}
                    OpKind::Inset | OpKind::InsetIndividual => {
                        changed |= dv(ui, "Espessura", &mut p.thickness, 0.005);
                        changed |= dv(ui, "Profundidade", &mut p.depth, 0.005);
                        if kind == OpKind::Inset {
                            changed |= ui.checkbox(&mut p.outset, "Outset").changed();
                            changed |= ui.checkbox(&mut p.even, "Espessura uniforme").changed();
                        }
                    }
                    OpKind::Bevel => {
                        changed |= dv(ui, "Largura", &mut p.width, 0.005);
                        ui.horizontal(|ui| {
                            ui.label("Segmentos");
                            changed |= ui.add(egui::DragValue::new(&mut p.segments).range(1..=32)).changed();
                        });
                        changed |= ui.add(egui::Slider::new(&mut p.profile, 0.0..=1.0).text("Perfil")).changed();
                    }
                    OpKind::LoopCut => {
                        ui.horizontal(|ui| {
                            ui.label("Cortes");
                            changed |= ui.add(egui::DragValue::new(&mut p.cuts).range(1..=64)).changed();
                        });
                        changed |= ui.add(egui::Slider::new(&mut p.slide, -1.0..=1.0).text("Deslizar")).changed();
                    }
                    OpKind::Bisect => {
                        ui.horizontal(|ui| {
                            for (i, n) in ["X", "Y", "Z"].iter().enumerate() {
                                changed |= ui.selectable_value(&mut p.plane_axis, i, *n).changed();
                            }
                        });
                        changed |= dv(ui, "Posição do plano", &mut p.plane_offset, 0.01);
                        changed |= ui.checkbox(&mut p.clear_inner, "Remover lado negativo").changed();
                        changed |= ui.checkbox(&mut p.clear_outer, "Remover lado positivo").changed();
                        changed |= ui.checkbox(&mut p.fill, "Preencher corte").changed();
                    }
                }
            });
        if changed {
            if let Some(last) = self.last_op.take() {
                self.modeling.rewind_to(last.before);
                self.op_params = p.clone();
                self.run_op_with(kind, p, false);
            }
        }
    }
}

pub fn tool_to_op(tool_id: &str) -> Option<OpKind> {
    match tool_id {
        "extrude" => Some(OpKind::Extrude),
        "inset" => Some(OpKind::Inset),
        "bevel" => Some(OpKind::Bevel),
        "loop_cut" => Some(OpKind::LoopCut),
        "knife" => Some(OpKind::Bisect),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modeling::{ModelingSession, PrimitiveKind};
    use anigo_modeling::{SelectMode, Topology};

    fn session_with(kind: PrimitiveKind) -> ModelingSession {
        let mut s = ModelingSession::new();
        s.add_primitive(kind, Vec3::ZERO);
        s.toggle_edit_mode();
        s
    }

    #[test]
    fn extrude_face_through_session_is_undoable_and_keeps_manifold() {
        let mut s = session_with(PrimitiveKind::Cube);
        s.set_select_mode(SelectMode::Face);
        let sel = Selection::from_faces(&s.active_object().unwrap().mesh, [0u32]);
        s.set_selection(sel, None);
        let before = s.active_object().unwrap().mesh.clone();
        let out = s.edit_op("Extrudar", |m, sel| apply_kind(OpKind::Extrude, &OpParams::default(), m, sel)).unwrap();
        assert_eq!(out.selection.faces.len(), 1);
        let mesh = &s.active_object().unwrap().mesh;
        assert_eq!((mesh.vert_count(), mesh.face_count()), (12, 10));
        assert!(Topology::build(mesh).is_closed_manifold());
        s.undo();
        assert_eq!(s.active_object().unwrap().mesh, before);
    }

    #[test]
    fn failing_operator_leaves_mesh_and_history_untouched() {
        let mut s = session_with(PrimitiveKind::Cube);
        let before = s.active_object().unwrap().mesh.clone();
        let depth = s.history_labels().len();
        // sem aresta selecionada: loop cut recusa
        let r = s.edit_op("Corte", |m, sel| apply_kind(OpKind::LoopCut, &OpParams::default(), m, sel));
        assert!(r.is_err());
        assert_eq!(s.active_object().unwrap().mesh, before);
        assert_eq!(s.history_labels().len(), depth);
    }

    #[test]
    fn inset_bevel_loopcut_bisect_all_run_and_validate() {
        let mut s = session_with(PrimitiveKind::Cube);
        s.set_select_mode(SelectMode::Face);
        let sel = Selection::from_faces(&s.active_object().unwrap().mesh, [2u32]);
        s.set_selection(sel, None);
        s.edit_op("Inset", |m, sel| apply_kind(OpKind::Inset, &OpParams::default(), m, sel)).unwrap();
        // bevel de todas as arestas
        s.select_all();
        s.set_select_mode(SelectMode::Edge);
        s.select_all();
        s.edit_op("Chanfro", |m, sel| apply_kind(OpKind::Bevel, &OpParams::default(), m, sel)).unwrap();
        s.active_object().unwrap().mesh.validate().unwrap();
        // loop cut numa esfera quad
        let mut s2 = session_with(PrimitiveKind::QuadSphere);
        s2.set_select_mode(SelectMode::Edge);
        let m = s2.active_object().unwrap().mesh.clone();
        let faces_before = m.face_count();
        let e = *Topology::build(&m).edges.first().map(|e| &e.verts).unwrap();
        s2.set_selection(Selection::from_edges([e]), None);
        s2.edit_op("Corte", |m, sel| apply_kind(OpKind::LoopCut, &OpParams::default(), m, sel)).unwrap();
        assert!(s2.active_object().unwrap().mesh.face_count() > faces_before);
        // bisect no plano XZ (Y = 0)
        let mut s3 = session_with(PrimitiveKind::QuadSphere);
        s3.edit_op("Bisect", |m, sel| {
            apply_kind(OpKind::Bisect, &OpParams { fill: true, ..OpParams::default() }, m, sel)
        })
        .unwrap();
        s3.active_object().unwrap().mesh.validate().unwrap();
    }

    #[test]
    fn rewind_replays_with_new_parameters() {
        let mut s = session_with(PrimitiveKind::Cube);
        s.set_select_mode(SelectMode::Face);
        let sel = Selection::from_faces(&s.active_object().unwrap().mesh, [0u32]);
        s.set_selection(sel, None);
        let snap = s.capture();
        let d0 = s.history_labels().len();
        s.edit_op("Inset", |m, sel| apply_kind(OpKind::Inset, &OpParams { thickness: 0.1, ..Default::default() }, m, sel)).unwrap();
        let hash_a = s.active_object().unwrap().mesh.content_hash();
        s.rewind_to(snap);
        assert_eq!(s.history_labels().len(), d0, "rewind descarta a entrada de undo da operação");
        s.edit_op("Inset", |m, sel| apply_kind(OpKind::Inset, &OpParams { thickness: 0.3, ..Default::default() }, m, sel)).unwrap();
        assert_ne!(hash_a, s.active_object().unwrap().mesh.content_hash());
        assert_eq!(s.history_labels().len(), d0 + 1);
    }

    #[test]
    fn tool_mapping() {
        assert_eq!(tool_to_op("bevel"), Some(OpKind::Bevel));
        assert_eq!(tool_to_op("move"), None);
        let _ = tr("tool.bevel");
    }
}
