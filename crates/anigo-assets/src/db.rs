//! Camada SQLite: conexoes (1 escritora + pool de leitoras), PRAGMAs, migracoes por `user_version`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{Connection, OpenFlags};

use crate::error::{AssetError, Result};

/// Versao do esquema que este binario conhece.
pub const SCHEMA_VERSION: i64 = 1;

/// Migracoes incrementais: `MIGRATIONS[i]` leva do `user_version` `i` para `i + 1`.
/// Nunca edite uma migracao ja publicada: acrescente outra.
pub(crate) const MIGRATIONS: &[&str] = &[
    // v0 -> v1: esquema inicial.
    r#"
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;

CREATE TABLE root(
  id        INTEGER PRIMARY KEY,
  uuid      TEXT NOT NULL UNIQUE,
  name      TEXT NOT NULL,
  path      TEXT NOT NULL,
  mode      INTEGER NOT NULL,
  online    INTEGER NOT NULL DEFAULT 1,
  watch     INTEGER NOT NULL DEFAULT 0,
  last_scan INTEGER
) STRICT;

CREATE TABLE asset(
  seq           INTEGER PRIMARY KEY,
  id            BLOB NOT NULL UNIQUE,
  root_id       INTEGER NOT NULL,
  rel_path      TEXT NOT NULL,
  manifest_dir  TEXT NOT NULL,
  name          TEXT NOT NULL,
  name_key      TEXT NOT NULL,
  kind          TEXT NOT NULL,
  format        TEXT NOT NULL,
  description   TEXT NOT NULL DEFAULT '',
  author        TEXT NOT NULL DEFAULT '',
  license       TEXT NOT NULL DEFAULT '',
  source_url    TEXT NOT NULL DEFAULT '',
  content_hash  BLOB NOT NULL,
  size          INTEGER NOT NULL,
  primary_size  INTEGER NOT NULL DEFAULT 0,
  storage       INTEGER NOT NULL,
  state         INTEGER NOT NULL DEFAULT 0,
  rating        INTEGER NOT NULL DEFAULT 0,
  favorite      INTEGER NOT NULL DEFAULT 0,
  revision      INTEGER NOT NULL DEFAULT 1,
  tris          INTEGER NOT NULL DEFAULT 0,
  verts         INTEGER NOT NULL DEFAULT 0,
  materials     INTEGER NOT NULL DEFAULT 0,
  bones         INTEGER NOT NULL DEFAULT 0,
  anims         INTEGER NOT NULL DEFAULT 0,
  has_skin      INTEGER NOT NULL DEFAULT 0,
  has_vrm       INTEGER NOT NULL DEFAULT 0,
  file_mtime_ns INTEGER NOT NULL DEFAULT 0,
  imported_at   INTEGER NOT NULL,
  modified_at   INTEGER NOT NULL,
  trashed_at    INTEGER
) STRICT;

CREATE INDEX ix_asset_name   ON asset(state, name_key, seq);
CREATE INDEX ix_asset_kind   ON asset(state, kind, name_key, seq);
CREATE INDEX ix_asset_hash   ON asset(content_hash);
CREATE INDEX ix_asset_imp    ON asset(state, imported_at, seq);
CREATE INDEX ix_asset_mod    ON asset(state, modified_at, seq);
CREATE INDEX ix_asset_size   ON asset(state, size, seq);
CREATE INDEX ix_asset_rating ON asset(state, rating, seq);
CREATE INDEX ix_asset_root   ON asset(root_id, rel_path);
CREATE INDEX ix_asset_fav    ON asset(favorite, seq) WHERE favorite = 1;

-- arquivos dependentes (texturas/buffers) de ativos LINKED: o scan nao os registra como ativos soltos
CREATE TABLE file_claim(
  root_id   INTEGER NOT NULL,
  rel_path  TEXT NOT NULL,
  asset_seq INTEGER NOT NULL,
  PRIMARY KEY(root_id, rel_path, asset_seq)
) STRICT, WITHOUT ROWID;
CREATE INDEX ix_claim_asset ON file_claim(asset_seq);

CREATE TABLE tag(
  id    INTEGER PRIMARY KEY,
  path  TEXT NOT NULL UNIQUE,
  color TEXT NOT NULL DEFAULT ''
) STRICT;

CREATE TABLE asset_tag(
  tag_id    INTEGER NOT NULL,
  asset_seq INTEGER NOT NULL,
  PRIMARY KEY(tag_id, asset_seq)
) STRICT, WITHOUT ROWID;
CREATE INDEX ix_asset_tag_asset ON asset_tag(asset_seq);

CREATE TABLE dep(
  asset_id BLOB NOT NULL,
  dep_id   BLOB NOT NULL,
  kind     TEXT NOT NULL,
  PRIMARY KEY(asset_id, dep_id, kind)
) STRICT, WITHOUT ROWID;
CREATE INDEX ix_dep_rev ON dep(dep_id);

CREATE TABLE collection(
  id         INTEGER PRIMARY KEY,
  name       TEXT NOT NULL UNIQUE,
  smart      INTEGER NOT NULL DEFAULT 0,
  query      TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE collection_item(
  coll_id  INTEGER NOT NULL,
  asset_id BLOB NOT NULL,
  PRIMARY KEY(coll_id, asset_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX ix_coll_item_asset ON collection_item(asset_id);

CREATE VIRTUAL TABLE asset_fts USING fts5(
  name, description, tags, author,
  tokenize = 'unicode61 remove_diacritics 2',
  prefix = '2 3'
);
"#,
];

/// Aplica as migracoes pendentes de `migrations` a partir do `user_version` atual.
pub(crate) fn migrate(conn: &mut Connection, migrations: &[&str]) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let target = migrations.len() as i64;
    if current > target {
        return Err(AssetError::SchemaTooNew { found: current, supported: target });
    }
    for v in current..target {
        let tx = conn.transaction()?;
        tx.execute_batch(migrations[v as usize])?;
        tx.execute_batch(&format!("PRAGMA user_version = {}", v + 1))?;
        tx.commit()?;
    }
    Ok(())
}

fn apply_pragmas(conn: &Connection, writer: bool) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_millis(10_000))?;
    if writer {
        // `journal_mode` devolve uma linha; consumimos com query_row.
        let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        conn.execute_batch("PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;")?;
    } else {
        conn.execute_batch("PRAGMA query_only = ON;")?;
    }
    conn.execute_batch("PRAGMA mmap_size = 268435456; PRAGMA temp_store = MEMORY; PRAGMA cache_size = -32768;")?;
    Ok(())
}

