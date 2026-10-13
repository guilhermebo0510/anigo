//! Colecoes manuais (lista de ids) e inteligentes (consulta salva em JSON).

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::{AssetError, IoContext, Result};
use crate::library::{Library, COLLECTIONS_FILE};
use crate::model::*;
use crate::query::Query;
use crate::util::{atomic_write, now_secs};

/// Definicao de uma colecao.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectionDef {
    pub id: CollectionId,
    pub name: String,
    /// `Some` = inteligente (consulta salva); `None` = manual.
    pub query: Option<Query>,
    pub created_at: i64,
    /// Manual: numero de itens. Inteligente: 0 (use `Library::count` com `Query::collection`).
    pub item_count: u64,
}

impl CollectionDef {
    pub fn is_smart(&self) -> bool {
        self.query.is_some()
    }
}

#[derive(Serialize, Deserialize)]
struct SnapshotCollection {
    id: i64,
    name: String,
    query: Option<Query>,
    created_at: i64,
    items: Vec<AssetId>,
}

#[derive(Serialize, Deserialize, Default)]
struct Snapshot {
    collections: Vec<SnapshotCollection>,
}

fn validate_name(name: &str) -> Result<String> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > 128 || n.chars().any(|c| c.is_control()) {
        return Err(AssetError::InvalidInput(format!("nome de colecao invalido: {name:?}")));
    }
    Ok(n.to_string())
}

fn clean_smart_query(q: &Query) -> Result<Query> {
    if q.collection.is_some() {
        return Err(AssetError::InvalidInput("colecao inteligente nao pode referenciar outra colecao".into()));
    }
    let mut q = q.clone();
    q.after = None;
    q.with_total = false;
    // valida tags cedo
    for t in q.tags.iter().chain(q.any_tags.iter()) {
        if crate::store::normalize_tag(t).is_none() {
            return Err(AssetError::InvalidInput(format!("tag invalida: {t:?}")));
        }
    }
    Ok(q)
}

impl Library {
    /// Cria uma colecao manual.
    pub fn create_collection(&self, name: &str) -> Result<CollectionId> {
        self.insert_collection(name, None)
    }

    /// Cria uma colecao inteligente a partir de uma consulta (filtros: texto, tipos, tags, favorito...).
    /// `q.collection`, `after` e `with_total` sao ignorados/rejeitados.
    pub fn create_smart_collection(&self, name: &str, q: &Query) -> Result<CollectionId> {
        let q = clean_smart_query(q)?;
        self.insert_collection(name, Some(q))
    }

    fn insert_collection(&self, name: &str, q: Option<Query>) -> Result<CollectionId> {
        let name = validate_name(name)?;
        let qjson = match &q {
            Some(q) => serde_json::to_string(q)?,
            None => String::new(),
        };
        let id = self.inner.db.write(|c| {
            let exists: Option<i64> = c.query_row("SELECT id FROM collection WHERE name = ?1", [&name], |r| r.get(0)).optional()?;
            if exists.is_some() {
                return Err(AssetError::Conflict(format!("ja existe uma colecao chamada {name:?}")));
            }
            c.execute(
                "INSERT INTO collection(name, smart, query, created_at) VALUES (?1,?2,?3,?4)",
                params![name, q.is_some() as i64, qjson, now_secs()],
            )?;
            Ok(c.last_insert_rowid())
        })?;
        self.write_collections_snapshot()?;
        Ok(CollectionId(id))
    }

    /// Troca a consulta de uma colecao inteligente.
    pub fn update_smart_collection(&self, id: CollectionId, q: &Query) -> Result<()> {
        let q = clean_smart_query(q)?;
        let json = serde_json::to_string(&q)?;
        let n = self.inner.db.write(|c| Ok(c.execute("UPDATE collection SET query = ?2 WHERE id = ?1 AND smart = 1", params![id.0, json])?))?;
        if n == 0 {
            return Err(AssetError::NotFound(format!("colecao inteligente {id}")));
        }
        self.write_collections_snapshot()
    }

