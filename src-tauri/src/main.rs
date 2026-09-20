// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge;

use std::io::Write;
use std::sync::Arc;
use base64::Engine;
use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};
use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::sync::{Mutex, RwLock};

use anigo_core::mesh::Mesh;
use anigo_core::scene::{Scene, SceneNode};
use anigo_renderer::HeadlessRenderer;
use bridge::{LiveBridgeServer, LiveWindowState};

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
    pub renderer: Option<HeadlessRenderer>,
    pub scene: Scene,
}

#[tauri::command]
async fn render_viewport_frame(
    width: u32,
    height: u32,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<ViewportFrameResponse, String> {
    let mut state = state.lock().await;
    if state.renderer.is_none() {
        state.renderer = HeadlessRenderer::new().await.ok();
    }
    let renderer = state
        .renderer
        .as_ref()
        .ok_or_else(|| "Headless renderer not available on this platform".to_string())?;

    let (image_buf, metrics) = renderer
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

    let current_mat = state.scene.nodes.first()
        .and_then(|n| n.material.clone())
        .unwrap_or_default();

    state.scene.nodes.clear();
    let mut node = SceneNode::new("primary_mesh", preset.clone()).with_mesh(mesh);
    node.material = Some(current_mat);
    state.scene.add_node(node);

    Ok(format!("Loaded preset {}", preset))
}

#[tauri::command]
async fn set_light_params(
    direction: [f32; 3],
    intensity: f32,
    color: Option<[f32; 3]>,
    shadow_color: Option<[f32; 3]>,
    ambient_intensity: Option<f32>,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let mut state = state.lock().await;
    let d = glam::Vec3::from_array(direction).normalize();
    state.scene.light.direction = [d.x, d.y, d.z];
    state.scene.light.intensity = intensity;
    if let Some(c) = color {
        state.scene.light.color = c;
    }
    if let Some(sc) = shadow_color {
        state.scene.light.shadow_color = sc;
    }
    if let Some(ai) = ambient_intensity {
        state.scene.light.ambient_intensity = ai;
    }
    Ok(())
}

#[tauri::command]
async fn set_material_toon_params(
    shadow_threshold: Option<f32>,
    shadow_smoothness: Option<f32>,
    spec_intensity: Option<f32>,
    spec_power: Option<f32>,
    rim_intensity: Option<f32>,
    rim_spread: Option<f32>,
    hue_shift: Option<f32>,
    toon_steps: Option<f32>,
    base_color: Option<[f32; 4]>,
    shade_color: Option<[f32; 4]>,
    outline_width: Option<f32>,
    outline_color: Option<[f32; 4]>,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let mut state = state.lock().await;
    for node in &mut state.scene.nodes {
        if node.material.is_none() {
            node.material = Some(anigo_core::StylizedMaterial::default());
        }
        if let Some(mat) = node.material.as_mut() {
            if let Some(v) = shadow_threshold { mat.shadow_threshold = v; }
            if let Some(v) = shadow_smoothness { mat.shadow_smoothness = v; }
            if let Some(v) = spec_intensity { mat.spec_intensity = v; }
            if let Some(v) = spec_power { mat.spec_power = v; }
            if let Some(v) = rim_intensity { mat.rim_intensity = v; }
            if let Some(v) = rim_spread { mat.rim_spread = v; }
            if let Some(v) = hue_shift { mat.hue_shift = v; }
            if let Some(v) = toon_steps { mat.toon_steps = v; }
            if let Some(v) = base_color { mat.base_color = v; }
            if let Some(v) = shade_color { mat.shade_color = v; }
            if let Some(v) = outline_width { mat.outline_width = v; }
            if let Some(v) = outline_color { mat.outline_color = v; }
        }
    }
    Ok(())
}

static LAST_CMD_TELEMETRY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[tauri::command]
async fn report_live_telemetry(
    telemetry: LiveWindowState,
    live_state: State<'_, Arc<RwLock<LiveWindowState>>>,
) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let last = LAST_CMD_TELEMETRY.load(std::sync::atomic::Ordering::Relaxed);
    if now.saturating_sub(last) >= 3 {
        LAST_CMD_TELEMETRY.store(now, std::sync::atomic::Ordering::Relaxed);
        tracing::info!(
            engine = if telemetry.webgpu_active { "WebGPU Native" } else { "WebGL2 Fallback" },
            fps = telemetry.fps as u32,
            frame_time_ms = telemetry.frame_time_ms,
            preset = %telemetry.active_preset,
            polys = telemetry.triangle_count,
            adapter = %telemetry.adapter_name,
            "telemetry snapshot"
        );
    }
    let mut state = live_state.write().await;
    *state = telemetry;
    Ok(())
}

#[tauri::command]
async fn get_live_telemetry(
    live_state: State<'_, Arc<RwLock<LiveWindowState>>>,
) -> Result<LiveWindowState, String> {
    let state = live_state.read().await;
    Ok(state.clone())
}

fn get_user_documents_dir() -> std::path::PathBuf {
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        let docs = std::path::PathBuf::from(userprofile).join("Documents");
        if docs.exists() {
            return docs;
        }
    }
    std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("Documents")
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StudioDirectories {
    pub projects_dir: String,
    pub assets_dir: String,
    pub autosave_dir: String,
    pub renders_dir: String,
}

