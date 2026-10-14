//! Cabelo pela bridge (`HAIR`): o MESMO caminho do Modo Cabelo — cabeça-alvo, grupos, mechas (desenhar/pentear/editar pontos),
//! parâmetros por JSON parcial, material do grupo, física de bounce, regenerar malha e exportar. Entra no Modo Cabelo sozinho.
//!
//! Um comando que falha não altera o penteado nem sujam o histórico (os `checkpoint`s são descartados como na UI). O histórico
//! é o da Modelagem (`MODEL history`). Parâmetros/enums seguem os nomes do JSON do `anigo-modeling`
//! (`StrandParams`, `HairSection`, `HairClump`, `HairMaterial`, `BounceGroup`); `twist_deg`/`curl_deg` são atalhos em graus.

use super::StudioApp;
use crate::modeling::sculpt::ViewInfo;
use crate::modeling::Mode;
use crate::workspace::WorkspaceId;
use anigo_bridge::protocol::{BridgeError, BridgeResult};
use anigo_modeling::hair::{StrandParams, StrandRef, UvLayout};
use glam::{Vec2, Vec3};
use serde_json::{json, Map, Value};

const MAX_POINTS: usize = 512;

fn str_of<'a>(m: &'a Map<String, Value>, k: &str) -> Result<&'a str, BridgeError> {
    m.get(k).and_then(Value::as_str).ok_or_else(|| BridgeError::invalid_params(format!("'{k}' é obrigatório e deve ser texto")))
}

fn id_of(m: &Map<String, Value>, k: &str) -> Result<Option<u32>, BridgeError> {
    match m.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().filter(|n| *n < u32::MAX as u64).map(|n| Some(n as u32)).ok_or_else(|| BridgeError::invalid_params(format!("'{k}' deve ser inteiro >= 0"))),
    }
}

fn f32_of(v: &Value, name: &str) -> Result<f32, BridgeError> {
    v.as_f64().filter(|f| f.is_finite() && f.abs() < 1.0e6).map(|f| f as f32).ok_or_else(|| BridgeError::invalid_params(format!("'{name}' deve ser um número finito")))
}

fn arr<const N: usize>(v: &Value, name: &str) -> Result<[f32; N], BridgeError> {
    let a = v.as_array().filter(|a| a.len() == N).ok_or_else(|| BridgeError::invalid_params(format!("'{name}' deve ter {N} números")))?;
    let mut o = [0f32; N];
    for (i, x) in a.iter().enumerate() {
        o[i] = f32_of(x, name)?;
    }
    Ok(o)
}

fn err_s(e: impl ToString) -> BridgeError {
    BridgeError::failed(e.to_string())
}

/// Maiúscula inicial (aceita `flat`, `Flat`, `FLAT` para os enums do JSON).
fn canon_enum(s: &str) -> String {
    let lower = s.to_ascii_lowercase().replace('_', "");
    match lower.as_str() {
        "lengthv" => "LengthV".into(),
        "lengthu" => "LengthU".into(),
        _ => {
            let mut c = lower.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        }
    }
}

/// Mescla `patch` em `base` (objetos recursivos; o resto substitui). `null` apaga (`clump: null` desliga a mecha composta).
fn merge(base: &mut Value, patch: &Value) {
    match (base.is_object(), patch) {
        (true, Value::Object(p)) => {
            for (k, v) in p {
                let slot = base.as_object_mut().expect("objeto").entry(k.clone()).or_insert(Value::Null);
                if slot.is_null() && v.is_object() {
                    // campo opcional ligado pela primeira vez: o padrão do tipo é preenchido pelo chamador antes
                    *slot = Value::Object(Default::default());
                }
                merge(slot, v);
            }
        }
        _ => *base = patch.clone(),
    }
}

