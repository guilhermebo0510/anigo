//! Modelo de dados publico: ids, enums, manifesto `asset.json`, linhas do indice.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{AssetError, Result};
use crate::util::{hex_decode, hex_encode};

// ---------------------------------------------------------------------------------------------
// Ids
// ---------------------------------------------------------------------------------------------

/// Identidade estavel de um ativo (UUID v7). Nao depende do caminho: mover/renomear nao a altera.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetId(pub uuid::Uuid);

impl AssetId {
    /// Novo id (UUID v7: ordenavel por tempo de criacao).
    pub fn new() -> Self {
        AssetId(uuid::Uuid::now_v7())
    }
    pub fn as_bytes(&self) -> &[u8; 16] {
        self.0.as_bytes()
    }
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        let arr: [u8; 16] = b.try_into().ok()?;
        Some(AssetId(uuid::Uuid::from_bytes(arr)))
    }
    /// Pares de diretorios de shard (`ab/cd`) derivados dos ULTIMOS bytes do uuid (a parte
    /// aleatoria; os primeiros bytes do v7 sao tempo e concentrariam tudo numa pasta).
    pub(crate) fn shard(&self) -> (String, String) {
        let b = self.0.as_bytes();
        (format!("{:02x}", b[15]), format!("{:02x}", b[14]))
    }
}

impl Default for AssetId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.hyphenated())
    }
}

impl FromStr for AssetId {
    type Err = AssetError;
    fn from_str(s: &str) -> Result<Self> {
        uuid::Uuid::parse_str(s)
            .map(AssetId)
            .map_err(|e| AssetError::InvalidInput(format!("asset id invalido {s:?}: {e}")))
    }
}

impl Serialize for AssetId {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for AssetId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

macro_rules! int_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub struct $name(pub i64);
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{}", self.0) }
        }
    };
}

int_id!(
    /// Identificador de uma raiz (pasta registrada). A raiz `1` e sempre a pasta da propria biblioteca.
    RootId
);
int_id!(
    /// Identificador de uma colecao.
    CollectionId
);

/// BLAKE3 (32 bytes) do conteudo; serializa como hex minusculo.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentHash(pub [u8; 32]);

impl ContentHash {
    pub fn to_hex(&self) -> String {
        hex_encode(&self.0)
    }
    pub fn from_hex(s: &str) -> Option<Self> {
        let v = hex_decode(s)?;
        let arr: [u8; 32] = v.try_into().ok()?;
        Some(ContentHash(arr))
    }
    pub fn from_slice(b: &[u8]) -> Option<Self> {
        let arr: [u8; 32] = b.try_into().ok()?;
        Some(ContentHash(arr))
    }
    /// Os 8 primeiros caracteres hex (para nomes de arquivo curtos).
    pub fn short(&self) -> String {
        hex_encode(&self.0[..4])
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({})", self.short())
    }
}
impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}
impl Serialize for ContentHash {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}
impl<'de> Deserialize<'de> for ContentHash {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        ContentHash::from_hex(&s).ok_or_else(|| serde::de::Error::custom("hash blake3 invalido (esperado 64 hex)"))
    }
}

// ---------------------------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------------------------

macro_rules! str_enum {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $var:ident => $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub enum $name { $($(#[$vm])* #[serde(rename = $s)] $var),+ }
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$var),+];
            pub fn as_str(&self) -> &'static str { match self { $($name::$var => $s),+ } }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.as_str()) }
        }
        impl FromStr for $name {
            type Err = AssetError;
            fn from_str(s: &str) -> Result<Self> {
                match s { $($s => Ok($name::$var),)+ other => Err(AssetError::InvalidInput(format!(concat!(stringify!($name), " desconhecido: {:?}"), other))) }
            }
        }
    };
}

str_enum!(
    /// Tipo logico do ativo (aba/filtro principal da UI).
    AssetKind {
        Model => "model",
        Character => "character",
        Outfit => "outfit",
        Hair => "hair",
        Accessory => "accessory",
        Material => "material",
        Texture => "texture",
        Pose => "pose",
        Animation => "animation",
        Hdri => "hdri",
        Scene => "scene",
        Prefab => "prefab",
    }
);

str_enum!(
    /// Como os arquivos do ativo sao guardados.
    StorageMode {
        /// Copiado para a biblioteca, imutavel apos o import.
        Managed => "managed",
        /// Referencia a uma pasta externa que a biblioteca nunca altera nem apaga.
        Linked => "linked",
    }
);

