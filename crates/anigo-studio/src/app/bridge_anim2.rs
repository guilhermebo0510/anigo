//! Animação pela bridge, parte 2 (`ANIM`): edição fina de chaves (seleção, interpolação, mover, duplicar, espelhar, copiar/colar,
//! valor), marcadores, chaves de cena (sol/câmera/nó), onion skin, física secundária, importar/exportar, sequência de render,
//! Biblioteca de clips e opções. Usa as MESMAS funções da timeline e do inspetor (`anim_*`, `keys::plan_*`): cada ação é UM passo de
//! histórico do núcleo (`anigo_native_undo`). Tudo é validado antes de mexer.

use super::animation_ui::ExportKind;
use super::bridge_model::reject_unknown_keys;
use super::StudioApp;
use crate::anim::keys::{self, KeyId};
use crate::anim::secondary::SecondaryConfig;
use anigo_bridge::protocol::{BridgeError, BridgeResult};
use anigo_core::animation::{AnimEdit, CameraProp, ChannelTarget, Interp, Key, KeyInsert, KeyValue, Marker, SunProp};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

fn bad(msg: impl Into<String>) -> BridgeError {
    BridgeError::invalid_params(msg)
}

fn str_of<'a>(m: &'a Map<String, Value>, k: &str) -> Result<&'a str, BridgeError> {
    m.get(k).and_then(Value::as_str).ok_or_else(|| bad(format!("'{k}' é obrigatório e deve ser texto")))
}

fn frame_of(v: &Value, name: &str) -> Result<i32, BridgeError> {
    v.as_i64().filter(|n| (0..=1_000_000).contains(n)).map(|n| n as i32).ok_or_else(|| bad(format!("'{name}' deve ser um inteiro em 0..1000000")))
}

fn nums<const N: usize>(v: &Value, name: &str) -> Result<[f32; N], BridgeError> {
    let a = v.as_array().filter(|a| a.len() == N).ok_or_else(|| bad(format!("'{name}' deve ter {N} números")))?;
    let mut o = [0f32; N];
    for (i, x) in a.iter().enumerate() {
        o[i] = x.as_f64().filter(|f| f.is_finite() && f.abs() < 1.0e6).ok_or_else(|| bad(format!("'{name}' deve conter números finitos")))? as f32;
    }
    Ok(o)
}

fn bool_of(v: &Value, name: &str) -> Result<bool, BridgeError> {
    v.as_bool().ok_or_else(|| bad(format!("'{name}' deve ser booleano")))
}

/// `Interp` a partir de texto (`step|linear|ease|ease_in|ease_out|bezier`) e, para `bezier`, as alças `c` = [x1,y1,x2,y2].
fn interp_of(name: &str, c: Option<&Value>) -> Result<Interp, BridgeError> {
    Ok(match name {
        "step" => Interp::Step,
        "linear" => Interp::Linear,
        "ease" => Interp::Ease,
        "ease_in" => Interp::EaseIn,
        "ease_out" => Interp::EaseOut,
        "bezier" => match c {
            Some(c) => {
                let c = nums::<4>(c, "c")?;
                if !(0.0..=1.0).contains(&c[0]) || !(0.0..=1.0).contains(&c[2]) || c[1].abs() > 5.0 || c[3].abs() > 5.0 {
                    return Err(bad("alças Bézier: x1,x2 em 0..1 e y1,y2 em -5..5"));
                }
                Interp::Bezier { c }
            }
            None => Interp::bezier_linear(),
        },
        o => return Err(bad(format!("interpolação '{o}' desconhecida (step, linear, ease, ease_in, ease_out, bezier)"))),
    })
}

fn interp_name(i: &Interp) -> &'static str {
    match i {
        Interp::Step => "step",
        Interp::Linear => "linear",
        Interp::Ease => "ease",
        Interp::EaseIn => "ease_in",
        Interp::EaseOut => "ease_out",
        Interp::Bezier { .. } => "bezier",
    }
}

