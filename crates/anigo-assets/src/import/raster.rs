//! Analise de texturas (PNG/JPEG decodificados so no cabecalho; demais formatos catalogados como opacos)
//! e de arquivos `.anigomat.json`.

use std::path::Path;

use serde_json::Value;

use super::detect::{extension_lower, Format};
use super::Analysis;
use crate::error::{AssetError, IoContext, Result};
use crate::limits::Limits;
use crate::material::{ToonMaterial, MATERIAL_MAX_BYTES};
use crate::model::{AssetKind, AssetStats};
use crate::paths::confine;

pub(crate) fn analyze_texture(path: &Path, fmt: Format, limits: &Limits) -> Result<Analysis> {
    let meta = std::fs::metadata(path).at(path)?;
    Limits::check("imagem", meta.len(), limits.max_file_bytes)?;
    let ext = extension_lower(path);
    let (mut w, mut h) = (0u32, 0u32);
    let mut warnings = Vec::new();
    if fmt == Format::Raster {
        let reader = image::ImageReader::open(path)
            .at(path)?
            .with_guessed_format()
            .map_err(|e| AssetError::io(path, e))?;
        let (dw, dh) = reader.into_dimensions().map_err(|e| AssetError::parse("imagem", format!("cabecalho ilegivel: {e}")))?;
        if dw == 0 || dh == 0 {
            return Err(AssetError::parse("imagem", "dimensoes zero"));
        }
        Limits::check("largura da imagem", dw as u64, limits.max_image_dimension as u64)?;
        Limits::check("altura da imagem", dh as u64, limits.max_image_dimension as u64)?;
        w = dw;
        h = dh;
    } else {
        warnings.push(format!("formato .{ext}: dimensoes nao lidas (sem decodificador embutido); a miniatura usara um placeholder"));
    }
    let kind = if matches!(ext.as_str(), "hdr" | "exr") { AssetKind::Hdri } else { AssetKind::Texture };
    Ok(Analysis {
        kind,
        stats: AssetStats { textures: 1, width: w, height: h, ..Default::default() },
        dependents: Vec::new(),
        materials: Vec::new(),
        suggested_name: None,
        author: None,
        license: None,
        props: Value::Null,
        warnings,
    })
}

pub(crate) fn analyze_material_file(path: &Path, limits: &Limits) -> Result<Analysis> {
    let meta = std::fs::metadata(path).at(path)?;
    Limits::check("anigomat.json", meta.len(), MATERIAL_MAX_BYTES as u64)?;
    let base_dir = path.parent().ok_or_else(|| AssetError::InvalidInput("arquivo sem pasta pai".into()))?;
    let m = ToonMaterial::from_json(&std::fs::read(path).at(path)?)?;
    let mut dependents = Vec::new();
    let mut warnings = Vec::new();
    for t in m.textures() {
        if let Some(u) = &t.uri {
            let p = confine(base_dir, u)?;
            if p.is_file() {
                let rel = crate::paths::normalize_relative(u)?;
                if !dependents.contains(&rel) && dependents.len() < limits.max_files_per_asset {
                    dependents.push(rel);
                }
            } else {
                warnings.push(format!("textura ausente: {u}"));
            }
        }
    }
    Ok(Analysis {
        kind: AssetKind::Material,
        stats: AssetStats { materials: 1, textures: dependents.len() as u32, ..Default::default() },
        dependents,
        materials: Vec::new(), // o proprio arquivo e o material
        suggested_name: Some(m.name.clone()),
        author: None,
        license: None,
        props: Value::Null,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn png_dimensions_and_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.png");
        std::fs::write(&p, png_bytes(40, 30)).unwrap();
        let a = analyze_texture(&p, Format::Raster, &Limits::default()).unwrap();
        assert_eq!((a.stats.width, a.stats.height), (40, 30));
        assert_eq!(a.kind, AssetKind::Texture);
        // truncado dentro do cabecalho
        let good = png_bytes(40, 30);
        for n in [0usize, 4, 8, 12, 20] {
            std::fs::write(&p, &good[..n]).unwrap();
            assert!(analyze_texture(&p, Format::Raster, &Limits::default()).is_err(), "n={n}");
        }
        // lixo
        std::fs::write(&p, [0x13u8; 200]).unwrap();
        assert!(analyze_texture(&p, Format::Raster, &Limits::default()).is_err());
        // limite de dimensao
        std::fs::write(&p, &good).unwrap();
        let tight = Limits { max_image_dimension: 16, ..Limits::default() };
        assert!(matches!(analyze_texture(&p, Format::Raster, &tight), Err(AssetError::LimitExceeded { .. })));
    }

    #[test]
    fn opaque_textures_and_hdri() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sky.hdr");
        std::fs::write(&p, b"#?RADIANCE").unwrap();
        let a = analyze_texture(&p, Format::OpaqueTexture, &Limits::default()).unwrap();
        assert_eq!(a.kind, AssetKind::Hdri);
        assert!(!a.warnings.is_empty());
        let p = dir.path().join("a.tga");
        std::fs::write(&p, b"x").unwrap();
        assert_eq!(analyze_texture(&p, Format::OpaqueTexture, &Limits::default()).unwrap().kind, AssetKind::Texture);
    }

    #[test]
    fn material_file_analysis() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = ToonMaterial::named("Cabelo");
        m.base.texture = Some(crate::material::TextureRef::from_uri(crate::material::TextureRole::BaseColor, "tex/h.png"));
        let p = dir.path().join("hair.anigomat.json");
        std::fs::write(&p, m.to_json().unwrap()).unwrap();
        let a = analyze_material_file(&p, &Limits::default()).unwrap();
        assert_eq!(a.kind, AssetKind::Material);
        assert_eq!(a.suggested_name.as_deref(), Some("Cabelo"));
        assert!(a.dependents.is_empty());
        assert_eq!(a.warnings.len(), 1);
        std::fs::create_dir_all(dir.path().join("tex")).unwrap();
        std::fs::write(dir.path().join("tex/h.png"), b"x").unwrap();
        assert_eq!(analyze_material_file(&p, &Limits::default()).unwrap().dependents, vec!["tex/h.png"]);
        // invalido
        std::fs::write(&p, b"{\"schema\":\"x\"}").unwrap();
        assert!(analyze_material_file(&p, &Limits::default()).is_err());
        // uri hostil
        let mut evil = ToonMaterial::default();
        evil.base.texture = Some(crate::material::TextureRef::from_uri(crate::material::TextureRole::BaseColor, "../x.png"));
        std::fs::write(&p, serde_json::to_vec(&evil).unwrap()).unwrap();
        assert!(analyze_material_file(&p, &Limits::default()).is_err());
    }
}
