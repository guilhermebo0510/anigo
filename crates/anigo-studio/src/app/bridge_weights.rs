//! Pesos pela bridge (`WEIGHTS`): os canais NPR por vértice (AO, deslocamento de sombra, largura de contorno, máscara
//! spec/rim — a ferramenta "Pintar atributo") e os **pesos de skin** (joints/pesos de cada vértice), que ainda não têm ferramenta
//! no painel: atribuir, pintar com pincel esférico, normalizar, limitar influências, suavizar, transferir de outro objeto,
//! auto-peso por envelope de ossos e espelhar.
//!
//! Atua na malha do objeto ATIVO, em espaço do objeto (use `world_center`/`world_*` para dar posições de mundo: são convertidas
//! pelo transform do objeto). Cada operação é transacional (`edit_mesh`): em erro nada muda e o histórico fica intacto; em
//! sucesso é um passo do histórico de Modelagem. `use_selection=true` restringe aos vértices selecionados (modo Edição).

use super::StudioApp;
use crate::workspace::WorkspaceId;
use anigo_bridge::protocol::{BridgeError, BridgeResult};
use anigo_modeling::attributes::{self as attr, BoneEnvelope, BrushFalloff, MirrorAxis, PaintBlend, VertexChannel};
use anigo_modeling::{EditMesh, ModelError};
use glam::Vec3;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

fn str_of<'a>(m: &'a Map<String, Value>, k: &str) -> Result<&'a str, BridgeError> {
    m.get(k).and_then(Value::as_str).ok_or_else(|| BridgeError::invalid_params(format!("'{k}' é obrigatório e deve ser texto")))
}

fn f32_of(v: &Value, name: &str) -> Result<f32, BridgeError> {
    v.as_f64().filter(|f| f.is_finite() && f.abs() < 1.0e6).map(|f| f as f32).ok_or_else(|| BridgeError::invalid_params(format!("'{name}' deve ser um número finito")))
}

fn num(m: &Map<String, Value>, k: &str, default: f32) -> Result<f32, BridgeError> {
    m.get(k).map(|v| f32_of(v, k)).transpose().map(|o| o.unwrap_or(default))
}

fn vec3_of(v: &Value, name: &str) -> Result<Vec3, BridgeError> {
    let a = v.as_array().filter(|a| a.len() == 3).ok_or_else(|| BridgeError::invalid_params(format!("'{name}' deve ter 3 números")))?;
    Ok(Vec3::new(f32_of(&a[0], name)?, f32_of(&a[1], name)?, f32_of(&a[2], name)?))
}

fn model_err(e: ModelError) -> BridgeError {
    BridgeError::failed(e.to_string())
}

fn channel_of(m: &Map<String, Value>) -> Result<VertexChannel, BridgeError> {
    Ok(match str_of(m, "channel")? {
        "ao" => VertexChannel::AmbientOcclusion,
        "shadow_shift" => VertexChannel::ShadowShift,
        "outline_width" => VertexChannel::OutlineWidth,
        "spec_rim" => VertexChannel::SpecRimMask,
        _ => return Err(BridgeError::invalid_params("channel deve ser ao|shadow_shift|outline_width|spec_rim")),
    })
}

fn blend_of(m: &Map<String, Value>) -> Result<PaintBlend, BridgeError> {
    Ok(match m.get("blend").and_then(Value::as_str).unwrap_or("mix") {
        "mix" => PaintBlend::Mix,
        "add" => PaintBlend::Add,
        "subtract" => PaintBlend::Subtract,
        "multiply" => PaintBlend::Multiply,
        _ => return Err(BridgeError::invalid_params("blend deve ser mix|add|subtract|multiply")),
    })
}

fn falloff_of(m: &Map<String, Value>) -> Result<BrushFalloff, BridgeError> {
    Ok(match m.get("falloff").and_then(Value::as_str).unwrap_or("smooth") {
        "smooth" => BrushFalloff::Smooth,
        "linear" => BrushFalloff::Linear,
        "sphere" => BrushFalloff::Sphere,
        "root" => BrushFalloff::Root,
        "sharp" => BrushFalloff::Sharp,
        "constant" => BrushFalloff::Constant,
        _ => return Err(BridgeError::invalid_params("falloff deve ser smooth|linear|sphere|root|sharp|constant")),
    })
}

