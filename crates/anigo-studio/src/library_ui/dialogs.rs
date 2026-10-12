//! Janelas e modais da Biblioteca: confirmações, importação (+ relatório por arquivo), navegador de pastas,
//! gestão (uso de disco, cache, integridade, índice), duplicados e relink. Também a faixa inferior com o
//! progresso dos jobs e a sobreposição de arrastar-e-soltar de arquivos do sistema.

use super::widgets::*;
use super::*;
use crate::i18n::tr;
use crate::icons;
use egui::{Color32, RichText, Ui, Vec2};
use jobs::EntryKind;
use std::collections::HashMap;
use std::path::Path;

pub enum Confirm {
    EmptyTrash,
    Purge(Vec<AssetId>),
    /// Ativos que outros ativos usam: perguntar antes de mandar para a lixeira mesmo assim.
    ForceTrash(Vec<(AssetId, usize)>),
    RemoveRoot { id: RootId, name: String, managed: bool, delete_files: bool },
    DeleteCollection(CollectionId, String),
    ClearThumbs,
    Rebuild,
}

#[derive(Debug, Clone)]
pub enum PickPurpose {
    AddRootLinked,
    AddRootManaged,
    ImportPaths,
    RelinkFile,
    RelinkSearchDir,
    RelinkRoot,
    RelinkBatch,
}

struct PickEntry {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

pub struct FolderPicker {
    purpose: PickPurpose,
    files_mode: bool,
    dir: PathBuf,
    entries: Vec<PickEntry>,
    path_edit: String,
    selected: Vec<PathBuf>,
    show_hidden: bool,
    error: Option<String>,
    truncated: bool,
}

const PICK_LIMIT: usize = 20_000;

impl FolderPicker {
    fn new(purpose: PickPurpose, files_mode: bool, start: Option<PathBuf>) -> Self {
        let dir = start.filter(|p| p.is_dir()).or_else(home_dir).unwrap_or_else(|| PathBuf::from("/"));
        let mut p = Self { purpose, files_mode, dir, entries: Vec::new(), path_edit: String::new(), selected: Vec::new(), show_hidden: false, error: None, truncated: false };
        p.reload();
        p
    }

    fn go(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.selected.clear();
        self.reload();
    }

    fn reload(&mut self) {
        self.path_edit = self.dir.display().to_string();
        self.entries.clear();
        self.error = None;
        self.truncated = false;
        match std::fs::read_dir(&self.dir) {
            Ok(rd) => {
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if !self.show_hidden && name.starts_with('.') {
                        continue;
                    }
                    let path = e.path();
                    let is_dir = e.file_type().map(|t| t.is_dir() || (t.is_symlink() && path.is_dir())).unwrap_or(false);
                    if !is_dir && (!self.files_mode || format_by_extension(&path).is_none()) {
                        continue;
                    }
                    self.entries.push(PickEntry { name, path, is_dir });
                    if self.entries.len() >= PICK_LIMIT {
                        self.truncated = true;
                        break;
                    }
                }
                self.entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
            }
            Err(e) => self.error = Some(format!("{}: {e}", self.dir.display())),
        }
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).filter(|p| p.is_dir())
}

#[derive(Default)]
pub struct ImportForm {
    pub recursive: bool,
    pub allow_dups: bool,
    pub kind: Option<AssetKind>,
    pub tags: String,
    pub author: String,
    pub license: String,
}

pub struct RelinkState {
    pub id: AssetId,
    pub name: String,
    pub file: String,
    pub search_dir: String,
    pub candidates: Option<Result<Vec<PathBuf>, String>>,
    pub searching: bool,
}

pub struct RelinkRootState {
    pub root: RootInfo,
    pub new_path: String,
}

#[derive(Default)]
pub struct Dialogs {
    pub confirm: Option<Confirm>,
    pub picker: Option<FolderPicker>,
    // importação
    pub import_open: bool,
    pub import_path: String,
    pub import_paths: Vec<PathBuf>,
    pub import_form: ImportForm,
    pub import_preview: Option<(PathBuf, Result<Preview, String>)>,
    // gestão
    pub manage_open: bool,
    pub disk: Option<DiskUsage>,
    pub disk_busy: bool,
    pub verify_rehash: bool,
    pub verify_fix: bool,
    // duplicados
    pub dups_open: bool,
    pub dups: Option<Result<Vec<DuplicateGroup>, String>>,
    pub dup_keep: HashMap<ContentHash, AssetId>,
    // relink
    pub relink: Option<RelinkState>,
    pub relink_root: Option<RelinkRootState>,
    pub relink_batch: Option<String>,
    // coleção inteligente
    pub open_smart: bool,
    pub smart_name: String,
    pub log_errors_only: bool,
}

impl Dialogs {
    pub fn any_modal_open(&self) -> bool {
        self.confirm.is_some() || self.picker.is_some() || self.import_open || self.relink.is_some() || self.relink_root.is_some() || self.relink_batch.is_some() || self.open_smart
    }
    pub fn open_import(&mut self) {
        self.import_open = true;
        if self.import_form.tags.is_empty() && !self.import_form.recursive && self.import_paths.is_empty() {
            self.import_form.recursive = true;
        }
    }
    pub fn open_manage(&mut self) {
        self.manage_open = true;
    }
    pub fn open_picker(&mut self, purpose: PickPurpose, start: Option<PathBuf>) {
        let files = matches!(purpose, PickPurpose::ImportPaths | PickPurpose::RelinkFile);
        self.picker = Some(FolderPicker::new(purpose, files, start));
    }
    pub fn open_relink(&mut self, id: AssetId, name: String) {
        self.relink = Some(RelinkState { id, name, file: String::new(), search_dir: String::new(), candidates: None, searching: false });
    }
    pub fn open_relink_root(&mut self, root: RootInfo) {
        self.relink_root = Some(RelinkRootState { new_path: root.path.display().to_string(), root });
    }
    pub fn open_relink_batch(&mut self) {
        self.relink_batch = Some(String::new());
    }
    pub fn set_duplicates(&mut self, r: Result<Vec<DuplicateGroup>, String>) {
        if let Ok(groups) = &r {
            for g in groups {
                self.dup_keep.entry(g.content_hash).or_insert_with(|| default_keeper(g));
            }
        }
        self.dups = Some(r);
    }
    pub fn set_relink_candidates(&mut self, id: AssetId, r: Result<Vec<PathBuf>, String>) {
        if let Some(s) = &mut self.relink {
            if s.id == id {
                s.candidates = Some(r);
                s.searching = false;
            }
        }
    }
    pub fn set_import_preview(&mut self, path: PathBuf, r: Result<Preview, String>) {
        self.import_preview = Some((path, r));
    }
}

/// Mantém o mais antigo, preferindo cópia gerenciada.
fn default_keeper(g: &DuplicateGroup) -> AssetId {
    g.assets
        .iter()
        .min_by_key(|a| (a.storage != StorageMode::Managed, a.imported_at))
        .map(|a| a.id)
        .unwrap_or(g.assets[0].id)
}

impl LibraryWorkspace {
    pub(super) fn open_duplicates(&mut self) {
        self.dlg.dups_open = true;
        self.dlg.dups = None;
        self.spawn_task(|lib| TaskOut::Duplicates(lib.find_duplicates(5000).map_err(|e| e.to_string())));
    }

    // ================================================================ faixa inferior

