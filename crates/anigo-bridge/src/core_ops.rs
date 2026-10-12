//! Operações da bridge que só dependem da `CoreSession` (sem UI): são o MESMO caminho de `apply`/`undo`/`redo`
//! que o app usa para as ações do usuário. O studio chama [`handle`] da thread da UI; os testes de integração
//! chamam de uma thread "host" de teste.
//!
//! Ações: `GET_PROJECT`, `GET_SNAPSHOT`, `APPLY_COMMAND`, `UNDO`, `REDO`, `GET_LOG`.

use crate::protocol::BridgeError;
use anigo_core::command::{Command, CommandOutcome};
use anigo_session::CoreSession;
use serde_json::{json, Value};

/// Máximo de entradas por página de `GET_LOG`.
pub const MAX_LOG_PAGE: usize = 500;

pub const CORE_ACTIONS: &[&str] = &["GET_PROJECT", "GET_SNAPSHOT", "APPLY_COMMAND", "UNDO", "REDO", "GET_LOG"];

pub fn is_core_action(action: &str) -> bool {
    CORE_ACTIONS.contains(&action)
}

fn outcome_json(core: &CoreSession, o: &CommandOutcome) -> Value {
    json!({
        "outcome": serde_json::to_value(o).unwrap_or(Value::Null),
        "history": core.history_report(),
    })
}

fn opt_u64(params: &Value, key: &str) -> Result<Option<u64>, BridgeError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| BridgeError::invalid_params(format!("'{key}' deve ser um inteiro não negativo"))),
    }
}

fn opt_bool(params: &Value, key: &str, default: bool) -> Result<bool, BridgeError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_bool().ok_or_else(|| BridgeError::invalid_params(format!("'{key}' deve ser booleano"))),
    }
}

/// Decodifica e valida a forma de um `Command` (contrato v1: objeto com `kind`).
pub fn parse_command(v: &Value) -> Result<Command, BridgeError> {
    let obj = v.as_object().ok_or_else(|| BridgeError::invalid_params("'command' deve ser um objeto {\"kind\": ...}"))?;
    if !obj.get("kind").map(Value::is_string).unwrap_or(false) {
        return Err(BridgeError::invalid_params("'command.kind' é obrigatório (string)"));
    }
    serde_json::from_value::<Command>(v.clone()).map_err(|e| BridgeError::invalid_params(format!("command inválido: {e}")))
}

/// `None` quando a ação não é do núcleo.
pub fn handle(core: &mut CoreSession, action: &str, params: &Value) -> Option<Result<Value, BridgeError>> {
    Some(match action {
        "GET_PROJECT" => get_project(core, params),
        "GET_SNAPSHOT" => get_snapshot(core, params),
        "APPLY_COMMAND" => apply_command(core, params),
        "UNDO" => core.undo().map(|o| outcome_json(core, &o)).map_err(BridgeError::failed),
        "REDO" => core.redo().map(|o| outcome_json(core, &o)).map_err(BridgeError::failed),
        "GET_LOG" => get_log(core, params),
        _ => return None,
    })
}

fn get_project(core: &mut CoreSession, params: &Value) -> Result<Value, BridgeError> {
    let include_document = opt_bool(params, "include_document", true)?;
    let mut out = json!({ "history": core.history_report() });
    if include_document {
        let raw = core.document().map_err(BridgeError::failed)?;
        out["project"] = serde_json::from_str::<Value>(&raw).map_err(|e| BridgeError::failed(e.to_string()))?;
    }
    Ok(out)
}

fn get_snapshot(core: &mut CoreSession, params: &Value) -> Result<Value, BridgeError> {
    let include_static = opt_bool(params, "include_static", false)?;
    let client_rev = opt_u64(params, "client_static_revision")?;
    let snap = core.snapshot(include_static, client_rev).map_err(BridgeError::failed)?;
    Ok(json!({ "snapshot": serde_json::to_value(&snap).map_err(|e| BridgeError::failed(e.to_string()))?, "history": core.history_report() }))
}

