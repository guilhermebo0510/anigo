//! Ferramentas MCP do app NATIVO (T-2.13): `anigo_native_*`. Falam com a bridge do `anigo-studio --bridge`
//! (`crates/anigo-bridge`) pelo mesmo `LiveBridgeClient` das ferramentas antigas; toda mutação é um `Command`
//! do núcleo aplicado na MESMA sessão do usuário (desfazível com Ctrl+Z na janela).
//!
//! Entrada validada em `validate.rs`; arquivos só no sandbox de `fs_sandbox`; contadores em `metrics.rs`.

use crate::bridge_client::LiveBridgeClient;
use crate::fs_sandbox::sanitize_save_path;
use crate::metrics::McpMetrics;
use crate::validate;
use anyhow::{Context, Result};
use base64::Engine as _;
use serde_json::{json, Value};
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

/// Texto devolvido ao modelo é truncado acima disto (use `save_path` para o resto).
const MAX_TEXT_CHARS: usize = 120_000;

fn ann(read_only: bool, destructive: bool, idempotent: bool) -> Value {
    json!({ "readOnlyHint": read_only, "destructiveHint": destructive, "idempotentHint": idempotent, "openWorldHint": false })
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str], annotations: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": required, "additionalProperties": false },
        "annotations": annotations,
    })
}