    pub fn rename_collection(&self, id: CollectionId, name: &str) -> Result<()> {
        let name = validate_name(name)?;
        self.inner.db.write(|c| {
            let clash: Option<i64> = c.query_row("SELECT id FROM collection WHERE name = ?1 AND id != ?2", params![name, id.0], |r| r.get(0)).optional()?;
            if clash.is_some() {
                return Err(AssetError::Conflict(format!("ja existe uma colecao chamada {name:?}")));
            }
            let n = c.execute("UPDATE collection SET name = ?2 WHERE id = ?1", params![id.0, name])?;
            if n == 0 {
                return Err(AssetError::NotFound(format!("colecao {id}")));
            }
            Ok(())
        })?;
        self.write_collections_snapshot()
    }

    /// Apaga a colecao (os ativos nao sao afetados).
    pub fn delete_collection(&self, id: CollectionId) -> Result<()> {
        self.inner.db.write(|c| {
            let tx = c.transaction()?;
            let n = tx.execute("DELETE FROM collection WHERE id = ?1", [id.0])?;
            if n == 0 {
                return Err(AssetError::NotFound(format!("colecao {id}")));
            }
            tx.execute("DELETE FROM collection_item WHERE coll_id = ?1", [id.0])?;
            tx.commit()?;
            Ok(())
        })?;
        self.write_collections_snapshot()
    }

    /// Adiciona ativos a uma colecao manual. Devolve quantos eram novos.
    pub fn add_to_collection(&self, id: CollectionId, assets: &[AssetId]) -> Result<u64> {
        let def = self.collection(id)?;
        if def.is_smart() {
            return Err(AssetError::InvalidInput("colecoes inteligentes nao aceitam itens manuais".into()));
        }
        let added = self.inner.db.write(|c| {
            let tx = c.transaction()?;
            let mut added = 0u64;
            for a in assets {
                if crate::store::seq_of(&tx, a)?.is_none() {
                    return Err(AssetError::NotFound(format!("ativo {a}")));
                }
                added += tx.execute("INSERT OR IGNORE INTO collection_item(coll_id, asset_id) VALUES (?1,?2)", params![id.0, a.as_bytes().as_slice()])? as u64;
            }
            tx.commit()?;
            Ok(added)
        })?;
        self.write_collections_snapshot()?;
        Ok(added)
    }

    /// Remove ativos de uma colecao manual. Devolve quantos foram removidos.
    pub fn remove_from_collection(&self, id: CollectionId, assets: &[AssetId]) -> Result<u64> {
        let n = self.inner.db.write(|c| {
            let tx = c.transaction()?;
            let mut n = 0u64;
            for a in assets {
                n += tx.execute("DELETE FROM collection_item WHERE coll_id = ?1 AND asset_id = ?2", params![id.0, a.as_bytes().as_slice()])? as u64;
            }
            tx.commit()?;
            Ok(n)
        })?;
        self.write_collections_snapshot()?;
        Ok(n)
    }

    /// Definicao de uma colecao.
    pub fn collection(&self, id: CollectionId) -> Result<CollectionDef> {
        self.collections()?.into_iter().find(|c| c.id == id).ok_or_else(|| AssetError::NotFound(format!("colecao {id}")))
    }

