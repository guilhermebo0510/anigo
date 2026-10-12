//! Workspace Render nativo: câmera de render (lente, near/far, guias, enquadrar), configurações de saída,
//! passes/AOVs, PREVIEW da câmera de render (máscara de enquadramento + margens seguras) e a fila de render
//! real (thread de GPU dedicada, progresso/cancelar/retomar, histórico com miniaturas).
//!
//! Toda mudança persistente é um `Command` do núcleo (`StudioApp::core`, a MESMA sessão dos workspaces Personagem e
//! Cenário): `SetRenderOutput`, `SetCamera`, `SetBackgroundColor`, `SetRenderSettings`. Editar um campo numérico
//! usa `CoreSession::preview` (ao vivo) e vira UM passo de desfazer ao soltar. Pasta de saída, template de nome e
//! contador são preferências da máquina (sessão), não do projeto.
//!
//! * `panels` — painel direito (Câmera / Passes / Saída) e a fila + histórico;
//! * `viewport` — preview pela câmera de render, navegação, guias;
//! * `script` — cenários de captura (`--scenario render-*`).
//!
//! A lógica pura e a fila mora em `crate::render_out` (testada sem egui).

mod panels;
mod script;
#[cfg(test)]
mod pipeline_tests;
mod viewport;

use super::StudioApp;
use crate::render_out::history::{self, HistoryEntry};
use crate::render_out::job::{enabled_aovs, Event, FrameSource, JobSpec, Meta, OutFile, RenderService, RenderSpec, StillSource, Stage};
use crate::render_out::settings::{self, Allocation, NameTokens, DEFAULT_TEMPLATE};
use anigo_core::command::Command;
use anigo_core::ids::NodeId;
use anigo_core::scene::SceneNode;
use anigo_core::{NodeSource, RenderOutputPatch, Scene};
use anigo_renderer::GraphOverrides;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub(crate) use script::is_render_scenario;
#[allow(unused_imports)]
pub use script::RENDER_SCENARIOS;

/// Aba do painel direito (escolhida na tool shelf: câmera / passes / exportação).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RenderTab {
    #[default]
    Camera,
    Passes,
    Output,
}

impl RenderTab {
    pub fn from_tool_index(i: usize) -> Self {
        match i {
            1 => RenderTab::Passes,
            2 => RenderTab::Output,
            _ => RenderTab::Camera,
        }
    }
}

/// Guias de composição desenhadas sobre o preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderGuides {
    pub thirds: bool,
    pub center: bool,
    pub action_safe: bool,
    pub title_safe: bool,
    /// Escurece o que fica fora do enquadramento.
    pub dim_outside: bool,
    pub info: bool,
}

impl Default for RenderGuides {
    fn default() -> Self {
        Self { thirds: true, center: false, action_safe: false, title_safe: false, dim_outside: true, info: true }
    }
}

/// Preferências do Render que sobrevivem ao reinício (vão em `session.json`; não são do projeto).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderPrefs {
    /// Pasta de saída digitada (vazia = a sugerida).
    pub dir: String,
    pub template: String,
    /// Próximo número do contador.
    pub counter: u32,
    pub overwrite: bool,
    pub lock_aspect: bool,
    /// Sensor de referência para focal ↔ FOV (índice em `framing::SENSORS`).
    pub sensor: usize,
    pub guides: RenderGuides,
    pub queue_open: bool,
}