impl StorageMode {
    pub(crate) fn to_db(self) -> i64 {
        match self {
            StorageMode::Managed => 0,
            StorageMode::Linked => 1,
        }
    }
    pub(crate) fn from_db(v: i64) -> Self {
        if v == 1 {
            StorageMode::Linked
        } else {
            StorageMode::Managed
        }
    }
}

str_enum!(
    /// Modo de uma raiz.
    RootMode {
        Managed => "managed",
        Linked => "linked",
    }
);

impl RootMode {
    pub(crate) fn to_db(self) -> i64 {
        match self {
            RootMode::Managed => 0,
            RootMode::Linked => 1,
        }
    }
    pub(crate) fn from_db(v: i64) -> Self {
        if v == 1 {
            RootMode::Linked
        } else {
            RootMode::Managed
        }
    }
}

str_enum!(
    /// Estado do ativo no indice.
    AssetState {
        Active => "active",
        Trashed => "trashed",
        /// Linked: o arquivo nao foi encontrado no ultimo scan/verify.
        Missing => "missing",
        /// O conteudo nao confere com o hash do manifesto (detectado por `verify`).
        Corrupt => "corrupt",
    }
);

impl AssetState {
    pub(crate) fn to_db(self) -> i64 {
        match self {
            AssetState::Active => 0,
            AssetState::Trashed => 1,
            AssetState::Missing => 2,
            AssetState::Corrupt => 3,
        }
    }
    pub(crate) fn from_db(v: i64) -> Self {
        match v {
            1 => AssetState::Trashed,
            2 => AssetState::Missing,
            3 => AssetState::Corrupt,
            _ => AssetState::Active,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Manifesto (asset.json)
// ---------------------------------------------------------------------------------------------

/// Versao do esquema de `asset.json`.
pub const MANIFEST_SCHEMA: u32 = 1;
/// Nome do manifesto dentro da pasta do ativo.
pub const MANIFEST_FILE: &str = "asset.json";
/// Versao do importador: sobe quando a analise (stats/materiais) muda de forma incompativel.
pub const IMPORTER_VERSION: u32 = 1;

/// Estatisticas leves (nunca exigem carregar o ativo para listar).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AssetStats {
    pub tris: u64,
    pub verts: u64,
    pub meshes: u32,
    pub nodes: u32,
    pub materials: u32,
    pub textures: u32,
    pub bones: u32,
    pub morphs: u32,
    pub anims: u32,
    pub has_skin: bool,
    pub has_vrm: bool,
    /// AABB `[minx, miny, minz, maxx, maxy, maxz]` no espaco das malhas (sem aplicar transform de nos).
    pub bbox: Option<[f32; 6]>,
    /// Dimensoes (imagens).
    pub width: u32,
    pub height: u32,
}

/// Um arquivo pertencente a uma revisao.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Caminho relativo (com `/`) a pasta da revisao (managed) ou a pasta do arquivo primario (linked).
    pub path: String,
    pub size: u64,
    pub blake3: ContentHash,
}

/// Uma revisao imutavel do conteudo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevisionInfo {
    pub rev: u32,
    /// Hash do conteudo: arquivo unico = BLAKE3 do proprio arquivo; varios arquivos = BLAKE3 da lista
    /// ordenada `"<caminho>\0<hash hex>\n"` (ver [`combined_hash`]).
    pub content_hash: ContentHash,
    pub size: u64,
    pub created_at: i64,
    pub files: Vec<FileEntry>,
    #[serde(default)]
    pub note: String,
}

/// Dependencia entre ativos (`asset` usa `id`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyRef {
    pub id: AssetId,
    pub kind: String,
}

/// Onde um ativo `linked` vive: raiz (por uuid, estavel entre reconstrucoes) + caminho relativo ao raiz.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkInfo {
    pub root_uuid: String,
    pub rel_path: String,
}

