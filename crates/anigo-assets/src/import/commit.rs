//! Fase de commit do pipeline: move staging para o lugar definitivo, escreve `asset.json` e indexa.

use std::path::Path;

use super::{Decision, Prepared, RunParams};
use crate::error::{AssetError, IoContext, Result};
use crate::library::{layout, read_manifest, write_manifest, Library};
use crate::material::ToonMaterial;
use crate::model::*;
use crate::store::{self, normalize_tags, IndexEntry, TagCache};
use crate::util::{atomic_write, now_secs, sanitize_file_name};

/// Linha pendente de indexacao (manifesto ja gravado em disco).
pub(crate) struct IndexItem {
    pub manifest: AssetManifest,
    pub root_id: i64,
    pub manifest_dir: String,
    pub state: AssetState,
    pub file_mtime_ns: i64,
}

pub(crate) enum Applied {
    Imported { id: AssetId, name: String, kind: AssetKind, hash: ContentHash },
    Revised { id: AssetId, revision: u32 },
    Relinked { id: AssetId },
    Duplicate { existing: AssetId },
    Unchanged,
}

/// Subpasta de materiais canonicos dentro da pasta do ativo.
pub const MATERIALS_DIR: &str = "materials";

fn default_name(p: &Prepared) -> String {
    let file = p.src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let stem = if let Some(s) = file.to_lowercase().strip_suffix(crate::material::MATERIAL_SUFFIX) {
        file[..s.len()].to_string()
    } else if let Some(s) = file.to_lowercase().strip_suffix(anigo_core::pose::POSE_ASSET_SUFFIX) {
        file[..s.len()].to_string()
    } else {
        p.src.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or(file)
    };
    if stem.trim().is_empty() {
        "Sem nome".into()
    } else {
        stem
    }
}

fn clip(s: &str, max: usize) -> String {
    s.chars().filter(|c| *c != '\0').take(max).collect::<String>().trim().to_string()
}

/// Escreve os materiais canonicos em `dir/materials/` (substituindo o conteudo anterior). Devolve os nomes de arquivo.
pub(crate) fn write_materials(dir: &Path, mats: &[ToonMaterial]) -> Result<Vec<String>> {
    let mdir = dir.join(MATERIALS_DIR);
    if mdir.exists() {
        let _ = std::fs::remove_dir_all(&mdir);
    }
    if mats.is_empty() {
        return Ok(vec![]);
    }
    std::fs::create_dir_all(&mdir).at(&mdir)?;
    let mut names = Vec::new();
    for (i, m) in mats.iter().enumerate() {
        let file = format!("{:03}_{}{}", i, sanitize_file_name(&m.name), crate::material::MATERIAL_SUFFIX);
        atomic_write(&mdir.join(&file), &m.to_json()?)?;
        names.push(file);
    }
    Ok(names)
}

fn base_manifest(p: &Prepared, params: &RunParams<'_>, id: AssetId, single: bool, name_override: Option<String>) -> AssetManifest {
    let o = params.opts;
    let a = &p.analysis;
    let now = now_secs();
    let name = name_override
        .or_else(|| if single { o.name.clone() } else { None })
        .or_else(|| a.suggested_name.clone())
        .map(|n| clip(&n, 256))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| default_name(p));
    let primary = p.files.first().map(|f| f.path.clone()).unwrap_or_default();
    AssetManifest {
        schema: MANIFEST_SCHEMA,
        asset_id: id,
        name,
        kind: o.kind.unwrap_or(a.kind),
        description: o.description.as_deref().map(|s| clip(s, 8192)).unwrap_or_default(),
        author: o.author.clone().or_else(|| a.author.clone()).map(|s| clip(&s, 256)).unwrap_or_default(),
        license: o.license.clone().or_else(|| a.license.clone()).map(|s| clip(&s, 256)).unwrap_or_default(),
        source_url: o.source_url.as_deref().map(|s| clip(s, 2048)).unwrap_or_default(),
        format: super::detect::format_label(&p.src),
        storage: params.storage,
        primary,
        link: None,
        current_revision: p.rev,
        revisions: vec![RevisionInfo { rev: p.rev, content_hash: p.content_hash, size: p.size, created_at: now, files: p.files.clone(), note: String::new() }],
        tags: normalize_tags(o.tags.iter()),
        rating: 0,
        favorite: false,
        stats: a.stats.clone(),
        dependencies: vec![],
        materials: vec![],
        importer_ver: IMPORTER_VERSION,
        created_at: now,
        imported_at: now,
        modified_at: now,
        trashed_at: None,
        props: a.props.clone(),
        warnings: a.warnings.clone(),
    }
}

