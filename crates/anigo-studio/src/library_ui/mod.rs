//! Workspace Biblioteca (Asset Browser) sobre `anigo-assets`.
//!
//! * [`data`] — consultas paginadas por keyset em threads (a UI nunca espera o SQLite), facetas, detalhes, comandos.
//! * [`thumb_service`] + [`cel_thumb`] — miniaturas pedidas só para o que está visível; render 3D cel em fila serial.
//! * `grid`, `side`, `details`, `dialogs`, `jobs`, `preview` — a UI egui (grade virtualizada, árvore, detalhes,
//!   importação, gestão).
//! * [`synth`] / [`scenario`] — bibliotecas sintéticas para cenários de screenshot, testes e benchmark.
//!
//! O estado vive em [`LibraryWorkspace`]; o app só repassa o `Ui` e executa as [`HostAction`] devolvidas.

pub mod cel_thumb;
pub mod data;
pub mod scenario;
pub mod synth;
pub mod thumb_service;

mod details;
mod dialogs;
mod grid;
mod jobs;
mod preview;
mod side;
mod widgets;

use crate::theme;
use anigo_assets::*;
use data::*;
use std::collections::{HashSet, VecDeque};
use std::result::Result;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thumb_service::{TexCache, ThumbService};

/// Orçamento de texturas egui das miniaturas (VRAM/RAM da GPU).
const TEX_BUDGET: usize = 96 * 1024 * 1024;

/// Carga arrastada de um card (gancho de drag-and-drop interno; ver `docs/native/BIBLIOTECA_UI.md`).
#[derive(Debug, Clone)]
pub struct AssetDragPayload {
    pub ids: Vec<AssetId>,
    pub label: String,
    pub kind: AssetKind,
}

/// O que o workspace pede ao app (que é dono do resto do studio).
#[derive(Debug, Clone, PartialEq)]
pub enum HostAction {
    /// Abrir o ativo na Modelagem (importa a malha como objeto editável).
    EditInModeling { path: PathBuf, format: String, name: String },
    /// Mensagem curta para a barra de status.
    Status(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Grid,
    List,
}

/// De onde a biblioteca vem e como ela é preparada.
#[derive(Debug, Clone)]
pub struct LibrarySpec {
    pub home: PathBuf,
    /// Pastas somente-leitura adicionadas como locais LIGADOS na primeira abertura (nome, caminho).
    pub linked_defaults: Vec<(String, PathBuf)>,
    /// Escaneia os locais ligados ao abrir (incremental por mtime+tamanho).
    pub scan_on_open: bool,
}

pub(crate) enum OpenState {
    Closed,
    Opening(crossbeam_channel::Receiver<Result<Library, String>>),
    Ready,
    Failed(String),
}

/// Peças que só existem com a biblioteca aberta.
pub(crate) struct Runtime {
    pub lib: Library,
    pub q: QueryWorker,
    pub misc: MiscWorker,
    pub thumbs: ThumbService,
    pub cel: Arc<cel_thumb::CelThumbnailRenderer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToastKind {
    Info,
    Error,
}

#[derive(Default)]
pub(crate) struct Selection {
    pub ids: HashSet<AssetId>,
    /// Último clicado (alvo do painel de detalhes).
    pub primary: Option<AssetId>,
    /// Índice (na lista) do âncora de Shift e do foco do teclado.
    pub anchor: Option<usize>,
    pub cursor: Option<usize>,
}

impl Selection {
    pub fn clear(&mut self) {
        self.ids.clear();
        self.primary = None;
        self.anchor = None;
        self.cursor = None;
    }
    pub fn set_one(&mut self, id: AssetId, idx: usize) {
        self.ids.clear();
        self.ids.insert(id);
        self.primary = Some(id);
        self.anchor = Some(idx);
        self.cursor = Some(idx);
    }
    pub fn toggle(&mut self, id: AssetId, idx: usize) {
        if !self.ids.remove(&id) {
            self.ids.insert(id);
            self.primary = Some(id);
        } else if self.primary == Some(id) {
            self.primary = self.ids.iter().next().copied();
        }
        self.anchor = Some(idx);
        self.cursor = Some(idx);
    }
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    pub fn contains(&self, id: &AssetId) -> bool {
        self.ids.contains(id)
    }
}

/// Resultado de tarefas pontuais em thread (duplicados, uso de disco, relink...).
pub(crate) enum TaskOut {
    Duplicates(Result<Vec<DuplicateGroup>, String>),
    DiskUsage(DiskUsage),
    RelinkCandidates { id: AssetId, result: Result<Vec<PathBuf>, String> },
    Preview { path: PathBuf, result: Result<Preview, String> },
    Message { text: String, error: bool, refresh: Refresh },
    TrashResult { trashed: usize, blocked: Vec<(AssetId, usize)>, error: Option<String> },
}

#[derive(Debug, Clone, Default)]
pub struct DiskUsage {
    pub home_bytes: u64,
    pub assets_bytes: u64,
    pub trash_bytes: u64,
    pub index_bytes: u64,
    pub thumbs_bytes: u64,
    pub elapsed_ms: u64,
}

pub struct LibraryWorkspace {
    pub(crate) spec: Option<LibrarySpec>,
    pub(crate) open: OpenState,
    pub(crate) rt: Option<Runtime>,
    pub(crate) wake: Wake,
    pub(crate) ctx: Option<egui::Context>,

