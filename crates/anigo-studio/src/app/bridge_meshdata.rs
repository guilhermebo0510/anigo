//! Dados da malha pela bridge (`MESHDATA`): **modificadores** (pilha não destrutiva), **normais** (auto-smooth, normais
//! customizadas, média/suavização/transferência, relatório) e **UV** (costuras, unwrap, projeções, empacotar, relatório).
//!
//! Tudo atua no objeto ATIVO da Modelagem pelas mesmas funções do painel (`ModelingSession::edit_mesh`, `run_uv_action`), com um
//! passo de histórico de Modelagem por operação e sem alterar nada quando a operação falha. Parâmetros de modificadores seguem o
//! JSON do `anigo-modeling` (mesclados sobre o padrão do tipo).

use super::bridge_model::obj_or_err;
use super::uv_ui::UvAction;
use super::StudioApp;
use crate::modeling::Mode;
use crate::workspace::WorkspaceId;
use anigo_bridge::protocol::{BridgeError, BridgeResult};
use anigo_modeling::modifiers::stack::{EvalMode, Modifier, ModifierEntry};
use anigo_modeling::normals_tools::{self as nt, AverageMode, NormalMapping, NormalScope};
use anigo_modeling::uv::unwrap::UnwrapMethod;
use anigo_modeling::ModelError;
use serde_json::{json, Map, Value};

fn str_of<'a>(m: &'a Map<String, Value>, k: &str) -> Result<&'a str, BridgeError> {
    m.get(k).and_then(Value::as_str).ok_or_else(|| BridgeError::invalid_params(format!("'{k}' é obrigatório e deve ser texto")))
}

fn f32_of(v: &Value, name: &str) -> Result<f32, BridgeError> {
    v.as_f64().filter(|f| f.is_finite() && f.abs() < 1.0e6).map(|f| f as f32).ok_or_else(|| BridgeError::invalid_params(format!("'{name}' deve ser um número finito")))
}

fn index_of(m: &Map<String, Value>, k: &str) -> Result<usize, BridgeError> {
    m.get(k).and_then(Value::as_u64).map(|n| n as usize).ok_or_else(|| BridgeError::invalid_params(format!("'{k}' é obrigatório (inteiro >= 0)")))
}

fn model_err(e: ModelError) -> BridgeError {
    BridgeError::failed(e.to_string())
}

