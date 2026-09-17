use std::io::{self, BufRead, Write};
use std::sync::Arc;
use anyhow::{Context, Result};
use base64::Engine;
use image::codecs::png::PngEncoder;
use image::{ColorType, ImageEncoder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use anigo_core::mesh::Mesh;
use anigo_core::scene::{Scene, SceneNode};
use anigo_renderer::HeadlessRenderer;

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

struct AppState {
    renderer: HeadlessRenderer,
    scene: Scene,
}

#[tokio::main]
async fn main() -> Result<()> {
    eprintln!("[anigo-mcp] Initializing ANIGO MCP Server with WebGPU engine...");

    let renderer = HeadlessRenderer::new()
        .await
        .context("Failed to initialize headless WebGPU renderer in anigo-mcp")?;

    eprintln!(
        "[anigo-mcp] Connected to GPU: {} ({:?})",
        renderer.adapter_info.name, renderer.adapter_info.backend
    );

    let state = Arc::new(Mutex::new(AppState {
        renderer,
        scene: Scene::default(),
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
                // MCP client notification, no response required
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
            "description": "Checks the health and responsiveness of the ANIGO engine.",
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
            "name": "anigo_inspect_scene",
            "description": "Returns a detailed JSON summary of the active 3D scene (nodes, meshes, materials, camera, polygon count).",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "anigo_render_frame",
            "description": "Performs a headless offscreen WebGPU render of the current scene and returns a Base64-encoded PNG image and render metrics (draw calls, render time ms).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "width": { "type": "integer", "description": "Render width in pixels (default: 800)" },
                    "height": { "type": "integer", "description": "Render height in pixels (default: 600)" },
                    "save_path": { "type": "string", "description": "Optional local path to save the rendered PNG image" }
                }
            }
        },
        {
            "name": "anigo_set_camera",
            "description": "Manipulates the orbital camera parameters (eye position, target, orbit rotation, zoom).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "orbit_azimuth": { "type": "number", "description": "Horizontal orbit angle delta in radians" },
                    "orbit_elevation": { "type": "number", "description": "Vertical orbit angle delta in radians" },
                    "zoom_factor": { "type": "number", "description": "Distance multiplier (e.g. 0.8 for zoom in, 1.2 for zoom out)" },
                    "eye": { "type": "array", "items": { "type": "number" }, "description": "Exact camera eye [x, y, z]" },
                    "target": { "type": "array", "items": { "type": "number" }, "description": "Exact camera target [x, y, z]" }
                }
            }
        },
        {
            "name": "anigo_load_mesh_preset",
            "description": "Swaps the scene mesh to a predefined test preset: 'mannequin', 'sphere', or 'cube'.",
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
            "description": "Updates the direction, intensity, or stylized shadow color tint of the cel-shading light.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "direction": { "type": "array", "items": { "type": "number" }, "description": "[x, y, z]" },
                    "intensity": { "type": "number" },
                    "shadow_color": { "type": "array", "items": { "type": "number" }, "description": "[r, g, b] hue-shifted shadow tint" }
                }
            }
        }
    ])
}

async fn notify_live_app(payload: &Value) {
    use tokio::io::AsyncWriteExt;
    if let Ok(mut stream) = tokio::net::TcpStream::connect("127.0.0.1:39090").await {
        if let Ok(data) = serde_json::to_vec(payload) {
            let _ = stream.write_all(&data).await;
        }
    }
}

