//! Testes de protocolo, autenticação, limites e concorrência da bridge (sockets reais em 127.0.0.1).

use anigo_bridge::protocol::code;
use anigo_bridge::{channel, core_ops, Backend, BridgeError, BridgeResult, BridgeServer, Request, ServerConfig};
use anigo_session::CoreSession;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

struct FnBackend<F: Fn(&Request) -> BridgeResult + Send + Sync + 'static>(F);
impl<F: Fn(&Request) -> BridgeResult + Send + Sync + 'static> Backend for FnBackend<F> {
    fn call(&self, req: &Request) -> BridgeResult {
        (self.0)(req)
    }
}

fn echo_backend() -> Arc<dyn Backend> {
    Arc::new(FnBackend(|r: &Request| match r.action.as_str() {
        "ECHO" => Ok(r.params.clone()),
        "BIG" => Ok(json!({"blob": "x".repeat(r.params["n"].as_u64().unwrap_or(0) as usize)})),
        "FAIL" => Err(BridgeError::failed("falhou de propósito")),
        other => Err(BridgeError::unknown_action(other)),
    }))
}

fn cfg() -> ServerConfig {
    ServerConfig {
        token: Some(TOKEN.into()),
        auth_failure_delay: Duration::from_millis(5),
        header_timeout: Duration::from_millis(400),
        body_timeout: Duration::from_millis(600),
        idle_timeout: Duration::from_secs(5),
        rate_per_sec: 100_000,
        rate_burst: 100_000,
        ..Default::default()
    }
}

fn start(cfg: ServerConfig, backend: Arc<dyn Backend>) -> BridgeServer {
    BridgeServer::start(cfg, backend).expect("bridge sobe")
}

fn connect(s: &BridgeServer) -> TcpStream {
    let c = TcpStream::connect(("127.0.0.1", s.port())).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    c
}

fn read_line(c: &mut TcpStream) -> Option<String> {
    let mut out = Vec::new();
    let mut b = [0u8; 1];
    loop {
        match c.read(&mut b) {
            Ok(0) => return if out.is_empty() { None } else { Some(String::from_utf8_lossy(&out).into()) },
            Ok(_) => {
                if b[0] == b'\n' {
                    return Some(String::from_utf8_lossy(&out).into());
                }
                out.push(b[0]);
            }
            Err(_) => return None,
        }
    }
}

fn nd(c: &mut TcpStream, v: Value) -> Value {
    let mut line = v.to_string();
    line.push('\n');
    c.write_all(line.as_bytes()).unwrap();
    serde_json::from_str(&read_line(c).expect("resposta")).expect("json")
}

fn req(action: &str, params: Value) -> Value {
    json!({"id": "t", "action": action, "params": params, "token": TOKEN})
}

fn is_closed(c: &mut TcpStream) -> bool {
    let mut b = [0u8; 1];
    matches!(c.read(&mut b), Ok(0) | Err(_))
}

/// Resposta HTTP crua: (status, cabeçalhos em minúsculas, corpo).
fn http_raw(s: &BridgeServer, raw: &[u8]) -> (u16, String, String) {
    let mut c = connect(s);
    let _ = c.write_all(raw);
    let mut buf = Vec::new();
    let _ = c.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf).to_string();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    (status, head.to_ascii_lowercase(), body.to_string())
}

fn post(s: &BridgeServer, extra_headers: &str, body: &str) -> (u16, String, String) {
    let raw = format!(
        "POST /rpc HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra_headers}\r\n{body}",
        s.port(),
        body.len()
    );
    http_raw(s, raw.as_bytes())
}

// ------------------------------------------------------------------------------------------ NDJSON

#[test]
fn ndjson_ping_echo_and_errors_keep_the_legacy_envelope() {
    let s = start(cfg(), echo_backend());
    let mut c = connect(&s);
    let r = nd(&mut c, req("PING", json!({})));
    assert_eq!(r["success"], true);
    assert_eq!(r["id"], "t");
    assert_eq!(r["data"]["pong"], true);
    let r = nd(&mut c, req("ECHO", json!({"a": [1, 2, 3]})));
    assert_eq!(r["data"]["a"][2], 3);
    let r = nd(&mut c, req("FAIL", json!({})));
    assert_eq!((r["success"].as_bool(), r["error"].as_str(), r["code"].as_i64()), (Some(false), Some("falhou de propósito"), Some(code::ACTION_FAILED as i64)));
    let r = nd(&mut c, req("NOPE", json!({})));
    assert_eq!(r["code"], code::UNKNOWN_ACTION);
    // a mesma conexão continua útil depois de erros de ação
    assert_eq!(nd(&mut c, req("PING", json!({})))["success"], true);
}