impl Default for RenderPrefs {
    fn default() -> Self {
        Self {
            dir: settings::suggested_output_dir().display().to_string(),
            template: DEFAULT_TEMPLATE.to_string(),
            counter: 1,
            overwrite: false,
            lock_aspect: true,
            sensor: 0,
            guides: RenderGuides::default(),
            queue_open: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum JobStatus {
    Queued,
    Running,
    Done,
    Failed(String),
    Cancelled,
}

/// Um job na lista da UI (o `JobSpec` fica para "retomar").
pub(crate) struct JobView {
    pub id: u64,
    pub label: String,
    pub status: JobStatus,
    pub frac: f32,
    pub stage: Option<Stage>,
    pub frame_idx: usize,
    pub total: usize,
    pub files: Vec<OutFile>,
    pub notes: Vec<String>,
    pub cancel: Arc<AtomicBool>,
    pub spec: JobSpec,
    pub render_ms: f64,
    /// Nomes alocados (retomar um render avulso reaproveita o mesmo nome).
    pub alloc: Option<Allocation>,
    pub started: std::time::Instant,
    pub finished: Option<std::time::Duration>,
}

/// Gesto de navegação da câmera de render no preview (órbita / pan / dolly): um passo de desfazer ao soltar.
pub(crate) struct CameraNav {
    pub eye: glam::Vec3,
    pub target: glam::Vec3,
    pub last: Option<Command>,
}

pub(crate) struct RenderUi {
    pub tab: RenderTab,
    pub prefs: RenderPrefs,
    pub service: Option<RenderService>,
    pub jobs: Vec<JobView>,
    pub next_id: u64,
    pub history: Vec<HistoryEntry>,
    pub history_dir: Option<PathBuf>,
    pub thumbs: HashMap<String, Option<egui::TextureHandle>>,
    pub message: String,
    pub message_seen: (String, Option<std::time::Instant>),
    /// Edição numérica em andamento: último comando ao vivo (vira um passo ao soltar o ponteiro).
    pub gesture: Option<Command>,
    pub nav: Option<CameraNav>,
    /// Roda do mouse: dolly acumulado até uma pausa (então vira um passo).
    pub wheel: Option<(CameraNav, std::time::Instant)>,
    /// Câmera de cena usada como câmera de render (só para exibir/reaplicar; o dado vive no projeto).
    pub linked: Option<NodeId>,
    pub ffmpeg: Option<Result<String, String>>,
    /// Proporção travada (largura/altura) enquanto "bloquear aspecto" está ligado.
    pub aspect_ref: f32,
    pub script: Option<script::Script>,
    /// Retângulo da imagem no preview (para o trace do smoke) e posições de botões (pixels físicos).
    pub trace: Option<String>,
    pub render_btn: egui::Pos2,
    pub swap_btn: egui::Pos2,
    pub cancel_btn: egui::Pos2,
    pub img_rect: egui::Rect,
}

impl Default for RenderUi {
    fn default() -> Self {
        Self {
            tab: RenderTab::Camera,
            prefs: RenderPrefs::default(),
            service: None,
            jobs: Vec::new(),
            next_id: 1,
            history: Vec::new(),
            history_dir: None,
            thumbs: HashMap::new(),
            message: String::new(),
            message_seen: (String::new(), None),
            gesture: None,
            nav: None,
            wheel: None,
            linked: None,
            ffmpeg: None,
            aspect_ref: 16.0 / 9.0,
            script: None,
            trace: None,
            render_btn: egui::Pos2::ZERO,
            swap_btn: egui::Pos2::ZERO,
            cancel_btn: egui::Pos2::ZERO,
            img_rect: egui::Rect::NOTHING,
        }
    }
}

impl RenderUi {
    pub fn apply_prefs(&mut self, p: &RenderPrefs) {
        self.prefs = p.clone();
        self.prefs.counter = self.prefs.counter.max(1);
        self.prefs.sensor = self.prefs.sensor.min(crate::render_out::framing::SENSORS.len() - 1);
        if self.prefs.template.trim().is_empty() {
            self.prefs.template = DEFAULT_TEMPLATE.to_string();
        }
    }

    /// Pasta de saída efetiva.
    pub fn out_dir(&self) -> PathBuf {
        let d = self.prefs.dir.trim();
        if d.is_empty() {
            settings::suggested_output_dir()
        } else {
            PathBuf::from(d)
        }
    }

    pub fn running(&self) -> bool {
        self.jobs.iter().any(|j| matches!(j.status, JobStatus::Running | JobStatus::Queued))
    }
}

/// Estado da pasta digitada, sem criar nada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DirStatus {
    Ready,
    WillCreate,
    NotADirectory,
    ReadOnly,
}

pub(crate) fn dir_status(dir: &std::path::Path) -> DirStatus {
    match std::fs::metadata(dir) {
        Ok(m) if m.is_dir() => {
            if m.permissions().readonly() {
                DirStatus::ReadOnly
            } else {
                DirStatus::Ready
            }
        }
        Ok(_) => DirStatus::NotADirectory,
        Err(_) => DirStatus::WillCreate,
    }
}

/// Detecta o `ffmpeg` do sistema (só `ffmpeg -version`; nunca executa encode).
pub(crate) fn detect_ffmpeg() -> Result<String, String> {
    let out = std::process::Command::new("ffmpeg").arg("-version").stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("ffmpeg retornou erro".into());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text.lines().next().unwrap_or("ffmpeg").to_string())
}

/// Linha de comando sugerida para montar uma sequência PNG (`%04d`) — só texto, nada é executado.
pub(crate) fn ffmpeg_command(dir: &std::path::Path, stem: &str, fps: u32) -> String {
    format!("ffmpeg -framerate {fps} -i \"{}\" -c:v libx264 -pix_fmt yuv420p -crf 14 \"{}\"", dir.join(format!("{stem}_%04d.png")).display(), dir.join(format!("{stem}.mp4")).display())
}

/// Núcleo puro de [`StudioApp::build_render_scene`]: projeto + geometria resolvida -> `Scene` do renderer canônico.
pub(crate) fn scene_from_project(project: &anigo_core::ProjectState, runtime: &crate::scene::runtime::SceneRuntime, loading: bool) -> Result<(Scene, Vec<String>), String> {
    use crate::scene::runtime::GeomState;
    if loading || runtime.geom.values().any(|g| matches!(g, GeomState::Loading)) {
        return Err(crate::i18n::tr("render.err.loading").to_string());
    }
    let mut scene = Scene::new_empty();
    let mut notes = Vec::new();
    let mut missing = 0;
    for slot in &project.scene.nodes {
        let mut n = SceneNode::new(slot.node_id.to_string(), slot.name.clone());
        n.parent_id = slot.parent_id.as_ref().map(|p| p.to_string());
        n.transform = slot.transform;
        n.kind = slot.kind;
        n.visible = !runtime.hidden.contains(&slot.node_id);
        n.material = Some(super::scene_ui::node_material(project, slot));
        match runtime.geom.get(&slot.node_id) {
            Some(GeomState::Ready(g)) => n.mesh = Some((*g.mesh).clone()),
            Some(GeomState::Missing(_)) => missing += 1,
            _ => {}
        }
        scene.add_node(n);
    }
    if missing > 0 {
        notes.push(format!("{missing} {}", crate::i18n::tr("render.warn.missing_assets")));
    }
    if !scene.nodes.iter().any(|n| n.visible && n.mesh.is_some()) {
        return Err(crate::i18n::tr("render.err.empty").to_string());
    }
    scene.rebuild_children();
    scene.camera = project.scene.camera.camera.clone();
    scene.light = project.scene.lights.first().map(|l| l.light.clone()).unwrap_or_default();
    scene.background_color = project.render.background_color;
    scene.dof = project.render.output.dof;
    Ok((scene, notes))
}

impl StudioApp {
    pub(crate) fn render_say(&mut self, msg: impl Into<String>) {
        self.render_ui.message = msg.into();
    }