fn joint_of(m: &Map<String, Value>, k: &str) -> Result<u16, BridgeError> {
    m.get(k).and_then(Value::as_u64).filter(|n| *n <= u16::MAX as u64).map(|n| n as u16).ok_or_else(|| BridgeError::invalid_params(format!("'{k}' deve ser inteiro 0..65535 (índice do joint)")))
}

fn id_list(v: &Value, name: &str) -> Result<Vec<u32>, BridgeError> {
    let a = v.as_array().filter(|a| a.len() <= 200_000).ok_or_else(|| BridgeError::invalid_params(format!("'{name}' deve ser uma lista de inteiros")))?;
    a.iter().map(|x| x.as_u64().filter(|n| *n < u32::MAX as u64).map(|n| n as u32).ok_or_else(|| BridgeError::invalid_params(format!("'{name}': só inteiros >= 0")))).collect()
}

/// Estatísticas de pesos/canais de uma malha (para conferir uma edição numericamente).
pub(crate) fn weight_stats(mesh: &EditMesh) -> Value {
    let mut weighted = 0usize;
    let mut max_infl = 0usize;
    let mut sum_infl = 0usize;
    let mut worst_norm: f32 = 0.0;
    let mut joints: BTreeMap<u16, usize> = BTreeMap::new();
    let mut ch = [(f32::MAX, 0.0f64, f32::MIN); 4];
    for v in &mesh.verts {
        let inf = attr::influences(v);
        if !inf.is_empty() {
            weighted += 1;
            max_infl = max_infl.max(inf.len());
            sum_infl += inf.len();
            let sum: f32 = inf.iter().map(|x| x.1).sum();
            worst_norm = worst_norm.max((sum - 1.0).abs());
            for (j, _) in inf {
                *joints.entry(j).or_default() += 1;
            }
        }
        for (i, c) in v.color.iter().enumerate() {
            ch[i].0 = ch[i].0.min(*c);
            ch[i].1 += *c as f64;
            ch[i].2 = ch[i].2.max(*c);
        }
    }
    let n = mesh.vert_count().max(1) as f64;
    let names = ["ao", "shadow_shift", "outline_width", "spec_rim"];
    json!({
        "verts": mesh.vert_count(),
        "skin": { "weighted_verts": weighted, "max_influences": max_infl, "mean_influences": if weighted > 0 { sum_infl as f64 / weighted as f64 } else { 0.0 }, "worst_normalization_error": worst_norm, "vertices_per_joint": joints },
        "npr": names.iter().enumerate().map(|(i, n_)| json!({ "channel": n_, "min": if mesh.vert_count() > 0 { ch[i].0 } else { 0.0 }, "mean": ch[i].1 / n, "max": if mesh.vert_count() > 0 { ch[i].2 } else { 0.0 } })).collect::<Vec<_>>(),
    })
}

impl StudioApp {
    pub(crate) fn bridge_weights(&mut self, params: &Value) -> BridgeResult {
        let m = params.as_object().ok_or_else(|| BridgeError::invalid_params("params deve ser um objeto"))?;
        let op = str_of(m, "op")?;
        if self.ws != WorkspaceId::Modelagem {
            self.switch_workspace(WorkspaceId::Modelagem);
        }
        if self.modeling.active_object().is_none() {
            return Err(BridgeError::failed("não há objeto ativo"));
        }
        match op {
            "info" => self.weights_info(m),
            "npr" => self.weights_npr(m),
            "skin" => self.weights_skin(m),
            other => Err(BridgeError::invalid_params(format!("WEIGHTS op '{other}' desconhecida (info, npr, skin)"))),
        }
    }

