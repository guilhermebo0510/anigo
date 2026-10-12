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

pub mod anim_io;
pub mod exporter;
pub mod gltf;
pub mod spring_bridge;
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
    is_vrm_document, map_mtoon_to_anime_material, parse_spring_bone, parse_vrm1, MToonMaterialMapping,
    ParsedVrm1, Vrm1Collider, Vrm1ColliderGroup, Vrm1ColliderShape, Vrm1Error, Vrm1ErrorCode, Vrm1Meta,
    Vrm1MToon, Vrm1Spring, Vrm1SpringBone, Vrm1SpringJoint, SPRING_BONE_SPEC_VERSION,
};
pub use exporter::spring_bone_to_json;

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
                    sparse_morphs: false,
                }],
                target_names: Vec::new(),
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

    // ---- VRMC_springBone (schema oficial 1.0) ------------------------------------------------

    fn sample_spring_bone() -> Vrm1SpringBone {
        Vrm1SpringBone {
            spec_version: SPRING_BONE_SPEC_VERSION.to_string(),
            colliders: vec![
                Vrm1Collider { node: 0, shape: Vrm1ColliderShape::Sphere { offset: [0.0, 0.1, 0.0], radius: 0.08, inside: false } },
                Vrm1Collider {
                    node: 1,
                    shape: Vrm1ColliderShape::Capsule { offset: [0.0, 0.0, 0.0], radius: 0.06, tail: [0.0, 0.3, 0.0], inside: false },
                },
                Vrm1Collider { node: 0, shape: Vrm1ColliderShape::Sphere { offset: [0.0; 3], radius: 0.5, inside: true } },
                Vrm1Collider { node: 0, shape: Vrm1ColliderShape::Plane { offset: [0.0, 0.0, 0.0], normal: [0.0, 1.0, 0.0] } },
            ],
            collider_groups: vec![
                Vrm1ColliderGroup { name: Some("body".into()), colliders: vec![0, 1] },
                Vrm1ColliderGroup { name: None, colliders: vec![2, 3] },
            ],
            springs: vec![Vrm1Spring {
                name: Some("hair_front".into()),
                joints: vec![
                    Vrm1SpringJoint { node: 1, hit_radius: 0.02, stiffness: 0.8, gravity_power: 0.5, gravity_dir: [0.0, -1.0, 0.0], drag_force: 0.4 },
                    Vrm1SpringJoint { node: 2, hit_radius: 0.02, stiffness: 0.6, gravity_power: 0.5, gravity_dir: [0.0, -1.0, 0.0], drag_force: 0.45 },
                    Vrm1SpringJoint { node: 3, ..Default::default() },
                ],
                collider_groups: vec![0, 1],
                center: Some(0),
            }],
        }
    }

    fn vrm_with_spring() -> VrmExportData {
        VrmExportData { spring_bone: Some(sample_spring_bone()), ..vrm_data() }
    }

    #[test]
    fn spring_bone_roundtrip_schema_oficial() {
        let bytes = export_model(&sample_scene(), Some(&vrm_with_spring()));
        let result = parse_model(&bytes).expect("VRM com springBone deve parsear");
        let vrm = result.vrm.expect("VRM");
        let parsed = vrm.spring_bone.expect("springBone");
        assert_eq!(parsed, sample_spring_bone(), "ida e volta preserva tudo (inclusive extended_collider)");
        assert!(
            !vrm.warnings.iter().any(|w| w.contains("legado")),
            "formato novo não deve gerar aviso de legado: {:?}",
            vrm.warnings
        );
    }

    /// Validador mínimo do JSON-schema oficial `VRMC_springBone-1.0` (chaves permitidas, tipos, obrigatórios).
    fn assert_spring_bone_schema(spring: &Value) {
        let allowed = |obj: &Value, keys: &[&str], where_: &str| {
            for key in obj.as_object().unwrap_or_else(|| panic!("{where_}: objeto")).keys() {
                assert!(keys.contains(&key.as_str()), "{where_}: chave fora do schema: {key}");
            }
        };
        allowed(spring, &["specVersion", "colliders", "colliderGroups", "springs", "extensions", "extras"], "VRMC_springBone");
        assert_eq!(spring["specVersion"], "1.0", "specVersion obrigatório");
        for (i, c) in spring["colliders"].as_array().into_iter().flatten().enumerate() {
            allowed(c, &["node", "shape", "extensions", "extras"], "collider");
            assert!(c["node"].is_u64(), "colliders[{i}].node");
            let shape = &c["shape"];
            allowed(shape, &["sphere", "capsule"], "shape");
            assert_eq!(shape.as_object().unwrap().len(), 1, "shape exclusivo sphere|capsule");
            if let Some(s) = shape.get("sphere") {
                allowed(s, &["offset", "radius"], "sphere");
            }
            if let Some(cap) = shape.get("capsule") {
                allowed(cap, &["offset", "radius", "tail"], "capsule");
            }
        }
        for g in spring["colliderGroups"].as_array().into_iter().flatten() {
            allowed(g, &["name", "colliders", "extras"], "colliderGroup");
            assert!(g["colliders"].as_array().unwrap().iter().all(Value::is_u64));
        }
        for s in spring["springs"].as_array().into_iter().flatten() {
            allowed(s, &["name", "joints", "colliderGroups", "center", "extras"], "spring");
            assert!(s["joints"].as_array().unwrap().len() >= 2, "joints minItems 2");
            for j in s["joints"].as_array().unwrap() {
                allowed(j, &["node", "hitRadius", "stiffness", "gravityPower", "gravityDir", "dragForce", "extras"], "joint");
                assert!(j["node"].is_u64());
                assert!(j["dragForce"].as_f64().unwrap() >= 0.0 && j["dragForce"].as_f64().unwrap() <= 1.0);
                assert!(j["stiffness"].as_f64().unwrap() >= 0.0);
                assert_eq!(j["gravityDir"].as_array().unwrap().len(), 3);
            }
        }
    }

    #[test]
    fn spring_bone_exportado_segue_a_spec_oficial() {
        let (json, _) = export_gltf(&sample_scene(), Some(&vrm_with_spring()));
        let spring = json.pointer("/extensions/VRMC_springBone").expect("VRMC_springBone");
        assert!(spring.get("secondaryRig").is_none(), "formato legado não pode ser emitido");
        assert_spring_bone_schema(spring);
        // Extensão dentro de `extensionsUsed`, nunca em `extensionsRequired`.
        let used: Vec<&str> = json["extensionsUsed"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert!(used.contains(&"VRMC_springBone"));
        assert!(used.contains(&"VRMC_springBone_extended_collider"));
        let required: Vec<&str> = json["extensionsRequired"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert!(!required.iter().any(|e| e.contains("springBone")), "springBone fora de extensionsRequired: {required:?}");
        // Colliders extendidos levam fallback na spec base.
        let inside = &spring["colliders"][2];
        assert_eq!(inside["shape"]["sphere"]["radius"], 0.0);
        assert_eq!(inside["extensions"]["VRMC_springBone_extended_collider"]["shape"]["sphere"]["inside"], true);
        let plane = &spring["colliders"][3];
        assert_eq!(plane["shape"]["sphere"]["radius"], 1000.0);
        assert_eq!(plane["extensions"]["VRMC_springBone_extended_collider"]["shape"]["plane"]["normal"][1], 1.0);
        // Valores curtos (sem ruído de f32 -> f64).
        assert_eq!(spring["springs"][0]["joints"][0]["stiffness"], 0.8);
        assert_eq!(spring["springs"][0]["joints"][0]["dragForce"], 0.4);
        assert_eq!(spring["springs"][0]["center"], 0);
    }

    #[test]
    fn spring_bone_sem_extended_nao_declara_extended_collider() {
        let mut data = vrm_with_spring();
        let sb = data.spring_bone.as_mut().unwrap();
        sb.colliders.truncate(2);
        sb.collider_groups = vec![Vrm1ColliderGroup { name: None, colliders: vec![0, 1] }];
        sb.springs[0].collider_groups = vec![0];
        let (json, _) = export_gltf(&sample_scene(), Some(&data));
        let used: Vec<&str> = json["extensionsUsed"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert!(used.contains(&"VRMC_springBone"));
        assert!(!used.contains(&"VRMC_springBone_extended_collider"));
        assert_spring_bone_schema(json.pointer("/extensions/VRMC_springBone").unwrap());
    }

    #[test]
    fn spring_bone_export_e_deterministico() {
        let a = export_model(&sample_scene(), Some(&vrm_with_spring()));
        let b = export_model(&sample_scene(), Some(&vrm_with_spring()));
        assert_eq!(fnv1a64(&a), fnv1a64(&b));
    }

    fn legacy_spring_json() -> Value {
        serde_json::json!({
            "secondaryRig": {
                "groups": [{
                    "name": "HairFront",
                    "center": { "node": 0, "distance": 0.1 },
                    "joints": [
                        { "node": 1, "distance": 0.1, "springStiffness": 0.8, "springDamping": 0.3, "hitRadius": 0.02 },
                        { "node": 2, "distance": 0.12, "springStiffness": 0.6, "springDamping": 0.4 }
                    ],
                    "colliderGroups": [7]
                }],
                "colliders": [
                    { "spheres": [{ "group": 7, "node": 0, "radius": 0.08, "offset": [0.0, 0.01, 0.0] }] },
                    { "capsules": [{ "group": 7, "node": 1, "from": [0.0, 0.5, 0.0], "to": [0.0, 0.8, 0.0], "radius": 0.06 }] },
                    { "spheres": [{ "group": 3, "node": 2, "radius": 0.1 }] }
                ]
            }
        })
    }

    #[test]
    fn spring_bone_legado_secondary_rig_ainda_le() {
        let mut warnings = Vec::new();
        let nodes: Vec<Value> = vec![
            serde_json::json!({ "children": [1] }),
            serde_json::json!({ "children": [2] }),
            serde_json::json!({}),
        ];
        let sb = parse_spring_bone(Some(&legacy_spring_json()), &nodes, &mut warnings).expect("legado lê");
        assert!(warnings.iter().any(|w| w.contains("legado")));
        assert_eq!(sb.springs.len(), 1);
        let spring = &sb.springs[0];
        assert_eq!(spring.name.as_deref(), Some("HairFront"));
        assert_eq!(spring.center, Some(0));
        assert_eq!(spring.joints.len(), 2);
        assert!((spring.joints[0].stiffness - 0.8).abs() < 1e-6, "springStiffness -> stiffness");
        assert!((spring.joints[0].drag_force - 0.3).abs() < 1e-6, "springDamping -> dragForce");
        assert!((spring.joints[0].hit_radius - 0.02).abs() < 1e-6);
        assert_eq!(sb.colliders.len(), 3);
        assert_eq!(
            sb.colliders[1].shape,
            Vrm1ColliderShape::Capsule { offset: [0.0, 0.5, 0.0], radius: 0.06, tail: [0.0, 0.8, 0.0], inside: false }
        );
        // ids legados 3 e 7 viram índices densos 0 e 1 (ordem crescente de id)
        assert_eq!(sb.collider_groups.len(), 2);
        assert_eq!(sb.collider_groups[0].colliders, vec![2]);
        assert_eq!(sb.collider_groups[1].colliders, vec![0, 1]);
        assert_eq!(spring.collider_groups, vec![1]);
    }

    #[test]
    fn spring_bone_legado_em_arquivo_vrm_converte_e_reexporta_no_formato_novo() {
        let (mut json, bin) = export_gltf(&sample_scene(), Some(&vrm_data()));
        json["extensions"]["VRMC_springBone"] = legacy_spring_json();
        let glb = build_glb(&json, &bin);
        let result = parse_model(&glb).expect("arquivo antigo ainda abre");
        let vrm = result.vrm.expect("VRM");
        let sb = vrm.spring_bone.clone().expect("springBone convertido");
        assert!(vrm.warnings.iter().any(|w| w.contains("legado")));
        // Reexportar grava o schema oficial.
        let data = VrmExportData { spring_bone: Some(sb.clone()), ..vrm_data() };
        let (new_json, _) = export_gltf(&sample_scene(), Some(&data));
        let spring = new_json.pointer("/extensions/VRMC_springBone").unwrap();
        assert!(spring.get("secondaryRig").is_none());
        assert_spring_bone_schema(spring);
        let again = parse_model(&export_model(&sample_scene(), Some(&data))).unwrap().vrm.unwrap().spring_bone.unwrap();
        assert_eq!(again, sb);
    }

    #[test]
    fn spring_bone_malformado_gera_erro_estavel_sem_panic() {
        let nodes: Vec<Value> = vec![serde_json::json!({ "children": [1] }), serde_json::json!({})];
        let cases = [
            serde_json::json!(42),
            serde_json::json!({ "specVersion": "1.0", "springs": [{ "joints": [{ "stiffness": 1.0 }] }] }),
            serde_json::json!({ "specVersion": "1.0", "springs": [{ "joints": [{ "node": 0, "stiffness": -1.0 }, { "node": 1 }] }] }),
            serde_json::json!({ "specVersion": "1.0", "springs": [{ "joints": [{ "node": 0, "dragForce": 2.0 }, { "node": 1 }] }] }),
            serde_json::json!({ "specVersion": "1.0", "springs": [{ "joints": [{ "node": 0, "gravityDir": [0, 1] }, { "node": 1 }] }] }),
            serde_json::json!({ "specVersion": "1.0", "springs": [{ "joints": [{ "node": 0, "hitRadius": "x" }, { "node": 1 }] }] }),
            serde_json::json!({ "specVersion": "1.0", "springs": [{ "joints": "nope" }] }),
            serde_json::json!({ "specVersion": "1.0", "colliders": [{ "node": 0 }] }),
            serde_json::json!({ "specVersion": "1.0", "colliders": [{ "node": 0, "shape": {} }] }),
            serde_json::json!({ "specVersion": "1.0", "colliders": [{ "node": 0, "shape": { "sphere": { "radius": -1 } } }] }),
            serde_json::json!({ "specVersion": "1.0", "colliders": [{ "node": -3, "shape": { "sphere": {} } }] }),
            serde_json::json!({ "specVersion": "1.0", "colliderGroups": [{ "colliders": ["a"] }] }),
            serde_json::json!({ "secondaryRig": 5 }),
        ];
        for (i, case) in cases.iter().enumerate() {
            let mut warnings = Vec::new();
            let err = parse_spring_bone(Some(case), &nodes, &mut warnings).expect_err(&format!("caso {i} deve falhar: {case}"));
            assert_eq!(err.code, Vrm1ErrorCode::BadSpringBone, "caso {i}");
        }
        let mut warnings = Vec::new();
        assert!(parse_spring_bone(None, &nodes, &mut warnings).is_err());
    }

    #[test]
    fn spring_bone_referencias_invalidas_viram_aviso_e_sao_descartadas() {
        let nodes: Vec<Value> = vec![serde_json::json!({ "children": [1] }), serde_json::json!({}), serde_json::json!({})];
        let raw = serde_json::json!({
            "colliders": [{ "node": 99, "shape": { "sphere": { "radius": 0.1 } } }],
            "colliderGroups": [{ "colliders": [0, 5] }],
            "springs": [
                { "joints": [{ "node": 0 }, { "node": 2 }], "colliderGroups": [0, 9] },
                { "joints": [{ "node": 1 }] }
            ]
        });
        let mut warnings = Vec::new();
        let sb = parse_spring_bone(Some(&raw), &nodes, &mut warnings).expect("tolerante");
        assert_eq!(sb.collider_groups[0].colliders, vec![0]);
        assert_eq!(sb.springs[0].collider_groups, vec![0]);
        let all = warnings.join(" | ");
        assert!(all.contains("specVersion ausente"), "{all}");
        assert!(all.contains("fora da faixa"), "{all}");
        assert!(all.contains("não é descendente"), "{all}");
        assert!(all.contains("collider inexistente"), "{all}");
        assert!(all.contains("grupo inexistente"), "{all}");
        assert!(all.contains(">= 2"), "{all}");
    }

    #[test]
    fn spring_bone_defaults_da_spec() {
        let nodes: Vec<Value> = vec![serde_json::json!({ "children": [1] }), serde_json::json!({})];
        let raw = serde_json::json!({ "specVersion": "1.0", "springs": [{ "joints": [{ "node": 0 }, { "node": 1 }] }] });
        let mut warnings = Vec::new();
        let sb = parse_spring_bone(Some(&raw), &nodes, &mut warnings).unwrap();
        let j = sb.springs[0].joints[0];
        assert_eq!((j.hit_radius, j.stiffness, j.gravity_power, j.drag_force), (0.0, 1.0, 0.0, 0.5));
        assert_eq!(j.gravity_dir, [0.0, -1.0, 0.0]);
        assert!(warnings.is_empty(), "{warnings:?}");
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
