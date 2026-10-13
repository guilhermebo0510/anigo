//! Subcomandos de linha de comando do `anigo-mcp` que falam com a bridge do app nativo SEM iniciar o renderer
//! headless nem o servidor MCP (usados por scripts de CI/smoke e para depurar a conexão):
//!
//! ```text
//! anigo-mcp bridge-info
//! anigo-mcp bridge-call <ACAO> [PARAMS_JSON | @arquivo.json] [--save-png <arquivo.png>] [--timeout <s>]
//! ```
//!
//! O resultado (`data`) sai em JSON numa linha no stdout; erros vão para o stderr com código de saída 1
//! (2 = uso incorreto). `--save-png` grava o campo `png_base64` no sandbox de arquivos (`fs_sandbox`).

use crate::bridge_client::LiveBridgeClient;
use crate::fs_sandbox::{sanitize_read_path, sanitize_save_path};
use anyhow::{Context, Result};
use base64::Engine as _;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, PartialEq)]
pub enum Cmd {
    Info,
    Call { action: String, params: Value, save_png: Option<String>, timeout: Option<u64> },
}

const MAX_PARAMS_BYTES: usize = 1 << 20;

pub fn parse(args: &[String]) -> Result<Option<Cmd>, String> {
    match args.first().map(String::as_str) {
        Some("bridge-info") => Ok(Some(Cmd::Info)),
        Some("bridge-call") => {
            let mut it = args[1..].iter();
            let action = it.next().ok_or("bridge-call exige a ação (ex.: PING)")?.clone();
            if action.is_empty() || action.len() > 32 || !action.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
                return Err(format!("ação inválida {action:?} (esperado MAIUSCULAS_COM_SUBLINHADO)"));
            }
            let mut params = Value::Object(Default::default());
            let (mut save_png, mut timeout) = (None, None);
            let mut positional = false;
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--save-png" => save_png = Some(it.next().ok_or("--save-png exige um caminho")?.clone()),
                    "--timeout" => {
                        let t: u64 = it.next().ok_or("--timeout exige segundos")?.parse().map_err(|_| "--timeout precisa ser inteiro".to_string())?;
                        if !(1..=300).contains(&t) {
                            return Err("--timeout fora de 1..300".into());
                        }
                        timeout = Some(t);
                    }
                    other if other.starts_with("--") => return Err(format!("opção desconhecida: {other}")),
                    json_or_file => {
                        if positional {
                            return Err("params informado duas vezes".into());
                        }
                        positional = true;
                        params = parse_params(json_or_file)?;
                    }
                }
            }
            Ok(Some(Cmd::Call { action, params, save_png, timeout }))
        }
        _ => Ok(None),
    }
}