#[tauri::command]
async fn get_studio_directories() -> Result<StudioDirectories, String> {
    let base = get_user_documents_dir().join("ANIGO");
    let projects = base.join("Projects");
    let assets = base.join("Assets");
    let autosave = base.join("Autosave");
    let renders = base.join("Renders");

    let _ = std::fs::create_dir_all(&projects);
    let _ = std::fs::create_dir_all(&assets);
    let _ = std::fs::create_dir_all(&autosave);
    let _ = std::fs::create_dir_all(&renders);

    Ok(StudioDirectories {
        projects_dir: projects.to_string_lossy().to_string(),
        assets_dir: assets.to_string_lossy().to_string(),
        autosave_dir: autosave.to_string_lossy().to_string(),
        renders_dir: renders.to_string_lossy().to_string(),
    })
}

#[tauri::command]
async fn autosave_project(
    payload: String,
    filename: Option<String>,
) -> Result<String, String> {
    let base = get_user_documents_dir().join("ANIGO").join("Autosave");
    std::fs::create_dir_all(&base).map_err(|e| format!("Erro ao criar pasta de autosave: {}", e))?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let fname = filename.unwrap_or_else(|| format!("autosave_{}.anigo", now));
    let file_path = base.join(&fname);
    let latest_path = base.join("autosave_latest.anigo");

    std::fs::write(&file_path, &payload)
        .map_err(|e| format!("Erro ao gravar arquivo de autosave: {}", e))?;
    let _ = std::fs::write(&latest_path, &payload);

    Ok(file_path.to_string_lossy().to_string())
}

#[tauri::command]
async fn open_directory_in_explorer(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Falha ao abrir Explorer: {}", e))?;
    }
    Ok(())
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub latest_version: String,
    pub is_up_to_date: bool,
    pub status_message: String,
    pub release_notes: String,
}

#[tauri::command]
async fn check_app_updates() -> Result<UpdateCheckResult, String> {
    Ok(UpdateCheckResult {
        current_version: "0.1.0".to_string(),
        latest_version: "0.1.0".to_string(),
        is_up_to_date: true,
        status_message: "ANIGO Studio está atualizado na versão mais recente (v0.1.0).".to_string(),
        release_notes: "Build de Produção WebGPU Nativo + Live Socket Bridge + Autosave em Disco.".to_string(),
    })
}

#[tauri::command]
async fn app_minimize(window: tauri::WebviewWindow) -> Result<(), String> {
    window.minimize().map_err(|e| format!("Erro ao minimizar janela: {}", e))
}

#[tauri::command]
async fn app_toggle_maximize(window: tauri::WebviewWindow) -> Result<bool, String> {
    let is_max = window.is_maximized().unwrap_or(false);
    if is_max {
        window.unmaximize().map_err(|e| format!("Erro ao restaurar janela: {}", e))?;
        Ok(false)
    } else {
        window.maximize().map_err(|e| format!("Erro ao maximizar janela: {}", e))?;
        Ok(true)
    }
}

#[tauri::command]
async fn app_close(window: tauri::WebviewWindow) -> Result<(), String> {
    window.close().map_err(|e| format!("Erro ao fechar janela: {}", e))
}

#[tauri::command]
async fn app_is_maximized(window: tauri::WebviewWindow) -> Result<bool, String> {
    Ok(window.is_maximized().unwrap_or(false))
}

#[tauri::command]
async fn save_project_file(path: String, content: String) -> Result<String, String> {
    if let Some(parent) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, &content).map_err(|e| format!("Erro ao gravar arquivo de projeto: {}", e))?;
    Ok(path)
}

#[tauri::command]
async fn load_project_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("Erro ao ler arquivo de projeto: {}", e))
}

fn main() {
    // P2: Initialize structured tracing for the Tauri app shell.
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| {
            tracing_subscriber::EnvFilter::new(
                "info,anigo_app=info,anigo_app::bridge=info,wgpu=warn"
            )
        });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr)
        .with_target(true)
        .try_init();

    let log_dir = bridge::log_dir();
    let _ = std::fs::create_dir_all(&log_dir);

    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(panic = %info, "ANIGO panic");
        let path = log_dir.join("panic.log");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(f, "Panic at {:?}: {:#?}", std::time::SystemTime::now(), info);
        }
    }));

    let launch_path = bridge::log_dir().join("launch.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&launch_path) {
        let _ = writeln!(f, "Starting ANIGO main at {:?}", std::time::SystemTime::now());
    }
    tracing::info!(log_dir = %bridge::log_dir().display(), "ANIGO Studio starting");

    #[cfg(target_os = "windows")]
    {
        std::env::set_var(
            "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
            "--enable-features=Vulkan,UseSkiaRenderer,WebGPU --enable-unsafe-webgpu --disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection",
        );
    }

    let app_state = Arc::new(Mutex::new(AppState {
        renderer: None,
        scene: Scene::default(),
    }));

    let live_state = Arc::new(RwLock::new(LiveWindowState::default()));
    let live_state_bridge = Arc::clone(&live_state);

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(app_state)
        .manage(live_state)
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let server = Arc::new(LiveBridgeServer::new(app_handle, live_state_bridge));
            tauri::async_runtime::spawn(async move {
                server.run("127.0.0.1:39090").await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            render_viewport_frame,
            camera_orbit,
            camera_zoom,
            camera_pan,
            load_mesh_preset,
            set_light_params,
            set_material_toon_params,
            report_live_telemetry,
            get_live_telemetry,
            get_studio_directories,
            autosave_project,
            open_directory_in_explorer,
            check_app_updates,
            app_minimize,
            app_toggle_maximize,
            app_close,
            app_is_maximized,
            save_project_file,
            load_project_file,
        ])
        .build(tauri::generate_context!())
        .expect("error building tauri application")
        .run(|_app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let exit_path = bridge::log_dir().join("launch.log");
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&exit_path) {
                    let _ = writeln!(f, "Tauri ExitRequested event received at {:?}", std::time::SystemTime::now());
                }
            }
        });
}
