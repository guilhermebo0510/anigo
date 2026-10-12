//! `Library`: abertura, configuracao em disco, raizes (managed/linked) e resolucao de caminhos.
//!
//! Layout em disco (`home` = pasta passada a [`Library::open`]):
//!
//! ```text
//! home/
//!   anigo-library.json        configuracao (library_id, raizes) - verdade das raizes
//!   collections.json          snapshot de colecoes (restaurado se o indice for perdido)
//!   index.db (+ -wal/-shm)    indice SQLite, DESCARTAVEL e reconstruivel (ver `rebuild_index`)
//!   assets/ab/cd/<id>/        ativos MANAGED: asset.json + r<N>/ (arquivos) + materials/
//!   linked/ab/cd/<id>/        manifestos (e materiais derivados) de ativos LINKED; os arquivos ficam fora
//!   trash/<yyyymmdd>/<id>/    lixeira (soft delete); restaurar devolve ao lugar original
//!   cache/                    miniaturas etc. (DESCARTAVEL; pode ser movido via OpenOptions::cache_dir)
//! ```
//!
//! Raizes managed adicionais (`add_root(.., Managed)`) repetem `assets/` e `trash/` dentro da propria pasta.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::{AssetError, IoContext, Result};
use crate::limits::Limits;
use crate::model::*;
use crate::util::{atomic_write, now_secs};

pub(crate) const CONFIG_FILE: &str = "anigo-library.json";
pub(crate) const COLLECTIONS_FILE: &str = "collections.json";
const CONFIG_SCHEMA: u32 = 1;

/// Id da raiz implicita (a propria pasta da biblioteca).
pub const HOME_ROOT: RootId = RootId(1);

/// Opcoes de abertura.
#[derive(Debug, Clone)]
pub struct OpenOptions {
    /// Onde guardar `index.db`. Padrao: dentro de `home`. **Use um disco local** se `home` estiver em rede
    /// (SQLite/WAL em SMB/NFS pode corromper ou travar).
    pub index_dir: Option<PathBuf>,
    /// Onde guardar o cache de miniaturas. Padrao: `home/cache`.
    pub cache_dir: Option<PathBuf>,
    /// Limites de seguranca.
    pub limits: Limits,
    /// Cria a biblioteca se `home` ainda nao for uma. Com `false`, abrir uma pasta sem biblioteca e erro.
    pub create: bool,
    /// Versao do pipeline de derivados (miniaturas). Mudar invalida o cache de miniaturas.
    pub pipeline_ver: u32,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self { index_dir: None, cache_dir: None, limits: Limits::default(), create: true, pipeline_ver: 1 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RootConfig {
    pub id: i64,
    pub uuid: String,
    pub name: String,
    /// Raiz 1: sempre "." (relativa a home, para a pasta poder ser movida). Demais: caminho absoluto.
    pub path: String,
    pub mode: RootMode,
    #[serde(default)]
    pub watch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LibraryConfig {
    pub schema: u32,
    pub library_id: String,
    pub name: String,
    pub created_at: i64,
    pub next_root_id: i64,
    pub roots: Vec<RootConfig>,
}

/// Descricao publica de uma raiz.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RootInfo {
    pub id: RootId,
    pub uuid: String,
    pub name: String,
    pub path: PathBuf,
    pub mode: RootMode,
    /// `false` quando a pasta nao esta acessivel (HD externo/NAS desconectado).
    pub online: bool,
    pub watch: bool,
    pub last_scan: Option<i64>,
    pub asset_count: u64,
}

pub(crate) struct Inner {
    pub home: PathBuf,
    pub opts: OpenOptions,
    pub db: Db,
    pub config: Mutex<LibraryConfig>,
    pub cache_dir: PathBuf,
    /// Serializa read-modify-write de manifestos (evita perder edicoes concorrentes).
    pub mutate_lock: Mutex<()>,
    pub thumbs: std::sync::OnceLock<Arc<crate::thumbs::ThumbnailCache>>,
}

/// Handle da biblioteca: barato de clonar, `Send + Sync`. Toda a API publica esta aqui.
#[derive(Clone)]
pub struct Library {
    pub(crate) inner: Arc<Inner>,
}

impl std::fmt::Debug for Library {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Library").field("home", &self.inner.home).finish()
    }
}

impl Library {
    /// Abre (ou cria) a biblioteca em `home` com opcoes padrao.
    pub fn open(home: impl AsRef<Path>) -> Result<Library> {
        Self::open_with(home, OpenOptions::default())
    }

