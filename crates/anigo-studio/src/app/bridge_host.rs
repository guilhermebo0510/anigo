//! Lado do studio da bridge local (T-2.13): recebe, no início de cada quadro, os pedidos que o `anigo-bridge`
//! (threads de conexão) enfileirou, executa-os na thread da UI com acesso ao `StudioApp` e responde.
//!
//! Toda mutação passa pela MESMA `CoreSession` do usuário (`APPLY_COMMAND`/`UNDO`/`REDO`/`PLACE_ASSET`/...), então
//! entra no histórico, reflete ao vivo na UI (a thread da bridge acorda a UI com `request_repaint`) e o Ctrl+Z do
//! usuário a desfaz. Desligada por padrão (`--bridge` / `ANIGO_BRIDGE=1`). Protocolo e segurança: `anigo-bridge`
//! e `docs/native/MCP_BRIDGE_NATIVA.md`.
//!
//! Ações aqui (as do núcleo — `GET_PROJECT/GET_SNAPSHOT/APPLY_COMMAND/UNDO/REDO/GET_LOG` — vêm de
//! `anigo_bridge::core_ops`): `GET_STATUS`, `GET_WORKSPACE`, `SET_WORKSPACE`, `GET_SELECTION`, `LIST_ASSETS`,
//! `PLACE_ASSET`, `RENDER_FRAME`, `RENDER_STATUS`, `SCREENSHOT` e os atalhos legados `ORBIT/ZOOM/PAN/SET_CAMERA/
//! SET_LIGHT` (viram `Command`s do núcleo).

use super::render_ui::JobStatus;
use super::StudioApp;
use crate::cli::Cli;
use crate::i18n::Lang;
use crate::workspace::WorkspaceId;
use anigo_bridge::protocol::{code, BridgeError, BridgeResult};
use anigo_bridge::{core_ops, discovery, BridgeServer, HostQueue, HostRequest, Reply, ServerConfig};
use anigo_core::command::Command;
use base64::Engine as _;
use serde_json::{json, Value};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Máximo de pedidos tratados por quadro (a UI nunca fica refém de um cliente barulhento).
const MAX_PER_FRAME: usize = 16;
/// Prazo (menor que o da ponte) para pedidos adiados esperando a Biblioteca/ativos.
const DEFER_LIMIT: Duration = Duration::from_secs(12);
const MAX_INLINE_IMAGE: u64 = 12 * 1024 * 1024;

/// Marca do pedido de screenshot dentro do `egui::UserData` (o `--screenshot` do CLI ignora estes).
pub(crate) struct BridgeShot(pub u64);

struct PendingShot {
    id: u64,
    reply: Reply,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
    whole_window: bool,
    max_width: Option<u32>,
    asked: Instant,
}

struct Deferred {
    req: HostRequest,
    since: Instant,
}

pub(crate) struct BridgeHost {
    server: BridgeServer,
    queue: HostQueue,
    shots: Vec<PendingShot>,
    deferred: Vec<Deferred>,
    next_shot: u64,
    /// Retângulo (em pontos) onde o workspace ativo desenha a viewport 3D; atualizado a cada quadro.
    pub viewport_rect: egui::Rect,
}

impl BridgeHost {
    /// Sobe a bridge se `--bridge` ou `ANIGO_BRIDGE=1|true`. Falha ao subir não derruba o app.
    pub(crate) fn start(cli: &Cli, ctx: &egui::Context) -> Option<Self> {
        let env_on = std::env::var("ANIGO_BRIDGE").map(|v| matches!(v.as_str(), "1" | "true" | "yes")).unwrap_or(false);
        if !cli.bridge && !env_on {
            return None;
        }
        let wake_ctx = ctx.clone();
        let (backend, queue) = anigo_bridge::channel(64, Arc::new(move || wake_ctx.request_repaint()));
        let path = cli.bridge_discovery.as_ref().map(std::path::PathBuf::from).unwrap_or_else(discovery::default_path);
        let cfg = ServerConfig { port: cli.bridge_port.unwrap_or(0), discovery: Some(path.clone()), ..Default::default() };
        match BridgeServer::start(cfg, Arc::new(backend)) {
            Ok(server) => {
                eprintln!("bridge: escutando em {} (descoberta: {})", server.addr(), path.display());
                Some(Self { server, queue, shots: Vec::new(), deferred: Vec::new(), next_shot: 1, viewport_rect: egui::Rect::NOTHING })
            }
            Err(e) => {
                eprintln!("bridge: falha ao iniciar: {e}");
                tracing::error!("bridge indisponível: {e}");
                None
            }
        }
    }

    pub(crate) fn addr(&self) -> String {
        self.server.addr()
    }

    pub(crate) fn requests_total(&self) -> u64 {
        self.server.metrics().requests_total.load(Relaxed)
    }
}