impl StudioApp {
    /// Canal pelo JSON: `{ "bone": "LeftElbow" }` ou `{ "target": { "kind": "root" } }` (o formato de `anim_info`).
    fn anim_target_of(&self, v: &Map<String, Value>) -> Result<ChannelTarget, BridgeError> {
        if let Some(b) = v.get("bone").and_then(Value::as_str) {
            let name = anigo_core::pose::BONE_NAMES.iter().find(|n| n.eq_ignore_ascii_case(b)).ok_or_else(|| bad(format!("osso '{b}' desconhecido")))?;
            return Ok(ChannelTarget::bone(name));
        }
        let t = v.get("target").ok_or_else(|| bad("informe 'bone' ou 'target' (objeto do canal, como em info.active_clip.channels)"))?;
        serde_json::from_value(t.clone()).map_err(|e| bad(format!("'target' inválido: {e}")))
    }

    fn anim_key_ids(&self, items: &Value) -> Result<BTreeSet<KeyId>, BridgeError> {
        let a = items.as_array().filter(|a| a.len() <= 5000).ok_or_else(|| bad("'items' deve ser uma lista de {bone|target, frame}"))?;
        let mut out = BTreeSet::new();
        for it in a {
            let o = it.as_object().ok_or_else(|| bad("cada item deve ser um objeto {bone|target, frame}"))?;
            out.insert((self.anim_target_of(o)?, frame_of(o.get("frame").ok_or_else(|| bad("item sem 'frame'"))?, "frame")?));
        }
        Ok(out)
    }

    fn anim_selection_json(&self) -> Value {
        let clip = self.anim_clip();
        json!(self
            .anim_selected_existing()
            .iter()
            .map(|(t, f)| {
                let key = clip.and_then(|c| c.channel(t)).and_then(|c| c.key_at(*f));
                json!({ "target": serde_json::to_value(t).unwrap_or(Value::Null), "frame": f, "interp": key.map(|k| interp_name(&k.interp)), "value": key.map(|k| serde_json::to_value(k.value).unwrap_or(Value::Null)) })
            })
            .collect::<Vec<_>>())
    }

    fn anim_full_info(&self) -> Value {
        let mut v = self.anim_info_pub();
        v["selected_keys"] = self.anim_selection_json();
        v["onion"] = json!({ "enabled": self.anim_ui.onion.enabled, "before": self.anim_ui.onion.before, "after": self.anim_ui.onion.after, "step": self.anim_ui.onion.step, "on_keys": self.anim_ui.onion.on_keys });
        v["markers"] = json!(self.anim_clip().map(|c| c.markers.iter().map(|m| json!({ "frame": m.frame, "name": m.name })).collect::<Vec<_>>()).unwrap_or_default());
        v["physics"] = json!({ "enabled": self.anim_ui.phys.enabled, "hair": self.anim_ui.phys.cfg.hair, "skirt": self.anim_ui.phys.cfg.skirt, "pre_roll_s": self.anim_ui.phys.cfg.pre_roll_s, "status": self.anim_ui.phys.status });
        v["options"] = json!({ "interp": match self.anim_ui.interp { super::animation_ui::InterpPick::Step => "step", super::animation_ui::InterpPick::Linear => "linear", super::animation_ui::InterpPick::Ease => "ease", super::animation_ui::InterpPick::Bezier => "bezier" }, "quantize": self.anim_ui.transport.quantize, "only_selected_bones": self.anim_ui.only_selected_bones, "retime": self.anim_ui.retime });
        v["sequence"] = json!({ "custom": self.anim_ui.seq_custom, "start": self.anim_ui.seq_start, "end": self.anim_ui.seq_end, "step": self.anim_ui.seq_step, "template": self.anim_ui.seq_template, "job_id": self.anim_ui.seq_job });
        v["io_status"] = json!(self.anim_ui.io_status);
        v["clip_assets"] = json!(self.anim_ui.cards.iter().enumerate().map(|(i, c)| json!({ "index": i, "name": c.name, "revision": c.revision, "keys": c.asset.key_count() })).collect::<Vec<_>>());
        v
    }

