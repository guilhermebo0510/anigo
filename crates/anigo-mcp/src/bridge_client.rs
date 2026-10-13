use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

/// Porta histórica do bridge do app Tauri (usada quando não há descoberta nem `ANIGO_BRIDGE_ADDR`).
pub const LEGACY_ADDR: &str = "127.0.0.1:39090";
/// Requisição: mesmo teto do servidor.
const MAX_REQUEST: usize = 1 << 20;
/// Resposta: screenshots/snapshots em base64 passam de 1 MiB (o servidor nativo limita em 32 MiB).
const MAX_RESPONSE: usize = 32 << 20;

/// Erro devolvido pela bridge (`code` é o campo aditivo do servidor nativo; o Tauri não envia).
#[derive(Debug)]
pub struct BridgeCallError {
    pub code: Option<i64>,
    pub message: String,
}

impl std::fmt::Display for BridgeCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.code {
            Some(c) => write!(f, "[{c}] {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for BridgeCallError {}

/// Para onde conectar e com qual token.
enum Target {
    Fixed { addr: String, token: String },
    /// Reavalia a cada chamada: o app nativo escolhe porta e token novos a cada execução.
    Discover { path: PathBuf, fallback_addr: String, fallback_token: String },
}

/// P2-14: Authenticated client for the ANIGO Live bridges (Tauri legado ou app nativo).
///
/// Every request carries a monotonic `id` (P-05) and a `token` that the server validates before dispatching.
/// Com `from_env()` o cliente descobre porta e token do app NATIVO pelo arquivo de descoberta
/// (`anigo_bridge::discovery`), caindo para `127.0.0.1:39090` + `ANIGO_BRIDGE_TOKEN` (app Tauri).
pub struct LiveBridgeClient {
    target: Target,
    /// Monotonic request counter (P-05 fix).
    next_id: AtomicU64,
}

/// Prazo de leitura por ação: as ações antigas continuam com 2 s; as do app nativo esperam a UI.
pub fn read_timeout_for(action: &str) -> Duration {
    match action {
        "SCREENSHOT" | "PLACE_ASSET" | "RENDER_FRAME" | "LIST_ASSETS" | "MODEL" | "HAIR" | "WEIGHTS" | "MESHDATA" | "SHADING" | "ANIM" | "RENDER" | "POSE" | "LIGHT" | "SCENE" | "CHAR" | "APP" => Duration::from_secs(30),
        "LIBRARY" => Duration::from_secs(60),
        "GET_SNAPSHOT" | "GET_PROJECT" => Duration::from_secs(20),
        "APPLY_COMMAND" | "UNDO" | "REDO" | "GET_LOG" | "SET_WORKSPACE" | "GET_WORKSPACE" | "GET_SELECTION" | "GET_UI_STATE" | "UI_ACTION" | "INPUT" | "RENDER_STATUS" | "GET_METRICS" | "GET_STATUS" => Duration::from_secs(12),
        _ => Duration::from_secs(2),
    }
}

impl LiveBridgeClient {
    #[allow(dead_code)]
    pub fn new(addr: impl Into<String>) -> Self {
        Self::with_token(addr, std::env::var("ANIGO_BRIDGE_TOKEN").unwrap_or_default())
    }

    pub fn with_token(addr: impl Into<String>, token: impl Into<String>) -> Self {
        Self { target: Target::Fixed { addr: addr.into(), token: token.into() }, next_id: AtomicU64::new(1) }
    }

    /// `ANIGO_BRIDGE_ADDR` (+ `ANIGO_BRIDGE_TOKEN`) fixa o destino; senão descobre pelo arquivo de
    /// descoberta do app nativo (`ANIGO_BRIDGE_DISCOVERY` ou o caminho padrão), com o Tauri como alternativa.
    pub fn from_env() -> Self {
        let token = std::env::var("ANIGO_BRIDGE_TOKEN").unwrap_or_default();
        if let Some(addr) = std::env::var("ANIGO_BRIDGE_ADDR").ok().filter(|a| !a.trim().is_empty()) {
            return Self::with_token(addr, token);
        }
        Self::discovering(anigo_bridge::discovery::default_path(), LEGACY_ADDR, token)
    }

    pub fn discovering(path: PathBuf, fallback_addr: impl Into<String>, fallback_token: impl Into<String>) -> Self {
        Self { target: Target::Discover { path, fallback_addr: fallback_addr.into(), fallback_token: fallback_token.into() }, next_id: AtomicU64::new(1) }
    }

    /// (endereço, token, origem em texto) para a próxima chamada.
    fn resolve(&self) -> (String, String, String) {
        match &self.target {
            Target::Fixed { addr, token } => (addr.clone(), token.clone(), "ANIGO_BRIDGE_ADDR".to_string()),
            Target::Discover { path, fallback_addr, fallback_token } => match anigo_bridge::discovery::read(path) {
                Ok(d) => (d.addr, d.token, format!("descoberta {}", path.display())),
                Err(_) => (fallback_addr.clone(), fallback_token.clone(), "porta legada (sem arquivo de descoberta)".to_string()),
            },
        }
    }

    /// Texto curto para mensagens de erro/diagnóstico (nunca inclui o token).
    pub fn describe_target(&self) -> String {
        let (addr, token, source) = self.resolve();
        format!("{addr} via {source}{}", if token.is_empty() { " (sem token)" } else { "" })
    }

    fn alloc_id(&self) -> String {
        let n = self.next_id.fetch_add(1, Ordering::Relaxed);
        format!("mcp-{n}")
    }

    pub async fn send_command(&self, action: &str, params: Value) -> Result<Value> {
        self.send_command_timeout(action, params, read_timeout_for(action)).await
    }

    pub async fn send_command_timeout(&self, action: &str, params: Value, read_timeout: Duration) -> Result<Value> {
        let (addr, token, source) = self.resolve();
        // P1-06: Timeout for connect (800ms) and read
        let connect_fut = TcpStream::connect(&addr);
        let stream = timeout(Duration::from_millis(800), connect_fut)
            .await
            .map_err(|_| anyhow::anyhow!("Bridge connect timeout 800ms to {addr} (ANIGO Studio not responding)"))?
            .with_context(|| format!("ANIGO Studio is not running on {addr} ({source}); se o arquivo de descoberta for de uma execução anterior, abra o studio com --bridge"))?;

        stream.set_nodelay(true).ok();

        // P-05: monotonic request id, not wall-clock millis
        let req_id = self.alloc_id();

        // P2-14: attach auth token to every request
        let payload = json!({ "id": req_id, "action": action, "params": params, "token": token });

        let mut line = serde_json::to_string(&payload)?;
        line.push('\n');

        // Protect against huge payloads
        if line.len() > MAX_REQUEST {
            anyhow::bail!("Bridge payload too large: {} > {} bytes", line.len(), MAX_REQUEST);
        }

        let mut stream = stream;
        timeout(Duration::from_secs(2), stream.write_all(line.as_bytes()))
            .await
            .map_err(|_| anyhow::anyhow!("Bridge write timeout to {addr}"))?
            .context("Failed to write to bridge")?;
        timeout(Duration::from_secs(1), stream.flush())
            .await
            .map_err(|_| anyhow::anyhow!("Bridge flush timeout"))?
            .context("Failed to flush bridge")?;

        // a resposta é lida com teto: um servidor defeituoso não pode esgotar a memória
        let mut reader = BufReader::new(stream.take(MAX_RESPONSE as u64 + 1));
        let mut response_line = String::new();
        let secs = read_timeout.as_secs();
        timeout(read_timeout, reader.read_line(&mut response_line))
            .await
            .map_err(|_| anyhow::anyhow!("Bridge read timeout ({secs}s) from {addr} for action {action}"))?
            .context("Failed to read from bridge")?;

        if response_line.len() > MAX_RESPONSE {
            anyhow::bail!("Bridge response too large: > {MAX_RESPONSE} bytes");
        }
        if response_line.trim().is_empty() {
            anyhow::bail!("Empty response from bridge for action {action}");
        }

        let resp: Value = serde_json::from_str(&response_line).with_context(|| {
            let head: String = response_line.chars().take(200).collect();
            format!("Failed to parse bridge response: {head}")
        })?;

        if !resp.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
            let err = resp.get("error").and_then(|v| v.as_str()).unwrap_or("Unknown bridge error");
            let code = resp.get("code").and_then(|v| v.as_i64());
            // P2-14: detect auth errors explicitly (código -32001 do servidor nativo; texto para o Tauri)
            let is_auth = code == Some(-32001) || (code.is_none() && (err.contains("Unauthorized") || err.contains("token") || err.contains("auth")));
            if is_auth {
                if token.is_empty() {
                    anyhow::bail!(
                        "Bridge auth required but no token was found. Start ANIGO Studio with --bridge (the token is read from the discovery file) \
                         or set ANIGO_BRIDGE_TOKEN=<token> (error: {err})"
                    );
                } else {
                    anyhow::bail!(
                        "Bridge rejected the token ({}…): {err}. Make sure the MCP talks to the same ANIGO Studio instance (target: {source}).",
                        &token[..token.len().min(4)]
                    );
                }
            }
            return Err(anyhow::Error::new(BridgeCallError { code, message: format!("Live bridge action failed [{action}]: {err}") }));
        }

        Ok(resp.get("data").cloned().unwrap_or(Value::Null))
    }

    pub async fn is_live(&self) -> bool {
        // is_live should be fast: use timeout already inside send_command
        matches!(timeout(Duration::from_millis(900), self.send_command("PING", json!({}))).await, Ok(Ok(_)))
    }
}
