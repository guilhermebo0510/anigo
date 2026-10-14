//! Comandos de modelagem da bridge (`MODEL`): deixam o `anigo-mcp` modelar e esculpir como um usuário — objetos, seleção de
//! vértices/arestas/faces, os MESMOS operadores do painel (extrudar, inset, chanfro, loop cut…), transformações numéricas,
//! pinceladas de escultura (em coordenadas de tela normalizadas ou de mundo), máscara, remalhar — e ler o estado da malha.
//!
//! Tudo roda na thread da UI, pelas mesmas funções dos botões (`ModelingSession::edit_op/edit_mesh`, `SculptState::run_script`),
//! então entra no histórico de Modelagem (`undo`/`redo` abaixo; é um histórico separado do núcleo), aparece ao vivo na janela e um
//! operador que falha NÃO altera nada. O `ProjectState` do núcleo não é tocado.
//!
//! Ações (`op`): `info`, `object`, `select`, `edit_op`, `transform`, `sculpt`, `history`. Os parâmetros estão em `anigo_native_*`
//! (`crates/anigo-mcp/src/native.rs`) e em `docs/native/MCP_BRIDGE_NATIVA.md`.

use super::bridge_ui::parse_primitive;
use super::operators::{apply_kind, OpKind};
use super::StudioApp;
use crate::modeling::sculpt::{brush_tool_id, tool_id_to_brush, tool_index_for_brush, ScriptedStroke, ViewInfo, RADIUS_MAX_PX, RADIUS_MIN_PX, STRENGTH_MAX};
use crate::modeling::{Mode, TransformKind};
use crate::workspace::WorkspaceId;
use anigo_bridge::protocol::{BridgeError, BridgeResult};
use anigo_modeling::{EdgeKey, ModelError, SelectMode, Selection};
use glam::{EulerRot, Quat, Vec2, Vec3};
use serde_json::{json, Map, Value};

const MAX_LIST: usize = 2000;
const MAX_PATH_POINTS: usize = 512;
const MAX_PATHS: usize = 32;

fn obj<'a>(v: &'a Value) -> Result<&'a Map<String, Value>, BridgeError> {
    match v {
        Value::Object(m) => Ok(m),
        Value::Null => {
            static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
            Ok(EMPTY.get_or_init(Default::default))
        }
        _ => Err(BridgeError::invalid_params("params deve ser um objeto")),
    }
}

fn str_of<'a>(m: &'a Map<String, Value>, k: &str) -> Result<&'a str, BridgeError> {
    m.get(k).and_then(Value::as_str).ok_or_else(|| BridgeError::invalid_params(format!("'{k}' é obrigatório e deve ser texto")))
}

fn f32_of(v: &Value, name: &str) -> Result<f32, BridgeError> {
    v.as_f64().filter(|f| f.is_finite() && f.abs() < 1.0e6).map(|f| f as f32).ok_or_else(|| BridgeError::invalid_params(format!("'{name}' deve ser um número finito")))
}

fn vec3_of(v: &Value, name: &str) -> Result<Vec3, BridgeError> {
    let a = v.as_array().filter(|a| a.len() == 3).ok_or_else(|| BridgeError::invalid_params(format!("'{name}' deve ter 3 números")))?;
    Ok(Vec3::new(f32_of(&a[0], name)?, f32_of(&a[1], name)?, f32_of(&a[2], name)?))
}

fn vec2_of(v: &Value, name: &str) -> Result<Vec2, BridgeError> {
    let a = v.as_array().filter(|a| a.len() == 2).ok_or_else(|| BridgeError::invalid_params(format!("'{name}' deve ter 2 números")))?;
    Ok(Vec2::new(f32_of(&a[0], name)?, f32_of(&a[1], name)?))
}

fn indices(v: &Value, name: &str) -> Result<Vec<u32>, BridgeError> {
    let a = v.as_array().filter(|a| a.len() <= 100_000).ok_or_else(|| BridgeError::invalid_params(format!("'{name}' deve ser uma lista de inteiros")))?;
    a.iter().map(|x| x.as_u64().filter(|n| *n < u32::MAX as u64).map(|n| n as u32).ok_or_else(|| BridgeError::invalid_params(format!("'{name}': só inteiros >= 0")))).collect()
}

/// Inteiro opcional em `lo..=hi`; ausente = `default`; presente e fora da faixa = ERRO (nunca cai calado no padrão).
pub(super) fn opt_u64_in(m: &Map<String, Value>, k: &str, lo: u64, hi: u64, default: u64) -> Result<u64, BridgeError> {
    match m.get(k) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_u64().filter(|n| (lo..=hi).contains(n)).ok_or_else(|| BridgeError::invalid_params(format!("'{k}' deve ser um inteiro em {lo}..{hi}"))),
    }
}

/// Recusa chaves de `patch` que não existem em `base` (recursivo). O serde ignora campos desconhecidos por padrão: sem isto um
/// erro de digitação (`"tickness"`) passaria calado e nada mudaria. `base` nulo (campo opcional desligado) aceita qualquer coisa.
pub(super) fn reject_unknown_keys(base: &Value, patch: &Value, path: &str) -> Result<(), BridgeError> {
    if let (Some(b), Some(p)) = (base.as_object(), patch.as_object()) {
        for (k, v) in p {
            let here = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
            match b.get(k) {
                None => {
                    let mut known: Vec<&str> = b.keys().map(String::as_str).collect();
                    known.sort_unstable();
                    return Err(BridgeError::invalid_params(format!("campo desconhecido '{here}'; disponíveis em '{}': {}", if path.is_empty() { "." } else { path }, known.join(", "))));
                }
                Some(bv) => reject_unknown_keys(bv, v, &here)?,
            }
        }
    }
    Ok(())
}

