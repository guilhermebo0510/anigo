use std::sync::Arc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;

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
}

impl Default for LiveWindowState {
    fn default() -> Self {
        Self {
            fps: 120.0,
            frame_time_ms: 0.5,
            draw_calls: 2,
            triangle_count: 156,
            adapter_name: "WebGPU Native Hardware".into(),
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
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BridgeRequest {
    pub id: String,
    pub action: String,
    #[serde(default)]
    pub params: Value,
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

pub struct LiveBridgeServer {
    app_handle: AppHandle,
    state: Arc<RwLock<LiveWindowState>>,
}

impl LiveBridgeServer {
    pub fn new(app_handle: AppHandle, state: Arc<RwLock<LiveWindowState>>) -> Self {
        Self { app_handle, state }
    }

    pub async fn run(self: Arc<Self>, bind_addr: &str) {
        let listener = match TcpListener::bind(bind_addr).await {
            Ok(l) => {
                eprintln!("[ANIGO Live Bridge] Production server listening on {}", bind_addr);
                l
            }
            Err(e) => {
                eprintln!("[ANIGO Live Bridge] ERROR binding to {}: {:#}", bind_addr, e);
                return;
            }
        };

        loop {
            match listener.accept().await {
                Ok((socket, peer)) => {
                    let server = Arc::clone(&self);
                    tokio::spawn(async move {
                        server.handle_client(socket, peer).await;
                    });
                }
                Err(e) => {
                    eprintln!("[ANIGO Live Bridge] Socket accept error: {:#}", e);
                }
            }
        }
    }

    async fn handle_client(&self, socket: TcpStream, _peer: std::net::SocketAddr) {
        use tokio::time::{timeout, Duration};
        const MAX_LINE: usize = 1 << 20; // 1MB max frame - P0-03 hardening
        const READ_TIMEOUT_SECS: u64 = 5;

        let (reader, mut writer) = socket.into_split();
        let mut buf_reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();
            let read_fut = buf_reader.read_line(&mut line);
            let read_res = match timeout(Duration::from_secs(READ_TIMEOUT_SECS), read_fut).await {
                Ok(r) => r,
                Err(_) => {
                    // timeout, close connection to avoid hanging
                    break;
                }
            };

            let n = match read_res {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };

            if n > MAX_LINE {
                let err_res = BridgeResponse {
                    id: "unknown".into(),
                    success: false,
                    data: None,
                    error: Some(format!("Frame too large: {} > {} bytes", n, MAX_LINE)),
                };
                if let Ok(mut resp_bytes) = serde_json::to_vec(&err_res) {
                    resp_bytes.push(b'\n');
                    let _ = writer.write_all(&resp_bytes).await;
                    let _ = writer.flush().await;
                }
                break;
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let req: BridgeRequest = match serde_json::from_str(trimmed) {
                Ok(r) => r,
                Err(e) => {
                    let err_res = BridgeResponse {
                        id: "unknown".into(),
                        success: false,
                        data: None,
                        error: Some(format!("Invalid JSON frame: {}", e)),
                    };
                    if let Ok(mut resp_bytes) = serde_json::to_vec(&err_res) {
                        resp_bytes.push(b'\n');
                        let _ = writer.write_all(&resp_bytes).await;
                        let _ = writer.flush().await;
                    }
                    continue;
                }
            };

            let response = self.dispatch_action(&req).await;
            if let Ok(mut resp_bytes) = serde_json::to_vec(&response) {
                if resp_bytes.len() > MAX_LINE {
                    let err_res = BridgeResponse {
                        id: req.id.clone(),
                        success: false,
                        data: None,
                        error: Some(format!("Response too large: {} bytes", resp_bytes.len())),
                    };
                    resp_bytes = serde_json::to_vec(&err_res).unwrap_or_default();
                }
                resp_bytes.push(b'\n');
                let write_fut = writer.write_all(&resp_bytes);
                if timeout(Duration::from_secs(2), write_fut).await.is_err() {
                    break;
                }
                let flush_fut = writer.flush();
                if timeout(Duration::from_secs(1), flush_fut).await.is_err() {
                    break;
                }
            }
        }
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

            unknown => BridgeResponse {
                id: req.id.clone(),
                success: false,
                data: None,
                error: Some(format!("Unsupported bridge action: '{}'", unknown)),
            },
        }
    }
}
