//! Biblioteca de ativos (Asset Browser): catálogo REAL — bases canônicas lidas dos .glb
//! (contagem de polígonos/vértices medida do arquivo) + objetos criados no workspace de Modelagem.
//! Sem números inventados: se não dá para medir, o campo fica vazio.

use anigo_core::Mesh;
use std::path::{Path, PathBuf};

pub const CATEGORIES: [&str; 8] = [
    "Personagens", "Roupas", "Penteados", "Acessórios", "Poses", "Materiais", "Ambientes", "Cenários",
];

#[derive(Debug, Clone, PartialEq)]
pub struct AssetRecord {
    pub id: String,
    pub name: String,
    pub category: &'static str,
    pub tags: Vec<String>,
    pub tris: Option<usize>,
    pub verts: Option<usize>,
    pub format: &'static str,
    pub path: Option<PathBuf>,
    /// `true` se foi criado no workspace de Modelagem (ainda não exportado).
    pub authored: bool,
}

impl AssetRecord {
    pub fn matches(&self, category: Option<&str>, query: &str) -> bool {
        if let Some(c) = category {
            if self.category != c {
                return false;
            }
        }
        let q = query.trim().to_lowercase();
        q.is_empty()
            || self.name.to_lowercase().contains(&q)
            || self.category.to_lowercase().contains(&q)
            || self.format.to_lowercase().contains(&q)
            || self.tags.iter().any(|t| t.to_lowercase().contains(&q))
    }
}

/// Lê um .glb e mede triângulos/vértices. `None` se o arquivo não for um GLB válido.
pub fn measure_glb(path: &Path) -> Option<(usize, usize)> {
    let mesh = Mesh::from_glb_file(path).ok()?;
    Some((mesh.indices.len() / 3, mesh.vertices.len()))
}

fn category_for(file_stem: &str) -> &'static str {
    let s = file_stem.to_lowercase();
    if s.contains("hair") || s.contains("cabelo") {
        "Penteados"
    } else if s.contains("cloth") || s.contains("outfit") || s.contains("roupa") {
        "Roupas"
    } else if s.contains("base") || s.contains("char") || s.contains("mesh") {
        "Personagens"
    } else {
        "Cenários"
    }
}

/// Varre um diretório por `.glb` (não recursivo).
pub fn scan_glb_dir(dir: &Path) -> Vec<AssetRecord> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = read
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("glb")))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("asset").to_string();
            let measured = measure_glb(&p);
            AssetRecord {
                id: format!("glb:{stem}"),
                name: stem.replace('_', " "),
                category: category_for(&stem),
                tags: vec!["glb".into(), category_for(&stem).to_lowercase()],
                tris: measured.map(|m| m.0),
                verts: measured.map(|m| m.1),
                format: "GLB",
                path: Some(p),
                authored: false,
            }
        })
        .collect()
}

/// Diretório de ativos do usuário (`Documents/ANIGO/Assets`, mesmo do app Tauri legado).
/// `ANIGO_ASSETS_DIR` sobrescreve (testes/portabilidade).
pub fn studio_assets_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("ANIGO_ASSETS_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default();
    home.join("Documents").join("ANIGO").join("Assets")
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

pub fn authored_record(name: &str, mesh: &anigo_modeling::EditMesh) -> AssetRecord {
    AssetRecord {
        id: format!("authored:{name}"),
        name: name.to_string(),
        category: "Cenários",
        tags: vec!["modelagem".into(), "autoral".into()],
        tris: Some(mesh.triangle_count()),
        verts: Some(mesh.vert_count()),
        format: "ANIGO",
        path: None,
        authored: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_assets() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models")
    }

    #[test]
    fn scans_real_glb_with_measured_counts() {
        let list = scan_glb_dir(&repo_assets());
        assert!(list.len() >= 2, "bases canônicas devem existir: {list:?}");
        let female = list.iter().find(|a| a.id == "glb:anigo_base_female").expect("base feminina");
        assert_eq!(female.category, "Personagens");
        assert!(female.tris.unwrap() > 1000, "contagem deve vir do arquivo");
        assert!(female.verts.unwrap() > 500);
    }

    #[test]
    fn export_then_scan_then_import_roundtrip() {
        use crate::modeling::{ModelingSession, PrimitiveKind};
        let dir = std::env::temp_dir().join(format!("anigo_assets_{}", std::process::id()));
        let mut s = ModelingSession::new();
        s.add_primitive(PrimitiveKind::QuadSphere, glam::Vec3::ZERO);
        let obj = s.active_object().unwrap().clone();
        let path = export_object_glb(&obj, &dir).unwrap();
        assert!(path.exists());
        let found = scan_glb_dir(&dir);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].tris, Some(obj.mesh.triangle_count()));
        let back = import_glb_mesh(&path).unwrap();
        assert!(back.face_count() > 0);
        // a esfera tem 216 quads → 432 tris; o quadify recupera a maioria dos quads
        assert!(back.faces.iter().filter(|f| f.len() == 4).count() > 150);
        back.validate().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_stem_is_sanitized() {
        assert_eq!(sanitize_file_stem("Esfera Quad.002"), "Esfera_Quad_002");
        assert_eq!(sanitize_file_stem("///"), "objeto");
    }

    #[test]
    fn missing_dir_yields_empty() {
        assert!(scan_glb_dir(Path::new("/nonexistent")).is_empty());
    }

    #[test]
    fn filter_matches_name_tag_category_and_format() {
        let a = AssetRecord {
            id: "x".into(),
            name: "Twin Tails".into(),
            category: "Penteados",
            tags: vec!["ribbon".into()],
            tris: None,
            verts: None,
            format: "GLB",
            path: None,
            authored: false,
        };
        assert!(a.matches(None, "twin"));
        assert!(a.matches(None, "RIBBON"));
        assert!(a.matches(Some("Penteados"), ""));
        assert!(!a.matches(Some("Roupas"), ""));
        assert!(a.matches(None, "glb"));
        assert!(!a.matches(None, "zzz"));
    }

    #[test]
    fn authored_asset_reports_mesh_counts() {
        let m = anigo_modeling::primitives::cube(1.0);
        let r = authored_record("Cubo", &m);
        assert_eq!((r.tris, r.verts), (Some(12), Some(8)));
        assert!(r.authored);
    }
}