    /// Centro do pincel em espaço do objeto (`center` já local, ou `world_center`) e o raio convertido (mundo → local).
    fn weights_brush(&self, m: &Map<String, Value>) -> Result<(Vec3, f32), BridgeError> {
        let o = self.modeling.active_object().ok_or_else(|| BridgeError::failed("não há objeto ativo"))?;
        let radius = f32_of(m.get("radius").ok_or_else(|| BridgeError::invalid_params("'radius' é obrigatório (> 0)"))?, "radius")?;
        if radius <= 0.0 {
            return Err(BridgeError::invalid_params("radius deve ser > 0"));
        }
        match (m.get("center"), m.get("world_center")) {
            (Some(c), None) => Ok((vec3_of(c, "center")?, radius)),
            (None, Some(w)) => {
                let inv = o.model_matrix().inverse();
                let avg = (o.transform.scale.x + o.transform.scale.y + o.transform.scale.z) / 3.0;
                Ok((inv.transform_point3(vec3_of(w, "world_center")?), radius / avg.max(1e-4)))
            }
            _ => Err(BridgeError::invalid_params("informe 'center' (espaço do objeto) OU 'world_center' (mundo)")),
        }
    }

    /// `None` = todos; com `verts` a lista; com `use_selection` os selecionados (erro se não há seleção).
    fn weights_scope(&self, m: &Map<String, Value>) -> Result<Option<Vec<u32>>, BridgeError> {
        if m.get("use_selection").and_then(Value::as_bool).unwrap_or(false) {
            let v: Vec<u32> = self.modeling.selection.verts.iter().copied().collect();
            if v.is_empty() {
                return Err(BridgeError::failed("use_selection=true mas não há vértices selecionados (mesh_select)"));
            }
            return Ok(Some(v));
        }
        match m.get("verts") {
            None | Some(Value::Null) => Ok(None),
            Some(v) => Ok(Some(id_list(v, "verts")?)),
        }
    }

    fn weights_info(&self, m: &Map<String, Value>) -> BridgeResult {
        let o = self.modeling.active_object().ok_or_else(|| BridgeError::failed("não há objeto ativo"))?;
        let mut out = json!({ "object": o.name, "stats": weight_stats(&o.mesh) });
        if let Some(v) = m.get("vertices") {
            let ids = id_list(v, "vertices")?;
            if ids.len() > 500 {
                return Err(BridgeError::invalid_params("no máximo 500 vértices por consulta"));
            }
            let mut list = Vec::new();
            for id in ids {
                let vert = o.mesh.verts.get(id as usize).ok_or_else(|| BridgeError::invalid_params(format!("vértice {id} fora do intervalo (0..{})", o.mesh.vert_count())))?;
                list.push(json!({ "vertex": id, "position": vert.co.to_array(), "npr": vert.color, "influences": attr::influences(vert).iter().map(|(j, w)| json!({ "joint": j, "weight": w })).collect::<Vec<_>>() }));
            }
            out["vertices"] = json!(list);
        }
        Ok(out)
    }

    fn weights_result(&self, label: &str, n: usize) -> BridgeResult {
        let o = self.modeling.active_object().ok_or_else(|| BridgeError::failed("não há objeto ativo"))?;
        Ok(json!({ "op": label, "affected": n, "stats": weight_stats(&o.mesh) }))
    }

    // ------------------------------------------------------------------ canais NPR