pub(crate) fn apply(lib: &Library, params: &RunParams<'_>, decision: &Decision, mut p: Prepared, single: bool, pending: &mut Vec<IndexItem>) -> Result<Applied> {
    match decision {
        Decision::Duplicate(existing) => Ok(Applied::Duplicate { existing: *existing }),
        Decision::Unchanged(id) => {
            lib.touch_file(id, p.file_mtime_ns)?;
            Ok(Applied::Unchanged)
        }
        Decision::Relink(id) => apply_relink(lib, id, &p).map(|_| Applied::Relinked { id: *id }),
        Decision::Revision(id) => apply_revision(lib, params, id, &mut p).map(|rev| Applied::Revised { id: *id, revision: rev }),
        Decision::New => apply_new(lib, params, &mut p, single, pending),
    }
}

fn apply_new(lib: &Library, params: &RunParams<'_>, p: &mut Prepared, single: bool, pending: &mut Vec<IndexItem>) -> Result<Applied> {
    let id = AssetId::new();
    let mut manifest = base_manifest(p, params, id, single, None);
    let (dir_rel, dir_abs) = match params.storage {
        StorageMode::Managed => {
            let rel = layout::managed_dir_rel(&id);
            let abs = params.base.join(&rel);
            (rel, abs)
        }
        StorageMode::Linked => {
            let rel = layout::linked_dir_rel(&id);
            let abs = lib.home().join(&rel);
            (rel, abs)
        }
    };
    if params.storage == StorageMode::Linked {
        let root_uuid = lib.root_uuid(params.root_id)?;
        manifest.link = Some(LinkInfo { root_uuid, rel_path: p.rel_in_root.clone().ok_or_else(|| AssetError::Internal("linked sem rel_in_root".into()))? });
    }
    let result = (|| -> Result<()> {
        std::fs::create_dir_all(&dir_abs).at(&dir_abs)?;
        if params.storage == StorageMode::Managed {
            let staging = p.staging.as_ref().ok_or_else(|| AssetError::Internal("managed sem staging".into()))?;
            let rev_name = layout::revision_dir(p.rev);
            std::fs::rename(staging.join(&rev_name), dir_abs.join(&rev_name)).at(&dir_abs)?;
        }
        manifest.materials = write_materials(&dir_abs, &p.analysis.materials)?;
        manifest.validate()?;
        write_manifest(&dir_abs, &manifest)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&dir_abs);
        return Err(e);
    }
    let name = manifest.name.clone();
    let kind = manifest.kind;
    let hash = p.content_hash;
    pending.push(IndexItem { manifest, root_id: params.root_id, manifest_dir: dir_rel, state: AssetState::Active, file_mtime_ns: p.file_mtime_ns });
    Ok(Applied::Imported { id, name, kind, hash })
}

fn apply_revision(lib: &Library, params: &RunParams<'_>, id: &AssetId, p: &mut Prepared) -> Result<u32> {
    let loc = lib.loc(id)?;
    let dir = lib.loc_dir(&loc)?;
    let mut m = read_manifest(&dir)?;
    let new_rev = m.revisions.iter().map(|r| r.rev).max().unwrap_or(0) + 1;
    if m.storage == StorageMode::Managed {
        if p.rev != new_rev {
            return Err(AssetError::Conflict(format!("revisao esperada {new_rev}, preparada {}", p.rev)));
        }
        let staging = p.staging.as_ref().ok_or_else(|| AssetError::Internal("managed sem staging".into()))?;
        let rev_name = layout::revision_dir(p.rev);
        std::fs::rename(staging.join(&rev_name), dir.join(&rev_name)).at(&dir)?;
    }
    let _g = lib.inner.mutate_lock.lock().map_err(|_| AssetError::Internal("lock de mutacao envenenado".into()))?;
    let now = now_secs();
    m.revisions.push(RevisionInfo { rev: new_rev, content_hash: p.content_hash, size: p.size, created_at: now, files: p.files.clone(), note: String::new() });
    m.current_revision = new_rev;
    m.primary = p.files.first().map(|f| f.path.clone()).unwrap_or_default();
    m.format = super::detect::format_label(&p.src);
    m.stats = p.analysis.stats.clone();
    m.warnings = p.analysis.warnings.clone();
    m.props = p.analysis.props.clone();
    m.modified_at = now;
    if m.storage == StorageMode::Linked {
        if let (Some(link), Some(rel)) = (m.link.as_mut(), p.rel_in_root.clone()) {
            link.rel_path = rel;
        }
    }
    m.materials = write_materials(&dir, &p.analysis.materials)?;
    m.validate()?;
    write_manifest(&dir, &m)?;
    drop(_g);
    let mut items = vec![IndexItem { manifest: m, root_id: loc.root_id, manifest_dir: loc.manifest_dir.clone(), state: AssetState::Active, file_mtime_ns: p.file_mtime_ns }];
    let _ = params;
    lib.flush_index_items(&mut items)?;
    Ok(new_rev)
}

