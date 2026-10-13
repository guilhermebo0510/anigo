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

pub mod anim;
pub mod app;
pub mod brand;
pub mod celview;
pub mod character;
pub mod cli;
pub mod hot;
pub mod i18n;
pub mod i18n_animation;
pub mod i18n_dock;
pub mod i18n_hair;
pub mod i18n_lib;
pub mod i18n_lighting;
pub mod i18n_render;
pub mod i18n_posing;
pub mod i18n_scene;
pub mod i18n_shading;
pub mod icons;
pub mod icons_anigo;
pub mod keymap;
pub mod layout;
pub mod library;
pub mod library_ui;
pub mod lighting;
pub mod modeling;
pub mod render_out;
pub mod posing;
pub mod scene;
pub mod session;
pub mod shading;
pub mod theme;
pub mod viewport;
pub mod workbench;
pub mod workspace;