/// Definições das ferramentas nativas (acrescentadas ao catálogo existente).
pub fn definitions() -> Vec<Value> {
    let save = json!({ "type": "string", "description": "Optional local path to save the result (allowlisted: Documents/ANIGO, ./baselines, ./tmp/anigo-mcp, ANIGO_MCP_ALLOWED_DIRS)" });
    vec![
        tool(
            "anigo_native_get_status",
            "Native ANIGO Studio (started with --bridge): active workspace, real frame time, GPU adapter, project name, camera, history report (undo/redo depth, revisions) and bridge metrics.",
            json!({}), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_get_project",
            "Reads the canonical project (ProjectState) of the native studio session. With include_document=false (default) returns only the history report (revisions, undo/redo depth, fingerprint); with true returns the full canonical document JSON (can be large; use save_path).",
            json!({ "include_document": { "type": "boolean", "default": false }, "save_path": save }), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_get_snapshot",
            "Reads the CoreSnapshot (the canonical hand-off to renderers: dynamic weights/camera/lights/materials; geometry only if include_static=true — large). Prefer anigo_native_get_project for editing decisions.",
            json!({ "include_static": { "type": "boolean", "default": false }, "save_path": save }), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_apply_command",
            "Applies ONE core Command (contract v1 JSON, e.g. {\"kind\":\"rename_project\",\"name\":\"X\"}, {\"kind\":\"set_base_gender\",\"gender\":\"female\"}, {\"kind\":\"batch\",\"commands\":[...]} for atomic groups) through the same CoreSession as the user: it is validated by the core, enters the undo history, and shows live in the window. Optional if_revision (history revision from a previous call) rejects the command if someone else changed the project meanwhile.",
            json!({
                "command": { "type": "object", "description": "Command JSON with a snake_case 'kind' (see anigo-core Command)", "properties": { "kind": { "type": "string" } }, "required": ["kind"] },
                "if_revision": { "type": "integer", "minimum": 0, "description": "Optimistic concurrency: only apply if history.revision equals this" }
            }), &["command"], ann(false, false, false),
        ),
        tool(
            "anigo_native_undo",
            "Undoes the last accepted command of the native session (same history as Ctrl+Z in the core-backed workspaces: Personagem, Cenário, Iluminação, Render, Posing).",
            json!({}), &[], ann(false, true, false),
        ),
        tool(
            "anigo_native_redo",
            "Redoes the last undone command of the native session.",
            json!({}), &[], ann(false, false, false),
        ),
        tool(
            "anigo_native_get_log",
            "Lists the session command log (the log is the session: undo does not remove entries). Page with from/limit (limit<=500); include_commands=true returns each Command JSON; sequence=N reads one entry with its command.",
            json!({
                "from": { "type": "integer", "minimum": 0 }, "limit": { "type": "integer", "minimum": 1, "maximum": 500, "default": 50 },
                "include_commands": { "type": "boolean", "default": false }, "sequence": { "type": "integer", "minimum": 1 }
            }), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_list_assets",
            "Lists Library assets (id, name, kind, format, triangles, bones...). Filters: text (Library search syntax), kinds (model, character, outfit, hair, accessory, material, texture, pose, animation, hdri, scene, prefab), limit<=500, after (cursor from a previous page).",
            json!({
                "text": { "type": "string", "maxLength": 256 },
                "kinds": { "type": "array", "items": { "type": "string", "enum": validate::NATIVE_ASSET_KINDS }, "maxItems": 16 },
                "limit": { "type": "integer", "minimum": 1, "maximum": 500, "default": 100 }, "after": { "type": "string", "maxLength": 512 }
            }), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_place_asset",
            "Places Library asset(s) in the scene as ONE undoable step (like double-click in the Content Drawer). Optional world position [x,y,z]; default is the center of the scene viewport.",
            json!({
                "asset_id": { "type": "string", "description": "Library asset UUID" },
                "asset_ids": { "type": "array", "items": { "type": "string" }, "maxItems": 32 },
                "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
            }), &[], ann(false, false, false),
        ),
        tool(
            "anigo_native_render_frame",
            "Queues a still render with the project's current render settings in the native studio's render queue (real final-quality pipeline) and, with wait=true (default), waits for it. Returns file paths; inline=true also returns the PNG image.",
            json!({
                "wait": { "type": "boolean", "default": true },
                "timeout_seconds": { "type": "integer", "minimum": 5, "maximum": 300, "default": 90 },
                "inline": { "type": "boolean", "default": false }, "save_path": save
            }), &[], ann(false, false, false),
        ),
        tool(
            "anigo_native_screenshot",
            "Screenshot of the native studio: the active workspace's 3D viewport (default) or the whole window. Returns a PNG image; max_width downsizes to save tokens.",
            json!({
                "target": { "type": "string", "enum": ["viewport", "window"], "default": "viewport" },
                "max_width": { "type": "integer", "minimum": 64, "maximum": 4096 }, "save_path": save
            }), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_set_workspace",
            "Switches the active workspace of the native studio.",
            json!({ "workspace": { "type": "string", "enum": validate::NATIVE_WORKSPACES } }), &["workspace"], ann(false, false, true),
        ),
        tool(
            "anigo_native_get_selection",
            "Reads the current selection state: scene node ids, modeling selection counts, posing bones and Library selection.",
            json!({}), &[], ann(true, false, true),
        ),
    ]
}

pub fn is_native_tool(name: &str) -> bool {
    name.starts_with("anigo_native_")
}

fn text(v: Value) -> Value {
    json!({ "type": "text", "text": v })
}

/// JSON como texto, truncado de forma explícita (o modelo sabe que faltou).
fn json_text(v: &Value) -> Result<Value> {
    let mut s = serde_json::to_string_pretty(v)?;
    if s.chars().count() > MAX_TEXT_CHARS {
        let cut: String = s.chars().take(MAX_TEXT_CHARS).collect();
        s = format!("{cut}\n... [truncated: result larger than {MAX_TEXT_CHARS} characters; pass save_path to get all of it]");
    }
    Ok(json!({ "type": "text", "text": s }))
}

async fn call_bridge(client: &LiveBridgeClient, metrics: &McpMetrics, action: &str, params: Value) -> Result<Value> {
    metrics.bridge_commands_sent.fetch_add(1, Relaxed);
    match client.send_command(action, params).await {
        Ok(v) => Ok(v),
        Err(e) => {
            metrics.bridge_commands_failed.fetch_add(1, Relaxed);
            Err(e)
        }
    }
}

