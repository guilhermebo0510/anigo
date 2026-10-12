//! Pipeline de import em background (T-5.4).
//!
//! ```text
//! enumerar (walkdir) -> [modelos primeiro, depois texturas soltas]
//!   por lote:  PREPARAR em paralelo (rayon): analisar (limites, catch_unwind) -> copiar+hash para staging
//!              (managed) ou hash no lugar (linked)
//!              COMMIT sequencial: decidir (duplicado? nova revisao? relink?) -> mover staging -> escrever
//!              asset.json -> indexar em UMA transacao por lote -> eventos
//! ```
//!
//! * **Cancelar**: `ImportJob::cancel()`; o worker para entre arquivos e DENTRO da copia/hash. O que ja foi
//!   confirmado fica; o que estava em staging e descartado. O relatorio volta com `cancelled = true`.
//! * **Retomar**: reexecutar o mesmo `import` e idempotente - arquivos ja importados sao detectados por
//!   hash BLAKE3 (`DuplicatePolicy::Skip`) e staging orfao de execucoes mortas e limpo.
//! * **Erro por arquivo** nunca aborta o lote (`ImportEvent::Failed`).

pub mod detect;
mod fbx;
mod gltf;
mod obj;
mod raster;
pub(crate) mod commit;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use detect::{format_by_extension, Format};

use crate::error::{AssetError, IoContext, Result};
use crate::job::{guard, CancelToken, Job, JobCtx};
use crate::library::{Library, HOME_ROOT};
use crate::limits::Limits;
use crate::material::ToonMaterial;
use crate::model::*;
use crate::paths::relative_to;
use crate::util::{copy_and_hash, hash_file, mtime_ns};

/// O que fazer com um arquivo cujo conteudo (BLAKE3) ja existe na biblioteca.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DuplicatePolicy {
    /// Nao importa; reporta o ativo existente (padrao).
    #[default]
    Skip,
    /// Importa mesmo assim (cria outro ativo).
    Allow,
}

/// Opcoes de `Library::import`.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// `Managed` copia os arquivos para a biblioteca; `Linked` referencia a pasta (que precisa estar numa raiz linked).
    pub mode: StorageMode,
    /// Raiz de destino (managed: raiz managed; linked: raiz linked que contem o caminho). Padrao: raiz 1 / autodeteccao.
    pub root: Option<RootId>,
    /// Descer em subpastas quando o caminho for uma pasta.
    pub recursive: bool,
    /// Forca o tipo (padrao: inferido do conteudo).
    pub kind: Option<AssetKind>,
    /// Nome (so vale quando o import resulta em UM ativo).
    pub name: Option<String>,
    pub tags: Vec<String>,
    pub description: Option<String>,
    pub author: Option<String>,
    pub license: Option<String>,
    pub source_url: Option<String>,
    pub duplicates: DuplicatePolicy,
    /// Importar como NOVA REVISAO deste ativo (um unico arquivo).
    pub replace: Option<AssetId>,
    /// Adiciona os ativos importados a esta colecao manual.
    pub collection: Option<CollectionId>,
    /// Threads de preparo (0 = automatico, no maximo 8).
    pub threads: usize,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            mode: StorageMode::Managed,
            root: None,
            recursive: true,
            kind: None,
            name: None,
            tags: Vec::new(),
            description: None,
            author: None,
            license: None,
            source_url: None,
            duplicates: DuplicatePolicy::Skip,
            replace: None,
            collection: None,
            threads: 0,
        }
    }
}

/// Eventos de progresso.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportEvent {
    /// Enumeracao concluida: quantos arquivos candidatos.
    Discovered { files: usize },
    Progress { done: usize, total: usize, bytes: u64 },
    Imported { id: AssetId, name: String, path: PathBuf, kind: AssetKind },
    /// Nova revisao de um ativo existente.
    Revised { id: AssetId, revision: u32, path: PathBuf },
    Duplicate { path: PathBuf, existing: AssetId },
    Skipped { path: PathBuf, reason: String },
    Failed { path: PathBuf, error: String },
    /// Um ativo `missing` foi reencontrado em outro lugar (scan).
    Relinked { id: AssetId, path: PathBuf },
    Finished,
}

