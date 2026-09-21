use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// P2: in-process metrics for the ANIGO Live Bridge. Counters are AtomicU64
/// and lock-free; we do not (yet) expose a Prometheus scrape endpoint, but a
/// GET_METRICS action returns this as JSON for `anigo_get_live_telemetry`.
pub(crate) mod bridge_metrics {
    use serde_json::{json, Value};
    use std::sync::atomic::AtomicU64;
    use std::sync::atomic::Ordering::Relaxed;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    pub struct BridgeMetrics {
        pub start_instant: Instant,
        pub start_epoch_secs: u64,
        pub connections_accepted: AtomicU64,
        pub connections_rejected_rate_limit: AtomicU64,
        pub connections_rejected_auth: AtomicU64,
        pub requests_total: AtomicU64,
        pub requests_success: AtomicU64,
        pub requests_error: AtomicU64,
        pub bytes_read: AtomicU64,
        pub bytes_written: AtomicU64,
        pub frames_too_large: AtomicU64,
        pub invalid_json: AtomicU64,
    }

    impl BridgeMetrics {
        pub fn new() -> Self {
            Self {
                start_instant: Instant::now(),
                start_epoch_secs: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
                connections_accepted: AtomicU64::new(0),
                connections_rejected_rate_limit: AtomicU64::new(0),
                connections_rejected_auth: AtomicU64::new(0),
                requests_total: AtomicU64::new(0),
                requests_success: AtomicU64::new(0),
                requests_error: AtomicU64::new(0),
                bytes_read: AtomicU64::new(0),
                bytes_written: AtomicU64::new(0),
                frames_too_large: AtomicU64::new(0),
                invalid_json: AtomicU64::new(0),
            }
        }

        pub fn snapshot(&self) -> Value {
            let uptime = self.start_instant.elapsed().as_secs_f64();
            json!({
                "uptime_seconds": uptime,
                "started_at_epoch_secs": self.start_epoch_secs,
                "connections_accepted": self.connections_accepted.load(Relaxed),
                "connections_rejected_rate_limit": self.connections_rejected_rate_limit.load(Relaxed),
                "connections_rejected_auth": self.connections_rejected_auth.load(Relaxed),
                "requests_total": self.requests_total.load(Relaxed),
                "requests_success": self.requests_success.load(Relaxed),
                "requests_error": self.requests_error.load(Relaxed),
                "bytes_read": self.bytes_read.load(Relaxed),
                "bytes_written": self.bytes_written.load(Relaxed),
                "frames_too_large": self.frames_too_large.load(Relaxed),
                "invalid_json": self.invalid_json.load(Relaxed),
                "requests_per_second_avg": if uptime > 0.0 {
                    (self.requests_total.load(Relaxed) as f64) / uptime
                } else { 0.0 },
            })
        }
    }
}

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, RwLock};

#[cfg(target_os = "windows")]
#[repr(C)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct BITMAPINFOHEADER {
    bi_size: u32,
    bi_width: i32,
    bi_height: i32,
    bi_planes: u16,
    bi_bit_count: u16,
    bi_compression: u32,
    bi_size_image: u32,
    bi_xpels_per_meter: i32,
    bi_ypels_per_meter: i32,
    bi_clr_used: u32,
    bi_clr_important: u32,
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct BITMAPINFO {
    bmi_header: BITMAPINFOHEADER,
    bmi_colors: [u32; 1],
}