    /// Abre (ou cria) a biblioteca em `home`.
    ///
    /// Se o indice nao existir (ou foi criado para outra biblioteca) e houver manifestos em disco, o indice
    /// e reconstruido automaticamente.
    pub fn open_with(home: impl AsRef<Path>, opts: OpenOptions) -> Result<Library> {
        let home = home.as_ref();
        let config_path = home.join(CONFIG_FILE);
        if !config_path.exists() {
            if !opts.create {
                return Err(AssetError::NotFound(format!("{} nao e uma biblioteca ANIGO", home.display())));
            }
            std::fs::create_dir_all(home).at(home)?;
        }
        let home = home.canonicalize().at(home)?;
        let config = if config_path.exists() {
            let bytes = std::fs::read(&config_path).at(&config_path)?;
            if bytes.len() > 4 * 1024 * 1024 {
                return Err(AssetError::Corrupt("anigo-library.json grande demais".into()));
            }
            let c: LibraryConfig = serde_json::from_slice(&bytes)
                .map_err(|e| AssetError::Corrupt(format!("anigo-library.json ilegivel: {e}")))?;
            if c.schema == 0 || c.schema > CONFIG_SCHEMA {
                return Err(AssetError::Corrupt(format!("schema de configuracao {} nao suportado", c.schema)));
            }
            c
        } else {
            let id = uuid::Uuid::now_v7().to_string();
            let c = LibraryConfig {
                schema: CONFIG_SCHEMA,
                library_id: id.clone(),
                name: home.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Biblioteca".into()),
                created_at: now_secs(),
                next_root_id: 2,
                roots: vec![RootConfig { id: 1, uuid: id, name: "Biblioteca".into(), path: ".".into(), mode: RootMode::Managed, watch: false }],
            };
            atomic_write(&config_path, &serde_json::to_vec_pretty(&c)?)?;
            c
        };
        if !config.roots.iter().any(|r| r.id == 1) {
            return Err(AssetError::Corrupt("configuracao sem a raiz 1".into()));
        }
        let index_dir = opts.index_dir.clone().unwrap_or_else(|| home.clone());
        let db_path = index_dir.join("index.db");
        let db = Db::open(&db_path)?;
        let cache_dir = opts.cache_dir.clone().unwrap_or_else(|| home.join("cache"));
        let lib = Library { inner: Arc::new(Inner { home, opts, db, config: Mutex::new(config), cache_dir, mutate_lock: Mutex::new(()), thumbs: std::sync::OnceLock::new() }) };
        lib.sync_roots_to_db()?;
        let known: Option<String> = lib.inner.db.read(|c| {
            Ok(rusqlite::OptionalExtension::optional(c.query_row("SELECT value FROM meta WHERE key='library_id'", [], |r| r.get::<_, String>(0)))?)
        })?;
        let lid = lib.library_id();
        if known.as_deref() != Some(lid.as_str()) {
            // Indice novo (ou de outra biblioteca): reconstruir a partir dos manifestos, se houver.
            lib.inner.db.write(|c| {
                c.execute("INSERT OR REPLACE INTO meta(key,value) VALUES ('library_id', ?1)", [&lid])?;
                Ok(())
            })?;
            lib.rebuild_index()?;
        }
        Ok(lib)
    }