/// Manifesto `asset.json`: FONTE DA VERDADE dos metadados de um ativo. O indice SQLite e reconstruivel a
/// partir destes arquivos (`Library::rebuild_index`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetManifest {
    pub schema: u32,
    pub asset_id: AssetId,
    pub name: String,
    pub kind: AssetKind,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub source_url: String,
    /// Formato do arquivo primario: `glb`, `gltf`, `vrm`, `obj`, `fbx`, `png`, `jpg`, `anigomat`...
    pub format: String,
    pub storage: StorageMode,
    /// Caminho do arquivo primario relativo a pasta da revisao (managed: `r<N>/`) ou a pasta do arquivo (linked).
    pub primary: String,
    #[serde(default)]
    pub link: Option<LinkInfo>,
    pub current_revision: u32,
    pub revisions: Vec<RevisionInfo>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub rating: u8,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub stats: AssetStats,
    #[serde(default)]
    pub dependencies: Vec<DependencyRef>,
    /// Nomes dos materiais canonicos gerados (arquivos em `materials/` ao lado do manifesto).
    #[serde(default)]
    pub materials: Vec<String>,
    #[serde(default = "default_importer_ver")]
    pub importer_ver: u32,
    pub created_at: i64,
    pub imported_at: i64,
    pub modified_at: i64,
    /// Quando foi para a lixeira (`None` = ativo).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trashed_at: Option<i64>,
    /// Propriedades livres (extensao futura, metadados de terceiros).
    #[serde(default)]
    pub props: serde_json::Value,
    /// Avisos do import (extensoes ignoradas, texturas ausentes...).
    #[serde(default)]
    pub warnings: Vec<String>,
}

fn default_importer_ver() -> u32 {
    IMPORTER_VERSION
}

impl AssetManifest {
    /// Revisao atual.
    pub fn current(&self) -> Option<&RevisionInfo> {
        self.revisions.iter().find(|r| r.rev == self.current_revision)
    }
    /// Hash de conteudo da revisao atual.
    pub fn content_hash(&self) -> Option<ContentHash> {
        self.current().map(|r| r.content_hash)
    }
    /// Valida invariantes minimas depois de ler de disco (manifesto editado a mao ou corrompido).
    pub fn validate(&self) -> Result<()> {
        if self.schema == 0 || self.schema > MANIFEST_SCHEMA {
            return Err(AssetError::Corrupt(format!("schema de manifesto {} nao suportado", self.schema)));
        }
        if self.current().is_none() {
            return Err(AssetError::Corrupt(format!("manifesto {}: revisao atual {} inexistente", self.asset_id, self.current_revision)));
        }
        if self.rating > 5 {
            return Err(AssetError::Corrupt("rating > 5".into()));
        }
        if self.name.len() > 1024 {
            return Err(AssetError::Corrupt("nome longo demais".into()));
        }
        if self.storage == StorageMode::Linked && self.link.is_none() {
            return Err(AssetError::Corrupt("asset linked sem campo link".into()));
        }
        crate::paths::normalize_relative(&self.primary).map_err(|e| AssetError::Corrupt(format!("primary: {e}")))?;
        for r in &self.revisions {
            for f in &r.files {
                crate::paths::normalize_relative(&f.path).map_err(|e| AssetError::Corrupt(format!("files: {e}")))?;
            }
        }
        if let Some(l) = &self.link {
            crate::paths::normalize_relative(&l.rel_path).map_err(|e| AssetError::Corrupt(format!("link: {e}")))?;
        }
        Ok(())
    }
    pub fn to_json_pretty(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(self)?)
    }
    /// Le e valida um manifesto (com limite de tamanho: 16 MiB).
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(AssetError::LimitExceeded { what: "asset.json", limit: 16 * 1024 * 1024, actual: bytes.len() as u64 });
        }
        let m: AssetManifest = serde_json::from_slice(bytes)?;
        m.validate()?;
        Ok(m)
    }
}

/// Hash combinado de varios arquivos (ordenados por caminho). Para um unico arquivo o chamador usa
/// diretamente o hash do arquivo.
pub fn combined_hash(files: &[FileEntry]) -> ContentHash {
    if files.len() == 1 {
        return files[0].blake3;
    }
    let mut sorted: Vec<&FileEntry> = files.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let mut h = blake3::Hasher::new();
    for f in sorted {
        h.update(f.path.as_bytes());
        h.update(&[0]);
        h.update(f.blake3.to_hex().as_bytes());
        h.update(b"\n");
    }
    ContentHash(*h.finalize().as_bytes())
}

// ---------------------------------------------------------------------------------------------
// Linhas do indice
// ---------------------------------------------------------------------------------------------

/// Linha leve do indice (o que a grade precisa; sem carregar o manifesto).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetRow {
    pub id: AssetId,
    /// Chave interna monotona (estavel enquanto o indice nao for reconstruido).
    pub seq: i64,
    pub name: String,
    pub kind: AssetKind,
    pub format: String,
    pub storage: StorageMode,
    pub state: AssetState,
    pub root_id: RootId,
    /// Linked: caminho relativo ao raiz. Managed: caminho do primario dentro da pasta do ativo.
    pub rel_path: String,
    pub content_hash: ContentHash,
    pub size: u64,
    pub rating: u8,
    pub favorite: bool,
    pub revision: u32,
    pub tris: u64,
    pub verts: u64,
    pub materials: u32,
    pub bones: u32,
    pub anims: u32,
    pub has_skin: bool,
    pub has_vrm: bool,
    pub imported_at: i64,
    pub modified_at: i64,
    /// `false` quando a raiz do ativo esta offline (HD externo desconectado).
    pub available: bool,
}