/// Resultado final de um import/scan.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportReport {
    pub imported: Vec<AssetId>,
    pub revised: Vec<AssetId>,
    pub relinked: Vec<AssetId>,
    pub duplicates: Vec<(PathBuf, AssetId)>,
    pub failed: Vec<(PathBuf, String)>,
    /// Arquivos ignorados (extensao nao suportada, dependentes de modelos...).
    pub skipped: usize,
    pub bytes: u64,
    pub cancelled: bool,
    pub elapsed_ms: u64,
}

/// Handle do import em background.
pub type ImportJob = Job<ImportEvent, ImportReport>;

// ---------------------------------------------------------------------------------------------
// Analise
// ---------------------------------------------------------------------------------------------

/// Resultado da analise de um arquivo (sem tocar na biblioteca).
#[derive(Debug, Clone)]
pub(crate) struct Analysis {
    pub kind: AssetKind,
    pub stats: AssetStats,
    /// Caminhos relativos a pasta do arquivo primario (existentes, confinados).
    pub dependents: Vec<String>,
    pub materials: Vec<ToonMaterial>,
    pub suggested_name: Option<String>,
    pub author: Option<String>,
    pub license: Option<String>,
    pub props: Value,
    pub warnings: Vec<String>,
}

/// Analisa um arquivo: confere magic bytes, aplica limites, extrai estatisticas e materiais.
/// Nunca entra em `panic` (capturado e devolvido como erro).
pub(crate) fn analyze(path: &Path, limits: &Limits) -> Result<(Format, Analysis)> {
    let fmt = format_by_extension(path).ok_or_else(|| AssetError::Unsupported(format!("formato de {}", path.display())))?;
    guard(|| {
        detect::verify_magic(path, fmt)?;
        let a = match fmt {
            Format::Glb | Format::Gltf | Format::Vrm => gltf::analyze_gltf(path, fmt, limits)?,
            Format::Obj => obj::analyze_obj(path, limits)?,
            Format::Fbx => fbx::analyze_fbx(path, limits)?,
            Format::Raster | Format::OpaqueTexture => raster::analyze_texture(path, fmt, limits)?,
            Format::AnigoMat => raster::analyze_material_file(path, limits)?,
        };
        Ok((fmt, a))
    })
}

/// Analise pura, publica: util para a UI mostrar "o que sera importado" antes de confirmar.
#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    pub format: String,
    pub kind: AssetKind,
    pub stats: AssetStats,
    pub dependents: Vec<String>,
    pub material_names: Vec<String>,
    pub suggested_name: Option<String>,
    pub warnings: Vec<String>,
}

/// Analisa `path` sem importar.
pub fn preview(path: &Path, limits: &Limits) -> Result<Preview> {
    let (_, a) = analyze(path, limits)?;
    Ok(Preview {
        format: detect::format_label(path),
        kind: a.kind,
        stats: a.stats,
        dependents: a.dependents,
        material_names: a.materials.iter().map(|m| m.name.clone()).collect(),
        suggested_name: a.suggested_name,
        warnings: a.warnings,
    })
}

// ---------------------------------------------------------------------------------------------
// Enumeracao
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    pub path: PathBuf,
    pub fmt: Format,
}

pub(crate) struct Enumerated {
    pub models: Vec<Candidate>,
    pub others: Vec<Candidate>,
    pub unsupported: usize,
}

fn is_hidden(name: &std::ffi::OsStr) -> bool {
    name.to_string_lossy().starts_with('.')
}

