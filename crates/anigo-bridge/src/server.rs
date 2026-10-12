//! Servidor da bridge: `std::net` + uma thread por conexão (limitada), sem runtime async.
//!
//! Uma única porta (loopback, efêmera) fala dois dialetos, escolhidos pelo 1º byte:
//! * `{` (ou espaço/linha em branco) => **NDJSON** (o protocolo do `bridge_client.rs`);
//! * letra maiúscula => **HTTP/1.1 mínimo** (`POST /rpc`, `GET /health`) com validação de `Host`/`Origin`.
//!
//! Segurança (ver `docs/native/MCP_BRIDGE_NATIVA.md`): bind só 127.0.0.1; token de 256 bits comparado em
//! tempo constante; auth ANTES do rate limit; limites de corpo/linha/conexões/tempo; falha de auth fecha a
//! conexão e atrasa 100 ms; `Origin` presente => 403; `Host` exato.

use crate::discovery::{self, Discovery};
use crate::http;
use crate::protocol::{self, code, response_json, BridgeError, BridgeResult, Request, PROTOCOL_VERSION};
use serde_json::{json, Value};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Quem executa as ações (a UI do studio, ou uma sessão de teste).
pub trait Backend: Send + Sync + 'static {
    fn call(&self, req: &Request) -> BridgeResult;
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// 0 = efêmera.
    pub port: u16,
    /// `None` = gera um token novo (256 bits).
    pub token: Option<String>,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub max_connections: usize,
    pub rate_per_sec: u32,
    pub rate_burst: u32,
    /// Tempo para completar a linha/cabeçalhos depois do primeiro byte (slowloris).
    pub header_timeout: Duration,
    /// Tempo para o corpo HTTP chegar.
    pub body_timeout: Duration,
    /// Ociosidade máxima de uma conexão NDJSON entre requisições.
    pub idle_timeout: Duration,
    pub write_timeout: Duration,
    /// Atraso aplicado após uma falha de autenticação.
    pub auth_failure_delay: Duration,
    /// Arquivo de descoberta (`None` = não grava; testes).
    pub discovery: Option<PathBuf>,
    pub engine: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 0,
            token: None,
            max_request_bytes: protocol::MAX_REQUEST_BYTES,
            max_response_bytes: protocol::MAX_RESPONSE_BYTES,
            max_connections: 32,
            rate_per_sec: 100,
            rate_burst: 200,
            header_timeout: Duration::from_secs(5),
            body_timeout: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(30),
            write_timeout: Duration::from_secs(10),
            auth_failure_delay: Duration::from_millis(100),
            discovery: None,
            engine: "ANIGO Studio (native)".into(),
        }
    }
}

#[derive(Default)]
pub struct Metrics {
    pub connections_accepted: AtomicU64,
    pub connections_rejected_limit: AtomicU64,
    pub requests_total: AtomicU64,
    pub requests_success: AtomicU64,
    pub requests_error: AtomicU64,
    pub rejected_auth: AtomicU64,
    pub rejected_rate_limit: AtomicU64,
    pub rejected_host_origin: AtomicU64,
    pub rejected_too_large: AtomicU64,
    pub invalid_frames: AtomicU64,
    pub timeouts: AtomicU64,
    pub http_requests: AtomicU64,
    pub bytes_read: AtomicU64,
    pub bytes_written: AtomicU64,
}

impl Metrics {
    pub fn snapshot(&self) -> Value {
        json!({
            "connections_accepted": self.connections_accepted.load(Relaxed),
            "connections_rejected_limit": self.connections_rejected_limit.load(Relaxed),
            "requests_total": self.requests_total.load(Relaxed),
            "requests_success": self.requests_success.load(Relaxed),
            "requests_error": self.requests_error.load(Relaxed),
            "rejected_auth": self.rejected_auth.load(Relaxed),
            "rejected_rate_limit": self.rejected_rate_limit.load(Relaxed),
            "rejected_host_origin": self.rejected_host_origin.load(Relaxed),
            "rejected_too_large": self.rejected_too_large.load(Relaxed),
            "invalid_frames": self.invalid_frames.load(Relaxed),
            "timeouts": self.timeouts.load(Relaxed),
            "http_requests": self.http_requests.load(Relaxed),
            "bytes_read": self.bytes_read.load(Relaxed),
            "bytes_written": self.bytes_written.load(Relaxed),
        })
    }
}

