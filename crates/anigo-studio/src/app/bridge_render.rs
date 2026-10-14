//! Fila do Render pela bridge (`RENDER`): o painel Render sem janela — preferências de saída (pasta, template, contador, guias),
//! câmera de render (câmera de cena, da vista, enquadrar), a fila (enfileirar, cancelar, retomar, limpar), o histórico de imagens
//! (caminho, enviar à Biblioteca) e o ffmpeg. Usa as MESMAS funções dos botões (`render_enqueue`, `render_cancel`, `render_resume`…),
//! então a validação e o histórico do núcleo valem. Configurações do projeto (resolução, passes, AOVs, lente) seguem em
//! `APPLY_COMMAND` (`SetRenderOutput`, `SetRenderSettings`, `SetCamera`).

use super::bridge_model::reject_unknown_keys;
use super::render_ui::{detect_ffmpeg, dir_status, ffmpeg_command, DirStatus, JobStatus, RenderGuides};
use super::StudioApp;
use crate::render_out::framing::SENSORS;
use crate::render_out::job::Stage;
use crate::render_out::settings::{self, NameTokens};
use anigo_bridge::protocol::{BridgeError, BridgeResult};
use serde_json::{json, Map, Value};
use std::path::Path;

fn str_of<'a>(m: &'a Map<String, Value>, k: &str) -> Result<&'a str, BridgeError> {
    m.get(k).and_then(Value::as_str).ok_or_else(|| BridgeError::invalid_params(format!("'{k}' é obrigatório e deve ser texto")))
}

fn job_id_of(m: &Map<String, Value>) -> Result<u64, BridgeError> {
    m.get("job_id").and_then(Value::as_u64).ok_or_else(|| BridgeError::invalid_params("'job_id' é obrigatório (inteiro; veja info.jobs)"))
}

fn stage_name(s: Stage) -> &'static str {
    match s {
        Stage::Prepare => "prepare",
        Stage::Render => "render",
        Stage::Outline => "outline",
        Stage::Reduce => "reduce",
        Stage::Aov => "aov",
        Stage::Encode => "encode",
        Stage::Write => "write",
    }
}

fn status_name(s: &JobStatus) -> (&'static str, Option<String>) {
    match s {
        JobStatus::Queued => ("queued", None),
        JobStatus::Running => ("running", None),
        JobStatus::Done => ("done", None),
        JobStatus::Failed(e) => ("failed", Some(e.clone())),
        JobStatus::Cancelled => ("cancelled", None),
    }
}

fn dir_status_name(d: &DirStatus) -> &'static str {
    match d {
        DirStatus::Ready => "ready",
        DirStatus::WillCreate => "will_create",
        DirStatus::NotADirectory => "not_a_directory",
        DirStatus::ReadOnly => "read_only",
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

/// Valida o texto de uma pasta de saída: absoluta, sem `..`, sem NUL. (O sandbox de pastas do MCP fica no `anigo-mcp`.)
fn check_dir_text(d: &str) -> Result<(), BridgeError> {
    if d.len() > 1024 || d.contains('\0') {
        return Err(BridgeError::invalid_params("'dir' inválido (máx. 1024 caracteres, sem NUL)"));
    }
    if d.trim().is_empty() {
        return Ok(()); // vazio = pasta sugerida
    }
    let p = Path::new(d.trim());
    if !p.is_absolute() {
        return Err(BridgeError::invalid_params("'dir' deve ser um caminho absoluto (ou vazio para a pasta sugerida)"));
    }
    if p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(BridgeError::invalid_params("'dir' não pode conter '..'"));
    }
    if matches!(dir_status(p), DirStatus::NotADirectory) {
        return Err(BridgeError::invalid_params("'dir' aponta para um arquivo, não uma pasta"));
    }
    Ok(())
}

