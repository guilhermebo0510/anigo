//! Operadores do modo Edição ligados ao motor `anigo-modeling`, com o painel
//! "Ajustar última operação" (equivalente ao Redo Panel do Blender): alterar um parâmetro
//! volta ao estado anterior e refaz a operação.

use super::StudioApp;
use crate::icons;
use crate::modeling::{Constraint, Mode, SnapshotPub, TransformKind};
use crate::theme;
use anigo_modeling::ops::{
    bevel, bisect, bridge, delete, dissolve, extrude, fill, flip, inset, loops, merge, misc, smooth, triangulate, OpOutput,
};
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
    DeleteVerts,
    DeleteEdges,
    DeleteFaces,
    DeleteOnlyFaces,
    MergeCenter,
    MergeByDistance,
    DissolveVerts,
    DissolveEdges,
    DissolveFaces,
    LimitedDissolve,
    FillHole,
    GridFill,
    Bridge,
    Triangulate,
    TrisToQuads,
    FlipNormals,
    RecalcOutside,
    Smooth,
    Relax,
    Subdivide,
    Poke,
    MarkSharp,
    ClearSharp,
    MarkSeam,
    ClearSeam,
    SetCrease,
    HideSelected,
    RevealAll,
}

impl OpKind {
    pub const ALL: [OpKind; 35] = [
        OpKind::Extrude,
        OpKind::ExtrudeIndividual,
        OpKind::Inset,
        OpKind::InsetIndividual,
        OpKind::Bevel,
        OpKind::LoopCut,
        OpKind::Bisect,
        OpKind::DeleteVerts,
        OpKind::DeleteEdges,
        OpKind::DeleteFaces,
        OpKind::DeleteOnlyFaces,
        OpKind::MergeCenter,
        OpKind::MergeByDistance,
        OpKind::DissolveVerts,
        OpKind::DissolveEdges,
        OpKind::DissolveFaces,
        OpKind::LimitedDissolve,
        OpKind::FillHole,
        OpKind::GridFill,
        OpKind::Bridge,
        OpKind::Triangulate,
        OpKind::TrisToQuads,
        OpKind::FlipNormals,
        OpKind::RecalcOutside,
        OpKind::Smooth,
        OpKind::Relax,
        OpKind::Subdivide,
        OpKind::Poke,
        OpKind::MarkSharp,
        OpKind::ClearSharp,
        OpKind::MarkSeam,
        OpKind::ClearSeam,
        OpKind::SetCrease,
        OpKind::HideSelected,
        OpKind::RevealAll,
    ];