pub(crate) struct Db {
    path: PathBuf,
    writer: Mutex<Connection>,
    readers: Mutex<Vec<Connection>>,
}

const MAX_POOLED_READERS: usize = 8;

impl Db {
    /// Abre (criando e migrando) o banco em `path`.
    pub(crate) fn open(path: &Path) -> Result<Db> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| AssetError::io(parent, e))?;
        }
        let mut conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        apply_pragmas(&conn, true)?;
        migrate(&mut conn, MIGRATIONS)?;
        Ok(Db { path: path.to_path_buf(), writer: Mutex::new(conn), readers: Mutex::new(Vec::new()) })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// `user_version` atual (para diagnostico).
    pub(crate) fn user_version(&self) -> Result<i64> {
        self.read(|c| Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?))
    }

    /// Executa `f` em uma conexao somente-leitura do pool.
    pub(crate) fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let pooled = self.readers.lock().map_err(poisoned)?.pop();
        let conn = match pooled {
            Some(c) => c,
            None => {
                let c = Connection::open_with_flags(
                    &self.path,
                    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )?;
                apply_pragmas(&c, false)?;
                c
            }
        };
        let r = f(&conn);
        let mut pool = self.readers.lock().map_err(poisoned)?;
        if pool.len() < MAX_POOLED_READERS {
            pool.push(conn);
        }
        r
    }

    /// Executa `f` na conexao escritora (serializada por mutex).
    pub(crate) fn write<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut guard = self.writer.lock().map_err(poisoned)?;
        f(&mut guard)
    }
}