fn tooltip(lang: Lang, addr: &str, requests: u64) -> String {
    match lang {
        Lang::PtBr => format!("Bridge de automação ativa em {addr} (somente loopback, com token). Pedidos atendidos: {requests}. Ações entram no histórico e podem ser desfeitas."),
        Lang::En => format!("Automation bridge listening on {addr} (loopback only, token-protected). Requests served: {requests}. Actions enter the history and can be undone."),
    }
}

fn as_obj(v: &Value) -> Result<&serde_json::Map<String, Value>, BridgeError> {
    match v {
        Value::Null => {
            static EMPTY: std::sync::OnceLock<serde_json::Map<String, Value>> = std::sync::OnceLock::new();
            Ok(EMPTY.get_or_init(Default::default))
        }
        Value::Object(m) => Ok(m),
        _ => Err(BridgeError::invalid_params("params deve ser um objeto")),
    }
}

fn opt_str<'a>(m: &'a serde_json::Map<String, Value>, k: &str) -> Result<Option<&'a str>, BridgeError> {
    match m.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(BridgeError::invalid_params(format!("'{k}' deve ser texto"))),
    }
}

fn opt_u64(m: &serde_json::Map<String, Value>, k: &str) -> Result<Option<u64>, BridgeError> {
    match m.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| BridgeError::invalid_params(format!("'{k}' deve ser inteiro não negativo"))),
    }
}

pub(super) fn vec3(v: &Value, name: &str) -> Result<glam::Vec3, BridgeError> {
    let a = v.as_array().filter(|a| a.len() == 3).ok_or_else(|| BridgeError::invalid_params(format!("'{name}' deve ter 3 números")))?;
    let mut o = [0f32; 3];
    for (i, x) in a.iter().enumerate() {
        let f = x.as_f64().ok_or_else(|| BridgeError::invalid_params(format!("'{name}[{i}]' não é número")))?;
        if !f.is_finite() || f.abs() > 1.0e6 {
            return Err(BridgeError::invalid_params(format!("'{name}[{i}]' fora do intervalo")));
        }
        o[i] = f as f32;
    }
    Ok(glam::Vec3::from(o))
}

/// Ação legada -> `Command` (os nomes dos parâmetros já coincidem com os campos do comando).
fn legacy_command(action: &str, params: &Value) -> Option<Result<Command, BridgeError>> {
    let kind = match action {
        "ORBIT" => "orbit_camera",
        "ZOOM" => "zoom_camera",
        "PAN" => "pan_camera",
        "SET_CAMERA" => "set_camera",
        "SET_LIGHT" => "set_light",
        _ => return None,
    };
    Some((|| {
        let mut m = as_obj(params)?.clone();
        m.insert("kind".into(), json!(kind));
        core_ops::parse_command(&Value::Object(m))
    })())
}

