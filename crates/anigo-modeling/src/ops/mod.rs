//! Operadores de edit mode (equivalentes às ferramentas da Edit Mode do Blender).
//!
//! Convenção: cada operador é uma função livre `fn(&mut EditMesh, …) -> ModelResult<OpOutput>`
//! que valida a entrada ANTES de mutar (em erro a malha permanece intacta),
//! devolve a seleção resultante (para encadear "extrude → mover") e é
//! determinística. Nenhum operador usa `unwrap`/`expect` em dado do usuário.

use crate::mesh::Remap;
use crate::select::Selection;

/// Resultado comum dos operadores.
#[derive(Debug, Clone, Default)]
pub struct OpOutput {
    /// Seleção que a UI deve aplicar após o operador (ex.: faces extrudadas).
    pub selection: Selection,
    pub created_verts: Vec<u32>,
    pub created_faces: Vec<u32>,
    /// Presente quando índices foram compactados (deleções).
    pub remap: Option<Remap>,
}

pub mod bevel;
pub mod bisect;
pub mod bridge;
pub mod delete;
pub mod dissolve;
pub mod extrude;
pub mod fill;
pub mod flip;
pub mod inset;
pub mod loops;
pub mod merge;
pub mod misc;
pub mod smooth;
pub mod spin;
pub mod transform;
pub mod triangulate;
