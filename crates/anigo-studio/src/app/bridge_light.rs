//! Iluminação pela bridge (`LIGHT`): o workspace sem janela — sol (azimute/elevação, cor, temperatura, intensidade), sombra do
//! terminador e tinta, sombra projetada, ambiente hemisférico, rim do personagem, rigs prontos, luz presa à câmera e o gizmo.
//!
//! Usa os MESMOS comandos do painel (`SetLight`, `SetMaterialParams`, `rig_command`): um `op=set` com luz E material vira um
//! `Batch` (UM passo de desfazer do núcleo, `anigo_native_undo`). Valores são validados antes (finitos, faixas da UI, campos
//! desconhecidos são erro) e o núcleo valida de novo (`LightShadow::validate`).

use super::lighting_ui::LightPatch;
use super::StudioApp;
use crate::lighting::{self, rigs::RIGS};
use anigo_bridge::protocol::{BridgeError, BridgeResult};
use anigo_core::command::{Command, MaterialPatch};
use anigo_core::scene::LightShadow;
use glam::Vec3;
use serde_json::{json, Map, Value};

fn bad(msg: impl Into<String>) -> BridgeError {
    BridgeError::invalid_params(msg)
}

fn str_of<'a>(m: &'a Map<String, Value>, k: &str) -> Result<&'a str, BridgeError> {
    m.get(k).and_then(Value::as_str).ok_or_else(|| bad(format!("'{k}' é obrigatório e deve ser texto")))
}

fn num_in(v: &Value, name: &str, lo: f32, hi: f32) -> Result<f32, BridgeError> {
    let f = v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| bad(format!("'{name}' deve ser um número finito")))? as f32;
    if f < lo || f > hi {
        return Err(bad(format!("'{name}' fora de {lo}..{hi}: {f}")));
    }
    Ok(f)
}

