//! Analise de OBJ (+ MTL). Leitor em streaming com limite de linhas/bytes por linha; nunca confia no tamanho
//! declarado por quem escreveu o arquivo. Nao carrega a malha: so conta e mede.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use serde_json::Value;

use super::Analysis;
use crate::error::{AssetError, IoContext, Result};
use crate::limits::Limits;
use crate::material::convert::{parse_mtl, toon_from_pbr};
use crate::model::{AssetKind, AssetStats};
use crate::paths::{confine, normalize_relative};

/// Le uma linha (ate `\n`) com no maximo `max` bytes. `Ok(None)` no EOF; linha longa demais = erro.
fn read_line_limited<R: BufRead>(r: &mut R, buf: &mut Vec<u8>, max: usize) -> Result<Option<()>> {
    buf.clear();
    let n = r.by_ref().take(max as u64 + 1).read_until(b'\n', buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf.len() > max && !buf.ends_with(b"\n") {
        return Err(AssetError::LimitExceeded { what: "linha de texto", limit: max as u64, actual: buf.len() as u64 });
    }
    Ok(Some(()))
}

pub(crate) fn analyze_obj(path: &Path, limits: &Limits) -> Result<Analysis> {
    let meta = std::fs::metadata(path).at(path)?;
    Limits::check("arquivo OBJ", meta.len(), limits.max_file_bytes)?;
    let base_dir = path.parent().ok_or_else(|| AssetError::InvalidInput("arquivo sem pasta pai".into()))?;
    let mut reader = BufReader::with_capacity(256 * 1024, std::fs::File::open(path).at(path)?);

    let mut verts = 0u64;
    let mut tris = 0u64;
    let mut groups = 0u32;
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    let mut mtllibs: Vec<String> = Vec::new();
    let mut usemtl: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut lines = 0usize;
    let mut buf = Vec::with_capacity(256);

    while read_line_limited(&mut reader, &mut buf, limits.max_line_bytes)?.is_some() {
        lines += 1;
        if lines > limits.max_text_lines {
            return Err(AssetError::LimitExceeded { what: "linhas do OBJ", limit: limits.max_text_lines as u64, actual: lines as u64 });
        }
        let line = String::from_utf8_lossy(&buf);
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, rest) = match line.split_once(char::is_whitespace) {
            Some((k, r)) => (k, r.trim()),
            None => (line, ""),
        };
        match key {
            "v" => {
                let mut it = rest.split_whitespace().map(|t| t.parse::<f32>());
                if let (Some(Ok(x)), Some(Ok(y)), Some(Ok(z))) = (it.next(), it.next(), it.next()) {
                    if x.is_finite() && y.is_finite() && z.is_finite() {
                        verts += 1;
                        min = [min[0].min(x), min[1].min(y), min[2].min(z)];
                        max = [max[0].max(x), max[1].max(y), max[2].max(z)];
                        Limits::check("vertices", verts, limits.max_vertices)?;
                    }
                }
            }
            "f" => {
                let k = rest.split_whitespace().count() as u64;
                if k >= 3 {
                    tris += k - 2;
                }
            }
            "o" | "g" => groups = groups.saturating_add(1),
            "mtllib" => {
                for name in rest.split_whitespace() {
                    if !mtllibs.iter().any(|m| m == name) && mtllibs.len() < 32 {
                        mtllibs.push(name.to_string());
                    }
                }
            }
            "usemtl" => {
                if !usemtl.iter().any(|m| m == rest) && usemtl.len() < 4096 {
                    usemtl.push(rest.chars().take(256).collect());
                }
            }
            _ => {}
        }
    }
    if verts == 0 || tris == 0 {
        return Err(AssetError::parse("obj", "nenhum vertice/face valido (arquivo vazio, truncado ou nao e OBJ)"));
    }

    // MTL -> materiais
    let mut dependents: Vec<String> = Vec::new();
    let mut materials = Vec::new();
    for lib in &mtllibs {
        let rel = match normalize_relative(lib).and_then(|r| confine(base_dir, &r).map(|p| (r, p))) {
            Ok(x) => x,
            Err(e) => {
                warnings.push(format!("mtllib {lib:?} ignorada: {e}"));
                continue;
            }
        };
        let (rel, p) = rel;
        let Ok(m) = std::fs::metadata(&p) else {
            warnings.push(format!("mtllib ausente: {rel}"));
            continue;
        };
        if m.len() > 16 * 1024 * 1024 {
            warnings.push(format!("mtllib {rel} grande demais (>16 MiB); ignorada"));
            continue;
        }
        let text = String::from_utf8_lossy(&std::fs::read(&p).at(&p)?).into_owned();
        if !dependents.contains(&rel) {
            dependents.push(rel.clone());
        }
        let mtl_dir = rel.rsplit_once('/').map(|(d, _)| d.to_string());
        let mut mw = Vec::new();
        for mut pbr in parse_mtl(&text, limits.max_text_lines.min(1_000_000), limits.max_materials.min(4096), &mut mw) {
            // texturas do MTL sao relativas a pasta do MTL
            for t in [&mut pbr.base_color_texture, &mut pbr.normal_texture, &mut pbr.emissive_texture].into_iter().flatten() {
                if let (Some(dir), Some(u)) = (&mtl_dir, t.uri.clone()) {
                    t.uri = Some(format!("{dir}/{u}"));
                }
                if let Some(u) = &t.uri {
                    match confine(base_dir, u) {
                        Ok(tp) if tp.is_file() => {
                            if !dependents.contains(u) {
                                dependents.push(u.clone());
                            }
                        }
                        Ok(_) => mw.push(format!("textura ausente: {u}")),
                        Err(e) => mw.push(format!("textura ignorada: {e}")),
                    }
                }
            }
            materials.push(toon_from_pbr(&pbr, "obj-mtl"));
        }
        warnings.extend(mw);
    }
    Limits::check("arquivos dependentes", dependents.len() as u64, limits.max_files_per_asset as u64)?;

    let stats = AssetStats {
        tris,
        verts,
        meshes: groups.max(1),
        materials: usemtl.len().max(materials.len()) as u32,
        textures: dependents.len().saturating_sub(mtllibs.len().min(dependents.len())) as u32,
        bbox: Some([min[0], min[1], min[2], max[0], max[1], max[2]]),
        ..Default::default()
    };
    Ok(Analysis {
        kind: AssetKind::Model,
        stats,
        dependents,
        materials,
        suggested_name: None,
        author: None,
        license: None,
        props: Value::Null,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, bytes).unwrap();
        p
    }

    const CUBE: &str = "# cubo\nmtllib cube.mtl\no Cube\nv -1 -1 -1\nv 1 -1 -1\nv 1 1 -1\nv -1 1 -1\nv -1 -1 1\nv 1 -1 1\nv 1 1 1\nv -1 1 1\nusemtl Red\nf 1 2 3 4\nf 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf 4 1 5 8\n";

    #[test]
    fn analyzes_cube_with_mtl_and_texture() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "cube.mtl", b"newmtl Red\nKd 1 0 0\nmap_Kd tex/red.png\n");
        write(dir.path(), "tex/red.png", b"x");
        let p = write(dir.path(), "cube.obj", CUBE.as_bytes());
        let a = analyze_obj(&p, &Limits::default()).unwrap();
        assert_eq!(a.stats.verts, 8);
        assert_eq!(a.stats.tris, 12); // 6 quads
        assert_eq!(a.stats.bbox, Some([-1.0, -1.0, -1.0, 1.0, 1.0, 1.0]));
        assert_eq!(a.materials.len(), 1);
        assert_eq!(a.materials[0].converted_from.as_deref(), Some("obj-mtl"));
        assert!(a.dependents.contains(&"cube.mtl".to_string()));
        assert!(a.dependents.contains(&"tex/red.png".to_string()));
        assert_eq!(a.materials[0].base.texture.as_ref().unwrap().uri.as_deref(), Some("tex/red.png"));
    }

    #[test]
    fn mtl_texture_paths_are_relative_to_the_mtl_dir() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "mats/cube.mtl", b"newmtl Red\nmap_Kd red.png\n");
        write(dir.path(), "mats/red.png", b"x");
        let p = write(dir.path(), "cube.obj", CUBE.replace("cube.mtl", "mats/cube.mtl").as_bytes());
        let a = analyze_obj(&p, &Limits::default()).unwrap();
        assert!(a.dependents.contains(&"mats/red.png".to_string()), "{:?}", a.dependents);
    }

    #[test]
    fn missing_or_unsafe_mtl_is_a_warning_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "a.obj", b"mtllib ../../etc/passwd nope.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n");
        let a = analyze_obj(&p, &Limits::default()).unwrap();
        assert_eq!(a.stats.tris, 1);
        assert!(a.materials.is_empty());
        assert_eq!(a.warnings.len(), 2, "{:?}", a.warnings);
    }

    #[test]
    fn garbage_truncated_and_hostile_inputs() {
        let dir = tempfile::tempdir().unwrap();
        for (n, bytes) in [
            ("empty.obj", &b""[..]),
            ("binary.obj", &[0u8, 255, 1, 254, 2, 253][..]),
            ("nofaces.obj", b"v 0 0 0\nv 1 1 1\n"),
            ("nan.obj", b"v nan nan nan\nv inf 0 0\nf 1 2 3\n"),
            ("text.obj", b"hello world\nthis is not an obj\n"),
        ] {
            let p = write(dir.path(), n, bytes);
            assert!(analyze_obj(&p, &Limits::default()).is_err(), "{n}");
        }
        // faces com indices absurdos e degeneradas nao quebram a contagem
        let p = write(dir.path(), "weird.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 99999999999 -5 0\nf 1\nf 1 2\nf 1 2 3\n");
        let a = analyze_obj(&p, &Limits::default()).unwrap();
        assert_eq!(a.stats.tris, 2);
        // linha gigante
        let mut big = b"v 0 0 0\n".to_vec();
        big.extend(std::iter::repeat(b'a').take(5000));
        let p = write(dir.path(), "long.obj", &big);
        let tight = Limits { max_line_bytes: 1000, ..Limits::default() };
        assert!(matches!(analyze_obj(&p, &tight), Err(AssetError::LimitExceeded { .. })));
        // limite de linhas
        let tight = Limits { max_text_lines: 3, ..Limits::default() };
        let p = write(dir.path(), "many.obj", CUBE.as_bytes());
        assert!(matches!(analyze_obj(&p, &tight), Err(AssetError::LimitExceeded { .. })));
        // limite de vertices
        let tight = Limits { max_vertices: 4, ..Limits::default() };
        assert!(matches!(analyze_obj(&p, &tight), Err(AssetError::LimitExceeded { .. })));
    }

    #[test]
    fn handles_crlf_and_utf8_lossy() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "crlf.obj", b"o Caf\xe9\r\nv 0 0 0\r\nv 1 0 0\r\nv 0 1 0\r\nf 1 2 3\r\n");
        let a = analyze_obj(&p, &Limits::default()).unwrap();
        assert_eq!(a.stats.tris, 1);
    }
}