    /// Pasta da biblioteca.
    pub fn home(&self) -> &Path {
        &self.inner.home
    }
    /// Pasta do cache descartavel (miniaturas).
    pub fn cache_dir(&self) -> &Path {
        &self.inner.cache_dir
    }
    /// Caminho do `index.db`.
    pub fn index_path(&self) -> &Path {
        self.inner.db.path()
    }
    /// UUID da biblioteca.
    pub fn library_id(&self) -> String {
        self.inner.config.lock().map(|c| c.library_id.clone()).unwrap_or_default()
    }
    pub fn limits(&self) -> &Limits {
        &self.inner.opts.limits
    }
    /// Versao do pipeline de derivados configurada.
    pub fn pipeline_ver(&self) -> u32 {
        self.inner.opts.pipeline_ver
    }
    /// `user_version` do esquema do indice.
    pub fn schema_version(&self) -> Result<i64> {
        self.inner.db.user_version()
    }

    pub(crate) fn config_snapshot(&self) -> Result<LibraryConfig> {
        Ok(self.inner.config.lock().map_err(|_| AssetError::Internal("config envenenada".into()))?.clone())
    }

    pub(crate) fn save_config(&self, cfg: &LibraryConfig) -> Result<()> {
        atomic_write(&self.inner.home.join(CONFIG_FILE), &serde_json::to_vec_pretty(cfg)?)
    }

    fn abs_root_path(&self, rc: &RootConfig) -> PathBuf {
        if rc.id == 1 {
            self.inner.home.clone()
        } else {
            PathBuf::from(&rc.path)
        }
    }