fn apply_relink(lib: &Library, id: &AssetId, p: &Prepared) -> Result<()> {
    let rel = p.rel_in_root.clone().ok_or_else(|| AssetError::Internal("relink sem caminho".into()))?;
    let (loc, m) = lib.mutate_manifest(id, |m| {
        if m.storage != StorageMode::Linked {
            return Err(AssetError::InvalidInput("relink so vale para ativos linked".into()));
        }
        if let Some(l) = m.link.as_mut() {
            l.rel_path = rel.clone();
        }
        m.primary = p.files.first().map(|f| f.path.clone()).unwrap_or_default();
        Ok(())
    })?;
    let mut items = vec![IndexItem { manifest: m, root_id: loc.root_id, manifest_dir: loc.manifest_dir.clone(), state: AssetState::Active, file_mtime_ns: p.file_mtime_ns }];
    lib.flush_index_items(&mut items)
}

impl Library {
    /// Indexa (upsert) manifestos ja gravados, numa unica transacao. Esvazia `items`.
    pub(crate) fn flush_index_items(&self, items: &mut Vec<IndexItem>) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let batch = std::mem::take(items);
        self.inner.db.write(|c| {
            let tx = c.transaction()?;
            let mut cache = TagCache::default();
            for it in &batch {
                store::upsert_asset(
                    &tx,
                    &mut cache,
                    &IndexEntry { manifest: &it.manifest, root_id: it.root_id, manifest_dir: it.manifest_dir.clone(), state: it.state, file_mtime_ns: it.file_mtime_ns, trashed_at: it.manifest.trashed_at },
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// Atualiza mtime observado e reativa um ativo que estava `missing`/`corrupt`.
    pub(crate) fn touch_file(&self, id: &AssetId, mtime_ns: i64) -> Result<()> {
        self.inner.db.write(|c| {
            c.execute("UPDATE asset SET file_mtime_ns = ?2, state = CASE WHEN state IN (2,3) THEN 0 ELSE state END WHERE id = ?1", rusqlite::params![id.as_bytes().as_slice(), mtime_ns])?;
            Ok(())
        })
    }

    /// Materiais canonicos de um ativo: `(arquivo, nome)`.
    pub fn asset_materials(&self, id: &AssetId) -> Result<Vec<(String, String)>> {
        let dir = self.asset_dir(id)?;
        let m = read_manifest(&dir)?;
        let mut out = Vec::new();
        for f in &m.materials {
            let name = self.load_material(id, f).map(|t| t.name).unwrap_or_else(|_| f.clone());
            out.push((f.clone(), name));
        }
        Ok(out)
    }

    /// Carrega um material canonico do ativo (`file` e o nome do arquivo em `materials/`, sem separadores).
    pub fn load_material(&self, id: &AssetId, file: &str) -> Result<ToonMaterial> {
        if file.contains('/') || file.contains('\\') || file.contains("..") || !file.ends_with(crate::material::MATERIAL_SUFFIX) {
            return Err(AssetError::UnsafePath(format!("nome de material invalido: {file:?}")));
        }
        let dir = self.asset_dir(id)?.join(MATERIALS_DIR);
        let p = dir.join(file);
        let meta = std::fs::metadata(&p).at(&p)?;
        if meta.len() > crate::material::MATERIAL_MAX_BYTES as u64 {
            return Err(AssetError::LimitExceeded { what: "anigomat.json", limit: crate::material::MATERIAL_MAX_BYTES as u64, actual: meta.len() });
        }
        ToonMaterial::from_json(&std::fs::read(&p).at(&p)?)
    }
}
