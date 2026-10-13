//! Sessão canônica do núcleo: a única porta entre qualquer shell (Tauri legado, app nativo,
//! MCP) e a autoridade dos dados (`ProjectState`).
pub mod core_session;
pub use core_session::*;