    /// Aplica um comando do núcleo (UM passo de desfazer) e reporta o resultado. `true` se foi aceito.
    pub(crate) fn render_apply(&mut self, cmd: Command) -> bool {
        match self.core.apply(cmd) {
            Ok(o) => {
                self.render_ui.message = super::scene_ui::describe_scene(&o.description);
                true
            }
            Err(e) => {
                if !e.contains("no effect") && !e.contains("already") && !e.contains("already has") {
                    self.render_ui.message = format!("{}: {e}", crate::i18n::tr("char.error"));
                }
                false
            }
        }
    }

    pub(crate) fn render_apply_patch(&mut self, patch: RenderOutputPatch) -> bool {
        self.render_apply(Command::SetRenderOutput { patch })
    }

    /// Edição ao vivo (arrasto de campo): preview agora, UM passo de desfazer ao soltar.
    pub(crate) fn render_edit(&mut self, cmd: Command) {
        let _ = self.core.preview(cmd.clone());
        self.render_ui.gesture = Some(cmd);
    }

    pub(crate) fn render_edit_patch(&mut self, patch: RenderOutputPatch) {
        self.render_edit(Command::SetRenderOutput { patch });
    }

    /// Termina a edição numérica quando o ponteiro é solto.
    pub(crate) fn render_finish_gesture(&mut self, ctx: &egui::Context) {
        if self.render_ui.gesture.is_none() || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        if let Some(cmd) = self.render_ui.gesture.take() {
            match self.core.commit_preview(cmd) {
                Ok(o) => self.render_ui.message = super::scene_ui::describe_scene(&o.description),
                Err(e) => {
                    if !e.contains("no effect") && !e.contains("already") {
                        self.render_ui.message = e;
                    }
                }
            }
        }
    }

