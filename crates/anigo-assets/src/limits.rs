//! Limites de seguranca aplicados a arquivos de terceiros.

use serde::{Deserialize, Serialize};

use crate::error::{AssetError, Result};

/// Limites configuraveis. Os padroes sao conservadores para um app desktop; ajuste via
/// [`crate::OpenOptions::limits`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    /// Maior arquivo individual aceito na importacao (bytes).
    pub max_file_bytes: u64,
    /// Soma maxima dos arquivos de um unico ativo (bytes).
    pub max_asset_bytes: u64,
    /// Maximo de arquivos por ativo (glTF com muitas texturas, OBJ+MTL...).
    pub max_files_per_asset: usize,
    /// Maior arquivo que o importador le inteiro para a memoria para analisar (GLB, FBX, JSON).
    pub max_parse_bytes: u64,
    /// Tamanho maximo do JSON de um glTF/GLB/anigomat.
    pub max_json_bytes: u64,
    /// Maximos estruturais de glTF.
    pub max_nodes: usize,
    pub max_meshes: usize,
    pub max_materials: usize,
    pub max_accessors: usize,
    pub max_images: usize,
    pub max_animations: usize,
    /// Maximo de vertices declarados (somatorio de POSITION).
    pub max_vertices: u64,
    /// Lado maximo de imagem decodificada (miniaturas e analise de textura).
    pub max_image_dimension: u32,
    /// Maximo de arquivos visitados em um scan/import de pasta.
    pub max_scan_files: usize,
    /// Profundidade maxima de diretorios.
    pub max_scan_depth: usize,
    /// Maximo de linhas de um OBJ/MTL.
    pub max_text_lines: usize,
    /// Tamanho maximo de uma linha de texto (OBJ/MTL).
    pub max_line_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: 2 * 1024 * 1024 * 1024,
            max_asset_bytes: 4 * 1024 * 1024 * 1024,
            max_files_per_asset: 4096,
            max_parse_bytes: 512 * 1024 * 1024,
            max_json_bytes: 128 * 1024 * 1024,
            max_nodes: 500_000,
            max_meshes: 100_000,
            max_materials: 50_000,
            max_accessors: 1_000_000,
            max_images: 50_000,
            max_animations: 10_000,
            max_vertices: 200_000_000,
            max_image_dimension: 32_768,
            max_scan_files: 5_000_000,
            max_scan_depth: 64,
            max_text_lines: 100_000_000,
            max_line_bytes: 1024 * 1024,
        }
    }
}

impl Limits {
    pub(crate) fn check(what: &'static str, actual: u64, limit: u64) -> Result<()> {
        if actual > limit {
            Err(AssetError::LimitExceeded { what, limit, actual })
        } else {
            Ok(())
        }
    }
}