fn check_template(t: &str) -> Result<(), BridgeError> {
    if t.trim().is_empty() || t.chars().count() > 200 {
        return Err(BridgeError::invalid_params("'template' deve ter 1..200 caracteres"));
    }
    // ':' só vale dentro de {token:04}; fora das chaves é caractere reservado do Windows
    let mut depth = 0u32;
    for c in t.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ':' if depth == 0 => return Err(BridgeError::invalid_params("'template' não pode ter ':' fora de um token como {n:04}")),
            _ => {}
        }
        if matches!(c, '/' | '\\' | '\0') || c.is_control() {
            return Err(BridgeError::invalid_params("'template' não pode ter separadores de pasta nem caracteres de controle"));
        }
    }
    Ok(())
}

impl StudioApp {
    /// `None` = adiado (ativos ainda carregando para enfileirar; Biblioteca abrindo para enviar imagem).
    pub(crate) fn bridge_render(&mut self, ctx: &egui::Context, params: &Value) -> Option<BridgeResult> {
        let m = match params.as_object() {
            Some(m) => m,
            None => return Some(Err(BridgeError::invalid_params("params deve ser um objeto"))),
        };
        let op = match str_of(m, "op") {
            Ok(o) => o,
            Err(e) => return Some(Err(e)),
        };
        self.render_poll(ctx);
        match op {
            "info" => Some(Ok(self.render_info(m))),
            "prefs" => Some(self.render_set_prefs(m).map(|_| self.render_info(&Map::new()))),
            "camera" => Some(self.render_camera_op(m)),
            "enqueue" => self.bridge_render_frame(ctx),
            "job" => Some(self.render_job_op(ctx, m)),
            "history" => self.render_history_op(ctx, m),
            "ffmpeg" => {
                self.render_ui.ffmpeg = Some(detect_ffmpeg());
                Some(Ok(self.render_info(&Map::new())))
            }
            other => Some(Err(BridgeError::invalid_params(format!("RENDER op '{other}' desconhecida (info, prefs, camera, enqueue, job, history, ffmpeg)")))),
        }
    }

