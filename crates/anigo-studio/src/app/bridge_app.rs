//! Aplicativo pela bridge (`APP`): o que os menus Editar/Exibir/Janela/Ajuda fazem — idioma, desfazer/refazer do WORKSPACE ATIVO
//! (núcleo, Modelagem ou Shading, cada um com seu histórico), mapa de atalhos (ver, trocar, restaurar, importar/exportar),
//! layouts do dock (restaurar, presets, importar/exportar), enquadrar e exportar o objeto ativo para a Biblioteca.
//! Mesmas funções dos menus. Caminhos de arquivo passam pelo sandbox do `anigo-mcp` antes de chegar aqui.

use super::dock_ui::DockCmd;
use super::StudioApp;
use crate::i18n::{self, Lang};
use crate::keymap::{actions::ACTIONS, Chord, Keymap};
use anigo_bridge::protocol::{BridgeError, BridgeResult};
use serde_json::{json, Map, Value};

fn bad(msg: impl Into<String>) -> BridgeError {
    BridgeError::invalid_params(msg)
}

fn str_of<'a>(m: &'a Map<String, Value>, k: &str) -> Result<&'a str, BridgeError> {
    m.get(k).and_then(Value::as_str).ok_or_else(|| bad(format!("'{k}' é obrigatório e deve ser texto")))
}

fn abs_path(m: &Map<String, Value>) -> Result<std::path::PathBuf, BridgeError> {
    let p = std::path::PathBuf::from(str_of(m, "path")?);
    if !p.is_absolute() || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(bad("'path' deve ser um caminho absoluto sem '..'"));
    }
    Ok(p)
}

fn preset_name(m: &Map<String, Value>) -> Result<String, BridgeError> {
    let n = str_of(m, "name")?.trim().to_string();
    if n.is_empty() || n.chars().count() > 60 || n.contains('\0') {
        return Err(bad("'name' deve ter 1..60 caracteres"));
    }
    Ok(n)
}

impl StudioApp {
    pub(crate) fn bridge_app(&mut self, params: &Value) -> BridgeResult {
        let m = params.as_object().ok_or_else(|| bad("params deve ser um objeto"))?;
        match str_of(m, "op")? {
            "info" => Ok(self.app_info()),
            "lang" => {
                let l = match str_of(m, "value")? {
                    "pt" | "pt-br" | "pt_br" => Lang::PtBr,
                    "en" => Lang::En,
                    _ => return Err(bad("'value' deve ser pt|en")),
                };
                i18n::set_lang(l);
                Ok(self.app_info())
            }
            "history" => self.app_history_op(m),
            "keymap" => self.app_keymap_op(m),
            "layout" => self.app_layout_op(m),
            "frame" => {
                if self.ws == crate::workspace::WorkspaceId::Cenario {
                    self.scene_refresh();
                    self.scene_focus();
                } else if self.ws == crate::workspace::WorkspaceId::Modelagem {
                    self.frame_selected_or_all();
                } else {
                    return Err(BridgeError::failed("enquadrar vale na Modelagem e no Cenário (nos outros, use o preset da câmera do workspace)"));
                }
                Ok(self.app_info())
            }
            "export_active_to_library" => {
                if self.ws != crate::workspace::WorkspaceId::Modelagem {
                    self.switch_workspace(crate::workspace::WorkspaceId::Modelagem);
                }
                if self.modeling.active_object().is_none() {
                    return Err(BridgeError::failed("não há objeto ativo na Modelagem"));
                }
                self.export_active_to_library();
                let st = self.modeling.status.clone();
                Ok(json!({ "status": st, "note": "gravado na Biblioteca configurada (use ANIGO_LIBRARY_DIR para isolar em testes)" }))
            }
            other => Err(bad(format!("APP op '{other}' desconhecida (info, lang, history, keymap, layout, frame, export_active_to_library)"))),
        }
    }