    fn weights_npr(&mut self, m: &Map<String, Value>) -> BridgeResult {
        let action = str_of(m, "action")?;
        let (label, n) = match action {
            "paint" => {
                let (center, radius) = self.weights_brush(m)?;
                let (ch, blend, fo) = (channel_of(m)?, blend_of(m)?, falloff_of(m)?);
                let (value, strength) = (num(m, "value", 1.0)?, num(m, "strength", 1.0)?);
                let n = self.modeling.edit_mesh("Pintar atributo", |mesh, _| attr::paint_vertex_channel(mesh, center, radius, fo, ch, value, blend, strength)).map_err(model_err)?;
                ("paint", n)
            }
            "fill" => {
                let scope = self.weights_scope(m)?;
                let (ch, blend) = (channel_of(m)?, blend_of(m)?);
                let (value, strength) = (num(m, "value", 1.0)?, num(m, "strength", 1.0)?);
                let n = self.modeling.edit_mesh("Preencher atributo", |mesh, _| attr::fill_channel(mesh, scope.as_deref(), ch, value, blend, strength)).map_err(model_err)?;
                ("fill", n)
            }
            "smooth" => {
                let scope = self.weights_scope(m)?;
                let ch = channel_of(m)?;
                let iters = super::bridge_model::opt_u64_in(m, "iterations", 1, 100, 1)? as u32;
                let factor = num(m, "factor", 0.5)?;
                let n = self.modeling.edit_mesh("Suavizar atributo", |mesh, _| attr::smooth_channel(mesh, scope.as_deref(), ch, iters, factor)).map_err(model_err)?;
                ("smooth", n)
            }
            "gradient" => {
                let scope = self.weights_scope(m)?;
                let ch = channel_of(m)?;
                let p0 = vec3_of(m.get("p0").ok_or_else(|| BridgeError::invalid_params("gradient exige p0 e p1 (espaço do objeto)"))?, "p0")?;
                let p1 = vec3_of(m.get("p1").ok_or_else(|| BridgeError::invalid_params("gradient exige p0 e p1"))?, "p1")?;
                let (v0, v1) = (num(m, "v0", 0.0)?, num(m, "v1", 1.0)?);
                let n = self.modeling.edit_mesh("Gradiente de atributo", |mesh, _| attr::gradient_channel(mesh, scope.as_deref(), p0, p1, v0, v1, ch)).map_err(model_err)?;
                ("gradient", n)
            }
            "invert" => {
                let scope = self.weights_scope(m)?;
                let ch = channel_of(m)?;
                let n = self.modeling.edit_mesh("Inverter atributo", |mesh, _| attr::invert_channel(mesh, scope.as_deref(), ch)).map_err(model_err)?;
                ("invert", n)
            }
            "bake_ao" => {
                let mut p = attr::AoParams::default();
                if let Some(v) = m.get("samples") {
                    p.samples = v.as_u64().filter(|n| (1..=512).contains(n)).ok_or_else(|| BridgeError::invalid_params("samples: 1..512"))? as u32;
                }
                p.max_dist = num(m, "max_dist", p.max_dist)?;
                p.bias = num(m, "bias", p.bias)?;
                p.strength = num(m, "strength", p.strength)?.clamp(0.0, 1.0);
                if let Some(v) = m.get("seed") {
                    p.seed = v.as_u64().ok_or_else(|| BridgeError::invalid_params("seed: inteiro >= 0"))?;
                }
                let n = self.modeling.edit_mesh("Assar AO", |mesh, _| attr::bake_ao_per_vertex(mesh, &p)).map_err(model_err)?;
                ("bake_ao", n)
            }
            other => return Err(BridgeError::invalid_params(format!("npr.action '{other}' desconhecida (paint, fill, smooth, gradient, invert, bake_ao)"))),
        };
        self.weights_result(label, n)
    }

    // ------------------------------------------------------------------ pesos de skin

