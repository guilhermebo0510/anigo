//! Núcleo puro do workspace Animação (sem egui): transporte, planejadores de edição de chaves, matemática do Graph
//! Editor, auto-key, física secundária no playback e a fonte de quadros da fila de Render.
//!
//! A UI (`app::animation_ui`) só costura estes módulos e emite `Command::EditAnimation` para o núcleo.

pub mod autokey;
pub mod graph;
pub mod keys;
pub mod playback;
pub mod secondary;
pub mod sequence;

pub use playback::{Span, Transport};
