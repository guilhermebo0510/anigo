//! # anigo-assets - backend da Biblioteca de ativos
//!
//! Sem UI e sem GPU. Registry SQLite (WAL + FTS5) reconstruivel a partir de manifestos `asset.json`,
//! import em background (GLB/glTF/VRM, OBJ, FBX, imagens), material canonico **ANIGO Toon Material**,
//! cache de miniaturas, lixeira, duplicados, ausentes/relink e verificacao de integridade.
//!
//! ```no_run
//! use anigo_assets::*;
//! let lib = Library::open("/dados/minha-biblioteca")?;
//! let job = lib.import("/downloads/hero.glb", ImportOptions::default());
//! for ev in job.events().try_iter() { /* progresso na UI */ }
//! let report = job.join()?;
//! let page = lib.query(&Query { text: Some("hero".into()), kinds: vec![AssetKind::Character], ..Default::default() })?;
//! # Ok::<(), AssetError>(())
//! ```
//!
//! Visao geral e decisoes: `docs/native/BIBLIOTECA_BACKEND.md`.

pub mod collections;
pub mod db;
pub mod error;
pub mod import;
pub mod integrity;
pub mod job;
pub mod library;
pub mod limits;
pub mod material;
pub mod meta;
pub mod model;
pub mod paths;
pub mod query;
pub mod rebuild;
pub mod scan;
pub(crate) mod store;
pub mod thumbs;
pub mod trash;
pub(crate) mod util;
pub mod watch;

pub use collections::CollectionDef;
pub use error::{AssetError, Result};
pub use import::{format_by_extension, preview, DuplicatePolicy, Format, ImportEvent, ImportJob, ImportOptions, ImportReport, Preview};
pub use integrity::{DuplicateGroup, IssueKind, VerifyEvent, VerifyIssue, VerifyJob, VerifyOptions, VerifyReport};
pub use job::{CancelToken, Job};
pub use library::{layout, Library, OpenOptions, RootInfo, HOME_ROOT};
pub use limits::Limits;
pub use material::{StylizedExtras, ToonMaterial, MATERIAL_SUFFIX};
pub use meta::{InfoPatch, ResolvedFiles, TagInfo};
pub use model::*;
pub use query::{parse_text, Cursor, Page, Query, Sort, SortBy, StateFilter};
pub use rebuild::RebuildReport;
pub use scan::{ScanJob, ScanOptions, ScanReport};
pub use store::normalize_tag;
pub use thumbs::{
    default_cpu_renderer, ChainRenderer, ClosureRenderer, ImageThumbnailRenderer, MaterialSwatchRenderer, PlaceholderRenderer, PoseStickRenderer, Rendered, ThumbEvent, ThumbJob, ThumbOptions,
    ThumbReport, ThumbRequest, ThumbnailCache, ThumbnailRenderer, MASTER_SIZE, THUMB_SIZES,
};
pub use trash::{PurgeReport, TrashOptions, TrashReport};
pub use util::now_secs;
pub use watch::{WatchEvent, Watcher};