    fn weights_skin(&mut self, m: &Map<String, Value>) -> BridgeResult {
        let action = str_of(m, "action")?;
        let normalize = m.get("normalize").and_then(Value::as_bool).unwrap_or(true);
        let (label, n) = match action {
            "assign" => {
                let joint = joint_of(m, "joint")?;
                let weight = num(m, "weight", 1.0)?;
                let ids = match (m.get("vertex"), m.get("vertices")) {
                    (Some(v), None) => vec![v.as_u64().filter(|n| *n < u32::MAX as u64).ok_or_else(|| BridgeError::invalid_params("vertex: inteiro >= 0"))? as u32],
                    (None, Some(v)) => id_list(v, "vertices")?,
                    _ => match self.weights_scope(m)? {
                        Some(v) => v,
                        None => return Err(BridgeError::invalid_params("informe vertex, vertices ou use_selection")),
                    },
                };
                self.modeling
                    .edit_mesh("Atribuir peso", |mesh, _| {
                        for v in &ids {
                            attr::assign_weight(mesh, *v, joint, weight, normalize)?;
                        }
                        Ok(ids.len())
                    })
                    .map_err(model_err)
                    .map(|n| ("assign", n))?
            }
            "paint" => {
                let (center, radius) = self.weights_brush(m)?;
                let (blend, fo) = (blend_of(m)?, falloff_of(m)?);
                let joint = joint_of(m, "joint")?;
                let (value, strength) = (num(m, "value", 1.0)?, num(m, "strength", 1.0)?);
                let n = self.modeling.edit_mesh("Pintar peso", |mesh, _| attr::paint_weight(mesh, center, radius, fo, joint, value, blend, strength, normalize)).map_err(model_err)?;
                ("paint", n)
            }
            "normalize" => {
                self.modeling
                    .edit_mesh("Normalizar pesos", |mesh, _| {
                        attr::normalize_all(mesh);
                        Ok(mesh.vert_count())
                    })
                    .map_err(model_err)
                    .map(|n| ("normalize", n))?
            }
            "limit" => {
                let max = m.get("max").and_then(Value::as_u64).filter(|n| (1..=4).contains(n)).ok_or_else(|| BridgeError::invalid_params("limit exige 'max' (1..4)"))? as usize;
                let n = self.modeling.edit_mesh("Limitar influências", |mesh, _| attr::limit_influences(mesh, max)).map_err(model_err)?;
                ("limit", n)
            }
            "smooth" => {
                let scope = self.weights_scope(m)?;
                let iters = super::bridge_model::opt_u64_in(m, "iterations", 1, 100, 1)? as u32;
                let factor = num(m, "factor", 0.5)?;
                let n = self
                    .modeling
                    .edit_mesh("Suavizar pesos", |mesh, _| {
                        let ids: Vec<u32> = scope.clone().unwrap_or_else(|| (0..mesh.vert_count() as u32).collect());
                        attr::smooth_weights(mesh, &ids, iters, factor)
                    })
                    .map_err(model_err)?;
                ("smooth", n)
            }
            "transfer" => {
                let src_idx = m.get("from_object").and_then(Value::as_u64).ok_or_else(|| BridgeError::invalid_params("transfer exige 'from_object' (índice do objeto de origem)"))? as usize;
                let dst_idx = self.modeling.doc.active.ok_or_else(|| BridgeError::failed("não há objeto ativo"))?;
                if src_idx == dst_idx {
                    return Err(BridgeError::invalid_params("a origem deve ser OUTRO objeto (o ativo é o destino)"));
                }
                let src_obj = self.modeling.doc.objects.get(src_idx).ok_or_else(|| BridgeError::invalid_params(format!("objeto {src_idx} não existe")))?;
                // origem levada ao espaço do destino (os dois podem ter transforms diferentes)
                let to_dst = self.modeling.doc.objects[dst_idx].model_matrix().inverse() * src_obj.model_matrix();
                let mut src = src_obj.mesh.clone();
                src.transform_positions(|p| to_dst.transform_point3(p));
                let max_d = m.get("max_distance").map(|v| f32_of(v, "max_distance")).transpose()?;
                let n = self.modeling.edit_mesh("Transferir pesos", |mesh, _| attr::transfer_weights(mesh, &src, max_d)).map_err(model_err)?;
                ("transfer", n)
            }
            "auto_envelope" => {
                let list = m.get("bones").and_then(Value::as_array).filter(|a| !a.is_empty() && a.len() <= 256).ok_or_else(|| BridgeError::invalid_params("auto_envelope exige 'bones': [{joint,head,tail,radius_head,radius_tail}] (1..256)"))?;
                let world = m.get("space").and_then(Value::as_str) == Some("world");
                let inv = self.modeling.active_object().map(|o| o.model_matrix().inverse()).unwrap_or_default();
                let avg = self.modeling.active_object().map(|o| (o.transform.scale.x + o.transform.scale.y + o.transform.scale.z) / 3.0).unwrap_or(1.0).max(1e-4);
                let mut bones = Vec::new();
                for b in list {
                    let b = b.as_object().ok_or_else(|| BridgeError::invalid_params("cada osso deve ser um objeto"))?;
                    let (mut head, mut tail) = (vec3_of(b.get("head").ok_or_else(|| BridgeError::invalid_params("osso sem 'head'"))?, "head")?, vec3_of(b.get("tail").ok_or_else(|| BridgeError::invalid_params("osso sem 'tail'"))?, "tail")?);
                    let (mut rh, mut rt) = (num(b, "radius_head", 0.1)?, num(b, "radius_tail", 0.1)?);
                    if world {
                        head = inv.transform_point3(head);
                        tail = inv.transform_point3(tail);
                        rh /= avg;
                        rt /= avg;
                    }
                    bones.push(BoneEnvelope { joint: joint_of(b, "joint")?, head, tail, radius_head: rh, radius_tail: rt });
                }
                let falloff = num(m, "falloff", 0.5)?;
                let n = self.modeling.edit_mesh("Auto-peso por envelope", |mesh, _| attr::auto_weights_envelope(mesh, &bones, falloff)).map_err(model_err)?;
                ("auto_envelope", n)
            }
            "mirror" => {
                let axis = match m.get("axis").and_then(Value::as_str).unwrap_or("x") {
                    "x" => MirrorAxis::X,
                    "y" => MirrorAxis::Y,
                    "z" => MirrorAxis::Z,
                    _ => return Err(BridgeError::invalid_params("axis deve ser x|y|z")),
                };
                let from_positive = m.get("from").and_then(Value::as_str).unwrap_or("positive") == "positive";
                let tol = num(m, "tolerance", 1.0e-3)?;
                let mut map: Vec<(u16, u16)> = Vec::new();
                if let Some(v) = m.get("joint_map") {
                    for p in v.as_array().filter(|a| a.len() <= 512).ok_or_else(|| BridgeError::invalid_params("joint_map: [[origem,destino],…]"))? {
                        let a = p.as_array().filter(|a| a.len() == 2).ok_or_else(|| BridgeError::invalid_params("joint_map: pares [origem,destino]"))?;
                        let g = |x: &Value| x.as_u64().filter(|n| *n <= u16::MAX as u64).map(|n| n as u16).ok_or_else(|| BridgeError::invalid_params("joint_map: inteiros 0..65535"));
                        map.push((g(&a[0])?, g(&a[1])?));
                    }
                }
                let n = self.modeling.edit_mesh("Espelhar pesos", |mesh, _| attr::mirror_weights(mesh, axis, from_positive, &map, tol)).map_err(model_err)?;
                ("mirror", n)
            }
            other => return Err(BridgeError::invalid_params(format!("skin.action '{other}' desconhecida (assign, paint, normalize, limit, smooth, transfer, auto_envelope, mirror)"))),
        };
        self.weights_result(label, n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_modeling::primitives;

    #[test]
    fn stats_report_channels_and_skin_of_a_fresh_mesh() {
        let mesh = anigo_modeling::primitives::cube(1.0);
        let s = weight_stats(&mesh);
        assert_eq!(s["verts"], 8);
        assert_eq!(s["npr"].as_array().unwrap().len(), 4);
        assert!(s["skin"]["worst_normalization_error"].as_f64().unwrap() < 1e-5);
        let _ = primitives::cube;
    }

    #[test]
    fn enum_parsers_reject_unknown_names() {
        let m = json!({ "channel": "bogus" });
        assert!(channel_of(m.as_object().unwrap()).is_err());
        assert!(blend_of(json!({ "blend": "nope" }).as_object().unwrap()).is_err());
        assert!(falloff_of(json!({ "falloff": "nope" }).as_object().unwrap()).is_err());
        assert!(matches!(channel_of(json!({ "channel": "spec_rim" }).as_object().unwrap()), Ok(VertexChannel::SpecRimMask)));
    }
}
