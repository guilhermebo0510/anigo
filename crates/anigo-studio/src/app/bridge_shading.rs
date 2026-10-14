//! Materiais pela bridge (`SHADING`): o editor do workspace Shading sem janela — escolher a fonte (Personagem, Cenário ou Modelagem)
//! e o alvo, ler o material EFETIVO (JSON do `StylizedMaterial`), editar por patch JSON parcial, aplicar presets embutidos, copiar/colar,
//! resetar e desfazer/refazer. Cada edição é UM passo de histórico (o do núcleo para Personagem/Cenário; o da Modelagem para objetos).
//!
//! Usa as mesmas funções do painel (`shading::edit::apply`), então a validação do núcleo vale (valores inválidos são recusados e nada muda).

use super::shading_ui::Source;
use super::StudioApp;
use crate::shading::edit;
use crate::shading::presets::BUILT_IN;
use anigo_bridge::protocol::{BridgeError, BridgeResult};
use anigo_core::StylizedMaterial;
use serde_json::{json, Map, Value};

fn str_of<'a>(m: &'a Map<String, Value>, k: &str) -> Result<&'a str, BridgeError> {
    m.get(k).and_then(Value::as_str).ok_or_else(|| BridgeError::invalid_params(format!("'{k}' é obrigatório e deve ser texto")))
}

fn source_key(s: Source) -> &'static str {
    match s {
        Source::Personagem => "personagem",
        Source::Cenario => "cenario",
        Source::Modelagem => "modelagem",
    }
}

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

/// O serde OMITE `npr` (e `npr.ramp_custom`) quando estão no padrão: sem recolocá-los, um patch como `{ "npr": { "ramp_blend": 1 } }`
/// seria recusado como "campo desconhecido" em qualquer material que ainda não os usa.
fn fill_omitted(v: &mut Value) {
    use anigo_core::npr::{NprParams, RampCustom};
    let Some(m) = v.as_object_mut() else { return };
    let npr = m.entry("npr").or_insert_with(|| serde_json::to_value(NprParams::default()).unwrap_or(Value::Null));
    if let Some(n) = npr.as_object_mut() {
        n.entry("ramp_custom").or_insert_with(|| serde_json::to_value(RampCustom::DEFAULT).unwrap_or(Value::Null));
    }
}

/// `base` + patch JSON parcial → material. Chaves desconhecidas são erro (o serde recusa campos que não existem).
pub(crate) fn patch_material(base: &StylizedMaterial, patch: &Value) -> Result<StylizedMaterial, BridgeError> {
    if !patch.is_object() {
        return Err(BridgeError::invalid_params("'patch' deve ser um objeto"));
    }
    let mut v = serde_json::to_value(base).map_err(|e| BridgeError::failed(e.to_string()))?;
    fill_omitted(&mut v);
    super::bridge_model::reject_unknown_keys(&v, patch, "patch")?;
    merge(&mut v, patch);
    serde_json::from_value(v).map_err(|e| BridgeError::invalid_params(format!("material inválido: {e}")))
}

impl StudioApp {
    pub(crate) fn bridge_shading(&mut self, params: &Value) -> BridgeResult {
        let m = params.as_object().ok_or_else(|| BridgeError::invalid_params("params deve ser um objeto"))?;
        let op = str_of(m, "op")?;
        self.shading_validate_target();
        match op {
            "info" => Ok(self.shading_info(m)),
            "source" => {
                let s = match str_of(m, "value")? {
                    "personagem" => Source::Personagem,
                    "cenario" => Source::Cenario,
                    "modelagem" => Source::Modelagem,
                    _ => return Err(BridgeError::invalid_params("value deve ser personagem|cenario|modelagem")),
                };
                self.shading_set_source(s);
                Ok(self.shading_info(&Map::new()))
            }
            "target" => {
                let idx = m.get("index").and_then(Value::as_u64).ok_or_else(|| BridgeError::invalid_params("'index' é obrigatório (posição em info.targets)"))? as usize;
                let rows = self.shading_targets(self.shading_ui.source);
                let row = rows.into_iter().nth(idx).ok_or_else(|| BridgeError::invalid_params(format!("alvo {idx} não existe na fonte '{}'", source_key(self.shading_ui.source))))?;
                self.shading_select(row.target);
                Ok(self.shading_info(&Map::new()))
            }
            "set" => {
                let patch = m.get("patch").ok_or_else(|| BridgeError::invalid_params("'patch' é obrigatório (campos do material, ver info.material)"))?;
                let (t, cur) = self.shading_target_material()?;
                let new = patch_material(&cur, patch)?;
                self.shading_apply_strict(&t, new)?;
                Ok(self.shading_info(&Map::new()))
            }
            "preset" => {
                let id = str_of(m, "id")?;
                let p = BUILT_IN.iter().find(|p| p.id == id).ok_or_else(|| BridgeError::invalid_params(format!("preset '{id}' desconhecido; disponíveis: {}", BUILT_IN.iter().map(|p| p.id).collect::<Vec<_>>().join(", "))))?;
                let (t, cur) = self.shading_target_material()?;
                let mut nm = (p.build)();
                nm.name = cur.name;
                self.shading_apply_strict(&t, nm)?;
                Ok(self.shading_info(&Map::new()))
            }
            "reset" => {
                let (t, cur) = self.shading_target_material()?;
                // volta ao padrão preservando o nome e o solver de olhar (que o painel também preserva)
                let mut nm = StylizedMaterial { name: cur.name.clone(), ..Default::default() };
                nm.gaze_tracking_enabled = cur.gaze_tracking_enabled;
                nm.gaze_saccade_amplitude = cur.gaze_saccade_amplitude;
                nm.gaze_damping = cur.gaze_damping;
                self.shading_apply_strict(&t, nm)?;
                Ok(self.shading_info(&Map::new()))
            }
            "copy" => {
                self.shading_copy();
                Ok(self.shading_info(&Map::new()))
            }
            "paste" => {
                if self.shading_ui.clipboard.is_none() {
                    return Err(BridgeError::failed("nada copiado (use op=copy antes)"));
                }
                self.shading_paste();
                Ok(self.shading_info(&Map::new()))
            }
            "history" => {
                let undo = match str_of(m, "action")? {
                    "undo" => true,
                    "redo" => false,
                    _ => return Err(BridgeError::invalid_params("history.action deve ser undo|redo")),
                };
                let (can_u, can_r) = self.shading_can_undo_redo();
                if (undo && !can_u) || (!undo && !can_r) {
                    return Err(BridgeError::failed(if undo { "nada para desfazer" } else { "nada para refazer" }));
                }
                let msg = self.shading_undo_redo(undo);
                let mut out = self.shading_info(&Map::new());
                out["message"] = json!(msg);
                Ok(out)
            }
            "view" | "param" | "palette" | "ramp" | "compare" => self.shading_ext_op(op, m),
            other => Err(BridgeError::invalid_params(format!("SHADING op '{other}' desconhecida (info, source, target, set, preset, reset, copy, paste, history, view, param, palette, ramp, compare, presets, texture)"))),
        }
    }