    /// Todas as colecoes, por nome.
    pub fn collections(&self) -> Result<Vec<CollectionDef>> {
        self.inner.db.read(|c| {
            let mut st = c.prepare(
                "SELECT c.id, c.name, c.smart, c.query, c.created_at, \
                 (SELECT count(*) FROM collection_item i WHERE i.coll_id = c.id) FROM collection c ORDER BY c.name COLLATE NOCASE",
            )?;
            let rows = st.query_map([], |r| {
                let smart: i64 = r.get(2)?;
                let qjson: String = r.get(3)?;
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, smart != 0, qjson, r.get::<_, i64>(4)?, r.get::<_, i64>(5)?))
            })?;
            let mut out = Vec::new();
            for r in rows {
                let (id, name, smart, qjson, created_at, items) = r?;
                let query = if smart {
                    Some(serde_json::from_str::<Query>(&qjson).map_err(|e| AssetError::Corrupt(format!("consulta da colecao {name:?}: {e}")))?)
                } else {
                    None
                };
                out.push(CollectionDef { id: CollectionId(id), name, query, created_at, item_count: if smart { 0 } else { items.max(0) as u64 } });
            }
            Ok(out)
        })
    }

    /// Colecoes manuais que contem o ativo.
    pub fn collections_of(&self, asset: &AssetId) -> Result<Vec<CollectionId>> {
        self.inner.db.read(|c| {
            let mut st = c.prepare("SELECT coll_id FROM collection_item WHERE asset_id = ?1 ORDER BY coll_id")?;
            let v = st.query_map([asset.as_bytes().as_slice()], |r| r.get::<_, i64>(0))?;
            Ok(v.collect::<std::result::Result<Vec<_>, _>>()?.into_iter().map(CollectionId).collect())
        })
    }

    /// Grava `collections.json` (copia de seguranca das colecoes, que nao vivem nos manifestos dos ativos).
    pub(crate) fn write_collections_snapshot(&self) -> Result<()> {
        let defs = self.collections()?;
        let mut snap = Snapshot::default();
        self.inner.db.read(|c| {
            for d in &defs {
                let items: Vec<AssetId> = if d.is_smart() {
                    vec![]
                } else {
                    let mut st = c.prepare("SELECT asset_id FROM collection_item WHERE coll_id = ?1 ORDER BY asset_id")?;
                    let rows = st.query_map([d.id.0], |r| r.get::<_, Vec<u8>>(0))?;
                    rows.filter_map(|r| r.ok()).filter_map(|b| AssetId::from_bytes(&b)).collect()
                };
                snap.collections.push(SnapshotCollection { id: d.id.0, name: d.name.clone(), query: d.query.clone(), created_at: d.created_at, items });
            }
            Ok(())
        })?;
        atomic_write(&self.inner.home.join(COLLECTIONS_FILE), &serde_json::to_vec(&snap)?)
    }

    /// Restaura colecoes de `collections.json` quando a tabela esta vazia (indice recriado).
    pub(crate) fn restore_collections_snapshot(&self) -> Result<u64> {
        let path = self.inner.home.join(COLLECTIONS_FILE);
        if !path.exists() {
            return Ok(0);
        }
        let empty = self.inner.db.read(|c| Ok(c.query_row("SELECT count(*) FROM collection", [], |r| r.get::<_, i64>(0))? == 0))?;
        if !empty {
            return Ok(0);
        }
        let meta = std::fs::metadata(&path).at(&path)?;
        if meta.len() > 256 * 1024 * 1024 {
            return Err(AssetError::LimitExceeded { what: "collections.json", limit: 256 * 1024 * 1024, actual: meta.len() });
        }
        let bytes = std::fs::read(&path).at(&path)?;
        let snap: Snapshot = serde_json::from_slice(&bytes).map_err(|e| AssetError::Corrupt(format!("collections.json ilegivel: {e}")))?;
        let n = snap.collections.len() as u64;
        self.inner.db.write(|c| {
            let tx = c.transaction()?;
            for col in &snap.collections {
                let qjson = match &col.query {
                    Some(q) => serde_json::to_string(q)?,
                    None => String::new(),
                };
                tx.execute(
                    "INSERT OR IGNORE INTO collection(id, name, smart, query, created_at) VALUES (?1,?2,?3,?4,?5)",
                    params![col.id, col.name, col.query.is_some() as i64, qjson, col.created_at],
                )?;
                for a in &col.items {
                    tx.execute("INSERT OR IGNORE INTO collection_item(coll_id, asset_id) VALUES (?1,?2)", params![col.id, a.as_bytes().as_slice()])?;
                }
            }
            tx.commit()?;
            Ok(())
        })?;
        Ok(n)
    }
}
