//! Duplicados (por BLAKE3), relink de ausentes e verificacao de integridade (T-5.8, parte de dados).

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use rusqlite::params;
use serde::Serialize;

use crate::error::{AssetError, Result};
use crate::job::{CancelToken, Job, JobCtx};
use crate::library::{read_manifest, Library};
use crate::model::*;
use crate::paths::relative_to;
use crate::store::{ROW_COLS, ROW_FROM};
use crate::trash::TrashOptions;
use crate::util::{hash_file, mtime_ns, now_secs};

// ---------------------------------------------------------------------------------------------
// Duplicados
// ---------------------------------------------------------------------------------------------

/// Grupo de ativos com o mesmo conteudo.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DuplicateGroup {
    pub content_hash: ContentHash,
    /// Tamanho de UMA copia.
    pub size: u64,
    pub assets: Vec<AssetRow>,
}

impl DuplicateGroup {
    /// Bytes recuperaveis se so uma copia MANAGED for mantida (`tamanho * (copias managed - 1)`).
    pub fn reclaimable_bytes(&self) -> u64 {
        let managed = self.assets.iter().filter(|a| a.storage == StorageMode::Managed).count() as u64;
        self.size * managed.saturating_sub(1)
    }
}

impl Library {
    /// Grupos de ativos nao-lixeira com o mesmo hash, os maiores desperdicios primeiro. No maximo `limit` grupos.
    pub fn find_duplicates(&self, limit: usize) -> Result<Vec<DuplicateGroup>> {
        let hashes: Vec<(Vec<u8>, i64)> = self.inner.db.read(|c| {
            let mut st = c.prepare(
                "SELECT content_hash, size FROM asset WHERE state != 1 GROUP BY content_hash HAVING count(*) > 1 \
                 ORDER BY size * (count(*) - 1) DESC LIMIT ?1",
            )?;
            let rows = st.query_map([limit.min(100_000) as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })?;
        let mut out = Vec::with_capacity(hashes.len());
        for (h, size) in hashes {
            let Some(hash) = ContentHash::from_slice(&h) else { continue };
            let assets = self.inner.db.read(|c| {
                let sql = format!("SELECT {ROW_COLS} {ROW_FROM} WHERE a.content_hash = ?1 AND a.state != 1 ORDER BY a.imported_at, a.seq");
                let mut st = c.prepare(&sql)?;
                let rows = st.query_map([h.as_slice()], crate::store::row_to_asset)?;
                Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
            })?;
            out.push(DuplicateGroup { content_hash: hash, size: size.max(0) as u64, assets });
        }
        Ok(out)
    }

    /// Resolve um grupo de duplicados: mantem `keep`, une tags/favorito/nota/colecoes/dependentes nele e envia
    /// `remove` para a lixeira. Todos devem ter o mesmo hash de `keep`.
    pub fn resolve_duplicates(&self, keep: &AssetId, remove: &[AssetId]) -> Result<()> {
        let keeper = self.require_row(keep)?;
        let mut merged_tags: Vec<String> = self.tags_of(keep)?;
        let mut favorite = keeper.favorite;
        let mut rating = keeper.rating;
        for r in remove {
            if r == keep {
                return Err(AssetError::InvalidInput("'keep' nao pode estar em 'remove'".into()));
            }
            let row = self.require_row(r)?;
            if row.content_hash != keeper.content_hash {
                return Err(AssetError::InvalidInput(format!("{r} nao e duplicado de {keep} (hash diferente)")));
            }
            merged_tags.extend(self.tags_of(r)?);
            favorite |= row.favorite;
            rating = rating.max(row.rating);
        }
        self.set_tags(keep, &merged_tags)?;
        self.set_favorite(keep, favorite)?;
        self.set_rating(keep, rating)?;
        for r in remove {
            // colecoes manuais
            let colls = self.collections_of(r)?;
            for c in colls {
                self.add_to_collection(c, &[*keep])?;
            }
            // dependentes passam a apontar para o ativo mantido
            for user in self.used_by_filtered(r, true)? {
                let kinds: Vec<String> = self.dependencies(&user)?.into_iter().filter(|d| d.id == *r).map(|d| d.kind).collect();
                for k in kinds {
                    self.remove_dependency(&user, r, &k)?;
                    if user != *keep {
                        self.add_dependency(&user, keep, &k)?;
                    }
                }
            }
        }
        self.trash(remove, TrashOptions { force: true })?;
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // Ausentes / relink
    // -----------------------------------------------------------------------------------------

    /// Ativos linked `missing`.
    pub fn find_missing(&self) -> Result<Vec<AssetRow>> {
        let q = crate::query::Query { state: crate::query::StateFilter::Missing, limit: 1000, ..Default::default() };
        let mut out = Vec::new();
        let mut q = q;
        loop {
            let page = self.query(&q)?;
            out.extend(page.items);
            match page.next {
                Some(c) => q.after = Some(c),
                None => return Ok(out),
            }
        }
    }

    /// Procura em `search_dir` (recursivo) arquivos com o mesmo tamanho E hash do primario do ativo. Retorna ate
    /// `max_results` caminhos. Cancelavel por `cancel`.
    pub fn relink_candidates(&self, id: &AssetId, search_dir: &Path, max_results: usize, cancel: &CancelToken) -> Result<Vec<PathBuf>> {
        let m = self.manifest(id)?;
        let rev = m.current().ok_or_else(|| AssetError::Corrupt("sem revisao atual".into()))?;
        let prim = rev.files.first().ok_or_else(|| AssetError::Corrupt("revisao sem arquivos".into()))?;
        let mut out = Vec::new();
        let mut visited = 0usize;
        for e in walkdir::WalkDir::new(search_dir).follow_links(false).max_depth(self.limits().max_scan_depth).into_iter().filter_map(|e| e.ok()) {
            visited += 1;
            if visited % 256 == 0 {
                cancel.check()?;
            }
            if visited > self.limits().max_scan_files {
                break;
            }
            if !e.file_type().is_file() {
                continue;
            }
            if e.metadata().map(|m| m.len()).unwrap_or(u64::MAX) != prim.size {
                continue;
            }
            if let Ok((h, _)) = hash_file(e.path(), Some(cancel)) {
                if h == prim.blake3 {
                    out.push(e.path().to_path_buf());
                    if out.len() >= max_results {
                        break;
                    }
                }
            }
        }
        Ok(out)
    }

    /// Aponta um ativo linked para `new_file` (dentro de uma raiz linked registrada). Exige o MESMO conteudo
    /// (tamanho + BLAKE3 do primario); se o arquivo mudou, use `scan` para registrar uma nova revisao.
    pub fn relink(&self, id: &AssetId, new_file: &Path) -> Result<()> {
        let row = self.require_row(id)?;
        if row.storage != StorageMode::Linked {
            return Err(AssetError::InvalidInput("relink so vale para ativos linked".into()));
        }
        let new_file = new_file.canonicalize().map_err(|e| AssetError::io(new_file, e))?;
        let roots = self.roots()?;
        let root = roots
            .iter()
            .find(|r| r.mode == RootMode::Linked && new_file.starts_with(&r.path))
            .ok_or_else(|| AssetError::InvalidInput("o arquivo nao esta dentro de nenhuma raiz linked registrada".into()))?;
        let m = self.manifest(id)?;
        let prim = m.current().and_then(|r| r.files.first()).ok_or_else(|| AssetError::Corrupt("revisao sem arquivos".into()))?.clone();
        let meta = std::fs::metadata(&new_file).map_err(|e| AssetError::io(&new_file, e))?;
        if meta.len() != prim.size {
            return Err(AssetError::Conflict("tamanho diferente do original".into()));
        }
        let (h, _) = hash_file(&new_file, None)?;
        if h != prim.blake3 {
            return Err(AssetError::Conflict("conteudo diferente do original (hash BLAKE3 nao confere)".into()));
        }
        let rel = relative_to(&root.path, &new_file).ok_or_else(|| AssetError::InvalidInput("caminho fora da raiz".into()))?;
        let root_uuid = root.uuid.clone();
        let file_name = new_file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let (loc, m) = self.mutate_manifest(id, |m| {
            m.link = Some(LinkInfo { root_uuid: root_uuid.clone(), rel_path: rel.clone() });
            m.primary = file_name.clone();
            if let Some(rev) = m.revisions.iter_mut().find(|r| r.rev == m.current_revision) {
                if let Some(f) = rev.files.first_mut() {
                    f.path = file_name.clone();
                }
            }
            Ok(())
        })?;
        let mut items = vec![crate::import::commit::IndexItem {
            manifest: m,
            root_id: root.id.0,
            manifest_dir: loc.manifest_dir.clone(),
            state: AssetState::Active,
            file_mtime_ns: mtime_ns(&meta),
        }];
        self.flush_index_items(&mut items)
    }

    /// Troca o caminho de uma raiz linked (disco remontado em outra letra/ponto). Os caminhos relativos dos
    /// ativos continuam validos. Em seguida roda `check_presence`.
    pub fn relink_root(&self, root: RootId, new_path: &Path) -> Result<(Vec<AssetId>, Vec<AssetId>)> {
        let info = self.root(root)?;
        if info.mode != RootMode::Linked {
            return Err(AssetError::InvalidInput("relink_root so vale para raizes linked".into()));
        }
        if !new_path.is_dir() {
            return Err(AssetError::NotFound(format!("pasta {} nao existe", new_path.display())));
        }
        let canon = new_path.canonicalize().map_err(|e| AssetError::io(new_path, e))?;
        if canon.starts_with(self.home()) || self.home().starts_with(&canon) {
            return Err(AssetError::Conflict("a raiz nao pode conter nem estar dentro da pasta da biblioteca".into()));
        }
        let mut cfg = self.config_snapshot()?;
        for r in cfg.roots.iter().filter(|r| r.id != 1 && r.id != root.0) {
            let other = PathBuf::from(&r.path);
            if canon.starts_with(&other) || other.starts_with(&canon) {
                return Err(AssetError::Conflict(format!("sobrepoe a raiz {}", r.id)));
            }
        }
        if let Some(rc) = cfg.roots.iter_mut().find(|r| r.id == root.0) {
            rc.path = canon.to_string_lossy().to_string();
        }
        self.save_config(&cfg)?;
        {
            let mut g = self.inner.config.lock().map_err(|_| AssetError::Internal("config envenenada".into()))?;
            *g = cfg;
        }
        self.refresh_roots()?;
        self.check_presence(root)
    }

    // -----------------------------------------------------------------------------------------
    // Verificacao de integridade
    // -----------------------------------------------------------------------------------------

    /// Verifica a integridade em background. Eventos: progresso e cada problema.
    pub fn verify(&self, opts: VerifyOptions) -> VerifyJob {
        let lib = self.clone();
        Job::spawn("anigo-verify", move |ctx| lib.run_verify(&ctx, &opts))
    }

    /// `verify` sincrono.
    pub fn verify_blocking(&self, opts: VerifyOptions) -> Result<VerifyReport> {
        self.verify(opts).join()
    }

    fn run_verify(&self, ctx: &JobCtx<VerifyEvent>, opts: &VerifyOptions) -> Result<VerifyReport> {
        let started = std::time::Instant::now();
        let mut rep = VerifyReport::default();
        let roots = self.roots()?;
        let offline: Vec<i64> = roots.iter().filter(|r| !r.online).map(|r| r.id.0).collect();
        for r in roots.iter().filter(|r| !r.online) {
            let issue = VerifyIssue { asset: None, kind: IssueKind::RootOffline, detail: format!("raiz {} ({}) offline", r.id, r.path.display()) };
            ctx.emit(VerifyEvent::Issue(issue.clone()));
            rep.issues.push(issue);
        }

        // 1. linhas do indice (id, localizacao)
        #[derive(Clone)]
        struct Item {
            id: AssetId,
            root_id: i64,
            storage: StorageMode,
            manifest_dir: String,
            state: AssetState,
            hash: ContentHash,
        }
        let items: Vec<Item> = self.inner.db.read(|c| {
            let mut st = c.prepare("SELECT id, root_id, storage, manifest_dir, state, content_hash FROM asset ORDER BY seq")?;
            let rows = st.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?, r.get::<_, Vec<u8>>(5)?)))?;
            let mut v = Vec::new();
            for r in rows {
                let (id, root_id, storage, dir, state, hash) = r?;
                if let (Some(id), Some(hash)) = (AssetId::from_bytes(&id), ContentHash::from_slice(&hash)) {
                    v.push(Item { id, root_id, storage: StorageMode::from_db(storage), manifest_dir: dir, state: AssetState::from_db(state), hash });
                }
            }
            Ok(v)
        })?;
        let total = items.len();
        ctx.emit(VerifyEvent::Started { total });

        // 2. checagem em paralelo, em lotes (progresso e cancelamento entre lotes)
        struct Outcome {
            item: Item,
            issues: Vec<VerifyIssue>,
            new_state: Option<AssetState>,
            hashed: u64,
        }
        let lib = self;
        let cancel = ctx.cancel.clone();
        let rehash = opts.rehash;
        let mut done = 0usize;
        let mut outcomes: Vec<Outcome> = Vec::new();
        for chunk in items.chunks(256) {
            if cancel.is_cancelled() {
                rep.cancelled = true;
                break;
            }
            let res: Vec<Outcome> = chunk
                .par_iter()
                .map(|it| {
                    let mut issues = Vec::new();
                    let mut hashed = 0u64;
                    let mut new_state: Option<AssetState> = None;
                    let mut push = |kind: IssueKind, detail: String| issues.push(VerifyIssue { asset: Some(it.id), kind, detail });
                    let base = match lib.manifest_base(it.storage, it.root_id) {
                        Ok(b) => b,
                        Err(e) => {
                            push(IssueKind::ManifestUnreadable, e.to_string());
                            return Outcome { item: it.clone(), issues, new_state, hashed };
                        }
                    };
                    let dir = base.join(&it.manifest_dir);
                    let m = match read_manifest(&dir) {
                        Ok(m) => m,
                        Err(e) => {
                            push(IssueKind::ManifestUnreadable, format!("{}: {e}", dir.display()));
                            return Outcome { item: it.clone(), issues, new_state, hashed };
                        }
                    };
                    if m.content_hash() != Some(it.hash) {
                        push(IssueKind::IndexMismatch, "hash do indice difere do manifesto".into());
                    }
                    let Some(rev) = m.current() else {
                        push(IssueKind::ManifestUnreadable, "sem revisao atual".into());
                        return Outcome { item: it.clone(), issues, new_state, hashed };
                    };
                    // base dos arquivos
                    let (fbase, offline_root) = match m.storage {
                        StorageMode::Managed => (Some(dir.join(crate::library::layout::revision_dir(m.current_revision))), false),
                        StorageMode::Linked => match m.link.as_ref().and_then(|l| lib.root_id_by_uuid(&l.root_uuid).map(|rid| (rid, l))) {
                            Some((rid, l)) => {
                                let off = offline.contains(&rid);
                                let p = lib.root_abs_path(rid).ok().map(|rp| rp.join(&l.rel_path)).and_then(|p| p.parent().map(|x| x.to_path_buf()));
                                (p, off)
                            }
                            None => {
                                push(IssueKind::ManifestUnreadable, "raiz do link nao registrada".into());
                                (None, false)
                            }
                        },
                    };
                    let Some(fbase) = fbase else { return Outcome { item: it.clone(), issues, new_state, hashed } };
                    if offline_root {
                        return Outcome { item: it.clone(), issues, new_state, hashed };
                    }
                    if it.state == AssetState::Trashed && m.storage == StorageMode::Linked {
                        return Outcome { item: it.clone(), issues, new_state, hashed };
                    }
                    let mut state_bad: Option<AssetState> = None;
                    for (i, f) in rev.files.iter().enumerate() {
                        let p = fbase.join(&f.path);
                        match std::fs::metadata(&p) {
                            Err(_) => {
                                push(IssueKind::MissingFile, f.path.clone());
                                if i == 0 && m.storage == StorageMode::Linked {
                                    state_bad = Some(AssetState::Missing);
                                } else if m.storage == StorageMode::Managed {
                                    state_bad = Some(AssetState::Corrupt);
                                }
                            }
                            Ok(meta) => {
                                if meta.len() != f.size {
                                    push(IssueKind::SizeMismatch, format!("{}: {} != {}", f.path, meta.len(), f.size));
                                    if m.storage == StorageMode::Managed {
                                        state_bad = Some(AssetState::Corrupt);
                                    }
                                } else if rehash {
                                    match hash_file(&p, Some(&cancel)) {
                                        Ok((h, n)) => {
                                            hashed += n;
                                            if h != f.blake3 {
                                                push(
                                                    if m.storage == StorageMode::Managed { IssueKind::HashMismatch } else { IssueKind::ExternalChanged },
                                                    f.path.clone(),
                                                );
                                                if m.storage == StorageMode::Managed {
                                                    state_bad = Some(AssetState::Corrupt);
                                                }
                                            }
                                        }
                                        Err(_) => {} // cancelado
                                    }
                                }
                            }
                        }
                    }
                    for d in &m.dependencies {
                        // dependencias para ativos inexistentes
                        if lib.row(&d.id).ok().flatten().is_none() {
                            push(IssueKind::DanglingDependency, d.id.to_string());
                        }
                    }
                    new_state = match (state_bad, it.state) {
                        (Some(s), _) => Some(s),
                        (None, AssetState::Missing | AssetState::Corrupt) => Some(AssetState::Active),
                        _ => None,
                    };
                    Outcome { item: it.clone(), issues, new_state, hashed }
                })
                .collect();
            for o in res {
                done += 1;
                rep.checked += 1;
                rep.bytes_hashed += o.hashed;
                if o.issues.is_empty() {
                    rep.ok += 1;
                }
                for i in &o.issues {
                    ctx.emit(VerifyEvent::Issue(i.clone()));
                }
                rep.issues.extend(o.issues.iter().cloned());
                outcomes.push(o);
            }
            ctx.emit(VerifyEvent::Progress { done, total });
        }

        // 3. orfaos (disco x indice)
        if !rep.cancelled {
            let (orphans, rows_without_dir) = self.index_vs_disk()?;
            for (f, _, id) in &orphans {
                let issue = VerifyIssue { asset: Some(*id), kind: IssueKind::OrphanManifest, detail: f.rel.clone() };
                ctx.emit(VerifyEvent::Issue(issue.clone()));
                rep.issues.push(issue);
            }
            for id in &rows_without_dir {
                let issue = VerifyIssue { asset: Some(*id), kind: IssueKind::OrphanRow, detail: "linha do indice sem pasta de manifesto".into() };
                ctx.emit(VerifyEvent::Issue(issue.clone()));
                rep.issues.push(issue);
            }
            // 4. correcoes
            if opts.fix {
                let changes: Vec<(AssetId, AssetState)> = outcomes.iter().filter_map(|o| o.new_state.map(|s| (o.item.id, s))).collect();
                if !changes.is_empty() || !rows_without_dir.is_empty() {
                    self.inner.db.write(|c| {
                        let tx = c.transaction()?;
                        for (id, s) in &changes {
                            tx.execute("UPDATE asset SET state = ?2 WHERE id = ?1 AND state != 1", params![id.as_bytes().as_slice(), s.to_db()])?;
                        }
                        for id in &rows_without_dir {
                            if let Some(seq) = crate::store::seq_of(&tx, id)? {
                                crate::store::delete_asset_rows(&tx, seq, id)?;
                            }
                        }
                        tx.commit()?;
                        Ok(())
                    })?;
                    rep.fixed += changes.len() + rows_without_dir.len();
                }
                for (f, base, _) in &orphans {
                    if self.index_orphan(f, base).is_ok() {
                        rep.fixed += 1;
                    }
                }
            }
        }
        rep.elapsed_ms = started.elapsed().as_millis() as u64;
        let _ = now_secs();
        ctx.emit(VerifyEvent::Finished);
        Ok(rep)
    }
}