/// Token bucket global (todo cliente é loopback, então por IP não faria sentido).
struct Bucket {
    tokens: f64,
    last: Instant,
    rate: f64,
    burst: f64,
}

impl Bucket {
    fn new(rate: u32, burst: u32) -> Self {
        Self { tokens: burst as f64, last: Instant::now(), rate: rate as f64, burst: burst as f64 }
    }
    fn take(&mut self) -> bool {
        let now = Instant::now();
        self.tokens = (self.tokens + now.duration_since(self.last).as_secs_f64() * self.rate).min(self.burst);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

struct Shared {
    cfg: ServerConfig,
    token: String,
    port: u16,
    backend: Arc<dyn Backend>,
    stop: AtomicBool,
    metrics: Metrics,
    bucket: Mutex<Bucket>,
    conns: AtomicUsize,
    started: Instant,
}

pub struct BridgeServer {
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
    discovery: Option<(PathBuf, Discovery)>,
}

impl BridgeServer {
    pub fn start(cfg: ServerConfig, backend: Arc<dyn Backend>) -> io::Result<BridgeServer> {
        let token = match cfg.token.clone() {
            Some(t) if !t.is_empty() => t,
            _ => protocol::generate_token().map_err(io::Error::other)?,
        };
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), cfg.port))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let shared = Arc::new(Shared {
            bucket: Mutex::new(Bucket::new(cfg.rate_per_sec.max(1), cfg.rate_burst.max(1))),
            cfg: cfg.clone(),
            token: token.clone(),
            port,
            backend,
            stop: AtomicBool::new(false),
            metrics: Metrics::default(),
            conns: AtomicUsize::new(0),
            started: Instant::now(),
        });
        let discovery = match &cfg.discovery {
            Some(path) => {
                let d = Discovery {
                    addr: format!("127.0.0.1:{port}"),
                    port,
                    token,
                    pid: std::process::id(),
                    started_at: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    protocol: PROTOCOL_VERSION,
                };
                discovery::write(path, &d)?;
                Some((path.clone(), d))
            }
            None => None,
        };
        let sh = Arc::clone(&shared);
        let accept = std::thread::Builder::new().name("anigo-bridge-accept".into()).spawn(move || accept_loop(listener, sh))?;
        tracing::info!(port, "bridge ligada em 127.0.0.1");
        Ok(BridgeServer { shared, accept: Some(accept), discovery })
    }

    pub fn port(&self) -> u16 {
        self.shared.port
    }
    pub fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.shared.port)
    }
    pub fn token(&self) -> &str {
        &self.shared.token
    }
    pub fn discovery_path(&self) -> Option<&PathBuf> {
        self.discovery.as_ref().map(|(p, _)| p)
    }
    pub fn metrics(&self) -> &Metrics {
        &self.shared.metrics
    }
    /// Conexões abertas agora.
    pub fn open_connections(&self) -> usize {
        self.shared.conns.load(Relaxed)
    }

    pub fn shutdown(mut self) {
        self.stop_inner();
    }

    fn stop_inner(&mut self) {
        self.shared.stop.store(true, Relaxed);
        if let Some(h) = self.accept.take() {
            let _ = h.join();
        }
        if let Some((path, d)) = self.discovery.take() {
            discovery::remove_if_owner(&path, d.pid, &d.token);
        }
    }
}

impl Drop for BridgeServer {
    fn drop(&mut self) {
        self.stop_inner();
    }
}

