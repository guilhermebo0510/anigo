//! Layout dockável (T-2.12): modelo persistido ([`model`]), registro de painéis e padrões por workspace
//! ([`registry`]). A parte com egui (conversão para `egui_tiles`, cabeçalhos, menus) vive em `app/dock_ui.rs`.

pub mod model;
pub mod registry;
pub mod tiles;

pub use model::{Home, LayoutError, LayoutNode, LayoutPreset, LayoutStore, Loaded, LAYOUT_SCHEMA};
pub use registry::{PanelKind, PanelSpec};

use std::path::PathBuf;

/// Diretório de configuração do usuário (layouts, atalhos): `ANIGO_CONFIG_DIR` ou `<dados do studio>/Config`.
pub fn config_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("ANIGO_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    crate::library::studio_data_dir().join("Config")
}

/// Arquivo padrão dos layouts do usuário.
pub fn default_layouts_path() -> PathBuf {
    config_dir().join("layouts.json")
}
