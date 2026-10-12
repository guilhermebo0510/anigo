//! Ponte entre o studio e a Biblioteca de ativos (`anigo-assets`): onde ficam os dados do usuário, quais
//! pastas entram como locais ligados por padrão e como um objeto da Modelagem vira (ou volta de) um ativo.
//! A UI mora em [`crate::library_ui`]; este módulo não tem egui.

use anigo_core::Mesh;
use std::path::{Path, PathBuf};

/// Diretório de ativos do usuário do app antigo (`Documents/ANIGO/Assets`, mesmo do app Tauri legado).
/// `ANIGO_ASSETS_DIR` sobrescreve (testes/portabilidade). Hoje entra como local LIGADO da biblioteca.
pub fn studio_assets_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("ANIGO_ASSETS_DIR") {
        return PathBuf::from(dir);
    }
    studio_data_dir().join("Assets")
}

/// `Documents/ANIGO` (dados do usuário do studio).
pub fn studio_data_dir() -> PathBuf {
    match std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).filter(|p| !p.as_os_str().is_empty()) {
        Some(home) => home.join("Documents").join("ANIGO"),
        // sem HOME (CI mínimo, serviço): nunca criar a biblioteca relativa ao diretório de trabalho
        None => std::env::temp_dir().join("ANIGO"),
    }
}

/// Pasta da biblioteca gerenciada padrão. `ANIGO_LIBRARY_DIR` sobrescreve.
pub fn default_library_home() -> PathBuf {
    if let Ok(dir) = std::env::var("ANIGO_LIBRARY_DIR") {
        return PathBuf::from(dir);
    }
    studio_data_dir().join("Library")
}

/// Locais LIGADOS registrados na primeira abertura: ativos legados do usuário e as bases canônicas do repositório
/// (quando o app roda de uma árvore de desenvolvimento). Só entram pastas que existem.
pub fn linked_defaults() -> Vec<(String, PathBuf)> {
    let mut v = Vec::new();
    let legacy = studio_assets_dir();
    if legacy.is_dir() {
        v.push(("Ativos legados (GLB)".to_string(), legacy));
    }
    let canon = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models");
    if canon.is_dir() {
        v.push(("Bases canônicas".to_string(), canon));
    }
    v
}

pub fn default_spec() -> crate::library_ui::LibrarySpec {
    crate::library_ui::LibrarySpec { home: default_library_home(), linked_defaults: linked_defaults(), scan_on_open: true }
}

fn sanitize_file_stem(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() {
        "objeto".into()
    } else {
        s
    }
}

/// Exporta um objeto de modelagem (malha avaliada, com modificadores) como GLB na pasta de ativos.
pub fn export_object_glb(
    obj: &crate::modeling::ModelObject,
    dir: &Path,
) -> Result<PathBuf, String> {
    use anigo_modeling::convert::{to_core_mesh, ToCoreOptions};
    let evaluated = match obj.evaluated(crate::modeling::EvalMode::Render) {
        Some(Ok(m)) => m,
        Some(Err(e)) => return Err(format!("modificadores: {e}")),
        None => obj.mesh.clone(),
    };
    let mut core = to_core_mesh(&evaluated, &ToCoreOptions::default()).map_err(|e| e.to_string())?;
    if core.indices.is_empty() {
        return Err("a malha está vazia".into());
    }
    core.name = obj.name.clone();
    std::fs::create_dir_all(dir).map_err(|e| format!("criar {}: {e}", dir.display()))?;
    let path = dir.join(format!("{}.glb", sanitize_file_stem(&obj.name)));
    core.to_glb_file(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Importa um GLB como malha editável (n-gons recuperados por quadify; normais preservadas).
pub fn import_glb_mesh(path: &Path) -> Result<anigo_modeling::EditMesh, String> {
    use anigo_modeling::convert::{from_core_mesh, FromCoreOptions};
    let core = Mesh::from_glb_file(path).map_err(|e| e.to_string())?;
    let mut em = from_core_mesh(&core, &FromCoreOptions::default()).map_err(|e| e.to_string())?;
    anigo_modeling::convert::quadify(&mut em, 5f32.to_radians());
    em.name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("importado").to_string();
    Ok(em)
}


/// Abre um modelo da Biblioteca (GLB/VRM, OBJ ou FBX) como malha editável da Modelagem.
pub fn import_model_mesh(path: &Path, format: &str) -> Result<anigo_modeling::EditMesh, String> {
    use anigo_modeling::convert::{from_core_mesh, FromCoreOptions};
    let core = crate::library_ui::cel_thumb::load_core_mesh(path, format)?;
    let mut em = from_core_mesh(&core, &FromCoreOptions::default()).map_err(|e| e.to_string())?;
    anigo_modeling::convert::quadify(&mut em, 5f32.to_radians());
    em.name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("importado").to_string();
    Ok(em)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_then_import_roundtrip() {
        use crate::modeling::{ModelingSession, PrimitiveKind};
        let dir = std::env::temp_dir().join(format!("anigo_assets_{}", std::process::id()));
        let mut s = ModelingSession::new();
        s.add_primitive(PrimitiveKind::QuadSphere, glam::Vec3::ZERO);
        let obj = s.active_object().unwrap().clone();
        let path = export_object_glb(&obj, &dir).unwrap();
        assert!(path.exists());
        let back = import_glb_mesh(&path).unwrap();
        assert!(back.face_count() > 0);
        // a esfera tem 216 quads -> 432 tris; o quadify recupera a maioria dos quads
        assert!(back.faces.iter().filter(|f| f.len() == 4).count() > 150);
        back.validate().unwrap();
        // o caminho genérico (GLB) dá a mesma malha
        let generic = import_model_mesh(&path, "glb").unwrap();
        assert_eq!(generic.face_count(), back.face_count());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_stem_is_sanitized() {
        assert_eq!(sanitize_file_stem("Esfera Quad.002"), "Esfera_Quad_002");
        assert_eq!(sanitize_file_stem("///"), "objeto");
    }

    #[test]
    fn model_formats_without_support_fail_with_a_message() {
        let e = import_model_mesh(Path::new("/nonexistent/x.png"), "png").unwrap_err();
        assert!(e.contains("png"), "{e}");
    }

    #[test]
    fn env_overrides_decide_the_data_dirs() {
        // só lê variáveis (não as altera: testes rodam em paralelo)
        let home = default_library_home();
        assert!(home.ends_with("Library") || std::env::var_os("ANIGO_LIBRARY_DIR").is_some());
        assert!(studio_assets_dir().ends_with("Assets") || std::env::var_os("ANIGO_ASSETS_DIR").is_some());
    }
}
