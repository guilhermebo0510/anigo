mod bridge_client;
mod win32_interact;

use std::io::{self, BufRead, Write};
use std::sync::Arc;
use anyhow::{Context, Result};
use base64::Engine;
use image::codecs::png::PngEncoder;
use image::{ColorType, ImageEncoder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;
use win32_interact::Win32Harness;

use anigo_core::mesh::{BaseGender, Mesh};
use anigo_core::morph_catalog::{find_slider_def, MorphCatalog};
use anigo_core::scene::{Scene, SceneNode};
use anigo_core::somatotype::SomatotypeCoords;
use anigo_renderer::HeadlessRenderer;
use bridge_client::LiveBridgeClient;

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

struct AppState {
    renderer: HeadlessRenderer,
    scene: Scene,
    bridge: LiveBridgeClient,
    current_gender: BaseGender,
    morph_catalog: MorphCatalog,
    base_mesh: Mesh,
}

#[tokio::main]
async fn main() -> Result<()> {
    eprintln!("[anigo-mcp] Initializing ANIGO MCP Server with WebGPU engine & Live Socket Bridge...");

    let renderer = HeadlessRenderer::new()
        .await
        .context("Failed to initialize headless WebGPU renderer in anigo-mcp")?;

    eprintln!(
        "[anigo-mcp] Connected to GPU: {} ({:?})",
        renderer.adapter_info.name, renderer.adapter_info.backend
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

    let state = Arc::new(Mutex::new(AppState {
        renderer,
        scene,
        bridge: LiveBridgeClient::new("127.0.0.1:39090"),
        current_gender: initial_gender,
        morph_catalog,
        base_mesh,
    }));

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

        let req_id = request.id.clone().unwrap_or(Value::Null);

        match request.method.as_str() {
            "initialize" => {
                let res = json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "serverInfo": {
                            "name": "anigo-mcp",
                            "version": "0.1.0"
                        },
                        "capabilities": {
                            "tools": {}
                        }
                    }
                });
                writeln!(stdout, "{}", serde_json::to_string(&res)?)?;
                stdout.flush()?;
            }
            "notifications/initialized" => {
                // MCP notification, no response required
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
                let tool_name = request.params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let arguments = request.params.get("arguments").cloned().unwrap_or(json!({}));

                let call_result = handle_tool_call(tool_name, arguments, Arc::clone(&state)).await;
                let res = match call_result {
                    Ok(val) => json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": {
                            "content": val
                        }
                    }),
                    Err(err) => json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": {
                            "isError": true,
                            "content": [{
                                "type": "text",
                                "text": format!("Tool execution failed: {:#}", err)
                            }]
                        }
                    }),
                };
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
    json!([
        {
            "name": "anigo_ping",
            "description": "Checks the health and responsiveness of the ANIGO engine and checks if the live desktop studio window is active on port 39090.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "anigo_get_system_info",
            "description": "Returns details about the GPU hardware adapter, backend driver, and WebGPU pipeline.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "anigo_get_live_telemetry",
            "description": "Queries the live running ANIGO Studio window via TCP port 39090 to retrieve real-time FPS, draw calls, triangles, camera position, and active parameters.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "anigo_inspect_scene",
            "description": "Returns a detailed JSON summary of the active 3D scene (nodes, meshes, materials, camera, polygon count).",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "anigo_render_frame",
            "description": "Performs a headless offscreen WebGPU render of the current scene and returns a Base64-encoded PNG image and render metrics (draw calls, render time ms). Can sync with the live window camera if sync_live is true.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "width": { "type": "integer", "description": "Render width in pixels (default: 800)" },
                    "height": { "type": "integer", "description": "Render height in pixels (default: 600)" },
                    "save_path": { "type": "string", "description": "Optional local path to save the rendered PNG image" },
                    "sync_live": { "type": "boolean", "description": "If true, queries the live app window to match its camera and parameters before rendering" }
                }
            }
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
                }
            }
        },
        {
            "name": "anigo_load_mesh_preset",
            "description": "Swaps the scene mesh to a predefined test preset: 'mannequin', 'sphere', or 'cube', notifying the live window immediately.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "preset": { "type": "string", "enum": ["mannequin", "sphere", "cube"] }
                },
                "required": ["preset"]
            }
        },
        {
            "name": "anigo_set_light",
            "description": "Updates the direction, intensity, sun color, ambient light, and stylized shadow color tint of the cel-shading light in both internal scene and the live window.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "direction": { "type": "array", "items": { "type": "number" }, "description": "[x, y, z] light direction vector" },
                    "intensity": { "type": "number", "description": "Direct light intensity multiplier" },
                    "color": { "type": "array", "items": { "type": "number" }, "description": "[r, g, b] sun/direct light color (0.0 to 1.0)" },
                    "ambient_intensity": { "type": "number", "description": "Ambient environmental light floor (0.0 to 2.0)" },
                    "shadow_color": { "type": "array", "items": { "type": "number" }, "description": "[r, g, b] hue-shifted shadow tint (0.0 to 1.0)" }
                }
            }
        },
        {
            "name": "anigo_set_material_toon",
            "description": "Sets stylized NPR cel-shading material parameters (base color, shade tint, outline, threshold, smoothness, anisotropic specular highlight, Fresnel rim lighting, mathematical hue shift, and toon ramp quantization steps). Synchronizes internal scene and live interactive window.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "base_color": { "type": "array", "items": { "type": "number" }, "description": "[r, g, b, a] base albedo color" },
                    "shade_color": { "type": "array", "items": { "type": "number" }, "description": "[r, g, b, a] shadow color tint" },
                    "outline_color": { "type": "array", "items": { "type": "number" }, "description": "[r, g, b, a] outline lineart color" },
                    "outline_width": { "type": "number", "description": "Outline stroke width (default: 0.0035)" },
                    "shadow_threshold": { "type": "number", "description": "N.L light-shadow split angle threshold (0.0 to 1.0, default 0.5)" },
                    "shadow_smoothness": { "type": "number", "description": "Penumbra softness edge filter width (0.001 to 0.5, default 0.02)" },
                    "spec_intensity": { "type": "number", "description": "Anisotropic specular highlight intensity (0.0 to 2.0, default 0.4)" },
                    "spec_power": { "type": "number", "description": "Specular exponent sharpness (4.0 to 128.0, default 32.0)" },
                    "rim_intensity": { "type": "number", "description": "Stylized Fresnel rim light intensity (0.0 to 3.0, default 0.8)" },
                    "rim_spread": { "type": "number", "description": "Rim light angular spread (0.05 to 1.0, default 0.4)" },
                    "hue_shift": { "type": "number", "description": "Shadow hue rotation angle in degrees (-180.0 to +180.0, default -15.0 for cool lavender)" },
                    "toon_steps": { "type": "number", "description": "Toon ramp bands: 1.0 = hard anime cel, 2.0 = 2-tier Ghibli soft, 0.0 = continuous (default 1.0)" }
                }
            }
        },
        {
            "name": "anigo_compare_baseline",
            "description": "Compares a rendered frame or image against a reference baseline PNG image using MSE, PSNR, and pixel tolerance metrics. If current_image_path is omitted, renders the current WebGPU scene at the baseline dimensions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "baseline_path": { "type": "string", "description": "Path to the reference/baseline PNG image" },
                    "current_image_path": { "type": "string", "description": "Optional path to the current rendered PNG image (if omitted, renders current scene)" },
                    "diff_save_path": { "type": "string", "description": "Optional path to save an amplified visual difference map PNG" },
                    "mse_threshold": { "type": "number", "description": "Maximum acceptable MSE (default: 50.0)" },
                    "psnr_threshold": { "type": "number", "description": "Minimum acceptable PSNR in dB (default: 25.0)" },
                    "tolerance_channel_diff": { "type": "integer", "description": "Max per-channel 8-bit difference for a pixel to count as matching (default: 8)" }
                },
                "required": ["baseline_path"]
            }
        },
        {
            "name": "anigo_set_proportions",
            "description": "Sets the anatomical anime proportions (head scale and head ratio canon) on the mannequin.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "head_scale": { "type": "number", "description": "Head scale multiplier (0.7 to 1.4, default 1.0)" },
                    "head_ratio": { "type": "number", "description": "Total body height in head units (2.0 chibi to 8.5 heroic, default 6.5)" }
                }
            }
        },
        {
            "name": "anigo_find_window",
            "description": "Locates the ANIGO Studio desktop window, checks if it is minimized, and returns its title, screen coordinates, width, and height.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "anigo_screenshot_window",
            "description": "Captures a full high-resolution screenshot of the physical ANIGO application window (including Tauri shell UI and WebGPU Viewport), auto-restoring it if minimized, and saves it to a PNG file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "save_path": { "type": "string", "description": "Local path to save the window PNG screenshot" }
                }
            }
        },
        {
            "name": "anigo_mouse_click",
            "description": "Simulates a physical mouse click on the ANIGO desktop window (auto-restoring and focusing the window first).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "x": { "type": "integer", "description": "Horizontal coordinate in pixels (relative to window top-left)" },
                    "y": { "type": "integer", "description": "Vertical coordinate in pixels (relative to window top-left)" },
                    "button": { "type": "string", "enum": ["left", "right", "middle"], "description": "Mouse button to click (default: 'left')" }
                },
                "required": ["x", "y"]
            }
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
                    "steps": { "type": "integer", "description": "Number of interpolation steps (default: 15)" }
                },
                "required": ["start_x", "start_y", "end_x", "end_y"]
            }
        },
        {
            "name": "anigo_maximize_window",
            "description": "Maximizes the ANIGO application window on the desktop, ensuring it is in foreground and focused.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "anigo_restore_window",
            "description": "Restores the ANIGO application window from minimized state to its normal restored size.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "anigo_mouse_scroll",
            "description": "Simulates a mouse wheel scroll on the ANIGO window (e.g. to zoom in or out in the 3D viewport). Positive delta zooms in, negative zooms out.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "delta": { "type": "integer", "description": "Wheel scroll delta (e.g. 120 for scroll up/zoom in, -120 for scroll down/zoom out)" }
                },
                "required": ["delta"]
            }
        },
        {
            "name": "anigo_send_key",
            "description": "Simulates a virtual key press on the ANIGO window (e.g. 0x12 for Alt, 0x10 for Shift, 0x11 for Ctrl).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "vk_code": { "type": "integer", "description": "Virtual key code (e.g. 18 for VK_MENU/Alt, 16 for VK_SHIFT)" }
                },
                "required": ["vk_code"]
            }
        },
        {
            "name": "anigo_get_diagnostics",
            "description": "Collects diagnostic logs (crash logs, panic logs, launch traces) and checks the status of the live bridge and GPU pipeline.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "anigo_get_window_state",
            "description": "Queries real-time window metrics from the live application (is_minimized, is_maximized, is_visible, is_focused, inner/outer size, screen position, scale factor).",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "anigo_focus_window",
            "description": "Focuses the ANIGO application window and brings it to the foreground.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "anigo_minimize_window",
            "description": "Minimizes the ANIGO application window to the taskbar.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "anigo_ui_action",
            "description": "Executes semantic UI actions in ANIGO Studio (switch workspace tabs, select tools on left bar, set slider values, change mesh presets). Supported action types: 'select_tab' (e.g. 'personagem', 'shading', 'mcp', 'settings'), 'select_tool' (e.g. 'mannequin', 'face', 'hair', 'cloth', 'rig'), 'set_slider' (property: 'head_scale' | 'head_ratio' | 'outline_width' | 'shadow_threshold' | 'light_azimuth' | 'light_elevation' | 'light_intensity', value: number), 'set_preset' (value: 'mannequin' | 'sphere' | 'cube').",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["select_tab", "select_tool", "set_slider", "set_preset"], "description": "Action type" },
                    "property": { "type": "string", "description": "Target property name (for sliders: head_scale, head_ratio, outline_width, etc.)" },
                    "value": { "description": "Value to set (string for tab/tool/preset, number for sliders)" }
                },
                "required": ["action"]
            }
        },
        {
            "name": "anigo_read_logs",
            "description": "Reads application runtime logs (launch.log, panic.log, app_crash.log, and error traces) for diagnostic and debugging purposes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filter": { "type": "string", "enum": ["all", "launch", "panic", "crash"], "description": "Filter log type (default: 'all')" }
                }
            }
        },
        {
            "name": "anigo_set_character_model",
            "description": "Swaps the active canonical base character model between male ('male') and female ('female') isomorphic meshes, retaining or adapting morph state.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "model_type": { "type": "string", "enum": ["male", "female"], "description": "Canonical base mesh model type" }
                },
                "required": ["model_type"]
            }
        },
        {
            "name": "anigo_set_somatotype",
            "description": "Applies Heath-Carter somatotype body shape coordinates (Endomorphy [adiposity], Mesomorphy [muscularity], Ectomorphy [linearity]) to continuous mesh morphing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "endo": { "type": "number", "description": "Endomorphy adiposity component (1.0 to 12.0, default 3.0)" },
                    "meso": { "type": "number", "description": "Mesomorphy musculoskeletal component (1.0 to 12.0, default 4.0)" },
                    "ecto": { "type": "number", "description": "Ectomorphy linearity/slenderness component (1.0 to 12.0, default 3.0)" }
                }
            }
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
                "required": ["slider_id", "value"]
            }
        },
        {
            "name": "anigo_inspect_mesh_integrity",
            "description": "Performs rigorous geometric validation on the active 3D character mesh: checks for degenerate triangles, inverted outward normals, NaN/infinite coordinates, and bounding box dimensions.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "anigo_get_active_morphs",
            "description": "Returns list of all morph sliders that deviate from their default neutral values along with active weights.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "anigo_reset_morphs",
            "description": "Resets all 148+ anatomical and anime morph sliders to their canonical neutral default values.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }
    ])
}