/// Lista candidatos sob `path` (arquivo ou pasta). Modelos antes de texturas soltas, ordem deterministica.
pub(crate) fn enumerate(path: &Path, recursive: bool, limits: &Limits, cancel: &CancelToken) -> Result<Enumerated> {
    let mut out = Enumerated { models: vec![], others: vec![], unsupported: 0 };
    let meta = std::fs::metadata(path).at(path)?;
    if meta.is_file() {
        match format_by_extension(path) {
            Some(fmt) => {
                if fmt.claims_dependents() {
                    out.models.push(Candidate { path: path.to_path_buf(), fmt })
                } else {
                    out.others.push(Candidate { path: path.to_path_buf(), fmt })
                }
            }
            None => return Err(AssetError::Unsupported(format!("formato de {}", path.display()))),
        }
        return Ok(out);
    }
    let max_depth = if recursive { limits.max_scan_depth } else { 1 };
    let mut visited = 0usize;
    let walker = walkdir::WalkDir::new(path).follow_links(false).max_depth(max_depth).sort_by_file_name().into_iter().filter_entry(|e| e.depth() == 0 || !is_hidden(e.file_name()));
    for entry in walker {
        if visited % 256 == 0 {
            cancel.check()?;
        }
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        visited += 1;
        Limits::check("arquivos visitados", visited as u64, limits.max_scan_files as u64)?;
        match format_by_extension(entry.path()) {
            Some(fmt) => {
                let c = Candidate { path: entry.path().to_path_buf(), fmt };
                if fmt.claims_dependents() {
                    out.models.push(c)
                } else {
                    out.others.push(c)
                }
            }
            None => out.unsupported += 1,
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Preparo (paralelo)
// ---------------------------------------------------------------------------------------------

/// Arquivo analisado e (managed) copiado para staging / (linked) hasheado no lugar.
#[derive(Debug)]
pub(crate) struct Prepared {
    pub src: PathBuf,
    pub fmt: Format,
    pub analysis: Analysis,
    pub files: Vec<FileEntry>,
    pub content_hash: ContentHash,
    pub size: u64,
    /// Managed: pasta temporaria que contem `r<rev>/`.
    pub staging: Option<PathBuf>,
    pub rev: u32,
    pub file_mtime_ns: i64,
    /// Linked: caminho do primario relativo a raiz.
    pub rel_in_root: Option<String>,
}

impl Drop for Prepared {
    fn drop(&mut self) {
        if let Some(s) = self.staging.take() {
            let _ = std::fs::remove_dir_all(s);
        }
    }
}

pub(crate) struct PrepareCtx<'a> {
    pub limits: &'a Limits,
    pub cancel: &'a CancelToken,
    pub storage: StorageMode,
    /// Managed: base da raiz destino (para `.staging`). Linked: caminho da raiz linked.
    pub base: &'a Path,
    pub rev: u32,
}

fn file_name_string(p: &Path) -> Result<String> {
    p.file_name().and_then(|n| n.to_str()).map(str::to_string).ok_or_else(|| AssetError::InvalidInput(format!("nome de arquivo invalido: {}", p.display())))
}

pub(crate) fn prepare(cand: &Candidate, ctx: &PrepareCtx<'_>) -> Result<Prepared> {
    ctx.cancel.check()?;
    let (fmt, analysis) = analyze(&cand.path, ctx.limits)?;
    let src_dir = cand.path.parent().ok_or_else(|| AssetError::InvalidInput("arquivo sem pasta pai".into()))?;
    let primary_name = file_name_string(&cand.path)?;
    let meta = std::fs::metadata(&cand.path).at(&cand.path)?;
    Limits::check("arquivo", meta.len(), ctx.limits.max_file_bytes)?;
    Limits::check("arquivos por ativo", 1 + analysis.dependents.len() as u64, ctx.limits.max_files_per_asset as u64)?;

    let mut rels: Vec<String> = vec![primary_name.clone()];
    for d in &analysis.dependents {
        if *d != primary_name && !rels.contains(d) {
            rels.push(d.clone());
        }
    }

    let mut staging: Option<PathBuf> = None;
    let rev_dir_name = crate::library::layout::revision_dir(ctx.rev);
    if ctx.storage == StorageMode::Managed {
        let s = ctx.base.join(".staging").join(uuid::Uuid::now_v7().to_string());
        std::fs::create_dir_all(s.join(&rev_dir_name)).at(&s)?;
        staging = Some(s);
    }

    // `guard` de limpeza: se algo falhar, o Drop de `Prepared` nao existe ainda -> remover o staging aqui.
    let result = (|| -> Result<(Vec<FileEntry>, u64)> {
        let mut files = Vec::with_capacity(rels.len());
        let mut total = 0u64;
        for rel in &rels {
            ctx.cancel.check()?;
            let src = crate::paths::confine(src_dir, rel)?;
            let (hash, size) = match &staging {
                Some(s) => copy_and_hash(&src, &s.join(&rev_dir_name).join(rel), Some(ctx.cancel))?,
                None => hash_file(&src, Some(ctx.cancel))?,
            };
            total += size;
            Limits::check("bytes do ativo", total, ctx.limits.max_asset_bytes)?;
            files.push(FileEntry { path: rel.clone(), size, blake3: hash });
        }
        Ok((files, total))
    })();
    let (files, size) = match result {
        Ok(x) => x,
        Err(e) => {
            if let Some(s) = &staging {
                let _ = std::fs::remove_dir_all(s);
            }
            return Err(e);
        }
    };
    let content_hash = combined_hash(&files);
    let rel_in_root = if ctx.storage == StorageMode::Linked { relative_to(ctx.base, &cand.path) } else { None };
    Ok(Prepared {
        src: cand.path.clone(),
        fmt,
        analysis,
        files,
        content_hash,
        size,
        staging,
        rev: ctx.rev,
        file_mtime_ns: mtime_ns(&meta),
        rel_in_root,
    })
}

// ---------------------------------------------------------------------------------------------
// Decisao + execucao
// ---------------------------------------------------------------------------------------------

/// O que fazer com um arquivo preparado.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Decision {
    New,
    /// Mesma identidade, conteudo novo.
    Revision(AssetId),
    /// Ativo `missing` reencontrado neste caminho.
    Relink(AssetId),
    /// Conteudo ja existente: nao importar.
    Duplicate(AssetId),
    /// Nada a fazer (ex.: so o mtime mudou).
    Unchanged(AssetId),
}

pub(crate) struct RunParams<'a> {
    pub storage: StorageMode,
    pub root_id: i64,
    /// Managed: base da raiz. Linked: caminho da raiz.
    pub base: PathBuf,
    pub opts: &'a ImportOptions,
    pub rev: u32,
}

