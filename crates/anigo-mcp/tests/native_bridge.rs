//! Integração: cliente do `anigo-mcp` (e as ferramentas `anigo_native_*`) -> bridge real (sockets em 127.0.0.1)
//! -> thread "host" dona de uma `CoreSession` REAL. O host de teste faz o papel da UI do studio para as ações
//! do núcleo (mesmo `core_ops::handle` que o studio usa); o comportamento da UI é coberto pelo smoke Xvfb
//! (`scripts/native_bridge_smoke.sh`).

#![allow(dead_code)]

#[path = "../src/bridge_client.rs"]
mod bridge_client;
#[path = "../src/fs_sandbox.rs"]
mod fs_sandbox;
#[path = "../src/metrics.rs"]
mod metrics;
#[path = "../src/native.rs"]
mod native;
#[path = "../src/validate.rs"]
mod validate;

use anigo_bridge::{channel, core_ops, BridgeError, BridgeServer, HostQueue, ServerConfig};
use anigo_session::CoreSession;
use base64::Engine as _;
use bridge_client::{BridgeCallError, LiveBridgeClient};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Host {
    stop: Arc<AtomicBool>,
    core: Arc<Mutex<CoreSession>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// 1x1 PNG válido.
fn tiny_png() -> Vec<u8> {
    let mut png = Vec::new();
    {
        use image::ImageEncoder;
        image::codecs::png::PngEncoder::new(&mut png).write_image(&[10, 20, 30, 255], 1, 1, image::ColorType::Rgba8.into()).unwrap();
    }
    png
}

impl Host {
    fn spawn(queue: HostQueue) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let core = Arc::new(Mutex::new({
            let mut c = CoreSession::new();
            c.set_history_limit(500);
            c
        }));
        let (s2, c2) = (Arc::clone(&stop), Arc::clone(&core));
        let handle = std::thread::spawn(move || {
            let mut workspace = "personagem".to_string();
            while !s2.load(Relaxed) {
                for r in queue.drain(16) {
                    let mut core = c2.lock().unwrap();
                    let res = match core_ops::handle(&mut core, &r.action, &r.params) {
                        Some(res) => res,
                        None => match r.action.as_str() {
                            "SET_WORKSPACE" => {
                                workspace = r.params["workspace"].as_str().unwrap_or("").to_string();
                                Ok(json!({ "workspace": workspace }))
                            }
                            "GET_STATUS" => Ok(json!({ "workspace": workspace, "history": core.history_report() })),
                            "GET_SELECTION" => Ok(json!({ "workspace": workspace })),
                            "SCREENSHOT" => Ok(json!({
                                "width": 1, "height": 1, "target": "viewport", "workspace": workspace,
                                "png_base64": base64::engine::general_purpose::STANDARD.encode(tiny_png()),
                            })),
                            "BLOB" => Ok(json!({ "blob": "z".repeat(3 << 20) })),
                            other => Err(BridgeError::unknown_action(other)),
                        },
                    };
                    r.reply.send(res);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        Self { stop, core, handle: Some(handle) }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn start(discovery: Option<std::path::PathBuf>, token: Option<&str>) -> (BridgeServer, Host) {
    let (backend, queue) = channel(64, Arc::new(|| {}));
    let host = Host::spawn(queue);
    let server = BridgeServer::start(
        ServerConfig { discovery, token: token.map(String::from), auth_failure_delay: Duration::from_millis(5), ..Default::default() },
        Arc::new(backend),
    )
    .unwrap();
    (server, host)
}

fn text_of(content: &[Value]) -> Value {
    serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap_or(Value::Null)
}

#[tokio::test]
async fn client_discovers_the_session_applies_a_command_and_undoes_it() {
    let dir = tempfile::tempdir().unwrap();
    let disc = dir.path().join("bridge.json");
    let (server, host) = start(Some(disc.clone()), None);
    // sem env: só o arquivo de descoberta (porta efêmera + token da sessão)
    let client = LiveBridgeClient::discovering(disc.clone(), "127.0.0.1:1", "");
    assert!(client.is_live().await);
    assert!(client.describe_target().contains("descoberta"));
    assert!(!client.describe_target().contains(server.token()), "o token nunca aparece em mensagens");

    let before = host.core.lock().unwrap().project().name.clone();
    let r = client.send_command("APPLY_COMMAND", json!({"command": {"kind": "rename_project", "name": "Pelo MCP"}})).await.unwrap();
    assert_eq!(r["outcome"]["undo_depth"], 1);
    assert_eq!(host.core.lock().unwrap().project().name, "Pelo MCP");

    let r = client.send_command("UNDO", json!({})).await.unwrap();
    assert_eq!(r["history"]["can_redo"], true);
    assert_eq!(host.core.lock().unwrap().project().name, before);
    client.send_command("REDO", json!({})).await.unwrap();
    assert_eq!(host.core.lock().unwrap().project().name, "Pelo MCP");

    let log = client.send_command("GET_LOG", json!({"include_commands": true})).await.unwrap();
    assert_eq!(log["total"], 1);
    assert_eq!(log["entries"][0]["command"]["name"], "Pelo MCP");
    let project = client.send_command("GET_PROJECT", json!({})).await.unwrap();
    assert_eq!(project["project"]["name"], "Pelo MCP");
}

#[tokio::test]
async fn invalid_command_surfaces_a_typed_error_with_the_wire_code() {
    let (server, host) = start(None, Some("tok-de-teste"));
    let client = LiveBridgeClient::with_token(server.addr(), "tok-de-teste");
    let err = client.send_command("APPLY_COMMAND", json!({"command": {"kind": "nao_existe"}})).await.unwrap_err();
    let typed = err.downcast_ref::<BridgeCallError>().expect("erro tipado");
    assert_eq!(typed.code, Some(-32602));
    assert!(!host.core.lock().unwrap().history().can_undo());
}

#[tokio::test]
async fn auth_failures_give_actionable_messages_without_leaking_the_token() {
    let (server, _host) = start(None, Some("segredo-1234567890"));
    let wrong = LiveBridgeClient::with_token(server.addr(), "outro-token");
    let e = wrong.send_command("PING", json!({})).await.unwrap_err().to_string();
    assert!(e.contains("rejected the token"), "{e}");
    assert!(!e.contains("outro-token"), "mensagem não pode repetir o token inteiro: {e}");
    let none = LiveBridgeClient::with_token(server.addr(), "");
    let e = none.send_command("PING", json!({})).await.unwrap_err().to_string();
    assert!(e.contains("--bridge"), "{e}");
}

#[tokio::test]
async fn responses_larger_than_the_legacy_1mib_cap_are_accepted() {
    let (server, _host) = start(None, Some("t"));
    let client = LiveBridgeClient::with_token(server.addr(), "t");
    let r = client.send_command("BLOB", json!({})).await.unwrap();
    assert_eq!(r["blob"].as_str().unwrap().len(), 3 << 20);
}

#[tokio::test]
async fn discovery_follows_a_restart_with_new_port_and_token() {
    let dir = tempfile::tempdir().unwrap();
    let disc = dir.path().join("bridge.json");
    let client = LiveBridgeClient::discovering(disc.clone(), "127.0.0.1:1", "");
    {
        let (s1, _h1) = start(Some(disc.clone()), None);
        assert!(client.is_live().await);
        let (p1, t1) = (s1.port(), s1.token().to_string());
        drop(s1);
        assert!(!client.is_live().await, "studio fechado: sem bridge");
        let (s2, _h2) = start(Some(disc.clone()), None);
        assert!(client.is_live().await, "o cliente relê o arquivo e acha a nova sessão");
        assert!(s2.port() != p1 || s2.token() != t1);
    }
    // sem arquivo e sem Tauri: erro claro
    let e = client.send_command("PING", json!({})).await.unwrap_err().to_string();
    assert!(e.contains("not running") || e.contains("not responding"), "{e}"); // Windows: conectar em 127.0.0.1:1 estoura o prazo em vez de recusar
}

#[tokio::test]
async fn native_tools_drive_the_session_end_to_end() {
    let (server, host) = start(None, Some("t"));
    let client = LiveBridgeClient::with_token(server.addr(), "t");
    let m = metrics::McpMetrics::new();

    let out = native::call("anigo_native_apply_command", &json!({"command": {"kind": "rename_project", "name": "Ferramenta MCP"}}), &client, &m).await.unwrap().unwrap();
    assert_eq!(text_of(&out)["outcome"]["undo_depth"], 1);
    assert_eq!(host.core.lock().unwrap().project().name, "Ferramenta MCP");

    // concorrência otimista: revisão antiga é recusada
    let rev = text_of(&native::call("anigo_native_get_project", &json!({}), &client, &m).await.unwrap().unwrap())["history"]["revision"].as_u64().unwrap();
    native::call("anigo_native_apply_command", &json!({"command": {"kind": "rename_project", "name": "B"}, "if_revision": rev}), &client, &m).await.unwrap().unwrap();
    let e = native::call("anigo_native_apply_command", &json!({"command": {"kind": "rename_project", "name": "C"}, "if_revision": rev}), &client, &m).await.unwrap().unwrap_err().to_string();
    assert!(e.contains("desatualizada"), "{e}");

    let out = native::call("anigo_native_undo", &json!({}), &client, &m).await.unwrap().unwrap();
    assert_eq!(text_of(&out)["history"]["can_redo"], true);
    assert_eq!(host.core.lock().unwrap().project().name, "Ferramenta MCP");
    native::call("anigo_native_redo", &json!({}), &client, &m).await.unwrap().unwrap();
    assert_eq!(host.core.lock().unwrap().project().name, "B");

    let log = text_of(&native::call("anigo_native_get_log", &json!({"limit": 10}), &client, &m).await.unwrap().unwrap());
    assert_eq!(log["total"], 2);

    let ws = text_of(&native::call("anigo_native_set_workspace", &json!({"workspace": "cenario"}), &client, &m).await.unwrap().unwrap());
    assert_eq!(ws["workspace"], "cenario");
    let st = text_of(&native::call("anigo_native_get_status", &json!({}), &client, &m).await.unwrap().unwrap());
    assert_eq!(st["status"]["workspace"], "cenario");

    assert_eq!(m.native_commands_applied.load(Relaxed), 4);
    assert!(m.bridge_commands_sent.load(Relaxed) >= 8);
    assert_eq!(m.bridge_commands_failed.load(Relaxed), 1, "só a revisão desatualizada falhou");
}

#[tokio::test]
async fn native_screenshot_tool_returns_an_image_and_saves_only_inside_the_sandbox() {
    let (server, _host) = start(None, Some("t"));
    let client = LiveBridgeClient::with_token(server.addr(), "t");
    let m = metrics::McpMetrics::new();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("ANIGO_MCP_ALLOWED_DIRS", dir.path());

    let target = dir.path().join("sub").join("shot.png");
    let out = native::call("anigo_native_screenshot", &json!({"save_path": target.to_str().unwrap()}), &client, &m).await.unwrap().unwrap();
    assert_eq!(out[1]["type"], "image");
    assert_eq!(out[1]["mimeType"], "image/png");
    let saved = std::fs::read(&target).expect("arquivo salvo no sandbox");
    assert_eq!(&saved[..4], b"\x89PNG");
    assert_eq!(m.native_images_received.load(Relaxed), 1);
    assert!(m.bytes_written_to_fs.load(Relaxed) > 0);

    // fora do sandbox: recusado ANTES de capturar (nenhum pedido ao studio)
    let sent = m.bridge_commands_sent.load(Relaxed);
    let e = native::call("anigo_native_screenshot", &json!({"save_path": "/etc/anigo_shot.png"}), &client, &m).await.unwrap().unwrap_err().to_string();
    assert!(!e.is_empty());
    assert_eq!(m.bridge_commands_sent.load(Relaxed), sent);
    assert_eq!(m.fs_sandbox_rejections.load(Relaxed), 1);
    let e = native::call("anigo_native_screenshot", &json!({"save_path": "../../fora.png"}), &client, &m).await.unwrap().unwrap_err().to_string();
    assert!(!e.is_empty());
    assert!(!std::path::Path::new("/etc/anigo_shot.png").exists());
}

#[tokio::test]
async fn many_mcp_clients_in_parallel_do_not_lose_commands() {
    let (server, host) = start(None, Some("t"));
    let addr = server.addr();
    let mut tasks = Vec::new();
    for t in 0..16 {
        let addr = addr.clone();
        tasks.push(tokio::spawn(async move {
            let client = LiveBridgeClient::with_token(addr, "t");
            let mut ok = 0;
            for i in 0..10 {
                let r = client.send_command("APPLY_COMMAND", json!({"command": {"kind": "rename_project", "name": format!("m{t}-{i}")}})).await;
                if r.is_ok() {
                    ok += 1;
                }
            }
            ok
        }));
    }
    let mut total = 0;
    for t in tasks {
        total += t.await.unwrap();
    }
    assert_eq!(total, 160);
    assert_eq!(host.core.lock().unwrap().history().export_log().entries.len(), 160);
}