/// `base` + patch JSON parcial → `StrandParams` validado.
pub(crate) fn strand_params_from(base: StrandParams, patch: &Value) -> Result<StrandParams, BridgeError> {
    let Value::Object(p) = patch else { return Err(BridgeError::invalid_params("'params' deve ser um objeto")) };
    let mut p = p.clone();
    // atalhos em graus
    if let Some(Value::Object(sec)) = p.get_mut("section") {
        for (deg, rad) in [("twist_deg", "twist"), ("curl_deg", "curl")] {
            if let Some(v) = sec.remove(deg) {
                sec.insert(rad.into(), json!(f32_of(&v, deg)?.to_radians()));
            }
        }
        if let Some(Value::String(k)) = sec.get_mut("kind") {
            *k = canon_enum(k);
        }
    }
    let mut v = serde_json::to_value(base).map_err(err_s)?;
    if p.get("clump").is_some_and(Value::is_object) && v["clump"].is_null() {
        v["clump"] = serde_json::to_value(anigo_modeling::hair::HairClump::default()).map_err(err_s)?;
    }
    super::bridge_model::reject_unknown_keys(&v, &Value::Object(p.clone()), "params")?;
    merge(&mut v, &Value::Object(p));
    let out: StrandParams = serde_json::from_value(v).map_err(|e| BridgeError::invalid_params(format!("params inválidos: {e}")))?;
    out.validate().map_err(err_s)?;
    Ok(out)
}

fn params_json(p: &StrandParams) -> Value {
    let mut v = serde_json::to_value(p).unwrap_or(Value::Null);
    v["section"]["twist_deg"] = json!(p.section.twist.to_degrees());
    v["section"]["curl_deg"] = json!(p.section.curl.to_degrees());
    v
}

/// Densifica uma polilinha em pixels (um ponto a cada ~5 px), como o mouse faria.
fn densify(px: &[Vec2]) -> Vec<Vec2> {
    let mut out = Vec::new();
    for (i, p) in px.iter().enumerate() {
        if i == 0 {
            out.push(*p);
            continue;
        }
        let a = px[i - 1];
        let steps = ((*p - a).length() / 5.0).ceil().max(1.0) as usize;
        for s in 1..=steps.min(2000) {
            out.push(a.lerp(*p, s as f32 / steps as f32));
        }
    }
    out
}

impl StudioApp {
    fn hair_view(&self) -> ViewInfo {
        let r = self.mview.rects[self.mview.active];
        let size = if r.is_positive() { Vec2::new(r.width(), r.height()) } else { Vec2::new(1000.0, 700.0) };
        ViewInfo { camera: self.camera, size }
    }