    /// Saída do workspace: o preview do Cenário não herda DoF nem passes desligados.
    pub(crate) fn render_leave(&mut self) {
        self.core.cancel_preview();
        self.render_ui.gesture = None;
        self.render_ui.nav = None;
        self.render_ui.wheel = None;
        if let Some(cel) = self.gpu.as_mut().and_then(|g| g.cel.as_mut()) {
            cel.clear_render_view();
        }
    }

    /// Ctrl+Z / Ctrl+Y (fora de campos de texto) — a mesma sessão do núcleo.
    pub(crate) fn render_shortcuts(&mut self, ctx: &egui::Context) {
        self.character_shortcuts(ctx);
    }

    /// Entrada no workspace: libera estado de edição de outros workspaces e abre a Biblioteca em segundo plano
    /// (as malhas dos ativos da cena vêm dela) — sem trocar o que o Cenário configurou.
    pub(crate) fn render_enter(&mut self, ctx: &egui::Context) {
        if !self.scene_ui.history_limit_set {
            self.core.set_history_limit(400);
            self.scene_ui.history_limit_set = true;
        }
        self.core.cancel_preview();
        self.scene_ui.gesture = None;
        self.scene_ui.session = None;
        // a Biblioteca abre em segundo plano e precisa ser "bombeada" a cada quadro (as malhas dos ativos vêm dela)
        let w = self.lib_workspace();
        w.ensure_open();
        w.pump(ctx);
        // aspecto de referência = o da saída atual
        let o = self.core.project().render.output;
        if self.render_ui.prefs.lock_aspect && (self.render_ui.aspect_ref - o.aspect()).abs() > 1e-4 && self.render_ui.gesture.is_none() {
            self.render_ui.aspect_ref = o.aspect();
        }
    }

    // ===================================================================== cena de render

    /// Monta a `Scene` do renderer a partir do projeto e da geometria já carregada pelo Cenário (mesmas malhas,
    /// matrizes e materiais que o preview). Recusa enquanto algum ativo ainda carrega.
    pub(crate) fn build_render_scene(&mut self) -> Result<(Scene, Vec<String>), String> {
        self.scene_refresh();
        let loading = self.scene_ui.cache.loading_count() > 0;
        scene_from_project(self.core.project(), &self.scene_ui.runtime, loading)
    }

    /// Nome da câmera de cena vinculada (ou "Câmera do projeto").
    pub(crate) fn render_camera_name(&self) -> String {
        let p = self.core.project();
        self.render_ui
            .linked
            .as_ref()
            .and_then(|id| p.scene.node(id))
            .map(|n| n.name.clone())
            .unwrap_or_else(|| crate::i18n::tr("render.camera.project").to_string())
    }

    /// Passes do projeto como overrides do render graph.
    pub(crate) fn render_graph(&self) -> GraphOverrides {
        let r = &self.core.project().render;
        GraphOverrides { order: r.graph_order.clone(), disabled: r.graph_disabled.clone(), depth_prepass: r.depth_prepass }
    }

    pub(crate) fn render_spec(&self) -> RenderSpec {
        let p = self.core.project();
        RenderSpec {
            output: p.render.output,
            display_space: p.render.color.display_space,
            bg: p.render.background_color,
            graph: self.render_graph(),
            meta: Meta {
                project_id: p.project_id.to_string(),
                project_name: p.name.clone(),
                fingerprint: p.content_fingerprint(),
                schema_version: p.schema_version,
                camera_name: self.render_camera_name(),
            },
        }
    }

    // ===================================================================== fila

    fn render_service(&mut self, ctx: &egui::Context) -> &RenderService {
        if self.render_ui.service.is_none() {
            let c = ctx.clone();
            self.render_ui.service = Some(RenderService::start(Some(Arc::new(move || c.request_repaint()))));
        }
        self.render_ui.service.as_ref().expect("serviço criado acima")
    }