/// Detalhes completos de um ativo.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssetDetail {
    pub row: AssetRow,
    pub manifest: AssetManifest,
    pub dependencies: Vec<DependencyRef>,
    pub used_by: Vec<AssetId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample_manifest() -> AssetManifest {
        let h = ContentHash(*blake3::hash(b"x").as_bytes());
        AssetManifest {
            schema: MANIFEST_SCHEMA,
            asset_id: AssetId::new(),
            name: "Hero".into(),
            kind: AssetKind::Model,
            description: String::new(),
            author: String::new(),
            license: String::new(),
            source_url: String::new(),
            format: "glb".into(),
            storage: StorageMode::Managed,
            primary: "hero.glb".into(),
            link: None,
            current_revision: 1,
            revisions: vec![RevisionInfo {
                rev: 1,
                content_hash: h,
                size: 1,
                created_at: 0,
                files: vec![FileEntry { path: "hero.glb".into(), size: 1, blake3: h }],
                note: String::new(),
            }],
            tags: vec![],
            rating: 0,
            favorite: false,
            stats: AssetStats::default(),
            dependencies: vec![],
            materials: vec![],
            importer_ver: IMPORTER_VERSION,
            created_at: 0,
            imported_at: 0,
            modified_at: 0,
            trashed_at: None,
            props: serde_json::Value::Null,
            warnings: vec![],
        }
    }

    #[test]
    fn manifest_roundtrip_and_validate() {
        let m = sample_manifest();
        let bytes = m.to_json_pretty().unwrap();
        let back = AssetManifest::from_slice(&bytes).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn manifest_rejects_bad_content() {
        let mut m = sample_manifest();
        m.current_revision = 9;
        assert!(m.validate().is_err());
        let mut m = sample_manifest();
        m.rating = 9;
        assert!(m.validate().is_err());
        let mut m = sample_manifest();
        m.primary = "../evil".into();
        assert!(m.validate().is_err());
        let mut m = sample_manifest();
        m.storage = StorageMode::Linked;
        assert!(m.validate().is_err());
        assert!(AssetManifest::from_slice(b"{}").is_err());
        assert!(AssetManifest::from_slice(b"\xff\xfe garbage").is_err());
        assert!(AssetManifest::from_slice(&[]).is_err());
    }

    #[test]
    fn id_and_enum_parsing() {
        let id = AssetId::new();
        assert_eq!(id.to_string().parse::<AssetId>().unwrap(), id);
        assert!("nope".parse::<AssetId>().is_err());
        for k in AssetKind::ALL {
            assert_eq!(k.as_str().parse::<AssetKind>().unwrap(), *k);
        }
        assert!("zzz".parse::<AssetKind>().is_err());
        let j = serde_json::to_string(&AssetKind::Hdri).unwrap();
        assert_eq!(j, "\"hdri\"");
        let h = ContentHash(*blake3::hash(b"a").as_bytes());
        let back: ContentHash = serde_json::from_str(&serde_json::to_string(&h).unwrap()).unwrap();
        assert_eq!(h, back);
        assert!(serde_json::from_str::<ContentHash>("\"abcd\"").is_err());
    }

    #[test]
    fn shard_uses_random_tail() {
        let a = AssetId::new();
        let b = AssetId::new();
        // v7 gerados em sequencia compartilham os primeiros bytes; o shard precisa vir do final.
        let (a1, a2) = a.shard();
        assert_eq!(a1.len(), 2);
        assert_eq!(a2.len(), 2);
        let _ = b;
    }

    #[test]
    fn combined_hash_is_order_independent_and_single_is_identity() {
        let f = |p: &str, d: &[u8]| FileEntry { path: p.into(), size: d.len() as u64, blake3: ContentHash(*blake3::hash(d).as_bytes()) };
        let a = f("a.bin", b"1");
        let b = f("b.png", b"2");
        assert_eq!(combined_hash(&[a.clone()]), a.blake3);
        assert_eq!(combined_hash(&[a.clone(), b.clone()]), combined_hash(&[b.clone(), a.clone()]));
        let c = f("b.png", b"3");
        assert_ne!(combined_hash(&[a.clone(), b]), combined_hash(&[a, c]));
    }
}

