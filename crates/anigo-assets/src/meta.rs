//! Leitura de detalhes e edicao de metadados (nome, tags, favorito, rating, dependencias).
//!
//! Regra de ouro: `asset.json` e a verdade. Toda edicao reescreve o manifesto (atomicamente) e so depois
//! atualiza o indice; se o processo morrer no meio, `rebuild_index`/`verify` reconciliam.

use std::path::PathBuf;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::{AssetError, Result};
use crate::library::{read_manifest, write_manifest, Library};
use crate::model::*;
use crate::store::{self, normalize_tag, normalize_tags, IndexEntry, TagCache};
use crate::util::now_secs;

/// Onde um ativo vive no indice.
#[derive(Debug, Clone)]
pub(crate) struct Loc {
    pub seq: i64,
    pub root_id: i64,
    pub storage: StorageMode,
    pub manifest_dir: String,
    pub state: AssetState,
    pub file_mtime_ns: i64,
    pub trashed_at: Option<i64>,
}

/// Arquivos resolvidos de um ativo (caminhos absolutos).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedFiles {
    /// Pasta que contem `asset.json`.
    pub asset_dir: PathBuf,
    /// Arquivo primario (o que o renderer/miniatura abre).
    pub primary: PathBuf,
    /// Todos os arquivos da revisao atual.
    pub files: Vec<PathBuf>,
    /// `false` se a raiz esta offline ou o primario nao existe.
    pub exists: bool,
}

/// Campos editaveis em lote. `None` = nao alterar.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InfoPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub author: Option<String>,
    pub license: Option<String>,
    pub source_url: Option<String>,
    pub kind: Option<AssetKind>,
}

/// Tag com contagem de usos.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagInfo {
    pub path: String,
    pub color: String,
    /// Ativos ativos com esta tag exata.
    pub count: u64,
}

fn check_len(what: &str, s: &str, max: usize) -> Result<()> {
    if s.chars().count() > max || s.chars().any(|c| c == '\0') {
        return Err(AssetError::InvalidInput(format!("{what} invalido (maximo {max} caracteres, sem NUL)")));
    }
    Ok(())
}

impl Library {
    pub(crate) fn loc(&self, id: &AssetId) -> Result<Loc> {
        self.inner
            .db
            .read(|c| {
                Ok(c.prepare_cached("SELECT seq, root_id, storage, manifest_dir, state, file_mtime_ns, trashed_at FROM asset WHERE id = ?1")?
                    .query_row([id.as_bytes().as_slice()], |r| {
                        Ok(Loc {
                            seq: r.get(0)?,
                            root_id: r.get(1)?,
                            storage: StorageMode::from_db(r.get(2)?),
                            manifest_dir: r.get(3)?,
                            state: AssetState::from_db(r.get(4)?),
                            file_mtime_ns: r.get(5)?,
                            trashed_at: r.get(6)?,
                        })
                    })
                    .optional()?)
            })?
            .ok_or_else(|| AssetError::NotFound(format!("ativo {id}")))
    }

    pub(crate) fn loc_dir(&self, loc: &Loc) -> Result<PathBuf> {
        Ok(self.manifest_base(loc.storage, loc.root_id)?.join(&loc.manifest_dir))
    }

    /// Pasta do ativo (onde fica o `asset.json`).
    pub fn asset_dir(&self, id: &AssetId) -> Result<PathBuf> {
        let loc = self.loc(id)?;
        self.loc_dir(&loc)
    }

    /// Linha do indice, se existir.
    pub fn row(&self, id: &AssetId) -> Result<Option<AssetRow>> {
        self.inner.db.read(|c| store::get_row(c, id))
    }

    /// Linha do indice ou `NotFound`.
    pub fn require_row(&self, id: &AssetId) -> Result<AssetRow> {
        self.row(id)?.ok_or_else(|| AssetError::NotFound(format!("ativo {id}")))
    }

    /// Le o manifesto do disco.
    pub fn manifest(&self, id: &AssetId) -> Result<AssetManifest> {
        let dir = self.asset_dir(id)?;
        read_manifest(&dir)
    }