/// Erro claro quando não há objeto ativo (as ferramentas de dados da malha agem nele).
pub(super) fn obj_or_err(app: &StudioApp) -> Result<(), BridgeError> {
    if app.modeling.active_object().is_none() {
        return Err(BridgeError::failed("não há objeto ativo (crie ou selecione um com anigo_native_model_object)"));
    }
    Ok(())
}


/// Posições novas de `(vértice, posição de mundo, peso)`: o peso 1 é a transformação cheia; menores a atenuam (como o modal da UI).
pub(super) fn weighted_transform(kind: TransformKind, pts: &[(u32, Vec3, f32)], pivot: Vec3, v: Vec3, rot: Quat) -> Vec<(u32, Vec3)> {
    let (axis, angle) = rot.to_axis_angle();
    pts.iter()
        .map(|(vid, w, k)| {
            let nw = match kind {
                TransformKind::Grab => *w + v * *k,
                TransformKind::Rotate => pivot + Quat::from_axis_angle(axis, angle * *k) * (*w - pivot),
                TransformKind::Scale => pivot + (*w - pivot) * (Vec3::ONE + (v - Vec3::ONE) * *k),
            };
            (*vid, nw)
        })
        .collect()
}

fn model_err(e: ModelError) -> BridgeError {
    BridgeError::failed(e.to_string())
}

fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Object => "object",
        Mode::Edit => "edit",
        Mode::Sculpt => "sculpt",
        Mode::Hair => "hair",
    }
}

fn select_name(m: SelectMode) -> &'static str {
    match m {
        SelectMode::Vertex => "vertex",
        SelectMode::Edge => "edge",
        SelectMode::Face => "face",
    }
}

impl StudioApp {
    pub(crate) fn bridge_model(&mut self, params: &Value) -> BridgeResult {
        let m = obj(params)?;
        let op = str_of(m, "op")?;
        if self.ws != WorkspaceId::Modelagem {
            self.switch_workspace(WorkspaceId::Modelagem);
        }
        match op {
            "info" => self.model_info(m),
            "object" => self.model_object(m),
            "select" => self.model_select(m),
            "edit_op" => self.model_edit_op(m),
            "transform" => self.model_transform(m),
            "sculpt" => self.model_sculpt(m),
            "options" => self.model_options(m),
            "history" => self.model_history(m),
            other => Err(BridgeError::invalid_params(format!("MODEL op '{other}' desconhecida (info, object, select, edit_op, transform, sculpt, options, history)"))),
        }
    }

    /// Entra no modo pedido (com objeto ativo); erro claro se não der.
    fn model_enter(&mut self, mode: Mode) -> Result<(), BridgeError> {
        if self.modeling.mode != mode {
            self.modeling.set_mode(mode);
            self.tool_idx.insert(self.ws, 0);
        }
        if self.modeling.mode != mode {
            return Err(BridgeError::failed(format!("não foi possível entrar no modo '{}': {}", mode_name(mode), self.modeling.status)));
        }
        Ok(())
    }

    fn selection_json(&self, limit: usize) -> Value {
        let s = &self.modeling.selection;
        json!({
            "verts": s.verts.len(), "edges": s.edges.len(), "faces": s.faces.len(),
            "vert_ids": s.verts.iter().take(limit).collect::<Vec<_>>(),
            "edge_ids": s.edges.iter().take(limit).map(|(a, b)| json!([a, b])).collect::<Vec<_>>(),
            "face_ids": s.faces.iter().take(limit).collect::<Vec<_>>(),
        })
    }

    fn mesh_summary(&self) -> Value {
        let st = self.modeling.stats();
        let smooth_faces = self.modeling.active_object().map(|o| o.mesh.faces.iter().filter(|f| f.smooth).count()).unwrap_or(0);
        json!({ "verts": st.verts, "faces": st.faces, "tris": st.tris, "selected_verts": st.selected_verts, "selected_faces": st.selected_faces, "smooth_faces": smooth_faces })
    }

    // ------------------------------------------------------------------ info