    /// Sincroniza a tabela `root` com `anigo-library.json` e atualiza o estado online.
    fn sync_roots_to_db(&self) -> Result<()> {
        let cfg = self.config_snapshot()?;
        self.inner.db.write(|c| {
            let tx = c.transaction()?;
            let ids: Vec<i64> = cfg.roots.iter().map(|r| r.id).collect();
            for rc in &cfg.roots {
                let path = self.abs_root_path(rc);
                let online = path.is_dir();
                tx.execute(
                    "INSERT INTO root(id, uuid, name, path, mode, online, watch) VALUES (?1,?2,?3,?4,?5,?6,?7) \
                     ON CONFLICT(id) DO UPDATE SET uuid=excluded.uuid, name=excluded.name, path=excluded.path, \
                     mode=excluded.mode, online=excluded.online, watch=excluded.watch",
                    params![rc.id, rc.uuid, rc.name, path.to_string_lossy(), rc.mode.to_db(), online as i64, rc.watch as i64],
                )?;
            }
            // raizes que sumiram da configuracao
            let mut stale = Vec::new();
            {
                let mut st = tx.prepare("SELECT id FROM root")?;
                for r in st.query_map([], |r| r.get::<_, i64>(0))? {
                    let id = r?;
                    if !ids.contains(&id) {
                        stale.push(id);
                    }
                }
            }
            for id in stale {
                tx.execute("DELETE FROM root WHERE id = ?1", [id])?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// Reavalia quais raizes estao acessiveis (chame antes de scans/verify, ou ao detectar montagem de disco).
    pub fn refresh_roots(&self) -> Result<Vec<RootInfo>> {
        self.sync_roots_to_db()?;
        self.roots()
    }

    /// Lista as raizes registradas.
    pub fn roots(&self) -> Result<Vec<RootInfo>> {
        self.inner.db.read(|c| {
            let mut st = c.prepare(
                "SELECT r.id, r.uuid, r.name, r.path, r.mode, r.online, r.watch, r.last_scan, \
                 (SELECT count(*) FROM asset a WHERE a.root_id = r.id AND a.state != 1) FROM root r ORDER BY r.id",
            )?;
            let rows = st.query_map([], |r| {
                Ok(RootInfo {
                    id: RootId(r.get(0)?),
                    uuid: r.get(1)?,
                    name: r.get(2)?,
                    path: PathBuf::from(r.get::<_, String>(3)?),
                    mode: RootMode::from_db(r.get(4)?),
                    online: r.get::<_, i64>(5)? != 0,
                    watch: r.get::<_, i64>(6)? != 0,
                    last_scan: r.get(7)?,
                    asset_count: r.get::<_, i64>(8)?.max(0) as u64,
                })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Informacoes de uma raiz.
    pub fn root(&self, id: RootId) -> Result<RootInfo> {
        self.roots()?.into_iter().find(|r| r.id == id).ok_or_else(|| AssetError::NotFound(format!("raiz {id}")))
    }

    /// Registra uma pasta como raiz.
    ///
    /// * `Linked`: a pasta deve existir; a biblioteca **nunca** altera nem apaga nada nela. Use [`Library::scan`]
    ///   para descobrir os ativos.
    /// * `Managed`: a pasta e criada se preciso e recebe copias gerenciadas (`import_to`). Se ja contiver
    ///   manifestos de uma biblioteca anterior, eles sao indexados.
    ///
    /// Raizes nao podem se sobrepor entre si nem com a pasta da biblioteca.
    pub fn add_root(&self, path: impl AsRef<Path>, mode: RootMode, name: Option<&str>) -> Result<RootId> {
        let path = path.as_ref();
        match mode {
            RootMode::Linked => {
                if !path.is_dir() {
                    return Err(AssetError::NotFound(format!("pasta {} nao existe", path.display())));
                }
            }
            RootMode::Managed => std::fs::create_dir_all(path).at(path)?,
        }
        let canon = path.canonicalize().at(path)?;
        let home = &self.inner.home;
        if canon.starts_with(home) || home.starts_with(&canon) {
            return Err(AssetError::Conflict("a raiz nao pode conter nem estar dentro da pasta da biblioteca".into()));
        }
        let mut cfg = self.config_snapshot()?;
        for r in cfg.roots.iter().filter(|r| r.id != 1) {
            let other = PathBuf::from(&r.path);
            if canon.starts_with(&other) || other.starts_with(&canon) {
                return Err(AssetError::Conflict(format!("sobrepoe a raiz existente {} ({})", r.id, other.display())));
            }
        }
        let id = cfg.next_root_id;
        cfg.next_root_id += 1;
        let display_name = name
            .map(str::to_string)
            .or_else(|| canon.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| format!("raiz {id}"));
        cfg.roots.push(RootConfig {
            id,
            uuid: uuid::Uuid::now_v7().to_string(),
            name: display_name,
            path: canon.to_string_lossy().to_string(),
            mode,
            watch: false,
        });
        self.save_config(&cfg)?;
        {
            let mut g = self.inner.config.lock().map_err(|_| AssetError::Internal("config envenenada".into()))?;
            *g = cfg;
        }
        self.sync_roots_to_db()?;
        if mode == RootMode::Managed {
            self.index_managed_root(RootId(id))?;
        }
        Ok(RootId(id))
    }

    /// Remove uma raiz do catalogo.
    ///
    /// * `Linked`: esquece os ativos (apaga so os manifestos dentro de `home/linked`); os arquivos externos
    ///   nao sao tocados.
    /// * `Managed`: esquece os ativos do indice; os arquivos em disco ficam (readicionar a pasta os recupera),
    ///   a menos que `delete_files` seja `true`.
    ///
    /// A raiz 1 nao pode ser removida. Devolve quantos ativos foram esquecidos.
    pub fn remove_root(&self, id: RootId, delete_files: bool) -> Result<u64> {
        if id == HOME_ROOT {
            return Err(AssetError::InvalidInput("a raiz da biblioteca nao pode ser removida".into()));
        }
        let info = self.root(id)?;
        let rows: Vec<(i64, Vec<u8>, String, i64)> = self.inner.db.read(|c| {
            let mut st = c.prepare("SELECT seq, id, manifest_dir, storage FROM asset WHERE root_id = ?1")?;
            let v = st.query_map([id.0], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            Ok(v.collect::<std::result::Result<Vec<_>, _>>()?)
        })?;
        for (_, _, dir, storage) in &rows {
            let abs = match StorageMode::from_db(*storage) {
                StorageMode::Linked => self.inner.home.join(dir),
                StorageMode::Managed => info.path.join(dir),
            };
            let is_linked = StorageMode::from_db(*storage) == StorageMode::Linked;
            if is_linked || delete_files {
                let _ = std::fs::remove_dir_all(&abs);
            }
        }
        let n = rows.len() as u64;
        self.inner.db.write(|c| {
            let tx = c.transaction()?;
            for (seq, idb, _, _) in &rows {
                if let Some(aid) = AssetId::from_bytes(idb) {
                    crate::store::delete_asset_rows(&tx, *seq, &aid)?;
                    tx.execute("DELETE FROM collection_item WHERE asset_id = ?1", [idb.as_slice()])?;
                }
            }
            tx.commit()?;
            Ok(())
        })?;
        let mut cfg = self.config_snapshot()?;
        cfg.roots.retain(|r| r.id != id.0);
        self.save_config(&cfg)?;
        {
            let mut g = self.inner.config.lock().map_err(|_| AssetError::Internal("config envenenada".into()))?;
            *g = cfg;
        }
        self.sync_roots_to_db()?;
        self.write_collections_snapshot()?;
        Ok(n)
    }

    /// Caminho absoluto de uma raiz (da configuracao).
    pub(crate) fn root_abs_path(&self, id: i64) -> Result<PathBuf> {
        let cfg = self.inner.config.lock().map_err(|_| AssetError::Internal("config envenenada".into()))?;
        cfg.roots
            .iter()
            .find(|r| r.id == id)
            .map(|r| self.abs_root_path(r))
            .ok_or_else(|| AssetError::NotFound(format!("raiz {id}")))
    }

    pub(crate) fn root_uuid(&self, id: i64) -> Result<String> {
        let cfg = self.inner.config.lock().map_err(|_| AssetError::Internal("config envenenada".into()))?;
        cfg.roots.iter().find(|r| r.id == id).map(|r| r.uuid.clone()).ok_or_else(|| AssetError::NotFound(format!("raiz {id}")))
    }

    pub(crate) fn root_id_by_uuid(&self, uuid: &str) -> Option<i64> {
        let cfg = self.inner.config.lock().ok()?;
        cfg.roots.iter().find(|r| r.uuid == uuid).map(|r| r.id)
    }

    /// Base onde vivem os manifestos de um ativo: raiz managed dele, ou `home` para linked.
    pub(crate) fn manifest_base(&self, storage: StorageMode, root_id: i64) -> Result<PathBuf> {
        match storage {
            StorageMode::Managed => self.root_abs_path(root_id),
            StorageMode::Linked => Ok(self.inner.home.clone()),
        }
    }
}

/// Funcoes de layout (publicas para ferramentas e para o benchmark).
pub mod layout {
    use super::AssetId;

    /// `assets/ab/cd/<uuid>` relativo a uma raiz managed.
    pub fn managed_dir_rel(id: &AssetId) -> String {
        let (a, b) = id.shard();
        format!("assets/{a}/{b}/{id}")
    }
    /// `linked/ab/cd/<uuid>` relativo a `home`.
    pub fn linked_dir_rel(id: &AssetId) -> String {
        let (a, b) = id.shard();
        format!("linked/{a}/{b}/{id}")
    }
    /// `trash/<yyyymmdd>/<uuid>`.
    pub fn trash_dir_rel(date: &str, id: &AssetId) -> String {
        format!("trash/{date}/{id}")
    }
    /// Pasta de uma revisao dentro da pasta do ativo.
    pub fn revision_dir(rev: u32) -> String {
        format!("r{rev}")
    }
}

/// Le `asset.json` de `dir`.
pub(crate) fn read_manifest(dir: &Path) -> Result<AssetManifest> {
    let p = dir.join(MANIFEST_FILE);
    let meta = std::fs::metadata(&p).at(&p)?;
    if meta.len() > 16 * 1024 * 1024 {
        return Err(AssetError::LimitExceeded { what: "asset.json", limit: 16 * 1024 * 1024, actual: meta.len() });
    }
    let bytes = std::fs::read(&p).at(&p)?;
    AssetManifest::from_slice(&bytes)
}

/// Grava `asset.json` atomicamente.
pub(crate) fn write_manifest(dir: &Path, m: &AssetManifest) -> Result<()> {
    atomic_write(&dir.join(MANIFEST_FILE), &m.to_json_pretty()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_layout_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("lib");
        let lib = Library::open(&home).unwrap();
        assert!(home.join(CONFIG_FILE).exists());
        assert!(lib.index_path().exists());
        let id = lib.library_id();
        assert!(!id.is_empty());
        let roots = lib.roots().unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].id, HOME_ROOT);
        assert!(roots[0].online);
        drop(lib);
        let lib2 = Library::open(&home).unwrap();
        assert_eq!(lib2.library_id(), id);
        assert_eq!(lib2.schema_version().unwrap(), crate::db::SCHEMA_VERSION);
    }

    #[test]
    fn open_without_create_fails_on_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let r = Library::open_with(dir.path(), OpenOptions { create: false, ..Default::default() });
        assert!(matches!(r, Err(AssetError::NotFound(_))));
    }

    #[test]
    fn corrupt_config_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(CONFIG_FILE), b"{ not json").unwrap();
        assert!(matches!(Library::open(dir.path()), Err(AssetError::Corrupt(_))));
        std::fs::write(dir.path().join(CONFIG_FILE), b"{\"schema\":1,\"library_id\":\"x\",\"name\":\"n\",\"created_at\":0,\"next_root_id\":2,\"roots\":[]}").unwrap();
        assert!(matches!(Library::open(dir.path()), Err(AssetError::Corrupt(_))));
    }

    #[test]
    fn add_remove_linked_root_and_overlap_rules() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path().join("lib")).unwrap();
        let ext = tempfile::tempdir().unwrap();
        let a = ext.path().join("a");
        std::fs::create_dir_all(a.join("inner")).unwrap();
        let rid = lib.add_root(&a, RootMode::Linked, Some("Meus modelos")).unwrap();
        assert_eq!(lib.root(rid).unwrap().name, "Meus modelos");
        assert_eq!(lib.root(rid).unwrap().mode, RootMode::Linked);
        // sobreposicao (dentro, fora) e duplicata
        assert!(matches!(lib.add_root(a.join("inner"), RootMode::Linked, None), Err(AssetError::Conflict(_))));
        assert!(matches!(lib.add_root(ext.path(), RootMode::Linked, None), Err(AssetError::Conflict(_))));
        assert!(matches!(lib.add_root(&a, RootMode::Linked, None), Err(AssetError::Conflict(_))));
        // dentro da propria biblioteca
        assert!(matches!(lib.add_root(lib.home().join("assets"), RootMode::Managed, None), Err(AssetError::Conflict(_))));
        // inexistente
        assert!(matches!(lib.add_root(ext.path().join("nope"), RootMode::Linked, None), Err(AssetError::NotFound(_))));
        assert_eq!(lib.roots().unwrap().len(), 2);
        assert_eq!(lib.remove_root(rid, false).unwrap(), 0);
        assert_eq!(lib.roots().unwrap().len(), 1);
        assert!(a.exists(), "remover raiz linked nunca apaga a pasta externa");
        assert!(lib.remove_root(HOME_ROOT, false).is_err());
    }

    #[test]
    fn roots_persist_and_offline_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("lib");
        let ext = tempfile::tempdir().unwrap();
        let a = ext.path().join("a");
        std::fs::create_dir_all(&a).unwrap();
        let rid;
        {
            let lib = Library::open(&home).unwrap();
            rid = lib.add_root(&a, RootMode::Linked, None).unwrap();
        }
        std::fs::remove_dir_all(&a).unwrap();
        let lib = Library::open(&home).unwrap();
        let r = lib.root(rid).unwrap();
        assert!(!r.online, "pasta sumiu: raiz offline");
        std::fs::create_dir_all(&a).unwrap();
        let roots = lib.refresh_roots().unwrap();
        assert!(roots.iter().find(|r| r.id == rid).unwrap().online);
    }

    #[test]
    fn layout_paths() {
        let id = AssetId::new();
        let m = layout::managed_dir_rel(&id);
        assert!(m.starts_with("assets/") && m.ends_with(&id.to_string()));
        assert_eq!(m.split('/').count(), 4);
        assert!(layout::linked_dir_rel(&id).starts_with("linked/"));
        assert_eq!(layout::trash_dir_rel("20261012", &id), format!("trash/20261012/{id}"));
        assert_eq!(layout::revision_dir(3), "r3");
    }
}
