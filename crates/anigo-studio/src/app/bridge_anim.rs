//! Animação pela bridge (`ANIM`): clips, transporte (quadro, play, velocidade), inserir/apagar chaves e auto-key — pelas MESMAS
//! funções da timeline (`anim_*`), então entram no histórico do núcleo (`anigo_native_undo` as desfaz). Edições finas de chaves
//! (mover, interpolação, marcadores, `PutClip` com chaves prontas) continuam sendo o `Command` `edit_animation` do núcleo, que o
//! `anigo_native_apply_command` já aceita; este módulo cobre o que vive só no estado local do app (transporte, auto-key, seleção
//! de ossos que escolhe os canais).

use super::StudioApp;
use crate::workspace::WorkspaceId;
use anigo_bridge::protocol::{BridgeError, BridgeResult};
use anigo_core::animation::{AnimEdit, Extrapolation, LoopMode};
use anigo_core::pose::BONE_NAMES;
use serde_json::{json, Map, Value};

fn str_of<'a>(m: &'a Map<String, Value>, k: &str) -> Result<&'a str, BridgeError> {
    m.get(k).and_then(Value::as_str).ok_or_else(|| BridgeError::invalid_params(format!("'{k}' é obrigatório e deve ser texto")))
}

fn f32_of(m: &Map<String, Value>, k: &str) -> Result<f32, BridgeError> {
    m.get(k).and_then(Value::as_f64).filter(|f| f.is_finite() && f.abs() < 1.0e7).map(|f| f as f32).ok_or_else(|| BridgeError::invalid_params(format!("'{k}' é obrigatório e deve ser um número finito")))
}

fn bones_of(v: &Value) -> Result<Vec<usize>, BridgeError> {
    let a = v.as_array().filter(|a| a.len() <= BONE_NAMES.len()).ok_or_else(|| BridgeError::invalid_params("'bones' deve ser uma lista de índices ou nomes de osso"))?;
    a.iter()
        .map(|b| match b {
            Value::Number(n) => n.as_u64().filter(|i| (*i as usize) < BONE_NAMES.len()).map(|i| i as usize).ok_or_else(|| BridgeError::invalid_params(format!("osso fora de 0..{}", BONE_NAMES.len()))),
            Value::String(s) => BONE_NAMES.iter().position(|n| n.eq_ignore_ascii_case(s)).ok_or_else(|| BridgeError::invalid_params(format!("osso '{s}' desconhecido"))),
            _ => Err(BridgeError::invalid_params("'bones': índices ou nomes")),
        })
        .collect()
}

fn enum_of<T: serde::de::DeserializeOwned>(v: &Value, what: &str, options: &str) -> Result<T, BridgeError> {
    serde_json::from_value(v.clone()).map_err(|_| BridgeError::invalid_params(format!("{what} inválido; use {options}")))
}

impl StudioApp {
    pub(crate) fn bridge_anim(&mut self, params: &Value) -> BridgeResult {
        let m = params.as_object().ok_or_else(|| BridgeError::invalid_params("params deve ser um objeto"))?;
        let op = str_of(m, "op")?;
        if self.ws != WorkspaceId::Animacao {
            self.switch_workspace(WorkspaceId::Animacao);
        }
        match op {
            "info" => Ok(self.anim_info_ext()),
            "keys" | "marker" | "scene_keys" | "onion" | "physics" | "file" | "library" | "options" => self.anim_ext_op(op, m),
            "clip" => self.anim_clip_op(m),
            "transport" => self.anim_transport_op(m),
            "key" => self.anim_key_op(m),
            other => Err(BridgeError::invalid_params(format!("ANIM op '{other}' desconhecida (info, clip, transport, key, keys, marker, scene_keys, onion, physics, file, library, options, sequence)"))),
        }
    }

    pub(crate) fn anim_info_pub(&self) -> Value {
        self.anim_info(&Map::new())
    }

    /// `ANIM`: `sequence` precisa do contexto (enfileira o render) e pode ser adiada; o resto roda direto.
    pub(crate) fn bridge_anim_ctx(&mut self, ctx: &egui::Context, params: &Value) -> Option<BridgeResult> {
        if let Some(m) = params.as_object() {
            if m.get("op").and_then(Value::as_str) == Some("sequence") {
                if self.ws != WorkspaceId::Animacao {
                    self.switch_workspace(WorkspaceId::Animacao);
                }
                return self.anim_sequence_op(ctx, m);
            }
        }
        Some(self.bridge_anim(params))
    }