    pub(crate) fn anim_ext_op(&mut self, op: &str, m: &Map<String, Value>) -> BridgeResult {
        let before = self.core.history().undo_depth();
        match op {
            "keys" => self.anim_keys_op(m)?,
            "marker" => self.anim_marker_op(m)?,
            "scene_keys" => self.anim_scene_keys_op(m)?,
            "onion" => self.anim_onion_op(m)?,
            "physics" => self.anim_physics_op(m)?,
            "file" => self.anim_file_op(m)?,
            "library" => self.anim_library_op(m)?,
            "options" => self.anim_options_op(m)?,
            _ => return Err(bad(format!("ANIM op '{op}' desconhecida"))),
        }
        let mut out = self.anim_full_info();
        out["history_steps_added"] = json!(self.core.history().undo_depth().saturating_sub(before));
        Ok(out)
    }

    fn anim_need_clip(&self) -> Result<(anigo_core::ids::AnimationClipId, anigo_core::project::AnimationClip), BridgeError> {
        match (self.anim_clip_id(), self.anim_clip().cloned()) {
            (Some(i), Some(c)) => Ok((i, c)),
            _ => Err(BridgeError::failed("não há clip ativo (crie um com clip new)")),
        }
    }

    // ------------------------------------------------------------------------------------------------ chaves