#[test]
fn ndjson_wrong_missing_or_empty_token_is_rejected_and_the_connection_closed() {
    let s = start(cfg(), echo_backend());
    for token in [Some("errado"), Some(""), None, Some(&TOKEN[..63]), Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdeg")] {
        let mut c = connect(&s);
        let mut r = json!({"id": "x", "action": "PING"});
        if let Some(t) = token {
            r["token"] = json!(t);
        }
        let resp = nd(&mut c, r);
        assert_eq!(resp["success"], false, "{token:?}");
        assert_eq!(resp["code"], code::UNAUTHORIZED);
        assert!(resp["error"].as_str().unwrap().contains("Unauthorized"));
        assert!(is_closed(&mut c), "conexão deve fechar após falha de auth");
    }
    assert_eq!(s.metrics().rejected_auth.load(Relaxed), 5);
    // nenhuma ação chegou ao backend com token inválido
}

#[test]
fn backend_is_never_called_without_a_valid_token() {
    let called = Arc::new(AtomicUsize::new(0));
    let c2 = Arc::clone(&called);
    let s = start(cfg(), Arc::new(FnBackend(move |_r: &Request| {
        c2.fetch_add(1, Relaxed);
        Ok(Value::Null)
    })));
    let mut c = connect(&s);
    let _ = nd(&mut c, json!({"id": 1, "action": "APPLY_COMMAND", "params": {}}));
    assert_eq!(called.load(Relaxed), 0);
    let mut c = connect(&s);
    let _ = nd(&mut c, req("APPLY_COMMAND", json!({})));
    assert_eq!(called.load(Relaxed), 1);
}

#[test]
fn ndjson_invalid_json_gets_errors_then_the_connection_is_dropped() {
    let s = start(cfg(), echo_backend());
    let mut c = connect(&s);
    for _ in 0..2 {
        c.write_all(b"{ isto nao e json\n").unwrap();
        let r: Value = serde_json::from_str(&read_line(&mut c).unwrap()).unwrap();
        assert_eq!(r["code"], code::INVALID_REQUEST);
    }
    // o cliente ainda pode se recuperar com um quadro válido
    assert_eq!(nd(&mut c, req("PING", json!({})))["success"], true);
    for _ in 0..3 {
        let _ = c.write_all(b"{ lixo\n");
    }
    // 3 falhas consecutivas: o servidor responde às 3 e fecha (lemos até o EOF)
    let mut replies = 0;
    while read_line(&mut c).is_some() {
        replies += 1;
    }
    assert_eq!(replies, 3);
    assert_eq!(s.metrics().invalid_frames.load(Relaxed), 5);
}

#[test]
fn unknown_first_byte_is_refused() {
    let s = start(cfg(), echo_backend());
    let mut c = connect(&s);
    c.write_all(b"\x16\x03\x01\x02\x00 (tls?)").unwrap();
    let r: Value = serde_json::from_str(&read_line(&mut c).unwrap()).unwrap();
    assert_eq!(r["code"], code::INVALID_REQUEST);
}

#[test]
fn ndjson_oversized_frame_is_refused_without_buffering_it_all() {
    let mut c0 = cfg();
    c0.max_request_bytes = 4096;
    let s = start(c0, echo_backend());
    let mut c = connect(&s);
    let mut big = format!("{{\"id\":\"b\",\"action\":\"ECHO\",\"token\":\"{TOKEN}\",\"params\":{{\"x\":\"");
    big.push_str(&"a".repeat(200_000));
    big.push_str("\"}}\n");
    let _ = c.write_all(big.as_bytes());
    let r: Value = serde_json::from_str(&read_line(&mut c).expect("resposta de erro")).unwrap();
    assert_eq!(r["code"], code::TOO_LARGE);
    assert!(is_closed(&mut c));
    assert!(s.metrics().rejected_too_large.load(Relaxed) >= 1);
    // um quadro dentro do limite passa
    let mut c = connect(&s);
    assert_eq!(nd(&mut c, req("ECHO", json!({"x": "ok"})))["data"]["x"], "ok");
}