    // consulta e visão
    pub(crate) filters: Filters,
    pub(crate) list: PagedList,
    pub(crate) view: ViewMode,
    pub(crate) card_size: f32,
    pub(crate) selection: Selection,
    pub(crate) search_text: String,
    pub(crate) search_edited_at: Option<Instant>,
    pub(crate) focus_search: bool,
    pub(crate) pending_scroll: Option<f32>,
    /// Faixa de itens visível no último quadro (primeiro, último) e colunas.
    pub(crate) visible: (usize, usize),
    pub(crate) cols: usize,
    pub(crate) range_tag: u32,

    // dados auxiliares
    pub(crate) facets: Facets,
    pub(crate) facets_loaded: bool,
    pub(crate) detail: Option<Arc<DetailData>>,
    pub(crate) detail_for: Option<AssetId>,
    pub(crate) detail_error: Option<String>,

    // miniaturas
    pub(crate) tex: TexCache,
    pub(crate) thumb_unavailable_reasons: usize,

    // jobs e tarefas
    pub(crate) jobs: jobs::Jobs,
    pub(crate) tasks_tx: crossbeam_channel::Sender<TaskOut>,
    pub(crate) tasks_rx: crossbeam_channel::Receiver<TaskOut>,

    // diálogos
    pub(crate) dlg: dialogs::Dialogs,
    pub(crate) preview3d: preview::Preview3d,
    pub(crate) details_ui: details::DetailsUi,
    pub(crate) side_ui: side::SideUi,

    // feedback
    pub(crate) toast: Option<(String, ToastKind, Instant)>,
    pub(crate) actions: Vec<HostAction>,
    pub(crate) frame: u64,
    /// IDs de ativos autorais (nome do objeto da Modelagem -> ativo) para exportar como nova revisão.
    pub(crate) authored: std::collections::HashMap<String, AssetId>,
    pub(crate) scroll_y: f32,
    pub(crate) viewport_h: f32,
    pub(crate) row_h: f32,
    pub scenario: Option<scenario::Run>,
}

impl LibraryWorkspace {
    /// Workspace que abre a biblioteca descrita por `spec` em segundo plano.
    pub fn new(spec: LibrarySpec) -> Self {
        let mut w = Self::empty(Some(spec));
        w.open = OpenState::Closed;
        w
    }

    /// Workspace sobre uma biblioteca já aberta (testes, cenários).
    pub fn with_library(lib: Library) -> Self {
        let mut w = Self::empty(None);
        w.attach(lib);
        w
    }

