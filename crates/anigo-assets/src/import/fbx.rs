//! Analise de FBX via `anigo-native` (ufbx, C vendorizado) quando a feature `fbx` esta ligada.
//! Sem a feature, o arquivo e catalogado como opaco (so o cabecalho e conferido).

use std::path::Path;

use super::Analysis;
use crate::error::Result;
use crate::limits::Limits;
use crate::model::{AssetKind, AssetStats};

#[cfg(feature = "fbx")]
pub(crate) fn analyze_fbx(path: &Path, limits: &Limits) -> Result<Analysis> {
    use crate::error::{AssetError, IoContext};
    use crate::material::convert::{pbr_from_fbx_parts, toon_from_pbr};

    let meta = std::fs::metadata(path).at(path)?;
    Limits::check("arquivo FBX", meta.len(), limits.max_parse_bytes)?;
    let base_dir = path.parent().ok_or_else(|| AssetError::InvalidInput("arquivo sem pasta pai".into()))?;
    let bytes = std::fs::read(path).at(path)?;
    let opts = anigo_native::FbxImportOptions { max_input_bytes: limits.max_parse_bytes as usize, ..Default::default() };
    // anigo-native ja captura panics (NativeError::Panic); o `guard` do pipeline cobre o resto.
    let scene = anigo_native::import_fbx_with(&bytes, &opts).map_err(|e| AssetError::parse("fbx", e.to_string()))?;
    drop(bytes);

    let mut warnings: Vec<String> = scene.info.warnings.clone();
    if scene.info.dropped_faces > 0 {
        warnings.push(format!("{} faces degeneradas descartadas pelo ufbx", scene.info.dropped_faces));
    }
    let mut tris = 0u64;
    let mut verts = 0u64;
    let mut morphs = 0u64;
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for m in &scene.meshes {
        verts += m.mesh.verts.len() as u64;
        morphs += m.blend_shapes.len() as u64;
        for f in &m.mesh.faces {
            let k = f.corners.len() as u64;
            if k >= 3 {
                tris += k - 2;
            }
        }
        for v in &m.mesh.verts {
            let c = v.co;
            min = [min[0].min(c.x), min[1].min(c.y), min[2].min(c.z)];
            max = [max[0].max(c.x), max[1].max(c.y), max[2].max(c.z)];
        }
    }
    Limits::check("vertices", verts, limits.max_vertices)?;
    let bones: u64 = scene.skins.iter().map(|s| s.joints.len() as u64).sum();

    let mut dependents: Vec<String> = Vec::new();
    let mut materials = Vec::new();
    for m in &scene.materials {
        let pbr = pbr_from_fbx_parts(&m.name, m.base_color, m.base_color_texture.as_deref(), &mut warnings);
        if let Some(t) = &pbr.base_color_texture {
            if let Some(u) = &t.uri {
                match crate::paths::confine(base_dir, u) {
                    Ok(p) if p.is_file() => {
                        if !dependents.contains(u) && dependents.len() < limits.max_files_per_asset {
                            dependents.push(u.clone());
                        }
                    }
                    _ => warnings.push(format!("textura FBX ausente ao lado do arquivo: {u}")),
                }
            }
        }
        if materials.len() < 256 {
            materials.push(toon_from_pbr(&pbr, "fbx"));
        }
    }
    let any_geometry = !scene.meshes.is_empty();
    let kind = if !any_geometry && !scene.clips.is_empty() { AssetKind::Animation } else { AssetKind::Model };
    let bbox = if verts > 0 { Some([min[0], min[1], min[2], max[0], max[1], max[2]]) } else { None };
    let props = serde_json::json!({
        "fbx_creator": scene.info.creator,
        "fbx_version": scene.info.fbx_version,
        "original_up_axis": scene.info.original_up_axis,
        "original_unit_meters": scene.info.original_unit_meters,
    });
    Ok(Analysis {
        kind,
        stats: AssetStats {
            tris,
            verts,
            meshes: scene.meshes.len() as u32,
            nodes: scene.nodes.len() as u32,
            materials: scene.materials.len() as u32,
            textures: dependents.len() as u32,
            bones: bones.min(u32::MAX as u64) as u32,
            morphs: morphs.min(u32::MAX as u64) as u32,
            anims: scene.clips.len() as u32,
            has_skin: !scene.skins.is_empty(),
            has_vrm: false,
            bbox,
            width: 0,
            height: 0,
        },
        dependents,
        materials,
        suggested_name: None,
        author: if scene.info.creator.is_empty() { None } else { Some(scene.info.creator.chars().take(256).collect()) },
        license: None,
        props,
        warnings,
    })
}

#[cfg(not(feature = "fbx"))]
pub(crate) fn analyze_fbx(path: &Path, limits: &Limits) -> Result<Analysis> {
    use crate::error::IoContext;
    use serde_json::Value;
    let meta = std::fs::metadata(path).at(path)?;
    Limits::check("arquivo FBX", meta.len(), limits.max_file_bytes)?;
    Ok(Analysis {
        kind: AssetKind::Model,
        stats: AssetStats::default(),
        dependents: Vec::new(),
        materials: Vec::new(),
        suggested_name: None,
        author: None,
        license: None,
        props: Value::Null,
        warnings: vec!["feature `fbx` desligada: estatisticas e materiais do FBX nao foram extraidos".into()],
    })
}

#[cfg(all(test, feature = "fbx"))]
mod tests {
    use super::*;

    #[test]
    fn garbage_with_valid_magic_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let mut data = b"Kaydara FBX Binary  \0\x1a\0".to_vec();
        data.extend_from_slice(&[0xFF; 300]);
        let p = dir.path().join("bad.fbx");
        std::fs::write(&p, &data).unwrap();
        assert!(analyze_fbx(&p, &Limits::default()).is_err());
        // ASCII truncado
        std::fs::write(&p, b"; FBX 7.4.0 project file\nFBXHeaderExtension:  {\n").unwrap();
        let _ = analyze_fbx(&p, &Limits::default());
        // limite de tamanho
        let tight = Limits { max_parse_bytes: 10, ..Limits::default() };
        std::fs::write(&p, &data).unwrap();
        assert!(analyze_fbx(&p, &tight).is_err());
    }
}
