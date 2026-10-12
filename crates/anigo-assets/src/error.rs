//! Erro unico do crate.

use std::path::PathBuf;

use crate::model::AssetId;

/// Resultado padrao do crate.
pub type Result<T> = std::result::Result<T, AssetError>;

/// Erros da biblioteca de ativos. Nenhuma entrada malformada deve causar `panic`: tudo vira um destes.
#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    /// Falha de E/S (com o caminho, quando conhecido).
    #[error("erro de E/S{}: {source}", path.as_ref().map(|p| format!(" em {}", p.display())).unwrap_or_default())]
    Io {
        path: Option<PathBuf>,
        #[source]
        source: std::io::Error,
    },
    /// Erro do SQLite.
    #[error("erro de banco: {0}")]
    Sql(#[from] rusqlite::Error),
    /// JSON invalido ou nao serializavel.
    #[error("json invalido: {0}")]
    Json(#[from] serde_json::Error),
    /// Recurso inexistente (asset, raiz, colecao, arquivo...).
    #[error("nao encontrado: {0}")]
    NotFound(String),
    /// Argumento invalido do chamador.
    #[error("entrada invalida: {0}")]
    InvalidInput(String),
    /// Formato/recurso nao suportado (inclui feature Cargo desligada).
    #[error("nao suportado: {0}")]
    Unsupported(String),
    /// Um limite de seguranca foi excedido.
    #[error("limite excedido em {what}: {actual} > {limit}")]
    LimitExceeded { what: &'static str, limit: u64, actual: u64 },
    /// Caminho/URI fora da pasta permitida (traversal, absoluto, esquema externo, symlink).
    #[error("caminho inseguro: {0}")]
    UnsafePath(String),
    /// Arquivo de ativo malformado ou truncado.
    #[error("{format} malformado: {message}")]
    Parse { format: &'static str, message: String },
    /// O banco foi criado por uma versao mais nova do programa.
    #[error("esquema do indice v{found} e mais novo que o suportado (v{supported})")]
    SchemaTooNew { found: i64, supported: i64 },
    /// Dado persistido inconsistente (manifesto ilegivel etc.).
    #[error("dados corrompidos: {0}")]
    Corrupt(String),
    /// A operacao foi bloqueada porque outros ativos dependem deste.
    #[error("ativo {asset} em uso por {} outros ativos", used_by.len())]
    InUse { asset: AssetId, used_by: Vec<AssetId> },
    /// Operacao cancelada pelo usuario.
    #[error("operacao cancelada")]
    Cancelled,
    /// Conflito de estado (raiz ja existe, ativo ja na lixeira...).
    #[error("conflito: {0}")]
    Conflict(String),
    /// Falha interna (inclui `panic` capturado em worker).
    #[error("falha interna: {0}")]
    Internal(String),
}

impl AssetError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        AssetError::Io { path: Some(path.into()), source }
    }
    pub(crate) fn parse(format: &'static str, message: impl Into<String>) -> Self {
        AssetError::Parse { format, message: message.into() }
    }
}

impl From<std::io::Error> for AssetError {
    fn from(source: std::io::Error) -> Self {
        AssetError::Io { path: None, source }
    }
}

/// Extensao para anexar o caminho a erros de E/S.
pub(crate) trait IoContext<T> {
    fn at(self, path: &std::path::Path) -> Result<T>;
}

impl<T> IoContext<T> for std::io::Result<T> {
    fn at(self, path: &std::path::Path) -> Result<T> {
        self.map_err(|e| AssetError::io(path, e))
    }
}
