//! Personagem pela bridge (`CHAR`): o workspace sem janela — os 157 sliders de morph (corpo e rosto), proporções macro, gênero e
//! dimorfismo, somatótipo, presets, "resetar tudo" e a câmera do preview. Usa os MESMOS construtores de comando da UI
//! (`character::{sliders, macros, presets}`) e `character::compose` (descarta o que já está no valor pedido), então cada ação é
//! UM passo de desfazer do núcleo (`anigo_native_undo`). Tudo é validado antes de mexer.

use super::StudioApp;
use crate::character::sliders::{self, Region};
use crate::character::{self, macros, presets};
use anigo_bridge::protocol::{BridgeError, BridgeResult};
use anigo_core::command::Command;
use anigo_core::mesh::BaseGender;
use anigo_core::morph_catalog::{find_slider_def, ALL_MORPH_SLIDERS};
use anigo_core::somatotype::SomatotypeCoords;
use serde_json::{json, Map, Value};

fn bad(msg: impl Into<String>) -> BridgeError {
    BridgeError::invalid_params(msg)
}

fn str_of<'a>(m: &'a Map<String, Value>, k: &str) -> Result<&'a str, BridgeError> {
    m.get(k).and_then(Value::as_str).ok_or_else(|| bad(format!("'{k}' é obrigatório e deve ser texto")))
}

fn num(v: &Value, name: &str) -> Result<f32, BridgeError> {
    v.as_f64().filter(|f| f.is_finite()).map(|f| f as f32).ok_or_else(|| bad(format!("'{name}' deve ser um número finito")))
}

fn region_of(s: &str) -> Result<Region, BridgeError> {
    Region::from_tool(s).ok_or_else(|| bad("region deve ser body|face"))
}

impl StudioApp {
    pub(crate) fn bridge_char(&mut self, params: &Value) -> BridgeResult {
        let m = params.as_object().ok_or_else(|| bad("params deve ser um objeto"))?;
        let op = str_of(m, "op")?;
        if op == "info" {
            return Ok(self.char_info());
        }
        self.bridge_reset_gestures();
        if self.ws != crate::workspace::WorkspaceId::Personagem {
            self.switch_workspace(crate::workspace::WorkspaceId::Personagem);
        }
        let depth = self.core.history().undo_depth();
        let before = self.core.project().character.clone();
        match op {
            "sliders" => return self.char_sliders_op(m, depth),
            "proportions" => self.char_proportions_op(m)?,
            "gender" => self.char_gender_op(m)?,
            "somatotype" => self.char_somatotype_op(m)?,
            "preset" => {
                let id = str_of(m, "id")?;
                let p = presets::find(id).ok_or_else(|| bad(format!("preset '{id}' desconhecido; disponíveis: {}", presets::PRESETS.iter().map(|p| p.id).collect::<Vec<_>>().join(", "))))?;
                let r = presets::apply_preset(self.core.project(), p);
                self.char_run(r)?;
            }
            "reset_all" => {
                let r = macros::reset_all(self.core.project());
                self.char_run(r)?;
            }
            "view" => self.char_view_op(m)?,
            other => return Err(bad(format!("CHAR op '{other}' desconhecida (info, sliders, proportions, gender, somatotype, preset, reset_all, view)"))),
        }
        let mut out = self.char_info();
        out["changed"] = json!(self.core.project().character != before);
        out["history_steps_added"] = json!(self.core.history().undo_depth().saturating_sub(depth));
        Ok(out)
    }

    /// Aplica o resultado de um construtor (`Ok(None)` = já estava assim; erro de verdade vira erro).
    fn char_run(&mut self, r: Result<Option<Command>, anigo_core::command::CommandError>) -> Result<(), BridgeError> {
        match r {
            Ok(Some(cmd)) => match self.core.apply(cmd) {
                Ok(o) => {
                    self.char_ui.message = super::character_ui::describe(&o.description);
                    Ok(())
                }
                Err(e) if e.contains("no effect") || e.contains("already") => Ok(()),
                Err(e) => Err(BridgeError::failed(e)),
            },
            Ok(None) => Ok(()),
            Err(e) => Err(BridgeError::failed(e.to_string())),
        }
    }