/// Executa o pipeline sobre `cands` (modelos primeiro). `decide` e chamado no commit, sequencialmente.
pub(crate) fn run_pipeline(
    lib: &Library,
    ctx: &JobCtx<ImportEvent>,
    mut enumerated: Enumerated,
    params: &RunParams<'_>,
    decide: &mut dyn FnMut(&Prepared) -> Result<Decision>,
    report: &mut ImportReport,
) -> Result<()> {
    let total = enumerated.models.len() + enumerated.others.len();
    report.skipped += enumerated.unsupported;
    ctx.emit(ImportEvent::Discovered { files: total });
    let threads = if params.opts.threads == 0 { std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).min(8) } else { params.opts.threads.clamp(1, 32) };
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|i| format!("anigo-import-{i}")).build().map_err(|e| AssetError::Internal(format!("pool de threads: {e}")))?;

    let single = total == 1;
    let mut claimed: HashSet<PathBuf> = HashSet::new();
    let mut seen_hashes: HashMap<ContentHash, AssetId> = HashMap::new();
    let mut done = 0usize;
    let mut pending_index: Vec<commit::IndexItem> = Vec::new();
    let mut pending_collection: Vec<AssetId> = Vec::new();

    let groups = [std::mem::take(&mut enumerated.models), std::mem::take(&mut enumerated.others)];
    'outer: for group in groups {
        let chunk_size = (threads * 4).max(8);
        for chunk in group.chunks(chunk_size) {
            if ctx.cancel.is_cancelled() {
                report.cancelled = true;
                break 'outer;
            }
            // texturas soltas reivindicadas por modelos nao viram ativos proprios
            let mut work: Vec<&Candidate> = Vec::with_capacity(chunk.len());
            for c in chunk {
                if !c.fmt.claims_dependents() && claimed.contains(&c.path) {
                    report.skipped += 1;
                    done += 1;
                    ctx.emit(ImportEvent::Skipped { path: c.path.clone(), reason: "arquivo dependente de um modelo importado".into() });
                    continue;
                }
                work.push(c);
            }
            let pctx = PrepareCtx { limits: lib.limits(), cancel: &ctx.cancel, storage: params.storage, base: &params.base, rev: params.rev };
            let results: Vec<(&Candidate, Result<Prepared>)> = pool.install(|| {
                use rayon::prelude::*;
                work.par_iter().map(|c| (*c, guard(|| prepare(c, &pctx)))).collect()
            });
            for (cand, res) in results {
                done += 1;
                let prepared = match res {
                    Ok(p) => p,
                    Err(AssetError::Cancelled) => {
                        report.cancelled = true;
                        continue;
                    }
                    Err(e) => {
                        report.failed.push((cand.path.clone(), e.to_string()));
                        ctx.emit(ImportEvent::Failed { path: cand.path.clone(), error: e.to_string() });
                        continue;
                    }
                };
                if prepared.fmt.claims_dependents() {
                    if let Some(dir) = prepared.src.parent() {
                        for d in &prepared.analysis.dependents {
                            claimed.insert(dir.join(d));
                        }
                    }
                }
                // duplicata dentro do proprio lote (ainda nao indexada)
                let decision = match seen_hashes.get(&prepared.content_hash) {
                    Some(id) if params.opts.duplicates == DuplicatePolicy::Skip && params.opts.replace.is_none() => Decision::Duplicate(*id),
                    _ => match decide(&prepared) {
                        Ok(d) => d,
                        Err(e) => {
                            report.failed.push((cand.path.clone(), e.to_string()));
                            ctx.emit(ImportEvent::Failed { path: cand.path.clone(), error: e.to_string() });
                            continue;
                        }
                    },
                };
                let path = prepared.src.clone();
                let bytes = prepared.size;
                let outcome = commit::apply(lib, params, &decision, prepared, single, &mut pending_index);
                match outcome {
                    Ok(commit::Applied::Imported { id, name, kind, hash }) => {
                        seen_hashes.insert(hash, id);
                        report.imported.push(id);
                        report.bytes += bytes;
                        pending_collection.push(id);
                        ctx.emit(ImportEvent::Imported { id, name, path, kind });
                    }
                    Ok(commit::Applied::Revised { id, revision }) => {
                        report.revised.push(id);
                        report.bytes += bytes;
                        ctx.emit(ImportEvent::Revised { id, revision, path });
                    }
                    Ok(commit::Applied::Relinked { id }) => {
                        report.relinked.push(id);
                        ctx.emit(ImportEvent::Relinked { id, path });
                    }
                    Ok(commit::Applied::Duplicate { existing }) => {
                        report.duplicates.push((path.clone(), existing));
                        ctx.emit(ImportEvent::Duplicate { path, existing });
                    }
                    Ok(commit::Applied::Unchanged) => {
                        report.skipped += 1;
                    }
                    Err(e) => {
                        report.failed.push((path.clone(), e.to_string()));
                        ctx.emit(ImportEvent::Failed { path, error: e.to_string() });
                    }
                }
                ctx.emit(ImportEvent::Progress { done, total, bytes: report.bytes });
            }
            // confirma o lote no indice (uma transacao)
            lib.flush_index_items(&mut pending_index)?;
            if report.cancelled {
                break 'outer;
            }
        }
    }
    lib.flush_index_items(&mut pending_index)?;
    if let (Some(cid), false) = (params.opts.collection, pending_collection.is_empty()) {
        lib.add_to_collection(cid, &pending_collection)?;
    }
    Ok(())
}