#[test]
fn slow_partial_lines_are_cut_by_the_deadline() {
    let s = start(cfg(), echo_backend());
    let mut c = connect(&s);
    c.write_all(b"{\"id\":\"slow\"").unwrap(); // nunca termina a linha
    let t = Instant::now();
    assert!(is_closed(&mut c));
    assert!(t.elapsed() < Duration::from_secs(3), "slowloris deve cair em ~header_timeout, levou {:?}", t.elapsed());
    assert!(s.metrics().timeouts.load(Relaxed) >= 1);
}

#[test]
fn idle_connections_time_out_and_are_released() {
    let mut c0 = cfg();
    c0.idle_timeout = Duration::from_millis(300);
    let s = start(c0, echo_backend());
    let mut c = connect(&s);
    assert!(is_closed(&mut c));
    for _ in 0..40 {
        if s.open_connections() == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(s.open_connections(), 0);
}

#[test]
fn response_over_the_cap_becomes_a_short_error() {
    let mut c0 = cfg();
    c0.max_response_bytes = 10_000;
    let s = start(c0, echo_backend());
    let mut c = connect(&s);
    let r = nd(&mut c, req("BIG", json!({"n": 100_000})));
    assert_eq!(r["success"], false);
    assert_eq!(r["code"], code::TOO_LARGE);
    let r = nd(&mut c, req("BIG", json!({"n": 100})));
    assert_eq!(r["success"], true);
}

#[test]
fn rate_limit_rejects_bursts_but_not_unauthenticated_floods_stealing_the_budget() {
    let mut c0 = cfg();
    c0.rate_per_sec = 2;
    c0.rate_burst = 5;
    let s = start(c0, echo_backend());
    let mut c = connect(&s);
    let mut limited = 0;
    for _ in 0..30 {
        let r = nd(&mut c, req("PING", json!({})));
        if r["code"] == code::RATE_LIMITED {
            limited += 1;
        }
    }
    assert!(limited >= 20, "esperava rajada limitada, limitadas={limited}");
    assert!(s.metrics().rejected_rate_limit.load(Relaxed) >= 20);
    // espera repor o balde: volta a aceitar
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(nd(&mut c, req("PING", json!({})))["success"], true);
    // requisições SEM token não consomem o balde (auth vem antes)
    let before = s.metrics().rejected_rate_limit.load(Relaxed);
    for _ in 0..5 {
        let mut u = connect(&s);
        let _ = nd(&mut u, json!({"id": "u", "action": "PING"}));
    }
    assert_eq!(s.metrics().rejected_rate_limit.load(Relaxed), before);
}

#[test]
fn connection_limit_drops_the_excess() {
    let mut c0 = cfg();
    c0.max_connections = 3;
    let s = start(c0, echo_backend());
    let mut held: Vec<TcpStream> = (0..3).map(|_| connect(&s)).collect();
    for c in held.iter_mut() {
        assert_eq!(nd(c, req("PING", json!({})))["success"], true);
    }
    let mut extra = connect(&s);
    let _ = extra.write_all(b"{\"id\":\"e\",\"action\":\"PING\"}\n");
    assert!(is_closed(&mut extra), "a 4ª conexão deve ser recusada");
    assert!(s.metrics().connections_rejected_limit.load(Relaxed) >= 1);
    // libera uma e a próxima passa
    drop(held.pop());
    std::thread::sleep(Duration::from_millis(500));
    let mut again = connect(&s);
    assert_eq!(nd(&mut again, req("PING", json!({})))["success"], true);
}

// ------------------------------------------------------------------------------------------ HTTP

#[test]
fn http_health_and_rpc_with_bearer_or_body_token() {
    let s = start(cfg(), echo_backend());
    let raw = format!("GET /health HTTP/1.1\r\nHost: localhost:{}\r\n\r\n", s.port());
    let (st, head, body) = http_raw(&s, raw.as_bytes());
    assert_eq!(st, 200);
    assert!(head.contains("connection: close") && head.contains("content-type: application/json"));
    assert!(!head.contains("access-control"), "sem CORS");
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["ok"], true);

    let b = json!({"id": "h", "action": "ECHO", "params": {"k": 1}}).to_string();
    let (st, _, body) = post(&s, &format!("Authorization: Bearer {TOKEN}\r\n"), &b);
    assert_eq!(st, 200);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!((v["success"].as_bool(), v["data"]["k"].as_i64(), v["id"].as_str()), (Some(true), Some(1), Some("h")));

    let b = json!({"id": "h", "action": "ECHO", "params": {"k": 2}, "token": TOKEN}).to_string();
    let (st, _, body) = post(&s, "", &b);
    assert_eq!(st, 200);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["data"]["k"], 2);

    // erro de ação = 200 com success=false (o transporte funcionou)
    let b = json!({"id": "h", "action": "FAIL"}).to_string();
    let (st, _, body) = post(&s, &format!("Authorization: Bearer {TOKEN}\r\n"), &b);
    assert_eq!(st, 200);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["success"], false);
    // host com [::1] também é nome de loopback permitido
    let raw = format!("GET /health HTTP/1.1\r\nHost: [::1]:{}\r\n\r\n", s.port());
    assert_eq!(http_raw(&s, raw.as_bytes()).0, 200);
}

