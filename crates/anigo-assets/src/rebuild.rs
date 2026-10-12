//! Reconstrucao do indice a partir dos `asset.json` (a verdade). Tambem usada na abertura quando o `index.db`
//! some, e ao registrar uma raiz managed que ja tem conteudo.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rayon::prelude::*;

use crate::error::{AssetError, Result};
use crate::job::Job;
use crate::library::{read_manifest, Library};
use crate::model::*;
use crate::store::{self, IndexEntry, TagCache};

/// Pasta de manifesto encontrada no disco.
#[derive(Debug, Clone)]
pub(crate) struct FoundDir {
    /// Relativa a `base`.
    pub rel: String,
    pub trashed: bool,
    /// Base onde a pasta foi achada (raiz managed ou home).
    pub base_root: i64,
    pub linked_area: bool,
}

/// Resultado de `rebuild_index`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RebuildReport {
    pub assets: u64,
    pub trashed: u64,
    pub linked: u64,
    /// Manifestos ilegiveis/invalidos (o ativo NAO entra no indice; a pasta fica intacta).
    pub bad_manifests: Vec<(PathBuf, String)>,
    /// Manifestos linked cuja raiz nao esta mais registrada.
    pub orphan_linked: u64,
    pub collections_restored: u64,
    pub elapsed_ms: u64,
}

fn list_dirs(p: &Path) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(p) else { return vec![] };
    rd.flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| (e.file_name().to_string_lossy().to_string(), e.path()))
        .collect()
}

/// Enumera pastas de ativos sob `base`: `<area>/ab/cd/<id>` e `trash/<data>/<id>`.
pub(crate) fn scan_layout(base: &Path, base_root: i64, include_linked: bool) -> Vec<FoundDir> {
    let mut out = Vec::new();
    let mut areas: Vec<(&str, bool)> = vec![("assets", false)];
    if include_linked {
        areas.push(("linked", true));
    }
    for (area, linked_area) in areas {
        for (a, pa) in list_dirs(&base.join(area)) {
            for (b, pb) in list_dirs(&pa) {
                for (id, pid) in list_dirs(&pb) {
                    if pid.join(MANIFEST_FILE).is_file() {
                        out.push(FoundDir { rel: format!("{area}/{a}/{b}/{id}"), trashed: false, base_root, linked_area });
                    }
                }
            }
        }
    }
    for (date, pd) in list_dirs(&base.join("trash")) {
        for (id, pid) in list_dirs(&pd) {
            if pid.join(MANIFEST_FILE).is_file() {
                out.push(FoundDir { rel: format!("trash/{date}/{id}"), trashed: true, base_root, linked_area: false });
            }
        }
    }
    out
}

struct Loaded {
    found: FoundDir,
    base: PathBuf,
    manifest: std::result::Result<AssetManifest, String>,
}

impl Library {
    /// Todas as areas de manifesto: cada raiz managed + (na raiz 1) a area `linked/`.
    fn all_found_dirs(&self, only_root: Option<i64>) -> Result<Vec<(FoundDir, PathBuf)>> {
        let cfg = self.config_snapshot()?;
        let mut out = Vec::new();
        for rc in cfg.roots.iter().filter(|r| r.mode == RootMode::Managed) {
            if only_root.map(|o| o != rc.id).unwrap_or(false) {
                continue;
            }
            let base = self.root_abs_path(rc.id)?;
            if !base.is_dir() {
                continue;
            }
            for f in scan_layout(&base, rc.id, rc.id == 1) {
                out.push((f, base.clone()));
            }
        }
        Ok(out)
    }

    fn load_all(&self, dirs: Vec<(FoundDir, PathBuf)>) -> Vec<Loaded> {
        dirs.into_par_iter()
            .map(|(found, base)| {
                let manifest = read_manifest(&base.join(&found.rel)).map_err(|e| e.to_string());
                Loaded { found, base, manifest }
            })
            .collect()
    }

    /// Resolve em qual raiz do indice o manifesto vive: managed -> raiz onde foi achado; linked -> raiz do link.
    fn index_root_of(&self, l: &Loaded, m: &AssetManifest) -> Option<i64> {
        match m.storage {
            StorageMode::Managed => Some(l.found.base_root),
            StorageMode::Linked => m.link.as_ref().and_then(|lk| self.root_id_by_uuid(&lk.root_uuid)),
        }
    }

