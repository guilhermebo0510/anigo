//! # anigo-modeling
//!
//! Motor de modelagem orgânica do ANIGO (inspirado no Blender, focado em
//! personagens anime e assets de cenário). Rust puro, sem GPU: tudo aqui é
//! testável em CI. A visualização e a UI ficam em `anigo-studio`.
//!
//! Camadas:
//! * [`mesh`] / [`topology`] / [`select`] — estrutura de dados (n-gons, atributos por corner/aresta).
//! * [`primitives`] — primitivas.
//! * [`normals`] — normais por corner (sharp/smooth/custom), crucial para cel shading.
//! * [`convert`] — ponte com `anigo_core::Mesh` (formato de render de 72 B).
//! * [`hair`] — ferramenta de cabelo estilo VRoid (mechas editáveis, malha, pentear, cadeias de física).
//! * [`history`] — undo/redo por snapshot (edit mode) e por delta (escultura).

pub mod attributes;
pub mod bvh;
pub mod convert;
pub mod curve;
pub mod error;
pub mod hair;
pub mod history;
pub mod mesh;
pub mod modifiers;
pub mod normals;
pub mod normals_tools;
pub mod ops;
pub mod primitives;
pub mod sculpt;
pub mod select;
pub mod shapekeys;
pub mod subdiv;
pub mod topology;
pub mod uv;

pub use error::{ModelError, ModelResult};
pub use mesh::{edge_key, Corner, EVert, EdgeAttr, EdgeKey, EditMesh, Face, Remap};
pub use select::{SelectMode, Selection};
pub use topology::Topology;