    /// "Renderizar quadro": valida, congela a cena e a configuração e enfileira. A UI continua livre.
    pub(crate) fn render_enqueue(&mut self, ctx: &egui::Context) {
        let dir = self.render_ui.out_dir();
        if let Err(e) = settings::check_output_dir(&dir) {
            self.render_say(format!("{}: {e}", crate::i18n::tr("render.err.dir")));
            return;
        }
        let (scene, notes) = match self.build_render_scene() {
            Ok(v) => v,
            Err(e) => {
                self.render_say(e);
                return;
            }
        };
        let spec = self.render_spec();
        let source: Arc<dyn FrameSource> = Arc::new(StillSource(scene));
        let id = self.render_ui.next_id;
        self.render_ui.next_id += 1;
        let (tw, th) = spec.output.target_size();
        let tokens = NameTokens { project: spec.meta.project_name.clone(), camera: spec.meta.camera_name.clone(), w: tw, h: th, ..Default::default() };
        let job = JobSpec {
            id,
            label: format!("{} · {}×{}", spec.meta.project_name, tw, th),
            source,
            spec,
            dir,
            template: self.render_ui.prefs.template.clone(),
            overwrite: self.render_ui.prefs.overwrite,
            counter: self.render_ui.prefs.counter,
            frames: None,
            skip_existing: false,
            reuse: None,
            tokens,
        };
        self.render_submit(ctx, job, notes);
    }

    fn render_submit(&mut self, ctx: &egui::Context, job: JobSpec, notes: Vec<String>) {
        let cancel = Arc::new(AtomicBool::new(false));
        let view = JobView {
            id: job.id,
            label: job.label.clone(),
            status: JobStatus::Queued,
            frac: 0.0,
            stage: None,
            frame_idx: 0,
            total: 1,
            files: Vec::new(),
            notes,
            cancel: cancel.clone(),
            spec: job.clone(),
            render_ms: 0.0,
            alloc: job.reuse.clone(),
            started: std::time::Instant::now(),
            finished: None,
        };
        self.render_service(ctx).submit(job, cancel);
        self.render_ui.jobs.push(view);
        if self.render_ui.jobs.len() > 40 {
            let drop = self.render_ui.jobs.iter().position(|j| !matches!(j.status, JobStatus::Queued | JobStatus::Running));
            if let Some(i) = drop {
                self.render_ui.jobs.remove(i);
            }
        }
        self.render_ui.prefs.queue_open = true;
        self.render_say(crate::i18n::tr("render.queued"));
    }