fn rgb(v: &Value, name: &str) -> Result<[f32; 3], BridgeError> {
    let a = v.as_array().filter(|a| a.len() == 3).ok_or_else(|| bad(format!("'{name}' deve ter 3 números (RGB 0..1)")))?;
    Ok([num_in(&a[0], name, 0.0, 1.0)?, num_in(&a[1], name, 0.0, 1.0)?, num_in(&a[2], name, 0.0, 1.0)?])
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

impl StudioApp {
    pub(crate) fn bridge_light(&mut self, params: &Value) -> BridgeResult {
        let m = params.as_object().ok_or_else(|| bad("params deve ser um objeto"))?;
        let op = str_of(m, "op")?;
        if op == "info" {
            return Ok(self.light_info());
        }
        self.bridge_reset_gestures();
        let view0 = self.light_view();
        let depth0 = self.core.history().undo_depth();
        self.light_ui.message.clear();
        match op {
            "set" => self.light_set_op(m)?,
            "rig" => {
                let id = str_of(m, "id")?;
                if lighting::rigs::rig_by_id(id).is_none() {
                    return Err(bad(format!("rig '{id}' desconhecido; disponíveis: {}", RIGS.iter().map(|r| r.id).collect::<Vec<_>>().join(", "))));
                }
                self.light_apply_rig(id);
            }
            "follow" => self.light_follow_op(m)?,
            "reset_sun" => {
                let d = anigo_core::StylizedLight::default();
                let id = view0.id.clone();
                self.light_apply(LightPatch { direction: Some(d.direction), color: Some(d.color), intensity: Some(d.intensity), ..Default::default() }.into_command(id));
                self.light_ui.temp_k = 5600.0;
            }
            "view" => {
                if let Some(v) = m.get("show_gizmo") {
                    self.light_ui.show_gizmo = v.as_bool().ok_or_else(|| bad("'show_gizmo' deve ser booleano"))?;
                }
                if m.get("frame").and_then(Value::as_bool).unwrap_or(false) {
                    self.light_frame_character();
                }
            }
            other => return Err(bad(format!("LIGHT op '{other}' desconhecida (info, set, rig, follow, reset_sun, view)"))),
        }
        let view1 = self.light_view();
        let mut out = self.light_info();
        out["changed"] = json!(view1.light != view0.light || view1.material != view0.material);
        out["history_steps_added"] = json!(self.core.history().undo_depth().saturating_sub(depth0));
        out["message"] = json!(self.light_ui.message);
        Ok(out)
    }

    fn light_info(&mut self) -> Value {
        let v = self.light_view();
        let l = &v.light;
        let (az, el) = lighting::az_el_from_dir(Vec3::from_array(l.direction), 0.0);
        let u = &self.light_ui;
        let (can_undo, can_redo) = (self.core.history().undo_depth() > 0, self.core.history().redo_depth() > 0);
        json!({
            "workspace": self.ws.key(),
            "light": {
                "direction": l.direction, "azimuth_deg": az, "elevation_deg": el, "color": l.color, "temperature_k": u.temp_k, "intensity": l.intensity,
                "shadow_color": l.shadow_color, "shadow_saturation": l.shadow_saturation,
                "ambient_intensity": l.ambient_intensity, "ambient_sky": l.ambient_sky, "ambient_ground": l.ambient_ground,
                "shadow": serde_json::to_value(l.shadow).unwrap_or(Value::Null),
            },
            "material": v.material.as_ref().map(|m| json!({
                "shadow_threshold": m.shadow_threshold, "shadow_smoothness": m.shadow_smoothness,
                "rim_intensity": m.rim_intensity, "rim_spread": m.rim_spread, "rim_color": m.rim_color,
            })),
            "follow_camera": { "on": u.follow_camera, "rel_az": u.rel_az, "rel_el": u.rel_el, "effective_direction": self.light_effective_dir().to_array() },
            "last_rig": u.last_rig,
            "rigs": RIGS.iter().map(|r| json!({ "id": r.id, "azimuth_deg": r.az, "elevation_deg": r.el, "kelvin": r.kelvin, "intensity": r.intensity, "ambient": r.ambient, "rim_intensity": r.rim_intensity })).collect::<Vec<_>>(),
            "show_gizmo": u.show_gizmo,
            "extra_lights": self.core.project().scene.lights.len().saturating_sub(1),
            "history": { "can_undo": can_undo, "can_redo": can_redo, "undo_depth": self.core.history().undo_depth() },
        })
    }

    /// Patch de luz e/ou material → UM comando (`Batch` quando são os dois).
    fn light_set_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let patch = m.get("patch").and_then(Value::as_object).ok_or_else(|| bad("'patch' é obrigatório (azimuth_deg, elevation_deg, direction, color, kelvin, intensity, shadow_color, shadow_saturation, ambient_intensity, ambient_sky, ambient_ground, shadow, material)"))?;
        let v = self.light_view();
        let (mut az, mut el) = self.light_current_angles();
        let (mut angles_set, mut lp, mut mp) = (false, LightPatch::default(), MaterialPatch::default());
        let mut has_material = false;
        let mut kelvin = None;
        for (k, val) in patch {
            match k.as_str() {
                "azimuth_deg" => {
                    az = num_in(val, k, -360.0, 360.0)?;
                    angles_set = true;
                }
                "elevation_deg" => {
                    el = num_in(val, k, -90.0, 90.0)?;
                    angles_set = true;
                }
                "direction" => {
                    if self.light_ui.follow_camera {
                        return Err(bad("'direction' não vale com a luz presa à câmera (desligue com op=follow, ou use azimuth_deg/elevation_deg relativos)"));
                    }
                    let a = val.as_array().filter(|a| a.len() == 3).ok_or_else(|| bad("'direction' deve ter 3 números"))?;
                    let d = Vec3::new(num_in(&a[0], k, -1.0e3, 1.0e3)?, num_in(&a[1], k, -1.0e3, 1.0e3)?, num_in(&a[2], k, -1.0e3, 1.0e3)?);
                    if d.length_squared() < 1e-10 {
                        return Err(bad("'direction' não pode ser o vetor zero"));
                    }
                    lp.direction = Some(d.normalize().to_array());
                }
                "color" => lp.color = Some(rgb(val, k)?),
                "kelvin" => kelvin = Some(num_in(val, k, 1000.0, 40000.0)?),
                "intensity" => lp.intensity = Some(num_in(val, k, 0.0, 10.0)?),
                "shadow_color" => lp.shadow_color = Some(rgb(val, k)?),
                "shadow_saturation" => lp.shadow_saturation = Some(num_in(val, k, 0.0, 2.0)?),
                "ambient_intensity" => lp.ambient_intensity = Some(num_in(val, k, 0.0, 2.0)?),
                "ambient_sky" => lp.ambient_sky = Some(rgb(val, k)?),
                "ambient_ground" => lp.ambient_ground = Some(rgb(val, k)?),
                "shadow" => {
                    let mut s = serde_json::to_value(v.light.shadow).map_err(|e| BridgeError::failed(e.to_string()))?;
                    super::bridge_model::reject_unknown_keys(&s, val, "shadow")?;
                    merge(&mut s, val);
                    let sh: LightShadow = serde_json::from_value(s).map_err(|e| bad(format!("shadow inválido: {e}")))?;
                    sh.validate().map_err(|e| bad(format!("shadow inválido: {e}")))?;
                    lp.shadow = Some(sh);
                }
                "material" => {
                    let o = val.as_object().ok_or_else(|| bad("'material' deve ser um objeto {shadow_threshold, shadow_smoothness, rim_intensity, rim_spread, rim_color}"))?;
                    for (mk, mv) in o {
                        match mk.as_str() {
                            "shadow_threshold" => mp.shadow_threshold = Some(num_in(mv, mk, 0.0, 1.0)?),
                            "shadow_smoothness" => mp.shadow_smoothness = Some(num_in(mv, mk, 0.0, 1.0)?),
                            "rim_intensity" => mp.rim_intensity = Some(num_in(mv, mk, 0.0, 3.0)?),
                            "rim_spread" => mp.rim_spread = Some(num_in(mv, mk, 0.0, 1.0)?),
                            "rim_color" => {
                                let a = mv.as_array().filter(|a| a.len() == 3 || a.len() == 4).ok_or_else(|| bad("'rim_color' deve ter 3 (RGB) ou 4 (RGBA) números"))?;
                                let c = |i: usize, d: f32| a.get(i).map(|x| num_in(x, mk, 0.0, 1.0)).unwrap_or(Ok(d));
                                let old = v.material.as_ref().map(|m| m.rim_color[3]).unwrap_or(1.0);
                                mp.rim_color = Some([c(0, 0.0)?, c(1, 0.0)?, c(2, 0.0)?, c(3, old)?]);
                            }
                            o => return Err(bad(format!("material.{o} desconhecido (shadow_threshold, shadow_smoothness, rim_intensity, rim_spread, rim_color)"))),
                        }
                        has_material = true;
                    }
                }
                o => return Err(bad(format!("campo desconhecido 'patch.{o}'; disponíveis: ambient_ground, ambient_intensity, ambient_sky, azimuth_deg, color, direction, elevation_deg, intensity, kelvin, material, shadow, shadow_color, shadow_saturation"))),
            }
        }
        if kelvin.is_some() && lp.color.is_some() {
            return Err(bad("informe 'color' OU 'kelvin', não os dois"));
        }
        if let Some(k) = kelvin {
            lp.color = Some(lighting::kelvin_to_rgb(k));
            self.light_ui.temp_k = k;
        }
        if has_material && v.material.is_none() {
            return Err(BridgeError::failed("não há material do personagem para editar (sem personagem no projeto)"));
        }
        if angles_set {
            if patch.contains_key("direction") {
                return Err(bad("use 'direction' OU azimuth_deg/elevation_deg"));
            }
            if self.light_ui.follow_camera {
                // como o painel: com a luz presa à câmera só muda o estado relativo (sem comando)
                self.light_ui.rel_az = az;
                self.light_ui.rel_el = el;
            } else {
                lp.direction = Some(lighting::dir_from_az_el(az, el).to_array());
            }
        }
        let light_cmd = {
            let any = lp.direction.is_some() || lp.color.is_some() || lp.intensity.is_some() || lp.shadow_color.is_some() || lp.ambient_intensity.is_some() || lp.shadow_saturation.is_some() || lp.ambient_sky.is_some() || lp.ambient_ground.is_some() || lp.shadow.is_some();
            any.then(|| lp.into_command(v.id.clone()))
        };
        let mat_cmd = has_material.then(|| Command::SetMaterialParams { material_id: None, patch: mp });
        let cmd = match (light_cmd, mat_cmd) {
            (Some(l), Some(mt)) => Command::Batch { commands: vec![l, mt] },
            (Some(c), None) | (None, Some(c)) => c,
            (None, None) => return Ok(()), // só ângulos relativos da luz presa
        };
        match self.core.apply(cmd) {
            Ok(o) => {
                self.light_ui.message = super::lighting_ui::describe_light(&o.description);
                Ok(())
            }
            Err(e) if e.contains("no effect") || e.contains("already") => Ok(()),
            Err(e) => Err(BridgeError::failed(e)),
        }
    }

    fn light_follow_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        if let Some(on) = m.get("on") {
            let on = on.as_bool().ok_or_else(|| bad("'on' deve ser booleano"))?;
            self.light_set_follow(on);
        }
        if let Some(a) = m.get("rel_az") {
            self.light_ui.rel_az = num_in(a, "rel_az", -360.0, 360.0)?;
        }
        if let Some(e) = m.get("rel_el") {
            self.light_ui.rel_el = num_in(e, "rel_el", -90.0, 90.0)?;
        }
        if m.get("bake").and_then(Value::as_bool).unwrap_or(false) {
            if !self.light_ui.follow_camera {
                return Err(BridgeError::failed("a luz não está presa à câmera (use on=true antes)"));
            }
            self.light_bake_follow();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_colors_are_range_checked() {
        assert!(num_in(&json!(1.0), "x", 0.0, 2.0).is_ok());
        assert!(num_in(&json!(3.0), "x", 0.0, 2.0).is_err());
        assert!(num_in(&json!("a"), "x", 0.0, 2.0).is_err());
        assert!(rgb(&json!([0.1, 0.2, 0.3]), "c").is_ok());
        assert!(rgb(&json!([0.1, 0.2, 1.3]), "c").is_err());
        assert!(rgb(&json!([0.1, 0.2]), "c").is_err());
    }

    #[test]
    fn shadow_patch_merges_and_the_core_rules_apply() {
        let mut base = serde_json::to_value(LightShadow::default()).unwrap();
        merge(&mut base, &json!({ "enabled": true, "strength": 0.5 }));
        let s: LightShadow = serde_json::from_value(base).unwrap();
        assert!(s.enabled && (s.strength - 0.5).abs() < 1e-6 && s.resolution == 2048);
        assert!(s.validate().is_ok());
        let bad_s = LightShadow { strength: 3.0, ..Default::default() };
        assert!(bad_s.validate().is_err());
    }
}