    fn render_info(&mut self, m: &Map<String, Value>) -> Value {
        let limit = m.get("limit").and_then(Value::as_u64).unwrap_or(48).clamp(1, 500) as usize;
        let dir = self.render_ui.out_dir();
        let out = self.core.project().render.output;
        let (tw, th) = out.target_size();
        let tokens = NameTokens { project: self.core.project().name.clone(), camera: self.render_camera_name(), w: tw, h: th, fmt: out.format.extension().to_string(), ..Default::default() };
        let aovs = self.render_aovs_enabled();
        let alloc = settings::allocate(&dir, &self.render_ui.prefs.template, &tokens, out.format.extension(), &aovs, self.render_ui.prefs.overwrite, self.render_ui.prefs.counter);
        let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let p = &self.render_ui.prefs;
        let cams: Vec<Value> = self.render_scene_cameras().into_iter().map(|(id, n)| json!({ "node_id": id.to_string(), "name": n })).collect();
        let cam = self.core.project().scene.camera.camera.clone();
        let rs = &self.core.project().render;
        let jobs: Vec<Value> = self
            .render_ui
            .jobs
            .iter()
            .map(|j| {
                let (status, error) = status_name(&j.status);
                json!({
                    "job_id": j.id, "label": j.label, "status": status, "error": error, "progress": j.frac,
                    "stage": j.stage.map(stage_name), "frame_idx": j.frame_idx, "total": j.total, "render_ms": j.render_ms,
                    "elapsed_s": j.finished.map(|d| d.as_secs_f32()),
                    "files": j.files.iter().map(|f| json!({ "pass": f.pass, "path": f.path.display().to_string(), "width": f.w, "height": f.h, "bytes": f.bytes })).collect::<Vec<_>>(),
                    "notes": j.notes,
                })
            })
            .collect();
        let history: Vec<Value> = self
            .render_ui
            .history
            .iter()
            .filter(|e| e.pass == "beauty")
            .take(limit)
            .map(|e| json!({ "file": e.file, "pass": e.pass, "width": e.w, "height": e.h, "bytes": e.bytes, "project": e.project, "camera": e.camera, "when": e.when, "frame": e.frame }))
            .collect();
        json!({
            "workspace": self.ws.key(),
            "prefs": {
                "dir": p.dir, "template": p.template, "counter": p.counter, "overwrite": p.overwrite, "lock_aspect": p.lock_aspect,
                "sensor": p.sensor, "sensor_name": SENSORS[p.sensor.min(SENSORS.len() - 1)].0, "queue_open": p.queue_open,
                "guides": serde_json::to_value(p.guides).unwrap_or(Value::Null),
            },
            "sensors": SENSORS.iter().map(|s| s.0).collect::<Vec<_>>(),
            "out_dir": dir.display().to_string(),
            "dir_status": dir_status_name(&dir_status(&dir)),
            "next_files": { "main": name(&alloc.main), "aovs": alloc.aovs.iter().map(|(k, p)| json!({ "pass": k, "file": name(p) })).collect::<Vec<_>>() },
            "output": serde_json::to_value(out).unwrap_or(Value::Null),
            "aovs_enabled": aovs,
            "passes": { "graph_order": rs.graph_order, "graph_disabled": rs.graph_disabled, "depth_prepass": rs.depth_prepass },
            "camera": {
                "name": self.render_camera_name(), "linked_node": self.render_ui.linked.as_ref().map(|i| i.to_string()), "scene_cameras": cams,
                "eye": [cam.eye.x, cam.eye.y, cam.eye.z], "target": [cam.target.x, cam.target.y, cam.target.z], "fov_deg": cam.fov_y.to_degrees(),
            },
            "running": self.render_ui.running(),
            "jobs": jobs,
            "history": history,
            "ffmpeg": match &self.render_ui.ffmpeg { None => Value::Null, Some(Ok(v)) => json!({ "found": true, "version": v }), Some(Err(e)) => json!({ "found": false, "error": e }) },
            "ffmpeg_command": ffmpeg_command(&dir, &settings::sanitize_file_name(&self.core.project().name), 24),
            "message": self.render_ui.message,
        })
    }

    /// Patch parcial das preferências (todo-ou-nada: um campo inválido não muda nenhum).
    fn render_set_prefs(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let patch = m.get("patch").ok_or_else(|| BridgeError::invalid_params("'patch' é obrigatório (dir, template, counter, overwrite, lock_aspect, sensor, guides, queue_open)"))?;
        let Some(po) = patch.as_object() else { return Err(BridgeError::invalid_params("'patch' deve ser um objeto")) };
        let mut next = self.render_ui.prefs.clone();
        for (k, v) in po {
            let want_bool = |n: &str| v.as_bool().ok_or_else(|| BridgeError::invalid_params(format!("'{n}' deve ser booleano")));
            match k.as_str() {
                "dir" => {
                    let s = v.as_str().ok_or_else(|| BridgeError::invalid_params("'dir' deve ser texto"))?;
                    check_dir_text(s)?;
                    next.dir = if s.trim().is_empty() { settings::suggested_output_dir().display().to_string() } else { s.trim().to_string() };
                }
                "template" => {
                    let s = v.as_str().ok_or_else(|| BridgeError::invalid_params("'template' deve ser texto"))?;
                    check_template(s)?;
                    next.template = s.to_string();
                }
                "counter" => {
                    let n = v.as_u64().filter(|n| (1..=999_999).contains(n)).ok_or_else(|| BridgeError::invalid_params("'counter' deve ser inteiro em 1..999999"))?;
                    next.counter = n as u32;
                }
                "overwrite" => next.overwrite = want_bool("overwrite")?,
                "lock_aspect" => next.lock_aspect = want_bool("lock_aspect")?,
                "queue_open" => next.queue_open = want_bool("queue_open")?,
                "sensor" => {
                    let n = v.as_u64().filter(|n| (*n as usize) < SENSORS.len()).ok_or_else(|| BridgeError::invalid_params(format!("'sensor' deve ser um índice 0..{} (veja info.sensors)", SENSORS.len() - 1)))?;
                    next.sensor = n as usize;
                }
                "guides" => {
                    let mut g = serde_json::to_value(next.guides).map_err(|e| BridgeError::failed(e.to_string()))?;
                    reject_unknown_keys(&g, v, "guides")?;
                    merge(&mut g, v);
                    next.guides = serde_json::from_value::<RenderGuides>(g).map_err(|e| BridgeError::invalid_params(format!("guides inválido: {e}")))?;
                }
                other => {
                    return Err(BridgeError::invalid_params(format!("campo desconhecido 'patch.{other}'; disponíveis: counter, dir, guides, lock_aspect, overwrite, queue_open, sensor, template")));
                }
            }
        }
        self.render_ui.prefs = next;
        self.render_ui.history_dir = None; // pasta pode ter mudado: recarrega o histórico
        Ok(())
    }