#[test]
fn http_auth_failures_are_401_with_www_authenticate() {
    let s = start(cfg(), echo_backend());
    let b = json!({"id": "h", "action": "ECHO"}).to_string();
    for hdr in ["", "Authorization: Bearer errado\r\n", "Authorization: Basic abc\r\n", "Authorization: Bearer\r\n"] {
        let (st, head, body) = post(&s, hdr, &b);
        assert_eq!(st, 401, "{hdr:?}");
        assert!(head.contains("www-authenticate: bearer"));
        assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["code"], code::UNAUTHORIZED);
    }
    // token na query string NUNCA é aceito
    let raw = format!("POST /rpc?token={TOKEN} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{b}", s.port(), b.len());
    assert_eq!(http_raw(&s, raw.as_bytes()).0, 404);
}

#[test]
fn http_rejects_foreign_host_and_any_origin_before_auth() {
    let s = start(cfg(), echo_backend());
    let b = json!({"id": "h", "action": "ECHO"}).to_string();
    let auth = format!("Authorization: Bearer {TOKEN}\r\n");
    // DNS rebinding: Host do atacante (mesmo com token válido)
    for host in ["evil.example:1", "evil.example", "127.0.0.1", "127.0.0.1.evil.example:1", "0.0.0.0:1"] {
        let raw = format!("POST /rpc HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{auth}\r\n{b}", b.len());
        assert_eq!(http_raw(&s, raw.as_bytes()).0, 403, "Host {host}");
    }
    // porta errada no Host
    let raw = format!("POST /rpc HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{auth}\r\n{b}", s.port().wrapping_add(1), b.len());
    assert_eq!(http_raw(&s, raw.as_bytes()).0, 403);
    // navegador: qualquer Origin, inclusive "null" e o próprio loopback
    for origin in ["http://evil.example", "null", "http://127.0.0.1", "http://localhost:3000"] {
        let (st, _, _) = post(&s, &format!("{auth}Origin: {origin}\r\n"), &b);
        assert_eq!(st, 403, "Origin {origin}");
    }
    // sem Host
    let raw = format!("POST /rpc HTTP/1.0\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{auth}\r\n{b}", b.len());
    assert_eq!(http_raw(&s, raw.as_bytes()).0, 400);
    assert!(s.metrics().rejected_host_origin.load(Relaxed) >= 9);
}

#[test]
fn http_validates_method_path_content_type_and_length() {
    let s = start(cfg(), echo_backend());
    let auth = format!("Authorization: Bearer {TOKEN}\r\n");
    let b = json!({"id": "h", "action": "ECHO"}).to_string();
    // content-type "simples" (CSRF por formulário) é recusado
    let raw = format!("POST /rpc HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n{auth}\r\n{b}", s.port(), b.len());
    assert_eq!(http_raw(&s, raw.as_bytes()).0, 415);
    // sem Content-Length
    let raw = format!("POST /rpc HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\n{auth}\r\n", s.port());
    assert_eq!(http_raw(&s, raw.as_bytes()).0, 411);
    // Transfer-Encoding não suportado
    let raw = format!("POST /rpc HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nContent-Length: 2\r\n{auth}\r\n{{}}", s.port());
    assert_eq!(http_raw(&s, raw.as_bytes()).0, 400);
    // Content-Length duplicado / inválido
    let raw = format!("POST /rpc HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nContent-Length: 2\r\n{auth}\r\n{{}}", s.port());
    assert_eq!(http_raw(&s, raw.as_bytes()).0, 400);
    let raw = format!("POST /rpc HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: -1\r\n{auth}\r\n", s.port());
    assert_eq!(http_raw(&s, raw.as_bytes()).0, 400);
    // métodos e caminhos
    let h = format!("Host: 127.0.0.1:{}\r\n", s.port());
    for (line, want) in [
        ("OPTIONS /rpc HTTP/1.1", 405),
        ("GET /rpc HTTP/1.1", 405),
        ("POST /health HTTP/1.1", 405),
        ("DELETE /rpc HTTP/1.1", 405),
        ("GET /outra HTTP/1.1", 404),
        ("GET / HTTP/1.1", 404),
        ("GET /health?x=1 HTTP/1.1", 404),
        ("GET /health HTTP/9.9", 505),
        ("GET /health", 400),
    ] {
        let raw = format!("{line}\r\n{h}\r\n");
        let (st, head, _) = http_raw(&s, raw.as_bytes());
        assert_eq!(st, want, "{line}");
        assert!(!head.contains("access-control"));
    }
    // JSON inválido
    let (st, _, _) = post(&s, &auth, "{ nao e json");
    assert_eq!(st, 400);
}