impl StudioApp {
    /// Chamado uma vez por quadro, no começo (antes de desenhar): atende pedidos e conclui screenshots.
    pub(crate) fn bridge_tick(&mut self, ctx: &egui::Context) {
        if self.bridge.is_none() {
            return;
        }
        // jobs de render em andamento precisam drenar eventos mesmo fora do workspace Render
        if self.render_ui.running() {
            self.render_poll(ctx);
            ctx.request_repaint_after(Duration::from_millis(60));
        }
        let mut work: Vec<(HostRequest, Instant)> = Vec::new();
        if let Some(b) = self.bridge.as_mut() {
            // pedidos adiados primeiro (mantêm a ordem de chegada e o instante original)
            work.extend(b.deferred.drain(..).filter(|d| !d.req.is_cancelled()).map(|d| (d.req, d.since)));
            let now = Instant::now();
            work.extend(b.queue.drain(MAX_PER_FRAME).into_iter().map(|r| (r, now)));
        }
        for (r, since) in work {
            self.bridge_dispatch(ctx, r, since);
        }
        self.bridge_poll_shots(ctx);
        if self.input.pending() > 0 {
            ctx.request_repaint();
        }
        if self.bridge.as_ref().is_some_and(|b| !b.deferred.is_empty() || !b.shots.is_empty()) {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }

    /// Guarda a região da viewport do quadro atual (para o recorte do screenshot).
    pub(crate) fn bridge_note_viewport(&mut self, rect: egui::Rect) {
        if let Some(b) = self.bridge.as_mut() {
            b.viewport_rect = rect;
        }
    }

    /// Indicador na barra de status (honesto: a bridge está ativa e onde).
    pub(crate) fn bridge_status_label(&self, ui: &mut egui::Ui) {
        if let Some(b) = self.bridge.as_ref() {
            ui.separator();
            ui.label(egui::RichText::new(format!("Bridge {}", b.addr())).color(crate::theme::ACCENT).size(11.5))
                .on_hover_text(tooltip(self.lang(), &b.addr(), b.requests_total()));
        }
    }

    fn bridge_dispatch(&mut self, ctx: &egui::Context, r: HostRequest, since: Instant) {
        if r.is_cancelled() {
            return;
        }
        if r.action == "SCREENSHOT" {
            return self.bridge_screenshot(ctx, r);
        }
        match self.bridge_exec(ctx, &r.action, &r.params) {
            Some(res) => r.reply.send(res),
            None => {
                if since.elapsed() > DEFER_LIMIT {
                    r.reply.send(Err(BridgeError::new(code::UI_TIMEOUT, "a Biblioteca/os ativos não ficaram prontos a tempo; tente de novo")));
                } else if let Some(b) = self.bridge.as_mut() {
                    b.deferred.push(Deferred { req: r, since });
                }
            }
        }
    }

    /// Zera gestos transitórios dos workspaces (arrasto/preview em andamento) antes de mexer na sessão pelo lado
    /// da bridge — mesmo que `undo_redo` faz para o usuário.
    pub(super) fn bridge_reset_gestures(&mut self) {
        self.char_ui.gesture = None;
        self.scene_ui.session = None;
        self.scene_ui.gesture = None;
        self.light_ui.gesture = None;
        self.light_ui.drag = None;
        self.render_ui.gesture = None;
        self.render_ui.nav = None;
        self.render_ui.wheel = None;
        self.pose_reset_transients();
    }

    /// Mensagem na barra de status do workspace ativo.
    fn bridge_say(&mut self, msg: String) {
        match self.ws {
            WorkspaceId::Cenario => self.scene_ui.message = msg,
            WorkspaceId::Iluminacao => self.light_ui.message = msg,
            WorkspaceId::Render => self.render_ui.message = msg,
            WorkspaceId::Posing => self.pose_ui.message = msg,
            WorkspaceId::Shading => self.shading_ui.message = msg,
            _ => self.char_ui.message = msg,
        }
    }

    fn bridge_core_mutation(&mut self, action: &str, params: &Value) -> BridgeResult {
        self.bridge_reset_gestures();
        let res = core_ops::handle(&mut self.core, action, params).unwrap_or_else(|| Err(BridgeError::unknown_action(action)));
        if let Ok(v) = &res {
            let desc = v["outcome"]["description"].as_str().unwrap_or(action);
            let msg = super::character_ui::describe(desc);
            self.bridge_say(format!("bridge: {msg}"));
        }
        res
    }

    /// `None` = ainda não dá (Biblioteca abrindo / ativos carregando): o pedido é adiado e tentado no quadro seguinte.
    fn bridge_exec(&mut self, ctx: &egui::Context, action: &str, params: &Value) -> Option<BridgeResult> {
        if let Some(cmd) = legacy_command(action, params) {
            return Some(match cmd {
                Ok(c) => {
                    let p = json!({ "command": serde_json::to_value(&c).unwrap_or(Value::Null) });
                    self.bridge_core_mutation("APPLY_COMMAND", &p)
                }
                Err(e) => Err(e),
            });
        }
        match action {
            "APPLY_COMMAND" | "UNDO" | "REDO" => Some(self.bridge_core_mutation(action, params)),
            "GET_PROJECT" | "GET_SNAPSHOT" | "GET_LOG" => Some(core_ops::handle(&mut self.core, action, params).unwrap_or_else(|| Err(BridgeError::unknown_action(action)))),
            "GET_STATUS" => Some(Ok(self.bridge_status(ctx))),
            "GET_WORKSPACE" => Some(Ok(json!({
                "workspace": self.ws.key(),
                "available": WorkspaceId::ALL.iter().map(|w| w.key()).collect::<Vec<_>>(),
            }))),
            "SET_WORKSPACE" => Some(self.bridge_set_workspace(params)),
            // `select_tab` mantém a compatibilidade com `anigo_ui_action`; o resto é a UI nativa (`bridge_ui`)
            "UI_ACTION" => Some((|| {
                let m = as_obj(params)?;
                match opt_str(m, "action")? {
                    Some("select_tab") => {
                        let tab = opt_str(m, "value")?.ok_or_else(|| BridgeError::invalid_params("select_tab exige 'value'"))?;
                        self.bridge_set_workspace(&json!({ "workspace": tab }))
                    }
                    Some(a) => self.bridge_ui_action(a, m),
                    None => Err(BridgeError::invalid_params("UI_ACTION exige 'action'")),
                }
            })()),
            "GET_UI_STATE" => Some(Ok(self.bridge_ui_state())),
            "INPUT" => Some(self.bridge_input(params)),
            "MODEL" => Some(self.bridge_model(params)),
            "HAIR" => Some(self.bridge_hair(params)),
            "SHADING" => self.bridge_shading_ctx(ctx, params),
            "ANIM" => self.bridge_anim_ctx(ctx, params),
            "MESHDATA" => Some(self.bridge_meshdata(params)),
            "WEIGHTS" => Some(self.bridge_weights(params)),
            "GET_SELECTION" => Some(Ok(self.bridge_selection())),
            "LIST_ASSETS" => self.bridge_list_assets(ctx, params),
            "PLACE_ASSET" => self.bridge_place_asset(ctx, params),
            "POSE" => Some(self.bridge_pose(params)),
            "SCENE" => Some(self.bridge_scene(params)),
            "CHAR" => Some(self.bridge_char(params)),
            "APP" => Some(self.bridge_app(params)),
            "LIGHT" => Some(self.bridge_light(params)),
            "RENDER" => self.bridge_render(ctx, params),
            "LIBRARY" => self.bridge_library_op(ctx, params),
            "RENDER_FRAME" => self.bridge_render_frame(ctx),
            "RENDER_STATUS" => Some(self.bridge_render_status(ctx, params)),
            other => Some(Err(BridgeError::unknown_action(other))),
        }
    }

    fn bridge_status(&mut self, ctx: &egui::Context) -> Value {
        let dt = ctx.input(|i| i.stable_dt);
        let p = self.core.project();
        let cam = &p.scene.camera.camera;
        let (adapter, backend) = self.gpu.as_ref().map(|g| (g.adapter_name.clone(), g.backend.clone())).unwrap_or_default();
        let metrics = self.bridge.as_ref().map(|b| b.server.metrics().snapshot()).unwrap_or(Value::Null);
        json!({
            "engine": "ANIGO Studio (native)",
            "workspace": self.ws.key(),
            "frame_count": self.frame_count,
            "fps": if dt > 0.0 { json!(1.0 / dt) } else { Value::Null },
            "frame_time_ms": if dt > 0.0 { json!(dt * 1000.0) } else { Value::Null },
            "adapter_name": adapter,
            "backend": backend,
            "project_name": p.name,
            "camera_eye": [cam.eye.x, cam.eye.y, cam.eye.z],
            "camera_target": [cam.target.x, cam.target.y, cam.target.z],
            "node_count": p.scene.nodes.len(),
            "history": self.core.history_report(),
            "render_jobs_running": self.render_ui.running(),
            "bridge": metrics,
        })
    }

    fn bridge_set_workspace(&mut self, params: &Value) -> BridgeResult {
        let m = as_obj(params)?;
        let key = opt_str(m, "workspace")?.ok_or_else(|| BridgeError::invalid_params("'workspace' é obrigatório"))?;
        let w = WorkspaceId::from_key(key).ok_or_else(|| BridgeError::invalid_params(format!("workspace desconhecido: {key}")))?;
        let from = self.ws;
        if w != from {
            self.switch_workspace(w);
        }
        Ok(json!({ "workspace": self.ws.key(), "previous": from.key() }))
    }

    fn bridge_selection(&self) -> Value {
        let scene_ids: Vec<String> = self.scene_ui.selection.ids().iter().map(|i| i.to_string()).collect();
        let s = self.modeling.stats();
        json!({
            "workspace": self.ws.key(),
            "scene": { "node_ids": scene_ids, "active": self.scene_ui.selection.active().map(|i| i.to_string()) },
            "modeling": {
                "mode": format!("{:?}", self.modeling.mode),
                "active_object": self.modeling.active_object().map(|o| o.name.clone()),
                "selected_verts": s.selected_verts,
                "selected_faces": s.selected_faces,
            },
            "posing": { "bones": self.pose_ui.selection.iter().copied().collect::<Vec<usize>>(), "active": self.pose_ui.active },
            "library": { "asset_ids": self.lib_ws.as_ref().map(|w| w.selected_asset_ids().iter().map(|i| i.to_string()).collect::<Vec<_>>()).unwrap_or_default() },
        })
    }

    /// A Biblioteca pronta (clone barato) ou `None` depois de pedir a abertura em segundo plano.
    pub(super) fn bridge_library(&mut self, ctx: &egui::Context) -> Option<anigo_assets::Library> {
        if let Some(lib) = self.lib_ws.as_ref().and_then(|w| w.library().cloned()) {
            return Some(lib);
        }
        let w = self.lib_workspace();
        w.ensure_open();
        w.pump(ctx);
        w.library().cloned()
    }

    /// `LIBRARY`: o Asset Browser sem janela (`library_ui/bridge.rs`); `edit_in_modeling` precisa do app inteiro.
    fn bridge_library_op(&mut self, ctx: &egui::Context, params: &Value) -> Option<BridgeResult> {
        let m = match as_obj(params) {
            Ok(m) => m,
            Err(e) => return Some(Err(e)),
        };
        let lib = self.bridge_library(ctx)?; // adia enquanto a Biblioteca abre
        if matches!(opt_str(m, "op"), Ok(Some("edit_in_modeling"))) {
            return Some((|| {
                let id: anigo_assets::AssetId = opt_str(m, "asset_id")?.ok_or_else(|| BridgeError::invalid_params("'asset_id' é obrigatório"))?.parse().map_err(|e: anigo_assets::AssetError| BridgeError::invalid_params(e.to_string()))?;
                let row = lib.require_row(&id).map_err(|e| BridgeError::failed(e.to_string()))?;
                let f = lib.resolve_files(&id).map_err(|e| BridgeError::failed(e.to_string()))?;
                if !f.exists {
                    return Err(BridgeError::failed("o arquivo do ativo não está disponível (local offline ou apagado)"));
                }
                let before = self.modeling.doc.objects.len();
                self.edit_library_asset(&f.primary, &row.format, &row.name);
                let after = self.modeling.doc.objects.len();
                if after <= before {
                    return Err(BridgeError::failed(self.modeling.status.clone()));
                }
                Ok(json!({ "workspace": self.ws.key(), "objects": after, "status": self.modeling.status }))
            })());
        }
        Some(self.lib_workspace().bridge_call(ctx, params))
    }

    fn bridge_list_assets(&mut self, ctx: &egui::Context, params: &Value) -> Option<BridgeResult> {
        let m = match as_obj(params) {
            Ok(m) => m,
            Err(e) => return Some(Err(e)),
        };
        let mut q = anigo_assets::Query { limit: 100, with_total: true, ..Default::default() };
        let parsed = (|| -> Result<(), BridgeError> {
            if let Some(t) = opt_str(m, "text")? {
                if t.len() > 256 {
                    return Err(BridgeError::invalid_params("'text' longo demais (máx. 256)"));
                }
                q.text = Some(t.to_string());
            }
            if let Some(k) = m.get("kinds").filter(|v| !v.is_null()) {
                let arr = k.as_array().filter(|a| a.len() <= 16).ok_or_else(|| BridgeError::invalid_params("'kinds' deve ser uma lista de até 16 tipos"))?;
                for item in arr {
                    let s = item.as_str().ok_or_else(|| BridgeError::invalid_params("'kinds' deve conter textos"))?;
                    q.kinds.push(s.parse::<anigo_assets::AssetKind>().map_err(|e| BridgeError::invalid_params(e.to_string()))?);
                }
            }
            if let Some(l) = opt_u64(m, "limit")? {
                q.limit = (l as usize).clamp(1, 500);
            }
            if let Some(a) = opt_str(m, "after")? {
                q.after = Some(anigo_assets::Cursor(a.to_string()));
            }
            Ok(())
        })();
        if let Err(e) = parsed {
            return Some(Err(e));
        }
        let Some(lib) = self.bridge_library(ctx) else { return None };
        Some(match lib.query(&q) {
            Ok(page) => Ok(json!({
                "total": page.total,
                "next": page.next.map(|c| c.0),
                "assets": page.items.iter().map(|a| json!({
                    "id": a.id.to_string(),
                    "name": a.name,
                    "kind": a.kind.as_str(),
                    "format": a.format,
                    "size": a.size,
                    "tris": a.tris,
                    "verts": a.verts,
                    "bones": a.bones,
                    "has_skin": a.has_skin,
                    "has_vrm": a.has_vrm,
                    "revision": a.revision,
                    "available": a.available,
                })).collect::<Vec<_>>(),
            })),
            Err(e) => Err(BridgeError::failed(format!("consulta à Biblioteca falhou: {e}"))),
        })
    }

    fn bridge_place_asset(&mut self, ctx: &egui::Context, params: &Value) -> Option<BridgeResult> {
        let m = match as_obj(params) {
            Ok(m) => m,
            Err(e) => return Some(Err(e)),
        };
        let parsed = (|| -> Result<(Vec<anigo_assets::AssetId>, Option<crate::scene::placement::DropTarget>), BridgeError> {
            let mut ids = Vec::new();
            if let Some(one) = opt_str(m, "asset_id")? {
                ids.push(one.parse().map_err(|e: anigo_assets::AssetError| BridgeError::invalid_params(e.to_string()))?);
            }
            if let Some(list) = m.get("asset_ids").filter(|v| !v.is_null()) {
                let arr = list.as_array().filter(|a| a.len() <= 32).ok_or_else(|| BridgeError::invalid_params("'asset_ids' deve ser uma lista de até 32 ids"))?;
                for s in arr {
                    let s = s.as_str().ok_or_else(|| BridgeError::invalid_params("'asset_ids' deve conter textos"))?;
                    ids.push(s.parse().map_err(|e: anigo_assets::AssetError| BridgeError::invalid_params(e.to_string()))?);
                }
            }
            if ids.is_empty() {
                return Err(BridgeError::invalid_params("informe 'asset_id' ou 'asset_ids'"));
            }
            let target = match m.get("position").filter(|v| !v.is_null()) {
                Some(p) => Some(crate::scene::placement::DropTarget { point: vec3(p, "position")?, normal: glam::Vec3::Y, on_geometry: false }),
                None => None,
            };
            Ok((ids, target))
        })();
        let (ids, target) = match parsed {
            Ok(v) => v,
            Err(e) => return Some(Err(e)),
        };
        if self.bridge_library(ctx).is_none() {
            return None;
        }
        self.bridge_reset_gestures();
        let ok = self.scene_place_library(&ids, target);
        if !ok {
            let why = self.scene_ui.message.clone();
            return Some(Err(BridgeError::failed(if why.is_empty() { "não foi possível colocar o ativo".to_string() } else { why })));
        }
        let placed: Vec<String> = self.scene_ui.selection.ids().iter().map(|i| i.to_string()).collect();
        Some(Ok(json!({ "placed": placed.len(), "node_ids": placed, "history": self.core.history_report() })))
    }

    pub(super) fn bridge_render_frame(&mut self, ctx: &egui::Context) -> Option<BridgeResult> {
        let before = self.render_ui.jobs.len();
        let next_id = self.render_ui.next_id;
        self.render_enqueue(ctx);
        if self.render_ui.jobs.len() > before {
            let j = self.render_ui.jobs.last().expect("job recém criado");
            return Some(Ok(json!({ "job_id": j.id, "label": j.label, "status": "queued", "output_dir": self.render_ui.out_dir().display().to_string() })));
        }
        debug_assert_eq!(next_id, self.render_ui.next_id);
        // ativos ainda carregando: espera (adiado); qualquer outra recusa vira erro
        if self.scene_ui.cache.loading_count() > 0 {
            return None;
        }
        Some(Err(BridgeError::failed(self.render_ui.message.clone())))
    }

    fn bridge_render_status(&mut self, ctx: &egui::Context, params: &Value) -> BridgeResult {
        let m = as_obj(params)?;
        self.render_poll(ctx);
        let job = match opt_u64(m, "job_id")? {
            Some(id) => self.render_ui.jobs.iter().find(|j| j.id == id).ok_or_else(|| BridgeError::invalid_params(format!("job {id} desconhecido")))?,
            None => self.render_ui.jobs.last().ok_or_else(|| BridgeError::invalid_params("nenhum job de render ainda"))?,
        };
        let (status, error) = match &job.status {
            JobStatus::Queued => ("queued", None),
            JobStatus::Running => ("running", None),
            JobStatus::Done => ("done", None),
            JobStatus::Failed(e) => ("failed", Some(e.clone())),
            JobStatus::Cancelled => ("cancelled", None),
        };
        let files: Vec<Value> = job.files.iter().map(|f| json!({ "pass": f.pass, "path": f.path.display().to_string(), "width": f.w, "height": f.h, "bytes": f.bytes })).collect();
        let mut out = json!({
            "job_id": job.id, "status": status, "progress": job.frac, "error": error, "files": files,
            "notes": job.notes, "render_ms": job.render_ms,
        });
        let inline = m.get("inline").and_then(Value::as_bool).unwrap_or(false);
        if inline && status == "done" {
            if let Some(f) = job.files.first() {
                match std::fs::metadata(&f.path) {
                    Ok(md) if md.len() <= MAX_INLINE_IMAGE => match std::fs::read(&f.path) {
                        Ok(bytes) => out["png_base64"] = json!(base64::engine::general_purpose::STANDARD.encode(bytes)),
                        Err(e) => out["inline_error"] = json!(e.to_string()),
                    },
                    Ok(md) => out["inline_error"] = json!(format!("arquivo de {} bytes excede o limite de {} para envio inline", md.len(), MAX_INLINE_IMAGE)),
                    Err(e) => out["inline_error"] = json!(e.to_string()),
                }
            }
        }
        Ok(out)
    }

    // ------------------------------------------------------------------------------------------ screenshot

    fn bridge_screenshot(&mut self, ctx: &egui::Context, r: HostRequest) {
        let parsed = (|| -> Result<(bool, Option<u32>), BridgeError> {
            let m = as_obj(&r.params)?;
            let whole = match opt_str(m, "target")? {
                None | Some("viewport") => false,
                Some("window") => true,
                Some(o) => return Err(BridgeError::invalid_params(format!("'target' deve ser 'viewport' ou 'window', recebi {o:?}"))),
            };
            let mw = opt_u64(m, "max_width")?;
            if let Some(w) = mw {
                if !(16..=8192).contains(&w) {
                    return Err(BridgeError::invalid_params("'max_width' fora de 16..8192"));
                }
            }
            Ok((whole, mw.map(|w| w as u32)))
        })();
        let (whole_window, max_width) = match parsed {
            Ok(v) => v,
            Err(e) => return r.reply.send(Err(e)),
        };
        let Some(b) = self.bridge.as_mut() else { return };
        let id = b.next_shot;
        b.next_shot += 1;
        b.shots.push(PendingShot { id, reply: r.reply, cancelled: r.cancelled, whole_window, max_width, asked: Instant::now() });
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(BridgeShot(id))));
        ctx.request_repaint();
    }

    fn bridge_poll_shots(&mut self, ctx: &egui::Context) {
        let ws = self.ws.key();
        let ppp = ctx.pixels_per_point();
        let Some(b) = self.bridge.as_mut() else { return };
        if b.shots.is_empty() {
            return;
        }
        let events: Vec<(u64, Arc<egui::ColorImage>)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { user_data, image, .. } => user_data.data.as_ref().and_then(|d| d.downcast_ref::<BridgeShot>()).map(|s| (s.0, image.clone())),
                    _ => None,
                })
                .collect()
        });
        let rect = b.viewport_rect;
        for (id, image) in events {
            if let Some(pos) = b.shots.iter().position(|s| s.id == id) {
                let shot = b.shots.remove(pos);
                if shot.cancelled.load(Relaxed) {
                    continue;
                }
                // PNG + base64 fora da thread da UI
                let _ = std::thread::Builder::new().name("anigo-bridge-shot".into()).spawn(move || {
                    shot.reply.send(encode_shot(&image, rect, ppp, shot.whole_window, shot.max_width, ws));
                });
            }
        }
        b.shots.retain(|s| !s.cancelled.load(Relaxed) && s.asked.elapsed() < Duration::from_secs(25));
    }
}

