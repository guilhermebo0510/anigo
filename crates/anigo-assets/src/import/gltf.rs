//! Analise de GLB / glTF / VRM. Reaproveita o parser de `anigo-vrm` (`parse_glb`, `parse_gltf`,
//! `summarize_import`) e acrescenta: limites de seguranca, buffers/imagens externos confinados a pasta do
//! arquivo, estatisticas e extracao dos materiais para o formato canonico.

use std::cell::RefCell;
use std::path::Path;

use anigo_vrm::gltf::{parse_glb, parse_gltf, summarize_import, GltfError, GltfErrorCode};
use serde_json::Value;

use super::detect::Format;
use super::Analysis;
use crate::error::{AssetError, IoContext, Result};
use crate::limits::Limits;
use crate::material::convert::convert_gltf_materials;
use crate::model::{AssetKind, AssetStats};
use crate::paths::{confine, normalize_relative};

use base64::Engine as _;

fn map_gltf_err(format: &'static str, e: GltfError) -> AssetError {
    AssetError::parse(format, e.to_string())
}

fn arr_len(json: &Value, key: &str) -> usize {
    json.get(key).and_then(Value::as_array).map(|a| a.len()).unwrap_or(0)
}

/// Confere contagens estruturais contra os limites.
fn check_structure(json: &Value, limits: &Limits) -> Result<()> {
    for (what, key, max) in [
        ("nodes", "nodes", limits.max_nodes),
        ("meshes", "meshes", limits.max_meshes),
        ("materials", "materials", limits.max_materials),
        ("accessors", "accessors", limits.max_accessors),
        ("images", "images", limits.max_images),
        ("animations", "animations", limits.max_animations),
        ("textures", "textures", limits.max_images * 2),
        ("bufferViews", "bufferViews", limits.max_accessors),
    ] {
        Limits::check(what, arr_len(json, key) as u64, max as u64)?;
    }
    Ok(())
}

/// Le o arquivo respeitando o limite de bytes.
fn read_limited(path: &Path, max: u64, what: &'static str) -> Result<Vec<u8>> {
    let meta = std::fs::metadata(path).at(path)?;
    Limits::check(what, meta.len(), max)?;
    std::fs::read(path).at(path)
}