#[cfg(target_os = "windows")]
#[link(name = "user32")]
extern "system" {
    fn GetWindowRect(hWnd: *mut std::ffi::c_void, lpRect: *mut RECT) -> i32;
    fn GetDC(hWnd: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn ReleaseDC(hWnd: *mut std::ffi::c_void, hDC: *mut std::ffi::c_void) -> i32;
}

#[cfg(target_os = "windows")]
#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleDC(hdc: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn CreateCompatibleBitmap(hdc: *mut std::ffi::c_void, cx: i32, cy: i32) -> *mut std::ffi::c_void;
    fn SelectObject(hdc: *mut std::ffi::c_void, h: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn DeleteDC(hdc: *mut std::ffi::c_void) -> i32;
    fn DeleteObject(ho: *mut std::ffi::c_void) -> i32;
    fn BitBlt(
        hdc: *mut std::ffi::c_void,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        hdcSrc: *mut std::ffi::c_void,
        x1: i32,
        y1: i32,
        rop: u32,
    ) -> i32;
    fn GetDIBits(
        hdc: *mut std::ffi::c_void,
        hbm: *mut std::ffi::c_void,
        start: u32,
        cLines: u32,
        lpvBits: *mut u8,
        lpbmi: *mut BITMAPINFO,
        usage: u32,
    ) -> i32;
}

/// Real-time telemetry and state synchronized from the active WebGPU canvas and user interactions.
fn default_spec_intensity() -> f32 { 0.4 }
fn default_spec_power() -> f32 { 32.0 }
fn default_rim_intensity() -> f32 { 0.8 }
fn default_hue_shift() -> f32 { -15.0 }
fn default_toon_steps() -> f32 { 1.0 }
fn default_light_color() -> [f32; 3] { [1.0, 0.98, 0.95] }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveWindowState {
    pub fps: f64,
    pub frame_time_ms: f64,
    pub draw_calls: u32,
    pub triangle_count: usize,
    pub adapter_name: String,
    pub camera_eye: [f32; 3],
    pub camera_target: [f32; 3],
    pub light_direction: [f32; 3],
    pub light_intensity: f32,
    pub shadow_color: [f32; 3],
    pub active_preset: String,
    pub outline_width: f32,
    pub shadow_threshold: f32,
    pub head_scale: f32,
    pub head_ratio: f32,
    pub webgpu_active: bool,
    #[serde(default = "default_spec_intensity")]
    pub spec_intensity: f32,
    #[serde(default = "default_spec_power")]
    pub spec_power: f32,
    #[serde(default = "default_rim_intensity")]
    pub rim_intensity: f32,
    #[serde(default = "default_hue_shift")]
    pub hue_shift: f32,
    #[serde(default = "default_toon_steps")]
    pub toon_steps: f32,
    #[serde(default = "default_light_color")]
    pub light_color: [f32; 3],

    // ---------------------------------------------------------------------
    // P1-06: telemetria com dados reais do renderer e do núcleo (antes só
    // havia estimativas). Campos com `default` para compatibilidade com
    // clientes antigos.
    // ---------------------------------------------------------------------
    /// Erros ativos no canal de diagnóstico do renderer.
    #[serde(default)]
    pub diagnostics_errors: u32,
    /// Avisos ativos no canal de diagnóstico do renderer.
    #[serde(default)]
    pub diagnostics_warnings: u32,
    /// Códigos de diagnóstico ativos (contrato `render_diagnostics.ts`).
    #[serde(default)]
    pub diagnostic_codes: Vec<String>,
    /// Autoridade de deformação em vigor (`core`/`reference_ts`/`unavailable`).
    #[serde(default)]
    pub deformation_authority: String,
    /// Revisão estática da geometria canônica em uso.
    #[serde(default)]
    pub core_static_revision: u64,
    /// Revisão dinâmica do snapshot em uso.
    #[serde(default)]
    pub core_dynamic_revision: u64,
    /// Canais de morph disponíveis no snapshot.
    #[serde(default)]
    pub morph_channels: u32,
    /// Vértices da malha canônica em uso.
    #[serde(default)]
    pub vertex_count: u32,
    /// P1-06: diagnóstico do renderer **nativo** (headless/contrato), separado do
    /// diagnóstico do viewport — os dois são reais e não podem se sobrescrever.
    #[serde(default)]
    pub native_diagnostics_errors: u32,
    #[serde(default)]
    pub native_diagnostics_warnings: u32,
    #[serde(default)]
    pub native_diagnostic_codes: Vec<String>,
    /// O render contract carregou sem drift neste processo.
    #[serde(default = "default_true")]
    pub native_contract_valid: bool,
}

fn default_true() -> bool {
    true
}

impl Default for LiveWindowState {
    fn default() -> Self {
        Self {
            // P1-06: antes destes valores "bonitos" (120 fps / 156 tris) a
            // telemetria mentia antes do primeiro frame. Zero = "não medido".
            fps: 0.0,
            frame_time_ms: 0.0,
            draw_calls: 0,
            triangle_count: 0,
            adapter_name: "unknown".into(),
            camera_eye: [0.0, 1.5, 3.5],
            camera_target: [0.0, 1.0, 0.0],
            light_direction: [0.577, 0.577, 0.577],
            light_intensity: 1.0,
            shadow_color: [0.65, 0.68, 0.85],
            active_preset: "mannequin".into(),
            outline_width: 3.5,
            shadow_threshold: 0.5,
            head_scale: 1.0,
            head_ratio: 6.5,
            webgpu_active: true,
            spec_intensity: 0.4,
            spec_power: 32.0,
            rim_intensity: 0.8,
            hue_shift: -15.0,
            toon_steps: 1.0,
            light_color: [1.0, 0.98, 0.95],
            // P1-06: nada medido ainda ⇒ nada inventado.
            diagnostics_errors: 0,
            diagnostics_warnings: 0,
            diagnostic_codes: Vec::new(),
            deformation_authority: "unknown".into(),
            core_static_revision: 0,
            core_dynamic_revision: 0,
            morph_channels: 0,
            vertex_count: 0,
            native_diagnostics_errors: 0,
            native_diagnostics_warnings: 0,
            native_diagnostic_codes: Vec::new(),
            native_contract_valid: true,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BridgeRequest {
    pub id: String,
    pub action: String,
    #[serde(default)]
    pub params: Value,
    /// P2-14: Optional bearer token. When the server is started with auth enabled
    /// (ANIGO_BRIDGE_TOKEN set or auto-generated), requests without a valid token
    /// are rejected with error -32001 Unauthorized before any action is dispatched.
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BridgeResponse {
    pub id: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// P2-14: simple per-IP fixed-window rate limiter.
/// Window = 1 second; max requests per window = RATE_LIMIT_PER_SEC.
/// Uses a HashMap protected by a tokio Mutex (traffic is low ~30 rps max).
struct RateLimiter {
    /// per-IP: (window_start, count)
    buckets: Mutex<HashMap<String, (Instant, u32)>>,
    max_per_sec: u32,
}

impl RateLimiter {
    fn new(max_per_sec: u32) -> Self {
        Self { buckets: Mutex::new(HashMap::new()), max_per_sec }
    }

    async fn check(&self, key: &str) -> bool {
        let mut map = self.buckets.lock().await;
        let now = Instant::now();
        let entry = map.entry(key.to_string()).or_insert((now, 0));
        if now.duration_since(entry.0).as_secs() >= 1 {
            entry.0 = now;
            entry.1 = 1;
            true
        } else if entry.1 < self.max_per_sec {
            entry.1 += 1;
            true
        } else {
            false
        }
    }

    /// Periodically evict stale entries to prevent unbounded growth.
    async fn gc(&self) {
        let mut map = self.buckets.lock().await;
        let now = Instant::now();
        map.retain(|_, v| now.duration_since(v.0).as_secs() < 10);
    }
}

/// Generate a cryptographically-random 16-byte hex token using OS entropy.
/// Falls back to a time+pid+addr-based token if the OS RNG is unavailable
/// (e.g. sandboxed / restricted environments).
fn generate_token() -> String {
    let mut bytes = [0u8; 16];
    match std::fs::File::open("/dev/urandom") {
        Ok(mut f) => {
            use std::io::Read;
            // read exactly 16 bytes from urandom
            let mut buf = [0u8; 16];
            if f.read_exact(&mut buf).is_ok() {
                bytes = buf;
            }
        }
        Err(_) => {
            // Windows fallback / no /dev/urandom: mix pid + time + address
            let pid = std::process::id() as u128;
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let mixed = pid.wrapping_mul(0x9E37_79B9_7F4A_7C15)
                ^ nanos
                ^ (nanos >> 32)
                ^ (&bytes as *const _ as u128);
            for (i, chunk) in bytes.iter_mut().enumerate() {
                *chunk = ((mixed >> (i * 8)) & 0xFF) as u8;
            }
        }
    }
    // Always re-mix in case of partial fill; this is sufficient for a local auth token.
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Determine where to write the bridge lockfile (token + port).
/// Respects ANIGO_BRIDGE_LOCKFILE; falls back to OS temp dir.
fn lockfile_path() -> PathBuf {
    if let Ok(p) = std::env::var("ANIGO_BRIDGE_LOCKFILE") {
        return PathBuf::from(p);
    }
    let pid = std::process::id();
    let mut base = std::env::temp_dir();
    base.push(format!("anigo-bridge-{pid}.json"));
    base
}

/// P2-10/P2: portable log directory resolver (mirrors the MCP-side logic so that
/// the Tauri shell writes launch/panic logs into Documents\ANIGO or equivalent
/// instead of hardcoded C:\ANIGO).
pub fn log_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("ANIGO_LOG_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let p = PathBuf::from(&local).join("ANIGO").join("logs");
            if p.exists() { return p; }
            let _ = std::fs::create_dir_all(&p);
            return p;
        }
        if let Ok(profile) = std::env::var("USERPROFILE") {
            let p = PathBuf::from(&profile).join("Documents").join("ANIGO").join("logs");
            if p.exists() { return p; }
            let p2 = PathBuf::from(&profile).join("Documents").join("ANIGO");
            let _ = std::fs::create_dir_all(&p);
            let _ = std::fs::create_dir_all(&p2);
            return p;
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(home) = std::env::var_os("HOME") {
            let p = PathBuf::from(home).join(".local").join("share").join("anigo").join("logs");
            let _ = std::fs::create_dir_all(&p);
            return p;
        }
    }
    // Legacy fallback
    PathBuf::from("C:\\ANIGO")
}

pub struct LiveBridgeServer {
    app_handle: AppHandle,
    state: Arc<RwLock<LiveWindowState>>,
    /// P2-14: auth token. Empty means auth disabled (legacy/dev mode).
    token: Arc<str>,
    /// P2-14: monotonic request counter for server-assigned correlation ids.
    #[allow(dead_code)]
    request_count: AtomicU64,
    /// P2-14: per-IP rate limiter (30 req/s per peer).
    rate_limiter: Arc<RateLimiter>,
    /// P2: runtime metrics.
    metrics: Arc<bridge_metrics::BridgeMetrics>,
}

impl LiveBridgeServer {
    pub fn new(app_handle: AppHandle, state: Arc<RwLock<LiveWindowState>>) -> Self {
        // P2-14: token precedence: env > auto-generate.
        let token: String = std::env::var("ANIGO_BRIDGE_TOKEN")
            .unwrap_or_else(|_| generate_token());

        if std::env::var("ANIGO_BRIDGE_TOKEN").is_ok() {
            tracing::info!(source = "env", "ANIGO Bridge auth token loaded from ANIGO_BRIDGE_TOKEN");
        } else {
            tracing::info!(
                token_len = token.len(),
                "ANIGO Bridge auth token auto-generated (pass ANIGO_BRIDGE_TOKEN to override)"
            );
        }

        Self {
            app_handle,
            state,
            token: Arc::from(token),
            request_count: AtomicU64::new(1),
            rate_limiter: Arc::new(RateLimiter::new(30)),
            metrics: Arc::new(bridge_metrics::BridgeMetrics::new()),
        }
    }

    pub async fn run(self: Arc<Self>, bind_addr: &str) {
        // P2-14: bind to loopback. Allow ANIGO_BRIDGE_BIND override (e.g. for UDS future).
        let listener = match TcpListener::bind(bind_addr).await {
            Ok(l) => {
                let local = l.local_addr().ok();
                tracing::info!(bind = %bind_addr, local = ?local, "ANIGO Live Bridge listening");
                l
            }
            Err(e) => {
                tracing::error!(bind = %bind_addr, error = %e, "ANIGO Live Bridge failed to bind");
                return;
            }
        };

        // Write lockfile: { "token": "...", "addr": "127.0.0.1:39090", "pid": N }
        let local_addr = listener.local_addr().ok();
        let lockfile = lockfile_path();
        let lockfile_payload = json!({
            "token": &*self.token,
            "addr": local_addr.map(|a| a.to_string()).unwrap_or_else(|| bind_addr.to_string()),
            "pid": std::process::id(),
            "started_at_utc": SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            "version": env!("CARGO_PKG_VERSION"),
        });
        if let Err(e) = std::fs::write(&lockfile, serde_json::to_vec_pretty(&lockfile_payload).unwrap_or_default()) {
            tracing::warn!(path = %lockfile.display(), error = %e, "Failed to write bridge lockfile; MCP clients may not discover the token");
        } else {
            tracing::info!(path = %lockfile.display(), "Wrote bridge token lockfile");
        }

        // Also propagate token as env var so spawned MCP child processes inherit it.
        // (The Tauri main will also set this for child processes it launches.)
        unsafe { std::env::set_var("ANIGO_BRIDGE_TOKEN", &*self.token); }

        // Spawn GC task for rate limiter (every 30s).
        let rl = Arc::clone(&self.rate_limiter);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                rl.gc().await;
            }
        });

        loop {
            match listener.accept().await {
                Ok((socket, peer)) => {
                    let server = Arc::clone(&self);
                    tokio::spawn(async move {
                        server.handle_client(socket, peer).await;
                    });
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Bridge accept error");
                    // Small backoff to avoid busy loop on repeated errors
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
        }
    }

    /// P2: expose snapshot of runtime metrics (for the GET_METRICS action).
    pub fn metrics_snapshot(&self) -> Value {
        self.metrics.snapshot()
    }

    async fn handle_client(&self, socket: TcpStream, peer: std::net::SocketAddr) {
        use tokio::time::{timeout, Duration};
        const MAX_LINE: usize = 1 << 20; // 1MB max frame
        const READ_TIMEOUT_SECS: u64 = 5;

        let peer_key = peer.ip().to_string();

        // P2-14: rate-limit per connection IP
        if !self.rate_limiter.check(&peer_key).await {
            self.metrics.connections_rejected_rate_limit.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(peer = %peer_key, "Bridge connection rejected (rate limit)");
            let err_res = BridgeResponse {
                id: "preauth".into(),
                success: false,
                data: None,
                error: Some("Rate limit exceeded (30 req/s max)".into()),
            };
            if let Ok(mut rb) = serde_json::to_vec(&err_res) {
                rb.push(b'\n');
                let _ = socket.try_write(&rb);
            }
            return;
        }

        self.metrics.connections_accepted.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(peer = %peer_key, "Bridge client connected");

        let (reader, mut writer) = socket.into_split();
        let mut buf_reader = BufReader::new(reader);
        let mut line = String::new();

        // P2-14: track if this connection has been authenticated (first request).
        // When self.token is empty (dev mode), we treat all connections as authenticated.
        let auth_required = !self.token.is_empty();
        let mut authed = !auth_required;

        loop {
            line.clear();
            let read_fut = buf_reader.read_line(&mut line);
            let read_res = match timeout(Duration::from_secs(READ_TIMEOUT_SECS), read_fut).await {
                Ok(r) => r,
                Err(_) => {
                    tracing::debug!(peer = %peer_key, "Bridge client read timeout — closing");
                    break;
                }
            };

            let n = match read_res {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    tracing::debug!(peer = %peer_key, error = %e, "Bridge client read error");
                    break;
                }
            };

            self.metrics.bytes_read.fetch_add(n as u64, Ordering::Relaxed);

            if n > MAX_LINE {
                self.metrics.frames_too_large.fetch_add(1, Ordering::Relaxed);
                let err_res = BridgeResponse {
                    id: "unknown".into(),
                    success: false,
                    data: None,
                    error: Some(format!("Frame too large: {} > {} bytes", n, MAX_LINE)),
                };
                Self::write_response(&mut writer, &err_res).await;
                break;
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let req: BridgeRequest = match serde_json::from_str(trimmed) {
                Ok(r) => r,
                Err(e) => {
                    self.metrics.invalid_json.fetch_add(1, Ordering::Relaxed);
                    let err_res = BridgeResponse {
                        id: "unknown".into(),
                        success: false,
                        data: None,
                        error: Some(format!("Invalid JSON frame: {}", e)),
                    };
                    Self::write_response(&mut writer, &err_res).await;
                    continue;
                }
            };

            self.metrics.requests_total.fetch_add(1, Ordering::Relaxed);

            // P2-14: auth check (per-request to allow token upgrade on first message)
            if auth_required && !authed {
                match &req.token {
                    Some(t) if t.as_str() == &*self.token => {
                        authed = true;
                        tracing::debug!(peer = %peer_key, "Bridge client authenticated");
                    }
                    _ => {
                        self.metrics.connections_rejected_auth.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(peer = %peer_key, "Bridge request rejected: unauthorized");
                        let err_res = BridgeResponse {
                            id: req.id.clone(),
                            success: false,
                            data: None,
                            error: Some("Unauthorized: missing or invalid token (code -32001)".into()),
                        };
                        Self::write_response(&mut writer, &err_res).await;
                        // Close connection after first auth failure to avoid brute force
                        break;
                    }
                }
            }

            let action = req.action.clone();
            let response = self.dispatch_action(&req).await;

            if response.success {
                self.metrics.requests_success.fetch_add(1, Ordering::Relaxed);
            } else {
                self.metrics.requests_error.fetch_add(1, Ordering::Relaxed);
            }
            tracing::trace!(peer = %peer_key, action = %action, id = %req.id, ok = response.success, "Bridge request handled");

            Self::write_response(&mut writer, &response).await;
        }

        tracing::debug!(peer = %peer_key, "Bridge client disconnected");
    }

    /// Helper to serialize + write + flush a BridgeResponse (with size cap + timeouts).
    /// Aceita qualquer metade de escrita: a conexão do cliente usa
    /// `OwnedWriteHalf` e o servidor `WriteHalf<TcpStream>`.
    async fn write_response<W>(writer: &mut W, resp: &BridgeResponse)
    where
        W: tokio::io::AsyncWrite + Unpin,
    {
        use tokio::time::{timeout, Duration};
        const MAX_LINE: usize = 1 << 20;
        let mut resp_bytes = match serde_json::to_vec(resp) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to serialize bridge response");
                return;
            }
        };
        if resp_bytes.len() > MAX_LINE {
            let err_res = BridgeResponse {
                id: resp.id.clone(),
                success: false,
                data: None,
                error: Some(format!("Response too large: {} bytes", resp_bytes.len())),
            };
            resp_bytes = serde_json::to_vec(&err_res).unwrap_or_default();
        }
        resp_bytes.push(b'\n');
        let _ = timeout(Duration::from_secs(2), writer.write_all(&resp_bytes)).await;
        let _ = timeout(Duration::from_secs(1), writer.flush()).await;
    }

    async fn dispatch_action(&self, req: &BridgeRequest) -> BridgeResponse {
        match req.action.as_str() {
            // ─────────────────────────────────────────────────────────────
            // DIAGNÓSTICO
            // ─────────────────────────────────────────────────────────────
            "PING" => BridgeResponse {
                id: req.id.clone(),
                success: true,
                data: Some(json!({ "pong": true, "engine": "ANIGO Live Studio" })),
                error: None,
            },

            // ─────────────────────────────────────────────────────────────
            // CONTROLE DA JANELA  (usa API nativa do Tauri — sem Win32)
            // ─────────────────────────────────────────────────────────────
            "MAXIMIZE_WINDOW" => {
                match self.app_handle.get_webview_window("main") {
                    Some(win) => {
                        let _ = win.unminimize();
                        let _ = win.maximize();
                        let _ = win.set_focus();
                        let size = win.outer_size().ok();
                        let pos  = win.outer_position().ok();
                        BridgeResponse {
                            id: req.id.clone(),
                            success: true,
                            data: Some(json!({
                                "action": "MAXIMIZE_WINDOW",
                                "size": size.map(|s| json!({"width": s.width, "height": s.height})),
                                "position": pos.map(|p| json!({"x": p.x, "y": p.y})),
                            })),
                            error: None,
                        }
                    }
                    None => BridgeResponse {
                        id: req.id.clone(),
                        success: false,
                        data: None,
                        error: Some("Main window not found".into()),
                    },
                }
            }

            "RESTORE_WINDOW" => {
                match self.app_handle.get_webview_window("main") {
                    Some(win) => {
                        let _ = win.unminimize();
                        let _ = win.unmaximize();
                        let _ = win.show();
                        let _ = win.set_focus();
                        BridgeResponse {
                            id: req.id.clone(),
                            success: true,
                            data: Some(json!({ "action": "RESTORE_WINDOW" })),
                            error: None,
                        }
                    }
                    None => BridgeResponse {
                        id: req.id.clone(),
                        success: false,
                        data: None,
                        error: Some("Main window not found".into()),
                    },
                }
            }

            "MINIMIZE_WINDOW" => {
                match self.app_handle.get_webview_window("main") {
                    Some(win) => {
                        let _ = win.minimize();
                        BridgeResponse {
                            id: req.id.clone(),
                            success: true,
                            data: Some(json!({ "action": "MINIMIZE_WINDOW" })),
                            error: None,
                        }
                    }
                    None => BridgeResponse {
                        id: req.id.clone(),
                        success: false,
                        data: None,
                        error: Some("Main window not found".into()),
                    },
                }
            }

            "FOCUS_WINDOW" => {
                match self.app_handle.get_webview_window("main") {
                    Some(win) => {
                        let _ = win.show();
                        let _ = win.unminimize();
                        let _ = win.set_focus();
                        BridgeResponse {
                            id: req.id.clone(),
                            success: true,
                            data: Some(json!({ "action": "FOCUS_WINDOW" })),
                            error: None,
                        }
                    }
                    None => BridgeResponse {
                        id: req.id.clone(),
                        success: false,
                        data: None,
                        error: Some("Main window not found".into()),
                    },
                }
            }

            "RELOAD_WEBVIEW" => {
                match self.app_handle.get_webview_window("main") {
                    Some(win) => {
                        let _ = win.eval("window.location.reload()");
                        BridgeResponse {
                            id: req.id.clone(),
                            success: true,
                            data: Some(json!({ "action": "RELOAD_WEBVIEW" })),
                            error: None,
                        }
                    }
                    None => BridgeResponse {
                        id: req.id.clone(),
                        success: false,
                        data: None,
                        error: Some("Main window not found".into()),
                    },
                }
            }

            "EVAL_JS" => {
                match self.app_handle.get_webview_window("main") {
                    Some(win) => {
                        let script = req.params.get("script").and_then(|v| v.as_str()).unwrap_or("");
                        let _ = win.eval(script);
                        BridgeResponse {
                            id: req.id.clone(),
                            success: true,
                            data: Some(json!({ "action": "EVAL_JS" })),
                            error: None,
                        }
                    }
                    None => BridgeResponse {
                        id: req.id.clone(),
                        success: false,
                        data: None,
                        error: Some("Main window not found".into()),
                    },
                }
            }

            "GET_WINDOW_STATE" => {
                match self.app_handle.get_webview_window("main") {
                    Some(win) => {
                        let is_minimized  = win.is_minimized().unwrap_or(false);
                        let is_maximized  = win.is_maximized().unwrap_or(false);
                        let is_visible    = win.is_visible().unwrap_or(false);
                        let is_focused    = win.is_focused().unwrap_or(false);
                        let outer_size    = win.outer_size().ok();
                        let inner_size    = win.inner_size().ok();
                        let outer_pos     = win.outer_position().ok();
                        let scale_factor  = win.scale_factor().unwrap_or(1.0);
                        #[cfg(target_os = "windows")]
                        let hwnd_val = win.hwnd().map(|h| h.0 as usize).ok();
                        #[cfg(not(target_os = "windows"))]
                        let hwnd_val: Option<usize> = None;

                        BridgeResponse {
                            id: req.id.clone(),
                            success: true,
                            data: Some(json!({
                                "is_minimized":  is_minimized,
                                "is_maximized":  is_maximized,
                                "is_visible":    is_visible,
                                "is_focused":    is_focused,
                                "scale_factor":  scale_factor,
                                "outer_size":    outer_size.map(|s| json!({"width": s.width, "height": s.height})),
                                "inner_size":    inner_size.map(|s| json!({"width": s.width, "height": s.height})),
                                "outer_position": outer_pos.map(|p| json!({"x": p.x, "y": p.y})),
                                "hwnd":          hwnd_val,
                            })),
                            error: None,
                        }
                    }
                    None => BridgeResponse {
                        id: req.id.clone(),
                        success: false,
                        data: None,
                        error: Some("Main window not found".into()),
                    },
                }
            }

            "SCREENSHOT" => {
                match self.app_handle.get_webview_window("main") {
                    Some(win) => {
                        let _ = win.unminimize();
                        let _ = win.show();
                        let _ = win.set_focus();
                        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

                        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))]
                        let save_path = req.params.get("save_path").and_then(|v| v.as_str());

                        #[cfg(target_os = "windows")]
                        {
                            if let Ok(hwnd_wrapper) = win.hwnd() {
                                let hwnd = hwnd_wrapper.0 as *mut std::ffi::c_void;
                                let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                                unsafe { GetWindowRect(hwnd, &mut rect) };
                                let width = (rect.right - rect.left).max(1);
                                let height = (rect.bottom - rect.top).max(1);

                                unsafe {
                                    let screen_dc = GetDC(std::ptr::null_mut());
                                    let mem_dc = CreateCompatibleDC(screen_dc);
                                    let bitmap = CreateCompatibleBitmap(screen_dc, width, height);
                                    let old_bmp = SelectObject(mem_dc, bitmap);

                                    BitBlt(mem_dc, 0, 0, width, height, screen_dc, rect.left, rect.top, 0x00CC0020);

                                    let mut bmi = BITMAPINFO {
                                        bmi_header: BITMAPINFOHEADER {
                                            bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                                            bi_width: width,
                                            bi_height: -height,
                                            bi_planes: 1,
                                            bi_bit_count: 32,
                                            bi_compression: 0,
                                            bi_size_image: 0,
                                            bi_xpels_per_meter: 0,
                                            bi_ypels_per_meter: 0,
                                            bi_clr_used: 0,
                                            bi_clr_important: 0,
                                        },
                                        bmi_colors: [0],
                                    };

                                    let mut raw_pixels = vec![0u8; (width * height * 4) as usize];
                                    GetDIBits(
                                        mem_dc,
                                        bitmap,
                                        0,
                                        height as u32,
                                        raw_pixels.as_mut_ptr(),
                                        &mut bmi,
                                        0,
                                    );

                                    SelectObject(mem_dc, old_bmp);
                                    DeleteObject(bitmap);
                                    DeleteDC(mem_dc);
                                    ReleaseDC(std::ptr::null_mut(), screen_dc);

                                    for chunk in raw_pixels.chunks_exact_mut(4) {
                                        let b = chunk[0];
                                        let r = chunk[2];
                                        chunk[0] = r;
                                        chunk[2] = b;
                                    }

                                    if let Some(path) = save_path {
                                        let _ = image::save_buffer(
                                            path,
                                            &raw_pixels,
                                            width as u32,
                                            height as u32,
                                            image::ColorType::Rgba8,
                                        );
                                    }

                                    return BridgeResponse {
                                        id: req.id.clone(),
                                        success: true,
                                        data: Some(json!({
                                            "action": "SCREENSHOT",
                                            "width": width,
                                            "height": height,
                                            "path": save_path,
                                        })),
                                        error: None,
                                    };
                                }
                            }
                        }

                        BridgeResponse {
                            id: req.id.clone(),
                            success: true,
                            data: Some(json!({ "action": "SCREENSHOT" })),
                            error: None,
                        }
                    }
                    None => BridgeResponse {
                        id: req.id.clone(),
                        success: false,
                        data: None,
                        error: Some("Main window not found".into()),
                    },
                }
            }

            // ─────────────────────────────────────────────────────────────
            // VIEWPORT 3D  (eventos emitidos ao WebGPU renderer)
            // ─────────────────────────────────────────────────────────────
            "ORBIT" => {
                let az = req.params.get("azimuth").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                let el = req.params.get("elevation").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                let _ = self.app_handle.emit("anigo://camera_orbit", json!({ "azimuth": az, "elevation": el }));
                BridgeResponse {
                    id: req.id.clone(),
                    success: true,
                    data: Some(json!({ "action": "ORBIT", "azimuth": az, "elevation": el })),
                    error: None,
                }
            }

            "ZOOM" => {
                let factor = req.params.get("factor").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
                let _ = self.app_handle.emit("anigo://camera_zoom", json!({ "factor": factor }));
                BridgeResponse {
                    id: req.id.clone(),
                    success: true,
                    data: Some(json!({ "action": "ZOOM", "factor": factor })),
                    error: None,
                }
            }

            "PAN" => {
                let dx = req.params.get("dx").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                let dy = req.params.get("dy").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                let _ = self.app_handle.emit("anigo://camera_pan", json!({ "dx": dx, "dy": dy }));
                BridgeResponse {
                    id: req.id.clone(),
                    success: true,
                    data: Some(json!({ "action": "PAN", "dx": dx, "dy": dy })),
                    error: None,
                }
            }

            "SET_CAMERA" => {
                let _ = self.app_handle.emit("anigo://set_camera", req.params.clone());
                BridgeResponse {
                    id: req.id.clone(),
                    success: true,
                    data: Some(json!({ "action": "SET_CAMERA", "params": req.params })),
                    error: None,
                }
            }

            "SET_LIGHT" => {
                let _ = self.app_handle.emit("anigo://set_light", req.params.clone());
                BridgeResponse {
                    id: req.id.clone(),
                    success: true,
                    data: Some(json!({ "action": "SET_LIGHT", "params": req.params })),
                    error: None,
                }
            }

            "LOAD_PRESET" => {
                let preset = req.params.get("preset").and_then(|v| v.as_str()).unwrap_or("mannequin");
                let _ = self.app_handle.emit("anigo://load_preset", json!({ "preset": preset }));
                BridgeResponse {
                    id: req.id.clone(),
                    success: true,
                    data: Some(json!({ "action": "LOAD_PRESET", "preset": preset })),
                    error: None,
                }
            }

            "SET_OUTLINE" => {
                let width = req.params.get("width").and_then(|v| v.as_f64()).unwrap_or(3.5) as f32;
                let _ = self.app_handle.emit("anigo://set_outline", json!({ "width": width }));
                BridgeResponse {
                    id: req.id.clone(),
                    success: true,
                    data: Some(json!({ "action": "SET_OUTLINE", "width": width })),
                    error: None,
                }
            }

            "SET_MATERIAL_TOON" | "SET_MATERIAL" => {
                let _ = self.app_handle.emit("anigo://set_material_toon", req.params.clone());
                BridgeResponse {
                    id: req.id.clone(),
                    success: true,
                    data: Some(json!({ "action": "SET_MATERIAL_TOON", "params": req.params })),
                    error: None,
                }
            }

            "SET_PROPORTIONS" => {
                let scale = req.params.get("head_scale").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
                let ratio = req.params.get("head_ratio").and_then(|v| v.as_f64()).unwrap_or(6.5) as f32;
                let _ = self.app_handle.emit("anigo://set_proportions", json!({ "head_scale": scale, "head_ratio": ratio }));
                BridgeResponse {
                    id: req.id.clone(),
                    success: true,
                    data: Some(json!({ "action": "SET_PROPORTIONS", "head_scale": scale, "head_ratio": ratio })),
                    error: None,
                }
            }

            "SET_CHARACTER_MODEL" => {
                let model_type = req.params.get("model_type").and_then(|v| v.as_str()).unwrap_or("male");
                let _ = self.app_handle.emit("anigo://set_character_model", json!({ "model_type": model_type }));
                BridgeResponse {
                    id: req.id.clone(),
                    success: true,
                    data: Some(json!({ "action": "SET_CHARACTER_MODEL", "model_type": model_type })),
                    error: None,
                }
            }

            "SET_SOMATOTYPE" => {
                let _ = self.app_handle.emit("anigo://set_somatotype", req.params.clone());
                BridgeResponse {
                    id: req.id.clone(),
                    success: true,
                    data: Some(json!({ "action": "SET_SOMATOTYPE", "params": req.params })),
                    error: None,
                }
            }

            "APPLY_MORPH_SLIDER" => {
                let _ = self.app_handle.emit("anigo://apply_morph_slider", req.params.clone());
                BridgeResponse {
                    id: req.id.clone(),
                    success: true,
                    data: Some(json!({ "action": "APPLY_MORPH_SLIDER", "params": req.params })),
                    error: None,
                }
            }

            "RESET_MORPHS" => {
                let _ = self.app_handle.emit("anigo://reset_morphs", json!({}));
                BridgeResponse {
                    id: req.id.clone(),
                    success: true,
                    data: Some(json!({ "action": "RESET_MORPHS" })),
                    error: None,
                }
            }

            // ─────────────────────────────────────────────────────────────
            // UI ACTIONS  (dispara ações semânticas na interface Svelte)
            // ─────────────────────────────────────────────────────────────
            "UI_ACTION" => {
                let _ = self.app_handle.emit("anigo://ui_action", req.params.clone());
                BridgeResponse {
                    id: req.id.clone(),
                    success: true,
                    data: Some(json!({ "action": "UI_ACTION", "params": req.params })),
                    error: None,
                }
            }

            // ─────────────────────────────────────────────────────────────
            // TELEMETRIA
            // ─────────────────────────────────────────────────────────────
            "GET_STATUS" => {
                let state = self.state.read().await;
                BridgeResponse {
                    id: req.id.clone(),
                    success: true,
                    data: Some(serde_json::to_value(&*state).unwrap_or(json!({}))),
                    error: None,
                }
            }

            // P2: runtime metrics for observability
            "GET_METRICS" => BridgeResponse {
                id: req.id.clone(),
                success: true,
                data: Some(self.metrics_snapshot()),
                error: None,
            },

            unknown => BridgeResponse {
                id: req.id.clone(),
                success: false,
                data: None,
                error: Some(format!("Unsupported bridge action: '{}'", unknown)),
            },
        }
    }
}