    fn char_info(&self) -> Value {
        let p = self.core.project();
        let c = &p.character;
        let pr = &c.proportions;
        let modified: Vec<Value> = ALL_MORPH_SLIDERS
            .iter()
            .filter(|d| sliders::is_modified(p, d))
            .map(|d| json!({ "id": d.id, "value": sliders::slider_value(p, d), "default": d.default_value, "min": d.min, "max": d.max }))
            .collect();
        let (can_undo, can_redo) = (self.core.history().undo_depth() > 0, self.core.history().redo_depth() > 0);
        json!({
            "workspace": self.ws.key(),
            "character_id": c.character_id.to_string(),
            "base_gender": match c.base_gender { BaseGender::Male => "male", BaseGender::Female => "female" },
            "gender_dimorphism": c.gender_dimorphism,
            "somatotype": { "endomorph": c.somatotype.endomorph, "mesomorph": c.somatotype.mesomorph, "ectomorph": c.somatotype.ectomorph },
            "proportions": {
                "height": pr.height_overall, "head_scale": pr.head_scale, "head_ratio": pr.head_ratio, "shoulders": pr.shoulder_width,
                "legs": pr.leg_length, "arms": pr.arm_length, "neck": pr.neck_length, "torso": pr.torso_length,
                "ranges": macros::Proportion::ALL.iter().map(|q| { let (lo, hi) = q.range(); json!({ "name": prop_key(*q), "min": lo, "max": hi, "default": q.default_value() }) }).collect::<Vec<_>>(),
            },
            "active_preset": c.active_preset_id,
            "presets": presets::PRESETS.iter().map(|p| p.id).collect::<Vec<_>>(),
            "slider_count": ALL_MORPH_SLIDERS.len(), "modified_sliders": modified,
            "somatotype_shortcuts": macros::SOMATOTYPE_SHORTCUTS.iter().map(|(k, e, m, c)| json!({ "key": k, "endomorph": e, "mesomorph": m, "ectomorph": c })).collect::<Vec<_>>(),
            "view": { "yaw": self.char_ui.camera.yaw, "pitch": self.char_ui.camera.pitch, "distance": self.char_ui.camera.distance, "turntable": self.char_ui.turntable, "search": self.char_ui.search },
            "history": { "can_undo": can_undo, "can_redo": can_redo, "undo_depth": self.core.history().undo_depth() },
            "message": self.char_ui.message,
        })
    }

    // ------------------------------------------------------------------------------------------------ sliders