fn apply_command(core: &mut CoreSession, params: &Value) -> Result<Value, BridgeError> {
    let cmd = parse_command(params.get("command").unwrap_or(&Value::Null))?;
    // controle otimista de concorrência: o cliente diz em que revisão do histórico decidiu agir
    if let Some(expected) = opt_u64(params, "if_revision")? {
        let current = core.history().revision();
        if expected != current {
            return Err(BridgeError::failed(format!("revisão desatualizada: esperada {expected}, atual {current}")));
        }
    }
    core.apply(cmd).map(|o| outcome_json(core, &o)).map_err(BridgeError::failed)
}

fn get_log(core: &mut CoreSession, params: &Value) -> Result<Value, BridgeError> {
    let log = core.history().export_log();
    let total = log.entries.len();
    if let Some(seq) = opt_u64(params, "sequence")? {
        let e = log.entries.iter().find(|e| e.sequence == seq).ok_or_else(|| BridgeError::invalid_params(format!("sequência {seq} não existe no log")))?;
        return Ok(json!({ "total": total, "entry": serde_json::to_value(e).unwrap_or(Value::Null) }));
    }
    let from = opt_u64(params, "from")?.unwrap_or(0) as usize;
    let limit = (opt_u64(params, "limit")?.unwrap_or(50) as usize).clamp(1, MAX_LOG_PAGE);
    let with_commands = opt_bool(params, "include_commands", false)?;
    let entries: Vec<Value> = log
        .entries
        .iter()
        .skip(from)
        .take(limit)
        .map(|e| {
            if with_commands {
                serde_json::to_value(e).unwrap_or(Value::Null)
            } else {
                json!({ "sequence": e.sequence, "revision": e.revision, "description": e.description, "scope": e.scope })
            }
        })
        .collect();
    let next = if from + entries.len() < total { Some(from + entries.len()) } else { None };
    Ok(json!({ "total": total, "from": from, "entries": entries, "next": next }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(v: Value) -> Value {
        json!({ "command": v })
    }

    #[test]
    fn apply_undo_redo_roundtrip_through_the_same_history() {
        let mut core = CoreSession::new();
        let before = core.project().name.clone();
        let r = handle(&mut core, "APPLY_COMMAND", &cmd(json!({"kind": "rename_project", "name": "Via bridge"}))).unwrap().unwrap();
        assert_eq!(core.project().name, "Via bridge");
        assert_eq!(r["history"]["can_undo"], true);
        assert_eq!(r["outcome"]["undo_depth"], 1);
        handle(&mut core, "UNDO", &Value::Null).unwrap().unwrap();
        assert_eq!(core.project().name, before);
        let r = handle(&mut core, "REDO", &Value::Null).unwrap().unwrap();
        assert_eq!(core.project().name, "Via bridge");
        assert_eq!(r["history"]["can_redo"], false);
        // undo sem nada a desfazer é erro explícito, não pânico
        core.undo().unwrap();
        assert!(handle(&mut core, "UNDO", &Value::Null).unwrap().is_err());
    }

    #[test]
    fn invalid_commands_are_rejected_with_invalid_params_and_leave_state_intact() {
        let mut core = CoreSession::new();
        let name = core.project().name.clone();
        let rev = core.history().revision();
        for bad in [
            Value::Null,
            json!("rename_project"),
            json!({}),
            json!({"kind": 3}),
            json!({"kind": "nao_existe"}),
            json!({"kind": "rename_project"}),
            json!({"kind": "rename_project", "name": 7}),
        ] {
            let e = handle(&mut core, "APPLY_COMMAND", &cmd(bad.clone())).unwrap().unwrap_err();
            assert_eq!(e.code, crate::protocol::code::INVALID_PARAMS, "{bad}");
        }
        // semanticamente inválido: o núcleo recusa (ação falhou), sem efeito
        let e = handle(&mut core, "APPLY_COMMAND", &cmd(json!({"kind": "set_morph_value", "target": "nao_existe_x", "value": 0.5}))).unwrap();
        assert!(e.is_err());
        assert_eq!(core.project().name, name);
        assert_eq!(core.history().revision(), rev);
        assert!(!core.history().can_undo());
    }

    #[test]
    fn if_revision_gives_optimistic_concurrency() {
        let mut core = CoreSession::new();
        let r0 = core.history().revision();
        let mut p = cmd(json!({"kind": "rename_project", "name": "A"}));
        p["if_revision"] = json!(r0);
        handle(&mut core, "APPLY_COMMAND", &p).unwrap().unwrap();
        // outro cliente decidiu com a revisão antiga
        let mut p2 = cmd(json!({"kind": "rename_project", "name": "B"}));
        p2["if_revision"] = json!(r0);
        let e = handle(&mut core, "APPLY_COMMAND", &p2).unwrap().unwrap_err();
        assert!(e.message.contains("desatualizada"), "{e}");
        assert_eq!(core.project().name, "A");
    }

    #[test]
    fn log_pages_and_single_entries() {
        let mut core = CoreSession::new();
        for n in ["a", "b", "c"] {
            handle(&mut core, "APPLY_COMMAND", &cmd(json!({"kind": "rename_project", "name": n}))).unwrap().unwrap();
        }
        core.undo().unwrap(); // o log é a sessão: undo não apaga
        let page = handle(&mut core, "GET_LOG", &json!({"limit": 2})).unwrap().unwrap();
        assert_eq!(page["total"], 3);
        assert_eq!(page["entries"].as_array().unwrap().len(), 2);
        assert_eq!(page["next"], 2);
        assert!(page["entries"][0].get("command").is_none(), "por padrão só descrições");
        let page2 = handle(&mut core, "GET_LOG", &json!({"from": 2, "limit": 10, "include_commands": true})).unwrap().unwrap();
        assert_eq!(page2["entries"].as_array().unwrap().len(), 1);
        assert_eq!(page2["entries"][0]["command"]["kind"], "rename_project");
        assert!(page2["next"].is_null());
        let seq = page["entries"][1]["sequence"].as_u64().unwrap();
        let one = handle(&mut core, "GET_LOG", &json!({"sequence": seq})).unwrap().unwrap();
        assert_eq!(one["entry"]["command"]["name"], "b");
        assert!(handle(&mut core, "GET_LOG", &json!({"sequence": 9999})).unwrap().is_err());
        assert!(handle(&mut core, "GET_LOG", &json!({"from": "x"})).unwrap().is_err());
        // limit é limitado
        let big = handle(&mut core, "GET_LOG", &json!({"limit": 1_000_000})).unwrap().unwrap();
        assert_eq!(big["entries"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn project_and_snapshot_reads_do_not_touch_history() {
        let mut core = CoreSession::new();
        let p = handle(&mut core, "GET_PROJECT", &Value::Null).unwrap().unwrap();
        assert!(p["project"]["project_id"].is_string() || p["project"].is_object());
        let lite = handle(&mut core, "GET_PROJECT", &json!({"include_document": false})).unwrap().unwrap();
        assert!(lite.get("project").is_none());
        let s = handle(&mut core, "GET_SNAPSHOT", &Value::Null).unwrap().unwrap();
        assert!(s["snapshot"].is_object());
        assert!(!core.history().can_undo());
        assert!(handle(&mut core, "GET_SNAPSHOT", &json!({"include_static": "sim"})).unwrap().is_err());
    }

    #[test]
    fn unknown_actions_are_not_core() {
        let mut core = CoreSession::new();
        assert!(handle(&mut core, "SCREENSHOT", &Value::Null).is_none());
        assert!(is_core_action("UNDO") && !is_core_action("PING"));
    }
}