impl Library {
    /// Remove staging orfao (execucoes interrompidas) com mais de `max_age_secs` de idade.
    pub fn cleanup_staging(&self, max_age_secs: u64) -> Result<usize> {
        let mut removed = 0;
        let cfg = self.config_snapshot()?;
        for rc in cfg.roots.iter().filter(|r| r.mode == RootMode::Managed) {
            let base = self.root_abs_path(rc.id)?;
            let staging = base.join(".staging");
            let Ok(rd) = std::fs::read_dir(&staging) else { continue };
            for e in rd.flatten() {
                let old = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .map(|d| d.as_secs() >= max_age_secs)
                    .unwrap_or(true);
                if old && std::fs::remove_dir_all(e.path()).is_ok() {
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }

    /// Inicia um import em background. `path` pode ser um arquivo ou uma pasta.
    pub fn import(&self, path: impl AsRef<Path>, opts: ImportOptions) -> ImportJob {
        let lib = self.clone();
        let path = path.as_ref().to_path_buf();
        Job::spawn("anigo-import", move |ctx| lib.run_import(&ctx, &path, &opts))
    }

    /// Import sincrono (bloqueia). Equivale a `import(..).join()`.
    pub fn import_blocking(&self, path: impl AsRef<Path>, opts: ImportOptions) -> Result<ImportReport> {
        self.import(path, opts).join()
    }

    fn run_import(&self, ctx: &JobCtx<ImportEvent>, path: &Path, opts: &ImportOptions) -> Result<ImportReport> {
        let started = Instant::now();
        let mut report = ImportReport::default();
        let path = path.canonicalize().at(path)?;
        if path.starts_with(self.home()) {
            return Err(AssetError::Conflict("a origem do import nao pode estar dentro da pasta da biblioteca".into()));
        }
        if let Some(rid) = opts.replace {
            self.require_row(&rid)?;
        }
        // destino
        let (root_id, base, rev, storage) = match opts.mode {
            StorageMode::Managed => {
                let rid = opts.root.unwrap_or(HOME_ROOT);
                let info = self.root(rid)?;
                if info.mode != RootMode::Managed {
                    return Err(AssetError::InvalidInput(format!("a raiz {rid} nao e managed")));
                }
                if !info.online {
                    return Err(AssetError::Conflict(format!("a raiz {rid} esta offline")));
                }
                let rev = match opts.replace {
                    Some(id) => self.require_row(&id)?.revision + 1,
                    None => 1,
                };
                let _ = self.cleanup_staging(24 * 3600);
                (rid.0, info.path, rev, StorageMode::Managed)
            }
            StorageMode::Linked => {
                if opts.replace.is_some() {
                    return Err(AssetError::InvalidInput("replace so vale para import managed (arquivos linked mudam no disco; use scan)".into()));
                }
                let roots = self.roots()?;
                let root = match opts.root {
                    Some(r) => roots.into_iter().find(|x| x.id == r),
                    None => roots.into_iter().find(|x| x.mode == RootMode::Linked && path.starts_with(&x.path)),
                }
                .ok_or_else(|| AssetError::InvalidInput("o caminho nao esta dentro de nenhuma raiz linked registrada (use add_root)".into()))?;
                if root.mode != RootMode::Linked || !path.starts_with(&root.path) {
                    return Err(AssetError::InvalidInput("raiz linked invalida para este caminho".into()));
                }
                (root.id.0, root.path, 1, StorageMode::Linked)
            }
        };
        let enumerated = enumerate(&path, opts.recursive, self.limits(), &ctx.cancel)?;
        if opts.replace.is_some() && enumerated.models.len() + enumerated.others.len() != 1 {
            return Err(AssetError::InvalidInput("replace exige exatamente um arquivo".into()));
        }
        if enumerated.models.is_empty() && enumerated.others.is_empty() && enumerated.unsupported == 0 {
            // pasta vazia
        }
        let params = RunParams { storage, root_id, base, opts, rev };
        let lib = self.clone();
        let policy = opts.duplicates;
        let replace = opts.replace;
        let mut decide = move |p: &Prepared| -> Result<Decision> {
            if let Some(id) = replace {
                let row = lib.require_row(&id)?;
                return Ok(if row.content_hash == p.content_hash { Decision::Duplicate(id) } else { Decision::Revision(id) });
            }
            if policy == DuplicatePolicy::Skip {
                if let Some(existing) = lib.find_by_hash(&p.content_hash)? {
                    return Ok(Decision::Duplicate(existing));
                }
            }
            // linked: o mesmo arquivo ja registrado neste caminho?
            if let Some(rel) = &p.rel_in_root {
                if let Some(existing) = lib.find_by_path(root_id, rel)? {
                    return Ok(Decision::Duplicate(existing));
                }
            }
            Ok(Decision::New)
        };
        run_pipeline(self, ctx, enumerated, &params, &mut decide, &mut report)?;
        report.elapsed_ms = started.elapsed().as_millis() as u64;
        ctx.emit(ImportEvent::Finished);
        Ok(report)
    }

    /// Ativo nao-lixeira com este conteudo, se existir.
    pub fn find_by_hash(&self, hash: &ContentHash) -> Result<Option<AssetId>> {
        self.inner.db.read(|c| {
            let v: Option<Vec<u8>> = rusqlite::OptionalExtension::optional(
                c.prepare_cached("SELECT id FROM asset WHERE content_hash = ?1 AND state != 1 ORDER BY seq LIMIT 1")?.query_row([hash.0.as_slice()], |r| r.get(0)),
            )?;
            Ok(v.and_then(|b| AssetId::from_bytes(&b)))
        })
    }

    /// Ativo linked registrado em `(root, rel_path)`, se existir.
    pub fn find_by_path(&self, root_id: i64, rel_path: &str) -> Result<Option<AssetId>> {
        self.inner.db.read(|c| {
            let v: Option<Vec<u8>> = rusqlite::OptionalExtension::optional(
                c.prepare_cached("SELECT id FROM asset WHERE root_id = ?1 AND storage = 1 AND rel_path = ?2 LIMIT 1")?.query_row(rusqlite::params![root_id, rel_path], |r| r.get(0)),
            )?;
            Ok(v.and_then(|b| AssetId::from_bytes(&b)))
        })
    }
}