pub(crate) fn analyze_gltf(path: &Path, fmt: Format, limits: &Limits) -> Result<Analysis> {
    let label: &'static str = if fmt == Format::Gltf { "gltf" } else { "glb" };
    let base_dir = path.parent().ok_or_else(|| AssetError::InvalidInput("arquivo sem pasta pai".into()))?;
    let bytes = read_limited(path, if fmt == Format::Gltf { limits.max_json_bytes } else { limits.max_parse_bytes }, "arquivo glTF/GLB")?;

    let (json, bin): (Value, Vec<u8>) = match fmt {
        Format::Gltf => (serde_json::from_slice(&bytes).map_err(|e| AssetError::parse(label, format!("JSON invalido: {e}")))?, Vec::new()),
        _ => {
            if bytes.len() >= 20 {
                let json_len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as u64;
                Limits::check("json do GLB", json_len, limits.max_json_bytes)?;
            }
            parse_glb(&bytes).map_err(|e| map_gltf_err(label, e))?
        }
    };
    drop(bytes);
    if !json.is_object() {
        return Err(AssetError::parse(label, "raiz do JSON nao e um objeto"));
    }
    check_structure(&json, limits)?;
    // NOTA: `anigo_vrm::gltf::parse_gltf` procura `extensionsRequired` em `/asset/...`; na spec glTF ele fica na
    // raiz do documento. Conferimos aqui no lugar correto.
    let mut early_warnings: Vec<String> = Vec::new();
    for ext in json.get("extensionsRequired").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        match ext {
            "KHR_draco_mesh_compression" => {
                return Err(AssetError::Unsupported("KHR_draco_mesh_compression obrigatoria: Draco ainda nao e decodificado (descomprima com gltf-transform/gltfpack antes)".into()))
            }
            "EXT_meshopt_compression" => early_warnings.push("EXT_meshopt_compression obrigatoria: catalogado, mas o carregador atual nao decodifica meshopt".into()),
            _ => {}
        }
    }

    // bytes declarados por buffer (para tolerar o padding de ate 3 bytes do chunk BIN)
    let declared: Vec<u64> = json.get("buffers").and_then(Value::as_array).map(|a| a.iter().map(|b| b.get("byteLength").and_then(Value::as_u64).unwrap_or(0)).collect()).unwrap_or_default();
    for d in &declared {
        Limits::check("buffer", *d, limits.max_parse_bytes)?;
    }

    let dependents: RefCell<Vec<String>> = RefCell::new(Vec::new());
    let fail: RefCell<Option<AssetError>> = RefCell::new(None);
    let budget: RefCell<u64> = RefCell::new(0);

    let resolver = |i: usize, uri: Option<&str>| -> std::result::Result<Vec<u8>, GltfError> {
        let set_err = |e: AssetError| {
            let msg = e.to_string();
            *fail.borrow_mut() = Some(e);
            GltfError::new(GltfErrorCode::MissingBuffer, msg)
        };
        match uri {
            None => {
                if fmt == Format::Gltf {
                    return Err(set_err(AssetError::parse(label, format!("buffers[{i}] sem uri em .gltf"))));
                }
                let want = declared.get(i).copied().unwrap_or(0) as usize;
                if bin.len() > want && bin.len() - want < 4 {
                    Ok(bin[..want].to_vec())
                } else {
                    Ok(bin.clone())
                }
            }
            Some(u) if u.starts_with("data:") => {
                let comma = u.find(',').ok_or_else(|| set_err(AssetError::parse(label, "data URI sem virgula")))?;
                if !u[..comma].ends_with(";base64") {
                    return Err(set_err(AssetError::parse(label, "data URI de buffer deve ser base64")));
                }
                let max_b64 = (limits.max_parse_bytes / 3 * 4 + 8) as usize;
                if u.len() - comma > max_b64 {
                    return Err(set_err(AssetError::LimitExceeded { what: "buffer data URI", limit: limits.max_parse_bytes, actual: u.len() as u64 }));
                }
                let dec = base64::engine::general_purpose::STANDARD
                    .decode(&u[comma + 1..])
                    .map_err(|e| set_err(AssetError::parse(label, format!("base64 invalido: {e}"))))?;
                Ok(dec)
            }
            Some(u) => {
                let rel = normalize_relative(u).map_err(&set_err)?;
                let p = confine(base_dir, &rel).map_err(&set_err)?;
                let meta = std::fs::metadata(&p).map_err(|e| set_err(AssetError::io(&p, e)))?;
                if let Err(e) = Limits::check("buffer externo", meta.len(), limits.max_file_bytes) {
                    return Err(set_err(e));
                }
                {
                    let mut b = budget.borrow_mut();
                    *b += meta.len();
                    if let Err(e) = Limits::check("soma de buffers", *b, limits.max_parse_bytes) {
                        return Err(set_err(e));
                    }
                }
                let data = std::fs::read(&p).map_err(|e| set_err(AssetError::io(&p, e)))?;
                let mut d = dependents.borrow_mut();
                if !d.contains(&rel) {
                    d.push(rel);
                }
                Ok(data)
            }
        }
    };

    let parsed = match parse_gltf(json, resolver) {
        Ok(p) => p,
        Err(e) => {
            if let Some(real) = fail.into_inner() {
                return Err(real);
            }
            return Err(map_gltf_err(label, e));
        }
    };
    let json = parsed.json;
    let mut dependents = dependents.into_inner();
    let mut warnings: Vec<String> = early_warnings;
    warnings.extend(parsed.warnings);

    // imagens externas: dependentes (ausentes viram aviso, nao erro)
    let mut dep_bytes = 0u64;
    if let Some(images) = json.get("images").and_then(Value::as_array) {
        for (i, im) in images.iter().enumerate() {
            let Some(u) = im.get("uri").and_then(Value::as_str) else { continue };
            if u.starts_with("data:") {
                continue;
            }
            match normalize_relative(u).and_then(|rel| confine(base_dir, &rel).map(|p| (rel, p))) {
                Ok((rel, p)) => match std::fs::metadata(&p) {
                    Ok(m) if m.is_file() => {
                        Limits::check("imagem externa", m.len(), limits.max_file_bytes)?;
                        dep_bytes += m.len();
                        if !dependents.contains(&rel) {
                            dependents.push(rel);
                        }
                    }
                    _ => warnings.push(format!("images[{i}]: arquivo ausente: {rel}")),
                },
                Err(e) => return Err(e),
            }
        }
    }
    Limits::check("soma dos arquivos dependentes", dep_bytes, limits.max_asset_bytes)?;

    // estatisticas
    let sum = summarize_import(&json);
    Limits::check("vertices", sum.vertex_count, limits.max_vertices)?;
    let bones: u32 = json
        .get("skins")
        .and_then(Value::as_array)
        .map(|s| s.iter().map(|k| k.get("joints").and_then(Value::as_array).map(|j| j.len()).unwrap_or(0) as u32).fold(0u32, |a, b| a.saturating_add(b)))
        .unwrap_or(0);
    let mut stats = AssetStats {
        tris: sum.triangle_count,
        verts: sum.vertex_count,
        meshes: sum.mesh_count as u32,
        nodes: sum.node_count as u32,
        materials: sum.material_count as u32,
        textures: sum.texture_count as u32,
        bones,
        morphs: sum.morph_target_count.min(u32::MAX as u64) as u32,
        anims: sum.animation_count as u32,
        has_skin: sum.skin_count > 0,
        has_vrm: sum.is_vrm,
        bbox: bbox_from_accessors(&json),
        width: 0,
        height: 0,
    };
    // `summarize_import` procura `extensionsUsed` em `/asset/...` (fora da spec); detectamos no lugar certo.
    let vrm1 = json.pointer("/extensions/VRMC_vrm").is_some()
        || json.get("extensionsUsed").and_then(Value::as_array).map(|a| a.iter().any(|v| v.as_str() == Some("VRMC_vrm"))).unwrap_or(false);
    if vrm1 {
        stats.has_vrm = true;
    }
    let legacy_vrm = json.pointer("/extensions/VRM").is_some();
    if legacy_vrm {
        stats.has_vrm = true;
        warnings.push("VRM 0.x detectado: materiais MToon legados nao foram convertidos (use VRM 1.0)".into());
    }

    // metadados VRM 1.0 / glTF
    let mut props = serde_json::Map::new();
    let mut name: Option<String> = None;
    let mut author: Option<String> = None;
    let mut license: Option<String> = None;
    if let Some(meta) = json.pointer("/extensions/VRMC_vrm/meta") {
        name = meta.get("name").or_else(|| meta.get("title")).and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(|s| s.chars().take(256).collect());
        author = meta
            .get("authors")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
            .filter(|s| !s.is_empty())
            .map(|s| s.chars().take(256).collect());
        license = meta.get("licenseUrl").and_then(Value::as_str).filter(|s| !s.is_empty()).map(|s| s.chars().take(256).collect());
        props.insert("vrm_meta".into(), meta.clone());
    }
    if let Some(gen) = json.pointer("/asset/generator").and_then(Value::as_str) {
        props.insert("generator".into(), Value::String(gen.chars().take(256).collect()));
    }
    props.insert("gltf_version".into(), Value::String(sum.gltf_version.clone()));
    if !sum.vrm_extensions.is_empty() {
        props.insert("vrm_extensions".into(), Value::from(sum.vrm_extensions.clone()));
    }
    if author.is_none() {
        author = json.pointer("/asset/copyright").and_then(Value::as_str).filter(|s| !s.is_empty()).map(|s| s.chars().take(256).collect());
    }
    if let Some(sc) = json.get("extensionsUsed").and_then(Value::as_array) {
        props.insert("extensions_used".into(), Value::Array(sc.iter().filter(|v| v.is_string()).take(64).cloned().collect()));
    }

    // materiais canonicos
    let mut materials = Vec::new();
    const MAX_MATERIALS_EXTRACTED: usize = 256;
    if sum.material_count > MAX_MATERIALS_EXTRACTED {
        warnings.push(format!("{} materiais: apenas os {MAX_MATERIALS_EXTRACTED} primeiros foram convertidos", sum.material_count));
    }
    for c in convert_gltf_materials(&json, MAX_MATERIALS_EXTRACTED) {
        for w in c.warnings {
            warnings.push(format!("material {:?}: {w}", c.material.name));
        }
        materials.push(c.material);
    }

    let has_humanoid = json.pointer("/extensions/VRMC_vrm/humanoid").is_some();
    let kind = if has_humanoid {
        AssetKind::Character
    } else if sum.mesh_count == 0 && sum.animation_count > 0 {
        AssetKind::Animation
    } else {
        AssetKind::Model
    };

    Ok(Analysis {
        kind,
        stats,
        dependents,
        materials,
        suggested_name: name,
        author,
        license,
        props: Value::Object(props),
        warnings,
    })
}