    pub(super) fn bottom_strip(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("lib_bottom").exact_size(26.0).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(6.0);
                let mut shown_job = false;
                if let Some(run) = &self.jobs.import {
                    shown_job = true;
                    let frac = if run.total > 0 { run.done as f32 / run.total as f32 } else { 0.0 };
                    ui.label(RichText::new(format!("{}  {}", icons::DOWNLOAD_SIMPLE, tr("lib.imp.importing"))).size(12.0));
                    ui.add(egui::ProgressBar::new(frac).desired_width(180.0).text(format!("{}/{} \u{00b7} {}", run.done, run.total.max(run.discovered), run.label)));
                    if !self.jobs.queue.is_empty() {
                        ui.label(small_dim(format!("+{}", self.jobs.queue.len())));
                    }
                    if ui.small_button(tr("lib.cancel")).clicked() {
                        self.cancel_imports();
                    }
                    if ui.small_button(tr("lib.imp.report")).clicked() {
                        self.jobs.log.open = true;
                    }
                }
                if let Some(s) = &self.jobs.scan {
                    shown_job = true;
                    let frac = if s.total > 0 { s.done as f32 / s.total as f32 } else { 0.0 };
                    ui.label(RichText::new(format!("{}  {}", icons::ARROWS_CLOCKWISE, tr("lib.scan.scanning"))).size(12.0));
                    ui.add(egui::ProgressBar::new(frac).desired_width(120.0).text(&s.name));
                    if ui.small_button(tr("lib.cancel")).clicked() {
                        if let Some(j) = &s.job {
                            j.cancel();
                        }
                    }
                }
                if let Some(v) = &self.jobs.verify {
                    shown_job = true;
                    let frac = if v.total > 0 { v.done as f32 / v.total as f32 } else { 0.0 };
                    ui.label(RichText::new(format!("{}  {}", icons::SHIELD_CHECK, tr("lib.verify.running"))).size(12.0));
                    ui.add(egui::ProgressBar::new(frac).desired_width(140.0).text(format!("{}/{}", v.done, v.total)));
                    if ui.small_button(tr("lib.cancel")).clicked() {
                        if let Some(j) = &v.job {
                            j.cancel();
                        }
                    }
                }
                if self.jobs.rebuild.is_some() {
                    shown_job = true;
                    ui.spinner();
                    ui.label(RichText::new(tr("lib.rebuild.running")).size(12.0));
                }
                if !shown_job {
                    if let Some((t, kind, at)) = &self.toast {
                        if at.elapsed() < Duration::from_secs(20) {
                            let col = if *kind == ToastKind::Error { err_color() } else { theme::TEXT_DIM };
                            ui.add(egui::Label::new(RichText::new(t).color(col).size(12.0)).truncate());
                        }
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(8.0);
                    if let Some(rt) = &self.rt {
                        let q = rt.thumbs.render_queue_len();
                        let gpu = match rt.cel.available() {
                            None => "\u{2014}".to_string(),
                            Some(true) => tr("lib.thumbs.gpu_ok").to_string(),
                            Some(false) => tr("lib.thumbs.gpu_off").to_string(),
                        };
                        let (loaded, rendered, failed) = rt.thumbs.stats();
                        let tip = format!(
                            "{}: {gpu}\n{}: {loaded}\n{}: {rendered}\n{}: {failed}\n{}: {} ({})",
                            tr("lib.thumbs.renderer"),
                            tr("lib.thumbs.from_cache"),
                            tr("lib.thumbs.rendered"),
                            tr("lib.thumbs.unavailable"),
                            tr("lib.thumbs.textures"),
                            self.tex.len(),
                            fmt_bytes(self.tex.bytes() as u64)
                        );
                        let color = if rt.cel.available() == Some(false) { warn_color() } else { theme::TEXT_DIM };
                        let label = if q > 0 { format!("{}  {} {}", icons::IMAGES, q, tr("lib.thumbs.queued")) } else { format!("{}  {}", icons::IMAGES, gpu) };
                        ui.label(RichText::new(label).size(11.5).color(color)).on_hover_text(tip);
                    }
                });
            });
        });
    }

    // ============================================================ dispatcher de janelas

    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        self.confirm_modal(ctx);
        self.picker_modal(ctx);
        self.import_window(ctx);
        self.report_window(ctx);
        self.manage_window(ctx);
        self.duplicates_window(ctx);
        self.relink_windows(ctx);
        self.smart_window(ctx);
    }

    // ================================================================== confirmações

    fn confirm_modal(&mut self, ctx: &egui::Context) {
        let Some(c) = self.dlg.confirm.take() else { return };
        let mut keep: Option<Confirm> = None;
        let (title, body, danger, ok_label): (String, String, bool, String) = match &c {
            Confirm::EmptyTrash => (tr("lib.trash.empty").into(), format!("{} {}", self.facets.n_trash, tr("lib.confirm.empty_trash")), true, tr("lib.trash.empty").into()),
            Confirm::Purge(ids) => (tr("lib.act.purge").into(), format!("{} {}", ids.len(), tr("lib.confirm.purge")), true, tr("lib.act.purge").into()),
            Confirm::ForceTrash(list) => (tr("lib.confirm.in_use").into(), format!("{} {}", list.len(), tr("lib.confirm.in_use.body")), false, tr("lib.confirm.trash_anyway").into()),
            Confirm::RemoveRoot { name, managed, .. } => (
                tr("lib.root.remove").into(),
                format!("{name}\n\n{}", if *managed { tr("lib.confirm.remove_managed") } else { tr("lib.confirm.remove_linked") }),
                *managed,
                tr("lib.root.remove").into(),
            ),
            Confirm::DeleteCollection(_, n) => (tr("lib.coll.delete").into(), format!("{n}\n\n{}", tr("lib.confirm.delete_collection")), false, tr("lib.coll.delete").into()),
            Confirm::ClearThumbs => (tr("lib.manage.clear_thumbs").into(), tr("lib.confirm.clear_thumbs").into(), false, tr("lib.manage.clear_thumbs").into()),
            Confirm::Rebuild => (tr("lib.manage.rebuild").into(), tr("lib.confirm.rebuild").into(), false, tr("lib.manage.rebuild").into()),
        };
        let mut c = c;
        let resp = egui::Modal::new(egui::Id::new("lib_confirm")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading(title);
            ui.add_space(6.0);
            ui.label(body);
            if let Confirm::RemoveRoot { managed: true, delete_files, .. } = &mut c {
                ui.add_space(4.0);
                ui.checkbox(delete_files, RichText::new(tr("lib.confirm.delete_files")).color(err_color()));
            }
            ui.add_space(10.0);
            let mut accepted = false;
            let mut cancelled = false;
            ui.horizontal(|ui| {
                let ok = if danger { egui::Button::new(RichText::new(&ok_label).color(Color32::WHITE)).fill(Color32::from_rgb(0xa0, 0x30, 0x30)) } else { egui::Button::new(&ok_label) };
                if ui.add(ok).clicked() {
                    accepted = true;
                }
                if ui.button(tr("lib.cancel")).clicked() {
                    cancelled = true;
                }
            });
            (accepted, cancelled)
        });
        let (accepted, cancelled) = resp.inner;
        if accepted {
            self.run_confirm(c);
        } else if cancelled || resp.should_close() {
            // descartado
        } else {
            keep = Some(c);
        }
        self.dlg.confirm = keep;
    }

    pub(super) fn run_confirm(&mut self, c: Confirm) {
        match c {
            Confirm::EmptyTrash => self.empty_trash(),
            Confirm::Purge(ids) => self.purge_ids(ids),
            Confirm::ForceTrash(list) => self.force_trash(list.into_iter().map(|(id, _)| id).collect()),
            Confirm::RemoveRoot { id, delete_files, .. } => {
                let section_was = self.filters.section.clone();
                self.command("root", Refresh::ALL, move |lib| lib.remove_root(id, delete_files).map(|n| format!("{n} {}", tr("lib.msg.assets_unindexed"))).map_err(|e| e.to_string()));
                if section_was == Section::Root(id) {
                    self.set_section(Section::All);
                }
            }
            Confirm::DeleteCollection(id, _) => {
                if self.filters.section == Section::Collection(id) {
                    self.set_section(Section::All);
                }
                self.command("collection", Refresh::ROWS_FACETS, move |lib| lib.delete_collection(id).map(|_| String::new()).map_err(|e| e.to_string()));
            }
            Confirm::ClearThumbs => self.clear_thumbnail_cache(),
            Confirm::Rebuild => self.start_rebuild(),
        }
    }

    pub(crate) fn clear_thumbnail_cache(&mut self) {
        let Some(rt) = &self.rt else { return };
        rt.thumbs.cache.clear();
        rt.thumbs.reset_failures();
        self.tex.clear();
        self.info(tr("lib.msg.thumbs_cleared"));
    }

    // =============================================================== navegador de pastas

    fn picker_modal(&mut self, ctx: &egui::Context) {
        let Some(mut p) = self.dlg.picker.take() else { return };
        let mut result: Option<Vec<PathBuf>> = None;
        let mut close = false;
        let title = match p.purpose {
            PickPurpose::ImportPaths | PickPurpose::RelinkFile => tr("lib.pick.files"),
            _ => tr("lib.pick.folder"),
        };
        let resp = egui::Modal::new(egui::Id::new("lib_picker")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading(title);
            ui.horizontal(|ui| {
                if icon_button(ui, icons::ARROW_UP, tr("lib.pick.up")).clicked() {
                    if let Some(parent) = p.dir.parent() {
                        let parent = parent.to_path_buf();
                        p.go(parent);
                    }
                }
                if icon_button(ui, icons::HOUSE, tr("lib.pick.home")).clicked() {
                    if let Some(h) = home_dir() {
                        p.go(h);
                    }
                }
                if icon_button(ui, icons::HARD_DRIVES, tr("lib.pick.root")).clicked() {
                    p.go(PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" }));
                }
                let r = ui.add(egui::TextEdit::singleline(&mut p.path_edit).desired_width(ui.available_width() - 110.0));
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let np = PathBuf::from(p.path_edit.trim());
                    if np.is_dir() {
                        p.go(np);
                    } else {
                        p.error = Some(format!("{}: {}", np.display(), tr("lib.pick.not_a_folder")));
                    }
                }
                if ui.checkbox(&mut p.show_hidden, RichText::new(tr("lib.pick.hidden")).size(11.5)).changed() {
                    p.reload();
                }
            });
            if let Some(e) = &p.error {
                ui.label(RichText::new(e).color(err_color()).size(12.0));
            }
            ui.separator();
            let h = 300.0;
            let mut navigate: Option<PathBuf> = None;
            egui::ScrollArea::vertical().id_salt("lib_picker_list").max_height(h).min_scrolled_height(h).auto_shrink([false, false]).show_rows(ui, 22.0, p.entries.len(), |ui, range| {
                for i in range {
                    let e = &p.entries[i];
                    let sel = p.selected.contains(&e.path);
                    let label = format!("{}  {}", if e.is_dir { icons::FOLDER } else { icons::FILE }, e.name);
                    let r = ui.selectable_label(sel, label);
                    if r.double_clicked() && e.is_dir {
                        navigate = Some(e.path.clone());
                    } else if r.clicked() {
                        let multi = p.files_mode;
                        if sel {
                            p.selected.retain(|x| *x != e.path);
                        } else {
                            if !multi || !ui.input(|i| i.modifiers.command || i.modifiers.shift) && !e.is_dir {
                                // arquivos: clique alterna (multi); pastas: seleção única
                            }
                            if !multi || e.is_dir {
                                p.selected.clear();
                            }
                            p.selected.push(e.path.clone());
                        }
                    }
                }
            });
            if let Some(d) = navigate {
                p.go(d);
            }
            if p.truncated {
                ui.label(small_dim(tr("lib.pick.truncated")));
            }
            ui.separator();
            ui.horizontal(|ui| {
                let label = if p.files_mode {
                    if p.selected.is_empty() { tr("lib.pick.use_folder").to_string() } else { format!("{} ({})", tr("lib.pick.use_files"), p.selected.len()) }
                } else if let Some(s) = p.selected.first() {
                    format!("{}: {}", tr("lib.pick.use"), s.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
                } else {
                    tr("lib.pick.use_current").to_string()
                };
                if ui.button(label).clicked() {
                    result = Some(if p.selected.is_empty() { vec![p.dir.clone()] } else { p.selected.clone() });
                }
                if ui.button(tr("lib.cancel")).clicked() {
                    close = true;
                }
            });
        });
        if resp.should_close() {
            close = true;
        }
        if let Some(paths) = result {
            self.finish_pick(p.purpose.clone(), paths);
        } else if !close {
            self.dlg.picker = Some(p);
        }
    }

    fn finish_pick(&mut self, purpose: PickPurpose, paths: Vec<PathBuf>) {
        let first = paths.first().cloned().unwrap_or_default();
        match purpose {
            PickPurpose::AddRootLinked => self.add_root(first, true),
            PickPurpose::AddRootManaged => self.add_root(first, false),
            PickPurpose::ImportPaths => {
                for p in paths {
                    if !self.dlg.import_paths.contains(&p) {
                        self.dlg.import_paths.push(p);
                    }
                }
                self.dlg.import_open = true;
            }
            PickPurpose::RelinkFile => {
                if let Some(s) = &mut self.dlg.relink {
                    s.file = first.display().to_string();
                }
            }
            PickPurpose::RelinkSearchDir => {
                if let Some(s) = &mut self.dlg.relink {
                    s.search_dir = first.display().to_string();
                }
            }
            PickPurpose::RelinkRoot => {
                if let Some(s) = &mut self.dlg.relink_root {
                    s.new_path = first.display().to_string();
                }
            }
            PickPurpose::RelinkBatch => self.dlg.relink_batch = Some(first.display().to_string()),
        }
    }

    // ================================================================== importação

    fn import_window(&mut self, ctx: &egui::Context) {
        if !self.dlg.import_open {
            return;
        }
        let mut open = true;
        let mut start = false;
        let mut analyze: Option<PathBuf> = None;
        let mut pick = false;
        egui::Window::new(format!("{}  {}", icons::DOWNLOAD_SIMPLE, tr("lib.imp.title"))).id(egui::Id::new("lib_import_win")).open(&mut open).collapsible(false).resizable(true).default_width(520.0).default_pos(win_pos(ctx, 520.0, 400.0)).show(ctx, |ui| {
            ui.label(small_dim(tr("lib.imp.hint")));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let w = 300.0;
                let r = ui.add(egui::TextEdit::singleline(&mut self.dlg.import_path).hint_text(tr("lib.imp.path_hint")).desired_width(w));
                let add = (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) || ui.button(format!("{}  {}", icons::PLUS, tr("lib.imp.add_path"))).clicked();
                if add && !self.dlg.import_path.trim().is_empty() {
                    let p = PathBuf::from(self.dlg.import_path.trim());
                    if p.exists() {
                        if !self.dlg.import_paths.contains(&p) {
                            self.dlg.import_paths.push(p);
                        }
                        self.dlg.import_path.clear();
                    } else {
                        self.toast = Some((format!("{}: {}", tr("lib.imp.not_found"), p.display()), ToastKind::Error, Instant::now()));
                    }
                }
                if ui.button(format!("{}  {}", icons::FOLDER_OPEN, tr("lib.imp.browse"))).clicked() {
                    pick = true;
                }
            });
            ui.add_space(4.0);
            if self.dlg.import_paths.is_empty() {
                ui.label(small_dim(tr("lib.imp.empty_list")));
            } else {
                let mut remove: Option<usize> = None;
                egui::ScrollArea::vertical().id_salt("lib_imp_paths").max_height(110.0).auto_shrink([false, true]).show(ui, |ui| {
                    for (i, p) in self.dlg.import_paths.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(if p.is_dir() { icons::FOLDER } else { icons::FILE }).color(theme::TEXT_DIM));
                            ui.add(egui::Label::new(RichText::new(p.display().to_string()).size(12.0)).truncate());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if icon_button(ui, icons::X, tr("lib.imp.remove")).clicked() {
                                    remove = Some(i);
                                }
                                if p.is_file() && ui.small_button(tr("lib.imp.analyze")).clicked() {
                                    analyze = Some(p.clone());
                                }
                            });
                        });
                    }
                });
                if let Some(i) = remove {
                    self.dlg.import_paths.remove(i);
                }
            }
            if let Some((path, res)) = &self.dlg.import_preview {
                ui.add_space(4.0);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.label(small_dim(path.display().to_string()));
                    match res {
                        Ok(p) => {
                            ui.label(format!("{}  {} \u{00b7} {}", kind_icon(p.kind), kind_label(p.kind), p.format.to_uppercase()));
                            if p.stats.tris > 0 {
                                ui.label(small_dim(format!("{} tris \u{00b7} {} {} \u{00b7} {} {} \u{00b7} {} {}", fmt_int(p.stats.tris), fmt_int(p.stats.verts), tr("lib.verts"), p.stats.materials, tr("lib.det.materials"), p.stats.textures, tr("lib.det.textures"))));
                            }
                            if p.stats.width > 0 {
                                ui.label(small_dim(format!("{} \u{00d7} {} px", p.stats.width, p.stats.height)));
                            }
                            if let Some(n) = &p.suggested_name {
                                ui.label(small_dim(format!("{}: {n}", tr("lib.imp.suggested"))));
                            }
                            for w in &p.warnings {
                                ui.label(RichText::new(format!("\u{26a0} {w}")).size(11.5).color(warn_color()));
                            }
                        }
                        Err(e) => {
                            ui.label(RichText::new(e).color(err_color()).size(12.0));
                        }
                    }
                });
            }
            ui.add_space(6.0);
            egui::CollapsingHeader::new(RichText::new(tr("lib.imp.options")).strong()).default_open(true).show(ui, |ui| {
                let f = &mut self.dlg.import_form;
                ui.checkbox(&mut f.recursive, tr("lib.imp.recursive"));
                ui.checkbox(&mut f.allow_dups, tr("lib.imp.allow_dups")).on_hover_text(tr("lib.imp.allow_dups.tip"));
                ui.horizontal(|ui| {
                    ui.label(small_dim(tr("lib.imp.kind")));
                    let cur = f.kind.map(|k| kind_label(k).to_string()).unwrap_or_else(|| tr("lib.imp.kind_auto").to_string());
                    egui::ComboBox::from_id_salt("lib_imp_kind").selected_text(cur).show_ui(ui, |ui| {
                        if ui.selectable_label(f.kind.is_none(), tr("lib.imp.kind_auto")).clicked() {
                            f.kind = None;
                        }
                        for k in AssetKind::ALL {
                            if ui.selectable_label(f.kind == Some(*k), format!("{} {}", kind_icon(*k), kind_label(*k))).clicked() {
                                f.kind = Some(*k);
                            }
                        }
                    });
                });
                egui::Grid::new("lib_imp_form").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                    ui.label(small_dim(tr("lib.det.tags")));
                    ui.add(egui::TextEdit::singleline(&mut f.tags).hint_text(tr("lib.imp.tags_hint")).desired_width(f32::INFINITY));
                    ui.end_row();
                    ui.label(small_dim(tr("lib.det.author")));
                    ui.add(egui::TextEdit::singleline(&mut f.author).desired_width(f32::INFINITY));
                    ui.end_row();
                    ui.label(small_dim(tr("lib.det.license")));
                    ui.add(egui::TextEdit::singleline(&mut f.license).desired_width(f32::INFINITY));
                    ui.end_row();
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let can = !self.dlg.import_paths.is_empty() || !self.dlg.import_path.trim().is_empty();
                if ui.add_enabled(can, egui::Button::new(RichText::new(format!("{}  {}", icons::DOWNLOAD_SIMPLE, tr("lib.imp.start"))).strong())).clicked() {
                    start = true;
                }
                if !self.jobs.log.is_empty() && ui.button(format!("{}  {}", icons::LIST_BULLETS, tr("lib.imp.report"))).clicked() {
                    self.jobs.log.open = true;
                }
            });
            if let Some(run) = &self.jobs.import {
                ui.add_space(4.0);
                let frac = if run.total > 0 { run.done as f32 / run.total as f32 } else { 0.0 };
                ui.add(egui::ProgressBar::new(frac).text(format!("{} \u{00b7} {}/{}", run.label, run.done, run.total.max(run.discovered))));
            }
        });
        if pick {
            self.dlg.open_picker(PickPurpose::ImportPaths, None);
        }
        if let Some(p) = analyze {
            let limits = self.rt.as_ref().map(|r| r.lib.limits().clone());
            if let Some(limits) = limits {
                self.spawn_task(move |_| TaskOut::Preview { path: p.clone(), result: preview(&p, &limits).map_err(|e| e.to_string()) });
            }
        }
        if start {
            if !self.dlg.import_path.trim().is_empty() {
                let p = PathBuf::from(self.dlg.import_path.trim());
                if p.exists() && !self.dlg.import_paths.contains(&p) {
                    self.dlg.import_paths.push(p);
                }
                self.dlg.import_path.clear();
            }
            let f = &self.dlg.import_form;
            let tags: Vec<String> = f.tags.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            let opts = ImportOptions {
                recursive: f.recursive,
                duplicates: if f.allow_dups { DuplicatePolicy::Allow } else { DuplicatePolicy::Skip },
                kind: f.kind,
                tags,
                author: (!f.author.trim().is_empty()).then(|| f.author.trim().to_string()),
                license: (!f.license.trim().is_empty()).then(|| f.license.trim().to_string()),
                ..Default::default()
            };
            let paths = std::mem::take(&mut self.dlg.import_paths);
            if !paths.is_empty() {
                self.jobs.log.clear();
                self.queue_import(paths, opts);
            }
            self.dlg.import_preview = None;
        }
        if !open {
            self.dlg.import_open = false;
        }
    }

    /// Arquivos soltos do sistema operacional: importação direta com as opções padrão.
    pub(super) fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if dropped.is_empty() {
            return;
        }
        if self.rt.is_none() {
            self.error(tr("lib.opening"));
            return;
        }
        self.info(format!("{} {}", dropped.len(), tr("lib.imp.dropped")));
        self.jobs.log.clear();
        let opts = ImportOptions { recursive: true, ..Default::default() };
        self.queue_import(dropped, opts);
    }

    pub(super) fn drop_overlay(&mut self, ctx: &egui::Context) {
        let hovered = ctx.input(|i| i.raw.hovered_files.len());
        if hovered == 0 {
            return;
        }
        let rect = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("lib_drop_overlay")));
        painter.rect_filled(rect, 0.0, Color32::from_black_alpha(160));
        painter.rect_stroke(rect.shrink(14.0), 10.0, egui::Stroke::new(2.0, theme::ACCENT), egui::StrokeKind::Inside);
        painter.text(rect.center() - Vec2::new(0.0, 14.0), egui::Align2::CENTER_CENTER, icons::DOWNLOAD_SIMPLE, egui::FontId::proportional(48.0), theme::ACCENT);
        painter.text(rect.center() + Vec2::new(0.0, 30.0), egui::Align2::CENTER_CENTER, format!("{} ({hovered})", tr("lib.imp.drop_here")), egui::FontId::proportional(18.0), Color32::WHITE);
    }

    /// Fantasma do que está sendo arrastado (card -> coleção/tag/favoritos/lixeira).
    pub(super) fn drag_ghost(&mut self, ctx: &egui::Context) {
        let Some(p) = egui::DragAndDrop::payload::<AssetDragPayload>(ctx) else { return };
        let Some(pos) = ctx.pointer_latest_pos() else { return };
        egui::Area::new(egui::Id::new("lib_drag_ghost")).order(egui::Order::Tooltip).fixed_pos(pos + Vec2::new(14.0, 14.0)).interactable(false).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(RichText::new(format!("{}  {}", kind_icon(p.kind), p.label)).size(12.5));
            });
        });
    }

    // ======================================================= relatório de importação

    fn report_window(&mut self, ctx: &egui::Context) {
        if !self.jobs.log.open {
            return;
        }
        let mut open = true;
        let mut cancel = false;
        let mut clear = false;
        let mut copy = false;
        egui::Window::new(format!("{}  {}", icons::LIST_BULLETS, tr("lib.imp.report"))).id(egui::Id::new("lib_report_win")).open(&mut open).collapsible(false).resizable(true).default_size([560.0, 380.0]).default_pos(win_pos(ctx, 560.0, 380.0)).show(ctx, |ui| {
            let log = &self.jobs.log;
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("{} {}", log.imported, tr("lib.imp.imported"))).color(ok_color()));
                ui.label(RichText::new(format!("{} {}", log.revised, tr("lib.imp.revised"))).color(theme::TEXT_DIM));
                ui.label(RichText::new(format!("{} {}", log.duplicates, tr("lib.imp.duplicates"))).color(warn_color()));
                ui.label(RichText::new(format!("{} {}", log.skipped, tr("lib.imp.skipped"))).color(theme::TEXT_DIM));
                ui.label(RichText::new(format!("{} {}", log.failed, tr("lib.imp.failed"))).color(if log.failed > 0 { err_color() } else { theme::TEXT_DIM }));
                if log.cancelled {
                    ui.label(RichText::new(tr("lib.imp.cancelled")).color(warn_color()));
                }
            });
            if let Some(run) = &self.jobs.import {
                let frac = if run.total > 0 { run.done as f32 / run.total as f32 } else { 0.0 };
                ui.horizontal(|ui| {
                    ui.add(egui::ProgressBar::new(frac).desired_width(ui.available_width() - 90.0).text(format!("{} \u{00b7} {}/{} \u{00b7} {}", run.label, run.done, run.total.max(run.discovered), fmt_bytes(run.bytes))));
                    if ui.button(tr("lib.cancel")).clicked() {
                        cancel = true;
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.dlg.log_errors_only, tr("lib.imp.only_errors"));
                if ui.small_button(format!("{}  {}", icons::COPY, tr("lib.imp.copy_errors"))).clicked() {
                    copy = true;
                }
                if ui.small_button(tr("lib.imp.clear")).clicked() {
                    clear = true;
                }
            });
            ui.separator();
            let only_err = self.dlg.log_errors_only;
            let rows: Vec<&jobs::ImportEntry> = self.jobs.log.entries.iter().filter(|e| !only_err || e.kind == EntryKind::Failed).collect();
            if rows.is_empty() {
                ui.label(small_dim(tr("lib.imp.no_entries")));
            }
            egui::ScrollArea::vertical().id_salt("lib_report_rows").auto_shrink([false, false]).show_rows(ui, 20.0, rows.len(), |ui, range| {
                for e in &rows[range] {
                    let (icon, col) = match e.kind {
                        EntryKind::Imported => (icons::CHECK_CIRCLE, ok_color()),
                        EntryKind::Revised => (icons::ARROWS_CLOCKWISE, ok_color()),
                        EntryKind::Duplicate => (icons::COPY, warn_color()),
                        EntryKind::Skipped => (icons::MINUS_CIRCLE, theme::TEXT_DIM),
                        EntryKind::Failed => (icons::X_CIRCLE, err_color()),
                        EntryKind::Relinked => (icons::LINK, ok_color()),
                    };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icon).color(col));
                        let name = e.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| e.path.display().to_string());
                        ui.label(RichText::new(name).size(12.0)).on_hover_text(e.path.display().to_string());
                        ui.add(egui::Label::new(RichText::new(&e.detail).size(11.5).color(if e.kind == EntryKind::Failed { err_color() } else { theme::TEXT_DIM })).truncate());
                    });
                }
            });
            if self.jobs.log.truncated {
                ui.label(small_dim(tr("lib.imp.truncated")));
            }
        });
        if cancel {
            self.cancel_imports();
        }
        if clear {
            self.jobs.log.clear();
            self.jobs.log.open = true;
        }
        if copy {
            let text: Vec<String> = self.jobs.log.entries.iter().filter(|e| e.kind == EntryKind::Failed).map(|e| format!("{}\t{}", e.path.display(), e.detail)).collect();
            ctx.copy_text(text.join("\n"));
        }
        if !open {
            self.jobs.log.open = false;
        }
    }

    // ======================================================================== gestão

    fn manage_window(&mut self, ctx: &egui::Context) {
        if !self.dlg.manage_open {
            return;
        }
        let mut open = true;
        let mut calc_disk = false;
        let mut open_dups = false;
        let mut rescan = false;
        let mut verify: Option<VerifyOptions> = None;
        let mut clear_thumbs = false;
        let mut purge_stale = false;
        let mut rebuild = false;
        let mut empty_trash = false;
        let mut relink_batch = false;
        let mut select: Option<AssetId> = None;
        egui::Window::new(format!("{}  {}", icons::GEAR, tr("lib.manage"))).id(egui::Id::new("lib_manage_win")).open(&mut open).collapsible(false).resizable(true).default_size([520.0, 560.0]).default_pos(win_pos(ctx, 520.0, 400.0)).show(ctx, |ui| {
            egui::ScrollArea::vertical().id_salt("lib_manage_scroll").auto_shrink([false, false]).show(ui, |ui| {
                // ---- disco
                egui::CollapsingHeader::new(RichText::new(format!("{}  {}", icons::DATABASE, tr("lib.manage.disk"))).strong()).default_open(true).show(ui, |ui| {
                    if let Some(rt) = &self.rt {
                        ui.label(small_dim(format!("{}: {}", tr("lib.manage.home"), rt.lib.home().display())));
                        ui.label(small_dim(format!("{}: {}", tr("lib.manage.index"), rt.lib.index_path().display())));
                    }
                    if let Some(u) = &self.dlg.disk {
                        egui::Grid::new("lib_disk_grid").num_columns(2).spacing([14.0, 3.0]).show(ui, |ui| {
                            kv_row(ui, tr("lib.manage.disk.home"), fmt_bytes(u.home_bytes));
                            kv_row(ui, tr("lib.manage.disk.assets"), fmt_bytes(u.assets_bytes));
                            kv_row(ui, tr("lib.manage.disk.trash"), fmt_bytes(u.trash_bytes));
                            kv_row(ui, tr("lib.manage.disk.index"), fmt_bytes(u.index_bytes));
                            kv_row(ui, tr("lib.manage.disk.thumbs"), fmt_bytes(u.thumbs_bytes));
                        });
                        ui.label(small_dim(format!("{} ms", u.elapsed_ms)));
                    }
                    if let Some(rt) = &self.rt {
                        ui.label(small_dim(format!("{}: {} ({})  \u{00b7}  {}: {} ({})", tr("lib.manage.ram_thumbs"), rt.thumbs.cache.ram_len(), fmt_bytes(rt.thumbs.cache.ram_bytes() as u64), tr("lib.thumbs.textures"), self.tex.len(), fmt_bytes(self.tex.bytes() as u64))));
                    }
                    if ui.add_enabled(!self.dlg.disk_busy, egui::Button::new(format!("{}  {}", icons::CHART_BAR, tr("lib.manage.disk.calc")))).clicked() {
                        calc_disk = true;
                    }
                });
                // ---- cache
                egui::CollapsingHeader::new(RichText::new(format!("{}  {}", icons::IMAGES, tr("lib.manage.cache"))).strong()).default_open(true).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(format!("{}  {}", icons::BROOM, tr("lib.manage.clear_thumbs"))).on_hover_text(tr("lib.manage.clear_thumbs.tip")).clicked() {
                            clear_thumbs = true;
                        }
                        if ui.button(tr("lib.manage.purge_stale")).on_hover_text(tr("lib.manage.purge_stale.tip")).clicked() {
                            purge_stale = true;
                        }
                    });
                });
                // ---- integridade
                egui::CollapsingHeader::new(RichText::new(format!("{}  {}", icons::SHIELD_CHECK, tr("lib.manage.integrity"))).strong()).default_open(true).show(ui, |ui| {
                    ui.checkbox(&mut self.dlg.verify_rehash, tr("lib.manage.rehash")).on_hover_text(tr("lib.manage.rehash.tip"));
                    ui.checkbox(&mut self.dlg.verify_fix, tr("lib.manage.fix")).on_hover_text(tr("lib.manage.fix.tip"));
                    ui.horizontal(|ui| {
                        if ui.add_enabled(self.jobs.verify.is_none(), egui::Button::new(format!("{}  {}", icons::SHIELD_CHECK, tr("lib.manage.verify")))).clicked() {
                            verify = Some(VerifyOptions { rehash: self.dlg.verify_rehash, fix: self.dlg.verify_fix });
                        }
                        if ui.add_enabled(self.jobs.rebuild.is_none(), egui::Button::new(format!("{}  {}", icons::WRENCH, tr("lib.manage.rebuild")))).on_hover_text(tr("lib.manage.rebuild.tip")).clicked() {
                            rebuild = true;
                        }
                    });
                    if let Some(msg) = &self.jobs.last_rebuild {
                        ui.label(small_dim(msg));
                    }
                    let run = self.jobs.verify.as_ref().or(self.jobs.last_verify.as_ref());
                    if let Some(run) = run {
                        if let Some(rep) = &run.report {
                            ui.label(RichText::new(format!("{} {} \u{00b7} {} ok \u{00b7} {} {} \u{00b7} {} {} \u{00b7} {} ms", rep.checked, tr("lib.verify.checked"), rep.ok, rep.issues.len(), tr("lib.verify.issues"), rep.fixed, tr("lib.verify.fixed"), rep.elapsed_ms)).size(12.0).color(if rep.issues.is_empty() { ok_color() } else { warn_color() }));
                        }
                        let issues = if let Some(rep) = &run.report { &rep.issues } else { &run.issues };
                        if !issues.is_empty() {
                            egui::ScrollArea::vertical().id_salt("lib_verify_issues").max_height(180.0).auto_shrink([false, true]).show_rows(ui, 20.0, issues.len().min(500), |ui, range| {
                                for i in &issues[range] {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(icons::WARNING).color(warn_color()));
                                        ui.label(RichText::new(format!("{:?}", i.kind)).size(11.5).strong());
                                        let r = ui.add(egui::Label::new(RichText::new(&i.detail).size(11.5).color(theme::TEXT_DIM)).truncate().sense(egui::Sense::click()));
                                        if r.clicked() {
                                            select = i.asset;
                                        }
                                    });
                                }
                            });
                        }
                    }
                });
                // ---- lixeira, duplicados, ausentes
                egui::CollapsingHeader::new(RichText::new(format!("{}  {}", icons::TRASH, tr("lib.manage.cleanup"))).strong()).default_open(true).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(self.facets.n_trash > 0, egui::Button::new(format!("{}  {} ({})", icons::RECYCLE, tr("lib.trash.empty"), self.facets.n_trash))).clicked() {
                            empty_trash = true;
                        }
                        if ui.button(format!("{}  {} ({})", icons::COPY, tr("lib.duplicates"), self.facets.dup_groups)).clicked() {
                            open_dups = true;
                        }
                        if ui.add_enabled(self.facets.n_missing > 0, egui::Button::new(format!("{}  {} ({})", icons::LINK, tr("lib.relink.batch"), self.facets.n_missing))).clicked() {
                            relink_batch = true;
                        }
                        if ui.button(format!("{}  {}", icons::ARROWS_CLOCKWISE, tr("lib.root.rescan_all"))).clicked() {
                            rescan = true;
                        }
                    });
                });
            });
        });
        if calc_disk {
            self.dlg.disk_busy = true;
            self.spawn_task(|lib| TaskOut::DiskUsage(measure_disk(&lib)));
        }
        if open_dups {
            self.open_duplicates();
        }
        if rescan {
            self.queue_scan_all_linked();
            self.info(tr("lib.scan.started"));
        }
        if let Some(v) = verify {
            self.jobs.last_verify = None;
            self.start_verify(v);
        }
        if clear_thumbs {
            self.dlg.confirm = Some(Confirm::ClearThumbs);
        }
        if purge_stale {
            if let Some(rt) = &self.rt {
                let freed = rt.thumbs.cache.purge_stale(None);
                rt.thumbs.reset_failures();
                self.tex.clear();
                self.info(format!("{} {}", fmt_bytes(freed), tr("lib.msg.freed")));
            }
        }
        if rebuild {
            self.dlg.confirm = Some(Confirm::Rebuild);
        }
        if empty_trash {
            self.dlg.confirm = Some(Confirm::EmptyTrash);
        }
        if relink_batch {
            self.dlg.open_relink_batch();
        }
        if let Some(id) = select {
            self.selection.set_one(id, 0);
            self.selection.cursor = None;
            self.sync_detail_target();
        }
        if !open {
            self.dlg.manage_open = false;
        }
    }

    // ===================================================================== duplicados

    fn duplicates_window(&mut self, ctx: &egui::Context) {
        if !self.dlg.dups_open {
            return;
        }
        let mut open = true;
        let mut resolve: Option<(AssetId, Vec<AssetId>)> = None;
        let mut resolve_all = false;
        egui::Window::new(format!("{}  {}", icons::COPY, tr("lib.duplicates"))).id(egui::Id::new("lib_dups_win")).open(&mut open).collapsible(false).resizable(true).default_size([620.0, 460.0]).default_pos(win_pos(ctx, 520.0, 400.0)).show(ctx, |ui| {
            match &self.dlg.dups {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(dim(tr("lib.dup.searching")));
                    });
                }
                Some(Err(e)) => {
                    ui.label(RichText::new(e).color(err_color()));
                }
                Some(Ok(groups)) if groups.is_empty() => {
                    ui.vertical_centered(|ui| {
                        ui.add_space(30.0);
                        ui.label(RichText::new(icons::CHECK_CIRCLE).size(36.0).color(ok_color()));
                        ui.label(dim(tr("lib.dup.none")));
                    });
                }
                Some(Ok(groups)) => {
                    let total: u64 = groups.iter().map(|g| g.reclaimable_bytes()).sum();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{} {} \u{00b7} {} {}", groups.len(), tr("lib.dup.groups"), fmt_bytes(total), tr("lib.dup.reclaimable"))).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(format!("{}  {}", icons::BROOM, tr("lib.dup.resolve_all"))).on_hover_text(tr("lib.dup.resolve_all.tip")).clicked() {
                                resolve_all = true;
                            }
                        });
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().id_salt("lib_dups_scroll").auto_shrink([false, false]).show_rows(ui, 24.0, groups.len(), |ui, range| {
                        for g in &groups[range] {
                            let first = g.assets.first().map(|a| a.name.clone()).unwrap_or_default();
                            let title = format!("{first}  \u{00b7}  {} {}  \u{00b7}  {}", g.assets.len(), tr("lib.dup.copies"), fmt_bytes(g.size));
                            egui::CollapsingHeader::new(RichText::new(title).size(12.5)).id_salt(g.content_hash.to_hex()).show(ui, |ui| {
                                let keep = self.dlg.dup_keep.get(&g.content_hash).copied();
                                for a in &g.assets {
                                    ui.horizontal(|ui| {
                                        let mut sel = keep == Some(a.id);
                                        if ui.radio(sel, "").clicked() {
                                            sel = true;
                                        }
                                        if sel && keep != Some(a.id) {
                                            self.dlg.dup_keep.insert(g.content_hash, a.id);
                                        }
                                        ui.label(RichText::new(&a.name).size(12.0));
                                        ui.label(small_dim(format!("{} \u{00b7} {}", if a.storage == StorageMode::Managed { tr("lib.root.managed") } else { tr("lib.root.linked") }, fmt_time(a.imported_at))));
                                    });
                                }
                                if ui.small_button(tr("lib.dup.resolve")).clicked() {
                                    if let Some(k) = self.dlg.dup_keep.get(&g.content_hash).copied() {
                                        resolve = Some((k, g.assets.iter().map(|a| a.id).filter(|i| *i != k).collect()));
                                    }
                                }
                            });
                        }
                    });
                }
            }
        });
        if let Some((keep, remove)) = resolve {
            let n = remove.len();
            self.command("dups", Refresh::ALL, move |lib| lib.resolve_duplicates(&keep, &remove).map(|_| format!("{n} {}", tr("lib.msg.dups_resolved"))).map_err(|e| e.to_string()));
            self.dlg.dups = None;
            self.spawn_task(|lib| TaskOut::Duplicates(lib.find_duplicates(5000).map_err(|e| e.to_string())));
        }
        if resolve_all {
            let plan: Vec<(AssetId, Vec<AssetId>)> = match &self.dlg.dups {
                Some(Ok(groups)) => groups
                    .iter()
                    .map(|g| {
                        let k = self.dlg.dup_keep.get(&g.content_hash).copied().unwrap_or_else(|| default_keeper(g));
                        (k, g.assets.iter().map(|a| a.id).filter(|i| *i != k).collect::<Vec<_>>())
                    })
                    .collect(),
                _ => Vec::new(),
            };
            let n: usize = plan.iter().map(|(_, r)| r.len()).sum();
            self.command("dups", Refresh::ALL, move |lib| {
                for (k, r) in &plan {
                    lib.resolve_duplicates(k, r).map_err(|e| e.to_string())?;
                }
                Ok(format!("{n} {}", tr("lib.msg.dups_resolved")))
            });
            self.dlg.dups = None;
            self.dlg.dups_open = false;
        }
        if !open {
            self.dlg.dups_open = false;
        }
    }

    // =========================================================================== relink

    fn relink_windows(&mut self, ctx: &egui::Context) {
        // ---- ativo ausente
        if let Some(mut s) = self.dlg.relink.take() {
            let mut open = true;
            let mut do_link: Option<PathBuf> = None;
            let mut pick_file = false;
            let mut pick_dir = false;
            let mut search = false;
            egui::Window::new(format!("{}  {}", icons::LINK, tr("lib.relink"))).id(egui::Id::new("lib_relink_win")).open(&mut open).collapsible(false).resizable(true).default_width(520.0).default_pos(win_pos(ctx, 520.0, 400.0)).show(ctx, |ui| {
                ui.label(RichText::new(&s.name).strong());
                ui.label(small_dim(tr("lib.relink.hint")));
                ui.add_space(6.0);
                ui.label(small_dim(tr("lib.relink.new_file")));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.file).desired_width(330.0));
                    if ui.button(tr("lib.imp.browse")).clicked() {
                        pick_file = true;
                    }
                    if ui.add_enabled(!s.file.trim().is_empty(), egui::Button::new(tr("lib.relink"))).clicked() {
                        do_link = Some(PathBuf::from(s.file.trim()));
                    }
                });
                ui.add_space(6.0);
                ui.label(small_dim(tr("lib.relink.search_folder")));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.search_dir).desired_width(330.0));
                    if ui.button(tr("lib.pick.folder")).clicked() {
                        pick_dir = true;
                    }
                    if ui.add_enabled(!s.search_dir.trim().is_empty() && !s.searching, egui::Button::new(tr("lib.relink.search"))).clicked() {
                        search = true;
                    }
                });
                if s.searching {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(dim(tr("lib.relink.searching")));
                    });
                }
                match &s.candidates {
                    Some(Ok(c)) if c.is_empty() => {
                        ui.label(RichText::new(tr("lib.relink.none")).color(warn_color()));
                    }
                    Some(Ok(c)) => {
                        for p in c {
                            ui.horizontal(|ui| {
                                if ui.small_button(tr("lib.relink.use")).clicked() {
                                    do_link = Some(p.clone());
                                }
                                ui.add(egui::Label::new(RichText::new(p.display().to_string()).size(12.0)).truncate());
                            });
                        }
                    }
                    Some(Err(e)) => {
                        ui.label(RichText::new(e).color(err_color()));
                    }
                    None => {}
                }
            });
            let id = s.id;
            if pick_file {
                self.dlg.relink = Some(s);
                self.dlg.open_picker(PickPurpose::RelinkFile, None);
            } else if pick_dir {
                self.dlg.relink = Some(s);
                self.dlg.open_picker(PickPurpose::RelinkSearchDir, None);
            } else if let Some(file) = do_link {
                self.command("relink", Refresh::ALL, move |lib| lib.relink(&id, &file).map(|_| tr("lib.relink.done").to_string()).map_err(|e| e.to_string()));
            } else if search {
                let dir = PathBuf::from(s.search_dir.trim());
                s.searching = true;
                s.candidates = None;
                self.spawn_task(move |lib| {
                    let r = lib.relink_candidates(&id, &dir, 20, &CancelToken::new()).map_err(|e| e.to_string());
                    TaskOut::RelinkCandidates { id, result: r }
                });
                self.dlg.relink = Some(s);
            } else if open {
                self.dlg.relink = Some(s);
            }
        }
        // ---- raiz inteira (disco remontado)
        if let Some(mut s) = self.dlg.relink_root.take() {
            let mut open = true;
            let mut go = false;
            let mut pick = false;
            egui::Window::new(format!("{}  {}", icons::LINK, tr("lib.root.relink"))).id(egui::Id::new("lib_relink_root_win")).open(&mut open).collapsible(false).default_width(480.0).default_pos(win_pos(ctx, 520.0, 400.0)).show(ctx, |ui| {
                ui.label(RichText::new(&s.root.name).strong());
                ui.label(small_dim(format!("{}: {}", tr("lib.root.current_path"), s.root.path.display())));
                ui.label(small_dim(tr("lib.root.relink.hint")));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.new_path).desired_width(330.0));
                    if ui.button(tr("lib.pick.folder")).clicked() {
                        pick = true;
                    }
                    if ui.add_enabled(!s.new_path.trim().is_empty(), egui::Button::new(tr("lib.relink"))).clicked() {
                        go = true;
                    }
                });
            });
            let rid = s.root.id;
            if pick {
                self.dlg.relink_root = Some(s);
                self.dlg.open_picker(PickPurpose::RelinkRoot, None);
            } else if go {
                let path = PathBuf::from(s.new_path.trim());
                self.command("relink", Refresh::ALL, move |lib| lib.relink_root(rid, &path).map(|(found, missing)| format!("{} {} \u{00b7} {} {}", found.len(), tr("lib.msg.found"), missing.len(), tr("lib.scan.missing"))).map_err(|e| e.to_string()));
            } else if open {
                self.dlg.relink_root = Some(s);
            }
        }
        // ---- em lote
        if let Some(mut dir) = self.dlg.relink_batch.take() {
            let mut open = true;
            let mut go = false;
            let mut pick = false;
            egui::Window::new(format!("{}  {}", icons::LINK, tr("lib.relink.batch"))).id(egui::Id::new("lib_relink_batch_win")).open(&mut open).collapsible(false).default_width(480.0).default_pos(win_pos(ctx, 520.0, 400.0)).show(ctx, |ui| {
                ui.label(small_dim(format!("{} {}", self.facets.n_missing, tr("lib.relink.batch.hint"))));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut dir).hint_text(tr("lib.relink.search_folder")).desired_width(330.0));
                    if ui.button(tr("lib.pick.folder")).clicked() {
                        pick = true;
                    }
                    if ui.add_enabled(!dir.trim().is_empty(), egui::Button::new(tr("lib.relink.batch.go"))).clicked() {
                        go = true;
                    }
                });
            });
            if pick {
                self.dlg.relink_batch = Some(dir);
                self.dlg.open_picker(PickPurpose::RelinkBatch, None);
            } else if go {
                let search = PathBuf::from(dir.trim());
                self.spawn_task(move |lib| {
                    let missing = match lib.find_missing() {
                        Ok(m) => m,
                        Err(e) => return TaskOut::Message { text: e.to_string(), error: true, refresh: Refresh::NONE },
                    };
                    let cancel = CancelToken::new();
                    let mut done = 0usize;
                    for a in &missing {
                        if let Ok(c) = lib.relink_candidates(&a.id, &search, 1, &cancel) {
                            if let Some(p) = c.first() {
                                if lib.relink(&a.id, p).is_ok() {
                                    done += 1;
                                }
                            }
                        }
                    }
                    TaskOut::Message { text: format!("{done}/{} {}", missing.len(), tr("lib.relink.batch.result")), error: done == 0 && !missing.is_empty(), refresh: Refresh::ALL }
                });
            } else if open {
                self.dlg.relink_batch = Some(dir);
            }
        }
    }

    // ================================================================ coleção inteligente

    fn smart_window(&mut self, ctx: &egui::Context) {
        if !self.dlg.open_smart {
            return;
        }
        let mut open = true;
        let mut create = false;
        egui::Window::new(format!("{}  {}", icons::MAGIC_WAND, tr("lib.coll.save_smart"))).id(egui::Id::new("lib_smart_win")).open(&mut open).collapsible(false).resizable(false).default_pos(win_pos(ctx, 520.0, 400.0)).show(ctx, |ui| {
            ui.label(small_dim(tr("lib.coll.save_smart.tip")));
            let r = ui.add(egui::TextEdit::singleline(&mut self.dlg.smart_name).hint_text(tr("lib.coll.name_hint")).desired_width(300.0));
            r.request_focus();
            if (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) || ui.button(tr("lib.coll.create")).clicked()) && !self.dlg.smart_name.trim().is_empty() {
                create = true;
            }
        });
        if create {
            let name = self.dlg.smart_name.trim().to_string();
            let mut q = self.filters.to_query();
            q.after = None;
            q.limit = 0;
            q.with_total = false;
            // a seção atual (tag/favoritos/raiz) faz parte da consulta; coleção dentro de coleção não
            q.collection = None;
            self.command("collection", Refresh::ROWS_FACETS, move |lib| lib.create_smart_collection(&name, &q).map(|_| tr("lib.coll.created").to_string()).map_err(|e| e.to_string()));
            self.dlg.open_smart = false;
        } else if !open {
            self.dlg.open_smart = false;
        }
    }
}