    /// Nome estável (snake_case) usado pela bridge/MCP.
    pub fn key(self) -> &'static str {
        match self {
            OpKind::Extrude => "extrude",
            OpKind::ExtrudeIndividual => "extrude_individual",
            OpKind::Inset => "inset",
            OpKind::InsetIndividual => "inset_individual",
            OpKind::Bevel => "bevel",
            OpKind::LoopCut => "loop_cut",
            OpKind::Bisect => "bisect",
            OpKind::DeleteVerts => "delete_verts",
            OpKind::DeleteEdges => "delete_edges",
            OpKind::DeleteFaces => "delete_faces",
            OpKind::DeleteOnlyFaces => "delete_only_faces",
            OpKind::MergeCenter => "merge_center",
            OpKind::MergeByDistance => "merge_by_distance",
            OpKind::DissolveVerts => "dissolve_verts",
            OpKind::DissolveEdges => "dissolve_edges",
            OpKind::DissolveFaces => "dissolve_faces",
            OpKind::LimitedDissolve => "limited_dissolve",
            OpKind::FillHole => "fill_hole",
            OpKind::GridFill => "grid_fill",
            OpKind::Bridge => "bridge",
            OpKind::Triangulate => "triangulate",
            OpKind::TrisToQuads => "tris_to_quads",
            OpKind::FlipNormals => "flip_normals",
            OpKind::RecalcOutside => "recalc_outside",
            OpKind::Smooth => "smooth",
            OpKind::Relax => "relax",
            OpKind::Subdivide => "subdivide",
            OpKind::Poke => "poke",
            OpKind::MarkSharp => "mark_sharp",
            OpKind::ClearSharp => "clear_sharp",
            OpKind::MarkSeam => "mark_seam",
            OpKind::ClearSeam => "clear_seam",
            OpKind::SetCrease => "set_crease",
            OpKind::HideSelected => "hide_selected",
            OpKind::RevealAll => "reveal_all",
        }
    }

    pub fn from_key(k: &str) -> Option<OpKind> {
        Self::ALL.into_iter().find(|o| o.key() == k)
    }

    /// Operadores que exigem uma seleção/ação direta (sem painel de ajuste de parâmetros).
    pub fn has_params(self) -> bool {
        matches!(
            self,
            OpKind::Extrude
                | OpKind::Inset
                | OpKind::InsetIndividual
                | OpKind::Bevel
                | OpKind::LoopCut
                | OpKind::Bisect
                | OpKind::MergeByDistance
                | OpKind::LimitedDissolve
                | OpKind::Bridge
                | OpKind::Smooth
                | OpKind::Relax
                | OpKind::Subdivide
                | OpKind::Poke
                | OpKind::SetCrease
                | OpKind::GridFill
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            OpKind::Extrude => "Extrudar região",
            OpKind::ExtrudeIndividual => "Extrudar individual",
            OpKind::Inset => "Inset região",
            OpKind::InsetIndividual => "Inset individual",
            OpKind::Bevel => "Chanfro",
            OpKind::LoopCut => "Corte em loop",
            OpKind::Bisect => "Cortar por plano",
            OpKind::DeleteVerts => "Excluir vértices",
            OpKind::DeleteEdges => "Excluir arestas",
            OpKind::DeleteFaces => "Excluir faces",
            OpKind::DeleteOnlyFaces => "Excluir só faces",
            OpKind::MergeCenter => "Unir no centro",
            OpKind::MergeByDistance => "Unir por distância",
            OpKind::DissolveVerts => "Dissolver vértices",
            OpKind::DissolveEdges => "Dissolver arestas",
            OpKind::DissolveFaces => "Dissolver faces",
            OpKind::LimitedDissolve => "Dissolução limitada",
            OpKind::FillHole => "Preencher buraco",
            OpKind::GridFill => "Preencher em grade",
            OpKind::Bridge => "Ponte entre loops",
            OpKind::Triangulate => "Triangular",
            OpKind::TrisToQuads => "Triângulos para quads",
            OpKind::FlipNormals => "Inverter normais",
            OpKind::RecalcOutside => "Recalcular normais (fora)",
            OpKind::Smooth => "Suavizar vértices",
            OpKind::Relax => "Relaxar vértices",
            OpKind::Subdivide => "Subdividir",
            OpKind::Poke => "Poke",
            OpKind::MarkSharp => "Marcar aresta dura",
            OpKind::ClearSharp => "Limpar aresta dura",
            OpKind::MarkSeam => "Marcar costura",
            OpKind::ClearSeam => "Limpar costura",
            OpKind::SetCrease => "Vinco de subdivisão",
            OpKind::HideSelected => "Ocultar seleção",
            OpKind::RevealAll => "Revelar tudo",
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
    pub factor: f32,
    pub iterations: u32,
    pub merge_distance: f32,
    pub dissolve_angle_deg: f32,
    pub subdiv_cuts: u32,
    pub subdiv_smooth: f32,
    pub crease: f32,
    pub poke_offset: f32,
    pub lock_boundary: bool,
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
            factor: 0.5,
            iterations: 1,
            merge_distance: 0.001,
            dissolve_angle_deg: 5.0,
            subdiv_cuts: 1,
            subdiv_smooth: 0.0,
            crease: 1.0,
            poke_offset: 0.0,
            lock_boundary: true,
        }
    }
}