    fn empty(spec: Option<LibrarySpec>) -> Self {
        let (tasks_tx, tasks_rx) = crossbeam_channel::unbounded();
        Self {
            spec,
            open: OpenState::Closed,
            rt: None,
            wake: Arc::new(|| {}),
            ctx: None,
            filters: Filters::default(),
            list: PagedList::new(),
            view: ViewMode::Grid,
            card_size: 150.0,
            selection: Selection::default(),
            search_text: String::new(),
            search_edited_at: None,
            focus_search: false,
            pending_scroll: None,
            visible: (0, 0),
            cols: 1,
            range_tag: 0,
            facets: Facets::default(),
            facets_loaded: false,
            detail: None,
            detail_for: None,
            detail_error: None,
            tex: TexCache::new(TEX_BUDGET),
            thumb_unavailable_reasons: 0,
            jobs: jobs::Jobs::default(),
            tasks_tx,
            tasks_rx,
            dlg: dialogs::Dialogs::default(),
            preview3d: preview::Preview3d::default(),
            details_ui: details::DetailsUi::default(),
            side_ui: side::SideUi { root_linked: true, ..Default::default() },
            toast: None,
            actions: Vec::new(),
            frame: 0,
            authored: Default::default(),
            scroll_y: 0.0,
            viewport_h: 600.0,
            row_h: 190.0,
            scenario: None,
        }
    }

    /// Liga o acordar da UI (chamado quando uma thread de fundo termina algo).
    pub fn set_context(&mut self, ctx: &egui::Context) {
        if self.ctx.is_none() {
            let c = ctx.clone();
            self.wake = Arc::new(move || c.request_repaint());
            self.ctx = Some(ctx.clone());
        }
    }

    pub fn is_ready(&self) -> bool {
        self.rt.is_some()
    }

    pub fn library(&self) -> Option<&Library> {
        self.rt.as_ref().map(|r| &r.lib)
    }

    /// Inicia a abertura em segundo plano (idempotente).
    pub fn ensure_open(&mut self) {
        if !matches!(self.open, OpenState::Closed) {
            return;
        }
        let Some(spec) = self.spec.clone() else { return };
        let (tx, rx) = crossbeam_channel::bounded(1);
        let wake = self.wake.clone();
        self.open = OpenState::Opening(rx);
        std::thread::Builder::new()
            .name("anigo-lib-open".into())
            .spawn(move || {
                let r = open_library(&spec);
                let _ = tx.send(r);
                wake();
            })
            .expect("thread de abertura");
    }

    /// Espera a abertura terminar (usado pelo export da Modelagem, que precisa da biblioteca agora).
    pub fn open_blocking(&mut self) -> Result<(), String> {
        self.ensure_open();
        if let OpenState::Opening(rx) = &self.open {
            if let Ok(r) = rx.recv() {
                self.finish_open(r);
            }
        }
        match &self.open {
            OpenState::Ready => Ok(()),
            OpenState::Failed(e) => Err(e.clone()),
            _ => Err("a biblioteca não está disponível".into()),
        }
    }

    fn finish_open(&mut self, r: Result<Library, String>) {
        match r {
            Ok(lib) => {
                self.attach(lib);
                if self.spec.as_ref().is_some_and(|s| s.scan_on_open) {
                    self.queue_scan_all_linked();
                }
            }
            Err(e) => self.open = OpenState::Failed(e),
        }
    }

    fn attach(&mut self, lib: Library) {
        let cel = Arc::new(cel_thumb::CelThumbnailRenderer::new());
        let renderer: Arc<dyn ThumbnailRenderer> = Arc::new(ChainRenderer::new(vec![cel.clone(), default_cpu_renderer()]));
        self.attach_with_renderer(lib, cel, renderer);
    }