fn save_bytes(metrics: &McpMetrics, path: &str, bytes: &[u8]) -> Result<std::path::PathBuf> {
    let p = match sanitize_save_path(path) {
        Ok(p) => p,
        Err(e) => {
            metrics.fs_sandbox_rejections.fetch_add(1, Relaxed);
            return Err(e);
        }
    };
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {dir:?}"))?;
    }
    std::fs::write(&p, bytes).with_context(|| format!("writing {p:?}"))?;
    metrics.bytes_written_to_fs.fetch_add(bytes.len() as u64, Relaxed);
    Ok(p)
}

fn opt_str<'a>(args: &'a Value, k: &str) -> Result<Option<&'a str>> {
    match args.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => anyhow::bail!("'{}' must be a string (code -32602)", k),
    }
}

fn opt_u64(args: &Value, k: &str, min: u64, max: u64) -> Result<Option<u64>> {
    match args.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let n = v.as_u64().ok_or_else(|| anyhow::anyhow!("'{}' must be a non-negative integer (code -32602)", k))?;
            if n < min || n > max {
                anyhow::bail!("'{}' out of range {}..{}: {} (code -32602)", k, min, max, n);
            }
            Ok(Some(n))
        }
    }
}

fn opt_bool(args: &Value, k: &str) -> Result<Option<bool>> {
    match args.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => anyhow::bail!("'{}' must be a boolean (code -32602)", k),
    }
}

/// Recusa propriedades desconhecidas (schema `additionalProperties:false`).
fn only_keys(args: &Value, allowed: &[&str]) -> Result<()> {
    match args {
        Value::Null => Ok(()),
        Value::Object(m) => {
            for k in m.keys() {
                if !allowed.contains(&k.as_str()) {
                    anyhow::bail!("Unknown argument '{}'. Allowed: {} (code -32602)", k, if allowed.is_empty() { "(none)".to_string() } else { allowed.join(", ") });
                }
            }
            Ok(())
        }
        _ => anyhow::bail!("arguments must be an object (code -32602)"),
    }
}

/// Imagem MCP a partir de um `png_base64` da bridge (+ salva no sandbox quando pedido).
fn image_result(metrics: &McpMetrics, label: String, b64: &str, save_path: Option<&str>) -> Result<Vec<Value>> {
    let mut note = label;
    if let Some(path) = save_path {
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).context("invalid base64 image from bridge")?;
        let p = save_bytes(metrics, path, &bytes)?;
        note.push_str(&format!("\nSaved to: {p:?}"));
    }
    metrics.native_images_received.fetch_add(1, Relaxed);
    Ok(vec![text(Value::String(note)), json!({ "type": "image", "data": b64, "mimeType": "image/png" })])
}

/// `None` = não é uma ferramenta nativa.
pub async fn call(name: &str, args: &Value, client: &LiveBridgeClient, metrics: &McpMetrics) -> Option<Result<Vec<Value>>> {
    if !is_native_tool(name) {
        return None;
    }
    Some(dispatch(name, args, client, metrics).await)
}