    /// Cancela um job (o render em andamento termina a etapa atual e descarta o quadro).
    pub(crate) fn render_cancel(&mut self, id: u64) {
        if let Some(j) = self.render_ui.jobs.iter().find(|j| j.id == id) {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Retomar: reenfileira um job cancelado/falho (sequências pulam o que já está no disco).
    pub(crate) fn render_resume(&mut self, ctx: &egui::Context, id: u64) {
        let Some(j) = self.render_ui.jobs.iter().find(|j| j.id == id) else { return };
        let mut spec = j.spec.clone();
        let notes = j.notes.clone();
        spec.id = self.render_ui.next_id;
        self.render_ui.next_id += 1;
        spec.skip_existing = true;
        spec.overwrite = true;
        spec.reuse = j.alloc.clone();
        self.render_submit(ctx, spec, notes);
    }

    /// Drena os eventos da fila (a cada quadro do workspace).
    pub(crate) fn render_poll(&mut self, ctx: &egui::Context) {
        let Some(svc) = self.render_ui.service.as_ref() else { return };
        let events: Vec<Event> = svc.events.try_iter().collect();
        for e in events {
            match e {
                Event::Started { id, total } => {
                    if let Some(j) = self.render_ui.jobs.iter_mut().find(|j| j.id == id) {
                        j.status = JobStatus::Running;
                        j.total = total;
                    }
                }
                Event::Progress { id, frame_idx, total, stage, frac } => {
                    if let Some(j) = self.render_ui.jobs.iter_mut().find(|j| j.id == id) {
                        j.status = JobStatus::Running;
                        j.frame_idx = frame_idx;
                        j.total = total;
                        j.stage = Some(stage);
                        j.frac = (frame_idx as f32 + frac) / total.max(1) as f32;
                    }
                }
                Event::FrameDone { id, files, notes, render_ms, alloc, .. } => {
                    if let Some(j) = self.render_ui.jobs.iter_mut().find(|j| j.id == id) {
                        j.files.extend(files);
                        j.notes.extend(notes);
                        j.render_ms = render_ms;
                        j.alloc = Some(alloc);
                    }
                    self.render_ui.history_dir = None; // recarrega o histórico
                }
                Event::FrameSkipped { id, .. } => {
                    if let Some(j) = self.render_ui.jobs.iter_mut().find(|j| j.id == id) {
                        j.notes.push(crate::i18n::tr("render.note.skipped").to_string());
                    }
                }
                Event::Finished { id, next_counter } => {
                    let mut label = String::new();
                    if let Some(j) = self.render_ui.jobs.iter_mut().find(|j| j.id == id) {
                        j.status = JobStatus::Done;
                        j.frac = 1.0;
                        j.finished = Some(j.started.elapsed());
                        label = j.label.clone();
                    }
                    self.render_ui.prefs.counter = self.render_ui.prefs.counter.max(next_counter);
                    self.render_ui.history_dir = None;
                    self.render_say(format!("{}: {label}", crate::i18n::tr("render.done")));
                }
                Event::Failed { id, error } => {
                    if let Some(j) = self.render_ui.jobs.iter_mut().find(|j| j.id == id) {
                        j.status = JobStatus::Failed(error.clone());
                        j.finished = Some(j.started.elapsed());
                    }
                    self.render_say(format!("{}: {error}", crate::i18n::tr("render.failed")));
                }
                Event::Cancelled { id } => {
                    if let Some(j) = self.render_ui.jobs.iter_mut().find(|j| j.id == id) {
                        j.status = JobStatus::Cancelled;
                        j.finished = Some(j.started.elapsed());
                    }
                    self.render_say(crate::i18n::tr("render.cancelled"));
                }
            }
        }
        // histórico da pasta atual (recarrega quando a pasta muda ou um job terminou)
        let dir = self.render_ui.out_dir();
        if self.render_ui.history_dir.as_ref() != Some(&dir) {
            self.render_ui.history = history::load(&dir);
            self.render_ui.history_dir = Some(dir);
        }
        let _ = ctx;
    }

    /// Miniatura do histórico (carrega do disco, no máximo algumas por quadro).
    pub(crate) fn render_thumb(&mut self, ctx: &egui::Context, entry: &HistoryEntry) -> Option<egui::TextureHandle> {
        let dir = self.render_ui.out_dir();
        let path = entry.thumb_path(&dir)?;
        let key = path.display().to_string();
        if let Some(t) = self.render_ui.thumbs.get(&key) {
            return t.clone();
        }
        let tex = image::open(&path).ok().map(|im| {
            let rgba = im.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            ctx.load_texture(key.clone(), egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()), egui::TextureOptions::LINEAR)
        });
        self.render_ui.thumbs.insert(key, tex.clone());
        tex
    }

    /// Envia uma saída para a Biblioteca como ativo de imagem.
    pub(crate) fn render_to_library(&mut self, entry: &HistoryEntry) {
        let path = entry.path(&self.render_ui.out_dir());
        let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "render".into());
        let r = self.lib_workspace().import_render_image(&name, &path);
        self.render_say(match r {
            Ok(m) => m,
            Err(e) => format!("{}: {e}", crate::i18n::tr("render.lib.error")),
        });
    }

    /// Câmeras de cena do projeto (nós com `NodeSource::Camera`).
    pub(crate) fn render_scene_cameras(&self) -> Vec<(NodeId, String)> {
        self.core
            .project()
            .scene
            .nodes
            .iter()
            .filter(|n| matches!(n.source, Some(NodeSource::Camera { .. })))
            .map(|n| (n.node_id.clone(), n.name.clone()))
            .collect()
    }

    /// Há algo rodando ou cenário de captura em andamento? (o screenshot espera)
    pub(crate) fn render_busy(&self) -> bool {
        self.render_ui.script.as_ref().is_some_and(|s| !s.is_done())
    }