#[test]
fn http_big_body_is_413_and_the_client_can_still_read_the_answer() {
    let mut c0 = cfg();
    c0.max_request_bytes = 8192;
    let s = start(c0, echo_backend());
    let body = format!("{{\"id\":\"b\",\"action\":\"ECHO\",\"params\":{{\"x\":\"{}\"}}}}", "a".repeat(3_000_000));
    let (st, _, _) = post(&s, &format!("Authorization: Bearer {TOKEN}\r\n"), &body);
    assert_eq!(st, 413);
    assert!(s.metrics().rejected_too_large.load(Relaxed) >= 1);
}

#[test]
fn http_slow_headers_and_slow_body_time_out() {
    let s = start(cfg(), echo_backend());
    // cabeçalhos que nunca terminam
    let mut c = connect(&s);
    c.write_all(format!("POST /rpc HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nX-Slow: a", s.port()).as_bytes()).unwrap();
    let t = Instant::now();
    let mut buf = Vec::new();
    let _ = c.read_to_end(&mut buf);
    assert!(t.elapsed() < Duration::from_secs(3));
    assert!(String::from_utf8_lossy(&buf).starts_with("HTTP/1.1 408"));
    // corpo prometido que não chega
    let mut c = connect(&s);
    let head = format!("POST /rpc HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: 500\r\nAuthorization: Bearer {TOKEN}\r\n\r\n{{\"id\"", s.port());
    c.write_all(head.as_bytes()).unwrap();
    let mut buf = Vec::new();
    let _ = c.read_to_end(&mut buf);
    assert!(String::from_utf8_lossy(&buf).starts_with("HTTP/1.1 408"), "{}", String::from_utf8_lossy(&buf));
}

#[test]
fn http_oversized_headers_are_431() {
    let s = start(cfg(), echo_backend());
    let mut raw = format!("GET /health HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n", s.port());
    for i in 0..400 {
        raw.push_str(&format!("X-Pad-{i}: {}\r\n", "p".repeat(100)));
    }
    raw.push_str("\r\n");
    assert_eq!(http_raw(&s, raw.as_bytes()).0, 431);
}

// ------------------------------------------------------------------------------------------ descoberta / ciclo de vida

#[test]
fn discovery_file_is_written_with_the_session_token_and_removed_on_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("anigo").join("bridge.json");
    let mut c0 = cfg();
    c0.token = None;
    c0.discovery = Some(path.clone());
    let s = start(c0, echo_backend());
    let d = anigo_bridge::discovery::read(&path).unwrap();
    assert_eq!(d.port, s.port());
    assert_eq!(d.token, s.token());
    assert_eq!(d.token.len(), 64);
    assert_eq!(d.pid, std::process::id());
    // o token do arquivo funciona
    let mut c = connect(&s);
    let r = nd(&mut c, json!({"id": 1, "action": "PING", "token": d.token}));
    assert_eq!(r["success"], true);
    s.shutdown();
    assert!(!path.exists(), "arquivo de descoberta removido");
}

#[test]
fn each_session_gets_a_fresh_token_and_an_ephemeral_port() {
    let a = start(ServerConfig { token: None, ..cfg() }, echo_backend());
    let b = start(ServerConfig { token: None, ..cfg() }, echo_backend());
    assert_ne!(a.token(), b.token());
    assert_ne!(a.port(), b.port());
    assert_ne!(a.port(), 0);
}

