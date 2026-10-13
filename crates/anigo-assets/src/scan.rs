//! Scan incremental de raizes `linked` (T-5.3): mtime + tamanho para pular o que nao mudou, BLAKE3 so do
//! que e novo/alterado, deteccao de arquivos movidos por hash, marcacao de ausentes.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::error::{AssetError, Result};
use crate::import::{enumerate, run_pipeline, Decision, DuplicatePolicy, ImportEvent, ImportOptions, ImportReport, Prepared, RunParams};
use crate::job::{Job, JobCtx};
use crate::library::Library;
use crate::model::*;
use crate::paths::relative_to;
use crate::util::{mtime_ns, now_secs};

/// Opcoes de `scan`.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Ignora a otimizacao mtime+tamanho e re-hasheia tudo.
    pub rehash_all: bool,
    /// Reencontra ativos `missing` que foram movidos (mesmo hash, outro caminho).
    pub detect_moves: bool,
    /// Marca como `missing` os ativos cujo arquivo nao existe mais.
    pub mark_missing: bool,
    /// Duplicatas dentro de pastas ligadas: `Allow` (padrao) registra todas as copias, evitando re-hash a cada scan.
    pub duplicates: DuplicatePolicy,
    pub threads: usize,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self { rehash_all: false, detect_moves: true, mark_missing: true, duplicates: DuplicatePolicy::Allow, threads: 0 }
    }
}

/// Resultado de um scan.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScanReport {
    /// Novos / revisados / relinkados / falhas (mesmo formato do import).
    pub import: ImportReport,
    /// Ativos cujo arquivo nao foi encontrado.
    pub missing: Vec<AssetId>,
    /// Ativos `missing` que voltaram a existir no mesmo caminho.
    pub restored: Vec<AssetId>,
    /// Arquivos que nao mudaram (pulados sem hash).
    pub unchanged: usize,
    /// A raiz estava offline: nada foi alterado.
    pub root_offline: bool,
    pub elapsed_ms: u64,
}

/// Handle do scan em background.
pub type ScanJob = Job<ImportEvent, ScanReport>;

#[derive(Debug, Clone)]
struct Known {
    id: AssetId,
    size: u64,
    mtime: i64,
    state: AssetState,
    hash: ContentHash,
}

impl Library {
    /// Varre uma raiz linked em background. Raiz managed: reindexa os manifestos (sem tocar em arquivos).
    pub fn scan(&self, root: RootId, opts: ScanOptions) -> ScanJob {
        let lib = self.clone();
        Job::spawn("anigo-scan", move |ctx| lib.run_scan(&ctx, root, &opts))
    }

    /// Scan sincrono.
    pub fn scan_blocking(&self, root: RootId, opts: ScanOptions) -> Result<ScanReport> {
        self.scan(root, opts).join()
    }

