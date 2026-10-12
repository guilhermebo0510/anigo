//! SQL de baixo nivel sobre o indice: upsert de ativos, tags, dependencias, FTS.
//! Tudo aqui opera sobre uma `&Connection` (normalmente dentro de uma transacao).

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::{AssetError, Result};
use crate::model::*;

/// Colunas de [`AssetRow`] (com `a` = asset, `r` = root).
pub(crate) const ROW_COLS: &str = "a.seq, a.id, a.name, a.kind, a.format, a.storage, a.state, a.root_id, a.rel_path, \
     a.content_hash, a.size, a.rating, a.favorite, a.revision, a.tris, a.verts, a.materials, a.bones, a.anims, \
     a.has_skin, a.has_vrm, a.imported_at, a.modified_at, r.online";
pub(crate) const ROW_FROM: &str = "FROM asset a JOIN root r ON r.id = a.root_id";

pub(crate) fn row_to_asset(r: &Row<'_>) -> rusqlite::Result<AssetRow> {
    let id_bytes: Vec<u8> = r.get(1)?;
    let kind: String = r.get(3)?;
    let hash: Vec<u8> = r.get(9)?;
    let bad = |what: &str| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Blob, what.into());
    Ok(AssetRow {
        seq: r.get(0)?,
        id: AssetId::from_bytes(&id_bytes).ok_or_else(|| bad("id invalido"))?,
        name: r.get(2)?,
        kind: kind.parse().unwrap_or(AssetKind::Model),
        format: r.get(4)?,
        storage: StorageMode::from_db(r.get(5)?),
        state: AssetState::from_db(r.get(6)?),
        root_id: RootId(r.get(7)?),
        rel_path: r.get(8)?,
        content_hash: ContentHash::from_slice(&hash).ok_or_else(|| bad("hash invalido"))?,
        size: r.get::<_, i64>(10)?.max(0) as u64,
        rating: r.get::<_, i64>(11)?.clamp(0, 5) as u8,
        favorite: r.get::<_, i64>(12)? != 0,
        revision: r.get::<_, i64>(13)?.max(0) as u32,
        tris: r.get::<_, i64>(14)?.max(0) as u64,
        verts: r.get::<_, i64>(15)?.max(0) as u64,
        materials: r.get::<_, i64>(16)?.max(0) as u32,
        bones: r.get::<_, i64>(17)?.max(0) as u32,
        anims: r.get::<_, i64>(18)?.max(0) as u32,
        has_skin: r.get::<_, i64>(19)? != 0,
        has_vrm: r.get::<_, i64>(20)? != 0,
        imported_at: r.get(21)?,
        modified_at: r.get(22)?,
        available: r.get::<_, i64>(23)? != 0,
    })
}

/// Normaliza uma tag hierarquica: segmentos separados por `/`, minusculos, sem vazios/controles,
/// no maximo 8 niveis de 64 caracteres. `None` se nada sobrar.
pub fn normalize_tag(raw: &str) -> Option<String> {
    let mut segs: Vec<String> = Vec::new();
    for seg in raw.split('/') {
        let cleaned: String = seg
            .chars()
            .filter(|c| !c.is_control())
            .collect::<String>()
            .trim()
            .to_lowercase()
            .chars()
            .take(64)
            .collect();
        let cleaned = cleaned.trim().to_string();
        if !cleaned.is_empty() {
            segs.push(cleaned);
        }
        if segs.len() >= 8 {
            break;
        }
    }
    if segs.is_empty() {
        None
    } else {
        Some(segs.join("/"))
    }
}

/// Normaliza, remove duplicatas e ordena.
pub(crate) fn normalize_tags<I: IntoIterator<Item = S>, S: AsRef<str>>(tags: I) -> Vec<String> {
    let mut v: Vec<String> = tags.into_iter().filter_map(|t| normalize_tag(t.as_ref())).collect();
    v.sort();
    v.dedup();
    v
}

/// Chave de ordenacao por nome: minuscula (Unicode).
pub(crate) fn name_key(name: &str) -> String {
    name.to_lowercase()
}

/// Dados para (re)indexar um ativo.
pub(crate) struct IndexEntry<'a> {
    pub manifest: &'a AssetManifest,
    pub root_id: i64,
    /// Pasta do manifesto relativa a base (raiz managed, ou home para linked).
    pub manifest_dir: String,
    pub state: AssetState,
    pub file_mtime_ns: i64,
    pub trashed_at: Option<i64>,
}

/// Cache de ids de tag durante operacoes em lote.
#[derive(Default)]
pub(crate) struct TagCache(HashMap<String, i64>);