    fn anim_info(&self, m: &Map<String, Value>) -> Value {
        let limit = m.get("limit").and_then(Value::as_u64).map(|n| (n as usize).min(500)).unwrap_or(60);
        let a = &self.core.project().animation;
        let clips: Vec<Value> = a
            .clips
            .values()
            .map(|c| {
                let keys: usize = c.channels.iter().map(|ch| ch.keys.len()).sum();
                let (lo, hi) = c.effective_range(a.fps);
                json!({ "id": c.clip_id, "name": c.name, "active": a.active_clip.as_ref() == Some(&c.clip_id), "channels": c.channels.len(), "keys": keys,
                        "range": [lo, hi], "loop_mode": c.loop_mode, "extrapolation": c.extrapolation, "hold_step": c.hold_step })
            })
            .collect();
        let active = self.anim_clip().map(|c| {
            json!({ "id": c.clip_id, "channels": c.channels.iter().take(limit).map(|ch| json!({
                "target": serde_json::to_value(&ch.target).unwrap_or(Value::Null),
                "frames": ch.keys.iter().map(|k| k.frame).collect::<Vec<_>>(),
            })).collect::<Vec<_>>() })
        });
        json!({
            "fps": a.fps, "frame": self.anim_frame(), "playing": self.anim_ui.transport.playing, "speed": self.anim_ui.transport.speed,
            "autokey": self.anim_ui.autokey, "clips": clips, "active_clip": active,
            "selected_bones": self.pose_ui.selection.iter().map(|b| BONE_NAMES[*b]).collect::<Vec<_>>(),
            "status": self.anim_ui.message,
        })
    }

    fn anim_clip_ref(&self, m: &Map<String, Value>) -> Result<anigo_core::ids::AnimationClipId, BridgeError> {
        match m.get("id").and_then(Value::as_str) {
            Some(id) => {
                let found = self.core.project().animation.clips.keys().find(|k| k.to_string() == id).cloned();
                found.ok_or_else(|| BridgeError::invalid_params(format!("clip '{id}' não existe (veja info.clips)")))
            }
            None => self.anim_clip_id().ok_or_else(|| BridgeError::failed("não há clip ativo (crie um com clip new)")),
        }
    }

    fn anim_clip_op(&mut self, m: &Map<String, Value>) -> BridgeResult {
        match str_of(m, "action")? {
            "new" => self.anim_new_clip(),
            "duplicate" => {
                let id = self.anim_clip_ref(m)?;
                self.anim_duplicate_clip(&id);
            }
            "select" => {
                let id = self.anim_clip_ref(m)?;
                self.anim_select_clip(&id);
            }
            "rename" => {
                let id = self.anim_clip_ref(m)?;
                let name = str_of(m, "name")?.trim().to_string();
                if name.is_empty() || name.len() > 128 {
                    return Err(BridgeError::invalid_params("'name' deve ter 1..128 caracteres"));
                }
                if !self.anim_apply(AnimEdit::RenameClip { clip_id: id, name }) {
                    return Err(BridgeError::failed(self.anim_ui.message.clone()));
                }
            }
            "remove" => {
                let id = self.anim_clip_ref(m)?;
                if !self.anim_apply(AnimEdit::RemoveClip { clip_id: id }) {
                    return Err(BridgeError::failed(self.anim_ui.message.clone()));
                }
            }
            "playback" => {
                let id = self.anim_clip_ref(m)?;
                let loop_mode = m.get("loop_mode").map(|v| enum_of::<LoopMode>(v, "loop_mode", "once|loop|pingpong")).transpose()?;
                // aceita "pingpong" além do "ping_pong" do serde
                let loop_mode = match (loop_mode, m.get("loop_mode").and_then(Value::as_str)) {
                    (None, Some("pingpong")) => Some(LoopMode::PingPong),
                    (l, _) => l,
                };
                let extrapolation = m.get("extrapolation").map(|v| enum_of::<Extrapolation>(v, "extrapolation", "constant|linear")).transpose()?;
                let hold_step = m.get("hold_step").map(|v| v.as_u64().filter(|n| (1..=12).contains(n)).map(|n| n as u8).ok_or_else(|| BridgeError::invalid_params("hold_step: 1..12"))).transpose()?;
                if loop_mode.is_none() && extrapolation.is_none() && hold_step.is_none() {
                    return Err(BridgeError::invalid_params("playback exige loop_mode, extrapolation e/ou hold_step"));
                }
                if !self.anim_apply(AnimEdit::SetClipPlayback { clip_id: id, loop_mode, extrapolation, hold_step }) {
                    return Err(BridgeError::failed(self.anim_ui.message.clone()));
                }
            }
            "range" => {
                let id = self.anim_clip_ref(m)?;
                let range = match m.get("range") {
                    None | Some(Value::Null) => None,
                    Some(v) => {
                        let a = v.as_array().filter(|a| a.len() == 2).ok_or_else(|| BridgeError::invalid_params("range: [início, fim] ou null"))?;
                        let g = |x: &Value| x.as_i64().filter(|n| (0..=100_000).contains(n)).map(|n| n as i32).ok_or_else(|| BridgeError::invalid_params("range: inteiros 0..100000"));
                        Some([g(&a[0])?, g(&a[1])?])
                    }
                };
                if !self.anim_apply(AnimEdit::SetClipRange { clip_id: id, range }) {
                    return Err(BridgeError::failed(self.anim_ui.message.clone()));
                }
            }
            "fps" => {
                let fps = f32_of(m, "fps")?;
                let retime = m.get("retime").and_then(Value::as_bool).unwrap_or(false);
                if !self.anim_apply(AnimEdit::SetFps { fps, retime }) {
                    return Err(BridgeError::failed(self.anim_ui.message.clone()));
                }
            }
            other => return Err(BridgeError::invalid_params(format!("clip.action '{other}' desconhecida (new, duplicate, select, rename, remove, playback, range, fps)"))),
        }
        Ok(self.anim_info(&Map::new()))
    }

