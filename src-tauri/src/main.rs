// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge;
mod core_session;

use std::io::Write;
use std::sync::Arc;
use base64::Engine;
use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};
use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::sync::{Mutex, RwLock};

use anigo_core::scene::Scene;
use anigo_renderer::HeadlessRenderer;
use bridge::{LiveBridgeServer, LiveWindowState};

/// P0 §7/§8: resposta da exportação de GLB (o arquivo é escrito pelo **núcleo**;
/// o frontend só escolhe o caminho e confere o manifesto).
#[derive(Serialize, Deserialize)]
pub struct ExportGlbResponse {
    pub glb_path: String,
    pub manifest_path: String,
    pub glb_bytes: u64,
    pub glb_checksum: String,
    pub manifest: serde_json::Value,
}

/// Resposta da exportação de frame (PNG renderizado pelo renderer canônico).
#[derive(Serialize, Deserialize)]
pub struct ExportFrameResponse {
    pub image_path: String,
    pub manifest_path: String,
    pub image_bytes: u64,
    pub adapter_name: String,
    pub backend: String,
    pub draw_calls: u32,
    pub triangle_count: usize,
    pub render_time_ms: f64,
    pub manifest: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
pub struct ViewportFrameResponse {
    pub image_base64: String,
    pub render_time_ms: f64,
    pub draw_calls: u32,
    pub triangle_count: usize,
    pub adapter_name: String,
    pub backend: String,
}

/// Issue #11: status do device headless para a UI (rodapé de notificação) —
/// disponibilidade, modo de apresentação, resumo de diagnósticos e os últimos
/// erros interceptados pelo canal MPSC de `on_uncaptured_error`.
#[derive(Serialize, Deserialize)]
pub struct HeadlessDeviceStatus {
    pub available: bool,
    pub adapter_name: Option<String>,
    pub backend: Option<String>,
    /// Modo de apresentação em vigor: `"fifo"`, `"immediate"` ou `"mailbox"`.
    pub presentation_mode: String,
    pub diagnostics_errors: usize,
    pub diagnostics_warnings: usize,
    /// Últimos erros de device interceptados (sem crash), mais recentes no fim.
    pub recent_device_errors: Vec<anigo_renderer::UncapturedErrorRecord>,
    /// Tempo (ms) da última `recreate_device_and_swapchain` bem-sucedida.
    pub last_recovery_ms: Option<f64>,
}

pub struct AppState {
    pub renderer: Option<HeadlessRenderer>,
    pub scene: Scene,
    /// Sessão canônica (P0 §7.4/§7.5): dona do `ProjectState`, do histórico de
    /// comandos e da geometria deformada que o snapshot entrega ao frontend.
    pub session: core_session::CoreSession,
    /// Espelho (`scene`) desatualizado em relação ao núcleo: a geometria é
    /// reconstruída sob demanda, no próximo frame — mexer na câmera ou no
    /// material não recalcula 157 canais de morph à toa.
    pub scene_dirty: bool,
    /// Issue #11: tempo (ms) da última recuperação de device (notificação da UI).
    pub last_recovery_ms: Option<f64>,
}

impl AppState {
    fn new() -> Self {
        Self {
            renderer: None,
            scene: Scene::default(),
            session: core_session::CoreSession::new(),
            scene_dirty: false,
            last_recovery_ms: None,
        }
    }

    /// Espelha câmera/luz/fundo/material (barato, sem tocar em geometria).
    fn sync_scene_view(&mut self) {
        self.scene.camera = self.session.project().scene.camera.camera.clone();
        if let Some(slot) = self.session.project().scene.lights.first() {
            self.scene.light = slot.light.clone();
        }
        self.scene.background_color = self.session.project().render.background_color;
        let material = self.session.project().character_material().map(|entry| entry.material.clone());
        for node in &mut self.scene.nodes {
            node.material = material.clone();
        }
    }