async fn dispatch(name: &str, args: &Value, client: &LiveBridgeClient, m: &McpMetrics) -> Result<Vec<Value>> {
    match name {
        "anigo_native_get_status" => {
            only_keys(args, &[])?;
            let status = call_bridge(client, m, "GET_STATUS", json!({})).await?;
            let sel = call_bridge(client, m, "GET_SELECTION", json!({})).await.ok();
            Ok(vec![json_text(&json!({ "status": status, "selection": sel }))?])
        }
        "anigo_native_get_project" => {
            only_keys(args, &["include_document", "save_path"])?;
            let include = opt_bool(args, "include_document")?.unwrap_or(false);
            let data = call_bridge(client, m, "GET_PROJECT", json!({ "include_document": include })).await?;
            finish_json(m, &data, opt_str(args, "save_path")?)
        }
        "anigo_native_get_snapshot" => {
            only_keys(args, &["include_static", "save_path"])?;
            let include = opt_bool(args, "include_static")?.unwrap_or(false);
            let data = call_bridge(client, m, "GET_SNAPSHOT", json!({ "include_static": include })).await?;
            finish_json(m, &data, opt_str(args, "save_path")?)
        }
        "anigo_native_apply_command" => {
            only_keys(args, &["command", "if_revision"])?;
            let cmd = args.get("command").ok_or_else(|| anyhow::anyhow!("Missing 'command' (code -32602)"))?;
            validate::validate_native_command(cmd)?;
            let mut params = json!({ "command": cmd });
            if let Some(r) = opt_u64(args, "if_revision", 0, u64::MAX)? {
                params["if_revision"] = json!(r);
            }
            let data = call_bridge(client, m, "APPLY_COMMAND", params).await?;
            m.native_commands_applied.fetch_add(1, Relaxed);
            Ok(vec![json_text(&data)?])
        }
        "anigo_native_undo" | "anigo_native_redo" => {
            only_keys(args, &[])?;
            let action = if name.ends_with("undo") { "UNDO" } else { "REDO" };
            let data = call_bridge(client, m, action, json!({})).await?;
            m.native_commands_applied.fetch_add(1, Relaxed);
            Ok(vec![json_text(&data)?])
        }
        "anigo_native_get_log" => {
            only_keys(args, &["from", "limit", "include_commands", "sequence"])?;
            let mut p = serde_json::Map::new();
            if let Some(v) = opt_u64(args, "from", 0, u32::MAX as u64)? {
                p.insert("from".into(), json!(v));
            }
            if let Some(v) = opt_u64(args, "limit", 1, 500)? {
                p.insert("limit".into(), json!(v));
            }
            if let Some(v) = opt_u64(args, "sequence", 1, u64::MAX)? {
                p.insert("sequence".into(), json!(v));
            }
            if let Some(v) = opt_bool(args, "include_commands")? {
                p.insert("include_commands".into(), json!(v));
            }
            let data = call_bridge(client, m, "GET_LOG", Value::Object(p)).await?;
            Ok(vec![json_text(&data)?])
        }
        "anigo_native_list_assets" => {
            only_keys(args, &["text", "kinds", "limit", "after"])?;
            let mut p = serde_json::Map::new();
            if let Some(t) = opt_str(args, "text")? {
                validate::validate_text(t, "text", 256)?;
                p.insert("text".into(), json!(t));
            }
            if let Some(k) = args.get("kinds").filter(|v| !v.is_null()) {
                let arr = k.as_array().filter(|a| a.len() <= 16).ok_or_else(|| anyhow::anyhow!("'kinds' must be an array of up to 16 strings (code -32602)"))?;
                let mut kinds = Vec::new();
                for i in arr {
                    let s = i.as_str().ok_or_else(|| anyhow::anyhow!("'kinds' must contain strings (code -32602)"))?;
                    validate::validate_native_asset_kind(s)?;
                    kinds.push(json!(s));
                }
                p.insert("kinds".into(), Value::Array(kinds));
            }
            if let Some(v) = opt_u64(args, "limit", 1, 500)? {
                p.insert("limit".into(), json!(v));
            }
            if let Some(a) = opt_str(args, "after")? {
                validate::validate_text(a, "after", 512)?;
                p.insert("after".into(), json!(a));
            }
            let data = call_bridge(client, m, "LIST_ASSETS", Value::Object(p)).await?;
            Ok(vec![json_text(&data)?])
        }
        "anigo_native_place_asset" => {
            only_keys(args, &["asset_id", "asset_ids", "position"])?;
            let mut p = serde_json::Map::new();
            let mut n = 0;
            if let Some(id) = opt_str(args, "asset_id")? {
                validate::validate_uuid(id, "asset_id")?;
                p.insert("asset_id".into(), json!(id));
                n += 1;
            }
            if let Some(list) = args.get("asset_ids").filter(|v| !v.is_null()) {
                let arr = list.as_array().filter(|a| a.len() <= 32).ok_or_else(|| anyhow::anyhow!("'asset_ids' must be an array of up to 32 UUIDs (code -32602)"))?;
                let mut ids = Vec::new();
                for i in arr {
                    let s = i.as_str().ok_or_else(|| anyhow::anyhow!("'asset_ids' must contain strings (code -32602)"))?;
                    validate::validate_uuid(s, "asset_ids[]")?;
                    ids.push(json!(s));
                }
                n += ids.len();
                p.insert("asset_ids".into(), Value::Array(ids));
            }
            if n == 0 {
                anyhow::bail!("Missing 'asset_id' or 'asset_ids' (code -32602)");
            }
            if let Some(pos) = args.get("position").filter(|v| !v.is_null()) {
                let arr = pos.as_array().ok_or_else(|| anyhow::anyhow!("'position' must be [x,y,z] (code -32602)"))?;
                let v = validate::validate_vec3(arr, "position").map_err(|e| anyhow::anyhow!("{e} (code -32602)"))?;
                if v.abs().max_element() > 1.0e6 {
                    anyhow::bail!("position out of range (code -32602)");
                }
                p.insert("position".into(), json!([v.x, v.y, v.z]));
            }
            let data = call_bridge(client, m, "PLACE_ASSET", Value::Object(p)).await?;
            m.native_commands_applied.fetch_add(1, Relaxed);
            Ok(vec![json_text(&data)?])
        }
        "anigo_native_set_workspace" => {
            only_keys(args, &["workspace"])?;
            let ws = opt_str(args, "workspace")?.ok_or_else(|| anyhow::anyhow!("Missing 'workspace' (code -32602)"))?;
            validate::validate_native_workspace(ws)?;
            let data = call_bridge(client, m, "SET_WORKSPACE", json!({ "workspace": ws })).await?;
            Ok(vec![json_text(&data)?])
        }
        "anigo_native_get_selection" => {
            only_keys(args, &[])?;
            let data = call_bridge(client, m, "GET_SELECTION", json!({})).await?;
            Ok(vec![json_text(&data)?])
        }
        "anigo_native_screenshot" => {
            only_keys(args, &["target", "max_width", "save_path"])?;
            let mut p = serde_json::Map::new();
            if let Some(t) = opt_str(args, "target")? {
                if t != "viewport" && t != "window" {
                    anyhow::bail!("'target' must be 'viewport' or 'window' (code -32602)");
                }
                p.insert("target".into(), json!(t));
            }
            if let Some(w) = opt_u64(args, "max_width", 64, 4096)? {
                p.insert("max_width".into(), json!(w));
            }
            let save = opt_str(args, "save_path")?;
            if let Some(path) = save {
                // valida o destino ANTES de gastar uma captura
                sanitize_save_path(path).inspect_err(|_| {
                    m.fs_sandbox_rejections.fetch_add(1, Relaxed);
                })?;
            }
            let data = call_bridge(client, m, "SCREENSHOT", Value::Object(p)).await?;
            let b64 = data.get("png_base64").and_then(Value::as_str).ok_or_else(|| anyhow::anyhow!("bridge returned no image"))?;
            let label = format!(
                "Native studio screenshot: {}x{} px, target={}, workspace={}",
                data["width"], data["height"], data["target"].as_str().unwrap_or("?"), data["workspace"].as_str().unwrap_or("?")
            );
            image_result(m, label, b64, save)
        }
        "anigo_native_render_frame" => render_frame(args, client, m).await,
        other => anyhow::bail!("Unknown tool: {} (code -32601)", other),
    }
}