    fn anim_transport_op(&mut self, m: &Map<String, Value>) -> BridgeResult {
        let span = self.anim_span();
        match str_of(m, "action")? {
            "goto" => {
                let f = f32_of(m, "frame")?;
                if !(0.0..=100_000.0).contains(&f) {
                    return Err(BridgeError::invalid_params("frame deve estar em 0..100000"));
                }
                self.scrub_to(f, false);
            }
            "step" => {
                let n = m.get("n").and_then(Value::as_i64).filter(|n| n.abs() <= 10_000).ok_or_else(|| BridgeError::invalid_params("step exige 'n' (inteiro, + avança / - volta)"))? as i32;
                self.anim_ui.transport.step(n, span);
            }
            "play" => {
                if !self.anim_ui.transport.playing {
                    self.anim_ui.transport.toggle(span);
                }
            }
            "pause" => {
                if self.anim_ui.transport.playing {
                    self.anim_ui.transport.toggle(span);
                }
            }
            "home" => self.anim_ui.transport.home(span),
            "end" => self.anim_ui.transport.end(span),
            "speed" => {
                let s = f32_of(m, "value")?;
                if !(-4.0..=4.0).contains(&s) || s == 0.0 {
                    return Err(BridgeError::invalid_params("value deve estar em -4..4 (diferente de 0)"));
                }
                self.anim_ui.transport.set_speed(s);
            }
            "next_key" | "prev_key" => {
                let forward = m.get("action").and_then(Value::as_str) == Some("next_key");
                let Some(clip) = self.anim_clip().cloned() else { return Err(BridgeError::failed("não há clip ativo")) };
                if !self.anim_ui.transport.jump_key(&clip, span, forward) {
                    return Err(BridgeError::failed("não há chave nessa direção"));
                }
            }
            other => return Err(BridgeError::invalid_params(format!("transport.action '{other}' desconhecida (goto, step, play, pause, home, end, speed, next_key, prev_key)"))),
        }
        self.anim_ui.phys.last_raw = None;
        Ok(self.anim_info(&Map::new()))
    }

    fn anim_key_op(&mut self, m: &Map<String, Value>) -> BridgeResult {
        match str_of(m, "action")? {
            "autokey" => {
                self.anim_ui.autokey = m.get("on").and_then(Value::as_bool).ok_or_else(|| BridgeError::invalid_params("autokey exige 'on' (true|false)"))?;
            }
            action @ ("insert" | "delete") => {
                if let Some(f) = m.get("frame") {
                    let f = f.as_f64().filter(|f| (0.0..=100_000.0).contains(f)).ok_or_else(|| BridgeError::invalid_params("frame: 0..100000"))?;
                    self.scrub_to(f as f32, false);
                }
                // `bones` escolhe os canais (como selecionar ossos no viewport); a seleção anterior volta depois
                let saved = self.pose_ui.selection.clone();
                if let Some(b) = m.get("bones") {
                    self.pose_ui.selection = bones_of(b)?.into_iter().collect();
                }
                let before = self.core.history().undo_depth();
                if action == "insert" {
                    self.anim_insert_key();
                } else {
                    self.anim_delete_key_at_frame();
                }
                self.pose_ui.selection = saved;
                if self.core.history().undo_depth() == before {
                    return Err(BridgeError::failed(format!("nada mudou: {}", self.anim_ui.message)));
                }
            }
            other => return Err(BridgeError::invalid_params(format!("key.action '{other}' desconhecida (insert, delete, autokey)"))),
        }
        Ok(self.anim_info(&Map::new()))
    }
}