    /// Aplica um comando canônico e marca o que precisa ser re-sincronizado.
    fn apply_command(
        &mut self,
        command: anigo_core::command::Command,
    ) -> Result<anigo_core::command::CommandOutcome, String> {
        let scope = command.scope();
        let outcome = self.session.apply(command)?;
        self.sync_scene_view();
        if scope.requires_static_rebuild()
            || matches!(scope, anigo_core::command::ChangeScope::Deformation)
        {
            self.scene_dirty = true;
        }
        Ok(outcome)
    }

    /// Espelha o estado canônico na `Scene` do renderer headless (frame PNG,
    /// exportação): a geometria desenhada pelo núcleo é a deformada por ele.
    fn sync_scene_from_session(&mut self) {
        if let Err(error) = self.session.sync_scene(&mut self.scene) {
            tracing::warn!(error = %error, "core session → scene sync failed");
            return;
        }
        self.scene_dirty = false;
    }

    /// Garante que o espelho está atualizado antes de renderizar.
    fn ensure_scene_current(&mut self) {
        if self.scene_dirty {
            self.sync_scene_from_session();
        }
    }
}

/// Aplica um comando canônico e reespelha a `Scene` do renderer headless.
///
/// P0 §7.4: existe **um** caminho de escrita do estado — `CoreSession` — e um só
/// dono da geometria. Estes wrappers tipados existem para call sites antigos do
/// frontend; eles não guardam estado próprio.
fn apply_session_command(
    state: &mut AppState,
    command: anigo_core::command::Command,
) -> Result<(), String> {
    state.apply_command(command).map(|_| ())
}

#[tauri::command]
async fn render_viewport_frame(
    width: u32,
    height: u32,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<ViewportFrameResponse, String> {
    let mut state = state.lock().await;
    // P0 §7.4: o frame desenhado é a geometria deformada pelo núcleo.
    state.ensure_scene_current();
    if state.renderer.is_none() {
        state.renderer = HeadlessRenderer::new().await.ok();
    }
    let renderer = state
        .renderer
        .as_mut()
        .ok_or_else(|| "Headless renderer not available on this platform".to_string())?;

    let width = width.max(64);
    let height = height.max(64);
    // Issue #11: falha de quadro dispara a recuperação do device (re-solicita
    // adapter/device, re-instancia PSOs e re-aloca os buffers canônicos)
    // antes de abrir mão do frame — sem crash do processo Tauri.
    let scene = &state.scene;
    let (image_buf, metrics) = match renderer.render_scene(scene, width, height).await {
        Ok(ok) => ok,
        Err(first_error) => match renderer.recreate_device_and_swapchain().await {
            Ok(report) => {
                state.last_recovery_ms = Some(report.duration_ms);
                tracing::info!(
                    target: "anigo::device",
                    adapter = %report.adapter_name,
                    duration_ms = report.duration_ms,
                    "headless device recuperado após falha de frame"
                );
                renderer
                    .render_scene(scene, width, height)
                    .await
                    .map_err(|second_error| {
                        format!(
                            "Render error (device recovery took {:.1} ms, then failed): {:#}",
                            report.duration_ms, second_error
                        )
                    })?
            }
            Err(recovery_error) => {
                return Err(format!(
                    "Render error (device recovery also failed: {:#}): {:#}",
                    recovery_error, first_error
                ))
            }
        },
    };

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
    // Mesma `Camera::orbit` do núcleo, agora pela autoridade dos dados (comando
    // canônico → undo/redo/persistência valem para a câmera também).
    apply_session_command(
        &mut state,
        anigo_core::command::Command::OrbitCamera { azimuth, elevation },
    )
}

#[tauri::command]
async fn camera_zoom(
    factor: f32,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let mut state = state.lock().await;
    apply_session_command(
        &mut state,
        anigo_core::command::Command::ZoomCamera { factor },
    )
}

#[tauri::command]
async fn camera_pan(
    dx: f32,
    dy: f32,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let mut state = state.lock().await;
    apply_session_command(
        &mut state,
        anigo_core::command::Command::PanCamera { dx, dy },
    )
}

#[tauri::command]
async fn load_mesh_preset(
    preset: String,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<String, String> {
    let mut state = state.lock().await;
    // P0 §7.4: o preset é um comando canônico — a malha (base canônica deformada
    // pelo núcleo ou primitiva) vem do `ProjectState`, nunca de uma cópia local.
    let parsed: anigo_core::command::MeshPreset = serde_json::from_value(
        serde_json::Value::String(preset.to_lowercase()),
    )
    .map_err(|_| format!("preset desconhecido: {}", preset))?;
    apply_session_command(
        &mut state,
        anigo_core::command::Command::LoadMeshPreset { preset: parsed },
    )?;
    Ok(format!("Loaded preset {}", preset))
}

#[tauri::command]
async fn set_light_params(
    direction: [f32; 3],
    intensity: f32,
    color: Option<[f32; 3]>,
    shadow_color: Option<[f32; 3]>,
    ambient_intensity: Option<f32>,
    shadow_saturation: Option<f32>,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let mut state = state.lock().await;
    let d = glam::Vec3::from_array(direction).normalize();
    let command = anigo_core::command::Command::SetLight {
        light_id: None,
        direction: Some([d.x, d.y, d.z]),
        color,
        intensity: Some(intensity),
        shadow_color,
        ambient_intensity,
        shadow_saturation,
        ambient_sky: None,
        ambient_ground: None,
    };
    apply_session_command(&mut state, command)?;
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
    specular_color: Option<[f32; 4]>,
    specular_softness: Option<f32>,
    specular_offset: Option<f32>,
    rim_color: Option<[f32; 4]>,
    outline_opacity: Option<f32>,
    outline_smoothness: Option<f32>,
    outline_depth_bias: Option<f32>,
    shadow_saturation: Option<f32>,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let mut state = state.lock().await;
    // P0 §7.4: o material é um patch de comando canônico — o núcleo valida,
    // guarda o inverso (undo) e é a fonte do material que o render desenha.
    let patch = anigo_core::command::MaterialPatch {
        base_color,
        shade_color,
        outline_color,
        outline_width,
        shadow_threshold,
        shadow_smoothness,
        spec_intensity,
        spec_power,
        specular_color,
        specular_softness,
        specular_offset,
        rim_color,
        rim_intensity,
        rim_spread,
        hue_shift,
        toon_steps,
        outline_opacity,
        outline_smoothness,
        outline_depth_bias,
        ..Default::default()
    };

    let mut commands = Vec::new();
    if !patch.is_empty() {
        commands.push(anigo_core::command::Command::SetMaterialParams {
            material_id: None,
            patch,
        });
    }
    if let Some(value) = shadow_saturation {
        commands.push(anigo_core::command::Command::SetLight {
            light_id: None,
            direction: None,
            color: None,
            intensity: None,
            shadow_color: None,
            ambient_intensity: None,
            shadow_saturation: Some(value),
            ambient_sky: None,
            ambient_ground: None,
        });
    }
    if commands.is_empty() {
        return Ok(());
    }
    let command = if commands.len() == 1 {
        commands.remove(0)
    } else {
        anigo_core::command::Command::Batch { commands }
    };
    apply_session_command(&mut state, command)
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
    let mut state = live_state.read().await.clone();
    // P1-06: o diagnóstico do renderer nativo (headless + contrato) entra na
    // telemetria como dado medido aqui, não como o que a UI supostamente viu.
    let summary = anigo_renderer::diagnostics_summary();
    state.native_diagnostics_errors = u32::try_from(summary.errors).unwrap_or(u32::MAX);
    state.native_diagnostics_warnings = u32::try_from(summary.warnings).unwrap_or(u32::MAX);
    state.native_diagnostic_codes = summary.codes;
    state.native_contract_valid = anigo_renderer::render_contract::contract_is_valid();
    Ok(state)
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
    /// P0 §8: pasta canônica de exportação (GLB/PNG + manifesto ao lado).
    pub exports_dir: String,
}

#[tauri::command]
async fn get_studio_directories() -> Result<StudioDirectories, String> {
    let base = get_user_documents_dir().join("ANIGO");
    let projects = base.join("Projects");
    let assets = base.join("Assets");
    let autosave = base.join("Autosave");
    let renders = base.join("Renders");
    let exports = base.join("Exports");

    let _ = std::fs::create_dir_all(&projects);
    let _ = std::fs::create_dir_all(&assets);
    let _ = std::fs::create_dir_all(&autosave);
    let _ = std::fs::create_dir_all(&renders);
    let _ = std::fs::create_dir_all(&exports);

    Ok(StudioDirectories {
        projects_dir: projects.to_string_lossy().to_string(),
        assets_dir: assets.to_string_lossy().to_string(),
        autosave_dir: autosave.to_string_lossy().to_string(),
        renders_dir: renders.to_string_lossy().to_string(),
        exports_dir: base.join("Exports").to_string_lossy().to_string(),
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
// Fora do Windows o parâmetro não é usado (não há Explorer) — mas a assinatura
// do comando é a mesma, porque é o contrato com o frontend.
#[cfg_attr(not(target_os = "windows"), allow(unused_variables))]
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

// ---------------------------------------------------------------------------
// P0 §7.4/§7.5 — superfície canônica do núcleo
//
// O frontend não deforma nada: ele envia comandos canônicos e recebe snapshots.
// Toda validação acontece em `anigo_core` (mesmos tipos da persistência).
// ---------------------------------------------------------------------------

/// Aplica um comando canônico (o mesmo JSON que o `CommandHistory` valida).
#[tauri::command]
async fn core_apply_command(
    command: serde_json::Value,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<anigo_core::command::CommandOutcome, String> {
    let mut state = state.lock().await;
    let parsed = anigo_core::command::Command::from_json(&command.to_string())
        .map_err(|error| error.to_string())?;
    state.apply_command(parsed)
}

/// Desfaz o último comando no núcleo (a UI só renderiza o resultado).
#[tauri::command]
async fn core_undo_command(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<anigo_core::command::CommandOutcome, String> {
    let mut state = state.lock().await;
    let outcome = state.session.undo()?;
    state.sync_scene_view();
    state.scene_dirty = true;
    Ok(outcome)
}

/// Refaz o último comando desfeito no núcleo.
#[tauri::command]
async fn core_redo_command(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<anigo_core::command::CommandOutcome, String> {
    let mut state = state.lock().await;
    let outcome = state.session.redo()?;
    state.sync_scene_view();
    state.scene_dirty = true;
    Ok(outcome)
}

/// Snapshot canônico para o viewport.
///
/// `client_static_revision` é a cache key do cliente: quando bate com a do
/// núcleo, a parte estática (malha + deltas) nem é serializada.
#[tauri::command]
async fn core_snapshot(
    include_static: bool,
    client_static_revision: Option<u64>,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<anigo_core::snapshot::CoreSnapshot, String> {
    let mut state = state.lock().await;
    state.session.snapshot(include_static, client_static_revision)
}

/// Revisões + profundidade de undo/redo autorais do núcleo.
#[tauri::command]
async fn core_history_state(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<serde_json::Value, String> {
    let state = state.lock().await;
    Ok(state.session.history_report())
}

/// Carrega um documento canônico (envelope/arquivo) na sessão.
#[tauri::command]
async fn core_load_document(
    document: String,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<serde_json::Value, String> {
    let mut state = state.lock().await;
    state.session.load_document(&document)?;
    // Documento novo ⇒ geometria nova: reconstrói o espelho agora.
    state.scene_dirty = true;
    state.sync_scene_from_session();
    Ok(state.session.history_report())
}

/// Devolve o documento canônico serializado (fonte da verdade ao salvar).
/// Exporta o GLB canônico (malha deformada pelo núcleo) + o manifesto.
///
/// A pasta de exportação aparece no erro quando o caminho é inválido — nada de
/// falha silenciosa com arquivo ausente (P1-01).
#[tauri::command]
async fn core_export_glb(
    path: String,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<ExportGlbResponse, String> {
    let mut state = state.lock().await;
    let (glb_path, manifest_path, glb_bytes, glb_checksum) =
        state.session.export_glb_to_file(&path).map_err(|error| {
            format!(
                "falha ao exportar GLB em '{}': {}",
                std::path::Path::new(&path)
                    .parent()
                    .map(|parent| parent.to_string_lossy().to_string())
                    .unwrap_or_else(|| ".".to_string()),
                error
            )
        })?;
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path)
            .map_err(|error| format!("manifesto não legível em '{manifest_path}': {error}"))?,
    )
    .map_err(|error| format!("manifesto inválido em '{manifest_path}': {error}"))?;

    tracing::info!(
        target: "anigo::export",
        glb = %glb_path,
        bytes = glb_bytes,
        checksum = %glb_checksum,
        "canonical GLB exported"
    );

    Ok(ExportGlbResponse {
        glb_path,
        manifest_path,
        glb_bytes,
        glb_checksum,
        manifest,
    })
}

/// Renderiza o frame de exportação com o renderer canônico e grava PNG +
/// manifesto (mesma geometria, mesmos shaders e mesmo contrato do viewport).
#[tauri::command]
async fn core_export_frame(
    path: String,
    width: u32,
    height: u32,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<ExportFrameResponse, String> {
    let mut state = state.lock().await;
    state.ensure_scene_current();

    if state.renderer.is_none() {
        state.renderer = HeadlessRenderer::new().await.ok();
    }
    let renderer = state
        .renderer
        .as_mut()
        .ok_or_else(|| "Headless renderer not available on this platform".to_string())?;

    let width = width.max(64);
    let height = height.max(64);
    // Issue #11: mesmo tratamento de recuperação do device do frame do
    // viewport — a exportação não pode falhar por crash de driver.
    let scene = &state.scene;
    let (image_buf, metrics) = match renderer.render_scene(scene, width, height).await {
        Ok(ok) => ok,
        Err(first_error) => match renderer.recreate_device_and_swapchain().await {
            Ok(report) => {
                state.last_recovery_ms = Some(report.duration_ms);
                tracing::info!(
                    target: "anigo::device",
                    adapter = %report.adapter_name,
                    duration_ms = report.duration_ms,
                    "headless device recuperado após falha de exportação"
                );
                renderer
                    .render_scene(scene, width, height)
                    .await
                    .map_err(|second_error| {
                        format!(
                            "Render error (device recovery took {:.1} ms, then failed): {:#}",
                            report.duration_ms, second_error
                        )
                    })?
            }
            Err(recovery_error) => {
                return Err(format!(
                    "Render error (device recovery also failed: {:#}): {:#}",
                    recovery_error, first_error
                ))
            }
        },
    };

    let render_info = anigo_core::export::ExportRenderInfo {
        width,
        height,
        color_format: format!(
            "{:?}",
            anigo_renderer::render_contract::offscreen_color_format()
        ),
        depth_format: format!("{:?}", anigo_renderer::render_contract::depth_format()),
        msaa_samples: anigo_renderer::render_contract::msaa_sample_count(),
        render_passes: anigo_renderer::render_contract::render_pass_order()
            .into_iter()
            .map(|name| name.to_string())
            .collect(),
        clear_source: anigo_renderer::render_contract::clear_color_source().to_string(),
        clear_color: state.scene.background_color,
        adapter_name: metrics.adapter_name.clone(),
        backend: metrics.backend.clone(),
        render_time_ms: metrics.render_time_ms,
    };

    let manifest = state
        .session
        .export_frame_manifest(render_info)
        .map_err(|error| format!("manifesto da exportação de frame falhou: {error}"))?;
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|error| format!("manifesto não serializou: {error}"))?;

    let image_path = std::path::PathBuf::from(&path);
    if let Some(parent) = image_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("Erro ao criar pasta de export: {error}"))?;
        }
    }

    let mut png_bytes = Vec::new();
    let encoder = PngEncoder::new(&mut png_bytes);
    encoder
        .write_image(&image_buf, width, height, ColorType::Rgba8.into())
        .map_err(|e| format!("PNG encode error: {:#}", e))?;
    std::fs::write(&image_path, &png_bytes)
        .map_err(|error| format!("Erro ao gravar o PNG: {error}"))?;

    let manifest_path = core_session::manifest_path_for(&image_path);
    std::fs::write(&manifest_path, &manifest_json)
        .map_err(|error| format!("Erro ao gravar o manifesto: {error}"))?;

    tracing::info!(
        target: "anigo::export",
        png = %image_path.to_string_lossy(),
        width,
        height,
        triangles = metrics.triangle_count,
        "canonical export frame rendered"
    );

    Ok(ExportFrameResponse {
        image_path: image_path.to_string_lossy().to_string(),
        manifest_path: manifest_path.to_string_lossy().to_string(),
        image_bytes: png_bytes.len() as u64,
        adapter_name: metrics.adapter_name,
        backend: metrics.backend,
        draw_calls: metrics.draw_calls,
        triangle_count: metrics.triangle_count,
        render_time_ms: metrics.render_time_ms,
        manifest: serde_json::from_str(&manifest_json)
            .map_err(|error| format!("manifesto inválido: {error}"))?,
    })
}

/// Issue #11: telemetria de tolerância a falhas do device headless — o rodapé
/// da UI reporta o evento de perda/recuperação e o tempo de recuperação.
#[tauri::command]
async fn headless_device_status(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<HeadlessDeviceStatus, String> {
    let state = state.lock().await;
    let summary = anigo_renderer::diagnostics_summary();
    Ok(HeadlessDeviceStatus {
        available: state.renderer.is_some(),
        adapter_name: state
            .renderer
            .as_ref()
            .map(|renderer| renderer.adapter_info.name.clone()),
        backend: state
            .renderer
            .as_ref()
            .map(|renderer| format!("{:?}", renderer.adapter_info.backend)),
        presentation_mode: state
            .renderer
            .as_ref()
            .map(|renderer| renderer.presentation_mode().as_str().to_string())
            .unwrap_or_else(|| "fifo".to_string()),
        diagnostics_errors: summary.errors,
        diagnostics_warnings: summary.warnings,
        recent_device_errors: anigo_renderer::recent_errors(),
        last_recovery_ms: state.last_recovery_ms,
    })
}

#[tauri::command]
async fn core_document(state: State<'_, Arc<Mutex<AppState>>>) -> Result<String, String> {
    let state = state.lock().await;
    state.session.document()
}

/// Malha deformada pelo núcleo (base + morphs ativos).
#[tauri::command]
async fn core_deformed_mesh(
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<anigo_core::mesh::Mesh, String> {
    let mut state = state.lock().await;
    state.session.deformed_mesh()
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

    let app_state = Arc::new(Mutex::new(AppState::new()));

    let live_state = Arc::new(RwLock::new(LiveWindowState::default()));
    let live_state_bridge = Arc::clone(&live_state);

    let built = tauri::Builder::default()
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
            core_apply_command,
            core_undo_command,
            core_redo_command,
            core_snapshot,
            core_history_state,
            core_load_document,
            core_document,
            core_deformed_mesh,
            core_export_glb,
            core_export_frame,
            headless_device_status,
        ])
        .build(tauri::generate_context!());

    // P1-01: falha ao construir a aplicação não é mais um `panic!` no caminho
    // crítico — vira uma mensagem explícita no log e código de saída != 0.
    let app = match built {
        Ok(app) => app,
        Err(error) => {
            let message = format!("erro ao construir a aplicação Tauri: {error}");
            tracing::error!(target: "anigo::startup", "{message}");
            eprintln!("[ANIGO] {message}");
            std::process::exit(1);
        }
    };

    app.run(|_app_handle, event| {
        if let tauri::RunEvent::ExitRequested { .. } = event {
            let exit_path = bridge::log_dir().join("launch.log");
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&exit_path) {
                let _ = writeln!(f, "Tauri ExitRequested event received at {:?}", std::time::SystemTime::now());
            }
        }
    });
}
