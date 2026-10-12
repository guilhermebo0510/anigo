//! # anigo-studio
//!
//! Shell nativo do ANIGO: UI (egui) e viewport (wgpu) 100 % Rust.
//!
//! * [`workspace`] — os 9 workspaces e suas ferramentas.
//! * [`viewport`] — câmera orbital, picking e projeção (matemática pura, testável).
//! * [`workbench`] — renderer de edição (grid, sólido, wireframe, pontos), equivalente
//!   ao "Workbench" do Blender; o render final cel-shaded continua no `anigo-renderer`.
//! * [`modeling`] — sessão do workspace de Modelagem (objetos, seleção, modos, transformações modais).
//! * [`app`] — `eframe::App` que costura tudo.

pub mod app;
pub mod celview;
pub mod character;
pub mod cli;
pub mod hot;
pub mod i18n;
pub mod i18n_hair;
pub mod i18n_lib;
pub mod icons;
pub mod library;
pub mod library_ui;
pub mod modeling;
pub mod session;
pub mod theme;
pub mod viewport;
pub mod workbench;
pub mod workspace;