    fn model_info(&self, m: &Map<String, Value>) -> BridgeResult {
        let limit = m.get("limit").and_then(Value::as_u64).map(|n| (n as usize).min(MAX_LIST)).unwrap_or(100);
        let geometry = m.get("include_geometry").and_then(Value::as_bool).unwrap_or(false);
        let objects: Vec<Value> = self
            .modeling
            .doc
            .objects
            .iter()
            .enumerate()
            .map(|(i, o)| json!({ "index": i, "name": o.name, "active": self.modeling.doc.active == Some(i), "verts": o.mesh.vert_count(), "faces": o.mesh.face_count(), "visible": o.visible }))
            .collect();
        let active = self.modeling.active_object().map(|o| {
            let model = o.model_matrix();
            let (rx, ry, rz) = o.transform.rotation.to_euler(EulerRot::XYZ);
            let local = o.mesh.bounds();
            let world = local.map(|(lo, hi)| {
                let mut wlo = Vec3::splat(f32::MAX);
                let mut whi = Vec3::splat(f32::MIN);
                for i in 0..8u32 {
                    let c = Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z });
                    let w = model.transform_point3(c);
                    wlo = wlo.min(w);
                    whi = whi.max(w);
                }
                (wlo, whi)
            });
            let mut v = json!({
                "name": o.name,
                "transform": { "translation": o.transform.translation.to_array(), "rotation_deg": [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()], "scale": o.transform.scale.to_array() },
                "bounds_local": local.map(|(a, b)| json!([a.to_array(), b.to_array()])),
                "bounds_world": world.map(|(a, b)| json!([a.to_array(), b.to_array()])),
                "modifiers": o.modifiers.modifiers.len(),
            });
            if geometry {
                v["vertices"] = json!(o.mesh.verts.iter().take(limit).map(|x| x.co.to_array()).collect::<Vec<_>>());
                v["faces"] = json!(o.mesh.faces.iter().take(limit).map(|f| f.corners.iter().map(|c| c.vert).collect::<Vec<_>>()).collect::<Vec<_>>());
                v["geometry_truncated"] = json!(o.mesh.vert_count() > limit || o.mesh.face_count() > limit);
            }
            v
        });
        let slot = self.sculpt.active_slot();
        let labels = self.modeling.history_labels();
        Ok(json!({
            "mode": mode_name(self.modeling.mode),
            "select_mode": select_name(self.modeling.select_mode),
            "objects": objects,
            "active_object": active,
            "mesh": self.mesh_summary(),
            "selection": self.selection_json(limit),
            "history": { "can_undo": self.modeling.can_undo(), "can_redo": self.modeling.can_redo(), "recent": labels.iter().rev().take(10).collect::<Vec<_>>() },
            "sculpt": {
                "brush": brush_tool_id(self.sculpt.active).trim_start_matches("sculpt."),
                "radius_px": slot.radius_px, "strength": slot.brush.strength, "invert": slot.brush.direction_invert,
                "mirror": self.sculpt.symmetry.mirror, "radial": self.sculpt.symmetry.radial, "dyntopo": self.sculpt.dyntopo,
            },
            "status": self.modeling.status,
        }))
    }

    // ------------------------------------------------------------------ objetos

    fn model_object(&mut self, m: &Map<String, Value>) -> BridgeResult {
        match str_of(m, "action")? {
            "add" => {
                let kind = parse_primitive(str_of(m, "kind")?).ok_or_else(|| BridgeError::invalid_params("kind deve ser plane|grid|cube|quad_sphere|uv_sphere|cylinder|cone|torus|capsule|circle"))?;
                let at = match m.get("position") {
                    Some(v) => vec3_of(v, "position")?,
                    None => Vec3::ZERO,
                };
                self.model_enter(Mode::Object)?;
                let idx = self.modeling.add_primitive(kind, at);
                Ok(json!({ "index": idx, "mesh": self.mesh_summary() }))
            }
            "select" => {
                let idx = m.get("index").and_then(Value::as_u64).ok_or_else(|| BridgeError::invalid_params("'index' é obrigatório (inteiro)"))? as usize;
                if idx >= self.modeling.doc.objects.len() {
                    return Err(BridgeError::invalid_params(format!("índice {idx} fora do intervalo (0..{})", self.modeling.doc.objects.len())));
                }
                self.model_enter(Mode::Object)?;
                self.modeling.set_active(idx);
                Ok(json!({ "active": idx }))
            }
            "delete" => {
                self.model_enter(Mode::Object)?;
                if self.modeling.delete_active_object() {
                    Ok(json!({ "deleted": true, "objects": self.modeling.doc.objects.len() }))
                } else {
                    Err(BridgeError::failed("não há objeto ativo para excluir"))
                }
            }
            "duplicate" => {
                self.model_enter(Mode::Object)?;
                let idx = self.modeling.duplicate_active_object().ok_or_else(|| BridgeError::failed("não há objeto ativo para duplicar"))?;
                Ok(json!({ "index": idx }))
            }
            "rename" => {
                let name = str_of(m, "name")?.trim().to_string();
                if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
                    return Err(BridgeError::invalid_params("'name' deve ter 1..128 caracteres sem controle"));
                }
                self.modeling.checkpoint("Renomear objeto");
                let o = self.modeling.active_object_mut().ok_or_else(|| BridgeError::failed("não há objeto ativo"))?;
                o.name = name.clone();
                self.modeling.revision += 1;
                Ok(json!({ "name": name }))
            }
            "set_transform" => {
                let t = m.get("translation").map(|v| vec3_of(v, "translation")).transpose()?;
                let r = m.get("rotation_deg").map(|v| vec3_of(v, "rotation_deg")).transpose()?;
                let s = m.get("scale").map(|v| vec3_of(v, "scale")).transpose()?;
                if t.is_none() && r.is_none() && s.is_none() {
                    return Err(BridgeError::invalid_params("informe translation, rotation_deg e/ou scale"));
                }
                if s.is_some_and(|s| s.min_element() <= 0.0) {
                    return Err(BridgeError::invalid_params("scale deve ser > 0"));
                }
                if self.modeling.active_object().is_none() {
                    return Err(BridgeError::failed("não há objeto ativo"));
                }
                self.modeling.checkpoint("Transformar objeto");
                if let Some(o) = self.modeling.active_object_mut() {
                    if let Some(t) = t {
                        o.transform.translation = t;
                    }
                    if let Some(r) = r {
                        o.transform.rotation = Quat::from_euler(EulerRot::XYZ, r.x.to_radians(), r.y.to_radians(), r.z.to_radians());
                    }
                    if let Some(s) = s {
                        o.transform.scale = s.max(Vec3::splat(1e-3));
                    }
                }
                self.modeling.revision += 1;
                self.model_info(&Map::new())
            }
            other => Err(BridgeError::invalid_params(format!("object.action '{other}' desconhecida (add, select, delete, duplicate, rename, set_transform)"))),
        }
    }

    // ------------------------------------------------------------------ seleção

    fn model_select(&mut self, m: &Map<String, Value>) -> BridgeResult {
        self.model_enter(Mode::Edit)?;
        if let Some(mode) = m.get("mode").and_then(Value::as_str) {
            let sm = match mode {
                "vertex" => SelectMode::Vertex,
                "edge" => SelectMode::Edge,
                "face" => SelectMode::Face,
                _ => return Err(BridgeError::invalid_params("mode deve ser vertex|edge|face")),
            };
            self.modeling.set_select_mode(sm);
        }
        let extend = m.get("extend").and_then(Value::as_bool).unwrap_or(false);
        let what = m.get("what").and_then(Value::as_str).unwrap_or("none");
        let mode = self.modeling.select_mode;
        let new_sel: Option<Selection> = {
            let Some(o) = self.modeling.active_object() else { return Err(BridgeError::failed("não há objeto ativo")) };
            let mesh = &o.mesh;
            let nv = mesh.vert_count() as u32;
            let nf = mesh.face_count() as u32;
            match what {
                "verts" => {
                    let ids = indices(m.get("indices").ok_or_else(|| BridgeError::invalid_params("'indices' é obrigatório para what=verts"))?, "indices")?;
                    if let Some(bad) = ids.iter().find(|v| **v >= nv) {
                        return Err(BridgeError::invalid_params(format!("vértice {bad} fora do intervalo (0..{nv})")));
                    }
                    Some(Selection::from_verts(ids).flush(mesh, mode))
                }
                "faces" => {
                    let ids = indices(m.get("indices").ok_or_else(|| BridgeError::invalid_params("'indices' é obrigatório para what=faces"))?, "indices")?;
                    if let Some(bad) = ids.iter().find(|f| **f >= nf) {
                        return Err(BridgeError::invalid_params(format!("face {bad} fora do intervalo (0..{nf})")));
                    }
                    Some(Selection::from_faces(mesh, ids))
                }
                "edges" => {
                    let arr = m.get("edges").and_then(Value::as_array).ok_or_else(|| BridgeError::invalid_params("'edges' é obrigatório para what=edges: [[a,b],…]"))?;
                    let mut list: Vec<EdgeKey> = Vec::new();
                    for e in arr.iter().take(100_000) {
                        let p = indices(e, "edges[i]")?;
                        if p.len() != 2 || p.iter().any(|v| *v >= nv) {
                            return Err(BridgeError::invalid_params("cada aresta deve ser [a,b] com vértices existentes"));
                        }
                        list.push((p[0], p[1]));
                    }
                    Some(Selection::from_edges(list))
                }
                "box" => {
                    // caixa em ESPAÇO DE MUNDO: vértices dentro; arestas/faces só se todos os seus vértices estão dentro
                    let bx = m.get("box").and_then(Value::as_object).ok_or_else(|| BridgeError::invalid_params("'box' é obrigatório para what=box: {min:[x,y,z],max:[x,y,z]}"))?;
                    let lo = vec3_of(bx.get("min").ok_or_else(|| BridgeError::invalid_params("box.min"))?, "box.min")?;
                    let hi = vec3_of(bx.get("max").ok_or_else(|| BridgeError::invalid_params("box.max"))?, "box.max")?;
                    let (lo, hi) = (lo.min(hi), lo.max(hi));
                    let model = o.model_matrix();
                    let inside: Vec<bool> = (0..nv).map(|v| {
                        let w = model.transform_point3(mesh.co(v));
                        w.cmpge(lo).all() && w.cmple(hi).all()
                    }).collect();
                    let verts: Vec<u32> = (0..nv).filter(|v| inside[*v as usize]).collect();
                    Some(match mode {
                        SelectMode::Vertex => Selection::from_verts(verts),
                        SelectMode::Edge => {
                            // arestas únicas (a<b) de todas as faces cujos dois vértices estão dentro
                            let mut es: std::collections::BTreeSet<EdgeKey> = Default::default();
                            for f in &mesh.faces {
                                for (a, b) in f.edges() {
                                    if inside[a as usize] && inside[b as usize] {
                                        es.insert((a.min(b), a.max(b)));
                                    }
                                }
                            }
                            Selection::from_edges(es)
                        }
                        SelectMode::Face => Selection::from_faces(mesh, (0..nf).filter(|f| mesh.faces[*f as usize].corners.iter().all(|c| inside[c.vert as usize]))),
                    })
                }
                "all" | "none" | "invert" | "more" | "less" | "linked" => None,
                other => return Err(BridgeError::invalid_params(format!("what '{other}' inválido (all, none, invert, more, less, linked, verts, edges, faces, box)"))),
            }
        };
        match (what, new_sel) {
            ("all", _) => self.modeling.select_all(),
            ("none", _) => self.modeling.select_none(),
            ("invert", _) => self.modeling.select_invert(),
            ("more", _) => self.modeling.select_more(),
            ("less", _) => self.modeling.select_less(),
            ("linked", _) => self.modeling.select_linked(),
            (_, Some(mut sel)) => {
                if extend {
                    let cur = &self.modeling.selection;
                    sel.verts.extend(cur.verts.iter().copied());
                    sel.edges.extend(cur.edges.iter().copied());
                    sel.faces.extend(cur.faces.iter().copied());
                }
                self.modeling.set_selection(sel, None);
            }
            _ => {}
        }
        let limit = m.get("limit").and_then(Value::as_u64).map(|n| (n as usize).min(MAX_LIST)).unwrap_or(100);
        Ok(json!({ "select_mode": select_name(self.modeling.select_mode), "selection": self.selection_json(limit) }))
    }

    // ------------------------------------------------------------------ operadores

    fn model_edit_op(&mut self, m: &Map<String, Value>) -> BridgeResult {
        let name = str_of(m, "name")?;
        let kind = OpKind::from_key(name).ok_or_else(|| {
            let all: Vec<&str> = OpKind::ALL.iter().map(|k| k.key()).collect();
            BridgeError::invalid_params(format!("operador '{name}' desconhecido; disponíveis: {}", all.join(", ")))
        })?;
        let mut params = self.op_params.clone();
        if let Some(p) = m.get("params") {
            params.apply_json(p.as_object().ok_or_else(|| BridgeError::invalid_params("'params' deve ser um objeto"))?).map_err(BridgeError::invalid_params)?;
        }
        self.model_enter(Mode::Edit)?;
        let before = self.mesh_summary();
        let p = params.clone();
        let out = self.modeling.edit_op(kind.label(), |mesh, sel| apply_kind(kind, &p, mesh, sel)).map_err(model_err)?;
        self.modeling.status = format!("{}: ok", kind.label());
        let _ = out;
        Ok(json!({ "op": kind.key(), "label": kind.label(), "before": before, "after": self.mesh_summary(), "selection": self.selection_json(50) }))
    }

    // ------------------------------------------------------------------ transformar

    fn model_transform(&mut self, m: &Map<String, Value>) -> BridgeResult {
        let kind = match str_of(m, "kind")? {
            "move" => TransformKind::Grab,
            "rotate" => TransformKind::Rotate,
            "scale" => TransformKind::Scale,
            _ => return Err(BridgeError::invalid_params("kind deve ser move|rotate|scale")),
        };
        let value = m.get("value").ok_or_else(|| BridgeError::invalid_params("'value' é obrigatório: move=[dx,dy,dz] (mundo), rotate=[rx,ry,rz] em graus (XYZ), scale=[sx,sy,sz] ou número"))?;
        let v = match (kind, value) {
            (TransformKind::Scale, Value::Number(_)) => Vec3::splat(f32_of(value, "value")?),
            _ => vec3_of(value, "value")?,
        };
        if kind == TransformKind::Scale && v.min_element() <= 0.0 {
            return Err(BridgeError::invalid_params("scale deve ser > 0"));
        }
        let pivot_origin = m.get("pivot").and_then(Value::as_str) == Some("origin");
        let label = match kind {
            TransformKind::Grab => "Mover",
            TransformKind::Rotate => "Rotacionar",
            TransformKind::Scale => "Escalar",
        };
        let rot = Quat::from_euler(EulerRot::XYZ, v.x.to_radians(), v.y.to_radians(), v.z.to_radians());
        match self.modeling.mode {
            Mode::Edit => {
                let Some(o) = self.modeling.active_object() else { return Err(BridgeError::failed("não há objeto ativo")) };
                let model = o.model_matrix();
                let inv = model.inverse();
                let avg_scale = (o.transform.scale.x + o.transform.scale.y + o.transform.scale.z) / 3.0;
                // edição proporcional: a do painel (tecla O), ou o que `proportional` pedir só nesta chamada
                let prop = self.model_proportional_for(m.get("proportional"))?;
                let mut affected = 0usize;
                self.modeling
                    .edit_mesh(label, |mesh, sel| {
                        if sel.verts.is_empty() {
                            return Err(ModelError::EmptySelection("selecione vértices antes de transformar (mesh_select)"));
                        }
                        let selected: Vec<u32> = sel.verts.iter().copied().collect();
                        let weights = prop.enabled.then(|| anigo_modeling::ops::transform::proportional_weights(mesh, &selected, prop.radius / avg_scale.max(1e-4), prop.falloff, prop.connected_only));
                        let pivot_pts: Vec<Vec3> = selected.iter().map(|vid| model.transform_point3(mesh.co(*vid))).collect();
                        let pivot = if pivot_origin { Vec3::ZERO } else { pivot_pts.iter().copied().sum::<Vec3>() / pivot_pts.len() as f32 };
                        let moved: Vec<(u32, Vec3, f32)> = match &weights {
                            Some(w) => (0..mesh.vert_count() as u32).filter(|v| w.get(*v as usize).copied().unwrap_or(0.0) > 1e-4).map(|v| (v, model.transform_point3(mesh.co(v)), w[v as usize])).collect(),
                            None => selected.iter().map(|v| (*v, model.transform_point3(mesh.co(*v)), 1.0)).collect(),
                        };
                        affected = moved.len();
                        for (vid, nw) in weighted_transform(kind, &moved, pivot, v, rot) {
                            mesh.verts[vid as usize].co = inv.transform_point3(nw);
                        }
                        Ok(())
                    })
                    .map_err(model_err)?;
                let mut out = self.model_info(&Map::new())?;
                out["proportional"] = json!({ "enabled": prop.enabled, "radius": prop.radius, "falloff": format!("{:?}", prop.falloff).to_lowercase(), "connected_only": prop.connected_only, "affected_vertices": affected });
                return Ok(out);
            }
            Mode::Object => {
                if self.modeling.active_object().is_none() {
                    return Err(BridgeError::failed("não há objeto ativo"));
                }
                self.modeling.checkpoint(label);
                if let Some(o) = self.modeling.active_object_mut() {
                    match kind {
                        TransformKind::Grab => o.transform.translation += v,
                        TransformKind::Rotate => o.transform.rotation = (rot * o.transform.rotation).normalize(),
                        TransformKind::Scale => o.transform.scale = (o.transform.scale * v).max(Vec3::splat(1e-3)),
                    }
                }
                self.modeling.revision += 1;
            }
            Mode::Sculpt | Mode::Hair => return Err(BridgeError::failed("transformações numéricas valem nos modos Objeto e Edição (use anigo_native_ui_action set_mode)")),
        }
        self.model_info(&Map::new())
    }


    /// Edição proporcional efetiva: a do painel (tecla O) com `patch` aplicado por cima SÓ para esta chamada.
    fn model_proportional_for(&self, patch: Option<&Value>) -> Result<crate::modeling::session::Proportional, BridgeError> {
        use anigo_modeling::ops::transform::ProportionalFalloff as F;
        let mut p = self.modeling.proportional;
        let Some(patch) = patch.filter(|v| !v.is_null()) else { return Ok(p) };
        let o = patch.as_object().ok_or_else(|| BridgeError::invalid_params("'proportional' deve ser um objeto {enabled, radius, falloff, connected_only}"))?;
        for (k, v) in o {
            match k.as_str() {
                "enabled" => p.enabled = v.as_bool().ok_or_else(|| BridgeError::invalid_params("'proportional.enabled' deve ser booleano"))?,
                "connected_only" => p.connected_only = v.as_bool().ok_or_else(|| BridgeError::invalid_params("'proportional.connected_only' deve ser booleano"))?,
                "radius" => {
                    let r = f32_of(v, "proportional.radius")?;
                    if !(0.01..=100.0).contains(&r) {
                        return Err(BridgeError::invalid_params("'proportional.radius' deve estar em 0.01..100"));
                    }
                    p.radius = r;
                }
                "falloff" => {
                    p.falloff = match v.as_str() {
                        Some("smooth") => F::Smooth,
                        Some("sphere") => F::Sphere,
                        Some("root") => F::Root,
                        Some("inverse_square") => F::InverseSquare,
                        Some("sharp") => F::Sharp,
                        Some("linear") => F::Linear,
                        Some("constant") => F::Constant,
                        Some("random") => F::Random,
                        _ => return Err(BridgeError::invalid_params("'proportional.falloff' deve ser smooth|sphere|root|inverse_square|sharp|linear|constant|random")),
                    }
                }
                other => return Err(BridgeError::invalid_params(format!("campo desconhecido 'proportional.{other}' (enabled, radius, falloff, connected_only)"))),
            }
        }
        Ok(p)
    }

    /// Opções da Modelagem (as do painel): edição proporcional, raio X e snap. Todo-ou-nada.
    fn model_options(&mut self, m: &Map<String, Value>) -> BridgeResult {
        let patch = m.get("patch").and_then(Value::as_object).ok_or_else(|| BridgeError::invalid_params("'patch' é obrigatório (proportional, xray, snap_enabled)"))?;
        let mut prop = self.modeling.proportional;
        let (mut xray, mut snap) = (self.modeling.xray, self.modeling.snap_enabled);
        for (k, v) in patch {
            match k.as_str() {
                "proportional" => prop = self.model_proportional_for(Some(v))?,
                "xray" => xray = v.as_bool().ok_or_else(|| BridgeError::invalid_params("'xray' deve ser booleano"))?,
                "snap_enabled" => snap = v.as_bool().ok_or_else(|| BridgeError::invalid_params("'snap_enabled' deve ser booleano"))?,
                other => return Err(BridgeError::invalid_params(format!("campo desconhecido 'patch.{other}' (proportional, xray, snap_enabled)"))),
            }
        }
        (self.modeling.proportional, self.modeling.xray, self.modeling.snap_enabled) = (prop, xray, snap);
        let mut out = self.model_info(&Map::new())?;
        out["options"] = json!({ "proportional": { "enabled": prop.enabled, "radius": prop.radius, "falloff": format!("{:?}", prop.falloff).to_lowercase(), "connected_only": prop.connected_only }, "xray": xray, "snap_enabled": snap });
        Ok(out)
    }

    // ------------------------------------------------------------------ escultura

    /// Câmera e tamanho (pontos lógicos) da vista ativa da Modelagem, para projetar/raio dos pincéis.
    fn model_view(&self) -> ViewInfo {
        let r = self.mview.rects[self.mview.active];
        let size = if r.is_positive() { Vec2::new(r.width(), r.height()) } else { Vec2::new(1000.0, 700.0) };
        ViewInfo { camera: self.camera, size }
    }

    fn model_sculpt(&mut self, m: &Map<String, Value>) -> BridgeResult {
        self.model_enter(Mode::Sculpt)?;
        self.sculpt.sync(&self.modeling);
        let action = str_of(m, "action")?;
        // pincel (opcional em todas as ações): vira o ativo, como clicar no shelf
        if let Some(b) = m.get("brush") {
            let name = b.as_str().ok_or_else(|| BridgeError::invalid_params("'brush' deve ser texto"))?;
            let kind = tool_id_to_brush(&format!("sculpt.{name}")).ok_or_else(|| BridgeError::invalid_params(format!("pincel '{name}' desconhecido (draw, clay, clay_strips, smooth, flatten, fill, scrape, pinch, inflate, grab, snake_hook, nudge, crease, layer, elastic, mask, mask_smooth, mask_sharpen, mask_invert, mask_clear)")))?;
            self.sculpt.active = kind;
            if let Some(i) = tool_index_for_brush(kind) {
                self.tool_idx.insert(self.ws, i);
            }
        }
        // parâmetros do pincel ativo
        {
            let kind = self.sculpt.active;
            let slot = self.sculpt.slot_mut(kind);
            if let Some(v) = m.get("radius_px") {
                slot.radius_px = f32_of(v, "radius_px")?.clamp(RADIUS_MIN_PX, RADIUS_MAX_PX);
            }
            if let Some(v) = m.get("strength") {
                slot.brush.strength = f32_of(v, "strength")?.clamp(0.0, STRENGTH_MAX);
            }
            if let Some(v) = m.get("spacing_pct") {
                slot.brush.spacing = (f32_of(v, "spacing_pct")? / 100.0).clamp(0.01, 10.0);
            }
            if let Some(v) = m.get("invert") {
                slot.brush.direction_invert = v.as_bool().ok_or_else(|| BridgeError::invalid_params("'invert' deve ser true|false"))?;
            }
        }
        if let Some(v) = m.get("mirror") {
            let a = v.as_array().filter(|a| a.len() == 3).ok_or_else(|| BridgeError::invalid_params("'mirror' deve ser [x,y,z] de true|false"))?;
            for (i, x) in a.iter().enumerate() {
                self.sculpt.symmetry.mirror[i] = x.as_bool().ok_or_else(|| BridgeError::invalid_params("'mirror' deve ser [x,y,z] de true|false"))?;
            }
        }
        if let Some(v) = m.get("radial") {
            let a = v.as_array().filter(|a| a.len() == 3).ok_or_else(|| BridgeError::invalid_params("'radial' deve ser [x,y,z] de inteiros (0 ou 1 = desligado)"))?;
            for (i, x) in a.iter().enumerate() {
                self.sculpt.symmetry.radial[i] = x.as_u64().filter(|n| *n <= 32).ok_or_else(|| BridgeError::invalid_params("'radial' deve ser [x,y,z] de inteiros 0..32"))? as u32;
            }
        }
        let report = |app: &StudioApp, msg: Value| -> BridgeResult {
            let mut v = msg;
            v["mesh"] = app.mesh_summary();
            v["brush"] = json!(brush_tool_id(app.sculpt.active).trim_start_matches("sculpt."));
            if let Some(e) = &app.sculpt.error {
                v["error"] = json!(e);
            }
            Ok(v)
        };
        match action {
            "set_brush" => report(self, json!({ "ok": true })),
            "stroke" => {
                // `frame_view`: enquadra o objeto ativo na vista ativa ANTES do traço (os pincéis só tocam o que aparece na vista)
                if m.get("frame_view").and_then(Value::as_bool).unwrap_or(false) {
                    self.frame_selected_or_all();
                }
                let view = self.model_view();
                let mut paths: Vec<Vec<Vec2>> = Vec::new();
                let mut add = |pts: Vec<Vec2>| {
                    if pts.len() >= 2 || pts.len() == 1 {
                        paths.push(pts);
                    }
                };
                let norm = |p: Vec2| p; // já normalizado (0..1 da vista)
                if let Some(p) = m.get("path") {
                    add(path_2d(p, "path")?.into_iter().map(norm).collect());
                }
                if let Some(ps) = m.get("paths") {
                    let list = ps.as_array().filter(|a| a.len() <= MAX_PATHS).ok_or_else(|| BridgeError::invalid_params(format!("'paths' deve ser uma lista de até {MAX_PATHS} traços")))?;
                    for p in list {
                        add(path_2d(p, "paths[i]")?);
                    }
                }
                if let Some(p) = m.get("world_path") {
                    add(path_world(p, "world_path", &view)?);
                }
                if paths.is_empty() {
                    return Err(BridgeError::invalid_params("informe 'path' ([[u,v],…] normalizado 0..1 da vista), 'paths' ou 'world_path' ([[x,y,z],…])"));
                }
                if paths.len() > MAX_PATHS {
                    return Err(BridgeError::invalid_params(format!("no máximo {MAX_PATHS} traços por chamada")));
                }
                let script = ScriptedStroke { brush: self.sculpt.active, invert: m.get("invert").and_then(Value::as_bool).unwrap_or(false), paths, hover: None };
                let n = script.paths.len();
                let before = self.modeling.history_labels().len();
                let dabs = self.sculpt.run_script(&mut self.modeling, &view, &script);
                let strokes = self.modeling.history_labels().len().saturating_sub(before);
                let mut out = json!({ "paths": n, "strokes_applied": strokes, "dabs": dabs, "view_size": [view.size.x, view.size.y] });
                if dabs == 0 {
                    out["warning"] = json!("nenhum dab: o traço não tocou o objeto. Os pincéis só atingem o que aparece na vista ativa e um traço só COMEÇA se o primeiro ponto cai sobre o objeto: use frame_view=true para enquadrá-lo antes e comece o traço sobre ele");
                }
                report(self, out)
            }
            "mask" => {
                match str_of(m, "what")? {
                    "invert" => self.sculpt.mask_invert(&mut self.modeling),
                    "clear" => self.sculpt.mask_clear(&mut self.modeling),
                    _ => return Err(BridgeError::invalid_params("mask.what deve ser invert|clear")),
                }
                .map_err(BridgeError::failed)?;
                report(self, json!({ "ok": true }))
            }
            "smooth_all" => {
                let iters = opt_u64_in(m, "iterations", 1, 50, 3)? as u32;
                let strength = m.get("smooth_strength").map(|v| f32_of(v, "smooth_strength")).transpose()?.unwrap_or(0.5).clamp(0.0, 1.0);
                self.sculpt.smooth_all(&mut self.modeling, iters, strength).map_err(BridgeError::failed)?;
                report(self, json!({ "ok": true, "iterations": iters }))
            }
            "symmetrize" => {
                let axis = match m.get("axis").and_then(Value::as_str).unwrap_or("x") {
                    "x" => 0,
                    "y" => 1,
                    "z" => 2,
                    _ => return Err(BridgeError::invalid_params("axis deve ser x|y|z")),
                };
                let pos_to_neg = m.get("from").and_then(Value::as_str).unwrap_or("positive") == "positive";
                let n = self.sculpt.symmetrize(&mut self.modeling, axis, pos_to_neg).map_err(BridgeError::failed)?;
                report(self, json!({ "ok": true, "vertices_changed": n }))
            }
            "voxel_remesh" => {
                if let Some(v) = m.get("voxel_size") {
                    self.sculpt.voxel_size = f32_of(v, "voxel_size")?.max(0.0);
                }
                if let Some(v) = m.get("shade_smooth") {
                    self.sculpt.voxel_shade_smooth = v.as_bool().ok_or_else(|| BridgeError::invalid_params("'shade_smooth' deve ser booleano"))?;
                }
                if let Some(v) = m.get("voxel_smooth") {
                    self.sculpt.voxel_smooth = v.as_u64().filter(|n| *n <= 10).ok_or_else(|| BridgeError::invalid_params("'voxel_smooth' deve ser 0..10"))? as u32;
                }
                let msg = self.sculpt.voxel_remesh(&mut self.modeling).map_err(BridgeError::failed)?;
                report(self, json!({ "ok": true, "message": msg }))
            }
            "dyntopo" => {
                self.sculpt.dyntopo = m.get("enabled").and_then(Value::as_bool).ok_or_else(|| BridgeError::invalid_params("dyntopo exige 'enabled' (true|false)"))?;
                if let Some(v) = m.get("detail_px") {
                    self.sculpt.dyntopo_detail_px = f32_of(v, "detail_px")?.clamp(2.0, 200.0);
                }
                report(self, json!({ "ok": true, "dyntopo": self.sculpt.dyntopo }))
            }
            other => Err(BridgeError::invalid_params(format!("sculpt.action '{other}' desconhecida (set_brush, stroke, mask, smooth_all, symmetrize, voxel_remesh, dyntopo)"))),
        }
    }

    // ------------------------------------------------------------------ histórico

    fn model_history(&mut self, m: &Map<String, Value>) -> BridgeResult {
        let steps = opt_u64_in(m, "steps", 1, 100, 1)?;
        let undo = match str_of(m, "action")? {
            "undo" => true,
            "redo" => false,
            _ => return Err(BridgeError::invalid_params("history.action deve ser undo|redo")),
        };
        let mut labels = Vec::new();
        for _ in 0..steps {
            let l = if undo { self.modeling.undo() } else { self.modeling.redo() };
            match l {
                Some(l) => labels.push(l),
                None => break,
            }
        }
        // a malha mudou por fora do estado de escultura: ressincroniza o contexto no próximo traço
        self.sculpt.clear();
        Ok(json!({ "applied": labels, "can_undo": self.modeling.can_undo(), "can_redo": self.modeling.can_redo(), "mesh": self.mesh_summary() }))
    }
}