    fn render_camera_op(&mut self, m: &Map<String, Value>) -> BridgeResult {
        self.scene_refresh();
        let depth = self.core.history().undo_depth();
        match str_of(m, "action")? {
            "source" => {
                if m.get("project").and_then(Value::as_bool).unwrap_or(false) {
                    self.render_ui.linked = None;
                } else {
                    let want = str_of(m, "node_id")?;
                    let id = self.render_scene_cameras().into_iter().find(|(i, _)| i.to_string() == want).map(|(i, _)| i).ok_or_else(|| BridgeError::invalid_params(format!("câmera de cena '{want}' não existe (veja info.camera.scene_cameras); use project=true para a câmera do projeto")))?;
                    self.render_ui.linked = Some(id.clone());
                    self.render_apply_scene_camera(&id);
                }
            }
            "reapply" => {
                let id = self.render_ui.linked.clone().ok_or_else(|| BridgeError::failed("não há câmera de cena vinculada (use action=source)"))?;
                self.render_apply_scene_camera(&id);
            }
            "from_view" => {
                let c = self.scene_ui.camera;
                let cmd = anigo_core::command::Command::SetCamera { eye: Some(c.eye().to_array()), target: Some(c.target.to_array()), up: Some([0.0, 1.0, 0.0]), fov_degrees: Some(c.fov_y.to_degrees()), projection: None };
                self.render_apply(cmd);
            }
            "frame" => {
                let selection = m.get("selection").and_then(Value::as_bool).unwrap_or(false);
                if selection && self.scene_ui.selection.is_empty() {
                    return Err(BridgeError::failed("não há seleção na cena para enquadrar"));
                }
                self.render_frame(selection);
            }
            other => return Err(BridgeError::invalid_params(format!("camera.action '{other}' desconhecida (source, reapply, from_view, frame)"))),
        }
        let mut out = self.render_info(&Map::new());
        out["history_changed"] = json!(self.core.history().undo_depth() != depth);
        Ok(out)
    }