pub(crate) fn ensure_tag(conn: &Connection, cache: &mut TagCache, path: &str) -> Result<i64> {
    if let Some(id) = cache.0.get(path) {
        return Ok(*id);
    }
    conn.prepare_cached("INSERT OR IGNORE INTO tag(path) VALUES (?1)")?.execute([path])?;
    let id: i64 = conn.prepare_cached("SELECT id FROM tag WHERE path = ?1")?.query_row([path], |r| r.get(0))?;
    cache.0.insert(path.to_string(), id);
    Ok(id)
}

fn rel_path_of(m: &AssetManifest) -> String {
    match (&m.storage, &m.link) {
        (StorageMode::Linked, Some(l)) => l.rel_path.clone(),
        _ => m.primary.clone(),
    }
}

/// Insere ou atualiza o ativo (por `id`) incluindo FTS, tags e dependencias. Devolve `seq`.
pub(crate) fn upsert_asset(conn: &Connection, cache: &mut TagCache, e: &IndexEntry<'_>) -> Result<i64> {
    let m = e.manifest;
    let rev = m.current().ok_or_else(|| AssetError::Corrupt(format!("manifesto {} sem revisao atual", m.asset_id)))?;
    let mut stmt = conn.prepare_cached(
        "INSERT INTO asset(id, root_id, rel_path, manifest_dir, name, name_key, kind, format, description, author, license, \
         source_url, content_hash, size, primary_size, storage, state, rating, favorite, revision, tris, verts, materials, bones, anims, \
         has_skin, has_vrm, file_mtime_ns, imported_at, modified_at, trashed_at) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31) \
         ON CONFLICT(id) DO UPDATE SET root_id=excluded.root_id, rel_path=excluded.rel_path, manifest_dir=excluded.manifest_dir, \
         name=excluded.name, name_key=excluded.name_key, kind=excluded.kind, format=excluded.format, \
         description=excluded.description, author=excluded.author, license=excluded.license, source_url=excluded.source_url, \
         content_hash=excluded.content_hash, size=excluded.size, primary_size=excluded.primary_size, storage=excluded.storage, state=excluded.state, \
         rating=excluded.rating, favorite=excluded.favorite, revision=excluded.revision, tris=excluded.tris, \
         verts=excluded.verts, materials=excluded.materials, bones=excluded.bones, anims=excluded.anims, \
         has_skin=excluded.has_skin, has_vrm=excluded.has_vrm, file_mtime_ns=excluded.file_mtime_ns, \
         imported_at=excluded.imported_at, modified_at=excluded.modified_at, trashed_at=excluded.trashed_at \
         RETURNING seq",
    )?;
    let s = &m.stats;
    let seq: i64 = stmt.query_row(
        params![
            m.asset_id.as_bytes().as_slice(),
            e.root_id,
            rel_path_of(m),
            e.manifest_dir,
            m.name,
            name_key(&m.name),
            m.kind.as_str(),
            m.format,
            m.description,
            m.author,
            m.license,
            m.source_url,
            rev.content_hash.0.as_slice(),
            rev.size as i64,
            rev.files.first().map(|f| f.size as i64).unwrap_or(0),
            m.storage.to_db(),
            e.state.to_db(),
            m.rating as i64,
            m.favorite as i64,
            m.current_revision as i64,
            s.tris as i64,
            s.verts as i64,
            s.materials as i64,
            s.bones as i64,
            s.anims as i64,
            s.has_skin as i64,
            s.has_vrm as i64,
            e.file_mtime_ns,
            m.imported_at,
            m.modified_at,
            e.trashed_at,
        ],
        |r| r.get(0),
    )?;
    set_asset_tags(conn, cache, seq, &m.tags)?;
    fts_replace(conn, seq, &m.name, &m.description, &m.tags, &m.author)?;
    replace_deps(conn, &m.asset_id, &m.dependencies)?;
    replace_claims(conn, seq, e.root_id, m)?;
    Ok(seq)
}

/// Registra os arquivos dependentes de um ativo linked (para o scan nao os tratar como ativos soltos).
pub(crate) fn replace_claims(conn: &Connection, seq: i64, root_id: i64, m: &AssetManifest) -> Result<()> {
    conn.prepare_cached("DELETE FROM file_claim WHERE asset_seq = ?1")?.execute([seq])?;
    if m.storage != StorageMode::Linked {
        return Ok(());
    }
    let (Some(link), Some(rev)) = (&m.link, m.current()) else { return Ok(()) };
    let dir = link.rel_path.rsplit_once('/').map(|(d, _)| d.to_string());
    for f in rev.files.iter().skip(1) {
        let rel = match &dir {
            Some(d) => format!("{d}/{}", f.path),
            None => f.path.clone(),
        };
        conn.prepare_cached("INSERT OR IGNORE INTO file_claim(root_id, rel_path, asset_seq) VALUES (?1,?2,?3)")?.execute(params![root_id, rel, seq])?;
    }
    Ok(())
}