fn poisoned<T>(_: std::sync::PoisonError<T>) -> AssetError {
    AssetError::Internal("mutex do banco envenenado por um panic anterior".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_schema_with_wal_and_fts5() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("index.db")).unwrap();
        assert_eq!(db.user_version().unwrap(), SCHEMA_VERSION);
        let mode: String = db.read(|c| Ok(c.query_row("PRAGMA journal_mode", [], |r| r.get(0))?)).unwrap();
        assert_eq!(mode, "wal");
        db.write(|c| {
            c.execute("INSERT INTO asset_fts(rowid,name,description,tags,author) VALUES (1,'Cabelo Longo','','estilo cel','')", [])?;
            Ok(())
        })
        .unwrap();
        let n: i64 = db
            .read(|c| Ok(c.query_row("SELECT count(*) FROM asset_fts WHERE asset_fts MATCH 'cabel*'", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(n, 1);
        // diacriticos removidos pelo tokenizer
        let n: i64 = db
            .read(|c| Ok(c.query_row("SELECT count(*) FROM asset_fts WHERE asset_fts MATCH 'cabêlo'", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn reopen_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("index.db");
        drop(Db::open(&p).unwrap());
        let db = Db::open(&p).unwrap();
        assert_eq!(db.user_version().unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn newer_schema_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("index.db");
        {
            let c = Connection::open(&p).unwrap();
            c.execute_batch("PRAGMA user_version = 99").unwrap();
        }
        match Db::open(&p) {
            Err(AssetError::SchemaTooNew { found: 99, supported: 1 }) => {}
            Err(e) => panic!("erro inesperado: {e}"),
            Ok(_) => panic!("deveria recusar esquema mais novo"),
        }
    }

    #[test]
    fn migrations_apply_incrementally() {
        let mut conn = Connection::open_in_memory().unwrap();
        let steps: &[&str] = &["CREATE TABLE a(x INTEGER);", "ALTER TABLE a ADD COLUMN y INTEGER DEFAULT 5;"];
        migrate(&mut conn, &steps[..1]).unwrap();
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 1);
        conn.execute("INSERT INTO a(x) VALUES (1)", []).unwrap();
        migrate(&mut conn, steps).unwrap();
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 2);
        let y: i64 = conn.query_row("SELECT y FROM a", [], |r| r.get(0)).unwrap();
        assert_eq!(y, 5);
        // reaplicar nao faz nada
        migrate(&mut conn, steps).unwrap();
        // um banco mais novo que a lista e recusado
        assert!(matches!(migrate(&mut conn, &steps[..1]), Err(AssetError::SchemaTooNew { .. })));
    }

    #[test]
    fn failed_migration_rolls_back() {
        let mut conn = Connection::open_in_memory().unwrap();
        let steps: &[&str] = &["CREATE TABLE a(x INTEGER);", "CREATE TABLE b(x); THIS IS NOT SQL;"];
        assert!(migrate(&mut conn, steps).is_err());
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 1);
        let n: i64 = conn.query_row("SELECT count(*) FROM sqlite_master WHERE name='b'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn readers_see_committed_writes() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("index.db")).unwrap();
        db.write(|c| {
            c.execute("INSERT INTO meta(key,value) VALUES ('k','v')", [])?;
            Ok(())
        })
        .unwrap();
        let v: String = db.read(|c| Ok(c.query_row("SELECT value FROM meta WHERE key='k'", [], |r| r.get(0))?)).unwrap();
        assert_eq!(v, "v");
        // leitoras sao somente-leitura
        assert!(db.read(|c| Ok(c.execute("INSERT INTO meta(key,value) VALUES ('x','y')", [])?)).is_err());
    }
}
