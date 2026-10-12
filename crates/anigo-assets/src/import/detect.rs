//! Deteccao de formato por extensao + conferencia de magic bytes. Nada aqui le o arquivo inteiro.

use std::io::Read;
use std::path::Path;

use crate::error::{AssetError, IoContext, Result};

/// Formatos que o importador reconhece.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Glb,
    Gltf,
    Vrm,
    Obj,
    Fbx,
    /// PNG/JPEG: dimensoes lidas do cabecalho.
    Raster,
    /// Outras texturas (tga, bmp, webp, dds, ktx2, exr, hdr): so catalogadas, sem decodificar.
    OpaqueTexture,
    AnigoMat,
    /// Pose do esqueleto canonico (`.anigopose.json`, ver `anigo_core::pose::PoseAsset`).
    AnigoPose,
}

impl Format {
    /// Formatos "modelo" que podem reivindicar arquivos dependentes (texturas, buffers).
    pub fn claims_dependents(self) -> bool {
        matches!(self, Format::Glb | Format::Gltf | Format::Vrm | Format::Obj | Format::Fbx | Format::AnigoMat)
    }
}

/// Extensao minuscula do arquivo ("" se nao houver). Trata `.anigomat.json` como `anigomat.json`.
pub(crate) fn extension_lower(path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if name.ends_with(crate::material::MATERIAL_SUFFIX) {
        return "anigomat.json".into();
    }
    if name.ends_with(anigo_core::pose::POSE_ASSET_SUFFIX) {
        return "anigopose.json".into();
    }
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Formato pela extensao (sem tocar no disco). `None` = nao suportado.
pub fn format_by_extension(path: &Path) -> Option<Format> {
    Some(match extension_lower(path).as_str() {
        "glb" => Format::Glb,
        "gltf" => Format::Gltf,
        "vrm" => Format::Vrm,
        "obj" => Format::Obj,
        "fbx" => Format::Fbx,
        "png" | "jpg" | "jpeg" => Format::Raster,
        "tga" | "bmp" | "webp" | "dds" | "ktx2" | "exr" | "hdr" => Format::OpaqueTexture,
        "anigomat.json" => Format::AnigoMat,
        "anigopose.json" => Format::AnigoPose,
        _ => return None,
    })
}

/// Extensao textual a gravar em `AssetManifest::format`.
pub(crate) fn format_label(path: &Path) -> String {
    let e = extension_lower(path);
    if e == "anigomat.json" {
        "anigomat".into()
    } else if e == "anigopose.json" {
        "anigopose".into()
    } else if e == "jpeg" {
        "jpg".into()
    } else {
        e
    }
}

fn read_head(path: &Path, n: usize) -> Result<Vec<u8>> {
    let mut f = std::fs::File::open(path).at(path)?;
    let mut buf = vec![0u8; n];
    let mut got = 0;
    while got < n {
        let r = f.read(&mut buf[got..]).at(path)?;
        if r == 0 {
            break;
        }
        got += r;
    }
    buf.truncate(got);
    Ok(buf)
}

/// Confere os magic bytes contra o formato esperado pela extensao. Erro `Parse` se nao baterem.
pub(crate) fn verify_magic(path: &Path, fmt: Format) -> Result<()> {
    let head = read_head(path, 32)?;
    let bad = |what: &'static str| AssetError::parse(what, "cabecalho invalido (arquivo truncado, corrompido ou com extensao errada)");
    match fmt {
        Format::Glb | Format::Vrm => {
            if head.len() < 12 || &head[..4] != b"glTF" {
                return Err(bad("glb"));
            }
        }
        Format::Fbx => {
            let binary = head.starts_with(b"Kaydara FBX Binary");
            let ascii = head.starts_with(b"; FBX") || head.starts_with(b"; Fbx");
            if !binary && !ascii {
                return Err(bad("fbx"));
            }
        }
        Format::Raster => {
            let png = head.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
            let jpg = head.starts_with(&[0xFF, 0xD8, 0xFF]);
            if !png && !jpg {
                return Err(bad("imagem"));
            }
        }
        Format::Gltf | Format::Obj | Format::OpaqueTexture | Format::AnigoMat | Format::AnigoPose => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_mapping() {
        for (name, fmt) in [
            ("a.GLB", Format::Glb),
            ("a.gltf", Format::Gltf),
            ("a.vrm", Format::Vrm),
            ("a.OBJ", Format::Obj),
            ("a.fbx", Format::Fbx),
            ("a.png", Format::Raster),
            ("a.JPEG", Format::Raster),
            ("a.exr", Format::OpaqueTexture),
            ("mat.anigomat.json", Format::AnigoMat),
            ("sentada.anigopose.json", Format::AnigoPose),
        ] {
            assert_eq!(format_by_extension(Path::new(name)), Some(fmt), "{name}");
        }
        for name in ["a.txt", "a", "a.json", "a.blend", ".png.bak", "a.zip"] {
            assert_eq!(format_by_extension(Path::new(name)), None, "{name}");
        }
        assert_eq!(format_label(Path::new("x.JPEG")), "jpg");
        assert_eq!(format_label(Path::new("x.anigomat.json")), "anigomat");
        assert_eq!(format_label(Path::new("x.AnigoPose.json")), "anigopose");
    }

    #[test]
    fn magic_checks() {
        let dir = tempfile::tempdir().unwrap();
        let w = |n: &str, b: &[u8]| {
            let p = dir.path().join(n);
            std::fs::write(&p, b).unwrap();
            p
        };
        assert!(verify_magic(&w("a.glb", b"glTF\x02\0\0\0\x14\0\0\0"), Format::Glb).is_ok());
        assert!(verify_magic(&w("b.glb", b"nope"), Format::Glb).is_err());
        assert!(verify_magic(&w("c.glb", b""), Format::Glb).is_err());
        assert!(verify_magic(&w("a.fbx", b"Kaydara FBX Binary  \0\x1a\0"), Format::Fbx).is_ok());
        assert!(verify_magic(&w("b.fbx", b"; FBX 7.4.0 project file"), Format::Fbx).is_ok());
        assert!(verify_magic(&w("c.fbx", b"garbage"), Format::Fbx).is_err());
        assert!(verify_magic(&w("a.png", &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0]), Format::Raster).is_ok());
        assert!(verify_magic(&w("a.jpg", &[0xFF, 0xD8, 0xFF, 0xE0]), Format::Raster).is_ok());
        assert!(verify_magic(&w("b.png", b"GIF89a"), Format::Raster).is_err());
        assert!(verify_magic(&dir.path().join("missing.png"), Format::Raster).is_err());
    }
}