pub(crate) fn fts_replace(conn: &Connection, seq: i64, name: &str, desc: &str, tags: &[String], author: &str) -> Result<()> {
    conn.prepare_cached("DELETE FROM asset_fts WHERE rowid = ?1")?.execute([seq])?;
    conn.prepare_cached("INSERT INTO asset_fts(rowid, name, description, tags, author) VALUES (?1,?2,?3,?4,?5)")?
        .execute(params![seq, name, desc, tags.join(" "), author])?;
    Ok(())
}

/// Substitui o conjunto de tags do ativo (as strings devem estar normalizadas).
pub(crate) fn set_asset_tags(conn: &Connection, cache: &mut TagCache, seq: i64, tags: &[String]) -> Result<()> {
    conn.prepare_cached("DELETE FROM asset_tag WHERE asset_seq = ?1")?.execute([seq])?;
    for t in tags {
        let id = ensure_tag(conn, cache, t)?;
        conn.prepare_cached("INSERT OR IGNORE INTO asset_tag(tag_id, asset_seq) VALUES (?1,?2)")?.execute([id, seq])?;
    }
    Ok(())
}

pub(crate) fn replace_deps(conn: &Connection, id: &AssetId, deps: &[DependencyRef]) -> Result<()> {
    conn.prepare_cached("DELETE FROM dep WHERE asset_id = ?1")?.execute([id.as_bytes().as_slice()])?;
    for d in deps {
        conn.prepare_cached("INSERT OR IGNORE INTO dep(asset_id, dep_id, kind) VALUES (?1,?2,?3)")?
            .execute(params![id.as_bytes().as_slice(), d.id.as_bytes().as_slice(), d.kind])?;
    }
    Ok(())
}

/// Remove todas as linhas do ativo no indice (nao toca em disco). Colecoes manuais mantem o id (sobrevive a restore).
pub(crate) fn delete_asset_rows(conn: &Connection, seq: i64, id: &AssetId) -> Result<()> {
    conn.prepare_cached("DELETE FROM asset_fts WHERE rowid = ?1")?.execute([seq])?;
    conn.prepare_cached("DELETE FROM asset_tag WHERE asset_seq = ?1")?.execute([seq])?;
    conn.prepare_cached("DELETE FROM file_claim WHERE asset_seq = ?1")?.execute([seq])?;
    conn.prepare_cached("DELETE FROM dep WHERE asset_id = ?1")?.execute([id.as_bytes().as_slice()])?;
    conn.prepare_cached("DELETE FROM asset WHERE seq = ?1")?.execute([seq])?;
    Ok(())
}

pub(crate) fn seq_of(conn: &Connection, id: &AssetId) -> Result<Option<i64>> {
    Ok(conn
        .prepare_cached("SELECT seq FROM asset WHERE id = ?1")?
        .query_row([id.as_bytes().as_slice()], |r| r.get(0))
        .optional()?)
}

pub(crate) fn require_seq(conn: &Connection, id: &AssetId) -> Result<i64> {
    seq_of(conn, id)?.ok_or_else(|| AssetError::NotFound(format!("ativo {id}")))
}

pub(crate) fn get_row(conn: &Connection, id: &AssetId) -> Result<Option<AssetRow>> {
    let sql = format!("SELECT {ROW_COLS} {ROW_FROM} WHERE a.id = ?1");
    Ok(conn.prepare_cached(&sql)?.query_row([id.as_bytes().as_slice()], row_to_asset).optional()?)
}

/// Tags de um ativo (ordenadas).
pub(crate) fn tags_of(conn: &Connection, seq: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare_cached("SELECT t.path FROM asset_tag at JOIN tag t ON t.id = at.tag_id WHERE at.asset_seq = ?1 ORDER BY t.path")?;
    let rows = stmt.query_map([seq], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_normalization() {
        assert_eq!(normalize_tag("  Estilo / CEL /Noir "), Some("estilo/cel/noir".into()));
        assert_eq!(normalize_tag("//"), None);
        assert_eq!(normalize_tag(""), None);
        assert_eq!(normalize_tag("a\u{0}b/c"), Some("ab/c".into()));
        let deep = (0..20).map(|i| format!("n{i}")).collect::<Vec<_>>().join("/");
        assert_eq!(normalize_tag(&deep).unwrap().split('/').count(), 8);
        assert_eq!(normalize_tag(&"x".repeat(200)).unwrap().len(), 64);
        assert_eq!(normalize_tags(["B", "a", "b", " A "]), vec!["a", "b"]);
    }
}