    /// Detalhes completos (linha + manifesto + dependencias).
    pub fn get(&self, id: &AssetId) -> Result<AssetDetail> {
        let row = self.require_row(id)?;
        let manifest = self.manifest(id)?;
        let dependencies = self.dependencies(id)?;
        let used_by = self.used_by(id)?;
        Ok(AssetDetail { row, manifest, dependencies, used_by })
    }

    /// Tags de um ativo (do indice).
    pub fn tags_of(&self, id: &AssetId) -> Result<Vec<String>> {
        self.inner.db.read(|c| {
            let seq = store::require_seq(c, id)?;
            store::tags_of(c, seq)
        })
    }

    /// Caminhos absolutos dos arquivos da revisao atual.
    pub fn resolve_files(&self, id: &AssetId) -> Result<ResolvedFiles> {
        let loc = self.loc(id)?;
        let asset_dir = self.loc_dir(&loc)?;
        let m = read_manifest(&asset_dir)?;
        let rev = m.current().ok_or_else(|| AssetError::Corrupt("sem revisao atual".into()))?;
        let (base, primary) = match m.storage {
            StorageMode::Managed => {
                let base = asset_dir.join(crate::library::layout::revision_dir(m.current_revision));
                let primary = base.join(&m.primary);
                (base, primary)
            }
            StorageMode::Linked => {
                let link = m.link.as_ref().ok_or_else(|| AssetError::Corrupt("linked sem link".into()))?;
                let root = self.root_abs_path(self.root_id_by_uuid(&link.root_uuid).ok_or_else(|| AssetError::NotFound(format!("raiz {}", link.root_uuid)))?)?;
                let primary = root.join(&link.rel_path);
                let base = primary.parent().map(|p| p.to_path_buf()).unwrap_or(root);
                (base, primary)
            }
        };
        let files: Vec<PathBuf> = rev.files.iter().map(|f| base.join(&f.path)).collect();
        let exists = primary.is_file();
        Ok(ResolvedFiles { asset_dir, primary, files, exists })
    }

    /// Le-modifica-grava o manifesto sob o lock de mutacao. Devolve `(loc, manifest)` ja gravado.
    pub(crate) fn mutate_manifest(&self, id: &AssetId, f: impl FnOnce(&mut AssetManifest) -> Result<()>) -> Result<(Loc, AssetManifest)> {
        let _g = self.inner.mutate_lock.lock().map_err(|_| AssetError::Internal("lock de mutacao envenenado".into()))?;
        let loc = self.loc(id)?;
        let dir = self.loc_dir(&loc)?;
        let mut m = read_manifest(&dir)?;
        f(&mut m)?;
        m.modified_at = now_secs();
        m.validate()?;
        write_manifest(&dir, &m)?;
        Ok((loc, m))
    }