impl OpParams {
    /// Aplica sobreposições vindas de JSON (`{"thickness":0.2,"segments":3,…}`). Chave desconhecida ou tipo errado = erro
    /// (nada é aplicado pela metade).
    pub fn apply_json(&mut self, m: &serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
        let mut next = self.clone();
        for (k, v) in m {
            let num = || v.as_f64().filter(|f| f.is_finite()).map(|f| f as f32).ok_or_else(|| format!("'{k}' deve ser número"));
            let uint = || v.as_u64().filter(|n| *n <= 10_000).map(|n| n as u32).ok_or_else(|| format!("'{k}' deve ser inteiro 0..10000"));
            let flag = || v.as_bool().ok_or_else(|| format!("'{k}' deve ser true|false"));
            match k.as_str() {
                "distance" => next.distance = num()?,
                "thickness" => next.thickness = num()?,
                "depth" => next.depth = num()?,
                "outset" => next.outset = flag()?,
                "even" => next.even = flag()?,
                "width" => next.width = num()?,
                "segments" => next.segments = uint()?.max(1),
                "profile" => next.profile = num()?,
                "cuts" => next.cuts = uint()?.max(1),
                "slide" => next.slide = num()?,
                "plane_axis" => next.plane_axis = (uint()? as usize).min(2),
                "plane_offset" => next.plane_offset = num()?,
                "clear_inner" => next.clear_inner = flag()?,
                "clear_outer" => next.clear_outer = flag()?,
                "fill" => next.fill = flag()?,
                "factor" => next.factor = num()?,
                "iterations" => next.iterations = uint()?.max(1),
                "merge_distance" => next.merge_distance = num()?,
                "dissolve_angle_deg" => next.dissolve_angle_deg = num()?,
                "subdiv_cuts" => next.subdiv_cuts = uint()?.max(1),
                "subdiv_smooth" => next.subdiv_smooth = num()?,
                "crease" => next.crease = num()?,
                "poke_offset" => next.poke_offset = num()?,
                "lock_boundary" => next.lock_boundary = flag()?,
                other => return Err(format!("parâmetro desconhecido: '{other}'")),
            }
        }
        *self = next;
        Ok(())
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
        _ => apply_more(kind, params, mesh, sel),
    }
}

fn components(edges: &[anigo_modeling::EdgeKey]) -> Vec<Vec<anigo_modeling::EdgeKey>> {
    let mut parent: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
    fn find(p: &mut std::collections::BTreeMap<u32, u32>, x: u32) -> u32 {
        let mut r = x;
        while p.get(&r).copied().unwrap_or(r) != r {
            r = p[&r];
        }
        p.insert(x, r);
        r
    }
    for &(a, b) in edges {
        parent.entry(a).or_insert(a);
        parent.entry(b).or_insert(b);
        let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
        if ra != rb {
            parent.insert(ra, rb);
        }
    }
    let mut groups: std::collections::BTreeMap<u32, Vec<anigo_modeling::EdgeKey>> = std::collections::BTreeMap::new();
    for &e in edges {
        let r = find(&mut parent, e.0);
        groups.entry(r).or_default().push(e);
    }
    groups.into_values().collect()
}