    fn app_info(&self) -> Value {
        let (can_undo, can_redo) = self.can_undo_redo();
        let (adapter, backend) = self.gpu.as_ref().map(|g| (g.adapter_name.clone(), g.backend.clone())).unwrap_or_default();
        json!({
            "workspace": self.ws.key(),
            "lang": match i18n::lang() { Lang::PtBr => "pt", Lang::En => "en" },
            "history": {
                "can_undo": can_undo, "can_redo": can_redo,
                "scope": match self.ws { crate::workspace::WorkspaceId::Modelagem => "modelagem", crate::workspace::WorkspaceId::Shading => "shading", _ => "core" },
                "core_undo": self.core.history().undo_descriptions().iter().rev().take(20).map(|s| s.to_string()).collect::<Vec<_>>(),
                "core_redo": self.core.history().redo_descriptions().iter().take(20).map(|s| s.to_string()).collect::<Vec<_>>(),
                "modeling": self.modeling.history_labels().iter().rev().take(20).collect::<Vec<_>>(),
            },
            "keymap": { "path": self.keymap_path.as_ref().map(|p| p.display().to_string()), "modified": self.keymap.modified_count(), "conflicts": self.keymap.conflicts().len() },
            "layout": { "presets": self.dock.store.presets_for(self.ws.key()).map(|p| p.name.clone()).collect::<Vec<_>>(), "pending_commands": self.dock.cmds.len() },
            "gpu": { "adapter": adapter, "backend": backend },
        })
    }

    /// Desfazer/refazer do workspace ativo (as MESMAS rotas do menu Editar), 1..100 passos.
    fn app_history_op(&mut self, m: &Map<String, Value>) -> BridgeResult {
        let undo = match str_of(m, "action")? {
            "undo" => true,
            "redo" => false,
            "list" => return Ok(self.app_info()),
            o => return Err(bad(format!("history.action '{o}' desconhecida (undo, redo, list)"))),
        };
        let steps = match m.get("steps") {
            None | Some(Value::Null) => 1,
            Some(v) => v.as_u64().filter(|n| (1..=100).contains(n)).ok_or_else(|| bad("'steps' deve ser um inteiro em 1..100"))?,
        };
        self.bridge_reset_gestures();
        let mut done = 0;
        for _ in 0..steps {
            let (cu, cr) = self.can_undo_redo();
            if (undo && !cu) || (!undo && !cr) {
                break;
            }
            self.undo_redo(undo);
            done += 1;
        }
        if done == 0 {
            return Err(BridgeError::failed(if undo { "nada para desfazer neste workspace" } else { "nada para refazer neste workspace" }));
        }
        let mut out = self.app_info();
        out["steps_done"] = json!(done);
        Ok(out)
    }

    // ------------------------------------------------------------------------------------------------ atalhos

    fn app_keymap_op(&mut self, m: &Map<String, Value>) -> BridgeResult {
        let chords_of = |v: &Value| -> Result<Vec<Chord>, BridgeError> {
            let a = v.as_array().filter(|a| a.len() <= 4).ok_or_else(|| bad("'chords' deve ser uma lista de até 4 atalhos (ex.: [\"Ctrl+Shift+Z\"])"))?;
            a.iter().map(|c| Chord::parse(c.as_str().ok_or_else(|| bad("atalho deve ser texto"))?).map_err(|e| bad(format!("atalho inválido: {e}")))).collect()
        };
        let known = |id: &str| -> Result<(), BridgeError> {
            if ACTIONS.iter().any(|d| d.id == id) {
                Ok(())
            } else {
                Err(bad(format!("ação '{id}' desconhecida (veja keymap action=list)")))
            }
        };
        let mut changed = false;
        match str_of(m, "action")? {
            "list" => {
                let q = m.get("query").and_then(Value::as_str).unwrap_or("").to_lowercase();
                let only_mod = m.get("only_modified").and_then(Value::as_bool).unwrap_or(false);
                let list: Vec<Value> = ACTIONS
                    .iter()
                    .filter(|d| (q.is_empty() || d.id.contains(&q)) && (!only_mod || self.keymap.is_modified(d.id)))
                    .map(|d| json!({ "id": d.id, "label": self.keymap.label(d.id), "chords": self.keymap.chords(d.id).iter().map(|c| c.to_text()).collect::<Vec<_>>(), "defaults": d.defaults, "modified": self.keymap.is_modified(d.id) }))
                    .collect();
                return Ok(json!({ "actions": list, "conflicts": self.keymap.conflicts().iter().map(|c| json!({ "chord": c.chord.to_text(), "a": c.a, "b": c.b })).collect::<Vec<_>>() }));
            }
            "set" => {
                let id = str_of(m, "id")?;
                known(id)?;
                let cs = chords_of(m.get("chords").ok_or_else(|| bad("'chords' é obrigatório"))?)?;
                Keymap::set(std::sync::Arc::make_mut(&mut self.keymap), id, cs);
                changed = true;
            }
            "add" => {
                let id = str_of(m, "id")?;
                known(id)?;
                let c = Chord::parse(str_of(m, "chord")?).map_err(|e| bad(format!("atalho inválido: {e}")))?;
                std::sync::Arc::make_mut(&mut self.keymap).add(id, c);
                changed = true;
            }
            "reset" => {
                let id = str_of(m, "id")?;
                known(id)?;
                std::sync::Arc::make_mut(&mut self.keymap).reset(id);
                changed = true;
            }
            "reset_all" => {
                std::sync::Arc::make_mut(&mut self.keymap).reset_all();
                changed = true;
            }
            "export" => {
                let p = abs_path(m)?;
                if let Some(d) = p.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                self.keymap.save_to(&p, m.get("all").and_then(Value::as_bool).unwrap_or(false)).map_err(|e| BridgeError::failed(e.to_string()))?;
            }
            "import" => {
                let p = abs_path(m)?;
                let raw = std::fs::read_to_string(&p).map_err(|e| BridgeError::failed(format!("{}: {e}", p.display())))?;
                if raw.len() > 1024 * 1024 {
                    return Err(bad("arquivo de atalhos grande demais (máx. 1 MB)"));
                }
                let (km, warns) = Keymap::from_json(&raw).map_err(|e| BridgeError::failed(e.to_string()))?;
                self.keymap = std::sync::Arc::new(km);
                let mut out = self.app_info();
                out["warnings"] = json!(warns);
                self.keymap_save();
                return Ok(out);
            }
            o => return Err(bad(format!("keymap.action '{o}' desconhecida (list, set, add, reset, reset_all, export, import)"))),
        }
        if changed {
            self.keymap_save();
        }
        Ok(self.app_info())
    }

