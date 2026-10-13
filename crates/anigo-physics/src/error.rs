//! Erros de construção/configuração. A simulação em si (`step`) nunca devolve erro nem entra em
//! pânico: entrada inválida (NaN, `dt <= 0`) é descartada e contada em [`crate::StepReport`].

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum PhysicsError {
    #[error("índice inválido em {what}: {index} (limite {len})")]
    BadIndex { what: &'static str, index: usize, len: usize },
    #[error("valor não finito ou fora de faixa em {what}")]
    BadValue { what: String },
    #[error("hierarquia inválida: {0}")]
    BadHierarchy(String),
    #[error("malha inválida: {0}")]
    BadMesh(String),
    #[error("configuração inválida: {0}")]
    BadConfig(String),
    #[error("serialização: {0}")]
    Serde(String),
}

pub type PhysicsResult<T> = Result<T, PhysicsError>;

impl From<serde_json::Error> for PhysicsError {
    fn from(e: serde_json::Error) -> Self {
        PhysicsError::Serde(e.to_string())
    }
}

pub(crate) fn check_index(what: &'static str, index: usize, len: usize) -> PhysicsResult<()> {
    if index < len {
        Ok(())
    } else {
        Err(PhysicsError::BadIndex { what, index, len })
    }
}
