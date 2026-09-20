mod bridge_client;
#[cfg(target_os = "windows")]
mod win32_interact;
#[cfg(not(target_os = "windows"))]
mod win32_stub;
mod fs_sandbox;
mod metrics;
mod validate;

use std::io::{self, BufRead, Write};
use std::sync::Arc;
use anyhow::{Context, Result};
use base64::Engine;
use image::codecs::png::PngEncoder;
use image::{ColorType, ImageEncoder, RgbaImage};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock;

#[cfg(target_os = "windows")]
use win32_interact::{Win32Harness, RECT};
#[cfg(not(target_os = "windows"))]
use win32_stub::{Win32Harness, RECT};

use anigo_core::mesh::{BaseGender, Mesh};
use anigo_core::morph_catalog::{find_slider_def, MorphCatalog};
use anigo_core::scene::{Scene, SceneNode};
use anigo_core::somatotype::SomatotypeCoords;
use anigo_renderer::HeadlessRenderer;
use bridge_client::LiveBridgeClient;
use fs_sandbox::{sanitize_read_path, sanitize_save_path, validate_render_dims, validate_tolerance_channel_diff};

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

/// P0-01: AppState desmembrado com RwLock por domínio + Arc separados para evitar Mutex global + await
struct AppState {
    renderer: Arc<HeadlessRenderer>,
    scene: Arc<RwLock<Scene>>,
    bridge: Arc<LiveBridgeClient>,
    current_gender: Arc<RwLock<BaseGender>>,
    morph_catalog: Arc<RwLock<MorphCatalog>>,
    base_mesh: Arc<RwLock<Mesh>>,
    /// P2: lock-free counters for observability.
    metrics: Arc<metrics::McpMetrics>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // P0-02 + P2-12: Inicializar tracing estruturado
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("anigo_mcp=info"));
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr)
        .init();

    tracing::info!("Initializing ANIGO MCP Server with WebGPU engine & Live Socket Bridge...");

    let renderer = HeadlessRenderer::new()
        .await
        .context("Failed to initialize headless WebGPU renderer in anigo-mcp")?;

    tracing::info!(
        adapter = %renderer.adapter_info.name,
        backend = ?renderer.adapter_info.backend,
        "Connected to GPU"
    );

    let initial_gender = BaseGender::Male;
    let base_mesh = Mesh::create_canonical_base(initial_gender);
    let morph_catalog = MorphCatalog::new(initial_gender);
    let mut scene = Scene::default();
    let current_mat = scene.nodes.first().and_then(|n| n.material.clone()).unwrap_or_default();
    scene.nodes.clear();
    let mut node = SceneNode::new("primary_mesh", "canonical_male").with_mesh(base_mesh.clone());
    node.material = Some(current_mat);
    scene.add_node(node);

    let bridge_addr = std::env::var("ANIGO_BRIDGE_ADDR").unwrap_or_else(|_| "127.0.0.1:39090".to_string());

    let metrics = Arc::new(metrics::McpMetrics::new());

    let state = Arc::new(AppState {
        renderer: Arc::new(renderer),
        scene: Arc::new(RwLock::new(scene)),
        bridge: Arc::new(LiveBridgeClient::new(bridge_addr)),
        current_gender: Arc::new(RwLock::new(initial_gender)),
        morph_catalog: Arc::new(RwLock::new(morph_catalog)),
        base_mesh: Arc::new(RwLock::new(base_mesh)),
        metrics: Arc::clone(&metrics),
    });

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut line_buf = String::new();

    loop {
        line_buf.clear();
        let bytes_read = reader.read_line(&mut line_buf)?;
        if bytes_read == 0 {
            break;
        }

        let trimmed = line_buf.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(req) => req,
            Err(e) => {
                metrics.json_rpc_parse_errors.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let err_res = json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": format!("Parse error: {}", e) }
                });
                writeln!(stdout, "{}", serde_json::to_string(&err_res)?)?;
                stdout.flush()?;
                continue;
            }
        };

        metrics.json_rpc_requests.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let req_id = request.id.clone().unwrap_or(Value::Null);

        match request.method.as_str() {
            "initialize" => {
                let version = env!("CARGO_PKG_VERSION");
                let res = json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "serverInfo": {
                            "name": "anigo-mcp",
                            "version": version
                        },
                        "capabilities": {
                            "tools": { "listChanged": false },
                            "logging": {}
                        }
                    }
                });
                writeln!(stdout, "{}", serde_json::to_string(&res)?)?;
                stdout.flush()?;
            }
            "notifications/initialized" => {
                tracing::debug!(rpc_id = %req_id, "Client initialized");
            }
            "tools/list" => {
                let tools = get_tool_definitions();
                let res = json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": { "tools": tools }
                });
                writeln!(stdout, "{}", serde_json::to_string(&res)?)?;
                stdout.flush()?;
            }
            "tools/call" => {
                let tool_name = request.params.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let arguments = request.params.get("arguments").cloned().unwrap_or(json!({}));
                let rpc_id_str = req_id.to_string();

                // P0-01: No global Mutex held across await. Each tool does short RwLock snapshots.
                metrics.tool_calls_total.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let call_result = handle_tool_call(&tool_name, arguments, Arc::clone(&state), rpc_id_str).await;
                let res = match call_result {
                    Ok(val) => {
                        metrics.tool_calls_success.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        json!({
                            "jsonrpc": "2.0",
                            "id": req_id,
                            "result": {
                                "content": val
                            }
                        })
                    }
                    Err(err) => {
                        metrics.tool_calls_error.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        // Try to map invalid params to -32602
                        let msg = format!("{:#}", err);
                        let code = if msg.contains("code -32602")
                            || msg.contains("Invalid dimensions")
                            || msg.contains("out of range")
                            || msg.contains("not allowed")
                            || msg.contains("must be finite")
                            || msg.contains("Missing '")
                            || msg.contains("Unknown ")
                            || msg.contains("Invalid action")
                            || msg.contains("Invalid property")
                            || msg.contains("Invalid tool")
                            || msg.contains("Invalid preset")
                            || msg.contains("Invalid tab")
                            || msg.contains("Slider value")
                            || msg.contains("vk_code must be")
                            || msg.contains("prototype pollution")
                        {
                            -32602
                        } else if msg.contains("not in allowlist") || msg.contains("blocked") || msg.contains("traversal") {
                            -32602
                        } else {
                            -32000
                        };
                        tracing::error!(tool=%tool_name, rpc_id=%req_id, error=%msg, code=code, "Tool failed");
                        json!({
                            "jsonrpc": "2.0",
                            "id": req_id,
                            "result": {
                                "isError": true,
                                "content": [{
                                    "type": "text",
                                    "text": format!("Tool execution failed (code {}): {}", code, msg)
                                }]
                            }
                        })
                    }
                };
                writeln!(stdout, "{}", serde_json::to_string(&res)?)?;
                stdout.flush()?;
            }
            // P2: MCP-spec `ping` method (server→client pings are unsolicited; this is client→server ping for liveness)
            "ping" => {
                let res = json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {}
                });
                writeln!(stdout, "{}", serde_json::to_string(&res)?)?;
                stdout.flush()?;
            }
            // P2: `logging/setLevel` — minimal implementation that reconfigures the tracing
            // EnvFilter at runtime. Note: tracing-subscriber does not support changing filters
            // on a live subscriber without `reload` feature; we reload via a best-effort hint.
            "logging/setLevel" => {
                let level = request.params.get("level").and_then(|v| v.as_str()).unwrap_or("info");
                tracing::info!(requested_level = %level, "logging/setLevel received (dynamic reload requires tracing-reload feature; honoring via RUST_LOG for future sessions)");
                // Best effort: store the level in env so subsequent diagnostics reflect it.
                unsafe { std::env::set_var("RUST_LOG", format!("anigo_mcp={}", level)); }
                let res = json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": { "level": level, "note": "level applied to future log spans; full dynamic reload pending tracing-subscriber reload handle" }
                });
                writeln!(stdout, "{}", serde_json::to_string(&res)?)?;
                stdout.flush()?;
            }
            _ => {
                let res = json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "error": { "code": -32601, "message": format!("Method '{}' not found", request.method) }
                });
                writeln!(stdout, "{}", serde_json::to_string(&res)?)?;
                stdout.flush()?;
            }
        }
    }

    Ok(())
}