    fn char_sliders_op(&mut self, m: &Map<String, Value>, depth: usize) -> BridgeResult {
        let action = str_of(m, "action")?;
        match action {
            "list" => {
                let region = region_of(m.get("region").and_then(Value::as_str).unwrap_or("body"))?;
                let query = m.get("query").and_then(Value::as_str).unwrap_or("");
                if query.len() > 64 {
                    return Err(bad("'query' longa demais (máx. 64)"));
                }
                let only_mod = m.get("only_modified").and_then(Value::as_bool).unwrap_or(false);
                let p = self.core.project();
                let groups: Vec<Value> = sliders::visible_groups(region, query)
                    .into_iter()
                    .map(|(zone, defs)| {
                        json!({ "zone": format!("{zone:?}"), "sliders": defs.into_iter().filter(|d| !only_mod || sliders::is_modified(p, d)).map(|d| json!({
                            "id": d.id, "name": d.name, "value": sliders::slider_value(p, d), "default": d.default_value, "min": d.min, "max": d.max, "modified": sliders::is_modified(p, d), "macro_driven": sliders::is_macro_driven(d),
                        })).collect::<Vec<_>>() })
                    })
                    .collect();
                return Ok(json!({ "region": m.get("region").and_then(Value::as_str).unwrap_or("body"), "groups": groups }));
            }
            "set" => {
                let vals = m.get("values").and_then(Value::as_object).filter(|o| !o.is_empty() && o.len() <= 200).ok_or_else(|| bad("'values' deve ser {id: valor} com 1..200 sliders (veja sliders action=list)"))?;
                let mut cmds = Vec::new();
                for (id, v) in vals {
                    let d = find_slider_def(id).ok_or_else(|| bad(format!("slider '{id}' desconhecido (veja sliders action=list)")))?;
                    let x = num(v, id)?;
                    if x < d.min || x > d.max {
                        return Err(bad(format!("slider '{id}' fora de {}..{}: {x}", d.min, d.max)));
                    }
                    cmds.push(sliders::slider_command(d, x));
                }
                let r = character::compose(self.core.project(), cmds);
                self.char_run(r)?;
            }
            "reset" => {
                let mut defs = Vec::new();
                if m.get("all").and_then(Value::as_bool).unwrap_or(false) {
                    defs.extend(ALL_MORPH_SLIDERS.iter());
                } else if let Some(ids) = m.get("ids").and_then(Value::as_array) {
                    for id in ids {
                        let id = id.as_str().ok_or_else(|| bad("'ids' deve conter textos"))?;
                        defs.push(find_slider_def(id).ok_or_else(|| bad(format!("slider '{id}' desconhecido")))?);
                    }
                } else {
                    return Err(bad("informe 'ids' ou all=true"));
                }
                let cmds = defs.into_iter().map(sliders::slider_reset_command).collect();
                let r = character::compose(self.core.project(), cmds);
                self.char_run(r)?;
            }
            o => return Err(bad(format!("sliders.action '{o}' desconhecida (list, set, reset)"))),
        }
        let mut out = self.char_info();
        let steps = self.core.history().undo_depth().saturating_sub(depth);
        out["history_steps_added"] = json!(steps);
        out["changed"] = json!(steps > 0);
        Ok(out)
    }