    // ------------------------------------------------------------------------------------------------ layouts do dock

    /// Os comandos de layout rodam no começo do próximo quadro do dock: a resposta diz `queued` e o efeito aparece no `info` seguinte.
    fn app_layout_op(&mut self, m: &Map<String, Value>) -> BridgeResult {
        let cmd = match str_of(m, "action")? {
            "reset_current" => DockCmd::ResetCurrent,
            "reset_all" => DockCmd::ResetAll,
            "save_preset" => DockCmd::SavePreset(preset_name(m)?),
            "apply_preset" => {
                let n = preset_name(m)?;
                if !self.dock.store.presets_for(self.ws.key()).any(|p| p.name == n) {
                    return Err(bad(format!("preset '{n}' não existe neste workspace (veja info.layout.presets)")));
                }
                DockCmd::ApplyPreset(n)
            }
            "delete_preset" => {
                let n = preset_name(m)?;
                if !self.dock.store.presets_for(self.ws.key()).any(|p| p.name == n) {
                    return Err(bad(format!("preset '{n}' não existe neste workspace (veja info.layout.presets)")));
                }
                DockCmd::DeletePreset(n)
            }
            "export" => DockCmd::Export(abs_path(m)?),
            "import" => DockCmd::Import(abs_path(m)?),
            o => return Err(bad(format!("layout.action '{o}' desconhecida (reset_current, reset_all, save_preset, apply_preset, delete_preset, export, import)"))),
        };
        self.dock.cmds.push(cmd);
        let mut out = self.app_info();
        out["queued"] = json!(true);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_names_are_validated() {
        let mut m = Map::new();
        m.insert("path".into(), json!("relativo.json"));
        assert!(abs_path(&m).is_err());
        let tmp = std::env::temp_dir().join("x.json");
        m.insert("path".into(), json!(tmp.display().to_string()));
        assert!(abs_path(&m).is_ok());
        m.insert("path".into(), json!(tmp.join("..").join("y").display().to_string()));
        assert!(abs_path(&m).is_err());
        let mut n = Map::new();
        n.insert("name".into(), json!("  "));
        assert!(preset_name(&n).is_err());
        n.insert("name".into(), json!("Meu layout"));
        assert_eq!(preset_name(&n).unwrap(), "Meu layout");
    }

    #[test]
    fn every_action_default_is_a_valid_chord() {
        for d in ACTIONS {
            for c in d.defaults {
                assert!(Chord::parse(c).is_ok(), "{} -> {c}", d.id);
            }
        }
    }
}