async fn handle_tool_call(
    name: &str,
    args: Value,
    state: Arc<Mutex<AppState>>,
) -> Result<Vec<Value>> {
    let mut state = state.lock().await;

    match name {
        "anigo_ping" => Ok(vec![json!({
            "type": "text",
            "text": "Pong! ANIGO Engine is running smoothly on WebGPU/wgpu."
        })]),
        "anigo_get_system_info" => {
            let info = &state.renderer.adapter_info;
            let report = json!({
                "adapter": info.name,
                "vendor": info.vendor,
                "device": info.device,
                "deviceType": format!("{:?}", info.device_type),
                "driver": info.driver,
                "driverInfo": info.driver_info,
                "backend": format!("{:?}", info.backend),
            });
            Ok(vec![json!({
                "type": "text",
                "text": serde_json::to_string_pretty(&report)?
            })])
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
            if let Some(az) = args.get("orbit_azimuth").and_then(|v| v.as_f64()) {
                let el = args.get("orbit_elevation").and_then(|v| v.as_f64()).unwrap_or(0.0);
                state.scene.camera.orbit(az as f32, el as f32);
                notify_live_app(&json!({
                    "action": "ORBIT",
                    "azimuth": az,
                    "elevation": el
                })).await;
            }
            if let Some(zoom) = args.get("zoom_factor").and_then(|v| v.as_f64()) {
                state.scene.camera.zoom(zoom as f32);
                notify_live_app(&json!({
                    "action": "ZOOM",
                    "factor": zoom
                })).await;
            }
            if let Some(eye_arr) = args.get("eye").and_then(|v| v.as_array()) {
                if eye_arr.len() == 3 {
                    state.scene.camera.eye = glam::Vec3::new(
                        eye_arr[0].as_f64().unwrap_or(0.0) as f32,
                        eye_arr[1].as_f64().unwrap_or(1.5) as f32,
                        eye_arr[2].as_f64().unwrap_or(3.5) as f32,
                    );
                }
            }
            if let Some(target_arr) = args.get("target").and_then(|v| v.as_array()) {
                if target_arr.len() == 3 {
                    state.scene.camera.target = glam::Vec3::new(
                        target_arr[0].as_f64().unwrap_or(0.0) as f32,
                        target_arr[1].as_f64().unwrap_or(1.0) as f32,
                        target_arr[2].as_f64().unwrap_or(0.0) as f32,
                    );
                }
            }

            Ok(vec![json!({
                "type": "text",
                "text": format!("Camera updated: Eye = {:?}, Target = {:?}", state.scene.camera.eye, state.scene.camera.target)
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
            if let Some(shadow_tint) = args.get("shadow_color").and_then(|v| v.as_array()) {
                if shadow_tint.len() == 3 {
                    state.scene.light.shadow_color = [
                        shadow_tint[0].as_f64().unwrap_or(0.65) as f32,
                        shadow_tint[1].as_f64().unwrap_or(0.68) as f32,
                        shadow_tint[2].as_f64().unwrap_or(0.85) as f32,
                    ];
                }
            }

            notify_live_app(&json!({
                "action": "SET_LIGHT",
                "direction": state.scene.light.direction,
                "intensity": state.scene.light.intensity,
            })).await;

            Ok(vec![json!({
                "type": "text",
                "text": format!("Light updated: Dir = {:?}, Intensity = {}", state.scene.light.direction, state.scene.light.intensity)
            })])
        }
        "anigo_load_mesh_preset" => {
            let preset = args.get("preset").and_then(|v| v.as_str()).unwrap_or("mannequin");
            let mesh = match preset {
                "sphere" => Mesh::create_uv_sphere(0.8, 32, 64),
                "cube" => Mesh::create_cube(1.0),
                _ => Mesh::create_mannequin_proxy(),
            };

            state.scene.nodes.clear();
            state.scene.add_node(SceneNode::new("primary_mesh", preset).with_mesh(mesh));

            Ok(vec![json!({
                "type": "text",
                "text": format!("Loaded mesh preset '{}' with {} vertices.", preset, state.scene.total_vertices())
            })])
        }
        "anigo_render_frame" => {
            let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(800) as u32;
            let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(600) as u32;
            let save_path = args.get("save_path").and_then(|v| v.as_str());

            let (image_buf, metrics) = state.renderer.render_scene(&state.scene, width, height).await?;

            // Encode to PNG bytes
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
        _ => anyhow::bail!("Unknown tool: {}", name),
    }
}