/// Recorta (com limites seguros), reduz se pedido e codifica em PNG/base64.
fn encode_shot(image: &egui::ColorImage, rect: egui::Rect, ppp: f32, whole: bool, max_width: Option<u32>, ws: &str) -> BridgeResult {
    let (iw, ih) = (image.size[0], image.size[1]);
    if iw == 0 || ih == 0 {
        return Err(BridgeError::failed("captura vazia"));
    }
    let (mut x0, mut y0, mut x1, mut y1) = (0usize, 0usize, iw, ih);
    let mut target = "window";
    if !whole && rect.is_positive() && rect.width().is_finite() {
        let cx0 = ((rect.min.x * ppp).floor().max(0.0) as usize).min(iw);
        let cy0 = ((rect.min.y * ppp).floor().max(0.0) as usize).min(ih);
        let cx1 = ((rect.max.x * ppp).ceil().max(0.0) as usize).min(iw);
        let cy1 = ((rect.max.y * ppp).ceil().max(0.0) as usize).min(ih);
        if cx1 > cx0 && cy1 > cy0 {
            (x0, y0, x1, y1) = (cx0, cy0, cx1, cy1);
            target = "viewport";
        }
    }
    let (w, h) = (x1 - x0, y1 - y0);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for row in y0..y1 {
        for c in &image.pixels[row * iw + x0..row * iw + x1] {
            rgba.extend_from_slice(&c.to_array());
        }
    }
    let mut img = image::RgbaImage::from_raw(w as u32, h as u32, rgba).ok_or_else(|| BridgeError::failed("buffer de captura inconsistente"))?;
    if let Some(mw) = max_width.filter(|mw| img.width() > *mw) {
        let nh = ((img.height() as f64) * (mw as f64) / (img.width() as f64)).round().max(1.0) as u32;
        img = image::imageops::resize(&img, mw, nh, image::imageops::FilterType::Triangle);
    }
    let mut png = Vec::new();
    {
        use image::ImageEncoder;
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(img.as_raw(), img.width(), img.height(), image::ColorType::Rgba8.into())
            .map_err(|e| BridgeError::failed(format!("PNG: {e}")))?;
    }
    Ok(json!({
        "width": img.width(), "height": img.height(), "target": target, "workspace": ws, "pixels_per_point": ppp,
        "png_base64": base64::engine::general_purpose::STANDARD.encode(&png),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_actions_become_core_commands() {
        let c = legacy_command("ORBIT", &json!({"azimuth": 0.5, "elevation": -0.1})).unwrap().unwrap();
        assert!(matches!(c, Command::OrbitCamera { .. }));
        let c = legacy_command("SET_CAMERA", &json!({"eye": [1.0, 2.0, 3.0], "target": [0.0, 1.0, 0.0]})).unwrap().unwrap();
        assert!(matches!(c, Command::SetCamera { eye: Some(_), .. }));
        let c = legacy_command("SET_LIGHT", &json!({"intensity": 0.8})).unwrap().unwrap();
        assert!(matches!(c, Command::SetLight { intensity: Some(_), .. }));
        assert!(legacy_command("ZOOM", &json!({"factor": "x"})).unwrap().is_err());
        assert!(legacy_command("ORBIT", &json!([1, 2])).unwrap().is_err());
        assert!(legacy_command("PING", &Value::Null).is_none());
    }

    #[test]
    fn param_helpers_validate_types_and_ranges() {
        assert!(vec3(&json!([1, 2, 3]), "p").is_ok());
        assert!(vec3(&json!([1, 2]), "p").is_err());
        assert!(vec3(&json!([1, 2, "x"]), "p").is_err());
        assert!(vec3(&json!([1e9, 0, 0]), "p").is_err());
        let m = json!({"a": "x", "n": 3, "neg": -1});
        let m = m.as_object().unwrap();
        assert_eq!(opt_str(m, "a").unwrap(), Some("x"));
        assert!(opt_str(m, "n").is_err());
        assert_eq!(opt_u64(m, "n").unwrap(), Some(3));
        assert!(opt_u64(m, "neg").is_err());
        assert_eq!(opt_u64(m, "missing").unwrap(), None);
        assert!(as_obj(&json!(5)).is_err());
        assert!(as_obj(&Value::Null).unwrap().is_empty());
    }

    fn solid(w: usize, h: usize) -> egui::ColorImage {
        egui::ColorImage::filled([w, h], egui::Color32::from_rgb(10, 20, 30))
    }

    fn decode(v: &Value) -> image::RgbaImage {
        let png = base64::engine::general_purpose::STANDARD.decode(v["png_base64"].as_str().unwrap()).unwrap();
        image::load_from_memory(&png).unwrap().to_rgba8()
    }

    #[test]
    fn screenshot_crops_to_the_viewport_with_hidpi_and_clamps_out_of_range_rects() {
        let img = solid(400, 300);
        let rect = egui::Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(110.0, 120.0)); // pontos
        let v = encode_shot(&img, rect, 2.0, false, None, "cenario").unwrap();
        assert_eq!((v["width"].as_u64(), v["height"].as_u64(), v["target"].as_str()), (Some(200), Some(200), Some("viewport")));
        assert_eq!(decode(&v).dimensions(), (200, 200));
        // retângulo que passa da imagem é recortado, nunca causa pânico
        let big = egui::Rect::from_min_max(egui::pos2(-50.0, -50.0), egui::pos2(5000.0, 5000.0));
        let v = encode_shot(&img, big, 1.0, false, None, "cenario").unwrap();
        assert_eq!((v["width"].as_u64(), v["height"].as_u64()), (Some(400), Some(300)));
        // sem retângulo válido (ou janela inteira) = captura inteira
        let v = encode_shot(&img, egui::Rect::NOTHING, 1.0, false, None, "cenario").unwrap();
        assert_eq!(v["target"], "window");
        let v = encode_shot(&img, rect, 1.0, true, None, "cenario").unwrap();
        assert_eq!((v["width"].as_u64(), v["target"].as_str()), (Some(400), Some("window")));
        // retângulo totalmente fora = captura inteira
        let out = egui::Rect::from_min_max(egui::pos2(900.0, 900.0), egui::pos2(950.0, 950.0));
        assert_eq!(encode_shot(&img, out, 1.0, false, None, "x").unwrap()["target"], "window");
    }

    #[test]
    fn screenshot_downscales_keeping_aspect_and_keeps_pixels() {
        let mut img = solid(400, 200);
        img.pixels[0] = egui::Color32::from_rgb(255, 0, 0);
        let v = encode_shot(&img, egui::Rect::NOTHING, 1.0, true, Some(100), "x").unwrap();
        assert_eq!((v["width"].as_u64(), v["height"].as_u64()), (Some(100), Some(50)));
        let full = encode_shot(&img, egui::Rect::NOTHING, 1.0, true, None, "x").unwrap();
        assert_eq!(decode(&full).get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(decode(&full).get_pixel(5, 5).0, [10, 20, 30, 255]);
        // max_width maior que a imagem não amplia
        let v = encode_shot(&img, egui::Rect::NOTHING, 1.0, true, Some(4000), "x").unwrap();
        assert_eq!(v["width"], 400);
    }

    #[test]
    fn tooltip_is_localized_and_mentions_the_address() {
        assert!(tooltip(Lang::PtBr, "127.0.0.1:1", 3).contains("loopback"));
        assert!(tooltip(Lang::En, "127.0.0.1:1", 3).contains("Requests served: 3"));
        assert!(tooltip(Lang::PtBr, "127.0.0.1:9", 0).contains("127.0.0.1:9"));
    }
}