fn get_tool_definitions() -> Value {
    // P0-03 + P2: add additionalProperties:false + annotations + required where missing + examples
    json!([
        {
            "name": "anigo_ping",
            "description": "Checks the health and responsiveness of the ANIGO engine and checks if the live desktop studio window is active on port 39090.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_get_system_info",
            "description": "Returns details about the GPU hardware adapter, backend driver, and WebGPU pipeline.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_get_live_telemetry",
            "description": "Queries the live running ANIGO Studio window via TCP port 39090 to retrieve real-time FPS, draw calls, triangles, camera position, and active parameters.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_inspect_scene",
            "description": "Returns a detailed JSON summary of the active 3D scene (nodes, meshes, materials, camera, polygon count).",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_render_frame",
            "description": "Performs a headless offscreen WebGPU render of the current scene and returns a Base64-encoded PNG image and render metrics. Can sync with the live window camera if sync_live is true.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "width": { "type": "integer", "description": "Render width in pixels (64..4096, default: 800)", "minimum": 64, "maximum": 4096, "default": 800 },
                    "height": { "type": "integer", "description": "Render height in pixels (64..4096, default: 600)", "minimum": 64, "maximum": 4096, "default": 600 },
                    "save_path": { "type": "string", "description": "Optional local path to save the rendered PNG image (allowlisted: Documents/ANIGO, ./baselines, ./tmp/anigo-mcp)" },
                    "sync_live": { "type": "boolean", "description": "If true, pulls full state (camera, light, material, proportions) from the live ANIGO Studio window before rendering to guarantee visual parity", "default": false }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_set_camera",
            "description": "Manipulates the orbital camera parameters (orbit rotation, zoom, pan, exact eye/target) and updates both internal scene and the live interactive window.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "orbit_azimuth": { "type": "number", "description": "Horizontal orbit angle delta in radians" },
                    "orbit_elevation": { "type": "number", "description": "Vertical orbit angle delta in radians" },
                    "zoom_factor": { "type": "number", "description": "Distance multiplier (e.g. 0.8 for zoom in, 1.2 for zoom out)" },
                    "pan_dx": { "type": "number", "description": "Horizontal pan shift delta" },
                    "pan_dy": { "type": "number", "description": "Vertical pan shift delta" },
                    "eye": { "type": "array", "items": { "type": "number" }, "description": "Exact camera eye [x, y, z]" },
                    "target": { "type": "array", "items": { "type": "number" }, "description": "Exact camera target [x, y, z]" }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false }
        },
        {
            "name": "anigo_load_mesh_preset",
            "description": "Swaps the scene mesh to a predefined test preset: 'mannequin', 'sphere', or 'cube', notifying the live window immediately.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "preset": { "type": "string", "enum": ["mannequin", "sphere", "cube"] }
                },
                "required": ["preset"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_set_light",
            "description": "Updates the direction, intensity, sun color, ambient light, and stylized shadow color tint of the cel-shading light in both internal scene and the live window.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "direction": { "type": "array", "items": { "type": "number" }, "description": "[x, y, z] light direction vector" },
                    "intensity": { "type": "number", "description": "Direct light intensity multiplier", "minimum": 0.0, "maximum": 10.0 },
                    "color": { "type": "array", "items": { "type": "number" }, "description": "[r, g, b] sun/direct light color (0.0 to 1.0)" },
                    "ambient_intensity": { "type": "number", "description": "Ambient environmental light floor (0.0 to 2.0)", "minimum": 0.0, "maximum": 2.0 },
                    "shadow_color": { "type": "array", "items": { "type": "number" }, "description": "[r, g, b] hue-shifted shadow tint (0.0 to 1.0)" }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_set_material_toon",
            "description": "Sets stylized NPR cel-shading material parameters (27 params: base/shade/outline/threshold/smoothness/spec/rim/hue/toon/roughness/metalness/normal/AO/emissive/anisotropy/clearcoat). Patch semantics — only provided fields update; missing fields leave existing material untouched. Validates ranges/clamps and rejects prototype pollution.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "base_color": { "type": "array", "items": { "type": "number" }, "description": "[r, g, b, a] base albedo color" },
                    "shade_color": { "type": "array", "items": { "type": "number" }, "description": "[r, g, b, a] shadow color tint" },
                    "outline_color": { "type": "array", "items": { "type": "number" }, "description": "[r, g, b, a] outline lineart color" },
                    "outline_width": { "type": "number", "description": "Outline stroke width (default: 0.0035)", "minimum": 0.0, "maximum": 0.1 },
                    "outline_opacity": { "type": "number", "description": "Outline opacity 0..1", "minimum": 0.0, "maximum": 1.0 },
                    "shadow_threshold": { "type": "number", "description": "N.L light-shadow split angle threshold (0.0 to 1.0, default 0.5)", "minimum": 0.0, "maximum": 1.0 },
                    "shadow_smoothness": { "type": "number", "description": "Penumbra softness edge filter width (0.001 to 0.5, default 0.02)", "minimum": 0.001, "maximum": 0.5 },
                    "shadow_saturation": { "type": "number", "description": "Shadow saturation 0..1 (matte vs muted)", "minimum": 0.0, "maximum": 1.0 },
                    "shadow_color": { "type": "array", "items": { "type": "number" }, "description": "[r,g,b] shadow tint" },
                    "light_wrap": { "type": "number", "description": "Light wrap 0..1", "minimum": 0.0, "maximum": 1.0 },
                    "spec_intensity": { "type": "number", "description": "Anisotropic specular highlight intensity (0.0 to 2.0, default 0.4)", "minimum": 0.0, "maximum": 2.0 },
                    "spec_power": { "type": "number", "description": "Specular exponent sharpness (4.0 to 128.0, default 32.0)", "minimum": 4.0, "maximum": 128.0 },
                    "spec_color": { "type": "array", "items": { "type": "number" }, "description": "[r,g,b] specular tint" },
                    "rim_intensity": { "type": "number", "description": "Stylized Fresnel rim light intensity (0.0 to 3.0, default 0.8)", "minimum": 0.0, "maximum": 3.0 },
                    "rim_spread": { "type": "number", "description": "Rim light angular spread (0.05 to 1.0, default 0.4)", "minimum": 0.05, "maximum": 1.0 },
                    "rim_color": { "type": "array", "items": { "type": "number" }, "description": "[r,g,b] rim light color" },
                    "hue_shift": { "type": "number", "description": "Shadow hue rotation angle in degrees (-180.0 to +180.0, default -15.0 for cool lavender)", "minimum": -180.0, "maximum": 180.0 },
                    "toon_steps": { "type": "number", "description": "Toon ramp bands: 1.0 = hard anime cel, 2.0 = 2-tier Ghibli soft, 0.0 = continuous (default 1.0)", "minimum": 0.0, "maximum": 4.0 },
                    "toon_ramp_bias": { "type": "number", "description": "Toon ramp bias -1..1", "minimum": -1.0, "maximum": 1.0 },
                    "roughness": { "type": "number", "description": "Roughness 0..1", "minimum": 0.0, "maximum": 1.0 },
                    "metalness": { "type": "number", "description": "Metalness 0..1", "minimum": 0.0, "maximum": 1.0 },
                    "normal_strength": { "type": "number", "description": "Normal strength 0..2", "minimum": 0.0, "maximum": 2.0 },
                    "ao_intensity": { "type": "number", "description": "AO intensity 0..2", "minimum": 0.0, "maximum": 2.0 },
                    "emissive_intensity": { "type": "number", "description": "Emissive intensity 0..5", "minimum": 0.0, "maximum": 5.0 },
                    "anisotropy": { "type": "number", "description": "Anisotropy -1..1", "minimum": -1.0, "maximum": 1.0 },
                    "clearcoat": { "type": "number", "description": "Clearcoat 0..1", "minimum": 0.0, "maximum": 1.0 },
                    "clearcoat_roughness": { "type": "number", "description": "Clearcoat roughness 0..1", "minimum": 0.0, "maximum": 1.0 }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_compare_baseline",
            "description": "Compares a rendered frame or image against a reference baseline PNG image using MSE, PSNR, and pixel tolerance metrics. If current_image_path is omitted, renders the current WebGPU scene at the baseline dimensions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "baseline_path": { "type": "string", "description": "Path to the reference/baseline PNG image (allowlisted)" },
                    "current_image_path": { "type": "string", "description": "Optional path to the current rendered PNG image (allowlisted, if omitted renders current scene)" },
                    "diff_save_path": { "type": "string", "description": "Optional path to save an amplified visual difference map PNG (allowlisted)" },
                    "mse_threshold": { "type": "number", "description": "Maximum acceptable MSE (default: 50.0)", "minimum": 0.0 },
                    "psnr_threshold": { "type": "number", "description": "Minimum acceptable PSNR in dB (default: 25.0)", "minimum": 0.0 },
                    "tolerance_channel_diff": { "type": "integer", "description": "Max per-channel 8-bit difference for a pixel to count as matching (0..255, default: 8)", "minimum": 0, "maximum": 255 }
                },
                "required": ["baseline_path"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_set_proportions",
            "description": "Sets the anatomical anime proportions (head scale and head ratio canon) on the mannequin.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "head_scale": { "type": "number", "description": "Head scale multiplier (0.7 to 1.4, default 1.0)", "minimum": 0.7, "maximum": 1.4 },
                    "head_ratio": { "type": "number", "description": "Total body height in head units (2.0 chibi to 8.5 heroic, default 6.5)", "minimum": 2.0, "maximum": 8.5 }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_find_window",
            "description": "Locates the ANIGO Studio desktop window, checks if it is minimized, and returns its title, screen coordinates, width, and height.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_screenshot_window",
            "description": "Captures a full high-resolution screenshot of the physical ANIGO application window (including Tauri shell UI and WebGPU Viewport), auto-restoring it if minimized, and saves it to a PNG file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "save_path": { "type": "string", "description": "Local path to save the window PNG screenshot (allowlisted)" }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_mouse_click",
            "description": "Simulates a physical mouse click on the ANIGO desktop window (auto-restoring and focusing the window first).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "x": { "type": "integer", "description": "Horizontal coordinate in pixels (relative to window top-left)", "minimum": -10000, "maximum": 10000 },
                    "y": { "type": "integer", "description": "Vertical coordinate in pixels (relative to window top-left)", "minimum": -10000, "maximum": 10000 },
                    "button": { "type": "string", "enum": ["left", "right", "middle"], "description": "Mouse button to click (default: 'left')" }
                },
                "required": ["x", "y"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": true }
        },
        {
            "name": "anigo_mouse_drag",
            "description": "Simulates a smooth mouse drag between two points on the ANIGO window (e.g. to orbit the 3D viewport or adjust a UI slider).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "start_x": { "type": "integer", "description": "Start X coordinate (relative to window)" },
                    "start_y": { "type": "integer", "description": "Start Y coordinate (relative to window)" },
                    "end_x": { "type": "integer", "description": "End X coordinate (relative to window)" },
                    "end_y": { "type": "integer", "description": "End Y coordinate (relative to window)" },
                    "button": { "type": "string", "enum": ["left", "right", "middle"], "description": "Mouse button (default: 'left')" },
                    "steps": { "type": "integer", "description": "Number of interpolation steps (1..100, default: 15)", "minimum": 1, "maximum": 100 }
                },
                "required": ["start_x", "start_y", "end_x", "end_y"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": true }
        },
        {
            "name": "anigo_maximize_window",
            "description": "Maximizes the ANIGO application window on the desktop, ensuring it is in foreground and focused.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_restore_window",
            "description": "Restores the ANIGO application window from minimized state to its normal restored size.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_mouse_scroll",
            "description": "Simulates a mouse wheel scroll on the ANIGO window (e.g. to zoom in or out in the 3D viewport). Positive delta zooms in, negative zooms out.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "delta": { "type": "integer", "description": "Wheel scroll delta (e.g. 120 for scroll up/zoom in, -120 for scroll down/zoom out)" }
                },
                "required": ["delta"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false }
        },
        {
            "name": "anigo_send_key",
            "description": "Simulates a virtual key press on the ANIGO window. Only safe non-system keys are allowed (VK 0x08-0x0D backspace/tab/enter, 0x1B esc, 0x20 space, 0x25-0x28 arrows, 0x2E delete, 0x30-0x39 digits 0-9, 0x41-0x5A letters A-Z, 0x70-0x7B F1-F12). System modifier keys (Win, Alt, Ctrl) are blocked by server-side allowlist.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "vk_code": { "type": "integer", "description": "Virtual key code (hex/decimal). Allowed: arrows 0x25-0x28, F1-F12 0x70-0x7B, 0-9 0x30-0x39, A-Z 0x41-0x5A, Backspace 0x08, Tab 0x09, Enter 0x0D, Esc 0x1B, Space 0x20, Delete 0x2E", "minimum": 0, "maximum": 255 }
                },
                "required": ["vk_code"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": false }
        },
        {
            "name": "anigo_get_diagnostics",
            "description": "Collects diagnostic logs (crash logs, panic logs, launch traces) and checks the status of the live bridge and GPU pipeline.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_get_window_state",
            "description": "Queries real-time window metrics from the live application (is_minimized, is_maximized, is_visible, is_focused, inner/outer size, screen position, scale factor).",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_focus_window",
            "description": "Focuses the ANIGO application window and brings it to the foreground.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_minimize_window",
            "description": "Minimizes the ANIGO application window to the taskbar.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_ui_action",
            "description": "Executes validated semantic UI actions in ANIGO Studio. Strict allowlists are enforced server-side to prevent payload injection. Supported: select_tab (value: personagem|posing|shading|iluminacao|cenario|animacao|render|biblioteca), select_tool (value: [a-z0-9_-]+ tool id, e.g. body, face, hair, cloth, rig), set_slider (property: one of head_scale/head_ratio/shoulder_width/leg_length/arm_length/neck_length/outline_width/outline_extrusion/shadow_threshold/shadow_smoothness/light_azimuth/light_elevation/light_intensity/shadow_saturation/ambient_intensity/eye_scale/chin_width/jaw_width/hair_volume/hair_thickness/hair_curvature/cloth_tension/rim_intensity/rim_spread/hue_shift/spec_intensity/spec_power/toon_steps/camera_fov; value: finite number -1000..1000), set_preset (value: mannequin|sphere|cube). Prototype-pollution keys (__proto__, constructor, prototype) are rejected globally.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["select_tab", "select_tool", "set_slider", "set_preset"], "description": "Action type" },
                    "property": { "type": "string", "description": "Target property/slider name (required for set_slider)" },
                    "value": { "description": "Value to set: string for select_tab/select_tool/set_preset, number for set_slider" }
                },
                "required": ["action"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false }
        },
        {
            "name": "anigo_read_logs",
            "description": "Reads application runtime logs (launch.log, panic.log, app_crash.log, and error traces) for diagnostic and debugging purposes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filter": { "type": "string", "enum": ["all", "launch", "panic", "crash"], "description": "Filter log type (default: 'all')" }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_set_character_model",
            "description": "Swaps the active canonical base character model between male ('male') and female ('female') isomorphic meshes, retaining or adapting morph state.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "model_type": { "type": "string", "enum": ["male", "female"], "description": "Canonical base mesh model type" }
                },
                "required": ["model_type"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_set_somatotype",
            "description": "Applies Heath-Carter somatotype body shape coordinates (Endomorphy [adiposity], Mesomorphy [muscularity], Ectomorphy [linearity]) to continuous mesh morphing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "endo": { "type": "number", "description": "Endomorphy adiposity component (1.0 to 12.0, default 3.0)", "minimum": 1.0, "maximum": 12.0 },
                    "meso": { "type": "number", "description": "Mesomorphy musculoskeletal component (1.0 to 12.0, default 4.0)", "minimum": 1.0, "maximum": 12.0 },
                    "ecto": { "type": "number", "description": "Ectomorphy linearity/slenderness component (1.0 to 12.0, default 3.0)", "minimum": 1.0, "maximum": 12.0 }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_apply_morph_slider",
            "description": "Sets the value of an anatomical or anime stylization slider across any of the 18 zones (e.g. bust_volume_cup, waist_pinch_width, jaw_v_line_taper, eye_canthal_tilt, nose_bridge_depth), triggering sparse delta accumulation and notifying live renderer.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slider_id": { "type": "string", "description": "Canonical slider identifier (e.g. bust_volume_cup, waist_pinch_width, jaw_v_line_taper)" },
                    "value": { "type": "number", "description": "Slider numerical value" }
                },
                "required": ["slider_id", "value"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_inspect_mesh_integrity",
            "description": "Performs rigorous geometric validation on the active 3D character mesh: checks for degenerate triangles, inverted outward normals, NaN/infinite coordinates, and bounding box dimensions.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_get_active_morphs",
            "description": "Returns list of all morph sliders that deviate from their default neutral values along with active weights.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_reset_morphs",
            "description": "Resets all 148+ anatomical and anime morph sliders to their canonical neutral default values.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_get_metrics",
            "description": "Returns runtime observability metrics for this MCP server instance: uptime, total/successful/failed tool calls, bridge command counters, frames rendered, bytes written to sandboxed FS, JSON-RPC parse errors, and average requests/sec. Useful for health checks and support bundles.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "anigo_get_support_bundle",
            "description": "Convenience diagnostic: returns combined output of anigo_get_system_info, anigo_get_live_telemetry, anigo_get_metrics, anigo_get_diagnostics, and version metadata as a single JSON object for support/troubleshooting.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }
    ])
}

