//! Erros tipados do motor de modelagem (nada de `unwrap` em caminho crítico).

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ModelError {
    #[error("índice de vértice {0} fora do intervalo (malha tem {1} vértices)")]
    VertexOutOfRange(u32, usize),
    #[error("índice de face {0} fora do intervalo (malha tem {1} faces)")]
    FaceOutOfRange(u32, usize),
    #[error("aresta ({0}, {1}) não existe na malha")]
    MissingEdge(u32, u32),
    #[error("face inválida: {0}")]
    InvalidFace(String),
    #[error("seleção vazia: {0}")]
    EmptySelection(&'static str),
    #[error("parâmetro inválido: {0}")]
    InvalidParameter(String),
    #[error("operação não suportada para esta topologia: {0}")]
    UnsupportedTopology(String),
    #[error("falha numérica: {0}")]
    Numeric(String),
}

pub type ModelResult<T> = Result<T, ModelError>;
