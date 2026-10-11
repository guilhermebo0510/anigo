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