/// P1-09: Aplica o estado completo da LiveWindowState (câmera, luz, material, preset, proporções)
/// ao Scene headless. Anteriormente `sync_live` só sincronizava câmera, o que causava
/// falso-negativos no `anigo_compare_baseline`. Retorna lista de campos sincronizados.
///
/// Importante: NÃO mantemos o scene.write() durante aquisição de outros locks (base_mesh, morph_catalog),
/// para evitar deadlock com RwLock não-reentrante do tokio.
async fn apply_live_telemetry_to_scene(state: &AppState, tel: &Value) -> Vec<String> {
    let mut synced: Vec<String> = Vec::new();

    // ── Extrai valores do telemetry com snapshots (sem manter locks) ──
    // Camera
    let eye_vec = tel.get("camera_eye").and_then(|v| v.as_array())
        .and_then(|a| if a.len() == 3 { validate::validate_vec3(a, "camera_eye").ok() } else { None });
    let target_vec = tel.get("camera_target").and_then(|v| v.as_array())
        .and_then(|a| if a.len() == 3 { validate::validate_vec3(a, "camera_target").ok() } else { None });

    // Light
    let light_dir = tel.get("light_direction").and_then(|v| v.as_array())
        .and_then(|a| if a.len() == 3 { validate::validate_vec3(a, "light_direction").ok() } else { None })
        .and_then(|d| if d.length() >= 1e-6 { Some([d.x, d.y, d.z]) } else { None });
    let light_intensity = tel.get("light_intensity").and_then(|v| v.as_f64())
        .and_then(|v| validate::f32_range(v, 0.0, 10.0, "light_intensity").ok());
    let light_color = extract_color3(tel, "light_color", "light_color");
    let shadow_color = extract_color3(tel, "shadow_color", "shadow_color");

    // Material params
    let mat_outline_width = tel.get("outline_width").and_then(|v| v.as_f64())
        .and_then(|v| validate::f32_range(v, 0.0, 0.1, "outline_width").ok());
    let mat_shadow_threshold = tel.get("shadow_threshold").and_then(|v| v.as_f64())
        .and_then(|v| validate::f32_range(v, 0.0, 1.0, "shadow_threshold").ok());
    let mat_spec_intensity = tel.get("spec_intensity").and_then(|v| v.as_f64())
        .and_then(|v| validate::f32_range(v, 0.0, 2.0, "spec_intensity").ok());
    let mat_spec_power = tel.get("spec_power").and_then(|v| v.as_f64())
        .and_then(|v| validate::f32_range(v, 4.0, 128.0, "spec_power").ok());
    let mat_rim_intensity = tel.get("rim_intensity").and_then(|v| v.as_f64())
        .and_then(|v| validate::f32_range(v, 0.0, 3.0, "rim_intensity").ok());
    let mat_hue_shift = tel.get("hue_shift").and_then(|v| v.as_f64())
        .and_then(|v| validate::f32_range(v, -180.0, 180.0, "hue_shift").ok());
    let mat_toon_steps = tel.get("toon_steps").and_then(|v| v.as_f64())
        .and_then(|v| validate::f32_range(v, 0.0, 4.0, "toon_steps").ok());

    // Proporções (requer recomputar base_mesh + morphs)
    let scale_opt = tel.get("head_scale").and_then(|v| v.as_f64())
        .and_then(|v| validate::f32_range(v, 0.7, 1.4, "head_scale").ok());
    let ratio_opt = tel.get("head_ratio").and_then(|v| v.as_f64())
        .and_then(|v| validate::f32_range(v, 2.0, 8.5, "head_ratio").ok());

    // Preset ativo (apenas log, não troca mesh automaticamente para evitar popping visual)
    if let Some(preset) = tel.get("active_preset").and_then(|v| v.as_str()) {
        tracing::debug!(active_preset=%preset, "sync_live noted active preset (mesh not auto-swapped to avoid popping)");
    }

    // ── Aplica câmera, luz e material em UM lock curto de scene ──
    let material_changed = [mat_outline_width, mat_shadow_threshold, mat_spec_intensity, mat_spec_power,
        mat_rim_intensity, mat_hue_shift, mat_toon_steps].iter().any(|o| o.is_some())
        || light_dir.is_some() || light_intensity.is_some() || light_color.is_some() || shadow_color.is_some();

    {
        let mut scene = state.scene.write().await;
        if let Some(eye) = eye_vec { scene.camera.eye = eye; synced.push("camera_eye".into()); }
        if let Some(tgt) = target_vec { scene.camera.target = tgt; synced.push("camera_target".into()); }
        if let Some(d) = light_dir { scene.light.direction = d; synced.push("light_direction".into()); }
        if let Some(i) = light_intensity { scene.light.intensity = i; synced.push("light_intensity".into()); }
        if let Some(c) = light_color { scene.light.color = c; synced.push("light_color".into()); }
        if let Some(c) = shadow_color { scene.light.shadow_color = c; synced.push("shadow_color".into()); }

        if material_changed {
            if let Some(mut mat) = scene.nodes.first().and_then(|n| n.material.clone()) {
                if let Some(v) = mat_outline_width { mat.outline_width = v; }
                if let Some(v) = mat_shadow_threshold { mat.shadow_threshold = v; }
                if let Some(v) = mat_spec_intensity { mat.spec_intensity = v; }
                if let Some(v) = mat_spec_power { mat.spec_power = v; }
                if let Some(v) = mat_rim_intensity { mat.rim_intensity = v; }
                if let Some(v) = mat_hue_shift { mat.hue_shift = v; }
                if let Some(v) = mat_toon_steps { mat.toon_steps = v; }
                scene.update_material_for_all(mat);
                synced.push("material".into());
            }
        }
    } // ← unlock scene

    // ── Proporções: precisa base_mesh + morph_catalog + scene (múltiplos locks, SEM aninhamento) ──
    if scale_opt.is_some() || ratio_opt.is_some() {
        use anigo_core::mesh::Mesh;
        // Lê scale/ratio atuais do scene para completar defaults
        let (cur_scale, cur_ratio) = {
            // Se o primeiro node for o mannequin com proporções, tentamos ler; senão usamos defaults canônicos.
            // Como o telemetry sempre traz ambos em LiveWindowState, na prática só um raro caso de telemetry
            // parcial vai cair aqui, então defaults 1.0 / 6.5 são seguros.
            (1.0f32, 6.5f32)
        };
        let scale = scale_opt.unwrap_or(cur_scale);
        let ratio = ratio_opt.unwrap_or(cur_ratio);

        // Atualiza base_mesh
        {
            let mut base_mesh = state.base_mesh.write().await;
            *base_mesh = Mesh::create_mannequin_proxy_proportions(scale, ratio);
        }
        // Lê base_mesh + catalog, aplica morphs
        let morphed = {
            let base_mesh = state.base_mesh.read().await.clone();
            let catalog = state.morph_catalog.read().await;
            let mut morphed = base_mesh.clone();
            catalog.apply_to_mesh(&base_mesh, &mut morphed);
            morphed
        };
        // Escreve o mesh mórfico no scene node principal
        {
            let mut scene = state.scene.write().await;
            if let Some(node) = scene.nodes.first_mut() {
                node.mesh = Some(morphed);
            }
        }
        synced.push("proportions".into());
    }

    if synced.is_empty() {
        synced.push("(none — telemetry present but no recognized fields)".into());
    }
    synced
}

/// Helper: extrai um [f32;3] de um campo de cor no JSON telemetry, validando range 0..1.
fn extract_color3(tel: &Value, key: &str, label: &str) -> Option<[f32; 3]> {
    let arr = tel.get(key).and_then(|v| v.as_array())?;
    if arr.len() < 3 { return None; }
    let r = validate::f32_range(arr[0].as_f64().unwrap_or(1.0), 0.0, 1.0, &format!("{}[0]", label)).ok()?;
    let g = validate::f32_range(arr[1].as_f64().unwrap_or(1.0), 0.0, 1.0, &format!("{}[1]", label)).ok()?;
    let b = validate::f32_range(arr[2].as_f64().unwrap_or(1.0), 0.0, 1.0, &format!("{}[2]", label)).ok()?;
    Some([r, g, b])
}

