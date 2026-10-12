//! Jobs em segundo plano da Biblioteca: importação (fila), scan de locais ligados, verificação de
//! integridade e reconstrução do índice. Os eventos são recolhidos a cada quadro (`poll_jobs`).

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Imported,
    Revised,
    Duplicate,
    Skipped,
    Failed,
    Relinked,
}

#[derive(Debug, Clone)]
pub struct ImportEntry {
    pub kind: EntryKind,
    pub path: PathBuf,
    pub detail: String,
}

/// Limite de linhas guardadas no relatório (as contagens continuam exatas).
const LOG_LIMIT: usize = 5000;

#[derive(Default)]
pub struct ImportLog {
    pub entries: Vec<ImportEntry>,
    pub imported: usize,
    pub revised: usize,
    pub duplicates: usize,
    pub failed: usize,
    pub skipped: usize,
    pub cancelled: bool,
    pub truncated: bool,
    pub open: bool,
}

impl ImportLog {
    pub fn push(&mut self, e: ImportEntry) {
        match e.kind {
            EntryKind::Imported => self.imported += 1,
            EntryKind::Revised => self.revised += 1,
            EntryKind::Duplicate => self.duplicates += 1,
            EntryKind::Failed => self.failed += 1,
            EntryKind::Skipped => self.skipped += 1,
            EntryKind::Relinked => {}
        }
        if self.entries.len() < LOG_LIMIT {
            self.entries.push(e);
        } else {
            self.truncated = true;
        }
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.imported + self.failed + self.duplicates + self.revised + self.skipped == 0
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

pub struct ImportRun {
    pub job: Option<ImportJob>,
    pub label: String,
    pub discovered: usize,
    pub done: usize,
    pub total: usize,
    pub bytes: u64,
    pub cancel_requested: bool,
}

pub struct ScanRun {
    pub job: Option<ScanJob>,
    pub root: RootId,
    pub name: String,
    pub done: usize,
    pub total: usize,
}

pub struct VerifyRun {
    pub job: Option<VerifyJob>,
    pub total: usize,
    pub done: usize,
    pub issues: Vec<VerifyIssue>,
    pub report: Option<VerifyReport>,
}

pub struct RebuildRun {
    pub job: Option<Job<(), RebuildReport>>,
}

#[derive(Default)]
pub struct Jobs {
    pub import: Option<ImportRun>,
    pub queue: VecDeque<(PathBuf, ImportOptions)>,
    pub log: ImportLog,
    pub scan: Option<ScanRun>,
    pub scan_queue: VecDeque<RootId>,
    pub verify: Option<VerifyRun>,
    pub last_verify: Option<VerifyRun>,
    pub rebuild: Option<RebuildRun>,
    pub last_rebuild: Option<String>,
}

impl Jobs {
    pub fn any_running(&self) -> bool {
        self.import.is_some() || !self.queue.is_empty() || self.scan.is_some() || !self.scan_queue.is_empty() || self.verify.is_some() || self.rebuild.is_some()
    }
}

fn kind_entry(ev: &ImportEvent) -> Option<ImportEntry> {
    Some(match ev {
        ImportEvent::Imported { name, path, kind, .. } => ImportEntry { kind: EntryKind::Imported, path: path.clone(), detail: format!("{name} ({kind})") },
        ImportEvent::Revised { revision, path, .. } => ImportEntry { kind: EntryKind::Revised, path: path.clone(), detail: format!("r{revision}") },
        ImportEvent::Duplicate { path, existing } => ImportEntry { kind: EntryKind::Duplicate, path: path.clone(), detail: existing.to_string() },
        ImportEvent::Skipped { path, reason } => ImportEntry { kind: EntryKind::Skipped, path: path.clone(), detail: reason.clone() },
        ImportEvent::Failed { path, error } => ImportEntry { kind: EntryKind::Failed, path: path.clone(), detail: error.clone() },
        ImportEvent::Relinked { path, .. } => ImportEntry { kind: EntryKind::Relinked, path: path.clone(), detail: String::new() },
        _ => return None,
    })
}

impl LibraryWorkspace {
    // ------------------------------------------------------------------ import

    /// Enfileira a importação gerenciada de arquivos/pastas.
    pub fn queue_import(&mut self, paths: Vec<PathBuf>, opts: ImportOptions) {
        for p in paths {
            self.jobs.queue.push_back((p, opts.clone()));
        }
        self.start_next_import();
    }

    pub(crate) fn start_next_import(&mut self) {
        if self.jobs.import.is_some() {
            return;
        }
        let Some(rt) = &self.rt else { return };
        let Some((path, opts)) = self.jobs.queue.pop_front() else { return };
        let label = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string());
        let job = rt.lib.import(&path, opts);
        self.jobs.import = Some(ImportRun { job: Some(job), label, discovered: 0, done: 0, total: 0, bytes: 0, cancel_requested: false });
        self.jobs.log.open = true;
    }

    pub(crate) fn cancel_imports(&mut self) {
        self.jobs.queue.clear();
        if let Some(run) = &mut self.jobs.import {
            run.cancel_requested = true;
            if let Some(j) = &run.job {
                j.cancel();
            }
        }
    }

    // -------------------------------------------------------------------- scan

    pub(crate) fn queue_scan(&mut self, root: RootId) {
        if !self.jobs.scan_queue.contains(&root) && self.jobs.scan.as_ref().map(|s| s.root) != Some(root) {
            self.jobs.scan_queue.push_back(root);
        }
    }

    pub(crate) fn queue_scan_all_linked(&mut self) {
        let Some(rt) = &self.rt else { return };
        let roots = rt.lib.roots().unwrap_or_default();
        for r in roots.into_iter().filter(|r| r.mode == RootMode::Linked && r.online) {
            self.queue_scan(r.id);
        }
    }

    fn start_next_scan(&mut self) {
        if self.jobs.scan.is_some() {
            return;
        }
        let Some(rt) = &self.rt else { return };
        while let Some(root) = self.jobs.scan_queue.pop_front() {
            let Ok(info) = rt.lib.root(root) else { continue };
            let job = rt.lib.scan(root, ScanOptions::default());
            self.jobs.scan = Some(ScanRun { job: Some(job), root, name: info.name, done: 0, total: 0 });
            return;
        }
    }

    // ------------------------------------------------------------ verify/rebuild

    pub(crate) fn start_verify(&mut self, opts: VerifyOptions) {
        if self.jobs.verify.is_some() {
            return;
        }
        let Some(rt) = &self.rt else { return };
        let job = rt.lib.verify(opts.clone());
        self.jobs.verify = Some(VerifyRun { job: Some(job), total: 0, done: 0, issues: Vec::new(), report: None });
    }

    pub(crate) fn start_rebuild(&mut self) {
        if self.jobs.rebuild.is_some() {
            return;
        }
        let Some(rt) = &self.rt else { return };
        self.jobs.rebuild = Some(RebuildRun { job: Some(rt.lib.rebuild_index_job()) });
    }

    // ----------------------------------------------------------------- polling

    pub(crate) fn poll_jobs(&mut self, refresh_rows: &mut bool, refresh_facets: &mut bool) {
        self.start_next_import();
        self.start_next_scan();

        // ---- import
        let mut finished_import: Option<Result<ImportReport, String>> = None;
        if let Some(run) = &mut self.jobs.import {
            if let Some(job) = &run.job {
                let done = job.is_finished();
                let evs: Vec<ImportEvent> = job.events().try_iter().collect();
                for ev in evs {
                    match &ev {
                        ImportEvent::Discovered { files } => run.discovered = *files,
                        ImportEvent::Progress { done, total, bytes } => {
                            run.done = *done;
                            run.total = *total;
                            run.bytes = *bytes;
                        }
                        _ => {}
                    }
                    if let Some(e) = kind_entry(&ev) {
                        self.jobs.log.push(e);
                    }
                }
                if done {
                    let job = run.job.take().unwrap();
                    finished_import = Some(job.join().map_err(|e| e.to_string()));
                }
            }
        }
        if let Some(res) = finished_import {
            let run = self.jobs.import.take();
            match res {
                Ok(rep) => {
                    self.jobs.log.cancelled |= rep.cancelled;
                    let msg = format!(
                        "{}: {} {}, {} {}, {} {}",
                        run.map(|r| r.label).unwrap_or_default(),
                        rep.imported.len(),
                        crate::i18n::tr("lib.imp.imported"),
                        rep.duplicates.len(),
                        crate::i18n::tr("lib.imp.duplicates"),
                        rep.failed.len(),
                        crate::i18n::tr("lib.imp.failed"),
                    );
                    if rep.failed.is_empty() {
                        self.info(msg);
                    } else {
                        self.error(msg);
                    }
                }
                Err(e) => self.error(format!("{}: {e}", crate::i18n::tr("lib.imp.title"))),
            }
            *refresh_rows = true;
            *refresh_facets = true;
            self.start_next_import();
        }

        // ---- scan
        let mut finished_scan: Option<(String, Result<ScanReport, String>)> = None;
        if let Some(run) = &mut self.jobs.scan {
            if let Some(job) = &run.job {
                let done = job.is_finished();
                let evs: Vec<ImportEvent> = job.events().try_iter().collect();
                for ev in evs {
                    if let ImportEvent::Progress { done, total, .. } = &ev {
                        run.done = *done;
                        run.total = *total;
                    }
                    if let Some(e) = kind_entry(&ev) {
                        if e.kind == EntryKind::Failed {
                            self.jobs.log.push(e);
                        }
                    }
                }
                if done {
                    let job = run.job.take().unwrap();
                    finished_scan = Some((run.name.clone(), job.join().map_err(|e| e.to_string())));
                }
            }
        }
        if let Some((name, res)) = finished_scan {
            self.jobs.scan = None;
            match res {
                Ok(rep) => {
                    if rep.root_offline {
                        self.error(format!("{name}: {}", crate::i18n::tr("lib.root_offline")));
                    } else if !rep.import.imported.is_empty() || !rep.missing.is_empty() || !rep.import.revised.is_empty() || !rep.import.failed.is_empty() {
                        self.info(format!(
                            "{name}: +{} {} · {} {} · {} {}",
                            rep.import.imported.len(),
                            crate::i18n::tr("lib.imp.imported"),
                            rep.missing.len(),
                            crate::i18n::tr("lib.scan.missing"),
                            rep.import.failed.len(),
                            crate::i18n::tr("lib.imp.failed")
                        ));
                    }
                    *refresh_rows = true;
                    *refresh_facets = true;
                }
                Err(e) => self.error(format!("scan {name}: {e}")),
            }
            self.start_next_scan();
        }

        // ---- verify
        let mut finished_verify = None;
        if let Some(run) = &mut self.jobs.verify {
            if let Some(job) = &run.job {
                let done = job.is_finished();
                let evs: Vec<VerifyEvent> = job.events().try_iter().collect();
                for ev in evs {
                    match ev {
                        VerifyEvent::Started { total } => run.total = total,
                        VerifyEvent::Progress { done, total } => {
                            run.done = done;
                            run.total = total;
                        }
                        VerifyEvent::Issue(i) => {
                            if run.issues.len() < 5000 {
                                run.issues.push(i)
                            }
                        }
                        VerifyEvent::Finished => {}
                    }
                }
                if done {
                    let job = run.job.take().unwrap();
                    finished_verify = Some(job.join().map_err(|e| e.to_string()));
                }
            }
        }
        if let Some(res) = finished_verify {
            let mut run = self.jobs.verify.take().unwrap();
            match res {
                Ok(rep) => {
                    let msg = format!("{} {} · {} {}", rep.checked, crate::i18n::tr("lib.verify.checked"), rep.issues.len(), crate::i18n::tr("lib.verify.issues"));
                    if rep.issues.is_empty() {
                        self.info(msg);
                    } else {
                        self.error(msg);
                    }
                    run.report = Some(rep);
                }
                Err(e) => self.error(format!("verify: {e}")),
            }
            self.jobs.last_verify = Some(run);
            *refresh_rows = true;
            *refresh_facets = true;
        }

        // ---- rebuild
        let mut finished_rebuild = None;
        if let Some(run) = &mut self.jobs.rebuild {
            if run.job.as_ref().is_some_and(|j| j.is_finished()) {
                finished_rebuild = Some(run.job.take().unwrap().join().map_err(|e| e.to_string()));
            }
        }
        if let Some(res) = finished_rebuild {
            self.jobs.rebuild = None;
            match res {
                Ok(rep) => {
                    let msg = format!("{}: {} · {} {} · {} ms", crate::i18n::tr("lib.rebuild.done"), rep.assets, crate::i18n::tr("lib.rebuild.bad"), rep.bad_manifests.len(), rep.elapsed_ms);
                    self.jobs.last_rebuild = Some(msg.clone());
                    self.info(msg);
                }
                Err(e) => self.error(format!("rebuild: {e}")),
            }
            *refresh_rows = true;
            *refresh_facets = true;
            // o índice foi reconstruído: cursores e linhas antigos não valem mais
            self.list.reset();
            *refresh_rows = true;
        }
    }
}