    /// Reindexa completamente um ativo a partir do manifesto (preserva estado/localizacao).
    pub(crate) fn reindex(&self, loc: &Loc, m: &AssetManifest) -> Result<()> {
        self.inner.db.write(|c| {
            let tx = c.transaction()?;
            let mut cache = TagCache::default();
            store::upsert_asset(
                &tx,
                &mut cache,
                &IndexEntry { manifest: m, root_id: loc.root_id, manifest_dir: loc.manifest_dir.clone(), state: loc.state, file_mtime_ns: loc.file_mtime_ns, trashed_at: loc.trashed_at },
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Marca/desmarca favorito.
    pub fn set_favorite(&self, id: &AssetId, favorite: bool) -> Result<()> {
        let (_, m) = self.mutate_manifest(id, |m| {
            m.favorite = favorite;
            Ok(())
        })?;
        self.inner.db.write(|c| {
            c.execute("UPDATE asset SET favorite = ?2, modified_at = ?3 WHERE id = ?1", params![id.as_bytes().as_slice(), favorite as i64, m.modified_at])?;
            Ok(())
        })
    }

    /// Define a nota (0 a 5).
    pub fn set_rating(&self, id: &AssetId, rating: u8) -> Result<()> {
        if rating > 5 {
            return Err(AssetError::InvalidInput(format!("rating {rating} fora de 0..=5")));
        }
        let (_, m) = self.mutate_manifest(id, |m| {
            m.rating = rating;
            Ok(())
        })?;
        self.inner.db.write(|c| {
            c.execute("UPDATE asset SET rating = ?2, modified_at = ?3 WHERE id = ?1", params![id.as_bytes().as_slice(), rating as i64, m.modified_at])?;
            Ok(())
        })
    }

    /// Edita nome/descricao/autor/licenca/origem/tipo.
    pub fn update_info(&self, id: &AssetId, patch: &InfoPatch) -> Result<()> {
        if let Some(n) = &patch.name {
            if n.trim().is_empty() {
                return Err(AssetError::InvalidInput("nome vazio".into()));
            }
            check_len("nome", n, 256)?;
        }
        for (what, v, max) in [("descricao", &patch.description, 8192), ("autor", &patch.author, 256), ("licenca", &patch.license, 256), ("origem", &patch.source_url, 2048)] {
            if let Some(v) = v {
                check_len(what, v, max)?;
            }
        }
        let (loc, m) = self.mutate_manifest(id, |m| {
            if let Some(n) = &patch.name {
                m.name = n.trim().to_string();
            }
            if let Some(v) = &patch.description {
                m.description = v.clone();
            }
            if let Some(v) = &patch.author {
                m.author = v.clone();
            }
            if let Some(v) = &patch.license {
                m.license = v.clone();
            }
            if let Some(v) = &patch.source_url {
                m.source_url = v.clone();
            }
            if let Some(k) = patch.kind {
                m.kind = k;
            }
            Ok(())
        })?;
        self.reindex(&loc, &m)
    }

    /// Renomeia (so metadado: pastas e ids nao mudam).
    pub fn rename(&self, id: &AssetId, name: &str) -> Result<()> {
        self.update_info(id, &InfoPatch { name: Some(name.to_string()), ..Default::default() })
    }

    /// Substitui o conjunto de tags.
    pub fn set_tags(&self, id: &AssetId, tags: &[String]) -> Result<()> {
        let tags = normalize_tags(tags);
        let (loc, m) = self.mutate_manifest(id, |m| {
            m.tags = tags.clone();
            Ok(())
        })?;
        self.reindex(&loc, &m)
    }

    /// Acrescenta tags (normalizadas; invalidas sao rejeitadas).
    pub fn add_tags(&self, id: &AssetId, tags: &[String]) -> Result<()> {
        for t in tags {
            if normalize_tag(t).is_none() {
                return Err(AssetError::InvalidInput(format!("tag invalida: {t:?}")));
            }
        }
        let add = normalize_tags(tags);
        let (loc, m) = self.mutate_manifest(id, |m| {
            m.tags.extend(add.iter().cloned());
            m.tags = normalize_tags(m.tags.iter());
            Ok(())
        })?;
        self.reindex(&loc, &m)
    }

    /// Remove tags exatas.
    pub fn remove_tags(&self, id: &AssetId, tags: &[String]) -> Result<()> {
        let rm = normalize_tags(tags);
        let (loc, m) = self.mutate_manifest(id, |m| {
            m.tags.retain(|t| !rm.contains(t));
            Ok(())
        })?;
        self.reindex(&loc, &m)
    }

    /// Aplica `add_tags` a varios ativos. Devolve quantos foram alterados; para no primeiro erro.
    pub fn add_tags_many(&self, ids: &[AssetId], tags: &[String]) -> Result<usize> {
        for id in ids {
            self.add_tags(id, tags)?;
        }
        Ok(ids.len())
    }

    /// Todas as tags com contagem de usos.
    pub fn tags(&self) -> Result<Vec<TagInfo>> {
        self.inner.db.read(|c| {
            let mut st = c.prepare(
                "SELECT t.path, t.color, (SELECT count(*) FROM asset_tag at JOIN asset a ON a.seq = at.asset_seq \
                 WHERE at.tag_id = t.id AND a.state = 0) FROM tag t ORDER BY t.path",
            )?;
            let rows = st.query_map([], |r| Ok(TagInfo { path: r.get(0)?, color: r.get(1)?, count: r.get::<_, i64>(2)?.max(0) as u64 }))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Cor (qualquer texto curto, ex.: `#aabbcc`) de uma tag. Metadado so do indice/UI: nao entra no `asset.json`.
    pub fn set_tag_color(&self, path: &str, color: &str) -> Result<()> {
        let path = normalize_tag(path).ok_or_else(|| AssetError::InvalidInput("tag invalida".into()))?;
        check_len("cor", color, 32)?;
        self.inner.db.write(|c| {
            let mut cache = TagCache::default();
            let id = store::ensure_tag(c, &mut cache, &path)?;
            c.execute("UPDATE tag SET color = ?2 WHERE id = ?1", params![id, color])?;
            Ok(())
        })
    }

    fn assets_with_tag_tree(&self, path: &str) -> Result<Vec<AssetId>> {
        let like = format!("{}/%", path.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        self.inner.db.read(|c| {
            let mut st = c.prepare(
                "SELECT DISTINCT a.id FROM asset a JOIN asset_tag at ON at.asset_seq = a.seq JOIN tag t ON t.id = at.tag_id \
                 WHERE t.path = ?1 OR t.path LIKE ?2 ESCAPE '\\'",
            )?;
            let rows = st.query_map(params![path, like], |r| r.get::<_, Vec<u8>>(0))?;
            Ok(rows.filter_map(|r| r.ok()).filter_map(|b| AssetId::from_bytes(&b)).collect())
        })
    }

    /// Renomeia/move uma tag e todos os seus descendentes (`estilo/cel` -> `look/cel`). Reescreve os
    /// manifestos afetados. Devolve quantos ativos mudaram.
    pub fn rename_tag(&self, old: &str, new: &str) -> Result<usize> {
        let old = normalize_tag(old).ok_or_else(|| AssetError::InvalidInput("tag antiga invalida".into()))?;
        let new = normalize_tag(new).ok_or_else(|| AssetError::InvalidInput("tag nova invalida".into()))?;
        if old == new {
            return Ok(0);
        }
        if new.starts_with(&format!("{old}/")) {
            return Err(AssetError::InvalidInput("nao e possivel mover uma tag para dentro dela mesma".into()));
        }
        let ids = self.assets_with_tag_tree(&old)?;
        let prefix = format!("{old}/");
        for id in &ids {
            let (loc, m) = self.mutate_manifest(id, |m| {
                m.tags = normalize_tags(m.tags.iter().map(|t| {
                    if *t == old {
                        new.clone()
                    } else if let Some(rest) = t.strip_prefix(&prefix) {
                        format!("{new}/{rest}")
                    } else {
                        t.clone()
                    }
                }));
                Ok(())
            })?;
            self.reindex(&loc, &m)?;
        }
        self.prune_tags()?;
        Ok(ids.len())
    }

    /// Remove uma tag (e descendentes) de todos os ativos.
    pub fn delete_tag(&self, path: &str) -> Result<usize> {
        let path = normalize_tag(path).ok_or_else(|| AssetError::InvalidInput("tag invalida".into()))?;
        let ids = self.assets_with_tag_tree(&path)?;
        let prefix = format!("{path}/");
        for id in &ids {
            let (loc, m) = self.mutate_manifest(id, |m| {
                m.tags.retain(|t| *t != path && !t.starts_with(&prefix));
                Ok(())
            })?;
            self.reindex(&loc, &m)?;
        }
        self.prune_tags()?;
        Ok(ids.len())
    }

    /// Apaga linhas de `tag` sem nenhum uso. Devolve quantas.
    pub fn prune_tags(&self) -> Result<usize> {
        self.inner.db.write(|c| Ok(c.execute("DELETE FROM tag WHERE id NOT IN (SELECT DISTINCT tag_id FROM asset_tag)", [])?))
    }

    // -----------------------------------------------------------------------------------------
    // Dependencias
    // -----------------------------------------------------------------------------------------

    /// Do que este ativo depende.
    pub fn dependencies(&self, id: &AssetId) -> Result<Vec<DependencyRef>> {
        self.inner.db.read(|c| {
            let mut st = c.prepare_cached("SELECT dep_id, kind FROM dep WHERE asset_id = ?1 ORDER BY kind, dep_id")?;
            let rows = st.query_map([id.as_bytes().as_slice()], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?)))?;
            let mut out = Vec::new();
            for r in rows {
                let (b, kind) = r?;
                if let Some(id) = AssetId::from_bytes(&b) {
                    out.push(DependencyRef { id, kind });
                }
            }
            Ok(out)
        })
    }

    /// Quem depende deste ativo (ativos nao apagados; a lixeira conta como "em uso" so se `include_trashed`).
    pub fn used_by(&self, id: &AssetId) -> Result<Vec<AssetId>> {
        self.used_by_filtered(id, false)
    }

    pub(crate) fn used_by_filtered(&self, id: &AssetId, include_trashed: bool) -> Result<Vec<AssetId>> {
        self.inner.db.read(|c| {
            let sql = if include_trashed {
                "SELECT DISTINCT d.asset_id FROM dep d WHERE d.dep_id = ?1"
            } else {
                "SELECT DISTINCT d.asset_id FROM dep d JOIN asset a ON a.id = d.asset_id WHERE d.dep_id = ?1 AND a.state != 1"
            };
            let mut st = c.prepare_cached(sql)?;
            let rows = st.query_map([id.as_bytes().as_slice()], |r| r.get::<_, Vec<u8>>(0))?;
            Ok(rows.filter_map(|r| r.ok()).filter_map(|b| AssetId::from_bytes(&b)).collect())
        })
    }

    /// Declara que `asset` usa `dep` (grava no manifesto de `asset` e no indice).
    pub fn add_dependency(&self, asset: &AssetId, dep: &AssetId, kind: &str) -> Result<()> {
        if asset == dep {
            return Err(AssetError::InvalidInput("um ativo nao pode depender de si mesmo".into()));
        }
        check_len("tipo de dependencia", kind, 64)?;
        self.require_row(dep)?;
        if self.depends_transitively(dep, asset)? {
            return Err(AssetError::Conflict("a dependencia criaria um ciclo".into()));
        }
        let (loc, m) = self.mutate_manifest(asset, |m| {
            let r = DependencyRef { id: *dep, kind: kind.to_string() };
            if !m.dependencies.contains(&r) {
                m.dependencies.push(r);
            }
            Ok(())
        })?;
        self.reindex(&loc, &m)
    }

    /// Remove uma dependencia.
    pub fn remove_dependency(&self, asset: &AssetId, dep: &AssetId, kind: &str) -> Result<()> {
        let (loc, m) = self.mutate_manifest(asset, |m| {
            m.dependencies.retain(|d| !(d.id == *dep && d.kind == kind));
            Ok(())
        })?;
        self.reindex(&loc, &m)
    }

    /// `true` se `from` alcanca `target` seguindo dependencias (busca limitada a 100k nos).
    fn depends_transitively(&self, from: &AssetId, target: &AssetId) -> Result<bool> {
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![*from];
        while let Some(cur) = stack.pop() {
            if &cur == target {
                return Ok(true);
            }
            if !seen.insert(cur) || seen.len() > 100_000 {
                continue;
            }
            for d in self.dependencies(&cur)? {
                stack.push(d.id);
            }
        }
        Ok(false)
    }

    /// Grafo de referencias a partir de `root` ate `depth` niveis (estilo Reference Viewer), em duas direcoes:
    /// `(de, para, tipo)`, "de usa para". No maximo `max_edges` arestas.
    pub fn reference_graph(&self, root: &AssetId, depth: u32, include_users: bool, max_edges: usize) -> Result<Vec<(AssetId, AssetId, String)>> {
        let mut edges = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut frontier = vec![*root];
        seen.insert(*root);
        for _ in 0..depth {
            let mut next = Vec::new();
            for a in &frontier {
                for d in self.dependencies(a)? {
                    if edges.len() >= max_edges {
                        return Ok(edges);
                    }
                    edges.push((*a, d.id, d.kind.clone()));
                    if seen.insert(d.id) {
                        next.push(d.id);
                    }
                }
                if include_users {
                    for u in self.used_by(a)? {
                        if edges.len() >= max_edges {
                            return Ok(edges);
                        }
                        let kind = self.dependencies(&u)?.into_iter().find(|d| d.id == *a).map(|d| d.kind).unwrap_or_default();
                        edges.push((u, *a, kind));
                        if seen.insert(u) {
                            next.push(u);
                        }
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        Ok(edges)
    }
}