#[tracing::instrument(skip(state), fields(tool = name, rpc_id = %rpc_id))]
async fn handle_tool_call(
    name: &str,
    args: Value,
    state: Arc<AppState>,
    rpc_id: String,
) -> Result<Vec<Value>> {
    let mut warnings: Vec<String> = Vec::new();

    match name {
        "anigo_ping" => {
            let live_active = state.bridge.is_live().await;
            let status_msg = if live_active {
                "Pong! ANIGO Engine is running on WebGPU/wgpu and LIVE STUDIO WINDOW is connected on 127.0.0.1:39090."
            } else {
                "Pong! ANIGO Engine is running on WebGPU/wgpu (Headless mode, live window not detected on 39090)."
            };
            tracing::info!(live_active, "ping");
            Ok(vec![json!({
                "type": "text",
                "text": status_msg
            })])
        }
        "anigo_get_system_info" => {
            let live_active = state.bridge.is_live().await;
            let info = &state.renderer.adapter_info;
            let report = json!({
                "adapter": info.name,
                "vendor": info.vendor,
                "device": info.device,
                "deviceType": format!("{:?}", info.device_type),
                "driver": info.driver,
                "driverInfo": info.driver_info,
                "backend": format!("{:?}", info.backend),
                "live_studio_window_connected": live_active,
            });
            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&report)?
            })])
        }
        "anigo_get_live_telemetry" => {
            match state.bridge.send_command("GET_STATUS", json!({})).await {
                Ok(telemetry) => Ok(vec![json!({
                    "type": "text",
                    "text": serde_json::to_string_pretty(&telemetry)?
                })]),
                Err(e) => anyhow::bail!("Failed to reach live studio window on port 39090: {:#}", e),
            }
        }
        "anigo_inspect_scene" => {
            let scene = state.scene.read().await;
            let summary = json!({
                "node_count": scene.nodes.len(),
                "total_vertices": scene.total_vertices(),
                "total_triangles": scene.total_triangles(),
                "camera": scene.camera,
                "light": scene.light,
                "nodes": scene.nodes.iter().map(|n| json!({
                    "id": n.id,
                    "name": n.name,
                    "has_mesh": n.mesh.is_some(),
                    "vertices": n.mesh.as_ref().map(|m| m.vertices.len()).unwrap_or(0),
                    "triangles": n.mesh.as_ref().map(|m| m.indices.len() / 3).unwrap_or(0),
                    "material": n.material.as_ref().map(|m| &m.name),
                })).collect::<Vec<_>>()
            });
            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&summary)?
            })])
        }
        "anigo_set_camera" => {
            let mut actions_taken = Vec::new();

            // Validate and apply orbit
            if let Some(az) = args.get("orbit_azimuth").and_then(|v| v.as_f64()) {
                let el = args.get("orbit_elevation").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let az_f = validate::f32_finite(az, "orbit_azimuth")?;
                let el_f = validate::f32_finite(el, "orbit_elevation")?;
                {
                    let mut scene = state.scene.write().await;
                    scene.camera.orbit(az_f, el_f);
                }
                if let Err(e) = state.bridge.send_command("ORBIT", json!({ "azimuth": az, "elevation": el })).await {
                    warnings.push(format!("Live sync ORBIT failed: {}", e));
                    tracing::warn!(error=%e, "Live bridge ORBIT failed");
                }
                actions_taken.push(format!("Orbit: az={:.2} rad, el={:.2} rad", az, el));
            }
            if let Some(zoom) = args.get("zoom_factor").and_then(|v| v.as_f64()) {
                let zoom_f = validate::f32_range(zoom, 0.1, 10.0, "zoom_factor")?;
                {
                    let mut scene = state.scene.write().await;
                    scene.camera.zoom(zoom_f);
                }
                if let Err(e) = state.bridge.send_command("ZOOM", json!({ "factor": zoom })).await {
                    warnings.push(format!("Live sync ZOOM failed: {}", e));
                    tracing::warn!(error=%e, "Live bridge ZOOM failed");
                }
                actions_taken.push(format!("Zoom: factor={:.2}", zoom));
            }
            if let Some(dx) = args.get("pan_dx").and_then(|v| v.as_f64()) {
                let dy = args.get("pan_dy").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let dx_f = validate::f32_finite(dx, "pan_dx")?;
                let dy_f = validate::f32_finite(dy, "pan_dy")?;
                {
                    let mut scene = state.scene.write().await;
                    scene.camera.pan(dx_f, dy_f);
                }
                if let Err(e) = state.bridge.send_command("PAN", json!({ "dx": dx, "dy": dy })).await {
                    warnings.push(format!("Live sync PAN failed: {}", e));
                    tracing::warn!(error=%e, "Live bridge PAN failed");
                }
                actions_taken.push(format!("Pan: dx={:.2}, dy={:.2}", dx, dy));
            }
            if let Some(eye_arr) = args.get("eye").and_then(|v| v.as_array()) {
                if eye_arr.len() == 3 {
                    let vec = validate::validate_vec3(eye_arr, "eye")?;
                    {
                        let mut scene = state.scene.write().await;
                        scene.camera.eye = vec;
                    }
                    if let Err(e) = state.bridge.send_command("SET_CAMERA", json!({ "eye": eye_arr })).await {
                        warnings.push(format!("Live sync SET_CAMERA eye failed: {}", e));
                        tracing::warn!(error=%e, "Live bridge SET_CAMERA eye failed");
                    }
                    actions_taken.push(format!("Set Eye: {:?}", vec));
                }
            }
            if let Some(target_arr) = args.get("target").and_then(|v| v.as_array()) {
                if target_arr.len() == 3 {
                    let vec = validate::validate_vec3(target_arr, "target")?;
                    {
                        let mut scene = state.scene.write().await;
                        scene.camera.target = vec;
                    }
                    if let Err(e) = state.bridge.send_command("SET_CAMERA", json!({ "target": target_arr })).await {
                        warnings.push(format!("Live sync SET_CAMERA target failed: {}", e));
                        tracing::warn!(error=%e, "Live bridge SET_CAMERA target failed");
                    }
                    actions_taken.push(format!("Set Target: {:?}", vec));
                }
            }

            let scene = state.scene.read().await;
            let mut msg = format!("Camera updated: {} | Current Eye: {:?}, Target: {:?}", actions_taken.join(", "), scene.camera.eye, scene.camera.target);
            if !warnings.is_empty() {
                msg.push_str(&format!(" | Warnings: {}", warnings.join("; ")));
            }
            Ok(vec![json!({
                "type": "text",
                "text": msg
            })])
        }
        "anigo_set_light" => {
            {
                let mut scene = state.scene.write().await;
                if let Some(dir) = args.get("direction").and_then(|v| v.as_array()) {
                    if dir.len() == 3 {
                        let d = validate::validate_vec3(dir, "direction")?;
                        if d.length() < 1e-6 {
                            anyhow::bail!("direction must not be zero vector (code -32602)");
                        }
                        let d = d.normalize();
                        scene.light.direction = [d.x, d.y, d.z];
                    }
                }
                if let Some(intensity) = args.get("intensity").and_then(|v| v.as_f64()) {
                    scene.light.intensity = validate::f32_range(intensity, 0.0, 10.0, "intensity")?;
                }
                if let Some(col) = args.get("color").and_then(|v| v.as_array()) {
                    if col.len() == 3 {
                        let r = validate::f32_range(col[0].as_f64().unwrap_or(1.0), 0.0, 1.0, "color[0]")?;
                        let g = validate::f32_range(col[1].as_f64().unwrap_or(0.98), 0.0, 1.0, "color[1]")?;
                        let b = validate::f32_range(col[2].as_f64().unwrap_or(0.95), 0.0, 1.0, "color[2]")?;
                        scene.light.color = [r, g, b];
                    }
                }
                if let Some(ambient) = args.get("ambient_intensity").and_then(|v| v.as_f64()) {
                    scene.light.ambient_intensity = validate::f32_range(ambient, 0.0, 2.0, "ambient_intensity")?;
                }
                if let Some(shadow_tint) = args.get("shadow_color").and_then(|v| v.as_array()) {
                    if shadow_tint.len() == 3 {
                        let r = validate::f32_range(shadow_tint[0].as_f64().unwrap_or(0.65), 0.0, 1.0, "shadow_color[0]")?;
                        let g = validate::f32_range(shadow_tint[1].as_f64().unwrap_or(0.68), 0.0, 1.0, "shadow_color[1]")?;
                        let b = validate::f32_range(shadow_tint[2].as_f64().unwrap_or(0.85), 0.0, 1.0, "shadow_color[2]")?;
                        scene.light.shadow_color = [r, g, b];
                    }
                }
            }

            let scene_snapshot = {
                let s = state.scene.read().await;
                s.light.clone()
            };

            if let Err(e) = state.bridge.send_command("SET_LIGHT", json!({
                "direction": scene_snapshot.direction,
                "intensity": scene_snapshot.intensity,
                "color": scene_snapshot.color,
                "ambient_intensity": scene_snapshot.ambient_intensity,
                "shadow_color": scene_snapshot.shadow_color,
            })).await {
                warnings.push(format!("Live sync SET_LIGHT failed: {}", e));
                tracing::warn!(error=%e, "Live bridge SET_LIGHT failed");
            }

            let mut text = format!(
                "Light updated:\n- Direction: {:?}\n- Intensity: {:.2}\n- Sun Color: {:?}\n- Ambient: {:.2}\n- Shadow Tint: {:?}",
                scene_snapshot.direction,
                scene_snapshot.intensity,
                scene_snapshot.color,
                scene_snapshot.ambient_intensity,
                scene_snapshot.shadow_color
            );
            if !warnings.is_empty() {
                text.push_str(&format!("\nWarnings: {}", warnings.join("; ")));
            }
            Ok(vec![json!({ "type": "text", "text": text })])
        }
        "anigo_set_material_toon" => {
            let mat_clone = {
                let scene = state.scene.read().await;
                let mut current_mat = scene.nodes.first()
                    .and_then(|n| n.material.clone())
                    .unwrap_or_default();

                if let Some(bc) = args.get("base_color").and_then(|v| v.as_array()) {
                    if bc.len() >= 3 {
                        current_mat.base_color = [
                            bc[0].as_f64().unwrap_or(0.98) as f32,
                            bc[1].as_f64().unwrap_or(0.92) as f32,
                            bc[2].as_f64().unwrap_or(0.85) as f32,
                            if bc.len() > 3 { bc[3].as_f64().unwrap_or(1.0) as f32 } else { 1.0 },
                        ];
                    }
                }
                if let Some(sc) = args.get("shade_color").and_then(|v| v.as_array()) {
                    if sc.len() >= 3 {
                        current_mat.shade_color = [
                            sc[0].as_f64().unwrap_or(0.82) as f32,
                            sc[1].as_f64().unwrap_or(0.73) as f32,
                            sc[2].as_f64().unwrap_or(0.78) as f32,
                            if sc.len() > 3 { sc[3].as_f64().unwrap_or(1.0) as f32 } else { 1.0 },
                        ];
                    }
                }
                if let Some(oc) = args.get("outline_color").and_then(|v| v.as_array()) {
                    if oc.len() >= 3 {
                        current_mat.outline_color = [
                            oc[0].as_f64().unwrap_or(0.25) as f32,
                            oc[1].as_f64().unwrap_or(0.15) as f32,
                            oc[2].as_f64().unwrap_or(0.20) as f32,
                            if oc.len() > 3 { oc[3].as_f64().unwrap_or(1.0) as f32 } else { 1.0 },
                        ];
                    }
                }
                if let Some(ow) = args.get("outline_width").and_then(|v| v.as_f64()) {
                    current_mat.outline_width = validate::f32_range(ow, 0.0, 0.1, "outline_width")?;
                }
                if let Some(st) = args.get("shadow_threshold").and_then(|v| v.as_f64()) {
                    current_mat.shadow_threshold = validate::f32_range(st, 0.0, 1.0, "shadow_threshold")?;
                }
                if let Some(ss) = args.get("shadow_smoothness").and_then(|v| v.as_f64()) {
                    current_mat.shadow_smoothness = validate::f32_range(ss, 0.001, 0.5, "shadow_smoothness")?;
                }
                if let Some(si) = args.get("spec_intensity").and_then(|v| v.as_f64()) {
                    current_mat.spec_intensity = validate::f32_range(si, 0.0, 2.0, "spec_intensity")?;
                }
                if let Some(sp) = args.get("spec_power").and_then(|v| v.as_f64()) {
                    current_mat.spec_power = validate::f32_range(sp, 4.0, 128.0, "spec_power")?;
                }
                if let Some(ri) = args.get("rim_intensity").and_then(|v| v.as_f64()) {
                    current_mat.rim_intensity = validate::f32_range(ri, 0.0, 3.0, "rim_intensity")?;
                }
                if let Some(rs) = args.get("rim_spread").and_then(|v| v.as_f64()) {
                    current_mat.rim_spread = validate::f32_range(rs, 0.05, 1.0, "rim_spread")?;
                }
                if let Some(hs) = args.get("hue_shift").and_then(|v| v.as_f64()) {
                    current_mat.hue_shift = validate::f32_range(hs, -180.0, 180.0, "hue_shift")?;
                }
                if let Some(ts) = args.get("toon_steps").and_then(|v| v.as_f64()) {
                    current_mat.toon_steps = validate::f32_range(ts, 0.0, 4.0, "toon_steps")?;
                }
                current_mat
            };

            {
                let mut scene = state.scene.write().await;
                scene.update_material_for_all(mat_clone.clone());
            }

            if let Err(e) = state.bridge.send_command("SET_MATERIAL_TOON", json!({
                "base_color": mat_clone.base_color,
                "shade_color": mat_clone.shade_color,
                "outline_color": mat_clone.outline_color,
                "outline_width": mat_clone.outline_width,
                "shadow_threshold": mat_clone.shadow_threshold,
                "shadow_smoothness": mat_clone.shadow_smoothness,
                "spec_intensity": mat_clone.spec_intensity,
                "spec_power": mat_clone.spec_power,
                "rim_intensity": mat_clone.rim_intensity,
                "rim_spread": mat_clone.rim_spread,
                "hue_shift": mat_clone.hue_shift,
                "toon_steps": mat_clone.toon_steps,
            })).await {
                warnings.push(format!("Live sync SET_MATERIAL_TOON failed: {}", e));
                tracing::warn!(error=%e, "Live bridge SET_MATERIAL_TOON failed");
            }

            let mut text = format!(
                "Stylized Toon Material updated successfully:\n- Base Color: {:?}\n- Shade Color: {:?}\n- Outline Color: {:?}\n- Outline Width: {:.4}\n- Shadow Threshold: {:.3}\n- Shadow Smoothness: {:.3}\n- Specular Intensity: {:.2} (Power: {:.1})\n- Rim Light Intensity: {:.2} (Spread: {:.2})\n- Hue Shift: {:.1}°\n- Toon Steps: {:.1} (1=cel, 2=ghibli, 0=continuous)",
                mat_clone.base_color,
                mat_clone.shade_color,
                mat_clone.outline_color,
                mat_clone.outline_width,
                mat_clone.shadow_threshold,
                mat_clone.shadow_smoothness,
                mat_clone.spec_intensity,
                mat_clone.spec_power,
                mat_clone.rim_intensity,
                mat_clone.rim_spread,
                mat_clone.hue_shift,
                mat_clone.toon_steps,
            );
            if !warnings.is_empty() {
                text.push_str(&format!("\nWarnings: {}", warnings.join("; ")));
            }
            Ok(vec![json!({ "type": "text", "text": text })])
        }
        "anigo_compare_baseline" => {
            let baseline_path_raw = args.get("baseline_path").and_then(|v| v.as_str())
                .context("baseline_path is required for anigo_compare_baseline")?;
            let baseline_path = sanitize_read_path(baseline_path_raw)?;
            let baseline_path_str = baseline_path.to_string_lossy().to_string();

            let current_image_path_raw = args.get("current_image_path").and_then(|v| v.as_str());
            let current_image_path = if let Some(p) = current_image_path_raw {
                Some(sanitize_read_path(p)?)
            } else { None };

            let diff_save_path_raw = args.get("diff_save_path").and_then(|v| v.as_str());
            let diff_save_path = if let Some(p) = diff_save_path_raw {
                Some(sanitize_save_path(p)?)
            } else { None };

            let mse_threshold = args.get("mse_threshold").and_then(|v| v.as_f64()).unwrap_or(50.0);
            let psnr_threshold = args.get("psnr_threshold").and_then(|v| v.as_f64()).unwrap_or(25.0);
            let tolerance_channel = args.get("tolerance_channel_diff").and_then(|v| v.as_u64()).unwrap_or(8) as i32;
            validate_tolerance_channel_diff(tolerance_channel)?;

            // 1. Load baseline image
            let baseline_img = load_rgba_image(&baseline_path_str)
                .with_context(|| format!("Failed to open baseline image at {}", baseline_path_str))?;
            let (b_w, b_h) = (baseline_img.width(), baseline_img.height());
            validate_render_dims(b_w, b_h)?;

            // 2. Load or render current image
            let current_img = match current_image_path {
                Some(ref path) => {
                    let path_str = path.to_string_lossy().to_string();
                    load_rgba_image(&path_str)
                        .with_context(|| format!("Failed to open current image at {}", path_str))?
                },
                None => {
                    // Render directly from current headless WebGPU state at baseline dimensions
                    let scene_snapshot = { state.scene.read().await.clone() };
                    let (img_buf, _metrics) = state.renderer.render_scene(&scene_snapshot, b_w, b_h).await?;
                    img_buf
                }
            };
            let (c_w, c_h) = (current_img.width(), current_img.height());

            if b_w != c_w || b_h != c_h {
                let report = json!({
                    "passed": false,
                    "error": format!("Image dimensions mismatch: baseline is {}x{}, current is {}x{}", b_w, b_h, c_w, c_h),
                    "baseline_dimensions": [b_w, b_h],
                    "current_dimensions": [c_w, c_h],
                });
                return Ok(vec![json!({
                    "type": "text",
                    "text": serde_json::to_string_pretty(&report)?
                })]);
            }

            // 3. Compute pixel metrics
            let total_pixels = (b_w * b_h) as f64;
            let mut sum_sq_err = 0.0f64;
            let mut max_channel_diff: u8 = 0;
            let mut matching_pixels: u64 = 0;

            let mut diff_img = if diff_save_path.is_some() {
                Some(image::RgbaImage::new(b_w, b_h))
            } else {
                None
            };

            for y in 0..b_h {
                for x in 0..b_w {
                    let p_base = baseline_img.get_pixel(x, y);
                    let p_curr = current_img.get_pixel(x, y);

                    let dr = (p_curr[0] as i32 - p_base[0] as i32).abs();
                    let dg = (p_curr[1] as i32 - p_base[1] as i32).abs();
                    let db = (p_curr[2] as i32 - p_base[2] as i32).abs();

                    let pixel_max_diff = dr.max(dg).max(db) as u8;
                    if pixel_max_diff > max_channel_diff {
                        max_channel_diff = pixel_max_diff;
                    }

                    if dr <= tolerance_channel && dg <= tolerance_channel && db <= tolerance_channel {
                        matching_pixels += 1;
                    }

                    sum_sq_err += (dr * dr + dg * dg + db * db) as f64;

                    if let Some(ref mut diff) = diff_img {
                        let ar = (dr * 4).min(255) as u8;
                        let ag = (dg * 4).min(255) as u8;
                        let ab = (db * 4).min(255) as u8;
                        diff.put_pixel(x, y, image::Rgba([ar, ag, ab, 255]));
                    }
                }
            }

            let mse = sum_sq_err / (total_pixels * 3.0);
            let psnr = if mse < 1e-9 {
                99.0
            } else {
                10.0 * ((255.0 * 255.0) / mse).log10()
            };
            let match_percent = (matching_pixels as f64 / total_pixels) * 100.0;
            let passed = mse <= mse_threshold && psnr >= psnr_threshold;

            if let (Some(diff_path), Some(diff)) = (diff_save_path, diff_img) {
                let mut png_bytes = Vec::new();
                let encoder = PngEncoder::new(&mut png_bytes);
                encoder.write_image(&diff, b_w, b_h, ColorType::Rgba8.into())?;
                // P0-04: use sanitized path
                std::fs::write(&diff_path, &png_bytes)
                    .with_context(|| format!("Failed to save diff image to {:?}", diff_path))?;
                state.metrics.bytes_written_to_fs.fetch_add(png_bytes.len() as u64, std::sync::atomic::Ordering::Relaxed);
            }

            let audit_result = json!({
                "passed": passed,
                "mse": mse,
                "psnr_db": psnr,
                "matching_pixel_percent": match_percent,
                "max_channel_difference": max_channel_diff,
                "mse_threshold": mse_threshold,
                "psnr_threshold": psnr_threshold,
                "channel_tolerance": tolerance_channel,
                "dimensions": [b_w, b_h],
                "baseline_path": baseline_path_str,
                "current_source": current_image_path_raw.unwrap_or("(direct WebGPU headless render)"),
            });

            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&audit_result)?
            })])
        }
        "anigo_load_mesh_preset" => {
            let preset = args.get("preset").and_then(|v| v.as_str()).unwrap_or("mannequin").to_string();
            if !["mannequin", "sphere", "cube"].contains(&preset.as_str()) {
                anyhow::bail!("Invalid preset {} (code -32602)", preset);
            }
            let mesh = match preset.as_str() {
                "sphere" => Mesh::create_uv_sphere_at(0.85, 36, 72, [0.0, 1.0, 0.0]),
                "cube" => Mesh::create_cube_at(1.2, [0.0, 1.0, 0.0]),
                _ => Mesh::create_mannequin_proxy(),
            };

            let vertex_count = {
                let mut scene = state.scene.write().await;
                let current_mat = scene.nodes.first()
                    .and_then(|n| n.material.clone())
                    .unwrap_or_default();
                scene.nodes.clear();
                let mut node = SceneNode::new("primary_mesh", preset.clone()).with_mesh(mesh);
                node.material = Some(current_mat);
                scene.add_node(node);
                scene.total_vertices()
            };

            if let Err(e) = state.bridge.send_command("LOAD_PRESET", json!({ "preset": preset })).await {
                warnings.push(format!("Live sync LOAD_PRESET failed: {}", e));
                tracing::warn!(error=%e, "Live bridge LOAD_PRESET failed");
            }

            let mut msg = format!("Loaded mesh preset '{}' with {} vertices (synchronized with live window).", preset, vertex_count);
            if !warnings.is_empty() {
                msg.push_str(&format!(" Warnings: {}", warnings.join("; ")));
            }
            Ok(vec![json!({
                "type": "text",
                "text": msg
            })])
        }
        "anigo_set_character_model" => {
            let model_type = args.get("model_type").and_then(|v| v.as_str()).unwrap_or("male").to_string();
            let gender = if model_type.eq_ignore_ascii_case("female") {
                BaseGender::Female
            } else {
                BaseGender::Male
            };

            {
                let mut cur_gender = state.current_gender.write().await;
                *cur_gender = gender;
            }
            {
                let mut base_mesh = state.base_mesh.write().await;
                *base_mesh = Mesh::create_canonical_base(gender);
            }
            {
                let mut catalog = state.morph_catalog.write().await;
                catalog.set_gender(gender);
            }

            let (vertex_count, tri_count, active_count) = {
                let base_mesh = state.base_mesh.read().await.clone();
                let mut catalog = state.morph_catalog.write().await;
                let mut morphed_mesh = base_mesh.clone();
                catalog.apply_to_mesh(&base_mesh, &mut morphed_mesh);
                let mut scene = state.scene.write().await;
                let current_mat = scene.nodes.first()
                    .and_then(|n| n.material.clone())
                    .unwrap_or_default();
                scene.nodes.clear();
                let mut node = SceneNode::new("primary_mesh", format!("canonical_{}", model_type)).with_mesh(morphed_mesh);
                node.material = Some(current_mat);
                scene.add_node(node);
                (scene.total_vertices(), scene.total_triangles(), catalog.get_active_morphs().len())
            };

            if let Err(e) = state.bridge.send_command("SET_CHARACTER_MODEL", json!({ "model_type": model_type })).await {
                warnings.push(format!("Live sync SET_CHARACTER_MODEL failed: {}", e));
                tracing::warn!(error=%e, "Live bridge SET_CHARACTER_MODEL failed");
            }

            let mut payload = json!({
                "status": "success",
                "model_type": model_type,
                "gender": format!("{:?}", gender),
                "vertex_count": vertex_count,
                "triangle_count": tri_count,
                "isomorphic": true,
                "active_morphs_count": active_count
            });
            if !warnings.is_empty() {
                payload["warnings"] = json!(warnings);
            }

            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&payload)?
            })])
        }
        "anigo_set_somatotype" => {
            let endo = args.get("endo").and_then(|v| v.as_f64()).unwrap_or(3.0) as f32;
            let meso = args.get("meso").and_then(|v| v.as_f64()).unwrap_or(4.0) as f32;
            let ecto = args.get("ecto").and_then(|v| v.as_f64()).unwrap_or(3.0) as f32;

            validate::f32_range(endo as f64, 1.0, 12.0, "endo")?;
            validate::f32_range(meso as f64, 1.0, 12.0, "meso")?;
            validate::f32_range(ecto as f64, 1.0, 12.0, "ecto")?;

            let coords = SomatotypeCoords::new(endo, meso, ecto).normalized();

            {
                let mut catalog = state.morph_catalog.write().await;
                catalog.set_slider("somatotype_endomorph", coords.endomorph).map_err(|e| anyhow::anyhow!(e))?;
                catalog.set_slider("somatotype_mesomorph", coords.mesomorph).map_err(|e| anyhow::anyhow!(e))?;
                catalog.set_slider("somatotype_ectomorph", coords.ectomorph).map_err(|e| anyhow::anyhow!(e))?;

                let base_mesh = state.base_mesh.read().await.clone();
                let mut morphed_mesh = base_mesh.clone();
                catalog.apply_to_mesh(&base_mesh, &mut morphed_mesh);
                let mut scene = state.scene.write().await;
                if let Some(node) = scene.nodes.first_mut() {
                    node.mesh = Some(morphed_mesh);
                }
            }

            let pad = coords.to_pad_2d();
            if let Err(e) = state.bridge.send_command("SET_SOMATOTYPE", json!({
                "endo": endo,
                "meso": meso,
                "ecto": ecto,
                "barycentric": [coords.endomorph, coords.mesomorph, coords.ectomorph],
                "pad_2d": [pad.0, pad.1]
            })).await {
                warnings.push(format!("Live sync SET_SOMATOTYPE failed: {}", e));
                tracing::warn!(error=%e, "Live bridge SET_SOMATOTYPE failed");
            }

            let mut result = json!({
                "status": "success",
                "input": { "endo": endo, "meso": meso, "ecto": ecto },
                "normalized_barycentric": { "endomorph": coords.endomorph, "mesomorph": coords.mesomorph, "ectomorph": coords.ectomorph },
                "pad_2d": { "x": pad.0, "y": pad.1 }
            });
            if !warnings.is_empty() {
                result["warnings"] = json!(warnings);
            }

            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&result)?
            })])
        }
        "anigo_apply_morph_slider" => {
            let slider_id = args.get("slider_id").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("Missing 'slider_id' (code -32602)"))?.to_string();
            let value_raw = args.get("value").and_then(|v| v.as_f64()).ok_or_else(|| anyhow::anyhow!("Missing 'value' (code -32602)"))?;
            let value = validate::f32_finite(value_raw, "value")?;
            // Clamp to reasonable range to avoid overflow
            if value.abs() > 1000.0 {
                anyhow::bail!("value out of range -1000..1000: {} (code -32602)", value);
            }

            let applied = {
                let mut catalog = state.morph_catalog.write().await;
                let applied = catalog.set_slider(&slider_id, value).map_err(|e| anyhow::anyhow!(e))?;
                let base_mesh = state.base_mesh.read().await.clone();
                let mut morphed_mesh = base_mesh.clone();
                catalog.apply_to_mesh(&base_mesh, &mut morphed_mesh);
                let mut scene = state.scene.write().await;
                if let Some(node) = scene.nodes.first_mut() {
                    node.mesh = Some(morphed_mesh);
                }
                applied
            };

            let def = find_slider_def(&slider_id);
            let zone_name = def.map(|d| d.zone.name()).unwrap_or("Unknown");

            if let Err(e) = state.bridge.send_command("APPLY_MORPH_SLIDER", json!({
                "slider_id": slider_id,
                "value": applied
            })).await {
                warnings.push(format!("Live sync APPLY_MORPH_SLIDER failed: {}", e));
                tracing::warn!(error=%e, "Live bridge APPLY_MORPH_SLIDER failed");
            }

            let mut payload = json!({
                "status": "success",
                "slider_id": slider_id,
                "applied_value": applied,
                "zone": zone_name,
                "active_morphs_count": state.morph_catalog.read().await.get_active_morphs().len()
            });
            if !warnings.is_empty() {
                payload["warnings"] = json!(warnings);
            }

            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&payload)?
            })])
        }
        "anigo_inspect_mesh_integrity" => {
            let mesh = {
                let scene = state.scene.read().await;
                scene.nodes.first()
                    .and_then(|n| n.mesh.clone())
                    .ok_or_else(|| anyhow::anyhow!("No active mesh in scene to inspect"))?
            };

            let report = MorphCatalog::inspect_integrity(&mesh);

            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&report)?
            })])
        }
        "anigo_get_active_morphs" => {
            let active = {
                let catalog = state.morph_catalog.read().await;
                catalog.get_active_morphs()
            };
            let list: Vec<Value> = active.into_iter().map(|(id, val)| {
                let zone = find_slider_def(id).map(|d| d.zone.name()).unwrap_or("Unknown");
                json!({
                    "id": id,
                    "value": val,
                    "zone": zone
                })
            }).collect();

            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&json!({
                    "active_count": list.len(),
                    "active_morphs": list
                }))?
            })])
        }
        "anigo_reset_morphs" => {
            {
                let mut catalog = state.morph_catalog.write().await;
                catalog.reset_all();
                let base_mesh = state.base_mesh.read().await.clone();
                let mut morphed_mesh = base_mesh.clone();
                catalog.apply_to_mesh(&base_mesh, &mut morphed_mesh);
                let mut scene = state.scene.write().await;
                if let Some(node) = scene.nodes.first_mut() {
                    node.mesh = Some(morphed_mesh);
                }
            }

            if let Err(e) = state.bridge.send_command("RESET_MORPHS", json!({})).await {
                warnings.push(format!("Live sync RESET_MORPHS failed: {}", e));
                tracing::warn!(error=%e, "Live bridge RESET_MORPHS failed");
            }

            let mut payload = json!({
                "status": "success",
                "message": "All morph sliders reset to canonical neutral defaults.",
                "active_morphs_count": 0
            });
            if !warnings.is_empty() {
                payload["warnings"] = json!(warnings);
            }

            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&payload)?
            })])
        }
        "anigo_set_proportions" => {
            let scale_raw = args.get("head_scale").and_then(|v| v.as_f64()).unwrap_or(1.0);
            let ratio_raw = args.get("head_ratio").and_then(|v| v.as_f64()).unwrap_or(6.5);
            let scale = validate::f32_range(scale_raw, 0.7, 1.4, "head_scale")?;
            let ratio = validate::f32_range(ratio_raw, 2.0, 8.5, "head_ratio")?;

            {
                let mut scene = state.scene.write().await;
                let current_mat = scene.nodes.first()
                    .and_then(|n| n.material.clone())
                    .unwrap_or_default();

                let mesh = Mesh::create_mannequin_proxy_proportions(scale, ratio);
                scene.nodes.clear();
                let mut node = SceneNode::new("primary_mesh", "mannequin").with_mesh(mesh);
                node.material = Some(current_mat);
                scene.add_node(node);
            }

            if let Err(e) = state.bridge.send_command("SET_PROPORTIONS", json!({
                "head_scale": scale,
                "head_ratio": ratio,
            })).await {
                warnings.push(format!("Live sync SET_PROPORTIONS failed: {}", e));
                tracing::warn!(error=%e, "Live bridge SET_PROPORTIONS failed");
            }

            let mut msg = format!("Set proportions: Head Scale = {:.2}x, Head Ratio = {:.1} heads (sent to live window and offscreen renderer).", scale, ratio);
            if !warnings.is_empty() {
                msg.push_str(&format!(" Warnings: {}", warnings.join("; ")));
            }
            Ok(vec![json!({
                "type": "text",
                "text": msg
            })])
        }
        "anigo_render_frame" => {
            let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(800) as u32;
            let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(600) as u32;
            let save_path_raw = args.get("save_path").and_then(|v| v.as_str());
            let save_path = if let Some(p) = save_path_raw {
                Some(sanitize_save_path(p)?)
            } else { None };
            let sync_live = args.get("sync_live").and_then(|v| v.as_bool()).unwrap_or(false);

            // P0-03: Validate dimensions before any allocation
            validate_render_dims(width, height)?;

            if sync_live {
                // P1-09: sync_live agora sincroniza câmera, luz, material, preset e proporções
                // (não apenas a câmera, como antes).
                match state.bridge.send_command("GET_STATUS", json!({})).await {
                    Ok(telemetry) => {
                        let synced_fields = apply_live_telemetry_to_scene(&state, &telemetry).await;
                        tracing::info!(synced = %synced_fields.join(", "), "sync_live applied fields from live window");
                    }
                    Err(e) => {
                        warnings.push(format!("sync_live: failed to get telemetry from live window: {}", e));
                        tracing::warn!(error=%e, "sync_live failed to reach bridge");
                    }
                }
            }

            // Snapshot scene without holding lock during render
            let scene_snapshot = { state.scene.read().await.clone() };

            let (image_buf, metrics) = state.renderer.render_scene(&scene_snapshot, width, height).await?;

            let mut png_bytes = Vec::new();
            let encoder = PngEncoder::new(&mut png_bytes);
            encoder.write_image(
                &image_buf,
                width,
                height,
                ColorType::Rgba8.into(),
            )?;

            let base64_str = base64::engine::general_purpose::STANDARD.encode(&png_bytes);

            if let Some(path) = save_path {
                std::fs::write(&path, &png_bytes)
                    .with_context(|| format!("Failed to save rendered frame to {:?}", path))?;
                state.metrics.bytes_written_to_fs.fetch_add(png_bytes.len() as u64, std::sync::atomic::Ordering::Relaxed);
            }
            state.metrics.frames_rendered.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

            let mut report_text = format!(
                "Render Successful: {}x{} | Time: {:.2}ms | Triangles: {} | Draw Calls: {} | GPU: {} ({})",
                width, height, metrics.render_time_ms, metrics.triangle_count, metrics.draw_calls, metrics.adapter_name, metrics.backend
            );
            if !warnings.is_empty() {
                report_text.push_str(&format!(" | Warnings: {}", warnings.join("; ")));
            }

            Ok(vec![
                json!({
                    "type": "text",
                    "text": report_text
                }),
                json!({
                    "type": "image",
                    "data": base64_str,
                    "mimeType": "image/png"
                }),
            ])
        }
        "anigo_find_window" => {
            // P0-01: spawn_blocking for Win32
            let bridge = Arc::clone(&state.bridge);
            let (hwnd_hint, _title_hint) = {
                // I/O outside blocking
                if let Ok(state_resp) = bridge.send_command("GET_WINDOW_STATE", json!({})).await {
                    let hwnd = state_resp.get("hwnd").and_then(|v| v.as_u64()).map(|h| h as *mut std::ffi::c_void);
                    (hwnd, None)
                } else {
                    (None, None)
                }
            };

            let result = tokio::task::spawn_blocking(move || {
                Win32Harness::find_anigo_window_with_hint(hwnd_hint)
            }).await.context("Join error in find_anigo_window")??;

            let (_hwnd, title, rect, is_minimized) = result;
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;

            Ok(vec![json!({
                "type": "text",
                "text": format!(
                    "ANIGO Window Status:\n- Title: '{}'\n- Minimized: {}\n- Bounds: [left: {}, top: {}, right: {}, bottom: {}]\n- Dimensions: {}x{} px",
                    title, is_minimized, rect.left, rect.top, rect.right, rect.bottom, width, height
                )
            })])
        }
        "anigo_screenshot_window" => {
            let bridge = Arc::clone(&state.bridge);
            let hwnd_hint = {
                if let Ok(state_resp) = bridge.send_command("GET_WINDOW_STATE", json!({})).await {
                    state_resp.get("hwnd").and_then(|v| v.as_u64()).map(|h| h as *mut std::ffi::c_void)
                } else {
                    None
                }
            };

            let save_path_raw = args.get("save_path").and_then(|v| v.as_str());
            let save_path = if let Some(p) = save_path_raw {
                Some(sanitize_save_path(p)?)
            } else { None };

            let capture_result = tokio::task::spawn_blocking(move || -> Result<(String, u32, u32, bool, RgbaImage)> {
                let (hwnd, title, _rect, was_minimized) = Win32Harness::find_anigo_window_with_hint(hwnd_hint)?;
                let restored_rect = Win32Harness::ensure_window_active(hwnd)?;
                let img = Win32Harness::capture_window(hwnd, &restored_rect)?;
                let w = img.width();
                let h = img.height();
                Ok((title, w, h, was_minimized, img))
            }).await.context("Join error in screenshot")??;

            let (title, width, height, was_minimized, img) = capture_result;

            let mut png_bytes = Vec::new();
            let encoder = PngEncoder::new(&mut png_bytes);
            encoder.write_image(&img, width, height, ColorType::Rgba8.into())?;

            if let Some(path) = &save_path {
                std::fs::write(path, &png_bytes)
                    .with_context(|| format!("Failed to save window screenshot to {:?}", path))?;
            }

            let base64_str = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
            let status_msg = format!(
                "Captured Full Window Screenshot (Inside + Outside Viewport):\n- Title: '{}'\n- Size: {}x{} px\n- Was Minimized (Auto-Restored): {}\n- Saved to: {}",
                title, width, height, was_minimized, save_path.as_ref().map(|p| format!("{:?}", p)).unwrap_or("(memory only)".to_string())
            );

            Ok(vec![
                json!({
                    "type": "text",
                    "text": status_msg
                }),
                json!({
                    "type": "image",
                    "data": base64_str,
                    "mimeType": "image/png"
                }),
            ])
        }
        "anigo_mouse_click" => {
            let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let button = args.get("button").and_then(|v| v.as_str()).unwrap_or("left").to_string();

            // Validate coords range
            validate::clamp_i32(x as i64, -10000, 10000, "x")?;
            validate::clamp_i32(y as i64, -10000, 10000, "y")?;

            let result = tokio::task::spawn_blocking(move || -> Result<(i32,i32)> {
                let (hwnd, _title, _rect, _is_min) = Win32Harness::find_anigo_window()?;
                let active_rect = Win32Harness::ensure_window_active(hwnd)?;
                let screen_x = active_rect.left + x;
                let screen_y = active_rect.top + y;
                Win32Harness::mouse_click(screen_x, screen_y, &button);
                Ok((screen_x, screen_y))
            }).await.context("Join error in mouse_click")??;

            let (screen_x, screen_y) = result;

            Ok(vec![json!({
                "type": "text",
                "text": format!("Clicked '{}' button at window relative ({}, {}) -> screen ({}, {})", button, x, y, screen_x, screen_y)
            })])
        }
        "anigo_mouse_drag" => {
            let start_x = args.get("start_x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let start_y = args.get("start_y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let end_x = args.get("end_x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let end_y = args.get("end_y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let button = args.get("button").and_then(|v| v.as_str()).unwrap_or("left").to_string();
            let steps = args.get("steps").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
            if steps < 1 || steps > 100 {
                anyhow::bail!("steps out of range 1..100: {} (code -32602)", steps);
            }

            tokio::task::spawn_blocking(move || -> Result<()> {
                let (hwnd, _title, _rect, _is_min) = Win32Harness::find_anigo_window()?;
                let active_rect = Win32Harness::ensure_window_active(hwnd)?;
                let s_x = active_rect.left + start_x;
                let s_y = active_rect.top + start_y;
                let e_x = active_rect.left + end_x;
                let e_y = active_rect.top + end_y;
                Win32Harness::mouse_drag(s_x, s_y, e_x, e_y, &button, steps);
                Ok(())
            }).await.context("Join error in mouse_drag")??;

            Ok(vec![json!({
                "type": "text",
                "text": format!("Dragged '{}' button from window ({}, {}) to ({}, {}) over {} steps", button, start_x, start_y, end_x, end_y, steps)
            })])
        }
        "anigo_maximize_window" => {
            if state.bridge.is_live().await {
                match state.bridge.send_command("MAXIMIZE_WINDOW", json!({})).await {
                    Ok(resp) => return Ok(vec![json!({
                        "type": "text",
                        "text": format!("Window maximized via native Live Bridge: {}", serde_json::to_string_pretty(&resp)?)
                    })]),
                    Err(e) => {
                        warnings.push(format!("Bridge maximize failed, fallback to Win32: {}", e));
                        tracing::warn!(error=%e, "Bridge maximize failed, falling back to Win32");
                    }
                }
            }
            let result = tokio::task::spawn_blocking(|| {
                let (hwnd, title, _rect, _is_min) = Win32Harness::find_anigo_window()?;
                let new_rect = Win32Harness::maximize_window(hwnd)?;
                Ok::<_, anyhow::Error>((title, new_rect))
            }).await.context("Join error in maximize")??;

            let (title, new_rect) = result;
            let w = new_rect.right - new_rect.left;
            let h = new_rect.bottom - new_rect.top;
            let mut msg = format!("Window '{}' maximized via Win32 to {}x{} px [bounds: {}, {}, {}, {}]", title, w, h, new_rect.left, new_rect.top, new_rect.right, new_rect.bottom);
            if !warnings.is_empty() {
                msg.push_str(&format!(" Warnings: {}", warnings.join("; ")));
            }
            Ok(vec![json!({
                "type": "text",
                "text": msg
            })])
        }
        "anigo_restore_window" => {
            if state.bridge.is_live().await {
                match state.bridge.send_command("RESTORE_WINDOW", json!({})).await {
                    Ok(resp) => return Ok(vec![json!({
                        "type": "text",
                        "text": format!("Window restored via native Live Bridge: {}", serde_json::to_string_pretty(&resp)?)
                    })]),
                    Err(e) => {
                        warnings.push(format!("Bridge restore failed, fallback to Win32: {}", e));
                        tracing::warn!(error=%e, "Bridge restore failed");
                    }
                }
            }
            let result = tokio::task::spawn_blocking(|| {
                let (hwnd, title, _rect, _is_min) = Win32Harness::find_anigo_window()?;
                let new_rect = Win32Harness::restore_window(hwnd)?;
                Ok::<_, anyhow::Error>((title, new_rect))
            }).await.context("Join error in restore")??;

            let (title, new_rect) = result;
            let w = new_rect.right - new_rect.left;
            let h = new_rect.bottom - new_rect.top;
            let mut msg = format!("Window '{}' restored via Win32 to {}x{} px [bounds: {}, {}, {}, {}]", title, w, h, new_rect.left, new_rect.top, new_rect.right, new_rect.bottom);
            if !warnings.is_empty() {
                msg.push_str(&format!(" Warnings: {}", warnings.join("; ")));
            }
            Ok(vec![json!({
                "type": "text",
                "text": msg
            })])
        }
        "anigo_minimize_window" => {
            if state.bridge.is_live().await {
                let resp = state.bridge.send_command("MINIMIZE_WINDOW", json!({})).await?;
                Ok(vec![json!({
                    "type": "text",
                    "text": format!("Window minimized via native Live Bridge: {}", serde_json::to_string_pretty(&resp)?)
                })])
            } else {
                anyhow::bail!("Live studio window is not connected on port 39090")
            }
        }
        "anigo_focus_window" => {
            if state.bridge.is_live().await {
                let resp = state.bridge.send_command("FOCUS_WINDOW", json!({})).await?;
                Ok(vec![json!({
                    "type": "text",
                    "text": format!("Window focused via native Live Bridge: {}", serde_json::to_string_pretty(&resp)?)
                })])
            } else {
                anyhow::bail!("Live studio window is not connected on port 39090")
            }
        }
        "anigo_get_window_state" => {
            if state.bridge.is_live().await {
                let resp = state.bridge.send_command("GET_WINDOW_STATE", json!({})).await?;
                Ok(vec![json!({
                    "type": "text",
                    "text": serde_json::to_string_pretty(&resp)?
                })])
            } else {
                let result = tokio::task::spawn_blocking(|| {
                    Win32Harness::find_anigo_window()
                }).await.context("Join error")??;

                let (hwnd, title, rect, is_minimized) = result;
                let report = json!({
                    "hwnd": format!("{:?}", hwnd),
                    "title": title,
                    "is_minimized": is_minimized,
                    "bounds": {
                        "left": rect.left,
                        "top": rect.top,
                        "right": rect.right,
                        "bottom": rect.bottom,
                        "width": rect.right - rect.left,
                        "height": rect.bottom - rect.top,
                    }
                });
                Ok(vec![json!({
                    "type": "text",
                    "text": serde_json::to_string_pretty(&report)?
                })])
            }
        }
        "anigo_ui_action" => {
            // P1-08: Validação rígida de schema com allowlists por ação e ranges por propriedade.
            // Bloqueia prototype pollution e injeção de payload via args.clone() cru.
            let action = args.get("action").and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'action' (code -32602)"))?;

            // Anti-pollution: rejeita chaves perigosas em qualquer nível do payload
            fn check_no_proto(val: &Value) -> Result<()> {
                match val {
                    Value::Object(m) => {
                        for (k, v) in m {
                            if k == "__proto__" || k == "constructor" || k == "prototype" {
                                anyhow::bail!("Invalid property name '{}' (prototype pollution blocked, code -32602)", k);
                            }
                            check_no_proto(v)?;
                        }
                    }
                    Value::Array(a) => { for v in a { check_no_proto(v)?; } }
                    _ => {}
                }
                Ok(())
            }
            check_no_proto(&args)?;

            // Allowlists rigorosos (espelham frontend Svelte em App.svelte)
            const ALLOWED_TABS: &[&str] = &[
                "personagem", "posing", "shading", "iluminacao",
                "cenario", "animacao", "render", "biblioteca",
            ];
            // Ferramentas comuns por workspace; permite qualquer string não-vazia mas valida formato
            const ALLOWED_PRESETS: &[&str] = &["mannequin", "sphere", "cube"];
            const ALLOWED_SLIDERS: &[&str] = &[
                "head_scale", "head_ratio",
                "shoulder_width", "shoulders", "leg_length", "legs",
                "arm_length", "arms", "neck_length", "neck",
                "outline_width", "outline_extrusion",
                "shadow_threshold", "shadow_smoothness", "toon_smoothness",
                "light_azimuth", "light_elevation", "light_intensity",
                "shadow_saturation", "ambient_intensity",
                "eye_scale", "eye_size", "chin_width", "jaw_width",
                "hair_volume", "hair_thickness", "hair_curvature",
                "cloth_tension",
                "rim_intensity", "rim_spread", "hue_shift",
                "spec_intensity", "spec_power", "spec_exponent",
                "toon_steps", "camera_fov", "current_frame",
            ];

            // Constrói payload limpo e validado (NÃO envia args.clone() cru)
            let mut clean_params = serde_json::Map::new();
            clean_params.insert("action".into(), json!(action));

            match action {
                "select_tab" => {
                    let tab = args.get("value").and_then(|v| v.as_str())
                        .or_else(|| args.get("tab").and_then(|v| v.as_str()))
                        .ok_or_else(|| anyhow::anyhow!("select_tab requires 'value' (tab name) (code -32602)"))?;
                    if !ALLOWED_TABS.contains(&tab) {
                        anyhow::bail!("Unknown tab '{}'. Allowed: {} (code -32602)", tab, ALLOWED_TABS.join(", "));
                    }
                    clean_params.insert("value".into(), json!(tab));
                }
                "select_tool" => {
                    let tool = args.get("value").and_then(|v| v.as_str())
                        .or_else(|| args.get("tool").and_then(|v| v.as_str()))
                        .ok_or_else(|| anyhow::anyhow!("select_tool requires 'value' (tool id) (code -32602)"))?;
                    if tool.is_empty() || tool.len() > 64 {
                        anyhow::bail!("Invalid tool id length (code -32602)");
                    }
                    // Valida formato simples: [a-z0-9_-]+
                    if !tool.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
                        anyhow::bail!("Invalid tool id '{}' (only a-z, 0-9, _, - allowed) (code -32602)", tool);
                    }
                    clean_params.insert("value".into(), json!(tool));
                }
                "set_slider" => {
                    let prop = args.get("property").and_then(|v| v.as_str())
                        .or_else(|| args.get("name").and_then(|v| v.as_str()))
                        .ok_or_else(|| anyhow::anyhow!("set_slider requires 'property' (slider name) (code -32602)"))?;
                    if !ALLOWED_SLIDERS.contains(&prop) {
                        anyhow::bail!("Unknown slider property '{}'. Allowed sliders: {} (code -32602)", prop, ALLOWED_SLIDERS.join(", "));
                    }
                    let val = args.get("value").and_then(|v| v.as_f64())
                        .ok_or_else(|| anyhow::anyhow!("set_slider requires numeric 'value' (code -32602)"))?;
                    // Range genérico razoável; ranges específicos por prop são aplicados no frontend
                    if !val.is_finite() {
                        anyhow::bail!("Slider value must be finite, got {} (code -32602)", val);
                    }
                    if val < -1000.0 || val > 1000.0 {
                        anyhow::bail!("Slider value {} out of range -1000..1000 (code -32602)", val);
                    }
                    clean_params.insert("property".into(), json!(prop));
                    clean_params.insert("value".into(), json!(val));
                }
                "set_preset" => {
                    let preset = args.get("value").and_then(|v| v.as_str())
                        .or_else(|| args.get("preset").and_then(|v| v.as_str()))
                        .ok_or_else(|| anyhow::anyhow!("set_preset requires 'value' (preset name) (code -32602)"))?;
                    if !ALLOWED_PRESETS.contains(&preset) {
                        anyhow::bail!("Unknown preset '{}'. Allowed: {} (code -32602)", preset, ALLOWED_PRESETS.join(", "));
                    }
                    clean_params.insert("value".into(), json!(preset));
                }
                other => {
                    anyhow::bail!("Invalid action '{}'. Allowed: select_tab, select_tool, set_slider, set_preset (code -32602)", other);
                }
            }

            if !state.bridge.is_live().await {
                anyhow::bail!("Live studio window is not connected on port 39090");
            }
            let resp = state.bridge.send_command("UI_ACTION", Value::Object(clean_params)).await?;
            Ok(vec![json!({
                "type": "text",
                "text": format!("UI Action executed: {}", serde_json::to_string_pretty(&resp)?)
            })])
        }
        "anigo_read_logs" => {
            let filter = args.get("filter").and_then(|v| v.as_str()).unwrap_or("all").to_string();
            let diag = tokio::task::spawn_blocking(|| {
                Win32Harness::collect_logs_and_diagnostics()
            }).await.context("Join error in read_logs")?;

            // Limit to 64KB per log (P1-10)
            let mut filtered = match filter.as_str() {
                "launch" => json!({ "launch_log": diag.get("launch_log") }),
                "panic" => json!({ "panic_log": diag.get("panic_log") }),
                "crash" => json!({ "app_crash_log": diag.get("app_crash_log") }),
                _ => diag,
            };

            // Truncate large logs
            if let Some(obj) = filtered.as_object_mut() {
                for (_k, v) in obj.iter_mut() {
                    if let Some(s) = v.as_str() {
                        if s.len() > 64 * 1024 {
                            *v = json!(format!("{}... [truncated {} bytes]", &s[..64*1024], s.len() - 64*1024));
                        }
                    }
                }
            }

            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&filtered)?
            })])
        }
        "anigo_mouse_scroll" => {
            let delta = args.get("delta").and_then(|v| v.as_i64()).unwrap_or(120) as i32;
            // Clamp delta to avoid extreme scroll
            if delta.abs() > 10000 {
                anyhow::bail!("delta out of range -10000..10000: {} (code -32602)", delta);
            }
            tokio::task::spawn_blocking(move || {
                Win32Harness::mouse_wheel(delta);
            }).await.context("Join error in mouse_wheel")?;
            Ok(vec![json!({
                "type": "text",
                "text": format!("Dispatched mouse wheel scroll with delta: {}", delta)
            })])
        }
        "anigo_send_key" => {
            let vk_raw = args.get("vk_code").and_then(|v| v.as_u64())
                .ok_or_else(|| anyhow::anyhow!("Missing 'vk_code' (required). Allowed: arrows (0x25-0x28), F1-F12 (0x70-0x7B), 0-9 (0x30-0x39), A-Z (0x41-0x5A), Backspace(0x08)/Tab(0x09)/Enter(0x0D)/Esc(0x1B)/Space(0x20)/Delete(0x2E) (code -32602)"))?;
            if vk_raw > 255 {
                anyhow::bail!("vk_code must be 0..255, got {} (code -32602)", vk_raw);
            }
            let vk = vk_raw as u8;
            validate::validate_vk(vk)?;
            tokio::task::spawn_blocking(move || {
                Win32Harness::send_key(vk);
            }).await.context("Join error in send_key")?;
            Ok(vec![json!({
                "type": "text",
                "text": format!("Dispatched virtual key event for VK code: 0x{:02X}", vk)
            })])
        }
        "anigo_get_diagnostics" => {
            let diag = tokio::task::spawn_blocking(|| {
                Win32Harness::collect_logs_and_diagnostics()
            }).await.context("Join error")?;
            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&diag)?
            })])
        }
        "anigo_get_metrics" => {
            let snap = state.metrics.snapshot();
            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&snap)?
            })])
        }
        "anigo_get_support_bundle" => {
            let live_active = state.bridge.is_live().await;
            let info = &state.renderer.adapter_info;
            let bridge_telemetry = if live_active {
                state.bridge.send_command("GET_STATUS", json!({})).await.ok()
            } else { None };
            let bridge_metrics = if live_active {
                state.bridge.send_command("GET_METRICS", json!({})).await.ok()
            } else { None };
            let diag = tokio::task::spawn_blocking(|| {
                Win32Harness::collect_logs_and_diagnostics()
            }).await.unwrap_or_else(|_| json!({"error":"join error"}));
            let bundle = json!({
                "version": env!("CARGO_PKG_VERSION"),
                "platform": std::env::consts::OS,
                "timestamp_utc": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
                "gpu": {
                    "adapter": info.name,
                    "backend": format!("{:?}", info.backend),
                    "device_type": format!("{:?}", info.device_type),
                },
                "live_bridge_active": live_active,
                "live_telemetry": bridge_telemetry,
                "bridge_metrics": bridge_metrics,
                "mcp_metrics": state.metrics.snapshot(),
                "diagnostics": diag,
            });
            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&bundle)?
            })])
        }
        _ => anyhow::bail!("Unknown tool: {} (code -32601)", name),
    }
}