fn bbox_from_accessors(json: &Value) -> Option<[f32; 6]> {
    let accessors = json.get("accessors")?.as_array()?;
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    let mut any = false;
    for mesh in json.get("meshes")?.as_array()? {
        for prim in mesh.get("primitives").and_then(Value::as_array).into_iter().flatten() {
            let Some(pos) = prim.pointer("/attributes/POSITION").and_then(Value::as_u64) else { continue };
            let Some(acc) = accessors.get(pos as usize) else { continue };
            let (Some(mn), Some(mx)) = (acc.get("min").and_then(Value::as_array), acc.get("max").and_then(Value::as_array)) else { continue };
            if mn.len() < 3 || mx.len() < 3 {
                continue;
            }
            for i in 0..3 {
                let (Some(a), Some(b)) = (mn[i].as_f64(), mx[i].as_f64()) else { return None };
                if !a.is_finite() || !b.is_finite() {
                    return None;
                }
                min[i] = min[i].min(a as f32);
                max[i] = max[i].max(b as f32);
            }
            any = true;
        }
    }
    if any {
        Some([min[0], min[1], min[2], max[0], max[1], max[2]])
    } else {
        None
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Geradores de GLB/glTF minimos para testes (um triangulo).
    use serde_json::{json, Value};

    pub fn triangle_json(extra_material: Option<Value>) -> (Value, Vec<u8>) {
        let mut bin: Vec<u8> = Vec::new();
        for v in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 2.0, 0.5]] {
            for c in v {
                bin.extend_from_slice(&c.to_le_bytes());
            }
        }
        for i in [0u16, 1, 2] {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let mat = extra_material.unwrap_or_else(|| json!({"name": "Mat", "pbrMetallicRoughness": {"baseColorFactor": [0.5, 0.5, 0.5, 1.0]}}));
        let j = json!({
            "asset": {"version": "2.0", "generator": "test"},
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [{"mesh": 0, "name": "tri"}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1, "material": 0}]}],
            "materials": [mat],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0.0, 0.0, 0.0], "max": [1.0, 2.0, 0.5]},
                {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}
            ],
            "bufferViews": [
                {"buffer": 0, "byteOffset": 0, "byteLength": 36},
                {"buffer": 0, "byteOffset": 36, "byteLength": 6}
            ],
            "buffers": [{"byteLength": 42}]
        });
        (j, bin)
    }

    /// GLB valido com um triangulo.
    pub fn triangle_glb() -> Vec<u8> {
        let (j, bin) = triangle_json(None);
        anigo_vrm::gltf::build_glb(&j, &bin)
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use serde_json::json;

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn analyzes_a_valid_glb() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "tri.glb", &triangle_glb());
        let a = analyze_gltf(&p, Format::Glb, &Limits::default()).unwrap();
        assert_eq!(a.stats.tris, 1);
        assert_eq!(a.stats.verts, 3);
        assert_eq!(a.stats.meshes, 1);
        assert_eq!(a.stats.materials, 1);
        assert_eq!(a.stats.bbox, Some([0.0, 0.0, 0.0, 1.0, 2.0, 0.5]));
        assert_eq!(a.kind, AssetKind::Model);
        assert_eq!(a.materials.len(), 1);
        assert_eq!(a.materials[0].converted_from.as_deref(), Some("gltf-pbr"));
        assert!(a.dependents.is_empty());
    }

    #[test]
    fn tolerates_bin_chunk_padding() {
        // buffer declara 42 bytes; o chunk BIN tem 44 (padding)
        let dir = tempfile::tempdir().unwrap();
        let (j, mut bin) = triangle_json(None);
        assert_eq!(bin.len(), 44);
        bin.truncate(44);
        let glb = anigo_vrm::gltf::build_glb(&j, &bin);
        let p = write(dir.path(), "pad.glb", &glb);
        assert!(analyze_gltf(&p, Format::Glb, &Limits::default()).is_ok());
    }

    #[test]
    fn rejects_truncated_and_corrupt_glb_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let good = triangle_glb();
        // truncamentos em todas as fronteiras interessantes (e uma varredura)
        for n in (0..good.len()).step_by(1) {
            let p = write(dir.path(), "t.glb", &good[..n]);
            let r = analyze_gltf(&p, Format::Glb, &Limits::default());
            assert!(r.is_err(), "truncado em {n} bytes deveria falhar");
        }
        // bytes corrompidos: nunca panic (pode falhar ou, raramente, passar)
        for i in 0..good.len() {
            let mut bad = good.clone();
            bad[i] ^= 0xFF;
            let p = write(dir.path(), "c.glb", &bad);
            let _ = analyze_gltf(&p, Format::Glb, &Limits::default());
        }
        // lixo puro
        let p = write(dir.path(), "g.glb", &[0xAB; 4096]);
        assert!(analyze_gltf(&p, Format::Glb, &Limits::default()).is_err());
        let p = write(dir.path(), "e.glb", b"");
        assert!(analyze_gltf(&p, Format::Glb, &Limits::default()).is_err());
    }

    #[test]
    fn external_buffer_and_image_become_dependents() {
        let dir = tempfile::tempdir().unwrap();
        let (mut j, bin) = triangle_json(Some(json!({"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}})));
        j["buffers"][0]["uri"] = json!("tri.bin");
        j["images"] = json!([{"uri": "tex/a.png"}, {"uri": "tex/missing.png"}]);
        j["textures"] = json!([{"source": 0}, {"source": 1}]);
        std::fs::create_dir_all(dir.path().join("tex")).unwrap();
        write(dir.path(), "tex/a.png", b"png");
        write(dir.path(), "tri.bin", &bin[..42]);
        let p = write(dir.path(), "tri.gltf", &serde_json::to_vec(&j).unwrap());
        let a = analyze_gltf(&p, Format::Gltf, &Limits::default()).unwrap();
        assert!(a.dependents.contains(&"tri.bin".to_string()));
        assert!(a.dependents.contains(&"tex/a.png".to_string()));
        assert_eq!(a.dependents.len(), 2);
        assert!(a.warnings.iter().any(|w| w.contains("missing.png")));
        assert_eq!(a.materials[0].base.texture.as_ref().unwrap().uri.as_deref(), Some("tex/a.png"));
    }

    #[test]
    fn path_traversal_in_uris_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let secret = dir.path().join("secret.bin");
        std::fs::write(&secret, vec![0u8; 42]).unwrap();
        let sub = dir.path().join("asset");
        std::fs::create_dir_all(&sub).unwrap();
        for uri in ["../secret.bin", "/etc/passwd", "file:///etc/passwd", "http://x/y.bin", "..%2Fsecret.bin", "%2e%2e/secret.bin"] {
            let (mut j, _) = triangle_json(None);
            j["buffers"][0]["uri"] = json!(uri);
            let p = write(&sub, "evil.gltf", &serde_json::to_vec(&j).unwrap());
            let r = analyze_gltf(&p, Format::Gltf, &Limits::default());
            assert!(matches!(r, Err(AssetError::UnsafePath(_))), "{uri}: {r:?}");
        }
        // imagem com traversal
        let (mut j, bin) = triangle_json(None);
        j["buffers"][0]["uri"] = json!("tri.bin");
        write(&sub, "tri.bin", &bin[..42]);
        j["images"] = json!([{"uri": "../secret.bin"}]);
        let p = write(&sub, "evil2.gltf", &serde_json::to_vec(&j).unwrap());
        assert!(matches!(analyze_gltf(&p, Format::Gltf, &Limits::default()), Err(AssetError::UnsafePath(_))));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("b.bin"), vec![0u8; 42]).unwrap();
        let sub = dir.path().join("asset");
        std::fs::create_dir_all(&sub).unwrap();
        std::os::unix::fs::symlink(outside.path(), sub.join("lnk")).unwrap();
        let (mut j, _) = triangle_json(None);
        j["buffers"][0]["uri"] = json!("lnk/b.bin");
        let p = write(&sub, "s.gltf", &serde_json::to_vec(&j).unwrap());
        assert!(matches!(analyze_gltf(&p, Format::Gltf, &Limits::default()), Err(AssetError::UnsafePath(_))));
    }

    #[test]
    fn limits_are_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "tri.glb", &triangle_glb());
        let tight = Limits { max_nodes: 0, ..Limits::default() };
        // 1 no > 0
        assert!(matches!(analyze_gltf(&p, Format::Glb, &tight), Err(AssetError::LimitExceeded { .. })));
        let tight = Limits { max_vertices: 2, ..Limits::default() };
        assert!(matches!(analyze_gltf(&p, Format::Glb, &tight), Err(AssetError::LimitExceeded { .. })));
        let tight = Limits { max_parse_bytes: 10, ..Limits::default() };
        assert!(matches!(analyze_gltf(&p, Format::Glb, &tight), Err(AssetError::LimitExceeded { .. })));
        let tight = Limits { max_json_bytes: 10, ..Limits::default() };
        assert!(matches!(analyze_gltf(&p, Format::Glb, &tight), Err(AssetError::LimitExceeded { .. })));
    }

    #[test]
    fn accessor_out_of_range_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (mut j, bin) = triangle_json(None);
        j["bufferViews"][0]["byteLength"] = json!(100000);
        let p = write(dir.path(), "bad.glb", &anigo_vrm::gltf::build_glb(&j, &bin));
        assert!(matches!(analyze_gltf(&p, Format::Glb, &Limits::default()), Err(AssetError::Parse { .. })));
        // indice de material fora da faixa
        let (mut j, bin) = triangle_json(None);
        j["meshes"][0]["primitives"][0]["material"] = json!(99);
        let p = write(dir.path(), "bad2.glb", &anigo_vrm::gltf::build_glb(&j, &bin));
        assert!(analyze_gltf(&p, Format::Glb, &Limits::default()).is_err());
        // ciclo de nos
        let (mut j, bin) = triangle_json(None);
        j["nodes"] = json!([{"children": [1]}, {"children": [0]}]);
        let p = write(dir.path(), "bad3.glb", &anigo_vrm::gltf::build_glb(&j, &bin));
        assert!(analyze_gltf(&p, Format::Glb, &Limits::default()).is_err());
    }

    #[test]
    fn vrm_metadata_and_kind() {
        let dir = tempfile::tempdir().unwrap();
        let (mut j, bin) = triangle_json(Some(json!({"name": "Face", "extensions": {"VRMC_materials_mtoon": {"specVersion": "1.0", "shadeColorFactor": [0.5, 0.5, 0.5]}}})));
        j["extensionsUsed"] = json!(["VRMC_vrm", "VRMC_materials_mtoon"]);
        j["extensions"] = json!({"VRMC_vrm": {"specVersion": "1.0", "meta": {"name": "Aiko", "authors": ["Ana", "Bia"], "licenseUrl": "https://vrm.dev/licenses/1.0/"}, "humanoid": {"humanBones": {}}}});
        let p = write(dir.path(), "aiko.vrm", &anigo_vrm::gltf::build_glb(&j, &bin));
        let a = analyze_gltf(&p, Format::Vrm, &Limits::default()).unwrap();
        assert_eq!(a.kind, AssetKind::Character);
        assert!(a.stats.has_vrm);
        assert_eq!(a.suggested_name.as_deref(), Some("Aiko"));
        assert_eq!(a.author.as_deref(), Some("Ana, Bia"));
        assert_eq!(a.license.as_deref(), Some("https://vrm.dev/licenses/1.0/"));
        assert_eq!(a.materials[0].converted_from.as_deref(), Some("vrm-mtoon"));
    }

    #[test]
    fn animation_only_gltf_is_an_animation() {
        let dir = tempfile::tempdir().unwrap();
        let j = json!({"asset": {"version": "2.0"}, "nodes": [{}], "animations": [{"channels": [], "samplers": []}]});
        let p = write(dir.path(), "a.glb", &anigo_vrm::gltf::build_glb(&j, &[]));
        let a = analyze_gltf(&p, Format::Glb, &Limits::default()).unwrap();
        assert_eq!(a.kind, AssetKind::Animation);
        assert_eq!(a.stats.anims, 1);
    }

    #[test]
    fn data_uri_buffer_is_supported() {
        let dir = tempfile::tempdir().unwrap();
        let (mut j, bin) = triangle_json(None);
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bin[..42]);
        j["buffers"][0]["uri"] = json!(format!("data:application/octet-stream;base64,{b64}"));
        let p = write(dir.path(), "d.gltf", &serde_json::to_vec(&j).unwrap());
        let a = analyze_gltf(&p, Format::Gltf, &Limits::default()).unwrap();
        assert_eq!(a.stats.tris, 1);
        assert!(a.dependents.is_empty());
        // base64 invalido
        j["buffers"][0]["uri"] = json!("data:application/octet-stream;base64,@@@");
        let p = write(dir.path(), "d2.gltf", &serde_json::to_vec(&j).unwrap());
        assert!(analyze_gltf(&p, Format::Gltf, &Limits::default()).is_err());
        // data URI nao-base64
        j["buffers"][0]["uri"] = json!("data:text/plain,abc");
        let p = write(dir.path(), "d3.gltf", &serde_json::to_vec(&j).unwrap());
        assert!(analyze_gltf(&p, Format::Gltf, &Limits::default()).is_err());
    }
}