fn finish_json(m: &McpMetrics, data: &Value, save: Option<&str>) -> Result<Vec<Value>> {
    let mut out = vec![json_text(data)?];
    if let Some(path) = save {
        let bytes = serde_json::to_vec_pretty(data)?;
        let p = save_bytes(m, path, &bytes)?;
        out.push(text(Value::String(format!("Saved to: {p:?} ({} bytes)", bytes.len()))));
    }
    Ok(out)
}

async fn render_frame(args: &Value, client: &LiveBridgeClient, m: &McpMetrics) -> Result<Vec<Value>> {
    only_keys(args, &["wait", "timeout_seconds", "inline", "save_path"])?;
    let wait = opt_bool(args, "wait")?.unwrap_or(true);
    let timeout_s = opt_u64(args, "timeout_seconds", 5, 300)?.unwrap_or(90);
    let inline = opt_bool(args, "inline")?.unwrap_or(false);
    let save = opt_str(args, "save_path")?;
    if let Some(path) = save {
        sanitize_save_path(path).inspect_err(|_| {
            m.fs_sandbox_rejections.fetch_add(1, Relaxed);
        })?;
    }
    if (inline || save.is_some()) && !wait {
        anyhow::bail!("inline/save_path require wait=true (code -32602)");
    }
    let queued = call_bridge(client, m, "RENDER_FRAME", json!({})).await?;
    if !wait {
        return Ok(vec![json_text(&queued)?]);
    }
    let job = queued.get("job_id").and_then(Value::as_u64).ok_or_else(|| anyhow::anyhow!("bridge returned no job_id"))?;
    let deadline = Instant::now() + Duration::from_secs(timeout_s);
    loop {
        let st = call_bridge(client, m, "RENDER_STATUS", json!({ "job_id": job })).await?;
        match st.get("status").and_then(Value::as_str) {
            Some("done") => {
                m.frames_rendered.fetch_add(1, Relaxed);
                if inline || save.is_some() {
                    let full = call_bridge(client, m, "RENDER_STATUS", json!({ "job_id": job, "inline": true })).await?;
                    if let Some(b64) = full.get("png_base64").and_then(Value::as_str) {
                        let label = format!("Render job {job} done:\n{}", serde_json::to_string_pretty(&full["files"])?);
                        return image_result(m, label, b64, save);
                    }
                    anyhow::bail!("render finished but the image could not be returned inline: {}", full["inline_error"].as_str().unwrap_or("unknown"));
                }
                return Ok(vec![json_text(&st)?]);
            }
            Some("failed") | Some("cancelled") => {
                anyhow::bail!("Render job {} {}: {}", job, st["status"].as_str().unwrap_or("?"), st["error"].as_str().unwrap_or("(no details)"));
            }
            _ => {}
        }
        if Instant::now() > deadline {
            anyhow::bail!("Render job {job} still running after {timeout_s}s (progress {:.0}%); poll with wait=false is not available — raise timeout_seconds", st["progress"].as_f64().unwrap_or(0.0) * 100.0);
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_well_formed_and_names_are_unique_and_prefixed() {
        let defs = definitions();
        let mut names = std::collections::HashSet::new();
        for d in &defs {
            let n = d["name"].as_str().unwrap();
            assert!(is_native_tool(n), "{n}");
            assert!(names.insert(n.to_string()), "duplicado {n}");
            assert!(d["description"].as_str().unwrap().len() > 20);
            assert_eq!(d["inputSchema"]["type"], "object");
            assert_eq!(d["inputSchema"]["additionalProperties"], false);
            assert!(d["annotations"]["readOnlyHint"].is_boolean());
        }
        assert!(defs.len() >= 13);
    }

    #[test]
    fn only_keys_rejects_unknown_arguments() {
        assert!(only_keys(&json!({}), &[]).is_ok());
        assert!(only_keys(&Value::Null, &[]).is_ok());
        assert!(only_keys(&json!({"a": 1}), &["a"]).is_ok());
        assert!(only_keys(&json!({"b": 1}), &["a"]).is_err());
        assert!(only_keys(&json!([1]), &["a"]).is_err());
    }

    #[test]
    fn typed_argument_helpers_enforce_types_and_ranges() {
        let a = json!({"s": "x", "n": 5, "b": true, "neg": -1});
        assert_eq!(opt_str(&a, "s").unwrap(), Some("x"));
        assert!(opt_str(&a, "n").is_err());
        assert_eq!(opt_u64(&a, "n", 1, 10).unwrap(), Some(5));
        assert!(opt_u64(&a, "n", 6, 10).is_err());
        assert!(opt_u64(&a, "neg", 0, 10).is_err());
        assert_eq!(opt_u64(&a, "zz", 0, 1).unwrap(), None);
        assert_eq!(opt_bool(&a, "b").unwrap(), Some(true));
        assert!(opt_bool(&a, "s").is_err());
    }

    #[test]
    fn long_json_is_truncated_explicitly() {
        let v = json!({"blob": "a".repeat(MAX_TEXT_CHARS * 2)});
        let t = json_text(&v).unwrap();
        let s = t["text"].as_str().unwrap();
        assert!(s.contains("[truncated"));
        assert!(s.chars().count() < MAX_TEXT_CHARS + 300);
        let small = json_text(&json!({"a": 1})).unwrap();
        assert!(!small["text"].as_str().unwrap().contains("truncated"));
    }

    #[tokio::test]
    async fn validation_happens_before_any_network_call() {
        // cliente apontando para uma porta fechada: se a validação deixasse passar, o erro seria de conexão
        let client = LiveBridgeClient::with_token("127.0.0.1:1", "t");
        let m = McpMetrics::new();
        for (tool, args) in [
            ("anigo_native_apply_command", json!({"command": {"kind": "Bad Kind"}})),
            ("anigo_native_apply_command", json!({"command": "x"})),
            ("anigo_native_apply_command", json!({})),
            ("anigo_native_apply_command", json!({"command": {"kind": "x"}, "extra": 1})),
            ("anigo_native_set_workspace", json!({"workspace": "../x"})),
            ("anigo_native_place_asset", json!({"asset_id": "nao-uuid"})),
            ("anigo_native_place_asset", json!({})),
            ("anigo_native_place_asset", json!({"asset_id": "0197f3a2-1b2c-7d3e-8f40-0123456789ab", "position": [1, 2]})),
            ("anigo_native_list_assets", json!({"kinds": ["exe"]})),
            ("anigo_native_list_assets", json!({"limit": 5000})),
            ("anigo_native_get_log", json!({"limit": 0})),
            ("anigo_native_screenshot", json!({"target": "tela"})),
            ("anigo_native_screenshot", json!({"max_width": 10})),
            ("anigo_native_render_frame", json!({"wait": false, "inline": true})),
            ("anigo_native_undo", json!({"x": 1})),
        ] {
            let err = call(tool, &args, &client, &m).await.expect("é nativa").unwrap_err().to_string();
            assert!(err.contains("-32602"), "{tool} {args}: {err}");
        }
        assert_eq!(m.bridge_commands_sent.load(Relaxed), 0, "nada deveria ter chegado à rede");
        assert!(call("anigo_ping", &json!({}), &client, &m).await.is_none());
    }

    #[tokio::test]
    async fn unreachable_bridge_counts_a_failed_command() {
        let client = LiveBridgeClient::with_token("127.0.0.1:1", "t");
        let m = McpMetrics::new();
        let err = call("anigo_native_get_selection", &json!({}), &client, &m).await.unwrap().unwrap_err().to_string();
        assert!(err.contains("not running") || err.contains("timeout"), "{err}");
        assert_eq!((m.bridge_commands_sent.load(Relaxed), m.bridge_commands_failed.load(Relaxed)), (1, 1));
    }

    #[test]
    fn save_path_outside_the_sandbox_is_rejected_and_counted() {
        let m = McpMetrics::new();
        let e = save_bytes(&m, "/etc/anigo_nope.png", b"x").unwrap_err().to_string();
        assert!(!e.is_empty());
        assert_eq!(m.fs_sandbox_rejections.load(Relaxed), 1);
        assert_eq!(m.bytes_written_to_fs.load(Relaxed), 0);
    }
}
