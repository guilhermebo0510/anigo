//! ANIGO VRM, glTF, and BlendShape Management Engine (Fase 2, issue #25).
//!
//! Interoperabilidade glTF 2.0 + VRM 1.0 no lado Rust — espelho do módulo
//! TypeScript `src/services/vrm/` (que é a via primária do viewport):
//!
//!  - [`gltf`]    — parser glTF 2.0 completo (GLB + buffers externos,
//!                  accessors interleaved/quantizados/sparsos, validação com
//!                  códigos estáveis), contêiner GLB e sumário de import;
//!  - [`vrm1`]    — extensões VRM 1.0 (VRMC_vrm, VRMC_materials_mtoon,
//!                  VRMC_springBone, VRMC_node_constraint) e mapeamento
//!                  MToon → material anime do ANIGO;
//!  - [`exporter`] — exportador determinístico (mesma cena → bytes idênticos)
//!                  para `.glb`/`.gltf`/`.vrm`.
//!
//! `parse_model` / `parse_vrm` / `validate_model` / `export_model` formam a
//! fronteira usada pelos comandos Tauri (`src-tauri/src/main.rs`).
//!
//! Nota de ambiente: sem `cargo` no sandbox — validação de sintaxe via
//! `scripts/check_rust_syntax.mjs`; `cargo test` roda na CI (issue #59).

pub mod exporter;
pub mod gltf;
pub mod vrm1;

pub use exporter::{
    export_glb, export_gltf, ExportAnimation, ExportAnimationTrack, ExportMaterial, ExportMToon,
    ExportMesh, ExportMorphDelta, ExportNode, ExportPrimitive, ExportScene, ExportSkin, ExportVertex,
    VrmExportData,
};
pub use gltf::{
    build_glb, parse_glb, parse_gltf, read_accessor_f32, read_accessor_indices, resolve_glb_buffers,
    mip_level_count, summarize_import, GltfError, GltfErrorCode, ImportSummary, ParsedGltf,
};
pub use vrm1::{
    is_vrm_document, map_mtoon_to_anime_material, parse_vrm1, MToonMaterialMapping, ParsedVrm1,
    Vrm1Error, Vrm1ErrorCode, Vrm1Meta, Vrm1MToon, Vrm1SpringBone,
};

// ---------------------------------------------------------------------------
// API de alto nível (fronteira Tauri)
// ---------------------------------------------------------------------------

impl From<Vrm1Error> for GltfError {
    fn from(err: Vrm1Error) -> Self {
        GltfError { code: GltfErrorCode::BadAsset, message: err.to_string() }
    }
}

/// Resultado de `parse_model`: modelo glTF parseado + camada VRM (se for).
#[derive(Debug, Clone)]
pub struct ParseModelResult {
    pub model: ParsedGltf,
    pub vrm: Option<ParsedVrm1>,
}

/// Parse completo de bytes GLB/VRM: contêiner → glTF → camadas VRM 1.0.
/// Erros de contêiner viram `GltfError` com código estável.
pub fn parse_model(bytes: &[u8]) -> Result<ParseModelResult, GltfError> {
    let (json, bin) = parse_glb(bytes)?;
    let (buffers, warnings) = resolve_glb_buffers(&json, &bin, |_uri| None)?;
    let buffer_index = |index: usize, _uri: Option<&str>| -> Result<Vec<u8>, GltfError> {
        buffers.get(index).cloned().ok_or_else(|| {
            GltfError::new(GltfErrorCode::BadBufferIndex, format!("buffer {index} ausente no GLB"))
        })
    };
    let mut model = parse_gltf(json, buffer_index)?;
    model.warnings.extend(warnings);
    let vrm = if is_vrm_document(&model.json) { Some(parse_vrm1(&model)?) } else { None };
    Ok(ParseModelResult { model, vrm })
}

/// Parse de `.vrm` — falha com `Vrm1Error` quando o documento não é VRM 1.0.
pub fn parse_vrm(bytes: &[u8]) -> Result<(ParsedGltf, ParsedVrm1), Vrm1Error> {
    let result = parse_model(bytes)
        .map_err(|e| Vrm1Error { code: Vrm1ErrorCode::UnknownError, message: e.to_string() })?;
    match result.vrm {
        Some(vrm) => Ok((result.model, vrm)),
        None => Err(Vrm1Error {
            code: Vrm1ErrorCode::NotVrm,
            message: "bytes não formam um .vrm válido (sem VRMC_vrm)".to_string(),
        }),
    }
}