fn apply_more(kind: OpKind, params: &OpParams, mesh: &mut EditMesh, sel: &Selection) -> ModelResult<OpOutput> {
    let faces: Vec<u32> = sel.faces.iter().copied().collect();
    let edges: Vec<anigo_modeling::EdgeKey> = sel.edges.iter().copied().collect();
    let verts: Vec<u32> = sel.verts.iter().copied().collect();
    match kind {
        OpKind::DeleteVerts => delete::delete_verts(mesh, &verts),
        OpKind::DeleteEdges => delete::delete_edges(mesh, &edges),
        OpKind::DeleteFaces => delete::delete_faces(mesh, &faces),
        OpKind::DeleteOnlyFaces => delete::delete_faces_keep_verts(mesh, &faces),
        OpKind::MergeCenter => merge::merge_verts(mesh, &verts, merge::MergeMode::Center),
        OpKind::MergeByDistance => {
            let scope = (!verts.is_empty()).then_some(verts.as_slice());
            merge::merge_by_distance(mesh, scope, params.merge_distance, merge::ByDistanceOptions::default())
        }
        OpKind::DissolveVerts => dissolve::dissolve_verts(mesh, &verts, false, false),
        OpKind::DissolveEdges => dissolve::dissolve_edges(mesh, &edges, true),
        OpKind::DissolveFaces => dissolve::dissolve_faces(mesh, &faces),
        OpKind::LimitedDissolve => dissolve::limited_dissolve(
            mesh,
            params.dissolve_angle_deg.to_radians(),
            false,
            dissolve::Delimit::default(),
        ),
        OpKind::FillHole => fill::fill_hole(mesh, &edges),
        OpKind::GridFill => fill::grid_fill(mesh, &edges, 0, 0, false),
        OpKind::Bridge => {
            let comps = components(&edges);
            if comps.len() != 2 {
                return Err(ModelError::InvalidParameter(format!(
                    "a ponte precisa de exatamente 2 loops de arestas selecionados (há {})",
                    comps.len()
                )));
            }
            let topo = anigo_modeling::Topology::build(mesh);
            let path = |c: &Vec<anigo_modeling::EdgeKey>| -> ModelResult<loops::EdgePath> {
                let start = c[0];
                if topo.is_boundary_edge(start.0, start.1) {
                    loops::boundary_loop(mesh, &topo, start)
                } else {
                    loops::edge_loop(mesh, &topo, start)
                }
            };
            let (a, b) = (path(&comps[0])?, path(&comps[1])?);
            bridge::bridge_edge_loops(mesh, &a, &b, 1, 0, bridge::BridgeInterpolation::Linear)
        }
        OpKind::Triangulate => triangulate::triangulate_faces(
            mesh,
            &faces,
            triangulate::QuadMethod::Beauty,
            triangulate::NgonMethod::Beauty,
        ),
        OpKind::TrisToQuads => triangulate::tris_to_quads(mesh, &faces, 40f32.to_radians(), true, true, true),
        OpKind::FlipNormals => flip::flip_faces(mesh, &faces),
        OpKind::RecalcOutside => flip::recalc_normals_outside(mesh, false),
        OpKind::Smooth => smooth::smooth_verts(
            mesh,
            &verts,
            params.factor,
            params.iterations,
            params.lock_boundary,
            [true; 3],
            smooth::SmoothWeights::Uniform,
        ),
        OpKind::Relax => smooth::relax_verts(
            mesh,
            &verts,
            params.factor,
            params.iterations,
            smooth::RelaxMethod::Taubin { pass_band: 0.1 },
            params.lock_boundary,
        ),
        OpKind::Subdivide => {
            if !faces.is_empty() {
                misc::subdivide_faces(mesh, &faces, params.subdiv_cuts as usize, params.subdiv_smooth)
            } else {
                misc::subdivide_edges(mesh, &edges, params.subdiv_cuts as usize, params.subdiv_smooth, 0.0, 1)
            }
        }
        OpKind::Poke => misc::poke_faces(mesh, &faces, params.poke_offset, true),
        OpKind::MarkSharp => misc::mark_sharp(mesh, &edges, true),
        OpKind::ClearSharp => misc::mark_sharp(mesh, &edges, false),
        OpKind::MarkSeam => misc::mark_seam(mesh, &edges, true),
        OpKind::ClearSeam => misc::mark_seam(mesh, &edges, false),
        OpKind::SetCrease => misc::set_crease(mesh, &edges, params.crease),
        OpKind::HideSelected => misc::hide_faces(mesh, &faces),
        OpKind::RevealAll => Ok(misc::reveal_all(mesh)),
        // os demais já foram tratados em `apply_kind`
        _ => Err(ModelError::InvalidParameter("operador sem implementação neste ramo".into())),
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
        let km = self.keymap.clone();
        let consume = |ui: &mut egui::Ui, id: &str| km.consume(ui.ctx(), id);
        if consume(ui, "modelagem.edit.extrude") {
            self.run_op(OpKind::Extrude);
        } else if consume(ui, "modelagem.edit.inset") {
            self.run_op(OpKind::Inset);
        } else if consume(ui, "modelagem.edit.bevel") {
            self.run_op(OpKind::Bevel);
        } else if consume(ui, "modelagem.edit.loop_cut") {
            self.run_op(OpKind::LoopCut);
        } else if consume(ui, "modelagem.edit.bisect") {
            self.run_op(OpKind::Bisect);
        } else if consume(ui, "modelagem.edit.delete") {
            let kind = match self.modeling.select_mode {
                anigo_modeling::SelectMode::Vertex => OpKind::DeleteVerts,
                anigo_modeling::SelectMode::Edge => OpKind::DeleteEdges,
                anigo_modeling::SelectMode::Face => OpKind::DeleteFaces,
            };
            self.run_op(kind);
        } else if consume(ui, "modelagem.edit.merge") {
            self.run_op(OpKind::MergeCenter);
        } else if consume(ui, "modelagem.edit.fill") {
            self.run_op(OpKind::FillHole);
        } else if consume(ui, "modelagem.edit.triangulate") {
            self.run_op(OpKind::Triangulate);
        } else if consume(ui, "modelagem.edit.recalc_normals") {
            self.run_op(OpKind::RecalcOutside);
        } else if consume(ui, "modelagem.edit.reveal") {
            self.run_op(OpKind::RevealAll);
        } else if consume(ui, "modelagem.edit.hide") {
            self.run_op(OpKind::HideSelected);
        }
    }

    pub(crate) fn inspector_edit_operators(&mut self, ui: &mut egui::Ui) {
        const GROUPS: &[(&str, &[(OpKind, &str, &str)])] = &[
            (
                "Geometria",
                &[
                    (OpKind::Extrude, icons::ARROW_CLOCKWISE, "E"),
                    (OpKind::ExtrudeIndividual, icons::ARROW_CLOCKWISE, ""),
                    (OpKind::Inset, icons::SQUARES_FOUR, "I"),
                    (OpKind::InsetIndividual, icons::SQUARES_FOUR, ""),
                    (OpKind::Bevel, icons::POLYGON, "Ctrl+B"),
                    (OpKind::LoopCut, icons::LINE_SEGMENTS, "Ctrl+R"),
                    (OpKind::Bisect, icons::KNIFE, "K"),
                    (OpKind::Bridge, icons::PIPE, ""),
                    (OpKind::Subdivide, icons::GRID_FOUR, ""),
                    (OpKind::Poke, icons::SPARKLE, ""),
                ],
            ),
            (
                "Remover / unir",
                &[
                    (OpKind::DeleteVerts, icons::TRASH, "X"),
                    (OpKind::DeleteEdges, icons::TRASH, ""),
                    (OpKind::DeleteFaces, icons::TRASH, ""),
                    (OpKind::DeleteOnlyFaces, icons::TRASH, ""),
                    (OpKind::MergeCenter, icons::UNITE, "M"),
                    (OpKind::MergeByDistance, icons::UNITE, ""),
                    (OpKind::DissolveVerts, icons::EXCLUDE, ""),
                    (OpKind::DissolveEdges, icons::EXCLUDE, ""),
                    (OpKind::DissolveFaces, icons::EXCLUDE, ""),
                    (OpKind::LimitedDissolve, icons::EXCLUDE, ""),
                    (OpKind::FillHole, icons::PAINT_BUCKET, "F"),
                    (OpKind::GridFill, icons::GRID_NINE, ""),
                ],
            ),
            (
                "Faces / normais",
                &[
                    (OpKind::Triangulate, icons::POLYGON, "Ctrl+T"),
                    (OpKind::TrisToQuads, icons::SQUARES_FOUR, "Alt+J"),
                    (OpKind::FlipNormals, icons::ARROWS_CLOCKWISE, ""),
                    (OpKind::RecalcOutside, icons::ARROWS_OUT, "Ctrl+N"),
                    (OpKind::HideSelected, icons::EYE_SLASH, "H"),
                    (OpKind::RevealAll, icons::EYE, "Alt+H"),
                ],
            ),
            (
                "Vértices",
                &[(OpKind::Smooth, icons::DROP, ""), (OpKind::Relax, icons::SPIRAL, "")],
            ),
            (
                "Atributos de aresta",
                &[
                    (OpKind::MarkSharp, icons::LINE_SEGMENT, ""),
                    (OpKind::ClearSharp, icons::LINE_SEGMENT, ""),
                    (OpKind::MarkSeam, icons::SCISSORS, ""),
                    (OpKind::ClearSeam, icons::SCISSORS, ""),
                    (OpKind::SetCrease, icons::SNOWFLAKE, ""),
                ],
            ),
        ];
        crate::app::props_tabs::header(ui, RichText::new("Operadores").strong())
            .default_open(true)
            .show(ui, |ui| {
                for (title, ops) in GROUPS {
                    ui.label(RichText::new(*title).color(theme::TEXT_DIM).size(11.5));
                    ui.horizontal_wrapped(|ui| {
                        for (k, icon, key) in ops.iter() {
                            let mut btn = ui.button(format!("{icon} {}", k.label()));
                            if !key.is_empty() {
                                btn = btn.on_hover_text(*key);
                            }
                            if btn.clicked() {
                                self.run_op(*k);
                            }
                        }
                    });
                }
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
        if !kind.has_params() {
            return;
        }
        let mut p = last.params.clone();
        let mut changed = false;
        crate::app::props_tabs::header(ui, RichText::new(format!("Ajustar: {}", kind.label())).strong().color(theme::ACCENT))
            .default_open(true)
            .show(ui, |ui| {
                let dv = |ui: &mut egui::Ui, label: &str, v: &mut f32, speed: f64| {
                    ui.horizontal_wrapped(|ui| {
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
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Segmentos");
                            changed |= ui.add(egui::DragValue::new(&mut p.segments).range(1..=32)).changed();
                        });
                        changed |= ui.add(crate::app::props_tabs::lab("Perfil", egui::Slider::new(&mut p.profile, 0.0..=1.0))).changed();
                    }
                    OpKind::LoopCut => {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Cortes");
                            changed |= ui.add(egui::DragValue::new(&mut p.cuts).range(1..=64)).changed();
                        });
                        changed |= ui.add(crate::app::props_tabs::lab("Deslizar", egui::Slider::new(&mut p.slide, -1.0..=1.0))).changed();
                    }
                    OpKind::MergeByDistance => changed |= dv(ui, "Distância", &mut p.merge_distance, 0.0005),
                    OpKind::LimitedDissolve => changed |= dv(ui, "Ângulo (°)", &mut p.dissolve_angle_deg, 0.5),
                    OpKind::Smooth | OpKind::Relax => {
                        changed |= dv(ui, "Fator", &mut p.factor, 0.01);
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Iterações");
                            changed |= ui.add(egui::DragValue::new(&mut p.iterations).range(1..=100)).changed();
                        });
                        changed |= ui.checkbox(&mut p.lock_boundary, "Travar fronteira").changed();
                    }
                    OpKind::Subdivide => {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Cortes");
                            changed |= ui.add(egui::DragValue::new(&mut p.subdiv_cuts).range(1..=16)).changed();
                        });
                        changed |= dv(ui, "Suavização", &mut p.subdiv_smooth, 0.01);
                    }
                    OpKind::Poke => changed |= dv(ui, "Deslocamento", &mut p.poke_offset, 0.01),
                    OpKind::SetCrease => changed |= ui.add(crate::app::props_tabs::lab("Vinco", egui::Slider::new(&mut p.crease, 0.0..=1.0))).changed(),
                    OpKind::Bridge | OpKind::GridFill => {}
                    OpKind::Bisect => {
                        ui.horizontal_wrapped(|ui| {
                            for (i, n) in ["X", "Y", "Z"].iter().enumerate() {
                                changed |= ui.selectable_value(&mut p.plane_axis, i, *n).changed();
                            }
                        });
                        changed |= dv(ui, "Posição do plano", &mut p.plane_offset, 0.01);
                        changed |= ui.checkbox(&mut p.clear_inner, "Remover lado negativo").changed();
                        changed |= ui.checkbox(&mut p.clear_outer, "Remover lado positivo").changed();
                        changed |= ui.checkbox(&mut p.fill, "Preencher corte").changed();
                    }
                    _ => {}
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

    #[test]
    fn op_keys_are_unique_snake_case_and_roundtrip() {
        let mut seen = std::collections::HashSet::new();
        for k in OpKind::ALL {
            assert!(seen.insert(k.key()), "chave repetida: {}", k.key());
            assert!(k.key().chars().all(|c| c.is_ascii_lowercase() || c == '_'), "{}", k.key());
            assert_eq!(OpKind::from_key(k.key()), Some(k));
        }
        assert_eq!(seen.len(), 35);
        assert_eq!(OpKind::from_key("nao_existe"), None);
    }

    #[test]
    fn json_overrides_are_validated_and_atomic() {
        let mut p = OpParams::default();
        let ok = serde_json::json!({ "thickness": 0.3, "segments": 4, "outset": true, "cuts": 0 });
        p.apply_json(ok.as_object().unwrap()).unwrap();
        assert_eq!((p.thickness, p.segments, p.outset), (0.3, 4, true));
        assert_eq!(p.cuts, 1, "contagens nunca chegam a zero");
        // chave desconhecida, tipo errado e valor fora do intervalo recusam TUDO (nada aplicado pela metade)
        let before = p.clone();
        for bad in [serde_json::json!({ "width": 0.9, "bogus": 1 }), serde_json::json!({ "width": "x" }), serde_json::json!({ "segments": 99999 }), serde_json::json!({ "outset": 1 })] {
            assert!(p.apply_json(bad.as_object().unwrap()).is_err(), "{bad}");
            assert_eq!(p, before, "{bad}");
        }
    }

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

    fn run(s: &mut ModelingSession, kind: OpKind) -> Result<(), String> {
        s.edit_op(kind.label(), |m, sel| apply_kind(kind, &OpParams::default(), m, sel))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    #[test]
    fn delete_fill_triangulate_flip_smooth_subdivide_and_attributes() {
        // buraco → preencher
        let mut s = session_with(PrimitiveKind::Cube);
        s.set_select_mode(SelectMode::Face);
        let sel = Selection::from_faces(&s.active_object().unwrap().mesh, [0u32]);
        s.set_selection(sel, None);
        run(&mut s, OpKind::DeleteOnlyFaces).unwrap();
        assert_eq!(s.active_object().unwrap().mesh.face_count(), 5);
        s.set_select_mode(SelectMode::Edge);
        let topo = Topology::build(&s.active_object().unwrap().mesh);
        let boundary: Vec<_> = topo.edges.iter().filter(|e| e.faces.len() == 1).map(|e| e.verts).collect();
        assert_eq!(boundary.len(), 4);
        s.set_selection(Selection::from_edges(boundary), None);
        run(&mut s, OpKind::FillHole).unwrap();
        assert!(Topology::build(&s.active_object().unwrap().mesh).is_closed_manifold());

        // triangular tudo e voltar a quads
        s.set_select_mode(SelectMode::Face);
        s.select_all();
        run(&mut s, OpKind::Triangulate).unwrap();
        assert_eq!(s.active_object().unwrap().mesh.face_count(), 12);
        s.select_all();
        run(&mut s, OpKind::TrisToQuads).unwrap();
        assert_eq!(s.active_object().unwrap().mesh.face_count(), 6);

        // inverter e recalcular normais
        s.select_all();
        run(&mut s, OpKind::FlipNormals).unwrap();
        run(&mut s, OpKind::RecalcOutside).unwrap();
        let m = &s.active_object().unwrap().mesh;
        for f in 0..m.face_count() as u32 {
            assert!(m.face_normal(f).dot(m.face_center(f) - m.centroid()) > 0.0);
        }

        // atributos de aresta
        s.set_select_mode(SelectMode::Edge);
        s.select_all();
        run(&mut s, OpKind::MarkSeam).unwrap();
        assert!(s.active_object().unwrap().mesh.edge_attrs.values().all(|e| e.seam));
        run(&mut s, OpKind::ClearSeam).unwrap();
        assert!(s.active_object().unwrap().mesh.edge_attrs.values().all(|e| !e.seam));

        // subdividir faces e suavizar
        s.set_select_mode(SelectMode::Face);
        s.select_all();
        run(&mut s, OpKind::Subdivide).unwrap();
        assert_eq!(s.active_object().unwrap().mesh.face_count(), 24);
        s.select_all();
        s.set_select_mode(SelectMode::Vertex);
        s.select_all();
        let before = s.active_object().unwrap().mesh.content_hash();
        run(&mut s, OpKind::Smooth).unwrap();
        assert_ne!(before, s.active_object().unwrap().mesh.content_hash());
        s.active_object().unwrap().mesh.validate().unwrap();
    }

    #[test]
    fn merge_dissolve_and_bridge() {
        let mut s = session_with(PrimitiveKind::Cube);
        s.set_select_mode(SelectMode::Vertex);
        s.select_all();
        run(&mut s, OpKind::MergeByDistance).unwrap();
        assert_eq!(s.active_object().unwrap().mesh.vert_count(), 8);

        // dissolver as arestas internas de um grid → 1 face
        let mut g = session_with(PrimitiveKind::Grid);
        g.set_select_mode(SelectMode::Edge);
        let topo = Topology::build(&g.active_object().unwrap().mesh);
        let interior: Vec<_> = topo.edges.iter().filter(|e| e.faces.len() == 2).map(|e| e.verts).collect();
        g.set_selection(Selection::from_edges(interior), None);
        run(&mut g, OpKind::DissolveEdges).unwrap();
        assert_eq!(g.active_object().unwrap().mesh.face_count(), 1);

        // ponte entre os dois aros de um cilindro sem tampas
        let mut c = ModelingSession::new();
        c.add_primitive(PrimitiveKind::Cylinder, Vec3::ZERO);
        c.toggle_edit_mode();
        c.set_select_mode(SelectMode::Face);
        let mesh = c.active_object().unwrap().mesh.clone();
        let caps: Vec<u32> = (0..mesh.face_count() as u32).filter(|&f| mesh.faces[f as usize].len() > 4).collect();
        assert_eq!(caps.len(), 2);
        c.set_selection(Selection::from_faces(&mesh, caps), None);
        run(&mut c, OpKind::DeleteOnlyFaces).unwrap();
        c.set_select_mode(SelectMode::Edge);
        let topo = Topology::build(&c.active_object().unwrap().mesh);
        // arestas de fronteira = os dois aros
        let rims: Vec<_> = topo.edges.iter().filter(|e| e.faces.len() == 1).map(|e| e.verts).collect();
        c.set_selection(Selection::from_edges(rims), None);
        // as arestas verticais já existem: a ponte recusa ou gera não-manifold documentado; exige ao menos não corromper
        let hash = c.active_object().unwrap().mesh.content_hash();
        let r = run(&mut c, OpKind::Bridge);
        if r.is_err() {
            assert_eq!(hash, c.active_object().unwrap().mesh.content_hash(), "erro não altera a malha");
        }
        c.active_object().unwrap().mesh.validate().unwrap();
    }

    #[test]
    fn invalid_selection_errors_are_friendly_and_atomic() {
        let mut s = session_with(PrimitiveKind::Cube);
        let before = s.active_object().unwrap().mesh.clone();
        for kind in [OpKind::DeleteFaces, OpKind::Bridge, OpKind::FillHole, OpKind::DissolveFaces, OpKind::Bevel] {
            let r = run(&mut s, kind);
            assert!(r.is_err(), "{kind:?} sem seleção deveria falhar");
            assert_eq!(s.active_object().unwrap().mesh, before, "{kind:?}");
        }
    }

    #[test]
    fn tool_mapping() {
        assert_eq!(tool_to_op("bevel"), Some(OpKind::Bevel));
        assert_eq!(tool_to_op("move"), None);
    }
}
