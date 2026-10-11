//! # anigo-native — a fronteira C/C++ do ANIGO
//!
//! Todo código C/C++ do projeto entra por aqui, atrás de **features Cargo
//! independentes e desligáveis**. Sem nenhuma feature (`--no-default-features`)
//! a crate compila só com Rust (sem toolchain C++) e expõe apenas
//! [`NativeError`] e [`capabilities`], para a UI saber o que está disponível.
//!
//! | feature   | biblioteca (upstream)            | módulo        | para quê |
//! |-----------|----------------------------------|---------------|----------|
//! | `meshopt` | meshoptimizer 0.25 (C++)         | [`simplify`]  | simplificação/LOD, cache de vértices, overdraw |
//! | `ufbx`    | ufbx 0.23 (C)                    | [`fbx`]       | import FBX (n-gons, skin, blend shapes, animação) |
//! | `xatlas`  | xatlas (C++)                     | [`atlas`]     | auto-unwrap / empacotamento de atlas de UV |
//!
//! Regras (detalhes em `docs/native/CPP_FFI.md`):
//! * a API pública usa só tipos Rust do ANIGO (`anigo_core::Mesh`,
//!   `anigo_modeling::EditMesh`); **nenhum ponteiro cru** atravessa a fronteira;
//! * toda entrada externa é validada **antes** de chegar ao código nativo
//!   (índices em faixa, valores finitos, limites de tamanho);
//! * erros viram [`NativeError`]; pânicos dentro do wrapper viram
//!   [`NativeError::Panic`] (`catch_unwind`);
//! * `unsafe` existe somente em `atlas/ffi.rs` (a camada `extern "C"` do xatlas),
//!   isolado e documentado. Sem a feature `xatlas` a crate usa
//!   `#![forbid(unsafe_code)]`.

// `%` em vez de `is_multiple_of` para não elevar o MSRV do workspace.
#![allow(clippy::manual_is_multiple_of)]
#![cfg_attr(not(feature = "xatlas"), forbid(unsafe_code))]
#![cfg_attr(feature = "xatlas", deny(unsafe_code))]

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use thiserror::Error;

#[cfg(feature = "xatlas")]
pub mod atlas;
#[cfg(feature = "ufbx")]
pub mod fbx;
#[cfg(feature = "meshopt")]
pub mod simplify;

#[cfg(feature = "xatlas")]
pub use atlas::{apply_atlas, auto_unwrap, compute_atlas, AtlasOptions, AtlasResult};
#[cfg(feature = "ufbx")]
pub use fbx::{import_fbx, import_fbx_with, FbxImportOptions, FbxScene};
#[cfg(feature = "meshopt")]
pub use simplify::{
    generate_lods, generate_lods_with, optimize_mesh, optimize_mesh_with, simplify_mesh,
    AttributeWeights, LodLevel, LodOptions, OptimizeOptions, SimplifyOptions, SimplifyResult,
};

