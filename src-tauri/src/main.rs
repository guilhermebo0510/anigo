// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;
use base64::Engine;
use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};
use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::sync::Mutex;

use anigo_core::mesh::Mesh;
use anigo_core::scene::{Scene, SceneNode};
use anigo_renderer::HeadlessRenderer;

#[derive(Serialize, Deserialize)]
pub struct ViewportFrameResponse {
    pub image_base64: String,
    pub render_time_ms: f64,
    pub draw_calls: u32,
    pub triangle_count: usize,
    pub adapter_name: String,
    pub backend: String,
}

pub struct AppState {
    pub renderer: HeadlessRenderer,
    pub scene: Scene,
}

#[tauri::command]
async fn render_viewport_frame(
    width: u32,
    height: u32,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<ViewportFrameResponse, String> {
    let state = state.lock().await;
    let (image_buf, metrics) = state
        .renderer
        .render_scene(&state.scene, width.max(64), height.max(64))
        .await
        .map_err(|e| format!("Render error: {:#}", e))?;

    let mut png_bytes = Vec::new();
    let encoder = PngEncoder::new(&mut png_bytes);
    encoder
        .write_image(&image_buf, width.max(64), height.max(64), ColorType::Rgba8.into())
        .map_err(|e| format!("PNG encode error: {:#}", e))?;

    let image_base64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);

    Ok(ViewportFrameResponse {
        image_base64,
        render_time_ms: metrics.render_time_ms,
        draw_calls: metrics.draw_calls,
        triangle_count: metrics.triangle_count,
        adapter_name: metrics.adapter_name,
        backend: metrics.backend,
    })
}

#[tauri::command]
async fn camera_orbit(
    azimuth: f32,
    elevation: f32,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let mut state = state.lock().await;
    state.scene.camera.orbit(azimuth, elevation);
    Ok(())
}

#[tauri::command]
async fn camera_zoom(
    factor: f32,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let mut state = state.lock().await;
    state.scene.camera.zoom(factor);
    Ok(())
}

#[tauri::command]
async fn camera_pan(
    dx: f32,
    dy: f32,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let mut state = state.lock().await;
    state.scene.camera.pan(dx, dy);
    Ok(())
}

#[tauri::command]
async fn load_mesh_preset(
    preset: String,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<String, String> {
    let mut state = state.lock().await;
    let mesh = match preset.as_str() {
        "sphere" => Mesh::create_uv_sphere(0.8, 32, 64),
        "cube" => Mesh::create_cube(1.0),
        _ => Mesh::create_mannequin_proxy(),
    };

    state.scene.nodes.clear();
    state.scene.add_node(SceneNode::new("primary_mesh", preset.clone()).with_mesh(mesh));

    Ok(format!("Loaded preset {}", preset))
}

#[tauri::command]
async fn set_light_params(
    direction: [f32; 3],
    intensity: f32,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let mut state = state.lock().await;
    let d = glam::Vec3::from_array(direction).normalize();
    state.scene.light.direction = [d.x, d.y, d.z];
    state.scene.light.intensity = intensity;
    Ok(())
}

fn main() {
    // Initialize WebGPU renderer synchronously at startup
    let renderer = pollster::block_on(async {
        HeadlessRenderer::new()
            .await
            .expect("Failed to initialize WebGPU renderer for ANIGO")
    });

    let app_state = Arc::new(Mutex::new(AppState {
        renderer,
        scene: Scene::default(),
    }));

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            render_viewport_frame,
            camera_orbit,
            camera_zoom,
            camera_pan,
            load_mesh_preset,
            set_light_params,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