    fn run_scan(&self, ctx: &JobCtx<ImportEvent>, root: RootId, opts: &ScanOptions) -> Result<ScanReport> {
        let started = Instant::now();
        let info = self.refresh_roots()?.into_iter().find(|r| r.id == root).ok_or_else(|| AssetError::NotFound(format!("raiz {root}")))?;
        let mut rep = ScanReport::default();
        if info.mode == RootMode::Managed {
            self.index_managed_root(root)?;
            ctx.emit(ImportEvent::Finished);
            return Ok(rep);
        }
        if !info.online {
            rep.root_offline = true;
            ctx.emit(ImportEvent::Finished);
            return Ok(rep);
        }

        // 1. o que o indice ja sabe
        let known: HashMap<String, Known> = self.inner.db.read(|c| {
            let mut st = c.prepare("SELECT rel_path, id, primary_size, file_mtime_ns, state, content_hash FROM asset WHERE root_id = ?1 AND storage = 1")?;
            let rows = st.query_map([root.0], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, Vec<u8>>(5)?,
                ))
            })?;
            let mut m = HashMap::new();
            for r in rows {
                let (rel, id, size, mtime, state, hash) = r?;
                if let (Some(id), Some(hash)) = (AssetId::from_bytes(&id), ContentHash::from_slice(&hash)) {
                    m.insert(rel, Known { id, size: size.max(0) as u64, mtime, state: AssetState::from_db(state), hash });
                }
            }
            Ok(m)
        })?;
        let claimed: HashSet<String> = self.inner.db.read(|c| {
            let mut st = c.prepare("SELECT rel_path FROM file_claim WHERE root_id = ?1")?;
            let rows = st.query_map([root.0], |r| r.get::<_, String>(0))?;
            Ok(rows.filter_map(|r| r.ok()).collect())
        })?;

        // 2. enumerar o disco
        let mut en = enumerate(&info.path, true, self.limits(), &ctx.cancel)?;
        let mut seen: HashSet<String> = HashSet::new();
        let mut to_process_models = Vec::new();
        let mut to_process_others = Vec::new();
        let mut unchanged = 0usize;
        let mut restore_touch: Vec<(AssetId, i64)> = Vec::new();
        for (is_model, list) in [(true, std::mem::take(&mut en.models)), (false, std::mem::take(&mut en.others))] {
            for cand in list {
                let Some(rel) = relative_to(&info.path, &cand.path) else { continue };
                seen.insert(rel.clone());
                if claimed.contains(&rel) && !known.contains_key(&rel) {
                    continue; // dependente (textura/buffer) de um ativo linked: nao e ativo solto
                }
                if let Some(k) = known.get(&rel) {
                    if k.state == AssetState::Trashed {
                        continue; // o usuario mandou para a lixeira: nao reimportar
                    }
                    if !opts.rehash_all {
                        if let Ok(meta) = std::fs::metadata(&cand.path) {
                            if meta.len() == k.size && mtime_ns(&meta) == k.mtime && k.mtime != 0 {
                                unchanged += 1;
                                if matches!(k.state, AssetState::Missing | AssetState::Corrupt) {
                                    restore_touch.push((k.id, k.mtime));
                                    rep.restored.push(k.id);
                                }
                                continue;
                            }
                        }
                    }
                }
                if is_model {
                    to_process_models.push(cand);
                } else {
                    to_process_others.push(cand);
                }
            }
        }
        for (id, m) in &restore_touch {
            self.touch_file(id, *m)?;
        }
        rep.unchanged = unchanged;

        // 3. pipeline sobre o que e novo ou mudou
        let mut missing_by_hash: HashMap<ContentHash, Vec<AssetId>> = HashMap::new();
        if opts.detect_moves {
            for (rel, k) in &known {
                if !seen.contains(rel) && k.state != AssetState::Trashed {
                    missing_by_hash.entry(k.hash).or_default().push(k.id);
                }
            }
        }
        let import_opts = ImportOptions { mode: StorageMode::Linked, root: Some(root), duplicates: opts.duplicates, threads: opts.threads, ..Default::default() };
        let params = RunParams { storage: StorageMode::Linked, root_id: root.0, base: info.path.clone(), opts: &import_opts, rev: 1 };
        let lib = self.clone();
        let known_ref = &known;
        let mut consumed: HashSet<AssetId> = HashSet::new();
        let mut decide = |p: &Prepared| -> Result<Decision> {
            let rel = p.rel_in_root.clone().unwrap_or_default();
            if let Some(k) = known_ref.get(&rel) {
                return Ok(if k.hash == p.content_hash { Decision::Unchanged(k.id) } else { Decision::Revision(k.id) });
            }
            if let Some(cands) = missing_by_hash.get(&p.content_hash) {
                if let Some(id) = cands.iter().find(|id| !consumed.contains(id)) {
                    consumed.insert(*id);
                    return Ok(Decision::Relink(*id));
                }
            }
            if opts.duplicates == DuplicatePolicy::Skip {
                if let Some(existing) = lib.find_by_hash(&p.content_hash)? {
                    return Ok(Decision::Duplicate(existing));
                }
            }
            Ok(Decision::New)
        };
        let enumerated = crate::import::Enumerated { models: to_process_models, others: to_process_others, unsupported: en.unsupported };
        run_pipeline(self, ctx, enumerated, &params, &mut decide, &mut rep.import)?;

        // 4. ausentes
        if !rep.import.cancelled && opts.mark_missing {
            let relinked: HashSet<AssetId> = rep.import.relinked.iter().copied().collect();
            let gone: Vec<AssetId> = known.iter().filter(|(rel, k)| !seen.contains(*rel) && !relinked.contains(&k.id) && k.state == AssetState::Active).map(|(_, k)| k.id).collect();
            if !gone.is_empty() {
                self.inner.db.write(|c| {
                    let tx = c.transaction()?;
                    for id in &gone {
                        tx.execute("UPDATE asset SET state = 2 WHERE id = ?1 AND state = 0", [id.as_bytes().as_slice()])?;
                    }
                    tx.commit()?;
                    Ok(())
                })?;
            }
            rep.missing = known.iter().filter(|(rel, k)| !seen.contains(*rel) && !relinked.contains(&k.id) && matches!(k.state, AssetState::Active | AssetState::Missing)).map(|(_, k)| k.id).collect();
        }
        if !rep.import.cancelled {
            self.inner.db.write(|c| {
                c.execute("UPDATE root SET last_scan = ?2 WHERE id = ?1", rusqlite::params![root.0, now_secs()])?;
                Ok(())
            })?;
        }
        rep.elapsed_ms = started.elapsed().as_millis() as u64;
        ctx.emit(ImportEvent::Finished);
        Ok(rep)
    }

    /// Verifica so a EXISTENCIA (sem hash) dos arquivos primarios de uma raiz linked e atualiza `missing`/`active`.
    /// Barato (um `stat` por ativo): use apos remontar um disco ou `relink_root`. Devolve `(ausentes, reencontrados)`.
    pub fn check_presence(&self, root: RootId) -> Result<(Vec<AssetId>, Vec<AssetId>)> {
        let info = self.refresh_roots()?.into_iter().find(|r| r.id == root).ok_or_else(|| AssetError::NotFound(format!("raiz {root}")))?;
        if !info.online {
            return Ok((vec![], vec![]));
        }
        let rows: Vec<(Vec<u8>, String, i64)> = self.inner.db.read(|c| {
            let mut st = c.prepare("SELECT id, rel_path, state FROM asset WHERE root_id = ?1 AND storage = 1 AND state IN (0, 2)")?;
            let v = st.query_map([root.0], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            Ok(v.collect::<std::result::Result<Vec<_>, _>>()?)
        })?;
        let mut gone = Vec::new();
        let mut back = Vec::new();
        for (idb, rel, state) in rows {
            let Some(id) = AssetId::from_bytes(&idb) else { continue };
            let exists = crate::paths::confine(&info.path, &rel).map(|p| p.is_file()).unwrap_or(false);
            match (exists, state) {
                (false, 0) => gone.push(id),
                (true, 2) => back.push(id),
                _ => {}
            }
        }
        self.inner.db.write(|c| {
            let tx = c.transaction()?;
            for id in &gone {
                tx.execute("UPDATE asset SET state = 2 WHERE id = ?1", [id.as_bytes().as_slice()])?;
            }
            for id in &back {
                tx.execute("UPDATE asset SET state = 0 WHERE id = ?1", [id.as_bytes().as_slice()])?;
            }
            tx.commit()?;
            Ok(())
        })?;
        Ok((gone, back))
    }
}