fn kv_row(ui: &mut Ui, k: &str, v: String) {
    ui.label(small_dim(k));
    ui.label(RichText::new(v).size(12.0));
    ui.end_row();
}

fn dir_bytes(p: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(t) if t.is_file() => total += e.metadata().map(|m| m.len()).unwrap_or(0),
                _ => {}
            }
        }
    }
    total
}

/// Uso de disco da biblioteca (percorre as pastas; pode levar alguns segundos em bibliotecas grandes).
pub fn measure_disk(lib: &Library) -> DiskUsage {
    let t = Instant::now();
    let home = lib.home();
    let assets = dir_bytes(&home.join("assets")) + dir_bytes(&home.join("linked"));
    let trash = dir_bytes(&home.join("trash"));
    let idx = lib.index_path();
    let index = std::fs::metadata(idx).map(|m| m.len()).unwrap_or(0)
        + std::fs::metadata(format!("{}-wal", idx.display())).map(|m| m.len()).unwrap_or(0)
        + std::fs::metadata(format!("{}-shm", idx.display())).map(|m| m.len()).unwrap_or(0);
    let thumbs = lib.thumbnails().disk_usage();
    DiskUsage { home_bytes: dir_bytes(home), assets_bytes: assets, trash_bytes: trash, index_bytes: index, thumbs_bytes: thumbs, elapsed_ms: t.elapsed().as_millis() as u64 }
}

/// Posição inicial centralizada (sem ancorar: a janela continua arrastável).
fn win_pos(ctx: &egui::Context, w: f32, h: f32) -> egui::Pos2 {
    let r = ctx.content_rect();
    egui::Pos2::new((r.center().x - w / 2.0).max(8.0), (r.center().y - h / 2.0).max(48.0))
}