/// Erro único da fronteira nativa.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum NativeError {
    /// Entrada sem geometria utilizável (sem vértices, sem triângulos, bytes vazios).
    #[error("entrada vazia: {0}")]
    EmptyInput(&'static str),
    /// Geometria presente mas degenerada (todos os triângulos com área zero etc.).
    #[error("geometria degenerada: {0}")]
    Degenerate(String),
    /// Parâmetro ou dado inválido (índice fora de faixa, NaN, razão fora de (0,1]…).
    #[error("entrada inválida: {0}")]
    InvalidInput(String),
    /// A entrada excede um limite de segurança (tamanho, memória, profundidade).
    #[error("limite excedido: {what} = {got} (máximo {max})")]
    LimitExceeded {
        what: &'static str,
        got: u64,
        max: u64,
    },
    /// Falha reportada pela biblioteca nativa (arquivo corrompido, sem memória…).
    #[error("falha em {lib}: {message}")]
    Backend { lib: &'static str, message: String },
    /// Pânico capturado dentro do wrapper (bug em nossa camada ou em binding).
    #[error("pânico interno em {lib}: {message}")]
    Panic { lib: &'static str, message: String },
    /// Erro vindo do motor de modelagem ao construir/validar o `EditMesh`.
    #[error("modelagem: {0}")]
    Model(String),
}

impl From<anigo_modeling::ModelError> for NativeError {
    fn from(e: anigo_modeling::ModelError) -> Self {
        NativeError::Model(e.to_string())
    }
}

pub type NativeResult<T> = Result<T, NativeError>;

/// Executa `f` capturando pânico (jamais deixa um unwind cruzar a fronteira).
/// Usado por todos os wrappers que chamam código nativo ou bindings de terceiros.
#[allow(dead_code)] // sem nenhuma feature ligada ninguém chama
pub(crate) fn guarded<T>(
    lib: &'static str,
    f: impl FnOnce() -> NativeResult<T>,
) -> NativeResult<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "pânico sem mensagem".into());
            Err(NativeError::Panic { lib, message })
        }
    }
}

/// O que foi compilado nesta build — para a UI habilitar/desabilitar comandos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Simplificação/LOD/otimização de índices (meshoptimizer).
    pub meshopt: bool,
    /// Import FBX (ufbx).
    pub ufbx: bool,
    /// Auto-unwrap / atlas de UV (xatlas).
    pub xatlas: bool,
    /// Versões das bibliotecas **compiladas** (chave = nome da feature).
    /// Só contém as features ligadas.
    pub versions: BTreeMap<String, String>,
}

/// Capacidades da build atual (decidido em tempo de compilação pelas features).
pub fn capabilities() -> Capabilities {
    let mut versions = BTreeMap::new();
    if cfg!(feature = "meshopt") {
        versions.insert(
            "meshopt".to_string(),
            "meshoptimizer 0.25 (crate meshopt 0.6.2)".to_string(),
        );
    }
    if cfg!(feature = "ufbx") {
        versions.insert(
            "ufbx".to_string(),
            "ufbx 0.23.1 (crate ufbx 0.11.5)".to_string(),
        );
    }
    if cfg!(feature = "xatlas") {
        versions.insert(
            "xatlas".to_string(),
            "xatlas 2020 (vendor/xatlas, via crates.io xatlas-rs-v2 0.1.4)".to_string(),
        );
    }
    Capabilities {
        meshopt: cfg!(feature = "meshopt"),
        ufbx: cfg!(feature = "ufbx"),
        xatlas: cfg!(feature = "xatlas"),
        versions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_follow_features() {
        let c = capabilities();
        assert_eq!(c.meshopt, cfg!(feature = "meshopt"));
        assert_eq!(c.ufbx, cfg!(feature = "ufbx"));
        assert_eq!(c.xatlas, cfg!(feature = "xatlas"));
        assert_eq!(
            c.versions.len(),
            [c.meshopt, c.ufbx, c.xatlas].iter().filter(|b| **b).count()
        );
        let json = serde_json::to_string(&c).expect("serializa");
        let back: Capabilities = serde_json::from_str(&json).expect("desserializa");
        assert_eq!(back, c);
    }

    #[test]
    fn guarded_turns_panic_into_error() {
        let r: NativeResult<()> = guarded("teste", || panic!("boom"));
        match r {
            Err(NativeError::Panic { lib, message }) => {
                assert_eq!(lib, "teste");
                assert!(message.contains("boom"));
            }
            other => panic!("esperava Panic, veio {other:?}"),
        }
        let ok: NativeResult<u8> = guarded("teste", || Ok(7));
        assert_eq!(ok, Ok(7));
    }

    #[test]
    fn model_error_converts() {
        let e: NativeError = anigo_modeling::ModelError::Numeric("x".into()).into();
        assert!(matches!(e, NativeError::Model(_)));
    }
}
