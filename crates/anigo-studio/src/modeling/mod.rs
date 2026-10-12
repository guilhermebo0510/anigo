//! Sessão do workspace de Modelagem: documento de objetos, modos (Objeto/Edição/Escultura),
//! seleção, transformação modal e histórico. UI-agnóstica: o `app` só traduz entrada em chamadas.

pub mod history;
pub mod modal;
pub mod sculpt;
pub mod session;

pub use modal::{Axis, Constraint, ModalContext, ModalTransform, TransformKind};
pub use session::{Document, Mode, ModelObject, ModelingSession, PrimitiveKind, SnapshotPub};
pub use anigo_modeling::modifiers::stack::EvalMode;