    /// Reconstroi TODO o indice a partir dos manifestos em disco (`assets/`, `linked/`, `trash/` de cada raiz
    /// managed). Preserva: raizes, colecoes (restauradas de `collections.json` se a tabela estiver vazia),
    /// cores de tag e os estados `missing`/`corrupt`/mtime observados. Perde: nada que esteja nos manifestos.
    ///
    /// Bloqueante (o custo e linear no numero de ativos; ver `docs/native/BIBLIOTECA_BENCH.md`); use
    /// [`Library::rebuild_index_job`] para rodar em background.
    pub fn rebuild_index(&self) -> Result<RebuildReport> {
        let started = Instant::now();
        let dirs = self.all_found_dirs(None)?;
        let loaded = self.load_all(dirs);
        let mut report = RebuildReport::default();

        // estado anterior que nao esta nos manifestos
        let prev: HashMap<Vec<u8>, (i64, i64)> = self.inner.db.read(|c| {
            let mut st = c.prepare("SELECT id, file_mtime_ns, state FROM asset")?;
            let rows = st.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, (r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))))?;
            Ok(rows.filter_map(|r| r.ok()).collect())
        })?;

        self.inner.db.write(|c| {
            let tx = c.transaction()?;
            tx.execute_batch("DELETE FROM asset_fts; DELETE FROM asset_tag; DELETE FROM dep; DELETE FROM file_claim; DELETE FROM asset;")?;
            let mut cache = TagCache::default();
            for l in &loaded {
                let m = match &l.manifest {
                    Ok(m) => m,
                    Err(e) => {
                        report.bad_manifests.push((l.base.join(&l.found.rel), e.clone()));
                        continue;
                    }
                };
                let Some(root_id) = self.index_root_of(l, m) else {
                    report.orphan_linked += 1;
                    continue;
                };
                let (mtime, prev_state) = prev.get(m.asset_id.as_bytes().as_slice()).copied().unwrap_or((0, 0));
                let state = if l.found.trashed {
                    AssetState::Trashed
                } else if matches!(AssetState::from_db(prev_state), AssetState::Missing | AssetState::Corrupt) {
                    AssetState::from_db(prev_state)
                } else {
                    AssetState::Active
                };
                store::upsert_asset(
                    &tx,
                    &mut cache,
                    &IndexEntry { manifest: m, root_id, manifest_dir: l.found.rel.clone(), state, file_mtime_ns: mtime, trashed_at: m.trashed_at },
                )?;
                report.assets += 1;
                if l.found.trashed {
                    report.trashed += 1;
                }
                if m.storage == StorageMode::Linked {
                    report.linked += 1;
                }
            }
            tx.commit()?;
            Ok(())
        })?;
        report.collections_restored = self.restore_collections_snapshot()?;
        report.elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(report)
    }

    /// `rebuild_index` em thread propria.
    pub fn rebuild_index_job(&self) -> Job<(), RebuildReport> {
        let lib = self.clone();
        Job::spawn("anigo-rebuild", move |_ctx| lib.rebuild_index())
    }

    /// Indexa (sem apagar nada) os manifestos de UMA raiz managed. Usado por `add_root`.
    pub(crate) fn index_managed_root(&self, root: RootId) -> Result<u64> {
        let dirs = self.all_found_dirs(Some(root.0))?;
        let loaded = self.load_all(dirs);
        let mut n = 0u64;
        self.inner.db.write(|c| {
            let tx = c.transaction()?;
            let mut cache = TagCache::default();
            for l in &loaded {
                let Ok(m) = &l.manifest else { continue };
                if m.storage != StorageMode::Managed {
                    continue;
                }
                let state = if l.found.trashed { AssetState::Trashed } else { AssetState::Active };
                store::upsert_asset(&tx, &mut cache, &IndexEntry { manifest: m, root_id: l.found.base_root, manifest_dir: l.found.rel.clone(), state, file_mtime_ns: 0, trashed_at: m.trashed_at })?;
                n += 1;
            }
            tx.commit()?;
            Ok(())
        })?;
        Ok(n)
    }

    /// Manifestos em disco que o indice nao conhece (e linhas do indice sem manifesto): `(orfaos_em_disco, linhas_sem_manifesto)`.
    pub(crate) fn index_vs_disk(&self) -> Result<(Vec<(FoundDir, PathBuf, AssetId)>, Vec<AssetId>)> {
        let dirs = self.all_found_dirs(None)?;
        let known: HashMap<Vec<u8>, String> = self.inner.db.read(|c| {
            let mut st = c.prepare("SELECT id, manifest_dir FROM asset")?;
            let rows = st.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?)))?;
            Ok(rows.filter_map(|r| r.ok()).collect())
        })?;
        let mut on_disk: HashMap<Vec<u8>, ()> = HashMap::new();
        let mut orphans = Vec::new();
        for (f, base) in dirs {
            // o nome da pasta final e o uuid
            let Some(name) = f.rel.rsplit('/').next() else { continue };
            let Ok(id) = name.parse::<AssetId>() else { continue };
            on_disk.insert(id.as_bytes().to_vec(), ());
            if !known.contains_key(id.as_bytes().as_slice()) {
                orphans.push((f, base, id));
            }
        }
        let missing_dirs: Vec<AssetId> = known.keys().filter(|k| !on_disk.contains_key(*k)).filter_map(|k| AssetId::from_bytes(k)).collect();
        Ok((orphans, missing_dirs))
    }

    /// Indexa um manifesto orfao encontrado em disco (usado por `verify --fix`).
    pub(crate) fn index_orphan(&self, found: &FoundDir, base: &Path) -> Result<AssetId> {
        let m = read_manifest(&base.join(&found.rel))?;
        let loaded = Loaded { found: found.clone(), base: base.to_path_buf(), manifest: Ok(m.clone()) };
        let root_id = self.index_root_of(&loaded, &m).ok_or_else(|| AssetError::NotFound("raiz do ativo linked nao registrada".into()))?;
        let state = if found.trashed { AssetState::Trashed } else { AssetState::Active };
        self.inner.db.write(|c| {
            let tx = c.transaction()?;
            let mut cache = TagCache::default();
            store::upsert_asset(&tx, &mut cache, &IndexEntry { manifest: &m, root_id, manifest_dir: found.rel.clone(), state, file_mtime_ns: 0, trashed_at: m.trashed_at })?;
            tx.commit()?;
            Ok(())
        })?;
        Ok(m.asset_id)
    }
}