fn accept_loop(listener: TcpListener, sh: Arc<Shared>) {
    while !sh.stop.load(Relaxed) {
        match listener.accept() {
            Ok((stream, peer)) => {
                if !peer.ip().is_loopback() {
                    continue; // defesa em profundidade; o bind já é só loopback
                }
                if sh.conns.load(Relaxed) >= sh.cfg.max_connections {
                    sh.metrics.connections_rejected_limit.fetch_add(1, Relaxed);
                    let _ = stream.shutdown(Shutdown::Both);
                    continue;
                }
                sh.conns.fetch_add(1, Relaxed);
                sh.metrics.connections_accepted.fetch_add(1, Relaxed);
                let sh2 = Arc::clone(&sh);
                let spawned = std::thread::Builder::new().name("anigo-bridge-conn".into()).stack_size(512 * 1024).spawn(move || {
                    let _guard = ConnGuard(Arc::clone(&sh2));
                    handle_conn(&sh2, stream);
                });
                if spawned.is_err() {
                    sh.conns.fetch_sub(1, Relaxed);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(15)),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => {
                tracing::warn!("accept da bridge: {e}");
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

struct ConnGuard(Arc<Shared>);
impl Drop for ConnGuard {
    fn drop(&mut self) {
        self.0.conns.fetch_sub(1, Relaxed);
    }
}

// ------------------------------------------------------------------------------------------------ leitura limitada

enum ReadOutcome {
    Line(Vec<u8>),
    Eof,
    TooLong,
    Timeout,
    Stopped,
    Io,
}

struct Reader {
    r: BufReader<TcpStream>,
}

impl Reader {
    /// Espera o 1º byte (sem consumir). `Ok(None)` = EOF/timeout/parada.
    fn peek_first(&mut self, sh: &Shared, idle: Duration) -> Option<u8> {
        let start = Instant::now();
        loop {
            if sh.stop.load(Relaxed) {
                return None;
            }
            match self.r.fill_buf() {
                Ok([]) => return None,
                Ok(b) => return Some(b[0]),
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                    if start.elapsed() > idle {
                        sh.metrics.timeouts.fetch_add(1, Relaxed);
                        return None;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return None,
            }
        }
    }

    /// Lê uma linha (sem o `\n`/`\r\n`) com teto de tamanho. `idle`: espera pelo 1º byte; `complete`: prazo
    /// para terminar a linha depois do 1º byte (anti-slowloris).
    fn read_line(&mut self, sh: &Shared, max: usize, idle: Duration, complete: Duration) -> ReadOutcome {
        let start = Instant::now();
        let mut first: Option<Instant> = None;
        let mut buf: Vec<u8> = Vec::new();
        loop {
            if sh.stop.load(Relaxed) {
                return ReadOutcome::Stopped;
            }
            match self.r.fill_buf() {
                Ok([]) => return if buf.is_empty() { ReadOutcome::Eof } else { ReadOutcome::Io },
                Ok(b) => {
                    first.get_or_insert_with(Instant::now);
                    let nl = b.iter().position(|&c| c == b'\n');
                    let take = nl.map(|i| i + 1).unwrap_or(b.len());
                    buf.extend_from_slice(&b[..take]);
                    self.r.consume(take);
                    sh.metrics.bytes_read.fetch_add(take as u64, Relaxed);
                    if nl.is_some() {
                        buf.pop();
                        if buf.last() == Some(&b'\r') {
                            buf.pop();
                        }
                        return if buf.len() > max { ReadOutcome::TooLong } else { ReadOutcome::Line(buf) };
                    }
                    if buf.len() > max + 1 {
                        return ReadOutcome::TooLong;
                    }
                }
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                    let expired = match first {
                        None => start.elapsed() > idle,
                        Some(t) => t.elapsed() > complete,
                    };
                    if expired {
                        sh.metrics.timeouts.fetch_add(1, Relaxed);
                        return ReadOutcome::Timeout;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return ReadOutcome::Io,
            }
        }
    }

    /// Lê exatamente `n` bytes dentro do prazo.
    fn read_body(&mut self, sh: &Shared, n: usize, deadline: Duration) -> Option<Vec<u8>> {
        let start = Instant::now();
        let mut out = Vec::with_capacity(n.min(1 << 20));
        while out.len() < n {
            if sh.stop.load(Relaxed) || start.elapsed() > deadline {
                sh.metrics.timeouts.fetch_add(1, Relaxed);
                return None;
            }
            match self.r.fill_buf() {
                Ok([]) => return None,
                Ok(b) => {
                    let take = b.len().min(n - out.len());
                    out.extend_from_slice(&b[..take]);
                    self.r.consume(take);
                    sh.metrics.bytes_read.fetch_add(take as u64, Relaxed);
                }
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => {}
                Err(_) => return None,
            }
        }
        Some(out)
    }
}

fn write_all(sh: &Shared, stream: &mut TcpStream, bytes: &[u8]) -> bool {
    let ok = stream.write_all(bytes).and_then(|_| stream.flush()).is_ok();
    if ok {
        sh.metrics.bytes_written.fetch_add(bytes.len() as u64, Relaxed);
    }
    ok
}

// ------------------------------------------------------------------------------------------------ conexão

fn handle_conn(sh: &Shared, stream: TcpStream) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let _ = stream.set_write_timeout(Some(sh.cfg.write_timeout));
    let Ok(read_half) = stream.try_clone() else { return };
    let mut writer = stream;
    let mut rd = Reader { r: BufReader::with_capacity(16 * 1024, read_half) };

    let Some(first) = rd.peek_first(sh, sh.cfg.idle_timeout.min(sh.cfg.header_timeout * 2)) else {
        let _ = writer.shutdown(Shutdown::Both);
        return;
    };
    match first {
        b'{' | b' ' | b'\t' | b'\r' | b'\n' => ndjson_loop(sh, &mut rd, &mut writer),
        b'A'..=b'Z' => http_one(sh, &mut rd, &mut writer),
        _ => {
            sh.metrics.invalid_frames.fetch_add(1, Relaxed);
            let line = line_bytes(&response_json("", &Err(BridgeError::new(code::INVALID_REQUEST, "protocolo não reconhecido (esperado NDJSON ou HTTP)"))));
            let _ = write_all(sh, &mut writer, &line);
        }
    }
    finish(sh, &mut rd, &writer);
}

/// Fecha a escrita e descarta o que o cliente ainda mandar (um RST por dados não lidos poderia destruir a
/// resposta antes de o cliente lê-la, ex.: corpo grande recusado com 413).
fn finish(sh: &Shared, rd: &mut Reader, writer: &TcpStream) {
    let _ = writer.shutdown(Shutdown::Write);
    let start = Instant::now();
    let mut drained = 0usize;
    let mut sink = [0u8; 16 * 1024];
    while start.elapsed() < Duration::from_secs(1) && drained < (16 << 20) && !sh.stop.load(Relaxed) {
        match rd.r.read(&mut sink) {
            Ok(0) => break,
            Ok(n) => drained += n,
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => {}
            Err(_) => break,
        }
    }
    let _ = writer.shutdown(Shutdown::Both);
}

fn line_bytes(v: &Value) -> Vec<u8> {
    let mut b = serde_json::to_vec(v).unwrap_or_else(|_| b"{}".to_vec());
    b.push(b'\n');
    b
}

fn ndjson_loop(sh: &Shared, rd: &mut Reader, writer: &mut TcpStream) {
    let mut invalid = 0;
    loop {
        match rd.read_line(sh, sh.cfg.max_request_bytes, sh.cfg.idle_timeout, sh.cfg.header_timeout) {
            ReadOutcome::Line(buf) => {
                if buf.iter().all(|c| c.is_ascii_whitespace()) {
                    continue;
                }
                let req: Request = match serde_json::from_slice(&buf) {
                    Ok(r) => r,
                    Err(e) => {
                        sh.metrics.invalid_frames.fetch_add(1, Relaxed);
                        invalid += 1;
                        let r = Err(BridgeError::new(code::INVALID_REQUEST, format!("Invalid JSON frame: {e}")));
                        if !write_all(sh, writer, &line_bytes(&response_json("", &r))) || invalid >= 3 {
                            return;
                        }
                        continue;
                    }
                };
                invalid = 0; // só falhas CONSECUTIVAS contam
                let id = req.id_string();
                let (result, close) = process(sh, &req, None);
                let mut bytes = encode(sh, &id, result);
                bytes.push(b'\n');
                if !write_all(sh, writer, &bytes) || close {
                    return;
                }
            }
            ReadOutcome::TooLong => {
                sh.metrics.rejected_too_large.fetch_add(1, Relaxed);
                let r = Err(BridgeError::new(code::TOO_LARGE, format!("Frame too large (> {} bytes)", sh.cfg.max_request_bytes)));
                let _ = write_all(sh, writer, &line_bytes(&response_json("", &r)));
                return;
            }
            ReadOutcome::Eof | ReadOutcome::Timeout | ReadOutcome::Stopped | ReadOutcome::Io => return,
        }
    }
}

/// Auth -> rate limit -> ações embutidas -> backend. O `bool` pede para fechar a conexão.
fn process(sh: &Shared, req: &Request, header_token: Option<&str>) -> (BridgeResult, bool) {
    sh.metrics.requests_total.fetch_add(1, Relaxed);
    let presented = header_token.or(req.token.as_deref()).unwrap_or("");
    if !protocol::ct_eq(presented, &sh.token) {
        sh.metrics.rejected_auth.fetch_add(1, Relaxed);
        sh.metrics.requests_error.fetch_add(1, Relaxed);
        std::thread::sleep(sh.cfg.auth_failure_delay);
        return (Err(BridgeError::new(code::UNAUTHORIZED, "Unauthorized: missing or invalid token (code -32001)")), true);
    }
    if !sh.bucket.lock().map(|mut b| b.take()).unwrap_or(false) {
        sh.metrics.rejected_rate_limit.fetch_add(1, Relaxed);
        sh.metrics.requests_error.fetch_add(1, Relaxed);
        return (Err(BridgeError::new(code::RATE_LIMITED, "Rate limit exceeded")), false);
    }
    let result = match req.action.as_str() {
        "PING" => Ok(json!({ "pong": true, "engine": sh.cfg.engine, "protocol": PROTOCOL_VERSION, "version": env!("CARGO_PKG_VERSION") })),
        "GET_METRICS" => {
            let mut m = sh.metrics.snapshot();
            m["uptime_seconds"] = json!(sh.started.elapsed().as_secs_f64());
            m["open_connections"] = json!(sh.conns.load(Relaxed));
            Ok(m)
        }
        _ => sh.backend.call(req),
    };
    match &result {
        Ok(_) => sh.metrics.requests_success.fetch_add(1, Relaxed),
        Err(_) => sh.metrics.requests_error.fetch_add(1, Relaxed),
    };
    (result, false)
}

/// Serializa a resposta UMA vez; acima do teto vira um erro curto (nunca uma resposta gigante).
fn encode(sh: &Shared, id: &str, result: BridgeResult) -> Vec<u8> {
    let ok = result.is_ok();
    let bytes = serde_json::to_vec(&response_json(id, &result)).unwrap_or_else(|_| b"{}".to_vec());
    if ok && bytes.len() > sh.cfg.max_response_bytes {
        sh.metrics.rejected_too_large.fetch_add(1, Relaxed);
        let e = Err(BridgeError::new(code::TOO_LARGE, format!("Response too large (> {} bytes)", sh.cfg.max_response_bytes)));
        return serde_json::to_vec(&response_json(id, &e)).unwrap_or_else(|_| b"{}".to_vec());
    }
    bytes
}

// ------------------------------------------------------------------------------------------------ HTTP

fn http_reply(sh: &Shared, w: &mut TcpStream, status: u16, body: &Value, extra: &[(&str, &str)]) {
    let bytes = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    let _ = write_all(sh, w, &http::response(status, &bytes, extra));
}

fn http_error(sh: &Shared, w: &mut TcpStream, status: u16, message: &str, extra: &[(&str, &str)]) {
    let code = match status {
        401 => code::UNAUTHORIZED,
        413 => code::TOO_LARGE,
        429 => code::RATE_LIMITED,
        _ => code::INVALID_REQUEST,
    };
    http_reply(sh, w, status, &response_json("", &Err(BridgeError::new(code, message))), extra);
}

fn http_one(sh: &Shared, rd: &mut Reader, w: &mut TcpStream) {
    sh.metrics.http_requests.fetch_add(1, Relaxed);
    // ---- cabeçalhos (com teto total e prazo único)
    let started = Instant::now();
    let mut lines: Vec<String> = Vec::new();
    let mut total = 0usize;
    loop {
        let remaining = sh.cfg.header_timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            sh.metrics.timeouts.fetch_add(1, Relaxed);
            return http_error(sh, w, 408, "tempo esgotado lendo os cabeçalhos", &[]);
        }
        match rd.read_line(sh, http::MAX_REQUEST_LINE.max(http::MAX_HEADER_BYTES), remaining, remaining) {
            ReadOutcome::Line(buf) => {
                total += buf.len() + 2;
                if total > http::MAX_HEADER_BYTES {
                    return http_error(sh, w, 431, "cabeçalhos grandes demais", &[]);
                }
                if buf.is_empty() {
                    if lines.is_empty() {
                        continue; // linhas em branco antes da requisição são toleradas pela RFC
                    }
                    break;
                }
                match String::from_utf8(buf) {
                    Ok(s) => lines.push(s),
                    Err(_) => return http_error(sh, w, 400, "cabeçalhos devem ser UTF-8/ASCII", &[]),
                }
            }
            ReadOutcome::TooLong => return http_error(sh, w, 431, "linha de cabeçalho longa demais", &[]),
            ReadOutcome::Timeout => return http_error(sh, w, 408, "tempo esgotado lendo os cabeçalhos", &[]),
            ReadOutcome::Eof | ReadOutcome::Stopped | ReadOutcome::Io => return,
        }
    }
    let head = match http::parse_head(&lines) {
        Ok(h) => h,
        Err(e) => return http_error(sh, w, e.status, e.message, &[]),
    };

    // ---- Host / Origin ANTES de qualquer coisa (DNS rebinding e páginas web)
    match head.header("host") {
        None => return http_error(sh, w, 400, "Host ausente", &[]),
        Some(h) if !http::host_allowed(h, sh.port) => {
            sh.metrics.rejected_host_origin.fetch_add(1, Relaxed);
            return http_error(sh, w, 403, "Host não permitido (apenas loopback com a porta da bridge)", &[]);
        }
        Some(_) => {}
    }
    if head.header("origin").is_some() {
        sh.metrics.rejected_host_origin.fetch_add(1, Relaxed);
        return http_error(sh, w, 403, "requisições com Origin (navegador) não são aceitas", &[]);
    }

    match (head.method.as_str(), head.target.as_str()) {
        ("GET", "/health") => http_reply(sh, w, 200, &json!({ "ok": true, "engine": sh.cfg.engine, "protocol": PROTOCOL_VERSION }), &[]),
        ("POST", "/rpc") => {
            if !head.header("content-type").map(http::is_json_content_type).unwrap_or(false) {
                return http_error(sh, w, 415, "Content-Type deve ser application/json", &[]);
            }
            if head.header("transfer-encoding").is_some() {
                return http_error(sh, w, 400, "Transfer-Encoding não é suportado; use Content-Length", &[]);
            }
            let len = match head.header("content-length") {
                None => return http_error(sh, w, 411, "Content-Length é obrigatório", &[]),
                Some(v) => match v.parse::<usize>() {
                    Ok(n) if v.bytes().all(|c| c.is_ascii_digit()) => n,
                    _ => return http_error(sh, w, 400, "Content-Length inválido", &[]),
                },
            };
            if len > sh.cfg.max_request_bytes {
                sh.metrics.rejected_too_large.fetch_add(1, Relaxed);
                return http_error(sh, w, 413, "corpo grande demais", &[]);
            }
            let Some(body) = rd.read_body(sh, len, sh.cfg.body_timeout) else {
                return http_error(sh, w, 408, "corpo incompleto", &[]);
            };
            let req: Request = match serde_json::from_slice(&body) {
                Ok(r) => r,
                Err(e) => {
                    sh.metrics.invalid_frames.fetch_add(1, Relaxed);
                    return http_error(sh, w, 400, &format!("JSON inválido: {e}"), &[]);
                }
            };
            let header_token = head.header("authorization").and_then(http::bearer);
            if head.header("authorization").is_some() && header_token.is_none() {
                sh.metrics.rejected_auth.fetch_add(1, Relaxed);
                return http_error(sh, w, 401, "Authorization deve usar o esquema Bearer", &[("WWW-Authenticate", "Bearer")]);
            }
            let id = req.id_string();
            let (result, _) = process(sh, &req, header_token);
            let status = match result.as_ref().err().map(|e| e.code) {
                Some(code::UNAUTHORIZED) => 401,
                Some(code::RATE_LIMITED) => 429,
                Some(code::BUSY) | Some(code::SHUTTING_DOWN) => 503,
                Some(code::TOO_LARGE) => 413,
                _ => 200,
            };
            let extra: &[(&str, &str)] = if status == 401 { &[("WWW-Authenticate", "Bearer")] } else { &[] };
            let bytes = encode(sh, &id, result);
            let _ = write_all(sh, w, &http::response(status, &bytes, extra));
        }
        (_, "/health") => http_error(sh, w, 405, "use GET", &[("Allow", "GET")]),
        (_, "/rpc") => http_error(sh, w, 405, "use POST", &[("Allow", "POST")]),
        _ => http_error(sh, w, 404, "não encontrado", &[]),
    }
}