async fn handle_tool_call(
    name: &str,
    args: Value,
    state: Arc<Mutex<AppState>>,
) -> Result<Vec<Value>> {
    let mut state = state.lock().await;

    match name {
        "anigo_ping" => {
            let live_active = state.bridge.is_live().await;
            let status_msg = if live_active {
                "Pong! ANIGO Engine is running on WebGPU/wgpu and LIVE STUDIO WINDOW is connected on 127.0.0.1:39090."
            } else {
                "Pong! ANIGO Engine is running on WebGPU/wgpu (Headless mode, live window not detected on 39090)."
            };
            Ok(vec![json!({
                "type": "text",
                "text": status_msg
            })])
        }
        "anigo_get_system_info" => {
            let info = &state.renderer.adapter_info;
            let live_active = state.bridge.is_live().await;
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
            let summary = json!({
                "node_count": state.scene.nodes.len(),
                "total_vertices": state.scene.total_vertices(),
                "total_triangles": state.scene.total_triangles(),
                "camera": state.scene.camera,
                "light": state.scene.light,
                "nodes": state.scene.nodes.iter().map(|n| json!({
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

            if let Some(az) = args.get("orbit_azimuth").and_then(|v| v.as_f64()) {
                let el = args.get("orbit_elevation").and_then(|v| v.as_f64()).unwrap_or(0.0);
                state.scene.camera.orbit(az as f32, el as f32);
                let _ = state.bridge.send_command("ORBIT", json!({ "azimuth": az, "elevation": el })).await;
                actions_taken.push(format!("Orbit: az={:.2} rad, el={:.2} rad", az, el));
            }
            if let Some(zoom) = args.get("zoom_factor").and_then(|v| v.as_f64()) {
                state.scene.camera.zoom(zoom as f32);
                let _ = state.bridge.send_command("ZOOM", json!({ "factor": zoom })).await;
                actions_taken.push(format!("Zoom: factor={:.2}", zoom));
            }
            if let Some(dx) = args.get("pan_dx").and_then(|v| v.as_f64()) {
                let dy = args.get("pan_dy").and_then(|v| v.as_f64()).unwrap_or(0.0);
                state.scene.camera.pan(dx as f32, dy as f32);
                let _ = state.bridge.send_command("PAN", json!({ "dx": dx, "dy": dy })).await;
                actions_taken.push(format!("Pan: dx={:.2}, dy={:.2}", dx, dy));
            }
            if let Some(eye_arr) = args.get("eye").and_then(|v| v.as_array()) {
                if eye_arr.len() == 3 {
                    state.scene.camera.eye = glam::Vec3::new(
                        eye_arr[0].as_f64().unwrap_or(0.0) as f32,
                        eye_arr[1].as_f64().unwrap_or(1.5) as f32,
                        eye_arr[2].as_f64().unwrap_or(3.5) as f32,
                    );
                    let _ = state.bridge.send_command("SET_CAMERA", json!({ "eye": eye_arr })).await;
                    actions_taken.push(format!("Set Eye: {:?}", state.scene.camera.eye));
                }
            }
            if let Some(target_arr) = args.get("target").and_then(|v| v.as_array()) {
                if target_arr.len() == 3 {
                    state.scene.camera.target = glam::Vec3::new(
                        target_arr[0].as_f64().unwrap_or(0.0) as f32,
                        target_arr[1].as_f64().unwrap_or(1.0) as f32,
                        target_arr[2].as_f64().unwrap_or(0.0) as f32,
                    );
                    let _ = state.bridge.send_command("SET_CAMERA", json!({ "target": target_arr })).await;
                    actions_taken.push(format!("Set Target: {:?}", state.scene.camera.target));
                }
            }

            Ok(vec![json!({
                "type": "text",
                "text": format!("Camera updated: {} | Current Eye: {:?}, Target: {:?}", actions_taken.join(", "), state.scene.camera.eye, state.scene.camera.target)
            })])
        }
        "anigo_set_light" => {
            if let Some(dir) = args.get("direction").and_then(|v| v.as_array()) {
                if dir.len() == 3 {
                    let d = glam::Vec3::new(
                        dir[0].as_f64().unwrap_or(0.5) as f32,
                        dir[1].as_f64().unwrap_or(1.0) as f32,
                        dir[2].as_f64().unwrap_or(0.5) as f32,
                    ).normalize();
                    state.scene.light.direction = [d.x, d.y, d.z];
                }
            }
            if let Some(intensity) = args.get("intensity").and_then(|v| v.as_f64()) {
                state.scene.light.intensity = intensity as f32;
            }
            if let Some(col) = args.get("color").and_then(|v| v.as_array()) {
                if col.len() == 3 {
                    state.scene.light.color = [
                        col[0].as_f64().unwrap_or(1.0) as f32,
                        col[1].as_f64().unwrap_or(0.98) as f32,
                        col[2].as_f64().unwrap_or(0.95) as f32,
                    ];
                }
            }
            if let Some(ambient) = args.get("ambient_intensity").and_then(|v| v.as_f64()) {
                state.scene.light.ambient_intensity = ambient as f32;
            }
            if let Some(shadow_tint) = args.get("shadow_color").and_then(|v| v.as_array()) {
                if shadow_tint.len() == 3 {
                    state.scene.light.shadow_color = [
                        shadow_tint[0].as_f64().unwrap_or(0.65) as f32,
                        shadow_tint[1].as_f64().unwrap_or(0.68) as f32,
                        shadow_tint[2].as_f64().unwrap_or(0.85) as f32,
                    ];
                }
            }

            let _ = state.bridge.send_command("SET_LIGHT", json!({
                "direction": state.scene.light.direction,
                "intensity": state.scene.light.intensity,
                "color": state.scene.light.color,
                "ambient_intensity": state.scene.light.ambient_intensity,
                "shadow_color": state.scene.light.shadow_color,
            })).await;

            Ok(vec![json!({
                "type": "text",
                "text": format!(
                    "Light updated:\n- Direction: {:?}\n- Intensity: {:.2}\n- Sun Color: {:?}\n- Ambient: {:.2}\n- Shadow Tint: {:?}",
                    state.scene.light.direction,
                    state.scene.light.intensity,
                    state.scene.light.color,
                    state.scene.light.ambient_intensity,
                    state.scene.light.shadow_color
                )
            })])
        }
        "anigo_set_material_toon" => {
            let mut current_mat = state.scene.nodes.first()
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
                current_mat.outline_width = ow as f32;
            }
            if let Some(st) = args.get("shadow_threshold").and_then(|v| v.as_f64()) {
                current_mat.shadow_threshold = st as f32;
            }
            if let Some(ss) = args.get("shadow_smoothness").and_then(|v| v.as_f64()) {
                current_mat.shadow_smoothness = ss as f32;
            }
            if let Some(si) = args.get("spec_intensity").and_then(|v| v.as_f64()) {
                current_mat.spec_intensity = si as f32;
            }
            if let Some(sp) = args.get("spec_power").and_then(|v| v.as_f64()) {
                current_mat.spec_power = sp as f32;
            }
            if let Some(ri) = args.get("rim_intensity").and_then(|v| v.as_f64()) {
                current_mat.rim_intensity = ri as f32;
            }
            if let Some(rs) = args.get("rim_spread").and_then(|v| v.as_f64()) {
                current_mat.rim_spread = rs as f32;
            }
            if let Some(hs) = args.get("hue_shift").and_then(|v| v.as_f64()) {
                current_mat.hue_shift = hs as f32;
            }
            if let Some(ts) = args.get("toon_steps").and_then(|v| v.as_f64()) {
                current_mat.toon_steps = ts as f32;
            }

            // Update internal scene representation
            state.scene.update_material_for_all(current_mat.clone());

            // Synchronize with live interactive window if connected
            let _ = state.bridge.send_command("SET_MATERIAL_TOON", json!({
                "base_color": current_mat.base_color,
                "shade_color": current_mat.shade_color,
                "outline_color": current_mat.outline_color,
                "outline_width": current_mat.outline_width,
                "shadow_threshold": current_mat.shadow_threshold,
                "shadow_smoothness": current_mat.shadow_smoothness,
                "spec_intensity": current_mat.spec_intensity,
                "spec_power": current_mat.spec_power,
                "rim_intensity": current_mat.rim_intensity,
                "rim_spread": current_mat.rim_spread,
                "hue_shift": current_mat.hue_shift,
                "toon_steps": current_mat.toon_steps,
            })).await;

            Ok(vec![json!({
                "type": "text",
                "text": format!(
                    "Stylized Toon Material updated successfully:\n- Base Color: {:?}\n- Shade Color: {:?}\n- Outline Color: {:?}\n- Outline Width: {:.4}\n- Shadow Threshold: {:.3}\n- Shadow Smoothness: {:.3}\n- Specular Intensity: {:.2} (Power: {:.1})\n- Rim Light Intensity: {:.2} (Spread: {:.2})\n- Hue Shift: {:.1}°\n- Toon Steps: {:.1} (1=cel, 2=ghibli, 0=continuous)",
                    current_mat.base_color,
                    current_mat.shade_color,
                    current_mat.outline_color,
                    current_mat.outline_width,
                    current_mat.shadow_threshold,
                    current_mat.shadow_smoothness,
                    current_mat.spec_intensity,
                    current_mat.spec_power,
                    current_mat.rim_intensity,
                    current_mat.rim_spread,
                    current_mat.hue_shift,
                    current_mat.toon_steps,
                )
            })])
        }
        "anigo_compare_baseline" => {
            let baseline_path = args.get("baseline_path").and_then(|v| v.as_str())
                .context("baseline_path is required for anigo_compare_baseline")?;
            let current_image_path = args.get("current_image_path").and_then(|v| v.as_str());
            let diff_save_path = args.get("diff_save_path").and_then(|v| v.as_str());
            let mse_threshold = args.get("mse_threshold").and_then(|v| v.as_f64()).unwrap_or(50.0);
            let psnr_threshold = args.get("psnr_threshold").and_then(|v| v.as_f64()).unwrap_or(25.0);
            let tolerance_channel = args.get("tolerance_channel_diff").and_then(|v| v.as_u64()).unwrap_or(8) as i32;

            // 1. Load baseline image
            let baseline_img = load_rgba_image(baseline_path)
                .with_context(|| format!("Failed to open baseline image at {}", baseline_path))?;
            let (b_w, b_h) = (baseline_img.width(), baseline_img.height());

            // 2. Load or render current image
            let current_img = match current_image_path {
                Some(path) => load_rgba_image(path)
                    .with_context(|| format!("Failed to open current image at {}", path))?,
                None => {
                    // Render directly from current headless WebGPU state at baseline dimensions
                    let (img_buf, _metrics) = state.renderer.render_scene(&state.scene, b_w, b_h).await?;
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
                std::fs::write(diff_path, &png_bytes)
                    .with_context(|| format!("Failed to save diff image to {}", diff_path))?;
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
                "baseline_path": baseline_path,
                "current_source": current_image_path.unwrap_or("(direct WebGPU headless render)"),
            });

            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&audit_result)?
            })])
        }
        "anigo_load_mesh_preset" => {
            let preset = args.get("preset").and_then(|v| v.as_str()).unwrap_or("mannequin");
            let mesh = match preset {
                "sphere" => Mesh::create_uv_sphere_at(0.85, 36, 72, [0.0, 1.0, 0.0]),
                "cube" => Mesh::create_cube_at(1.2, [0.0, 1.0, 0.0]),
                _ => Mesh::create_mannequin_proxy(),
            };

            let current_mat = state.scene.nodes.first()
                .and_then(|n| n.material.clone())
                .unwrap_or_default();

            state.scene.nodes.clear();
            let mut node = SceneNode::new("primary_mesh", preset).with_mesh(mesh);
            node.material = Some(current_mat);
            state.scene.add_node(node);

            let _ = state.bridge.send_command("LOAD_PRESET", json!({ "preset": preset })).await;

            Ok(vec![json!({
                "type": "text",
                "text": format!("Loaded mesh preset '{}' with {} vertices (synchronized with live window).", preset, state.scene.total_vertices())
            })])
        }
        "anigo_set_character_model" => {
            let model_type = args.get("model_type").and_then(|v| v.as_str()).unwrap_or("male");
            let gender = if model_type.eq_ignore_ascii_case("female") {
                BaseGender::Female
            } else {
                BaseGender::Male
            };

            state.current_gender = gender;
            state.base_mesh = Mesh::create_canonical_base(gender);
            state.morph_catalog.set_gender(gender);

            let mut morphed_mesh = state.base_mesh.clone();
            state.morph_catalog.apply_to_mesh(&state.base_mesh, &mut morphed_mesh);

            let current_mat = state.scene.nodes.first()
                .and_then(|n| n.material.clone())
                .unwrap_or_default();

            state.scene.nodes.clear();
            let mut node = SceneNode::new("primary_mesh", format!("canonical_{}", model_type)).with_mesh(morphed_mesh);
            node.material = Some(current_mat);
            state.scene.add_node(node);

            let _ = state.bridge.send_command("SET_CHARACTER_MODEL", json!({ "model_type": model_type })).await;

            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&json!({
                    "status": "success",
                    "model_type": model_type,
                    "gender": format!("{:?}", gender),
                    "vertex_count": state.scene.total_vertices(),
                    "triangle_count": state.scene.total_triangles(),
                    "isomorphic": true,
                    "active_morphs_count": state.morph_catalog.get_active_morphs().len()
                }))?
            })])
        }
        "anigo_set_somatotype" => {
            let endo = args.get("endo").and_then(|v| v.as_f64()).unwrap_or(3.0) as f32;
            let meso = args.get("meso").and_then(|v| v.as_f64()).unwrap_or(4.0) as f32;
            let ecto = args.get("ecto").and_then(|v| v.as_f64()).unwrap_or(3.0) as f32;

            let coords = SomatotypeCoords::new(endo, meso, ecto).normalized();
            state.morph_catalog.set_slider("somatotype_endomorph", coords.endomorph).map_err(|e| anyhow::anyhow!(e))?;
            state.morph_catalog.set_slider("somatotype_mesomorph", coords.mesomorph).map_err(|e| anyhow::anyhow!(e))?;
            state.morph_catalog.set_slider("somatotype_ectomorph", coords.ectomorph).map_err(|e| anyhow::anyhow!(e))?;

            let mut morphed_mesh = state.base_mesh.clone();
            state.morph_catalog.apply_to_mesh(&state.base_mesh, &mut morphed_mesh);
            if let Some(node) = state.scene.nodes.first_mut() {
                node.mesh = Some(morphed_mesh);
            }

            let pad = coords.to_pad_2d();
            let _ = state.bridge.send_command("SET_SOMATOTYPE", json!({
                "endo": endo,
                "meso": meso,
                "ecto": ecto,
                "barycentric": [coords.endomorph, coords.mesomorph, coords.ectomorph],
                "pad_2d": [pad.0, pad.1]
            })).await;

            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&json!({
                    "status": "success",
                    "input": { "endo": endo, "meso": meso, "ecto": ecto },
                    "normalized_barycentric": { "endomorph": coords.endomorph, "mesomorph": coords.mesomorph, "ectomorph": coords.ectomorph },
                    "pad_2d": { "x": pad.0, "y": pad.1 }
                }))?
            })])
        }
        "anigo_apply_morph_slider" => {
            let slider_id = args.get("slider_id").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("Missing 'slider_id'"))?;
            let value = args.get("value").and_then(|v| v.as_f64()).ok_or_else(|| anyhow::anyhow!("Missing 'value'"))? as f32;

            let applied = state.morph_catalog.set_slider(slider_id, value).map_err(|e| anyhow::anyhow!(e))?;

            let mut morphed_mesh = state.base_mesh.clone();
            state.morph_catalog.apply_to_mesh(&state.base_mesh, &mut morphed_mesh);
            if let Some(node) = state.scene.nodes.first_mut() {
                node.mesh = Some(morphed_mesh);
            }

            let def = find_slider_def(slider_id);
            let zone_name = def.map(|d| d.zone.name()).unwrap_or("Unknown");

            let _ = state.bridge.send_command("APPLY_MORPH_SLIDER", json!({
                "slider_id": slider_id,
                "value": applied
            })).await;

            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&json!({
                    "status": "success",
                    "slider_id": slider_id,
                    "applied_value": applied,
                    "zone": zone_name,
                    "active_morphs_count": state.morph_catalog.get_active_morphs().len()
                }))?
            })])
        }
        "anigo_inspect_mesh_integrity" => {
            let mesh = state.scene.nodes.first()
                .and_then(|n| n.mesh.as_ref())
                .ok_or_else(|| anyhow::anyhow!("No active mesh in scene to inspect"))?;

            let report = MorphCatalog::inspect_integrity(mesh);

            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&report)?
            })])
        }
        "anigo_get_active_morphs" => {
            let active = state.morph_catalog.get_active_morphs();
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
            state.morph_catalog.reset_all();

            let mut morphed_mesh = state.base_mesh.clone();
            state.morph_catalog.apply_to_mesh(&state.base_mesh, &mut morphed_mesh);
            if let Some(node) = state.scene.nodes.first_mut() {
                node.mesh = Some(morphed_mesh);
            }

            let _ = state.bridge.send_command("RESET_MORPHS", json!({})).await;

            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&json!({
                    "status": "success",
                    "message": "All morph sliders reset to canonical neutral defaults.",
                    "active_morphs_count": 0
                }))?
            })])
        }
        "anigo_set_proportions" => {
            let scale = args.get("head_scale").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
            let ratio = args.get("head_ratio").and_then(|v| v.as_f64()).unwrap_or(6.5) as f32;

            let current_mat = state.scene.nodes.first()
                .and_then(|n| n.material.clone())
                .unwrap_or_default();

            let mesh = Mesh::create_mannequin_proxy_proportions(scale, ratio);
            state.scene.nodes.clear();
            let mut node = SceneNode::new("primary_mesh", "mannequin").with_mesh(mesh);
            node.material = Some(current_mat);
            state.scene.add_node(node);

            let _ = state.bridge.send_command("SET_PROPORTIONS", json!({
                "head_scale": scale,
                "head_ratio": ratio,
            })).await;

            Ok(vec![json!({
                "type": "text",
                "text": format!("Set proportions: Head Scale = {:.2}x, Head Ratio = {:.1} heads (sent to live window and offscreen renderer).", scale, ratio)
            })])
        }
        "anigo_render_frame" => {
            let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(800) as u32;
            let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(600) as u32;
            let save_path = args.get("save_path").and_then(|v| v.as_str());
            let sync_live = args.get("sync_live").and_then(|v| v.as_bool()).unwrap_or(false);

            if sync_live {
                if let Ok(telemetry) = state.bridge.send_command("GET_STATUS", json!({})).await {
                    if let Some(eye_arr) = telemetry.get("camera_eye").and_then(|v| v.as_array()) {
                        if eye_arr.len() == 3 {
                            state.scene.camera.eye = glam::Vec3::new(
                                eye_arr[0].as_f64().unwrap_or(0.0) as f32,
                                eye_arr[1].as_f64().unwrap_or(1.5) as f32,
                                eye_arr[2].as_f64().unwrap_or(3.5) as f32,
                            );
                        }
                    }
                    if let Some(target_arr) = telemetry.get("camera_target").and_then(|v| v.as_array()) {
                        if target_arr.len() == 3 {
                            state.scene.camera.target = glam::Vec3::new(
                                target_arr[0].as_f64().unwrap_or(0.0) as f32,
                                target_arr[1].as_f64().unwrap_or(1.0) as f32,
                                target_arr[2].as_f64().unwrap_or(0.0) as f32,
                            );
                        }
                    }
                }
            }

            let (image_buf, metrics) = state.renderer.render_scene(&state.scene, width, height).await?;

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
                std::fs::write(path, &png_bytes)
                    .with_context(|| format!("Failed to save rendered frame to {}", path))?;
            }

            let report_text = format!(
                "Render Successful: {}x{} | Time: {:.2}ms | Triangles: {} | Draw Calls: {} | GPU: {} ({})",
                width, height, metrics.render_time_ms, metrics.triangle_count, metrics.draw_calls, metrics.adapter_name, metrics.backend
            );

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
            let hwnd_hint = if let Ok(state_resp) = state.bridge.send_command("GET_WINDOW_STATE", json!({})).await {
                state_resp.get("hwnd").and_then(|v| v.as_u64()).map(|h| h as *mut std::ffi::c_void)
            } else {
                None
            };
            let (hwnd, title, rect, is_minimized) = Win32Harness::find_anigo_window_with_hint(hwnd_hint)?;
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;

            Ok(vec![json!({
                "type": "text",
                "text": format!(
                    "ANIGO Window Status:\n- HWND: {:?}\n- Title: '{}'\n- Minimized: {}\n- Bounds: [left: {}, top: {}, right: {}, bottom: {}]\n- Dimensions: {}x{} px",
                    hwnd, title, is_minimized, rect.left, rect.top, rect.right, rect.bottom, width, height
                )
            })])
        }
        "anigo_screenshot_window" => {
            let hwnd_hint = if let Ok(state_resp) = state.bridge.send_command("GET_WINDOW_STATE", json!({})).await {
                state_resp.get("hwnd").and_then(|v| v.as_u64()).map(|h| h as *mut std::ffi::c_void)
            } else {
                None
            };
            let (hwnd, title, _rect, was_minimized) = Win32Harness::find_anigo_window_with_hint(hwnd_hint)?;
            let restored_rect = Win32Harness::ensure_window_active(hwnd)?;
            let img = Win32Harness::capture_window(hwnd, &restored_rect)?;

            let width = img.width();
            let height = img.height();

            let mut png_bytes = Vec::new();
            let encoder = PngEncoder::new(&mut png_bytes);
            encoder.write_image(&img, width, height, ColorType::Rgba8.into())?;

            let save_path = args.get("save_path").and_then(|v| v.as_str());
            if let Some(path) = save_path {
                std::fs::write(path, &png_bytes)
                    .with_context(|| format!("Failed to save window screenshot to {}", path))?;
            }

            let base64_str = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
            let status_msg = format!(
                "Captured Full Window Screenshot (Inside + Outside Viewport):\n- Title: '{}'\n- Size: {}x{} px\n- Was Minimized (Auto-Restored): {}\n- Saved to: {}",
                title, width, height, was_minimized, save_path.unwrap_or("(memory only)")
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
            let button = args.get("button").and_then(|v| v.as_str()).unwrap_or("left");

            let (hwnd, _title, _rect, _is_min) = Win32Harness::find_anigo_window()?;
            let active_rect = Win32Harness::ensure_window_active(hwnd)?;

            let screen_x = active_rect.left + x;
            let screen_y = active_rect.top + y;

            Win32Harness::mouse_click(screen_x, screen_y, button);

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
            let button = args.get("button").and_then(|v| v.as_str()).unwrap_or("left");
            let steps = args.get("steps").and_then(|v| v.as_u64()).unwrap_or(15) as u32;

            let (hwnd, _title, _rect, _is_min) = Win32Harness::find_anigo_window()?;
            let active_rect = Win32Harness::ensure_window_active(hwnd)?;

            let s_x = active_rect.left + start_x;
            let s_y = active_rect.top + start_y;
            let e_x = active_rect.left + end_x;
            let e_y = active_rect.top + end_y;

            Win32Harness::mouse_drag(s_x, s_y, e_x, e_y, button, steps);

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
                    Err(e) => eprintln!("[MCP] Bridge maximize failed, falling back to Win32: {:#}", e),
                }
            }
            let (hwnd, title, _rect, _is_min) = Win32Harness::find_anigo_window()?;
            let new_rect = Win32Harness::maximize_window(hwnd)?;
            let w = new_rect.right - new_rect.left;
            let h = new_rect.bottom - new_rect.top;
            Ok(vec![json!({
                "type": "text",
                "text": format!("Window '{}' (HWND: {:?}) maximized via Win32 to {}x{} px [bounds: {}, {}, {}, {}]", title, hwnd, w, h, new_rect.left, new_rect.top, new_rect.right, new_rect.bottom)
            })])
        }
        "anigo_restore_window" => {
            if state.bridge.is_live().await {
                match state.bridge.send_command("RESTORE_WINDOW", json!({})).await {
                    Ok(resp) => return Ok(vec![json!({
                        "type": "text",
                        "text": format!("Window restored via native Live Bridge: {}", serde_json::to_string_pretty(&resp)?)
                    })]),
                    Err(e) => eprintln!("[MCP] Bridge restore failed, falling back to Win32: {:#}", e),
                }
            }
            let (hwnd, title, _rect, _is_min) = Win32Harness::find_anigo_window()?;
            let new_rect = Win32Harness::restore_window(hwnd)?;
            let w = new_rect.right - new_rect.left;
            let h = new_rect.bottom - new_rect.top;
            Ok(vec![json!({
                "type": "text",
                "text": format!("Window '{}' (HWND: {:?}) restored via Win32 to {}x{} px [bounds: {}, {}, {}, {}]", title, hwnd, w, h, new_rect.left, new_rect.top, new_rect.right, new_rect.bottom)
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
                let (hwnd, title, rect, is_minimized) = Win32Harness::find_anigo_window()?;
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
            if !state.bridge.is_live().await {
                anyhow::bail!("Live studio window is not connected on port 39090");
            }
            let resp = state.bridge.send_command("UI_ACTION", args.clone()).await?;
            Ok(vec![json!({
                "type": "text",
                "text": format!("UI Action executed: {}", serde_json::to_string_pretty(&resp)?)
            })])
        }
        "anigo_read_logs" => {
            let filter = args.get("filter").and_then(|v| v.as_str()).unwrap_or("all");
            let diag = Win32Harness::collect_logs_and_diagnostics();
            let filtered = match filter {
                "launch" => json!({ "launch_log": diag.get("launch_log") }),
                "panic" => json!({ "panic_log": diag.get("panic_log") }),
                "crash" => json!({ "app_crash_log": diag.get("app_crash_log") }),
                _ => diag,
            };
            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&filtered)?
            })])
        }
        "anigo_mouse_scroll" => {
            let delta = args.get("delta").and_then(|v| v.as_i64()).unwrap_or(120) as i32;
            Win32Harness::mouse_wheel(delta);
            Ok(vec![json!({
                "type": "text",
                "text": format!("Dispatched mouse wheel scroll with delta: {}", delta)
            })])
        }
        "anigo_send_key" => {
            let vk = args.get("vk_code").and_then(|v| v.as_u64()).unwrap_or(0x12) as u8;
            Win32Harness::send_key(vk);
            Ok(vec![json!({
                "type": "text",
                "text": format!("Dispatched virtual key event for VK code: 0x{:02X}", vk)
            })])
        }
        "anigo_get_diagnostics" => {
            let diag = Win32Harness::collect_logs_and_diagnostics();
            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&diag)?
            })])
        }
        _ => anyhow::bail!("Unknown tool: {}", name),
    }
}

fn load_rgba_image(path: &str) -> Result<image::RgbaImage> {
    let bytes = std::fs::read(path).with_context(|| format!("Failed to read image file: {}", path))?;
    let dyn_img = image::load_from_memory(&bytes).with_context(|| format!("Failed to decode image from {}", path))?;
    Ok(dyn_img.to_rgba8())
}