    fn char_proportions_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        use macros::Proportion as P;
        let patch = m.get("patch").and_then(Value::as_object).filter(|o| !o.is_empty()).ok_or_else(|| bad("'patch' é obrigatório (height, head_scale, head_ratio, shoulders, legs, arms, neck, torso)"))?;
        let mut cmds = Vec::new();
        for (k, v) in patch {
            let p = P::ALL.into_iter().find(|p| prop_key(*p) == k).ok_or_else(|| bad(format!("proporção '{k}' desconhecida (height, head_scale, head_ratio, shoulders, legs, arms, neck, torso)")))?;
            let x = num(v, k)?;
            let (lo, hi) = p.range();
            if x < lo || x > hi {
                return Err(bad(format!("'{k}' fora de {lo}..{hi}: {x}")));
            }
            cmds.push(p.command(x));
        }
        let r = character::compose(self.core.project(), cmds);
        self.char_run(r)
    }

    fn char_gender_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        match (m.get("gender").and_then(Value::as_str), m.get("dimorphism")) {
            (Some(g), None) => {
                let g = match g {
                    "male" => BaseGender::Male,
                    "female" => BaseGender::Female,
                    _ => return Err(bad("'gender' deve ser male|female")),
                };
                let r = macros::set_gender(self.core.project(), g);
                self.char_run(r)
            }
            (None, Some(d)) => {
                let d = num(d, "dimorphism")?;
                if !(0.0..=1.0).contains(&d) {
                    return Err(bad("'dimorphism' deve estar em 0..1 (0 feminino, 1 masculino)"));
                }
                let r = character::compose(self.core.project(), vec![macros::dimorphism_final(d)]);
                self.char_run(r)
            }
            _ => Err(bad("informe 'gender' (male|female) OU 'dimorphism' (0..1)")),
        }
    }

    fn char_somatotype_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let (e, me, ec) = if let Some(k) = m.get("shortcut").and_then(Value::as_str) {
            let s = macros::SOMATOTYPE_SHORTCUTS.iter().find(|s| s.0 == k).ok_or_else(|| bad(format!("atalho '{k}' desconhecido; disponíveis: {}", macros::SOMATOTYPE_SHORTCUTS.iter().map(|s| s.0).collect::<Vec<_>>().join(", "))))?;
            (s.1, s.2, s.3)
        } else {
            let g = |k: &str| -> Result<f32, BridgeError> {
                let v = num(m.get(k).ok_or_else(|| bad(format!("'{k}' é obrigatório (ou use 'shortcut')")))?, k)?;
                if !(0.0..=1.0).contains(&v) {
                    return Err(bad(format!("'{k}' deve estar em 0..1")));
                }
                Ok(v)
            };
            (g("endomorph")?, g("mesomorph")?, g("ectomorph")?)
        };
        if e + me + ec <= 1e-6 {
            return Err(bad("o somatótipo não pode ter os três componentes em 0"));
        }
        let r = character::compose(self.core.project(), vec![macros::somatotype_command(SomatotypeCoords::new(e, me, ec))]);
        self.char_run(r)
    }

    fn char_view_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let patch = m.get("patch").and_then(Value::as_object).ok_or_else(|| bad("'patch' é obrigatório (preset front|side|back|face, yaw, pitch, turntable, search, frame)"))?;
        for (k, v) in patch {
            match k.as_str() {
                "preset" => {
                    let (yaw, pitch) = match v.as_str() {
                        Some("front") => (0.0, 0.05),
                        Some("side") => (std::f32::consts::FRAC_PI_2, 0.05),
                        Some("back") => (std::f32::consts::PI, 0.05),
                        Some("face") => (0.0, 0.0),
                        _ => return Err(bad("'preset' deve ser front|side|back|face")),
                    };
                    self.char_ui.camera.yaw = yaw;
                    self.char_ui.camera.pitch = pitch;
                    self.char_ui.framed = false;
                }
                "yaw" => self.char_ui.camera.yaw = num(v, k)?.clamp(-100.0, 100.0),
                "pitch" => self.char_ui.camera.pitch = num(v, k)?.clamp(-1.5, 1.5),
                "turntable" => self.char_ui.turntable = v.as_bool().ok_or_else(|| bad("'turntable' deve ser booleano"))?,
                "frame" => {
                    if v.as_bool().ok_or_else(|| bad("'frame' deve ser booleano"))? {
                        self.char_ui.framed = false;
                    }
                }
                "search" => {
                    let s = v.as_str().filter(|s| s.len() <= 64).ok_or_else(|| bad("'search' deve ser texto (até 64)"))?;
                    self.char_ui.search = s.to_string();
                }
                other => return Err(bad(format!("campo desconhecido 'patch.{other}' (preset, yaw, pitch, turntable, search, frame)"))),
            }
        }
        Ok(())
    }
}

fn prop_key(p: macros::Proportion) -> &'static str {
    use macros::Proportion as P;
    match p {
        P::Height => "height",
        P::HeadScale => "head_scale",
        P::HeadRatio => "head_ratio",
        P::Shoulders => "shoulders",
        P::Legs => "legs",
        P::Arms => "arms",
        P::Neck => "neck",
        P::Torso => "torso",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proportion_keys_are_unique_and_cover_all() {
        let mut keys: Vec<_> = macros::Proportion::ALL.iter().map(|p| prop_key(*p)).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), macros::Proportion::ALL.len());
    }

    #[test]
    fn regions_and_numbers_are_validated() {
        assert!(region_of("body").is_ok() && region_of("face").is_ok());
        assert!(region_of("hair").is_err());
        assert!(num(&json!("a"), "x").is_err());
        assert!(find_slider_def("height_overall").is_some());
    }
}