fn path_2d(v: &Value, name: &str) -> Result<Vec<Vec2>, BridgeError> {
    let a = v.as_array().filter(|a| !a.is_empty() && a.len() <= MAX_PATH_POINTS).ok_or_else(|| BridgeError::invalid_params(format!("'{name}' deve ter de 1 a {MAX_PATH_POINTS} pontos [u,v]")))?;
    a.iter()
        .map(|p| {
            let p = vec2_of(p, name)?;
            if !(-0.5..=1.5).contains(&p.x) || !(-0.5..=1.5).contains(&p.y) {
                return Err(BridgeError::invalid_params(format!("'{name}': use coordenadas normalizadas da vista (0..1)")));
            }
            Ok(p)
        })
        .collect()
}

fn path_world(v: &Value, name: &str, view: &ViewInfo) -> Result<Vec<Vec2>, BridgeError> {
    let a = v.as_array().filter(|a| !a.is_empty() && a.len() <= MAX_PATH_POINTS).ok_or_else(|| BridgeError::invalid_params(format!("'{name}' deve ter de 1 a {MAX_PATH_POINTS} pontos [x,y,z]")))?;
    a.iter()
        .map(|p| {
            let w = vec3_of(p, name)?;
            view.camera.project(w, view.size).map(|(px, _)| px / view.size).ok_or_else(|| BridgeError::invalid_params(format!("'{name}': ponto atrás da câmera da vista ativa")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weighted_transform_attenuates_by_weight() {
        let pts = [(0u32, Vec3::new(1.0, 0.0, 0.0), 1.0), (1, Vec3::new(2.0, 0.0, 0.0), 0.5)];
        let g = weighted_transform(TransformKind::Grab, &pts, Vec3::ZERO, Vec3::new(0.0, 2.0, 0.0), Quat::IDENTITY);
        assert_eq!(g[0].1, Vec3::new(1.0, 2.0, 0.0));
        assert_eq!(g[1].1, Vec3::new(2.0, 1.0, 0.0), "peso 0,5 move metade");
        let rot = Quat::from_rotation_z(90f32.to_radians());
        let r = weighted_transform(TransformKind::Rotate, &pts, Vec3::ZERO, Vec3::ZERO, rot);
        assert!((r[0].1 - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-5);
        let half = Quat::from_rotation_z(45f32.to_radians()) * Vec3::new(2.0, 0.0, 0.0);
        assert!((r[1].1 - half).length() < 1e-5, "peso 0,5 gira metade do ângulo");
        let s = weighted_transform(TransformKind::Scale, &pts, Vec3::ZERO, Vec3::splat(3.0), Quat::IDENTITY);
        assert_eq!(s[0].1, Vec3::new(3.0, 0.0, 0.0));
        assert_eq!(s[1].1, Vec3::new(4.0, 0.0, 0.0), "fator 1 + (3-1)*0,5 = 2");
    }
}