/// Mescla `patch` em `base` recursivamente (objetos); o resto substitui.
fn merge(base: &mut Value, patch: &Value) {
    match (base.as_object_mut(), patch) {
        (Some(b), Value::Object(p)) => {
            for (k, v) in p {
                match b.get_mut(k) {
                    Some(slot) => merge(slot, v),
                    None => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        _ => *base = patch.clone(),
    }
}

/// Tipos de modificador (chave → padrão do catálogo do painel).
fn modifier_default(kind: &str) -> Option<Modifier> {
    let name = match kind {
        "subsurf" => "Subdivision Surface",
        "mirror" => "Mirror",
        "solidify" => "Solidify",
        "array" => "Array",
        "weld" => "Weld",
        "edge_split" => "Edge Split",
        "displace" => "Displace",
        "decimate" => "Decimate",
        "triangulate" => "Triangulate",
        _ => return None,
    };
    super::modifiers_ui::catalog().into_iter().find(|(n, _)| *n == name).map(|(_, m)| m)
}

const MODIFIER_KINDS: &str = "subsurf, mirror, solidify, array, weld, edge_split, displace, decimate, triangulate";

/// Aplica um patch JSON ao conteúdo (a variante externa `{"Tipo": {...}}` fica; só os parâmetros são mesclados).
fn patch_modifier(m: &Modifier, patch: &Value) -> Result<Modifier, BridgeError> {
    let mut v = serde_json::to_value(m).map_err(|e| BridgeError::failed(e.to_string()))?;
    let inner = v.as_object_mut().and_then(|o| o.values_mut().next()).ok_or_else(|| BridgeError::failed("modificador sem conteúdo"))?;
    super::bridge_model::reject_unknown_keys(inner, patch, "params")?;
    merge(inner, patch);
    serde_json::from_value(v).map_err(|e| BridgeError::invalid_params(format!("params inválidos: {e}")))
}

fn uv_action_of(k: &str) -> Option<UvAction> {
    Some(match k {
        "mark_seam" => UvAction::MarkSeam,
        "clear_seam" => UvAction::ClearSeam,
        "clear_all_seams" => UvAction::ClearAllSeams,
        "seams_by_angle" => UvAction::SeamsByAngle,
        "seam_path" => UvAction::SeamPath,
        "auto_seams_character" => UvAction::AutoSeamsCharacter,
        "seams_from_sharp" => UvAction::SeamsFromSharp,
        "unwrap" => UvAction::Unwrap,
        "minimize_stretch" => UvAction::MinimizeStretch,
        "pack" => UvAction::Pack,
        "normalize_density" => UvAction::NormalizeDensity,
        "project_planar" => UvAction::ProjectPlanar,
        "project_box" => UvAction::ProjectBox,
        "project_cylinder" => UvAction::ProjectCylinder,
        "project_sphere" => UvAction::ProjectSphere,
        "smart_project" => UvAction::SmartProject,
        "project_from_view" => UvAction::ProjectFromView,
        "flip_u" => UvAction::FlipU,
        "flip_v" => UvAction::FlipV,
        "rotate_90" => UvAction::Rotate90,
        _ => return None,
    })
}

const UV_ACTIONS: &str = "mark_seam, clear_seam, clear_all_seams, seams_by_angle, seam_path, auto_seams_character, seams_from_sharp, unwrap, minimize_stretch, pack, normalize_density, project_planar, project_box, project_cylinder, project_sphere, smart_project, project_from_view, flip_u, flip_v, rotate_90, report";

impl StudioApp {
    pub(crate) fn bridge_meshdata(&mut self, params: &Value) -> BridgeResult {
        let m = params.as_object().ok_or_else(|| BridgeError::invalid_params("params deve ser um objeto"))?;
        let op = str_of(m, "op")?;
        if self.ws != WorkspaceId::Modelagem {
            self.switch_workspace(WorkspaceId::Modelagem);
        }
        obj_or_err(self)?;
        match op {
            "modifiers" => self.meshdata_modifiers(m),
            "normals" => self.meshdata_normals(m),
            "uv" => self.meshdata_uv(m),
            other => Err(BridgeError::invalid_params(format!("MESHDATA op '{other}' desconhecida (modifiers, normals, uv)"))),
        }
    }

    // ------------------------------------------------------------------ modificadores

    fn modifiers_json(&self) -> Value {
        let entries = self.modeling.active_object().map(|o| o.modifiers.modifiers.clone()).unwrap_or_default();
        json!(entries.iter().enumerate().map(|(i, e)| json!({ "index": i, "name": e.name, "kind": e.modifier.kind_name(), "enabled_viewport": e.enabled_viewport, "enabled_render": e.enabled_render, "params": serde_json::to_value(&e.modifier).unwrap_or(Value::Null) })).collect::<Vec<_>>())
    }

    /// Avalia a pilha candidata no objeto ativo: recusa antes de gravar se algum modificador falharia.
    fn modifiers_validate(&self, entries: &[ModifierEntry]) -> Result<(), BridgeError> {
        let o = self.modeling.active_object().ok_or_else(|| BridgeError::failed("não há objeto ativo"))?;
        let mut stack = o.modifiers.clone();
        stack.modifiers = entries.to_vec();
        stack.evaluate(&o.mesh, EvalMode::Viewport).map(|_| ()).map_err(|e| BridgeError::failed(format!("a pilha falharia ao avaliar: {e}")))
    }

    fn meshdata_modifiers(&mut self, m: &Map<String, Value>) -> BridgeResult {
        let action = str_of(m, "action")?;
        let mut entries = self.modeling.active_object().map(|o| o.modifiers.modifiers.clone()).unwrap_or_default();
        let label: &str = match action {
            "list" => return Ok(json!({ "modifiers": self.modifiers_json() })),
            "add" => {
                let kind = str_of(m, "kind")?;
                let base = modifier_default(kind).ok_or_else(|| BridgeError::invalid_params(format!("kind '{kind}' desconhecido; disponíveis: {MODIFIER_KINDS}")))?;
                let md = match m.get("params") {
                    Some(p) => patch_modifier(&base, p)?,
                    None => base,
                };
                let mut e = ModifierEntry::new(md);
                if let Some(n) = m.get("name").and_then(Value::as_str) {
                    e = e.named(n);
                }
                entries.push(e);
                "Adicionar modificador"
            }
            "set" => {
                let i = index_of(m, "index")?;
                let len = entries.len();
                let e = entries.get_mut(i).ok_or_else(|| BridgeError::invalid_params(format!("modificador {i} não existe (0..{len})")))?;
                if let Some(p) = m.get("params") {
                    e.modifier = patch_modifier(&e.modifier, p)?;
                }
                if let Some(v) = m.get("enabled_viewport") {
                    e.enabled_viewport = v.as_bool().ok_or_else(|| BridgeError::invalid_params("enabled_viewport: true|false"))?;
                }
                if let Some(v) = m.get("enabled_render") {
                    e.enabled_render = v.as_bool().ok_or_else(|| BridgeError::invalid_params("enabled_render: true|false"))?;
                }
                if let Some(n) = m.get("name") {
                    e.name = n.as_str().filter(|s| !s.is_empty() && s.len() <= 64).ok_or_else(|| BridgeError::invalid_params("name: 1..64"))?.to_string();
                }
                "Editar modificador"
            }
            "remove" => {
                let i = index_of(m, "index")?;
                if i >= entries.len() {
                    return Err(BridgeError::invalid_params(format!("modificador {i} não existe (0..{})", entries.len())));
                }
                entries.remove(i);
                "Remover modificador"
            }
            "move" => {
                let (a, b) = (index_of(m, "from")?, index_of(m, "to")?);
                if a >= entries.len() || b >= entries.len() {
                    return Err(BridgeError::invalid_params(format!("índices fora de 0..{}", entries.len())));
                }
                let e = entries.remove(a);
                entries.insert(b, e);
                "Reordenar modificadores"
            }
            "apply" => {
                self.modeling.apply_modifier_stack().map_err(BridgeError::failed)?;
                return Ok(json!({ "applied": true, "mesh": { "verts": self.modeling.stats().verts, "faces": self.modeling.stats().faces }, "modifiers": self.modifiers_json() }));
            }
            other => return Err(BridgeError::invalid_params(format!("modifiers.action '{other}' desconhecida (list, add, set, remove, move, apply)"))),
        };
        self.modifiers_validate(&entries)?;
        self.modeling.checkpoint(label);
        if let Some(o) = self.modeling.active_object_mut() {
            o.modifiers.modifiers = entries;
        }
        self.modeling.revision += 1;
        let st = self.modeling.stats();
        let evaluated = self.modeling.active_object().and_then(|o| o.evaluated(EvalMode::Viewport)).and_then(Result::ok).map(|e| json!({ "verts": e.vert_count(), "faces": e.face_count() }));
        Ok(json!({ "modifiers": self.modifiers_json(), "base_mesh": { "verts": st.verts, "faces": st.faces }, "evaluated_mesh": evaluated }))
    }

    // ------------------------------------------------------------------ normais

    fn normal_scope(&self, m: &Map<String, Value>) -> Result<NormalScope, BridgeError> {
        if m.get("use_selection").and_then(Value::as_bool).unwrap_or(false) {
            let s = &self.modeling.selection;
            if !s.faces.is_empty() {
                return Ok(NormalScope::Faces(s.faces.iter().copied().collect()));
            }
            if !s.verts.is_empty() {
                return Ok(NormalScope::Verts(s.verts.iter().copied().collect()));
            }
            return Err(BridgeError::failed("use_selection=true mas não há seleção (anigo_native_mesh_select)"));
        }
        Ok(NormalScope::All)
    }

    fn meshdata_normals(&mut self, m: &Map<String, Value>) -> BridgeResult {
        let action = str_of(m, "action")?;
        let n = match action {
            "report" => {
                let r = nt::normal_smoothness_report(&self.modeling.active_object().ok_or_else(|| BridgeError::failed("sem objeto ativo"))?.mesh);
                return Ok(json!({
                    "edges": r.edge_count, "max_discontinuity_deg": r.max_discontinuity_deg(), "max_gradient_deg": r.max_gradient_deg(),
                    "mean_gradient_deg": r.mean_gradient.to_degrees(), "discontinuous_edges": r.discontinuous_edges, "worst_edge": r.worst_edge,
                    "auto_smooth_deg": self.modeling.active_object().map(|o| o.auto_smooth_angle.to_degrees()),
                }));
            }
            "auto_smooth" => {
                let deg = f32_of(m.get("angle_deg").ok_or_else(|| BridgeError::invalid_params("auto_smooth exige 'angle_deg' (0..180)"))?, "angle_deg")?;
                if !(0.0..=180.0).contains(&deg) {
                    return Err(BridgeError::invalid_params("angle_deg deve estar em 0..180"));
                }
                self.modeling.set_auto_smooth_angle(deg.to_radians(), true);
                return Ok(json!({ "auto_smooth_deg": self.modeling.active_object().map(|o| o.auto_smooth_angle.to_degrees()) }));
            }
            "bake_custom" => {
                let p = self.modeling.active_object().map(|o| o.normal_params()).ok_or_else(|| BridgeError::failed("sem objeto ativo"))?;
                self.modeling
                    .edit_mesh("Assar normais", |mesh, _| {
                        anigo_modeling::normals::bake_custom_normals(mesh, &p);
                        Ok(mesh.faces.iter().map(|f| f.corners.len()).sum::<usize>())
                    })
                    .map_err(model_err)?
            }
            "clear_custom" => self
                .modeling
                .edit_mesh("Limpar normais customizadas", |mesh, _| {
                    anigo_modeling::normals::clear_custom_normals(mesh);
                    Ok(mesh.faces.iter().map(|f| f.corners.len()).sum::<usize>())
                })
                .map_err(model_err)?,
            "reset" => {
                let scope = self.normal_scope(m)?;
                self.modeling.edit_mesh("Resetar normais", |mesh, _| nt::reset_normals(mesh, &scope)).map_err(model_err)?
            }
            "average" => {
                let scope = self.normal_scope(m)?;
                let mode = match m.get("mode").and_then(Value::as_str).unwrap_or("face_area") {
                    "face_area" => AverageMode::FaceArea,
                    "corner_angle" => AverageMode::CornerAngle,
                    "uniform" => AverageMode::Uniform,
                    _ => return Err(BridgeError::invalid_params("mode deve ser face_area|corner_angle|uniform")),
                };
                self.modeling.edit_mesh("Média de normais", |mesh, _| nt::average_normals(mesh, &scope, mode, &Default::default())).map_err(model_err)?
            }
            "smooth" => {
                let scope = self.normal_scope(m)?;
                let iters = super::bridge_model::opt_u64_in(m, "iterations", 1, 100, 1)? as u32;
                let factor = m.get("factor").map(|v| f32_of(v, "factor")).transpose()?.unwrap_or(0.5);
                self.modeling.edit_mesh("Suavizar normais", |mesh, _| nt::smooth_normals_region(mesh, &scope, iters, factor)).map_err(model_err)?
            }
            "transfer" => {
                let scope = self.normal_scope(m)?;
                let src_idx = index_of(m, "from_object")?;
                let dst_idx = self.modeling.doc.active.ok_or_else(|| BridgeError::failed("sem objeto ativo"))?;
                if src_idx == dst_idx {
                    return Err(BridgeError::invalid_params("a origem deve ser OUTRO objeto (o ativo é o destino)"));
                }
                let src_obj = self.modeling.doc.objects.get(src_idx).ok_or_else(|| BridgeError::invalid_params(format!("objeto {src_idx} não existe")))?;
                let to_dst = self.modeling.doc.objects[dst_idx].model_matrix().inverse() * src_obj.model_matrix();
                let mut src = src_obj.mesh.clone();
                src.transform_positions(|p| to_dst.transform_point3(p));
                let mapping = match m.get("mapping").and_then(Value::as_str).unwrap_or("closest_surface") {
                    "closest_surface" => NormalMapping::ClosestSurface,
                    "closest_vertex" => NormalMapping::ClosestVertex,
                    "closest_corner" => NormalMapping::ClosestCorner,
                    _ => return Err(BridgeError::invalid_params("mapping deve ser closest_surface|closest_vertex|closest_corner")),
                };
                let mix = m.get("mix").map(|v| f32_of(v, "mix")).transpose()?.unwrap_or(1.0);
                let max_d = m.get("max_distance").map(|v| f32_of(v, "max_distance")).transpose()?;
                self.modeling.edit_mesh("Transferir normais", |mesh, _| nt::transfer_normals_from_mesh(mesh, &src, &scope, mapping, mix, max_d, None)).map_err(model_err)?
            }
            other => return Err(BridgeError::invalid_params(format!("normals.action '{other}' desconhecida (report, auto_smooth, bake_custom, clear_custom, reset, average, smooth, transfer)"))),
        };
        let r = nt::normal_smoothness_report(&self.modeling.active_object().ok_or_else(|| BridgeError::failed("sem objeto ativo"))?.mesh);
        Ok(json!({ "action": action, "corners_changed": n, "max_gradient_deg": r.max_gradient_deg(), "discontinuous_edges": r.discontinuous_edges }))
    }

    // ------------------------------------------------------------------ UV

    fn uv_params_patch(&mut self, p: &Map<String, Value>) -> Result<(), BridgeError> {
        let mut u = self.uv.params;
        for (k, v) in p {
            let num = || f32_of(v, k);
            let flag = || v.as_bool().ok_or_else(|| BridgeError::invalid_params(format!("'{k}' deve ser true|false")));
            match k.as_str() {
                "tex_size" => u.tex_size = v.as_u64().filter(|n| (16..=16384).contains(n)).ok_or_else(|| BridgeError::invalid_params("tex_size: 16..16384"))? as u32,
                "seam_angle_deg" => u.seam_angle_deg = num()?.clamp(0.0, 180.0),
                "char_angle_deg" => u.char_angle_deg = num()?.clamp(0.0, 180.0),
                "method" => {
                    u.method = match v.as_str() {
                        Some("lscm") => UnwrapMethod::Lscm,
                        Some("lscm_arap") => UnwrapMethod::LscmArap,
                        _ => return Err(BridgeError::invalid_params("method deve ser lscm|lscm_arap")),
                    }
                }
                "margin_px" => u.margin_px = num()?.clamp(0.0, 256.0),
                "correct_aspect" => u.correct_aspect = flag()?,
                "aspect" => u.aspect = num()?.clamp(0.1, 10.0),
                "fill_holes" => u.fill_holes = flag()?,
                "stretch_iters" => u.stretch_iters = v.as_u64().filter(|n| *n <= 500).ok_or_else(|| BridgeError::invalid_params("stretch_iters: 0..500"))? as u32,
                "pack_margin_px" => u.pack_margin_px = num()?.clamp(0.0, 256.0),
                "pack_rotate" => u.pack_rotate = flag()?,
                "planar_axis" => u.planar_axis = v.as_u64().filter(|n| *n <= 2).ok_or_else(|| BridgeError::invalid_params("planar_axis: 0|1|2"))? as usize,
                "box_cross" => u.box_cross = flag()?,
                "smart_angle_deg" => u.smart_angle_deg = num()?.clamp(1.0, 180.0),
                other => return Err(BridgeError::invalid_params(format!("uv.params: chave desconhecida '{other}'"))),
            }
        }
        self.uv.params = u;
        Ok(())
    }

    fn uv_report_json(&self) -> Result<Value, BridgeError> {
        let o = self.modeling.active_object().ok_or_else(|| BridgeError::failed("sem objeto ativo"))?;
        let r = anigo_modeling::uv::metrics::uv_report(&o.mesh, self.uv.params.tex_size).map_err(model_err)?;
        Ok(json!({
            "faces": r.face_count, "islands": r.island_count, "uv_area": r.uv_area,
            "bounds": r.bounds.map(|(a, b)| json!([a.to_array(), b.to_array()])),
            "mean_stretch": r.mean_stretch, "max_stretch": r.max_stretch, "mean_area_distortion": r.mean_area_distortion,
            "degenerate_faces": r.degenerate_faces, "flipped_faces": r.flipped_faces, "overlapping_island_pairs": r.overlapping_island_pairs,
            "self_overlapping_faces": r.self_overlapping_faces, "outside_unit_square": r.outside_unit_square,
            "seams": o.mesh.edge_attrs.values().filter(|a| a.seam).count(),
        }))
    }

    fn meshdata_uv(&mut self, m: &Map<String, Value>) -> BridgeResult {
        let action = str_of(m, "action")?;
        if let Some(p) = m.get("params") {
            self.uv_params_patch(p.as_object().ok_or_else(|| BridgeError::invalid_params("'params' deve ser um objeto"))?)?;
        }
        if action == "report" {
            return self.uv_report_json();
        }
        let a = uv_action_of(action).ok_or_else(|| BridgeError::invalid_params(format!("uv.action '{action}' desconhecida; disponíveis: {UV_ACTIONS}")))?;
        if self.modeling.mode != Mode::Edit {
            self.modeling.set_mode(Mode::Edit);
        }
        if self.modeling.mode != Mode::Edit {
            return Err(BridgeError::failed("não foi possível entrar no modo Edição"));
        }
        if !self.run_uv_action(a) {
            let why = self.uv.last_error.clone().unwrap_or_else(|| self.modeling.status.clone());
            return Err(BridgeError::failed(why));
        }
        let mut out = self.uv_report_json()?;
        out["result"] = json!(self.modeling.status);
        Ok(out)
    }
}