    /// `ANIGO_TRACE_RENDER=1`: uma linha `rendertrace ...` por mudança de estado, para o smoke de xdotool
    /// (scripts/native_render_smoke.sh). Coordenadas em pixels físicos da janela.
    pub(crate) fn render_trace(&mut self, ctx: &egui::Context) {
        if std::env::var_os("ANIGO_TRACE_RENDER").is_none() {
            return;
        }
        let ppp = ctx.pixels_per_point();
        let p = self.core.project();
        let h = self.core.history();
        let o = p.render.output;
        let job = self.render_ui.jobs.last();
        let status = match job.map(|j| &j.status) {
            None => "none".to_string(),
            Some(JobStatus::Queued) => "queued".into(),
            Some(JobStatus::Running) => "running".into(),
            Some(JobStatus::Done) => "done".into(),
            Some(JobStatus::Failed(_)) => "failed".into(),
            Some(JobStatus::Cancelled) => "cancelled".into(),
        };
        let files = job.map(|j| j.files.iter().map(|f| f.path.display().to_string().replace(' ', "%20")).collect::<Vec<_>>().join(",")).unwrap_or_default();
        let r = &self.render_ui;
        let pt = |p: egui::Pos2| format!("{:.0},{:.0}", p.x * ppp, p.y * ppp);
        let line = format!(
            "rendertrace w={} h={} ss={} transparent={} undo={} redo={} jobs={} job={} frac={:.2} files={} render_btn={} swap_btn={} cancel_btn={} img={:.0},{:.0},{:.0},{:.0} fov={:.1} script={}",
            o.width,
            o.height,
            o.supersample,
            o.transparent,
            h.undo_depth(),
            h.redo_depth(),
            r.jobs.len(),
            status,
            job.map(|j| j.frac).unwrap_or(0.0),
            if files.is_empty() { "-".to_string() } else { files },
            pt(r.render_btn),
            pt(r.swap_btn),
            pt(r.cancel_btn),
            r.img_rect.min.x * ppp,
            r.img_rect.min.y * ppp,
            r.img_rect.max.x * ppp,
            r.img_rect.max.y * ppp,
            p.scene.camera.camera.fov_y.to_degrees(),
            match &r.script {
                None => "none",
                Some(s) if s.is_done() => "done",
                Some(_) => "running",
            },
        );
        if self.render_ui.trace.as_deref() != Some(line.as_str()) {
            eprintln!("{line}");
            self.render_ui.trace = Some(line);
        }
    }

    pub(crate) fn render_aovs_enabled(&self) -> Vec<&'static str> {
        enabled_aovs(&self.core.project().render.output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_survive_json_and_old_sessions_get_defaults() {
        let p = RenderPrefs { dir: "/x/y".into(), template: "a_{n:03}".into(), counter: 7, overwrite: true, ..Default::default() };
        let back: RenderPrefs = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!((back.dir.as_str(), back.counter, back.overwrite), ("/x/y", 7, true));
        let old: RenderPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(old.template, DEFAULT_TEMPLATE);
        let mut ui = RenderUi::default();
        ui.apply_prefs(&RenderPrefs { counter: 0, template: "  ".into(), sensor: 99, ..Default::default() });
        assert_eq!(ui.prefs.counter, 1);
        assert_eq!(ui.prefs.template, DEFAULT_TEMPLATE);
        assert!(ui.prefs.sensor < crate::render_out::framing::SENSORS.len());
    }

    #[test]
    fn directory_status_never_creates_anything() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(dir_status(dir.path()), DirStatus::Ready);
        let nope = dir.path().join("nova");
        assert_eq!(dir_status(&nope), DirStatus::WillCreate);
        assert!(!nope.exists());
        let f = dir.path().join("arquivo");
        std::fs::write(&f, b"x").unwrap();
        assert_eq!(dir_status(&f), DirStatus::NotADirectory);
    }

    #[test]
    fn ffmpeg_hint_is_plain_text_with_the_frame_pattern() {
        let c = ffmpeg_command(std::path::Path::new("/r"), "cena", 24);
        assert!(c.contains("-framerate 24") && c.contains("cena_%04d.png") && c.contains("cena.mp4"), "{c}");
    }

    #[test]
    fn tab_follows_the_tool_shelf_order() {
        assert_eq!(RenderTab::from_tool_index(0), RenderTab::Camera);
        assert_eq!(RenderTab::from_tool_index(1), RenderTab::Passes);
        assert_eq!(RenderTab::from_tool_index(2), RenderTab::Output);
        assert_eq!(RenderTab::from_tool_index(9), RenderTab::Camera);
    }
}