    fn render_job_op(&mut self, ctx: &egui::Context, m: &Map<String, Value>) -> BridgeResult {
        let action = str_of(m, "action")?;
        let find = |app: &StudioApp, id: u64| app.render_ui.jobs.iter().position(|j| j.id == id).ok_or_else(|| BridgeError::invalid_params(format!("job {id} desconhecido")));
        match action {
            "cancel" => {
                let id = job_id_of(m)?;
                let i = find(self, id)?;
                if !matches!(self.render_ui.jobs[i].status, JobStatus::Queued | JobStatus::Running) {
                    return Err(BridgeError::failed(format!("job {id} já terminou ({})", status_name(&self.render_ui.jobs[i].status).0)));
                }
                self.render_cancel(id);
            }
            "resume" => {
                let id = job_id_of(m)?;
                let i = find(self, id)?;
                if !matches!(self.render_ui.jobs[i].status, JobStatus::Failed(_) | JobStatus::Cancelled) {
                    return Err(BridgeError::failed(format!("só dá para retomar jobs cancelados ou com falha (job {id} está {})", status_name(&self.render_ui.jobs[i].status).0)));
                }
                self.render_resume(ctx, id);
            }
            "remove" => {
                let id = job_id_of(m)?;
                let i = find(self, id)?;
                if matches!(self.render_ui.jobs[i].status, JobStatus::Queued | JobStatus::Running) {
                    return Err(BridgeError::failed("cancele o job antes de remover"));
                }
                self.render_ui.jobs.remove(i);
            }
            "clear" => self.render_ui.jobs.retain(|j| matches!(j.status, JobStatus::Queued | JobStatus::Running)),
            other => return Err(BridgeError::invalid_params(format!("job.action '{other}' desconhecida (cancel, resume, remove, clear)"))),
        }
        Ok(self.render_info(&Map::new()))
    }

    fn render_history_op(&mut self, ctx: &egui::Context, m: &Map<String, Value>) -> Option<BridgeResult> {
        let action = match str_of(m, "action") {
            Ok(a) => a,
            Err(e) => return Some(Err(e)),
        };
        let entry = |app: &StudioApp| -> Result<crate::render_out::history::HistoryEntry, BridgeError> {
            let f = str_of(m, "file")?;
            app.render_ui.history.iter().find(|e| e.file == f).cloned().ok_or_else(|| BridgeError::invalid_params(format!("'{f}' não está no histórico da pasta de saída (veja info.history)")))
        };
        match action {
            "list" => Some(Ok(self.render_info(m))),
            "path" => Some(entry(self).map(|e| json!({ "file": e.file, "path": e.path(&self.render_ui.out_dir()).display().to_string(), "exists": e.path(&self.render_ui.out_dir()).is_file() }))),
            "to_library" => {
                let e = match entry(self) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                if !e.path(&self.render_ui.out_dir()).is_file() {
                    return Some(Err(BridgeError::failed("o arquivo da imagem não existe mais no disco")));
                }
                self.bridge_library(ctx)?; // adia enquanto a Biblioteca abre
                self.render_to_library(&e);
                Some(Ok(json!({ "file": e.file, "message": self.render_ui.message })))
            }
            other => Some(Err(BridgeError::invalid_params(format!("history.action '{other}' desconhecida (list, path, to_library)")))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_text_must_be_absolute_without_parent_components() {
        assert!(check_dir_text("").is_ok());
        assert!(check_dir_text("relativo/pasta").is_err());
        assert!(check_dir_text("a\0b").is_err());
        let tmp = std::env::temp_dir();
        assert!(check_dir_text(&tmp.display().to_string()).is_ok());
        let up = tmp.join("..").join("x");
        assert!(check_dir_text(&up.display().to_string()).is_err());
        let f = tmp.join(format!("anigo_dir_text_{}.txt", std::process::id()));
        std::fs::write(&f, b"x").unwrap();
        assert!(check_dir_text(&f.display().to_string()).is_err(), "arquivo não é pasta");
        let _ = std::fs::remove_file(f);
    }

    #[test]
    fn template_rejects_path_separators() {
        assert!(check_template("{project}_{n:04}").is_ok());
        assert!(check_template("a/b").is_err());
        assert!(check_template("a\\b").is_err());
        assert!(check_template("a:b").is_err());
        assert!(check_template("%04d_{frame:04}").is_ok());
        assert!(check_template("   ").is_err());
        assert!(check_template(&"x".repeat(201)).is_err());
    }

    #[test]
    fn stage_and_status_names_are_stable() {
        assert_eq!(stage_name(Stage::Aov), "aov");
        assert_eq!(status_name(&JobStatus::Failed("x".into())), ("failed", Some("x".into())));
        assert_eq!(dir_status_name(&DirStatus::WillCreate), "will_create");
    }
}