    /// Como `attach`, mas com o renderer escolhido (testes usam só o renderer CPU).
    pub fn attach_with_renderer(&mut self, lib: Library, cel: Arc<cel_thumb::CelThumbnailRenderer>, renderer: Arc<dyn ThumbnailRenderer>) {
        let wake = self.wake.clone();
        let q = QueryWorker::spawn(lib.clone(), wake.clone());
        let misc = MiscWorker::spawn(lib.clone(), wake.clone());
        let thumbs = ThumbService::spawn(lib.clone(), renderer, wake);
        self.rt = Some(Runtime { lib, q, misc, thumbs, cel });
        self.open = OpenState::Ready;
        self.reload_rows(true);
        self.reload_facets();
    }

    // ---------------------------------------------------------------- mensagens

    pub(crate) fn info(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), ToastKind::Info, Instant::now()));
    }

    pub(crate) fn error(&mut self, text: impl Into<String>) {
        let t = text.into();
        tracing::warn!("biblioteca: {t}");
        self.toast = Some((t, ToastKind::Error, Instant::now()));
    }

    /// Mensagem de erro vinda do app (ex.: falha ao abrir um ativo na Modelagem).
    pub fn report_error(&mut self, text: String) {
        self.error(text);
    }

    /// Registra um GLB exportado pela Modelagem como ativo AUTORAL (tags `autoral`/`modelagem`).
    /// Se já existe um ativo autoral com o mesmo nome, o conteúdo novo entra como NOVA REVISÃO (mesma identidade).
    pub fn import_authored(&mut self, name: &str, glb: &std::path::Path) -> Result<String, String> {
        self.open_blocking()?;
        let rt = self.rt.as_ref().ok_or("a biblioteca não está aberta")?;
        let lib = rt.lib.clone();
        let existing = self
            .authored
            .get(name)
            .copied()
            .filter(|id| lib.row(id).ok().flatten().is_some_and(|r| r.state == AssetState::Active))
            .or_else(|| {
                let q = Query { text: Some(name.to_string()), tags: vec!["autoral".into()], limit: 50, ..Default::default() };
                lib.query(&q).ok()?.items.into_iter().find(|r| r.name == name).map(|r| r.id)
            });
        let opts = ImportOptions {
            name: Some(name.to_string()),
            tags: vec!["autoral".into(), "modelagem".into()],
            kind: Some(AssetKind::Model),
            author: Some("ANIGO Studio".into()),
            replace: existing,
            ..Default::default()
        };
        let rep = lib.import_blocking(glb, opts).map_err(|e| e.to_string())?;
        let msg = if let Some(id) = rep.imported.first() {
            self.authored.insert(name.to_string(), *id);
            format!("\"{name}\" registrado como ativo autoral")
        } else if let Some(id) = rep.revised.first() {
            self.authored.insert(name.to_string(), *id);
            format!("\"{name}\": nova revisão do ativo autoral")
        } else if let Some((_, id)) = rep.duplicates.first() {
            self.authored.insert(name.to_string(), *id);
            format!("\"{name}\": já existe na biblioteca com o mesmo conteúdo")
        } else if let Some((p, e)) = rep.failed.first() {
            return Err(format!("{}: {e}", p.display()));
        } else {
            return Err("nada foi importado".into());
        };
        self.reload_rows(false);
        self.reload_facets();
        Ok(msg)
    }

    /// Texto da barra de status do app quando este workspace está ativo.
    pub fn status_text(&self) -> String {
        if let Some((t, _, at)) = &self.toast {
            if at.elapsed() < Duration::from_secs(12) {
                return t.clone();
            }
        }
        match &self.open {
            OpenState::Opening(_) | OpenState::Closed => crate::i18n::tr("lib.opening").to_string(),
            OpenState::Failed(e) => format!("{}: {e}", crate::i18n::tr("lib.open_failed")),
            OpenState::Ready => format!("{} {}", self.list.total_or_last(), crate::i18n::tr("lib.items")),
        }
    }

    // ---------------------------------------------------------------- consulta

    /// Recarrega a lista. `hard`: filtro mudou (zera páginas); senão mantém as páginas até as novas chegarem.
    pub(crate) fn reload_rows(&mut self, hard: bool) {
        let Some(rt) = &self.rt else { return };
        let gen = if hard { self.list.reset() } else { self.list.refresh() };
        rt.q.set_query(gen, self.filters.to_query());
        if hard {
            self.pending_scroll = Some(0.0);
        }
    }

    pub(crate) fn reload_facets(&self) {
        if let Some(rt) = &self.rt {
            rt.misc.request_facets();
        }
    }

    pub(crate) fn reload_detail(&self) {
        if let (Some(rt), Some(id)) = (&self.rt, self.detail_for) {
            rt.misc.request_detail(id);
        }
    }

    /// Aplica uma mudança de filtro/ordenação.
    pub(crate) fn filters_changed(&mut self) {
        self.reload_rows(true);
    }

    pub(crate) fn set_section(&mut self, s: Section) {
        if self.filters.section != s {
            self.filters.section = s;
            self.selection.clear();
            self.sync_detail_target();
            self.filters_changed();
        }
    }

    /// Roda um comando de mutação no worker e recarrega o que `refresh` pedir quando terminar.
    pub(crate) fn command(&self, label: &str, refresh: Refresh, f: impl FnOnce(&Library) -> Result<String, String> + Send + 'static) {
        if let Some(rt) = &self.rt {
            rt.misc.command(label, refresh, f);
        }
    }

    /// Tarefa pontual em thread própria (resultado volta por `TaskOut`).
    pub(crate) fn spawn_task(&self, f: impl FnOnce(Library) -> TaskOut + Send + 'static) {
        let Some(rt) = &self.rt else { return };
        let (lib, tx, wake) = (rt.lib.clone(), self.tasks_tx.clone(), self.wake.clone());
        std::thread::Builder::new()
            .name("anigo-lib-task".into())
            .spawn(move || {
                let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(lib))).unwrap_or(TaskOut::Message { text: "falha interna (panic) na tarefa".into(), error: true, refresh: Refresh::NONE });
                let _ = tx.send(out);
                wake();
            })
            .ok();
    }

    // ---------------------------------------------------------------- seleção

    /// Alvo do painel de detalhes = único selecionado (ou o último clicado).
    pub(crate) fn sync_detail_target(&mut self) {
        let want = if self.selection.len() == 1 { self.selection.primary } else { None };
        if want != self.detail_for {
            self.detail_for = want;
            self.detail = None;
            self.detail_error = None;
            self.details_ui.reset_edit();
            if let (Some(rt), Some(id)) = (&self.rt, want) {
                rt.misc.request_detail(id);
            }
        }
    }

    pub(crate) fn selected_ids(&self) -> Vec<AssetId> {
        self.selection.ids.iter().copied().collect()
    }

    pub(crate) fn click_item(&mut self, idx: usize, id: AssetId, mods: egui::Modifiers) {
        if mods.shift {
            let anchor = self.selection.anchor.unwrap_or(idx);
            if !mods.command {
                self.selection.ids.clear();
            }
            self.selection.ids.insert(id);
            self.selection.primary = Some(id);
            self.selection.cursor = Some(idx);
            self.range_tag = self.range_tag.wrapping_add(1);
            if let Some(rt) = &self.rt {
                rt.q.request_range(self.list.gen, anchor, idx, self.range_tag);
            }
        } else if mods.command {
            self.selection.toggle(id, idx);
        } else {
            self.selection.set_one(id, idx);
        }
        self.sync_detail_target();
    }

    pub(crate) fn select_all(&mut self) {
        let total = self.list.total_or_last();
        if total == 0 {
            return;
        }
        self.range_tag = self.range_tag.wrapping_add(1);
        if let Some(rt) = &self.rt {
            rt.q.request_range(self.list.gen, 0, total - 1, self.range_tag);
        }
    }

    // ---------------------------------------------------------------- quadro

    /// Recolhe tudo o que as threads de fundo produziram (chame uma vez por quadro, antes de desenhar).
    pub fn pump(&mut self, ctx: &egui::Context) {
        self.set_context(ctx);
        self.frame += 1;
        if let OpenState::Opening(rx) = &self.open {
            if let Ok(r) = rx.try_recv() {
                self.finish_open(r);
            }
        }
        let Some(rt) = &self.rt else { return };
        let (qrx, mrx, trx) = (rt.q.rx.clone(), rt.misc.rx.clone(), rt.thumbs.rx.clone());

        for r in qrx.try_iter() {
            match r {
                QResp::Range { gen, ids, .. } if gen == self.list.gen => {
                    self.selection.ids.extend(ids);
                    self.sync_detail_target();
                }
                QResp::Range { .. } => {}
                other => self.list.apply(other),
            }
        }
        if let Some(e) = self.list.error.take() {
            self.error(e);
        }

        let mut refresh_rows = false;
        let mut refresh_facets = false;
        let mut refresh_detail = false;
        for r in mrx.try_iter() {
            match r {
                MResp::Facets(f) => {
                    self.facets = *f;
                    self.facets_loaded = true;
                }
                MResp::Detail(id, res) => {
                    if Some(id) == self.detail_for {
                        match *res {
                            Ok(d) => {
                                self.detail = Some(Arc::new(d));
                                self.detail_error = None;
                            }
                            Err(e) => {
                                self.detail = None;
                                self.detail_error = Some(e);
                            }
                        }
                    }
                }
                MResp::CmdDone { label, refresh, result } => {
                    match result {
                        Ok(msg) => {
                            if !msg.is_empty() {
                                self.info(msg);
                            }
                        }
                        Err(e) => self.error(format!("{label}: {e}")),
                    }
                    refresh_rows |= refresh.rows;
                    refresh_facets |= refresh.facets;
                    refresh_detail |= refresh.detail;
                }
            }
        }
        let mut uploads = 0;
        for m in trx.try_iter().take(32) {
            uploads += 1;
            match m {
                thumb_service::ThumbMsg::Ready { hash, size, image } => self.tex.insert(ctx, hash, size, &image),
                thumb_service::ThumbMsg::Unavailable { hash, .. } => {
                    self.tex.unavailable.insert(hash);
                    self.thumb_unavailable_reasons += 1;
                }
            }
        }
        if uploads >= 32 {
            ctx.request_repaint();
        }
        for t in self.tasks_rx.clone().try_iter() {
            self.apply_task(t, &mut refresh_rows, &mut refresh_facets, &mut refresh_detail);
        }
        self.poll_jobs(&mut refresh_rows, &mut refresh_facets);
        if refresh_rows {
            self.reload_rows(false);
        }
        if refresh_facets {
            self.reload_facets();
        }
        if refresh_detail {
            self.reload_detail();
        }
        self.apply_search_debounce(ctx);
    }

    fn apply_search_debounce(&mut self, ctx: &egui::Context) {
        if let Some(at) = self.search_edited_at {
            let wait = Duration::from_millis(150);
            if at.elapsed() >= wait {
                self.search_edited_at = None;
                if self.filters.text != self.search_text {
                    self.filters.text = self.search_text.clone();
                    self.filters_changed();
                }
            } else {
                ctx.request_repaint_after(wait - at.elapsed());
            }
        }
    }

    fn apply_task(&mut self, t: TaskOut, rows: &mut bool, facets: &mut bool, detail: &mut bool) {
        match t {
            TaskOut::Duplicates(r) => self.dlg.set_duplicates(r),
            TaskOut::DiskUsage(u) => {
                self.dlg.disk = Some(u);
                self.dlg.disk_busy = false;
            }
            TaskOut::TrashResult { trashed, blocked, error } => {
                if let Some(e) = error {
                    self.error(e);
                } else {
                    if trashed > 0 {
                        self.info(format!("{trashed} {}", crate::i18n::tr("lib.msg.trashed")));
                        self.selection.clear();
                        self.sync_detail_target();
                    }
                    if !blocked.is_empty() {
                        self.dlg.confirm = Some(dialogs::Confirm::ForceTrash(blocked));
                    }
                }
                *rows = true;
                *facets = true;
                *detail = true;
            }
            TaskOut::RelinkCandidates { id, result } => self.dlg.set_relink_candidates(id, result),
            TaskOut::Preview { path, result } => self.dlg.set_import_preview(path, result),
            TaskOut::Message { text, error, refresh } => {
                if error {
                    self.error(text);
                } else {
                    self.info(text);
                }
                *rows |= refresh.rows;
                *facets |= refresh.facets;
                *detail |= refresh.detail;
            }
        }
    }

    /// Há algo carregando (consulta, miniaturas visíveis, jobs)? Os cenários esperam isto antes do screenshot.
    pub fn is_settled(&self) -> bool {
        let Some(rt) = &self.rt else { return matches!(self.open, OpenState::Failed(_)) };
        self.list.is_settled() && self.facets_loaded && rt.thumbs.is_idle() && !self.jobs.any_running() && self.search_edited_at.is_none() && (self.detail_for.is_none() || self.detail.is_some() || self.detail_error.is_some())
    }

    /// Desenha o workspace inteiro e devolve as ações para o app.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Vec<HostAction> {
        let ctx = ui.ctx().clone();
        self.pump(&ctx);
        self.ensure_open();
        match &self.open {
            OpenState::Closed | OpenState::Opening(_) => {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() * 0.4);
                        ui.spinner();
                        ui.label(egui::RichText::new(crate::i18n::tr("lib.opening")).color(theme::TEXT_DIM));
                    });
                });
                return std::mem::take(&mut self.actions);
            }
            OpenState::Failed(e) => {
                let e = e.clone();
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() * 0.4);
                        ui.label(egui::RichText::new(crate::i18n::tr("lib.open_failed")).strong().size(18.0));
                        ui.label(egui::RichText::new(e).color(egui::Color32::from_rgb(0xff, 0x70, 0x70)));
                    });
                });
                return std::mem::take(&mut self.actions);
            }
            OpenState::Ready => {}
        }
        self.handle_drops(&ctx);
        self.scenario_tick();
        self.handle_global_input(&ctx);
        self.left_panel(ui);
        self.right_panel(ui);
        self.bottom_strip(ui);
        egui::CentralPanel::default().show(ui, |ui| {
            self.toolbar(ui);
            ui.add_space(2.0);
            self.center(ui);
        });
        self.dialogs(&ctx);
        self.drop_overlay(&ctx);
        self.drag_ghost(&ctx);
        std::mem::take(&mut self.actions)
    }
}

/// Abre (criando) a biblioteca e registra os locais ligados padrão.
fn open_library(spec: &LibrarySpec) -> Result<Library, String> {
    let lib = Library::open_with(&spec.home, OpenOptions { pipeline_ver: cel_thumb::PIPELINE_VER, ..Default::default() }).map_err(|e| e.to_string())?;
    let _ = lib.cleanup_staging(24 * 3600);
    let existing: Vec<PathBuf> = lib.roots().map_err(|e| e.to_string())?.into_iter().map(|r| r.path).collect();
    for (name, path) in &spec.linked_defaults {
        let Ok(canon) = path.canonicalize() else { continue };
        if existing.iter().any(|p| *p == canon) || !canon.is_dir() {
            continue;
        }
        if let Err(e) = lib.add_root(&canon, RootMode::Linked, Some(name)) {
            tracing::warn!("local ligado padrão {} ignorado: {e}", canon.display());
        }
    }
    Ok(lib)
}

#[cfg(test)]
mod tests;