/// Resultado transportável pela fronteira Tauri (nunca lança).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub error: Option<ValidationError>,
    pub summary: Option<ImportSummary>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ValidationError {
    pub code: String,
    pub message: String,
}

/// Validação estrutural completa: contêiner + glTF + camadas VRM.
pub fn validate_model(bytes: &[u8]) -> ValidationResult {
    match parse_model(bytes) {
        Ok(result) => ValidationResult {
            valid: true,
            error: None,
            summary: Some(summarize_import(&result.model.json)),
        },
        Err(e) => ValidationResult {
            valid: false,
            error: Some(ValidationError { code: e.code.as_str().to_string(), message: e.message }),
            summary: None,
        },
    }
}

/// Exporta a cena para bytes `.glb` (ou `.vrm`). Determinístico.
pub fn export_model(scene: &ExportScene, vrm: Option<&VrmExportData>) -> Vec<u8> {
    export_glb(scene, vrm)
}

// ---------------------------------------------------------------------------
// Testes (rodam com `cargo test` — CI, issue #59)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use super::*;

    fn sample_scene() -> ExportScene {
        ExportScene {
            name: Some("Fase2 Scene".to_string()),
            root_nodes: vec![0],
            nodes: vec![
                ExportNode {
                    name: Some("Root".into()),
                    translation: Some([0.5, 1.0, 0.0]),
                    children: vec![1],
                    mesh_index: Some(0),
                    skin_index: Some(0),
                    ..Default::default()
                },
                ExportNode { name: Some("Hip".into()), translation: Some([0.0, 0.9, 0.0]), children: vec![2], ..Default::default() },
                ExportNode { name: Some("Spine".into()), translation: Some([0.0, 0.25, 0.0]), children: vec![3], ..Default::default() },
                ExportNode { name: Some("Head".into()), ..Default::default() },
            ],
            meshes: vec![ExportMesh {
                name: Some("Body".into()),
                primitives: vec![ExportPrimitive {
                    vertices: vec![
                        ExportVertex { position: [0.0, 0.0, 0.0], normal: [0.0, 1.0, 0.0], uv: [0.0, 0.0], color: [1.0; 4], joints: [0, 1, 0, 0], weights: [0.5, 0.5, 0.0, 0.0] },
                        ExportVertex { position: [1.0, 0.0, 0.0], normal: [0.0, 1.0, 0.0], uv: [1.0, 0.0], color: [1.0; 4], joints: [1, 0, 0, 0], weights: [1.0, 0.0, 0.0, 0.0] },
                        ExportVertex { position: [0.0, 0.0, 1.0], normal: [0.0, 0.0, 1.0], uv: [0.0, 1.0], color: [1.0; 4], joints: [1, 2, 0, 0], weights: [0.75, 0.25, 0.0, 0.0] },
                        ExportVertex { position: [1.0, 0.0, 1.0], normal: [0.0, 0.0, 1.0], uv: [1.0, 1.0], color: [1.0; 4], joints: [2, 1, 0, 0], weights: [0.25, 0.75, 0.0, 0.0] },
                    ],
                    indices: vec![0, 1, 2, 2, 1, 3],
                    material_index: 0,
                    morph_targets: vec![vec![
                        ExportMorphDelta { position: [0.01, 0.02, 0.03], normal: [0.0; 3] },
                        ExportMorphDelta { position: [0.0; 3], normal: [0.0, 0.5, 0.0] },
                        ExportMorphDelta { position: [0.04, 0.0, 0.0], normal: [0.1, 0.0, 0.0] },
                        ExportMorphDelta { position: [0.0, -0.01, 0.0], normal: [0.0, 0.0, 0.1] },
                    ]],
                }],
            }],
            skins: vec![ExportSkin {
                name: Some("Rig".into()),
                joints: vec![1, 2, 3, 0],
                inverse_bind_matrices: vec![[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]; 4],
                ..Default::default()
            }],
            materials: vec![ExportMaterial {
                name: Some("Skin".into()),
                base_color_factor: [0.98, 0.92, 0.85, 1.0],
                mtoon: Some(ExportMToon {
                    shadow_color: Some([0.82, 0.73, 0.78, 1.0]),
                    shade_toony: Some(0.5),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            animations: vec![],
        }
    }

    fn vrm_data() -> VrmExportData {
        VrmExportData {
            meta: Vrm1Meta {
                title: "ANIGO Test Avatar".into(),
                author: "Arena Agent".into(),
                version: "1.0.0".into(),
                year: 2026,
                license_name: Some("CC-BY-4.0".into()),
                contact_information: "https://anigo.studio".into(),
                metadata: vec![("software".into(), "anigo-studio".into())],
                reference: String::new(),
            },
            humanoid_bones: vec![
                ("hips".into(), Some(2)),
                ("spine".into(), Some(1)),
                ("head".into(), Some(3)),
                ("leftUpperArm".into(), None),
            ],
            expression_presets: vrm1::VRM1_EXPRESSION_PRESETS.iter().enumerate().map(|(i, name)| (name.to_string(), i as u64)).collect(),
            expression_custom: vec![("wink".into(), 16)],
            spring_bone: None,
        }
    }

    fn fnv1a64(bytes: &[u8]) -> String {
        anigo_core::export::buffer_checksum(bytes)
    }

    #[test]
    fn roundtrip_glb_preserva_geometria_skin_morph() {
        let bytes = export_model(&sample_scene(), None);
        let result = parse_model(&bytes).expect("GLB exportado deve parsear");
        let json = &result.model.json;
        assert_eq!(json.pointer("/asset/version").and_then(Value::as_str), Some("2.0"));
        assert_eq!(json.get("nodes").and_then(Value::as_array).map(|n| n.len()), Some(4));
        let prim = json.pointer("/meshes/0/primitives/0").expect("primitiva");
        let position_accessor = prim.pointer("/attributes/POSITION").and_then(Value::as_u64).expect("POSITION").to_string();
        let position = read_accessor_f32(&result.model, position_accessor.parse::<usize>().expect("índice")).expect("posição");
        assert_eq!(position.len(), 12);
        assert!((position[3] - 1.0).abs() < 1e-6, "v1.position.x");
        assert!((position[11] - 1.0).abs() < 1e-6, "v3.position.z");
        let indices = read_accessor_indices(&result.model, prim.get("indices").and_then(Value::as_u64).unwrap() as usize).expect("índices");
        assert_eq!(indices, vec![0, 1, 2, 2, 1, 3]);
        let skin = json.pointer("/skins/0").expect("skin");
        assert_eq!(skin.get("joints").and_then(Value::as_array).map(|j| j.len()), Some(4));
        let targets = prim.get("targets").and_then(Value::as_array);
        assert_eq!(targets.map(|t| t.len()), Some(1));
    }

    #[test]
    fn export_e_deterministico() {
        let a = export_model(&sample_scene(), None);
        let b = export_model(&sample_scene(), None);
        assert_eq!(fnv1a64(&a), fnv1a64(&b));
        let a_vrm = export_model(&sample_scene(), Some(&vrm_data()));
        let b_vrm = export_model(&sample_scene(), Some(&vrm_data()));
        assert_eq!(fnv1a64(&a_vrm), fnv1a64(&b_vrm));
        assert_ne!(fnv1a64(&a), fnv1a64(&a_vrm));
    }

    #[test]
    fn roundtrip_vrm_preserva_extensoes() {
        let bytes = export_model(&sample_scene(), Some(&vrm_data()));
        let result = parse_model(&bytes).expect("VRM exportado deve parsear");
        let vrm = result.vrm.expect("deve reconhecer VRMC_vrm");
        assert_eq!(vrm.meta.title, "ANIGO Test Avatar");
        assert_eq!(vrm.human_bone_node("hips"), Some(Some(2)));
        assert_eq!(vrm.human_bone_node("leftUpperArm"), Some(None));
        for (i, name) in vrm1::VRM1_EXPRESSION_PRESETS.iter().enumerate() {
            let binding = vrm.expression.preset.get(*name).expect("preset");
            assert_eq!(binding.get("blendShape").and_then(Value::as_u64), Some(i as u64));
        }
        assert_eq!(vrm.expression.custom.get("wink").and_then(|b| b.get("blendShape")).and_then(Value::as_u64), Some(16));
        let mtoon = vrm.materials[0].as_ref().expect("mtoon no material 0");
        assert!((mtoon.shade_toony - 0.5).abs() < 1e-6);
        let summary = summarize_import(&result.model.json);
        assert!(summary.is_vrm);
        assert_eq!(summary.title, "ANIGO Test Avatar");
        assert_eq!(summary.vertex_count, 4);
        assert_eq!(summary.triangle_count, 2);
    }

    #[test]
    fn parse_vrm_falha_com_codigo_estavel() {
        let bytes = export_model(&sample_scene(), None);
        let error = parse_vrm(&bytes).expect_err("não é VRM");
        assert_eq!(error.code, Vrm1ErrorCode::NotVrm);
    }

    #[test]
    fn validate_model_corrompido_gera_codigo_estavel() {
        let mut corrupted = export_model(&sample_scene(), None);
        corrupted[0] = 0;
        let result = validate_model(&corrupted);
        assert!(!result.valid);
        assert!(result.error.is_some());
    }

    /// GLB mínimo à mão: POSITION f32 (36 B @0), índices u16 (6 B @36),
    /// índices u32 (12 B @44), JOINTS_0 u8 VEC4 (12 B @56).
    fn glb_minimo(accessors: serde_json::Value) -> Vec<u8> {
        let mut bin: Vec<u8> = Vec::new();
        for v in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            bin.extend_from_slice(&v.to_le_bytes());
        }
        for v in [0u16, 1, 2] {
            bin.extend_from_slice(&v.to_le_bytes());
        }
        bin.extend_from_slice(&[0, 0]); // alinha em 4
        for v in [2u32, 1, 0] {
            bin.extend_from_slice(&v.to_le_bytes());
        }
        bin.extend_from_slice(&[1, 2, 0, 0, 3, 0, 0, 0, 255, 6, 7, 8]);
        assert_eq!(bin.len(), 68);
        let json = serde_json::json!({
            "asset": { "version": "2.0" },
            "scene": 0,
            "scenes": [{ "nodes": [0] }],
            "nodes": [{ "mesh": 0 }],
            "buffers": [{ "byteLength": bin.len() }],
            "bufferViews": [
                { "buffer": 0, "byteOffset": 0, "byteLength": 36 },
                { "buffer": 0, "byteOffset": 36, "byteLength": 6 },
                { "buffer": 0, "byteOffset": 44, "byteLength": 12 },
                { "buffer": 0, "byteOffset": 56, "byteLength": 12 }
            ],
            "accessors": accessors,
            "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0, "JOINTS_0": 3 }, "indices": 1, "mode": 4 }] }]
        });
        crate::gltf::build_glb(&json, &bin)
    }

    #[test]
    fn component_types_seguem_a_spec_5123_u16_5125_u32_5121_u8() {
        let bytes = glb_minimo(serde_json::json!([
            { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3" },
            { "bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR" },
            { "bufferView": 2, "componentType": 5125, "count": 3, "type": "SCALAR" },
            { "bufferView": 3, "componentType": 5121, "count": 3, "type": "VEC4" }
        ]));
        let result = parse_model(&bytes).expect("GLB mínimo com códigos corretos deve parsear");
        assert_eq!(read_accessor_indices(&result.model, 1).expect("u16 (5123)"), vec![0, 1, 2]);
        assert_eq!(read_accessor_indices(&result.model, 2).expect("u32 (5125)"), vec![2, 1, 0]);
        assert_eq!(
            read_accessor_f32(&result.model, 3).expect("joints u8 (5121)"),
            vec![1.0, 2.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 255.0, 6.0, 7.0, 8.0]
        );
    }

    #[test]
    fn component_types_invalidos_em_accessors_sao_rejeitados() {
        for code in [5124u64, 5127, 5130] {
            let bytes = glb_minimo(serde_json::json!([
                { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3" },
                { "bufferView": 1, "componentType": code, "count": 3, "type": "SCALAR" },
                { "bufferView": 2, "componentType": 5125, "count": 3, "type": "SCALAR" },
                { "bufferView": 3, "componentType": 5121, "count": 3, "type": "VEC4" }
            ]));
            let error = parse_model(&bytes).expect_err("componentType fora da spec");
            assert_eq!(error.code, GltfErrorCode::BadAccessorComponent, "código {code}");
        }
    }

    #[test]
    fn byte_e_short_assinados_sao_lidos_com_sinal() {
        let mut bin: Vec<u8> = vec![(-128i8) as u8, 127, (-64i8) as u8, 0];
        for v in [-32768i16, 32767, -1000, 0] {
            bin.extend_from_slice(&v.to_le_bytes());
        }
        let json = serde_json::json!({
            "asset": { "version": "2.0" },
            "buffers": [{ "byteLength": bin.len() }],
            "bufferViews": [
                { "buffer": 0, "byteOffset": 0, "byteLength": 3 },
                { "buffer": 0, "byteOffset": 4, "byteLength": 6 }
            ],
            "accessors": [
                { "bufferView": 0, "componentType": 5120, "count": 3, "type": "SCALAR" },
                { "bufferView": 0, "componentType": 5120, "normalized": true, "count": 3, "type": "SCALAR" },
                { "bufferView": 1, "componentType": 5122, "count": 3, "type": "SCALAR" },
                { "bufferView": 1, "componentType": 5122, "normalized": true, "count": 3, "type": "SCALAR" }
            ]
        });
        let result = parse_model(&crate::gltf::build_glb(&json, &bin)).expect("parse");
        assert_eq!(read_accessor_f32(&result.model, 0).unwrap(), vec![-128.0, 127.0, -64.0]);
        let n8 = read_accessor_f32(&result.model, 1).unwrap();
        assert!((n8[0] + 1.0).abs() < 1e-6 && (n8[1] - 1.0).abs() < 1e-6 && (n8[2] + 64.0 / 127.0).abs() < 1e-6);
        assert_eq!(read_accessor_f32(&result.model, 2).unwrap(), vec![-32768.0, 32767.0, -1000.0]);
        let n16 = read_accessor_f32(&result.model, 3).unwrap();
        assert!((n16[0] + 1.0).abs() < 1e-6 && (n16[1] - 1.0).abs() < 1e-6 && (n16[2] + 1000.0 / 32767.0).abs() < 1e-6);
    }

    #[test]
    fn export_grava_component_types_da_spec() {
        let bytes = export_model(&sample_scene(), None);
        let result = parse_model(&bytes).expect("parse");
        let json = &result.model.json;
        let prim = json.pointer("/meshes/0/primitives/0").expect("primitiva");
        let component = |accessor: u64| json.pointer(&format!("/accessors/{accessor}/componentType")).and_then(Value::as_u64);
        assert_eq!(component(prim.pointer("/attributes/JOINTS_0").and_then(Value::as_u64).unwrap()), Some(5123), "JOINTS_0 = UNSIGNED_SHORT");
        assert_eq!(component(prim.get("indices").and_then(Value::as_u64).unwrap()), Some(5123), "índices pequenos = UNSIGNED_SHORT");

        let mut big = sample_scene();
        big.meshes[0].primitives[0].indices = vec![0, 1, 70000];
        let result = parse_model(&export_model(&big, None)).expect("parse");
        let json = &result.model.json;
        let prim = json.pointer("/meshes/0/primitives/0").expect("primitiva");
        let idx = prim.get("indices").and_then(Value::as_u64).unwrap();
        assert_eq!(json.pointer(&format!("/accessors/{idx}/componentType")).and_then(Value::as_u64), Some(5125));
    }

    #[test]
    fn mip_level_count_segue_a_spec() {
        assert_eq!(mip_level_count(1, 1), 1);
        assert_eq!(mip_level_count(4096, 1024), 13);
        assert_eq!(mip_level_count(1000, 2000), 11);
        assert_eq!(mip_level_count(64, 64), 7);
    }

    #[test]
    fn mtoon_mapping_segue_convencao_vroid() {
        let material = Value::Object({
            let mut pbr = serde_json::Map::new();
            pbr.insert("baseColorFactor".into(), serde_json::json!([0.95, 0.85, 0.8, 1.0]));
            let mut m = serde_json::Map::new();
            m.insert("pbrMetallicRoughness".into(), serde_json::Value::Object(pbr));
            m
        });
        let mtoon = Vrm1MToon {
            shadow_color: [0.8, 0.7, 0.9, 1.0],
            shade_shift: 0.2,
            shade_toony: 0.3,
            ..Default::default()
        };
        let mapping = map_mtoon_to_anime_material(&material, Some(&mtoon));
        assert!((mapping.shadow_threshold - 0.6).abs() < 1e-6);
        assert_eq!(mapping.toon_steps, 2.0);
        assert!((mapping.shade_color[0] - 0.95 * 0.8).abs() < 1e-6);
    }
}