/// Opcoes de `verify`.
#[derive(Debug, Clone)]
pub struct VerifyOptions {
    /// Recalcula o BLAKE3 de todos os arquivos (lento em bibliotecas grandes). Sem isso so confere existencia e tamanho.
    pub rehash: bool,
    /// Aplica correcoes seguras: atualiza `missing`/`corrupt`/`active`, remove linhas orfas, indexa manifestos orfaos.
    /// Nunca apaga arquivos.
    pub fix: bool,
}

impl Default for VerifyOptions {
    fn default() -> Self {
        Self { rehash: true, fix: false }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum IssueKind {
    /// Arquivo listado no manifesto nao existe.
    MissingFile,
    SizeMismatch,
    /// Managed: o conteudo nao confere com o BLAKE3 do manifesto (corrupcao).
    HashMismatch,
    /// Linked: o arquivo externo foi alterado (use `scan` para criar uma revisao).
    ExternalChanged,
    ManifestUnreadable,
    /// O hash do indice nao e o do manifesto.
    IndexMismatch,
    /// Pasta com `asset.json` que o indice nao conhece.
    OrphanManifest,
    /// Linha do indice sem pasta correspondente.
    OrphanRow,
    DanglingDependency,
    RootOffline,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VerifyIssue {
    pub asset: Option<AssetId>,
    pub kind: IssueKind,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VerifyEvent {
    Started { total: usize },
    Progress { done: usize, total: usize },
    Issue(VerifyIssue),
    Finished,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct VerifyReport {
    pub checked: usize,
    pub ok: usize,
    pub issues: Vec<VerifyIssue>,
    pub bytes_hashed: u64,
    pub fixed: usize,
    pub cancelled: bool,
    pub elapsed_ms: u64,
}

pub type VerifyJob = Job<VerifyEvent, VerifyReport>;