fn parse_params(arg: &str) -> Result<Value, String> {
    let raw = if let Some(path) = arg.strip_prefix('@') {
        let p = sanitize_read_path(path).map_err(|e| e.to_string())?;
        let meta = std::fs::metadata(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        if meta.len() as usize > MAX_PARAMS_BYTES {
            return Err("arquivo de params grande demais (> 1 MiB)".into());
        }
        std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?
    } else {
        arg.to_string()
    };
    if raw.len() > MAX_PARAMS_BYTES {
        return Err("params grande demais (> 1 MiB)".into());
    }
    let v: Value = serde_json::from_str(&raw).map_err(|e| format!("params não é JSON válido: {e}"))?;
    if !v.is_object() {
        return Err("params deve ser um objeto JSON".into());
    }
    Ok(v)
}

/// `Some(código de saída)` quando os argumentos eram de um subcomando; `None` = seguir para o servidor MCP.
pub async fn maybe_run(args: Vec<String>) -> Option<i32> {
    let cmd = match parse(&args) {
        Ok(None) => return None,
        Ok(Some(c)) => c,
        Err(e) => {
            eprintln!("anigo-mcp: {e}\nuso: anigo-mcp bridge-info | bridge-call <ACAO> [PARAMS_JSON|@arquivo] [--save-png <png>] [--timeout <s>]");
            return Some(2);
        }
    };
    let client = LiveBridgeClient::from_env();
    match run(&client, cmd).await {
        Ok(out) => {
            println!("{out}");
            Some(0)
        }
        Err(e) => {
            eprintln!("anigo-mcp: {e:#}");
            Some(1)
        }
    }
}

async fn run(client: &LiveBridgeClient, cmd: Cmd) -> Result<String> {
    match cmd {
        Cmd::Info => {
            let live = client.is_live().await;
            Ok(serde_json::to_string(&json!({ "target": client.describe_target(), "live": live }))?)
        }
        Cmd::Call { action, params, save_png, timeout } => {
            // valida o destino antes de gastar a chamada
            if let Some(path) = &save_png {
                sanitize_save_path(path)?;
            }
            let t = timeout.map(Duration::from_secs).unwrap_or_else(|| crate::bridge_client::read_timeout_for(&action));
            let mut data = client.send_command_timeout(&action, params, t).await?;
            if let (Some(path), Some(obj)) = (save_png, data.as_object_mut()) {
                let b64 = obj.get("png_base64").and_then(Value::as_str).ok_or_else(|| anyhow::anyhow!("a resposta não tem png_base64 (use SCREENSHOT ou RENDER_STATUS com inline)"))?;
                let bytes = base64::engine::general_purpose::STANDARD.decode(b64).context("png_base64 inválido")?;
                let p = sanitize_save_path(&path)?;
                if let Some(dir) = p.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                std::fs::write(&p, &bytes).with_context(|| format!("gravando {p:?}"))?;
                obj.insert("png_base64".into(), json!(format!("<{} bytes salvos em {}>", bytes.len(), p.display())));
            }
            Ok(serde_json::to_string(&data)?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn non_subcommands_fall_through_to_the_mcp_server() {
        assert_eq!(parse(&a(&[])).unwrap(), None);
        assert_eq!(parse(&a(&["--help"])).unwrap(), None);
    }

    #[test]
    fn parses_info_and_call_forms() {
        assert_eq!(parse(&a(&["bridge-info"])).unwrap(), Some(Cmd::Info));
        let c = parse(&a(&["bridge-call", "PING"])).unwrap().unwrap();
        assert_eq!(c, Cmd::Call { action: "PING".into(), params: json!({}), save_png: None, timeout: None });
        let c = parse(&a(&["bridge-call", "APPLY_COMMAND", r#"{"command":{"kind":"reset_morphs"}}"#, "--timeout", "5"])).unwrap().unwrap();
        match c {
            Cmd::Call { action, params, timeout, .. } => {
                assert_eq!(action, "APPLY_COMMAND");
                assert_eq!(params["command"]["kind"], "reset_morphs");
                assert_eq!(timeout, Some(5));
            }
            _ => panic!(),
        }
        let c = parse(&a(&["bridge-call", "SCREENSHOT", "--save-png", "tmp/x.png"])).unwrap().unwrap();
        assert!(matches!(c, Cmd::Call { save_png: Some(ref p), .. } if p == "tmp/x.png"));
    }

    #[test]
    fn rejects_bad_usage() {
        for bad in [
            vec!["bridge-call"],
            vec!["bridge-call", "ping"],
            vec!["bridge-call", "PING; rm"],
            vec!["bridge-call", "PING", "nao-json"],
            vec!["bridge-call", "PING", "[1,2]"],
            vec!["bridge-call", "PING", "{}", "{}"],
            vec!["bridge-call", "PING", "--timeout"],
            vec!["bridge-call", "PING", "--timeout", "0"],
            vec!["bridge-call", "PING", "--timeout", "999"],
            vec!["bridge-call", "PING", "--wat"],
            vec!["bridge-call", "PING", "--save-png"],
        ] {
            assert!(parse(&a(&bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn params_files_go_through_the_read_sandbox() {
        assert!(parse_params("@/etc/shadow").is_err());
        assert!(parse_params("@../../etc/passwd").is_err());
        assert!(parse_params("@nao_existe_xyz.json").is_err());
    }
}