    pub(super) fn shading_target_material(&self) -> Result<(edit::Target, StylizedMaterial), BridgeError> {
        let t = self.shading_ui.target.clone().ok_or_else(|| BridgeError::failed("não há alvo de material (a fonte atual está vazia; veja info.targets e op=source)"))?;
        let cur = self.shading_read(&t).ok_or_else(|| BridgeError::failed("o alvo não existe mais"))?;
        Ok((t, cur))
    }

    /// Como `shading_apply_now`, mas devolve o erro (o painel só o mostra na barra de mensagem).
    pub(super) fn shading_apply_strict(&mut self, t: &edit::Target, new: StylizedMaterial) -> Result<(), BridgeError> {
        edit::apply(&mut self.core, &mut self.modeling, &mut self.shading_ui.gesture, t, new).map(|_| ()).map_err(BridgeError::failed)
    }

    pub(super) fn shading_info_pub(&self) -> Value {
        self.shading_info(&Map::new())
    }

    fn shading_info(&self, m: &Map<String, Value>) -> Value {
        let src = self.shading_ui.source;
        let rows = self.shading_targets(src);
        let cur_idx = self.shading_ui.target.as_ref().and_then(|t| rows.iter().position(|r| &r.target == t));
        let (can_undo, can_redo) = self.shading_can_undo_redo();
        let mut out = json!({
            "source": source_key(src),
            "targets": rows.iter().enumerate().map(|(i, r)| json!({ "index": i, "label": r.label, "depth": r.depth, "base_color": r.swatch.0, "shade_color": r.swatch.1, "current": Some(i) == cur_idx })).collect::<Vec<_>>(),
            "current": cur_idx,
            "material": self.shading_ui.target.as_ref().and_then(|t| self.shading_read(t)).and_then(|mat| serde_json::to_value(mat).ok()),
            "presets": BUILT_IN.iter().map(|p| p.id).collect::<Vec<_>>(),
            "can_undo": can_undo, "can_redo": can_redo, "has_clipboard": self.shading_ui.clipboard.is_some(),
            "message": self.shading_ui.message,
            "ui": self.shading_ui_json(),
        });
        if m.get("fields_only").and_then(Value::as_bool).unwrap_or(false) {
            out["material"] = out["material"].as_object().map(|o| json!(o.keys().collect::<Vec<_>>())).unwrap_or(Value::Null);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_patch_merges_nested_fields_and_rejects_unknown_ones() {
        let base = StylizedMaterial::default();
        let p = patch_material(&base, &json!({ "shadow_threshold": 0.7, "base_color": [1.0, 0.0, 0.0, 1.0], "name": "X" })).unwrap();
        assert_eq!(p.shadow_threshold, 0.7);
        assert_eq!(p.base_color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(p.name, "X");
        assert_eq!(p.shade_color, base.shade_color, "o que não veio fica");
        assert!(patch_material(&base, &json!({ "campo_que_nao_existe": 1 })).is_err());
        assert!(patch_material(&base, &json!({ "base_color": "vermelho" })).is_err());
        assert!(patch_material(&base, &json!(3)).is_err());
    }

    #[test]
    fn sources_have_stable_keys() {
        assert_eq!(source_key(Source::Personagem), "personagem");
        assert_eq!(source_key(Source::Cenario), "cenario");
        assert_eq!(source_key(Source::Modelagem), "modelagem");
    }

    #[test]
    fn patches_may_touch_nested_fields_the_serializer_omits_when_default() {
        let base = StylizedMaterial::default();
        let p = patch_material(&base, &json!({ "npr": { "ramp_blend": 0.7, "ramp_custom": { "constant": [true, false, false, false] } } })).unwrap();
        assert_eq!(p.npr.ramp_blend, 0.7);
        assert!(p.npr.ramp_custom.constant[0]);
        assert!(patch_material(&base, &json!({ "npr": { "campo_que_nao_existe": 1 } })).is_err());
    }
}