    fn anim_keys_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let action = str_of(m, "action")?;
        match action {
            "select" => {
                let (_, clip) = self.anim_need_clip()?;
                let set: BTreeSet<KeyId> = if m.get("all").and_then(Value::as_bool).unwrap_or(false) {
                    clip.channels.iter().flat_map(|c| c.keys.iter().map(|k| (c.target.clone(), k.frame))).collect()
                } else if let Some(r) = m.get("frame_range") {
                    let [a, b] = nums::<2>(r, "frame_range")?;
                    let bone = m.get("bone").and_then(Value::as_str).map(|s| s.to_lowercase());
                    clip.channels
                        .iter()
                        .filter(|c| bone.as_ref().is_none_or(|b| matches!(&c.target, ChannelTarget::Bone { bone } if bone.to_lowercase() == *b)))
                        .flat_map(|c| c.keys.iter().filter(|k| k.frame as f32 >= a && k.frame as f32 <= b).map(|k| (c.target.clone(), k.frame)))
                        .collect()
                } else if m.get("none").and_then(Value::as_bool).unwrap_or(false) {
                    BTreeSet::new()
                } else {
                    self.anim_key_ids(m.get("items").ok_or_else(|| bad("informe 'items', 'all', 'frame_range' ou 'none'"))?)?
                };
                for (t, f) in &set {
                    if clip.channel(t).and_then(|c| c.key_at(*f)).is_none() {
                        return Err(bad(format!("não existe chave em {:?} no quadro {f}", t)));
                    }
                }
                if m.get("mode").and_then(Value::as_str) == Some("add") {
                    self.anim_ui.selection.extend(set);
                } else {
                    self.anim_ui.selection = set;
                }
            }
            "set_interp" => {
                let i = interp_of(str_of(m, "interp")?, m.get("c"))?;
                self.anim_need_selection()?;
                self.anim_set_interp_selected(i);
            }
            "move" => {
                let d = m.get("delta").and_then(Value::as_i64).filter(|d| d.abs() <= 100_000 && *d != 0).ok_or_else(|| bad("'delta' deve ser um inteiro diferente de 0 (quadros)"))? as i32;
                let (id, clip) = self.anim_need_clip()?;
                self.anim_need_selection()?;
                let set: BTreeSet<KeyId> = self.anim_selected_existing().into_iter().collect();
                let (e, applied) = keys::plan_move(&id, &clip, &set, d).ok_or_else(|| BridgeError::failed("não foi possível mover (limite do clip?)"))?;
                let moved = set.iter().map(|(t, f)| (t.clone(), f + applied)).collect();
                if self.anim_apply(e) {
                    self.anim_ui.selection = moved;
                }
            }
            "duplicate" => {
                let d = m.get("delta").and_then(Value::as_i64).filter(|d| d.abs() <= 100_000 && *d != 0).ok_or_else(|| bad("'delta' deve ser um inteiro diferente de 0 (quadros)"))? as i32;
                self.anim_need_selection()?;
                self.anim_duplicate_selected(d);
            }
            "mirror" => {
                self.anim_need_selection()?;
                self.anim_mirror_selected();
            }
            "delete" => {
                self.anim_need_selection()?;
                self.anim_delete_selected();
            }
            "copy" => {
                self.anim_need_selection()?;
                self.anim_copy_keys();
            }
            "paste" => {
                if self.anim_ui.clipboard.is_none() {
                    return Err(BridgeError::failed("a área de transferência de chaves está vazia (use keys copy antes)"));
                }
                if let Some(f) = m.get("frame") {
                    let f = frame_of(f, "frame")?;
                    self.scrub_to(f as f32, false);
                }
                self.anim_paste_keys(m.get("mirrored").and_then(Value::as_bool).unwrap_or(false));
            }
            "set_value" => {
                let (id, clip) = self.anim_need_clip()?;
                let target = self.anim_target_of(m)?;
                let frame = frame_of(m.get("frame").ok_or_else(|| bad("'frame' é obrigatório"))?, "frame")?;
                let old = clip.channel(&target).and_then(|c| c.key_at(frame)).copied().ok_or_else(|| bad("não existe chave nesse canal/quadro (use key insert antes)"))?;
                let value = match target.value_kind() {
                    anigo_core::animation::ValueKind::Scalar => KeyValue::Scalar(nums::<1>(&json!([m.get("value").cloned().unwrap_or(Value::Null)]), "value")?[0]),
                    anigo_core::animation::ValueKind::Vec3 => KeyValue::Vec3(nums::<3>(m.get("value").ok_or_else(|| bad("'value' [x,y,z] é obrigatório"))?, "value")?),
                    anigo_core::animation::ValueKind::Curl => {
                        let c = nums::<5>(m.get("value").ok_or_else(|| bad("'value' [5 curvaturas 0..1] é obrigatório"))?, "value")?;
                        if c.iter().any(|v| !(0.0..=1.0).contains(v)) {
                            return Err(bad("curvaturas devem estar em 0..1"));
                        }
                        KeyValue::Curl(c)
                    }
                    anigo_core::animation::ValueKind::Quat => {
                        if let Some(e) = m.get("euler_deg") {
                            KeyValue::Quat(anigo_core::pose::euler_deg_to_quat(nums::<3>(e, "euler_deg")?).to_array())
                        } else {
                            let q = nums::<4>(m.get("quat").ok_or_else(|| bad("informe 'euler_deg' [x,y,z] ou 'quat' [x,y,z,w]"))?, "quat")?;
                            let q = glam::Quat::from_array(q);
                            if q.length_squared() < 1e-8 {
                                return Err(bad("'quat' não pode ser zero"));
                            }
                            KeyValue::Quat(q.normalize().to_array())
                        }
                    }
                };
                let interp = match m.get("interp").and_then(Value::as_str) {
                    Some(n) => interp_of(n, m.get("c"))?,
                    None => old.interp,
                };
                self.anim_apply(AnimEdit::InsertKeys { clip_id: id, keys: vec![KeyInsert { target: target.clone(), key: Key { frame, value, interp } }] });
                self.anim_ui.selection = [(target, frame)].into_iter().collect();
            }
            o => return Err(bad(format!("keys.action '{o}' desconhecida (select, set_interp, move, duplicate, mirror, delete, copy, paste, set_value)"))),
        }
        Ok(())
    }

    fn anim_need_selection(&self) -> Result<(), BridgeError> {
        if self.anim_selected_existing().is_empty() {
            return Err(BridgeError::failed("não há chaves selecionadas (use keys select)"));
        }
        Ok(())
    }

    // ------------------------------------------------------------------------------------------------ marcadores

    fn anim_marker_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let action = str_of(m, "action")?;
        if action == "list" {
            return Ok(());
        }
        let id = self.anim_ensure_clip().ok_or_else(|| BridgeError::failed("não foi possível criar o clip"))?;
        let mut markers: Vec<Marker> = self.anim_clip().map(|c| c.markers.clone()).unwrap_or_default();
        match action {
            "toggle" => {
                let f = match m.get("frame") {
                    Some(f) => frame_of(f, "frame")?,
                    None => self.anim_frame(),
                };
                if markers.iter().any(|x| x.frame == f) {
                    markers.retain(|x| x.frame != f);
                } else {
                    markers.push(Marker { frame: f, name: m.get("name").and_then(Value::as_str).unwrap_or("Marcador").chars().take(64).collect() });
                }
            }
            "add" => {
                let f = frame_of(m.get("frame").ok_or_else(|| bad("'frame' é obrigatório"))?, "frame")?;
                let name: String = str_of(m, "name")?.trim().chars().take(64).collect();
                if name.is_empty() {
                    return Err(bad("'name' não pode ser vazio"));
                }
                markers.retain(|x| x.frame != f);
                markers.push(Marker { frame: f, name });
            }
            "rename" => {
                let f = frame_of(m.get("frame").ok_or_else(|| bad("'frame' é obrigatório"))?, "frame")?;
                let name: String = str_of(m, "name")?.trim().chars().take(64).collect();
                let mk = markers.iter_mut().find(|x| x.frame == f).ok_or_else(|| bad(format!("não há marcador no quadro {f}")))?;
                mk.name = name;
            }
            "remove" => {
                let f = frame_of(m.get("frame").ok_or_else(|| bad("'frame' é obrigatório"))?, "frame")?;
                if !markers.iter().any(|x| x.frame == f) {
                    return Err(bad(format!("não há marcador no quadro {f}")));
                }
                markers.retain(|x| x.frame != f);
            }
            o => return Err(bad(format!("marker.action '{o}' desconhecida (list, toggle, add, rename, remove)"))),
        }
        markers.sort_by_key(|x| x.frame);
        self.anim_apply(AnimEdit::SetMarkers { clip_id: id, markers });
        Ok(())
    }

    // ------------------------------------------------------------------------------------------------ chaves de cena

    fn anim_scene_keys_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        if let Some(f) = m.get("frame") {
            let f = frame_of(f, "frame")?;
            self.scrub_to(f as f32, false);
        }
        let targets = match str_of(m, "what")? {
            "sun" => vec![ChannelTarget::Sun { prop: SunProp::Direction }, ChannelTarget::Sun { prop: SunProp::Intensity }, ChannelTarget::Sun { prop: SunProp::Color }],
            "camera" => vec![ChannelTarget::Camera { prop: CameraProp::Fov }, ChannelTarget::Camera { prop: CameraProp::Eye }, ChannelTarget::Camera { prop: CameraProp::Target }],
            "node" => {
                let want = str_of(m, "node_id")?;
                let id = self.core.project().scene.nodes.iter().map(|n| n.node_id.clone()).find(|i| i.to_string() == want).ok_or_else(|| bad(format!("nó '{want}' não existe na cena")))?;
                vec![ChannelTarget::NodeTranslation { node_id: id.clone() }, ChannelTarget::NodeRotation { node_id: id.clone() }, ChannelTarget::NodeScale { node_id: id }]
            }
            o => return Err(bad(format!("what '{o}' desconhecido (sun, camera, node)"))),
        };
        let depth = self.core.history().undo_depth();
        self.anim_insert_targets(targets);
        if self.core.history().undo_depth() == depth {
            return Err(BridgeError::failed(format!("nada mudou: {}", self.anim_ui.message)));
        }
        Ok(())
    }

    // ------------------------------------------------------------------------------------------------ onion, opções, física

    fn anim_onion_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let patch = m.get("patch").and_then(Value::as_object).ok_or_else(|| bad("'patch' é obrigatório (enabled, before 0..6, after 0..6, step 1..12, on_keys)"))?;
        let mut o = self.anim_ui.onion;
        for (k, v) in patch {
            let n = |max: u64, min: u64| v.as_u64().filter(|x| (min..=max).contains(x)).map(|x| x as u8).ok_or_else(|| bad(format!("'{k}' deve ser {min}..{max}")));
            match k.as_str() {
                "enabled" => o.enabled = bool_of(v, k)?,
                "on_keys" => o.on_keys = bool_of(v, k)?,
                "before" => o.before = n(6, 0)?,
                "after" => o.after = n(6, 0)?,
                "step" => o.step = n(12, 1)?,
                other => return Err(bad(format!("campo desconhecido 'patch.{other}' (enabled, before, after, step, on_keys)"))),
            }
        }
        self.anim_ui.onion = o;
        Ok(())
    }

    fn anim_options_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        use super::animation_ui::InterpPick;
        let patch = m.get("patch").and_then(Value::as_object).ok_or_else(|| bad("'patch' é obrigatório (interp, quantize, only_selected_bones, retime)"))?;
        let (mut pick, mut quant, mut only, mut retime) = (self.anim_ui.interp, self.anim_ui.transport.quantize, self.anim_ui.only_selected_bones, self.anim_ui.retime);
        for (k, v) in patch {
            match k.as_str() {
                "interp" => {
                    pick = match v.as_str() {
                        Some("step") => InterpPick::Step,
                        Some("linear") => InterpPick::Linear,
                        Some("ease") => InterpPick::Ease,
                        Some("bezier") => InterpPick::Bezier,
                        _ => return Err(bad("'interp' (das chaves NOVAS) deve ser step|linear|ease|bezier")),
                    }
                }
                "quantize" => quant = bool_of(v, k)?,
                "only_selected_bones" => only = bool_of(v, k)?,
                "retime" => retime = bool_of(v, k)?,
                other => return Err(bad(format!("campo desconhecido 'patch.{other}' (interp, quantize, only_selected_bones, retime)"))),
            }
        }
        self.anim_ui.interp = pick;
        self.anim_ui.transport.quantize = quant;
        self.anim_ui.only_selected_bones = only;
        self.anim_ui.retime = retime;
        Ok(())
    }

    fn anim_physics_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        match str_of(m, "action")? {
            "enable" => {
                self.anim_ui.phys.enabled = bool_of(m.get("on").ok_or_else(|| bad("'on' é obrigatório"))?, "on")?;
                self.anim_ui.phys.sim = None;
                self.anim_ui.phys.last_raw = None;
            }
            "config" => {
                let patch = m.get("patch").and_then(Value::as_object).ok_or_else(|| bad("'patch' é obrigatório (hair, skirt, pre_roll_s 0..1.5)"))?;
                let mut c: SecondaryConfig = self.anim_ui.phys.cfg;
                for (k, v) in patch {
                    match k.as_str() {
                        "hair" => c.hair = bool_of(v, k)?,
                        "skirt" => c.skirt = bool_of(v, k)?,
                        "pre_roll_s" => c.pre_roll_s = v.as_f64().filter(|f| (0.0..=1.5).contains(f)).ok_or_else(|| bad("'pre_roll_s' deve ser 0..1.5"))? as f32,
                        other => return Err(bad(format!("campo desconhecido 'patch.{other}' (hair, skirt, pre_roll_s)"))),
                    }
                }
                self.anim_ui.phys.cfg = c;
                self.anim_ui.phys.sim = None;
                self.anim_ui.phys.last_raw = None;
            }
            "bake" => {
                self.anim_need_clip()?;
                self.anim_with_export_dir(m, |app| app.anim_bake_physics())?;
                if self.anim_ui.phys.status.contains(&crate::i18n::tr("anim.io.error").to_string()) {
                    return Err(BridgeError::failed(self.anim_ui.phys.status.clone()));
                }
            }
            o => return Err(bad(format!("physics.action '{o}' desconhecida (enable, config, bake)"))),
        }
        Ok(())
    }

    /// Roda `f` com a pasta de exportação trocada por `dir` (absoluta; o sandbox é do `anigo-mcp`).
    fn anim_with_export_dir(&mut self, m: &Map<String, Value>, f: impl FnOnce(&mut Self)) -> Result<(), BridgeError> {
        let dir = match m.get("dir").and_then(Value::as_str) {
            Some(d) => {
                let p = std::path::PathBuf::from(d);
                if !p.is_absolute() || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
                    return Err(bad("'dir' deve ser um caminho absoluto sem '..'"));
                }
                Some(p)
            }
            None => None,
        };
        let old = std::mem::replace(&mut self.anim_ui.export_dir, dir);
        f(self);
        self.anim_ui.export_dir = old;
        Ok(())
    }

    // ------------------------------------------------------------------------------------------------ arquivos e Biblioteca

    fn anim_file_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let path = str_of(m, "path")?;
        let p = std::path::PathBuf::from(path);
        if !p.is_absolute() || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(bad("'path' deve ser um caminho absoluto sem '..'"));
        }
        match str_of(m, "action")? {
            "import" => {
                let msg = self.anim_import_path(&p);
                self.anim_ui.io_status = msg.clone();
                if msg.starts_with(&crate::i18n::tr("anim.io.error").to_string()) {
                    return Err(BridgeError::failed(msg));
                }
            }
            "export" => {
                self.anim_need_clip()?;
                let kind = match str_of(m, "kind")? {
                    "glb" => ExportKind::Glb,
                    "vrma" => ExportKind::Vrma,
                    "bvh" => ExportKind::Bvh,
                    "clip" => ExportKind::ClipJson,
                    o => return Err(bad(format!("kind '{o}' desconhecido (glb, vrma, bvh, clip)"))),
                };
                self.anim_ui.io_path = p.display().to_string();
                self.anim_export_file(kind);
                if self.anim_ui.io_status.starts_with(&crate::i18n::tr("anim.io.error").to_string()) {
                    return Err(BridgeError::failed(self.anim_ui.io_status.clone()));
                }
            }
            o => return Err(bad(format!("file.action '{o}' desconhecida (import, export)"))),
        }
        Ok(())
    }

    fn anim_library_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        match str_of(m, "action")? {
            "list" => self.anim_refresh_library(),
            "save" => {
                self.anim_need_clip()?;
                if let Some(n) = m.get("name").and_then(Value::as_str) {
                    if n.trim().is_empty() || n.chars().count() > 120 {
                        return Err(bad("'name' deve ter 1..120 caracteres"));
                    }
                    self.anim_ui.lib_name = n.trim().to_string();
                }
                let clip = self.anim_clip().cloned().ok_or_else(|| BridgeError::failed("não há clip ativo"))?;
                let name = if self.anim_ui.lib_name.trim().is_empty() { clip.name.clone() } else { self.anim_ui.lib_name.trim().to_string() };
                let asset = anigo_core::clip_asset::ClipAsset::from_clip(name, &clip, self.anim_fps()).map_err(|e| BridgeError::failed(e.to_string()))?;
                let msg = self.lib_workspace().save_clip_asset(&asset).map_err(BridgeError::failed)?;
                self.anim_say(msg);
                self.anim_refresh_library();
            }
            "apply" => {
                self.anim_refresh_library();
                let idx = match (m.get("index").and_then(Value::as_u64), m.get("name").and_then(Value::as_str)) {
                    (Some(i), None) => i as usize,
                    (None, Some(n)) => self.anim_ui.cards.iter().position(|c| c.name == n).ok_or_else(|| bad(format!("clip '{n}' não existe na Biblioteca (veja info.clip_assets)")))?,
                    _ => return Err(bad("informe 'index' OU 'name' (veja info.clip_assets)")),
                };
                if idx >= self.anim_ui.cards.len() {
                    return Err(bad(format!("índice {idx} fora de 0..{}", self.anim_ui.cards.len().saturating_sub(1))));
                }
                self.anim_apply_card(idx, m.get("at_playhead").and_then(Value::as_bool).unwrap_or(false));
            }
            o => return Err(bad(format!("library.action '{o}' desconhecida (list, save, apply)"))),
        }
        Ok(())
    }

    // ------------------------------------------------------------------------------------------------ sequência de render

    /// `None` = adiado (ativos ainda carregando).
    pub(crate) fn anim_sequence_op(&mut self, ctx: &egui::Context, m: &Map<String, Value>) -> Option<BridgeResult> {
        let r = (|| -> Result<bool, BridgeError> {
            if let Some(p) = m.get("patch").and_then(Value::as_object) {
                let (mut custom, mut start, mut end, mut step, mut tpl) = (self.anim_ui.seq_custom, self.anim_ui.seq_start, self.anim_ui.seq_end, self.anim_ui.seq_step, self.anim_ui.seq_template.clone());
                for (k, v) in p {
                    match k.as_str() {
                        "custom" => custom = bool_of(v, k)?,
                        "start" => start = frame_of(v, k)?,
                        "end" => end = frame_of(v, k)?,
                        "step" => step = v.as_u64().filter(|n| (1..=100).contains(n)).ok_or_else(|| bad("'step' deve ser 1..100"))? as u32,
                        "template" => {
                            let t = v.as_str().ok_or_else(|| bad("'template' deve ser texto"))?;
                            if t.trim().is_empty() || t.chars().count() > 200 || t.contains(['/', '\\', '\0']) {
                                return Err(bad("'template' deve ter 1..200 caracteres, sem separadores de pasta"));
                            }
                            tpl = t.to_string();
                        }
                        other => return Err(bad(format!("campo desconhecido 'patch.{other}' (custom, start, end, step, template)"))),
                    }
                }
                if end < start {
                    return Err(bad("'end' não pode ser menor que 'start'"));
                }
                (self.anim_ui.seq_custom, self.anim_ui.seq_start, self.anim_ui.seq_end, self.anim_ui.seq_step, self.anim_ui.seq_template) = (custom, start, end, step, tpl);
            }
            Ok(m.get("render").and_then(Value::as_bool).unwrap_or(false))
        })();
        let render = match r {
            Ok(r) => r,
            Err(e) => return Some(Err(e)),
        };
        if render {
            if self.anim_clip().is_none() {
                return Some(Err(BridgeError::failed("não há clip ativo")));
            }
            if self.scene_ui.cache.loading_count() > 0 {
                return None; // ativos ainda carregando: adia e tenta no próximo quadro
            }
            let before = self.anim_ui.seq_job;
            let n_jobs = self.render_ui.jobs.len();
            self.anim_render_sequence(ctx);
            if self.render_ui.jobs.len() == n_jobs || self.anim_ui.seq_job == before {
                return Some(Err(BridgeError::failed(self.anim_ui.message.clone())));
            }
        }
        let mut out = self.anim_full_info();
        if let Some(id) = self.anim_ui.seq_job {
            out["sequence"]["job"] = json!(self.render_ui.jobs.iter().find(|j| j.id == id).map(|j| json!({ "job_id": j.id, "label": j.label, "progress": j.frac, "total_frames": j.total })));
        }
        Some(Ok(out))
    }

    /// `ANIM info` com todos os campos novos (chaves selecionadas, marcadores, onion, física, opções, sequência).
    pub(crate) fn anim_info_ext(&self) -> Value {
        self.anim_full_info()
    }

    #[cfg(test)]
    pub(crate) fn interp_names() -> Vec<&'static str> {
        ["step", "linear", "ease", "ease_in", "ease_out", "bezier"].to_vec()
    }
}

#[allow(dead_code)]
fn _keep(_: &Value) {
    let _ = reject_unknown_keys;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation_names_roundtrip() {
        for n in StudioApp::interp_names() {
            let i = interp_of(n, None).unwrap();
            assert_eq!(interp_name(&i), n);
        }
        assert!(interp_of("bomba", None).is_err());
        assert!(interp_of("bezier", Some(&json!([0.2, 0.0, 0.8, 1.0]))).is_ok());
        assert!(interp_of("bezier", Some(&json!([2.0, 0.0, 0.8, 1.0]))).is_err());
        assert!(interp_of("bezier", Some(&json!([0.2, 0.0]))).is_err());
    }

    #[test]
    fn frames_and_vectors_are_validated() {
        assert_eq!(frame_of(&json!(12), "f").unwrap(), 12);
        assert!(frame_of(&json!(-1), "f").is_err());
        assert!(frame_of(&json!(1.5), "f").is_err());
        assert!(nums::<3>(&json!([1, 2, 3]), "v").is_ok());
        assert!(nums::<3>(&json!([1, 2]), "v").is_err());
    }
}