#[test]
fn shutdown_closes_open_connections_quickly() {
    let s = start(cfg(), echo_backend());
    let mut c = connect(&s);
    assert_eq!(nd(&mut c, req("PING", json!({})))["success"], true);
    let t = Instant::now();
    s.shutdown();
    assert!(is_closed(&mut c));
    assert!(t.elapsed() < Duration::from_secs(3));
    assert!(TcpStream::connect(("127.0.0.1", c.peer_addr().unwrap().port())).is_err());
}

// ------------------------------------------------------------------------------------------ concorrência com a "UI"

/// Thread "UI" de teste: dona de uma `CoreSession` real; drena pedidos como o studio e também aplica comandos
/// "do usuário" por conta própria, intercalados.
struct TestHost {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<CoreSession>>,
}

impl TestHost {
    fn spawn(queue: anigo_bridge::HostQueue, user_actions: usize) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            let mut core = CoreSession::new();
            core.set_history_limit(1000); // o editor usa profundidade maior que a do projeto (60)
            let mut user_done = 0usize;
            while !s2.load(Relaxed) {
                for r in queue.drain(16) {
                    let res = core_ops::handle(&mut core, &r.action, &r.params).unwrap_or_else(|| Err(BridgeError::unknown_action(&r.action)));
                    r.reply.send(res);
                }
                if user_done < user_actions {
                    let _ = core.apply(anigo_core::command::Command::RenameProject { name: format!("usuario-{user_done}") });
                    user_done += 1;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            core
        });
        Self { stop, handle: Some(handle) }
    }
    fn finish(mut self) -> CoreSession {
        self.stop.store(true, Relaxed);
        self.handle.take().unwrap().join().unwrap()
    }
}

#[test]
fn many_clients_and_the_ui_share_one_history_without_lost_updates() {
    let (backend, queue) = channel(256, Arc::new(|| {}));
    let host = TestHost::spawn(queue, 40);
    let s = Arc::new(start(ServerConfig { max_connections: 64, ..cfg() }, Arc::new(backend)));

    let ok = Arc::new(AtomicUsize::new(0));
    let errs = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut threads = Vec::new();
    for t in 0..24 {
        let (s, ok, errs) = (Arc::clone(&s), Arc::clone(&ok), Arc::clone(&errs));
        threads.push(std::thread::spawn(move || {
            let mut c = connect(&s);
            for i in 0..15 {
                let r = nd(&mut c, req("APPLY_COMMAND", json!({"command": {"kind": "rename_project", "name": format!("c{t}-{i}")}})));
                if r["success"] == true {
                    ok.fetch_add(1, Relaxed);
                } else {
                    errs.lock().unwrap().push(r.to_string());
                }
                // leituras intercaladas
                let r = nd(&mut c, req("GET_PROJECT", json!({"include_document": false})));
                assert_eq!(r["success"], true, "{r}");
            }
        }));
    }
    for t in threads {
        t.join().unwrap();
    }
    std::thread::sleep(Duration::from_millis(150)); // deixa a UI terminar as ações próprias
    let applied = ok.load(Relaxed);
    assert_eq!(applied, 24 * 15, "erros: {:?}", errs.lock().unwrap());
    let core = host.finish();
    // nenhum comando perdido nem duplicado: bridge + usuário = log da sessão
    assert_eq!(core.history().export_log().entries.len(), applied + 40);
    // e dá para desfazer tudo, em ordem, até o início
    let mut core = core;
    let mut undone = 0;
    while core.undo().is_ok() {
        undone += 1;
    }
    assert_eq!(undone, applied + 40);
    assert_eq!(core.project().name, anigo_core::project::ProjectState::default().name);
}

#[test]
fn stale_ui_never_applies_a_cancelled_mutation() {
    // a UI só acorda depois do prazo: o cliente recebe o timeout e a mutação NÃO pode ser aplicada depois
    let (backend, queue) = channel(8, Arc::new(|| {}));
    let backend = backend.with_timeout(|_| Duration::from_millis(150));
    let s = start(cfg(), Arc::new(backend));
    let mut c = connect(&s);
    let r = nd(&mut c, req("APPLY_COMMAND", json!({"command": {"kind": "rename_project", "name": "tarde demais"}})));
    assert_eq!(r["code"], code::UI_TIMEOUT);
    std::thread::sleep(Duration::from_millis(100));
    let mut core = CoreSession::new();
    for h in queue.drain(8) {
        let res = core_ops::handle(&mut core, &h.action, &h.params).unwrap();
        h.reply.send(res);
    }
    assert_ne!(core.project().name, "tarde demais");
    assert!(!core.history().can_undo());
}