    /// Pontos da pincelada em pixels da vista ativa: `path` (normalizado 0..1), `world_path` ou `local_path` (espaço da cabeça).
    fn hair_path_px(&self, m: &Map<String, Value>, view: &ViewInfo) -> Result<Vec<Vec2>, BridgeError> {
        let list = |k: &str| -> Result<Option<&Vec<Value>>, BridgeError> {
            match m.get(k) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => v.as_array().filter(|a| !a.is_empty() && a.len() <= MAX_POINTS).map(Some).ok_or_else(|| BridgeError::invalid_params(format!("'{k}' deve ter de 1 a {MAX_POINTS} pontos"))),
            }
        };
        let project = |w: Vec3| -> Result<Vec2, BridgeError> {
            view.camera.project(w, view.size).map(|(px, _)| px).ok_or_else(|| BridgeError::invalid_params("ponto atrás da câmera da vista ativa"))
        };
        let mut px: Vec<Vec2> = Vec::new();
        if let Some(a) = list("path")? {
            for p in a {
                let [u, v] = arr::<2>(p, "path")?;
                if !(-0.5..=1.5).contains(&u) || !(-0.5..=1.5).contains(&v) {
                    return Err(BridgeError::invalid_params("'path': use coordenadas normalizadas da vista (0..1)"));
                }
                px.push(Vec2::new(u, v) * view.size);
            }
        } else if let Some(a) = list("world_path")? {
            for p in a {
                px.push(project(Vec3::from(arr::<3>(p, "world_path")?))?);
            }
        } else if let Some(a) = list("local_path")? {
            let model = self.modeling.hair_target().map(|t| t.model_matrix()).ok_or_else(|| BridgeError::failed("defina a cabeça-alvo antes (hair target)"))?;
            for p in a {
                px.push(project(model.transform_point3(Vec3::from(arr::<3>(p, "local_path")?)))?);
            }
        } else {
            return Err(BridgeError::invalid_params("informe 'path' (normalizado 0..1 da vista), 'world_path' ([[x,y,z],…]) ou 'local_path' (espaço da cabeça)"));
        }
        Ok(px)
    }

    fn hair_enter_mode(&mut self) -> Result<(), BridgeError> {
        if self.modeling.mode != Mode::Hair {
            self.modeling.set_mode(Mode::Hair);
            self.tool_idx.insert(self.ws, 0);
        }
        if self.modeling.mode != Mode::Hair {
            return Err(BridgeError::failed(format!("não foi possível entrar no Modo Cabelo: {}", self.modeling.status)));
        }
        self.hair.sync(&self.modeling);
        self.hair.prune(&self.modeling);
        Ok(())
    }

    fn hair_regen(&mut self, group: Option<u32>) -> Result<(), BridgeError> {
        self.modeling.hair_regenerate(group).map_err(err_s)
    }

    pub(crate) fn bridge_hair(&mut self, params: &Value) -> BridgeResult {
        let m = params.as_object().ok_or_else(|| BridgeError::invalid_params("params deve ser um objeto"))?;
        let op = str_of(m, "op")?;
        if self.ws != WorkspaceId::Modelagem {
            self.switch_workspace(WorkspaceId::Modelagem);
        }
        match op {
            "info" => self.hair_info(m),
            "target" => self.hair_target_op(m),
            "group" => self.hair_group_op(m),
            "strand" => self.hair_strand_op(m),
            "draw" => self.hair_draw_op(m),
            "comb" => self.hair_comb_op(m),
            "bounce" => self.hair_bounce_op(m),
            "regenerate" => {
                self.hair_enter_mode()?;
                self.hair_regen(None)?;
                self.hair_info(&Map::new())
            }
            "export" => {
                self.export_hair_to_library();
                Ok(json!({ "status": self.modeling.status }))
            }
            other => Err(BridgeError::invalid_params(format!("HAIR op '{other}' desconhecida (info, target, group, strand, draw, comb, bounce, regenerate, export)"))),
        }
    }

    // ------------------------------------------------------------------ info

    fn hair_info(&self, m: &Map<String, Value>) -> BridgeResult {
        let limit = m.get("limit").and_then(Value::as_u64).map(|n| (n as usize).min(500)).unwrap_or(50);
        let want_strands = m.get("include_strands").and_then(Value::as_bool).unwrap_or(false);
        let h = &self.modeling.doc.hair;
        let groups: Vec<Value> = h
            .groups
            .iter()
            .map(|g| {
                let mut v = json!({
                    "id": g.id, "name": g.name, "strands": g.strands.len(), "symmetry": g.symmetry, "visible": g.visible,
                    "object_id": g.object_id, "active": self.hair.group == Some(g.id),
                    "material": g.material, "uv_layout": format!("{:?}", g.uv_layout),
                });
                if want_strands {
                    v["strand_list"] = json!(g.strands.iter().take(limit).map(|s| json!({
                        "id": s.id, "length": s.length(), "root": s.root().to_array(), "tip": s.tip().to_array(),
                        "points": s.guide.points.iter().map(|p| p.pos.to_array()).collect::<Vec<_>>(),
                        "params": params_json(&s.params),
                    })).collect::<Vec<_>>());
                }
                v
            })
            .collect();
        Ok(json!({
            "mode": if self.modeling.mode == Mode::Hair { "hair" } else { "other" },
            "target": self.modeling.hair_target().map(|o| json!({ "id": o.id, "name": o.name })),
            "groups": groups,
            "strand_count": h.strand_count(),
            "selected": self.hair.selected.map(|r| json!({ "group": r.group, "strand": r.strand, "point": self.hair.point })),
            "template": params_json(&self.hair.template),
            "stroke": { "resample_step": self.hair.stroke.resample_step, "smooth_iters": self.hair.stroke.smooth_iters, "rdp_epsilon": self.hair.stroke.rdp_epsilon, "lift_root": self.hair.stroke.lift_root, "lift_tip": self.hair.stroke.lift_tip, "max_points": self.hair.stroke.max_points, "min_length": self.hair.stroke.min_length },
            "comb": { "radius_px": self.hair.comb_radius_px, "strength": self.hair.comb_strength, "all_groups": self.hair.comb_all_groups },
            "bounce": h.bounce,
            "error": self.hair.error,
            "status": self.modeling.status,
        }))
    }

    // ------------------------------------------------------------------ alvo

    fn hair_target_op(&mut self, m: &Map<String, Value>) -> BridgeResult {
        if let Some(i) = m.get("object_index").and_then(Value::as_u64) {
            let i = i as usize;
            if i >= self.modeling.doc.objects.len() {
                return Err(BridgeError::invalid_params(format!("object_index {i} fora do intervalo (0..{})", self.modeling.doc.objects.len())));
            }
            self.modeling.set_mode(Mode::Object);
            self.modeling.set_active(i);
        }
        if self.modeling.active_object().is_none() {
            return Err(BridgeError::failed("não há objeto ativo para ser a cabeça-alvo"));
        }
        self.modeling.checkpoint("Definir cabeça do cabelo");
        if !self.modeling.hair_set_target_from_active() {
            self.modeling.discard_checkpoint();
            return Err(BridgeError::failed("não foi possível definir a cabeça"));
        }
        self.hair_enter_mode()?;
        self.hair_regen(None)?;
        self.hair_info(&Map::new())
    }

    // ------------------------------------------------------------------ grupos

    fn hair_group_ref(&mut self, m: &Map<String, Value>) -> Result<u32, BridgeError> {
        let g = match id_of(m, "group")? {
            Some(g) => g,
            None => self.hair.active_group(&self.modeling).ok_or_else(|| BridgeError::failed("não há grupo de cabelo (crie um com group new)"))?,
        };
        if self.modeling.doc.hair.group(g).is_none() {
            return Err(BridgeError::invalid_params(format!("grupo {g} não existe")));
        }
        Ok(g)
    }

    fn hair_group_op(&mut self, m: &Map<String, Value>) -> BridgeResult {
        self.hair_enter_mode()?;
        match str_of(m, "action")? {
            "new" => {
                let n = self.modeling.doc.hair.groups.len() + 1;
                let name = m.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("Grupo {n}"));
                self.modeling.checkpoint("Novo grupo");
                match self.modeling.doc.hair.add_group(name) {
                    Ok(id) => self.hair.group = Some(id),
                    Err(e) => {
                        self.modeling.discard_checkpoint();
                        return Err(err_s(e));
                    }
                }
            }
            "activate" => {
                let g = self.hair_group_ref(m)?;
                self.hair.group = Some(g);
            }
            "rename" => {
                let g = self.hair_group_ref(m)?;
                let name = str_of(m, "name")?.trim().to_string();
                if name.is_empty() {
                    return Err(BridgeError::invalid_params("'name' vazio"));
                }
                self.modeling.checkpoint("Renomear grupo");
                if let Err(e) = self.modeling.doc.hair.rename_group(g, name) {
                    self.modeling.discard_checkpoint();
                    return Err(err_s(e));
                }
                self.hair_regen(Some(g))?;
            }
            "duplicate" => {
                let g = self.hair_group_ref(m)?;
                self.modeling.checkpoint("Duplicar grupo");
                match self.modeling.doc.hair.duplicate_group(g) {
                    Ok(id) => {
                        self.hair.group = Some(id);
                        self.hair_regen(Some(id))?;
                    }
                    Err(e) => {
                        self.modeling.discard_checkpoint();
                        return Err(err_s(e));
                    }
                }
            }
            "flip" => {
                let g = self.hair_group_ref(m)?;
                self.modeling.checkpoint("Espelhar grupo");
                match self.modeling.doc.hair.flip_group(g) {
                    Ok(id) => {
                        self.hair.group = Some(id);
                        self.hair_regen(Some(id))?;
                    }
                    Err(e) => {
                        self.modeling.discard_checkpoint();
                        return Err(err_s(e));
                    }
                }
            }
            "delete" => {
                let g = self.hair_group_ref(m)?;
                self.modeling.hair_remove_group(g).map_err(err_s)?;
                self.hair.prune(&self.modeling);
            }
            "symmetry" => {
                let g = self.hair_group_ref(m)?;
                let on = m.get("on").and_then(Value::as_bool).ok_or_else(|| BridgeError::invalid_params("symmetry exige 'on' (true|false)"))?;
                self.modeling.checkpoint("Simetria do grupo");
                if let Err(e) = self.modeling.doc.hair.set_symmetry(g, on) {
                    self.modeling.discard_checkpoint();
                    return Err(err_s(e));
                }
                self.hair_regen(Some(g))?;
            }
            "visible" => {
                let g = self.hair_group_ref(m)?;
                let on = m.get("on").and_then(Value::as_bool).ok_or_else(|| BridgeError::invalid_params("visible exige 'on' (true|false)"))?;
                self.modeling.checkpoint("Visibilidade do grupo");
                if let Some(gr) = self.modeling.doc.hair.group_mut(g) {
                    gr.visible = on;
                }
                self.hair_regen(Some(g))?;
            }
            "material" => {
                let g = self.hair_group_ref(m)?;
                let patch = m.get("patch").and_then(Value::as_object).ok_or_else(|| BridgeError::invalid_params("material exige 'patch': {base_color,shade_color,highlight_color:[r,g,b] 0..1, root_ao, normal_bias, name, uv_layout}"))?;
                let (mut mat, mut layout) = {
                    let gr = self.modeling.doc.hair.group(g).ok_or_else(|| BridgeError::invalid_params("grupo inexistente"))?;
                    (gr.material.clone(), gr.uv_layout)
                };
                for (k, v) in patch {
                    match k.as_str() {
                        "base_color" => mat.base_color = arr::<3>(v, k)?.map(|c| c.clamp(0.0, 1.0)),
                        "shade_color" => mat.shade_color = arr::<3>(v, k)?.map(|c| c.clamp(0.0, 1.0)),
                        "highlight_color" => mat.highlight_color = arr::<3>(v, k)?.map(|c| c.clamp(0.0, 1.0)),
                        "root_ao" => mat.root_ao = f32_of(v, k)?.clamp(0.0, 1.0),
                        "normal_bias" => mat.normal_bias = f32_of(v, k)?.clamp(0.0, 1.0),
                        "name" => mat.name = v.as_str().filter(|s| !s.is_empty() && s.len() <= 64).ok_or_else(|| BridgeError::invalid_params("name: texto de 1..64"))?.to_string(),
                        "uv_layout" => {
                            layout = match canon_enum(v.as_str().unwrap_or("")).as_str() {
                                "LengthV" => UvLayout::LengthV,
                                "LengthU" => UvLayout::LengthU,
                                _ => return Err(BridgeError::invalid_params("uv_layout deve ser length_v|length_u")),
                            }
                        }
                        other => return Err(BridgeError::invalid_params(format!("material: chave desconhecida '{other}'"))),
                    }
                }
                self.modeling.checkpoint("Material do grupo");
                if let Some(gr) = self.modeling.doc.hair.group_mut(g) {
                    gr.material = mat;
                    gr.uv_layout = layout;
                }
                self.hair_regen(Some(g))?;
            }
            other => return Err(BridgeError::invalid_params(format!("group.action '{other}' desconhecida (new, activate, rename, duplicate, flip, delete, symmetry, visible, material)"))),
        }
        self.hair_info(&Map::new())
    }

    // ------------------------------------------------------------------ mechas

    fn hair_strand_ref(&self, m: &Map<String, Value>) -> Result<StrandRef, BridgeError> {
        let r = match (id_of(m, "group")?, id_of(m, "strand")?) {
            (Some(group), Some(strand)) => StrandRef { group, strand },
            (None, None) => self.hair.selected.ok_or_else(|| BridgeError::failed("nenhuma mecha selecionada (informe group+strand ou use strand select)"))?,
            _ => return Err(BridgeError::invalid_params("informe group E strand juntos")),
        };
        if self.modeling.doc.hair.strand(r).is_none() {
            return Err(BridgeError::invalid_params(format!("a mecha {}/{} não existe", r.group, r.strand)));
        }
        Ok(r)
    }

    fn hair_strand_op(&mut self, m: &Map<String, Value>) -> BridgeResult {
        self.hair_enter_mode()?;
        let action = str_of(m, "action")?;
        if action == "set_params" && m.get("apply").and_then(Value::as_str) == Some("template") {
            let patch = m.get("params").ok_or_else(|| BridgeError::invalid_params("set_params exige 'params'"))?;
            self.hair.template = strand_params_from(self.hair.template, patch)?;
            return self.hair_info(&Map::new());
        }
        let r = self.hair_strand_ref(m)?;
        match action {
            "select" => {
                let point = m.get("point").and_then(Value::as_u64).map(|p| p as usize);
                let n = self.modeling.doc.hair.strand(r).map(|s| s.guide.points.len()).unwrap_or(0);
                if point.is_some_and(|p| p >= n) {
                    return Err(BridgeError::invalid_params(format!("ponto fora do intervalo (0..{n})")));
                }
                self.hair.select(Some(r), point);
            }
            "set_params" => {
                let patch = m.get("params").ok_or_else(|| BridgeError::invalid_params("set_params exige 'params'"))?;
                let base = self.modeling.doc.hair.strand(r).map(|s| s.params).unwrap_or_default();
                let p = strand_params_from(base, patch)?;
                self.modeling.checkpoint("Parâmetros da mecha");
                let res = if m.get("apply").and_then(Value::as_str) == Some("group") { self.modeling.doc.hair.set_group_params(r.group, p) } else { self.modeling.doc.hair.set_params(r, p) };
                if let Err(e) = res {
                    self.modeling.discard_checkpoint();
                    return Err(err_s(e));
                }
                self.hair_regen(Some(r.group))?;
            }
            "simplify" => {
                let eps = m.get("epsilon").map(|v| f32_of(v, "epsilon")).transpose()?.unwrap_or(0.006).clamp(0.0001, 0.2);
                self.modeling.checkpoint("Simplificar mecha");
                if let Some(s) = self.modeling.doc.hair.strand_mut(r) {
                    s.simplify(eps);
                }
                self.modeling.doc.hair.sync_mirror(r);
                self.hair_regen(Some(r.group))?;
            }
            "duplicate" => {
                self.hair.select(Some(r), None);
                self.hair.duplicate_selected(&mut self.modeling).map_err(err_s)?;
            }
            "delete" => {
                self.hair.select(Some(r), None);
                self.hair.delete_selected(&mut self.modeling, true).map_err(err_s)?;
            }
            "move_to_group" => {
                let to = id_of(m, "to_group")?.ok_or_else(|| BridgeError::invalid_params("move_to_group exige 'to_group'"))?;
                self.modeling.checkpoint("Mover mecha de grupo");
                if let Err(e) = self.modeling.doc.hair.move_strand(r, to) {
                    self.modeling.discard_checkpoint();
                    return Err(err_s(e));
                }
                self.hair_regen(None)?;
                self.hair.select(Some(StrandRef { group: to, strand: r.strand }), None);
            }
            "insert_point" => {
                let after = m.get("after").and_then(Value::as_u64).ok_or_else(|| BridgeError::invalid_params("insert_point exige 'after' (índice do ponto)"))? as usize;
                self.hair.insert_point(&mut self.modeling, r, after).map_err(err_s)?;
            }
            "remove_point" => {
                let idx = m.get("index").and_then(Value::as_u64).ok_or_else(|| BridgeError::invalid_params("remove_point exige 'index'"))? as usize;
                self.hair.select(Some(r), Some(idx));
                self.hair.delete_selected(&mut self.modeling, false).map_err(err_s)?;
            }
            "move_point" => {
                let idx = m.get("index").and_then(Value::as_u64).ok_or_else(|| BridgeError::invalid_params("move_point exige 'index'"))? as usize;
                let pos = match (m.get("pos"), m.get("world_pos")) {
                    (Some(p), None) => Vec3::from(arr::<3>(p, "pos")?),
                    (None, Some(w)) => {
                        let model = self.modeling.hair_target().map(|t| t.model_matrix()).ok_or_else(|| BridgeError::failed("sem cabeça-alvo"))?;
                        model.inverse().transform_point3(Vec3::from(arr::<3>(w, "world_pos")?))
                    }
                    _ => return Err(BridgeError::invalid_params("informe 'pos' (espaço da cabeça) OU 'world_pos'")),
                };
                self.modeling.checkpoint("Mover ponto da mecha");
                if let Err(e) = self.modeling.doc.hair.set_point_pos(r, idx, pos) {
                    self.modeling.discard_checkpoint();
                    return Err(err_s(e));
                }
                self.hair_regen(Some(r.group))?;
            }
            other => return Err(BridgeError::invalid_params(format!("strand.action '{other}' desconhecida (select, set_params, simplify, duplicate, delete, move_to_group, insert_point, remove_point, move_point)"))),
        }
        self.hair_info(&Map::new())
    }

    // ------------------------------------------------------------------ desenhar / pentear

    fn hair_apply_stroke_patch(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        if let Some(p) = m.get("params") {
            self.hair.template = strand_params_from(self.hair.template, p)?;
        }
        if let Some(Value::Object(s)) = m.get("stroke") {
            let mut st = self.hair.stroke;
            for (k, v) in s {
                match k.as_str() {
                    "resample_step" => st.resample_step = f32_of(v, k)?.clamp(0.005, 0.5),
                    "smooth_iters" => st.smooth_iters = v.as_u64().filter(|n| *n <= 20).ok_or_else(|| BridgeError::invalid_params("smooth_iters: 0..20"))? as u32,
                    "rdp_epsilon" => st.rdp_epsilon = f32_of(v, k)?.clamp(0.0001, 0.2),
                    "lift_root" => st.lift_root = f32_of(v, k)?.clamp(0.0, 0.5),
                    "lift_tip" => st.lift_tip = f32_of(v, k)?.clamp(0.0, 1.0),
                    "max_points" => st.max_points = v.as_u64().filter(|n| (2..=64).contains(n)).ok_or_else(|| BridgeError::invalid_params("max_points: 2..64"))? as usize,
                    "min_length" => st.min_length = f32_of(v, k)?.max(0.0),
                    other => return Err(BridgeError::invalid_params(format!("stroke: chave desconhecida '{other}'"))),
                }
            }
            self.hair.stroke = st;
        }
        if let Some(g) = id_of(m, "group")? {
            if self.modeling.doc.hair.group(g).is_none() {
                return Err(BridgeError::invalid_params(format!("grupo {g} não existe")));
            }
            self.hair.group = Some(g);
        }
        Ok(())
    }

    fn hair_draw_op(&mut self, m: &Map<String, Value>) -> BridgeResult {
        self.hair_enter_mode()?;
        if self.modeling.hair_target().is_none() {
            return Err(BridgeError::failed("defina a cabeça-alvo antes (hair target)"));
        }
        self.hair_apply_stroke_patch(m)?;
        let view = self.hair_view();
        let px = densify(&self.hair_path_px(m, &view)?);
        let before = self.modeling.doc.hair.strand_count();
        let mut rays = px.iter().map(|p| view.ray(*p));
        let first = rays.next().ok_or_else(|| BridgeError::invalid_params("traço vazio"))?;
        if !self.hair.begin_draw(&mut self.modeling, first).map_err(err_s)? {
            return Err(BridgeError::failed("o traço não tocou a cabeça (confira o enquadramento da vista ativa)"));
        }
        for r in rays {
            self.hair.continue_draw(&self.modeling, r);
        }
        let created = self.hair.end_draw(&mut self.modeling).map_err(err_s)?;
        let Some(r) = created else {
            return Err(BridgeError::failed("traço curto demais: nenhuma mecha criada (alongue o traço)"));
        };
        let mut out = self.hair_info(&Map::new())?;
        out["created"] = json!({ "group": r.group, "strand": r.strand });
        out["strands_added"] = json!(self.modeling.doc.hair.strand_count().saturating_sub(before));
        Ok(out)
    }

    fn hair_comb_op(&mut self, m: &Map<String, Value>) -> BridgeResult {
        self.hair_enter_mode()?;
        if let Some(v) = m.get("radius_px") {
            self.hair.comb_radius_px = f32_of(v, "radius_px")?.clamp(10.0, 400.0);
        }
        if let Some(v) = m.get("strength") {
            self.hair.comb_strength = f32_of(v, "strength")?.clamp(0.02, 1.0);
        }
        if let Some(v) = m.get("all_groups") {
            self.hair.comb_all_groups = v.as_bool().ok_or_else(|| BridgeError::invalid_params("all_groups: true|false"))?;
        }
        if let Some(g) = id_of(m, "group")? {
            if self.modeling.doc.hair.group(g).is_none() {
                return Err(BridgeError::invalid_params(format!("grupo {g} não existe")));
            }
            self.hair.group = Some(g);
        }
        let view = self.hair_view();
        let px = densify(&self.hair_path_px(m, &view)?);
        let first = *px.first().ok_or_else(|| BridgeError::invalid_params("traço vazio"))?;
        self.hair.begin_comb(first);
        let mut touched = 0;
        for p in &px[1..] {
            touched += self.hair.comb_move(&mut self.modeling, &view, *p).map_err(err_s)?;
        }
        self.hair.end_comb();
        let mut out = self.hair_info(&Map::new())?;
        out["segments_combed"] = json!(touched);
        if touched == 0 {
            out["warning"] = json!("nenhum segmento ficou dentro do raio do pente (aumente radius_px ou passe sobre as mechas)");
        }
        Ok(out)
    }

    // ------------------------------------------------------------------ bounce (física)

    fn hair_bounce_op(&mut self, m: &Map<String, Value>) -> BridgeResult {
        self.hair_enter_mode()?;
        match str_of(m, "action")? {
            "info" => {}
            "auto" => {
                let groups = m.get("groups").and_then(Value::as_u64).filter(|n| (1..=30).contains(n)).unwrap_or(self.hair.bounce_groups as u64) as usize;
                let bones = m.get("bones").and_then(Value::as_u64).filter(|n| (1..=16).contains(n)).unwrap_or(self.hair.bounce_bones as u64) as u32;
                self.hair.bounce_groups = groups;
                self.hair.bounce_bones = bones;
                self.modeling.checkpoint("Auto-gerar grupos de bounce");
                if let Err(e) = self.modeling.doc.hair.auto_bounce(groups, bones) {
                    self.modeling.discard_checkpoint();
                    return Err(err_s(e));
                }
            }
            "set" => {
                let idx = m.get("index").and_then(Value::as_u64).ok_or_else(|| BridgeError::invalid_params("set exige 'index' (posição na lista bounce de info)"))? as usize;
                let patch = m.get("patch").and_then(Value::as_object).ok_or_else(|| BridgeError::invalid_params("set exige 'patch': {name,bones,fixed_point,stiffness,drag,gravity,hit_radius}"))?;
                let mut nb = self.modeling.doc.hair.bounce.get(idx).cloned().ok_or_else(|| BridgeError::invalid_params(format!("grupo de bounce {idx} não existe")))?;
                for (k, v) in patch {
                    match k.as_str() {
                        "name" => nb.name = v.as_str().filter(|s| !s.is_empty() && s.len() <= 64).ok_or_else(|| BridgeError::invalid_params("name: 1..64"))?.to_string(),
                        "bones" => nb.bones = v.as_u64().filter(|n| (1..=16).contains(n)).ok_or_else(|| BridgeError::invalid_params("bones: 1..16"))? as u32,
                        "fixed_point" => nb.fixed_point = f32_of(v, k)?,
                        "stiffness" => nb.stiffness = f32_of(v, k)?,
                        "drag" => nb.drag = f32_of(v, k)?,
                        "gravity" => nb.gravity = f32_of(v, k)?,
                        "hit_radius" => nb.hit_radius = f32_of(v, k)?,
                        other => return Err(BridgeError::invalid_params(format!("bounce: chave desconhecida '{other}'"))),
                    }
                }
                nb.validate().map_err(err_s)?;
                self.modeling.checkpoint("Parâmetros de bounce");
                self.modeling.doc.hair.bounce[idx] = nb;
            }
            other => return Err(BridgeError::invalid_params(format!("bounce.action '{other}' desconhecida (info, auto, set)"))),
        }
        let chains = self.modeling.doc.hair.generate_chains().map(|c| c.len()).unwrap_or(0);
        let mut out = self.hair_info(&Map::new())?;
        out["chains"] = json!(chains);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_modeling::hair::SectionKind;

    #[test]
    fn partial_json_params_merge_validate_and_convert_degrees() {
        let base = StrandParams::default();
        let p = strand_params_from(base, &json!({ "section": { "kind": "tube", "width": 0.1, "twist_deg": 90.0 }, "resolution": 20, "clump": { "count": 8 } })).unwrap();
        assert_eq!(p.section.kind, SectionKind::Tube);
        assert_eq!(p.section.width, 0.1);
        assert!((p.section.twist - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert_eq!(p.resolution, 20);
        assert_eq!(p.clump.map(|c| c.count), Some(8), "clump ligado com o padrão do tipo + o que veio no patch");
        // o que não veio fica como estava
        assert_eq!(p.tip_taper, base.tip_taper);
        // clump: null desliga
        let off = strand_params_from(p, &json!({ "clump": null })).unwrap();
        assert!(off.clump.is_none());
    }

    #[test]
    fn invalid_params_are_rejected_without_partial_effects() {
        let base = StrandParams::default();
        for bad in [json!({ "resolution": 0 }), json!({ "resolution": 9999 }), json!({ "section": { "width": -1.0 } }), json!({ "section": { "kind": "pentagono" } }), json!({ "tip_taper": 2.0 }), json!({ "bogus": 1 }), json!(5)] {
            assert!(strand_params_from(base, &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn densify_adds_points_about_every_five_pixels() {
        let d = densify(&[Vec2::ZERO, Vec2::new(50.0, 0.0)]);
        assert_eq!(d.first(), Some(&Vec2::ZERO));
        assert_eq!(d.last(), Some(&Vec2::new(50.0, 0.0)));
        assert!(d.len() >= 11 && d.windows(2).all(|w| (w[1] - w[0]).length() <= 5.0 + 1e-3));
    }

    #[test]
    fn enum_names_are_case_and_underscore_insensitive() {
        assert_eq!(canon_enum("flat"), "Flat");
        assert_eq!(canon_enum("ARC"), "Arc");
        assert_eq!(canon_enum("length_v"), "LengthV");
    }
}