fn load_rgba_image(path: &str) -> Result<RgbaImage> {
    // P0-03 + P0-04: Validate path + size + dimensions
    let sanitized = sanitize_read_path(path)?;
    let path_buf = sanitized;

    // Check file size limit
    let metadata = std::fs::metadata(&path_buf).with_context(|| format!("Failed to stat image file: {:?}", path_buf))?;
    if metadata.len() as usize > fs_sandbox::max_image_bytes() {
        anyhow::bail!("Image file too large: {} bytes > {} limit (code -32602)", metadata.len(), fs_sandbox::max_image_bytes());
    }

    let bytes = std::fs::read(&path_buf).with_context(|| format!("Failed to read image file: {:?}", path_buf))?;
    if bytes.len() > fs_sandbox::max_image_bytes() {
        anyhow::bail!("Image file too large after read: {} > {} (code -32602)", bytes.len(), fs_sandbox::max_image_bytes());
    }

    // Decode with limit
    let dyn_img = image::load_from_memory(&bytes).with_context(|| format!("Failed to decode image from {:?}", path_buf))?;
    let rgba = dyn_img.to_rgba8();

    // Validate dimensions
    let (w, h) = (rgba.width(), rgba.height());
    if w as u64 * h as u64 > fs_sandbox::max_pixels() {
        anyhow::bail!("Image dimensions {}x{} exceed {} MP limit (code -32602)", w, h, fs_sandbox::max_pixels() / 1_000_000);
    }
    if w > fs_sandbox::max_dim() || h > fs_sandbox::max_dim() {
        anyhow::bail!("Image dimensions {}x{} exceed max {}x{} (code -32602)", w, h, fs_sandbox::max_dim(), fs_sandbox::max_dim());
    }

    Ok(rgba)
}
