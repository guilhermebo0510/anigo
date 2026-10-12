//! Núcleo puro do workspace Cenário (sem egui): câmera de navegação, snap, gizmo, BVH de raycast,
//! seleção, Outliner, geometria dos nós (primitivas + ativos da Biblioteca) e colocação.
//!
//! A UI (`app::scene_ui`) só costura estes módulos e emite `Command`s para o núcleo; toda a
//! matemática que decide "onde fica o objeto" mora aqui e tem testes unitários.

pub mod bvh;
pub mod camview;
pub mod geometry;
pub mod gizmo;
pub mod outliner;
pub mod placement;
pub mod runtime;
pub mod selection;
pub mod snap;
