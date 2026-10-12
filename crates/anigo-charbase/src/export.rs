//! Exportação GLB (skin de 65 ossos, morph targets nomeados, submalhas/materiais) e VRM 1.0.
//!
//! Reusa o exportador determinístico de `anigo-vrm` (`export_gltf`) para nós, skin, malhas e
//! morph targets (com accessors esparsos e `extras.targetNames`). A camada `VRMC_vrm` é escrita
//! aqui no schema oficial 1.0 (humanoid, expressions com `morphTargetBinds`, lookAt, meta): o
//! escritor VRM embutido do `anigo-vrm` usa outro formato de expressões (16 presets próprios) e
//! não foi alterado.

use crate::character::BaseCharacter;
use crate::morphs::MorphSet;
use crate::parts::{build_parts, part_material};
use crate::qmesh::QuadMesh;
use crate::skeleton::humanoid_names;
use crate::topology::{quad_mesh_of, reference};
use anigo_vrm::exporter::{export_gltf, ExportMaterial, ExportMesh, ExportMorphDelta, ExportNode, ExportPrimitive, ExportScene, ExportSkin, ExportVertex};
use anigo_vrm::gltf::build_glb;
use glam::{Mat4, Vec3};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct ExportOptions {
    /// Acrescenta a extensão `VRMC_vrm` 1.0 (humanoid, expressões, lookAt) e MToon mínimo.
    pub vrm: bool,
    pub title: String,
    pub author: String,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            vrm: false,
            title: "ANIGO Base".into(),
            author: "ANIGO".into(),
        }
    }
}

pub const MATERIALS: [(&str, [f32; 4]); 9] = [
    ("skin", [0.965, 0.80, 0.72, 1.0]),
    ("eye_white", [0.97, 0.97, 0.98, 1.0]),
    ("eye_iris", [0.20, 0.52, 0.80, 1.0]),
    ("eye_highlight", [1.0, 1.0, 1.0, 1.0]),
    ("lash", [0.06, 0.05, 0.07, 1.0]),
    ("brow", [0.24, 0.15, 0.10, 1.0]),
    ("mouth_inside", [0.40, 0.10, 0.12, 1.0]),
    ("teeth", [0.98, 0.97, 0.93, 1.0]),
    ("tongue", [0.85, 0.35, 0.40, 1.0]),
];

fn material_index(name: &str) -> i64 {
    MATERIALS.iter().position(|(n, _)| *n == name).unwrap_or(0) as i64
}

fn tri_normals(pos: &[Vec3], tris: &[[u32; 3]]) -> Vec<Vec3> {
    let mut n = vec![Vec3::ZERO; pos.len()];
    for t in tris {
        let (a, b, c) = (pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
        let fnorm = (b - a).cross(c - a);
        for &i in t {
            n[i as usize] += fnorm;
        }
    }
    n.iter().map(|v| v.normalize_or_zero()).collect()
}

/// Resultado detalhado da montagem da cena (útil para testes).
pub struct BuiltScene {
    pub scene: ExportScene,
    pub target_names: Vec<String>,
    /// Número de vértices canônicos do Skin (os primeiros `skin_canonical` vértices do primitivo 0).
    pub skin_canonical: usize,
    /// Para cada vértice extra (índice >= skin_canonical) o vértice canônico de origem.
    pub extra_canonical: Vec<u32>,
    pub mesh_node: usize,
    pub root_node: usize,
}

/// Monta a cena de exportação: 65 ossos, malha "Body" com primitivos (pele + peças auxiliares).
pub fn build_scene(ch: &BaseCharacter, morphs: &MorphSet) -> BuiltScene {
    let topo = reference();
    let n_bones = ch.skeleton.len();
    let t_count = morphs.targets.len();

    // --- nós: ossos, raiz e malha ---
    let mut nodes: Vec<ExportNode> = ch
        .skeleton
        .bones
        .iter()
        .map(|b| ExportNode {
            name: Some(b.name.clone()),
            translation: Some(b.local),
            ..Default::default()
        })
        .collect();
    for (i, b) in ch.skeleton.bones.iter().enumerate() {
        if let Some(p) = b.parent {
            nodes[p].children.push(i as u64);
        }
    }
    let mesh_node = n_bones + 1;
    let root_node = n_bones;
    nodes.push(ExportNode {
        name: Some("ANIGO_Character".into()),
        children: vec![0, mesh_node as u64],
        ..Default::default()
    });
    nodes.push(ExportNode {
        name: Some("Body".into()),
        mesh_index: Some(0),
        skin_index: Some(0),
        morph_weights: Some(vec![0.0; t_count]),
        ..Default::default()
    });

    let skin = ExportSkin {
        name: Some("ANIGO_Armature".into()),
        joints: (0..n_bones as u64).collect(),
        inverse_bind_matrices: ch.skeleton.inverse_bind().iter().map(|m: &Mat4| m.to_cols_array()).collect(),
        skeleton: Some(0),
    };

    // --- pele ---
    let base_normals = ch.normals.clone();
    // normais de cada alvo (delta de normal esparso)
    let mut target_pos: Vec<Vec<Vec3>> = Vec::with_capacity(t_count);
    let mut target_nrm: Vec<Vec<Vec3>> = Vec::with_capacity(t_count);
    for t in &morphs.targets {
        let mut p = ch.positions.clone();
        for (v, d) in &t.deltas {
            p[*v as usize] += Vec3::from_array(*d);
        }
        let n = if t.deltas.is_empty() { base_normals.clone() } else { quad_mesh_of(topo, &p).vertex_normals() };
        target_pos.push(p);
        target_nrm.push(n);
    }
    // vértices canônicos primeiro (primeiro UV de cada um), depois duplicatas por UV
    let nv = topo.n_verts;
    let mut first_uv: Vec<Option<[u32; 2]>> = vec![None; nv];
    let mut vert_list: Vec<(u32, [f32; 2])> = Vec::with_capacity(nv + 4096); // (canônico, uv)
    let mut index_of: BTreeMap<(u32, [u32; 2]), u32> = BTreeMap::new();
    let uv_key = |uv: [f32; 2]| -> [u32; 2] { [((uv[0] * 65535.0).round()) as u32, ((uv[1] * 65535.0).round()) as u32] };
    // 1ª passada: canônicos
    let mut canonical_uv = vec![[0.0f32; 2]; nv];
    for (fi, f) in topo.faces.iter().enumerate() {
        for k in 0..4 {
            let v = f[k] as usize;
            if first_uv[v].is_none() {
                let uv = topo.uv[fi][k];
                first_uv[v] = Some(uv_key(uv));
                canonical_uv[v] = uv;
            }
        }
    }
    for v in 0..nv {
        vert_list.push((v as u32, canonical_uv[v]));
        index_of.insert((v as u32, first_uv[v].unwrap_or([0, 0])), v as u32);
    }
    let mut indices: Vec<u32> = Vec::with_capacity(topo.faces.len() * 6);
    for (fi, f) in topo.faces.iter().enumerate() {
        let mut ids = [0u32; 4];
        for k in 0..4 {
            let v = f[k];
            let uv = topo.uv[fi][k];
            let key = (v, uv_key(uv));
            let id = *index_of.entry(key).or_insert_with(|| {
                vert_list.push((v, uv));
                (vert_list.len() - 1) as u32
            });
            ids[k] = id;
        }
        indices.extend_from_slice(&[ids[0], ids[1], ids[2], ids[0], ids[2], ids[3]]);
    }
    let outline = topo.masks.get("mask_outline_weight");
    let skin_vertices: Vec<ExportVertex> = vert_list
        .iter()
        .map(|&(v, uv)| {
            let w = topo.weights[v as usize];
            let ob = outline.map(|m| m[v as usize]).unwrap_or(1.0);
            ExportVertex {
                position: ch.positions[v as usize].to_array(),
                normal: base_normals[v as usize].to_array(),
                uv: [uv[0], 1.0 - uv[1]],
                color: [1.0, 0.5, ob, 1.0],
                joints: [w[0].0, w[1].0, w[2].0, w[3].0],
                weights: [w[0].1, w[1].1, w[2].1, w[3].1],
            }
        })
        .collect();
    let skin_morphs: Vec<Vec<ExportMorphDelta>> = (0..t_count)
        .map(|ti| {
            vert_list
                .iter()
                .map(|&(v, _)| ExportMorphDelta {
                    position: (target_pos[ti][v as usize] - ch.positions[v as usize]).to_array(),
                    normal: (target_nrm[ti][v as usize] - base_normals[v as usize]).to_array(),
                })
                .collect()
        })
        .collect();
    let extra_canonical: Vec<u32> = vert_list[nv..].iter().map(|(v, _)| *v).collect();
    let mut primitives = vec![ExportPrimitive {
        vertices: skin_vertices,
        indices,
        material_index: material_index("skin"),
        morph_targets: skin_morphs,
        sparse_morphs: true,
    }];

    // --- peças auxiliares ---
    let parts = build_parts(topo, &ch.positions, &ch.positions, 0.0);
    let bone = |name: &str| -> u16 { ch.skeleton.index(name).unwrap_or(0) as u16 };
    for (pi, part) in parts.iter().enumerate() {
        let bone_idx = match part.name.as_str() {
            "eye_white_L" | "eye_iris_L" | "eye_highlight_L" => bone("leftEye"),
            "eye_white_R" | "eye_iris_R" | "eye_highlight_R" => bone("rightEye"),
            "teeth_lower" | "tongue" => bone("jaw"),
            _ => bone("head"),
        };
        let normals = tri_normals(&part.positions, &part.tris);
        let vertices: Vec<ExportVertex> = part
            .positions
            .iter()
            .zip(&normals)
            .zip(&part.uv)
            .map(|((p, n), uv)| ExportVertex {
                position: p.to_array(),
                normal: n.to_array(),
                uv: [uv[0], 1.0 - uv[1]],
                color: [1.0; 4],
                joints: [bone_idx, 0, 0, 0],
                weights: [1.0, 0.0, 0.0, 0.0],
            })
            .collect();
        let morph_targets: Vec<Vec<ExportMorphDelta>> = morphs
            .targets
            .iter()
            .map(|t| {
                let d = t.part_deltas.get(pi);
                (0..part.positions.len())
                    .map(|i| ExportMorphDelta {
                        position: d.and_then(|d| d.get(i)).copied().unwrap_or([0.0; 3]),
                        normal: [0.0; 3],
                    })
                    .collect()
            })
            .collect();
        primitives.push(ExportPrimitive {
            vertices,
            indices: part.tris.iter().flatten().copied().collect(),
            material_index: material_index(part_material(&part.name)),
            morph_targets,
            sparse_morphs: true,
        });
    }

    let target_names = morphs.names();
    let materials: Vec<ExportMaterial> = MATERIALS
        .iter()
        .map(|(name, color)| ExportMaterial {
            name: Some((*name).to_string()),
            base_color_factor: *color,
            metallic_factor: Some(0.0),
            roughness_factor: Some(0.9),
            double_sided: Some(*name != "skin"),
            ..Default::default()
        })
        .collect();
    let scene = ExportScene {
        name: Some("ANIGO Character Base".into()),
        root_nodes: vec![root_node as u64],
        nodes,
        meshes: vec![ExportMesh {
            name: Some("Body".into()),
            primitives,
            target_names: target_names.clone(),
        }],
        skins: vec![skin],
        materials,
        animations: Vec::new(),
    };
    BuiltScene {
        scene,
        target_names,
        skin_canonical: nv,
        extra_canonical,
        mesh_node,
        root_node,
    }
}

fn vrm_extension(ch: &BaseCharacter, built: &BuiltScene, opts: &ExportOptions) -> Value {
    let mut human_bones = Map::new();
    for name in humanoid_names() {
        if let Some(i) = ch.skeleton.index(&name) {
            human_bones.insert(name, json!({ "node": i }));
        }
    }
    let mut preset = Map::new();
    let idx = |n: &str| built.target_names.iter().position(|x| x == n);
    for name in ["happy", "angry", "sad", "relaxed", "surprised", "aa", "ih", "ou", "ee", "oh", "blink", "blinkLeft", "blinkRight", "lookUp", "lookDown"] {
        if let Some(i) = idx(name) {
            preset.insert(
                name.to_string(),
                json!({
                    "morphTargetBinds": [{ "node": built.mesh_node, "index": i, "weight": 1.0 }],
                    "isBinary": false,
                    "overrideBlink": "none",
                    "overrideLookAt": "none",
                    "overrideMouth": "none",
                }),
            );
        }
    }
    let head = ch.skeleton.pos(ch.skeleton.index("head").unwrap_or(0));
    let eye = (ch.skeleton.pos(ch.skeleton.index("leftEye").unwrap_or(0)) + ch.skeleton.pos(ch.skeleton.index("rightEye").unwrap_or(0))) * 0.5;
    let off = eye - head;
    json!({
        "specVersion": "1.0",
        "meta": {
            "name": opts.title,
            "version": "0.1.0",
            "authors": [opts.author],
            "copyrightInformation": "Base procedural Gen-0 do ANIGO (geometria gerada por codigo; nenhuma malha de terceiros).",
            "licenseUrl": "https://vrm.dev/licenses/1.0/",
            "avatarPermission": "onlyAuthor",
            "allowExcessivelyViolentUsage": false,
            "allowExcessivelySexualUsage": false,
            "commercialUsage": "personalNonProfit",
            "allowPoliticalOrReligiousUsage": false,
            "allowAntisocialOrHateUsage": false,
            "creditNotation": "required",
            "allowRedistribution": false,
            "modification": "prohibited",
        },
        "humanoid": { "humanBones": human_bones },
        "lookAt": {
            "type": "bone",
            "offsetFromHeadBone": [off.x, off.y, off.z],
            "rangeMapHorizontalInner": { "inputMaxValue": 90.0, "outputScale": 10.0 },
            "rangeMapHorizontalOuter": { "inputMaxValue": 90.0, "outputScale": 10.0 },
            "rangeMapVerticalDown": { "inputMaxValue": 90.0, "outputScale": 10.0 },
            "rangeMapVerticalUp": { "inputMaxValue": 90.0, "outputScale": 10.0 },
        },
        "expressions": { "preset": preset, "custom": {} },
    })
}

/// GLB determinístico do personagem base. Com `opts.vrm`, vira um VRM 1.0 (mesmo formato binário).
pub fn export_glb(ch: &BaseCharacter, morphs: &MorphSet, opts: &ExportOptions) -> Vec<u8> {
    let built = build_scene(ch, morphs);
    let (mut json, bin) = export_gltf(&built.scene, None);
    let topo = reference();
    if let Value::Object(root) = &mut json {
        // metadados do contrato no nó raiz e mapa de vértices extras (duplicados por UV)
        if let Some(Value::Array(nodes)) = root.get_mut("nodes") {
            if let Some(Value::Object(n)) = nodes.get_mut(built.root_node) {
                n.insert(
                    "extras".into(),
                    json!({
                        "anigo_charbase": {
                            "contract_version": crate::contract::CONTRACT_VERSION,
                            "skin_connectivity_hash": format!("{:016x}", topo.connectivity_hash()),
                            "skin_canonical_vertices": built.skin_canonical,
                            "note": "Os primeiros skin_canonical vertices do primitivo 0 seguem a ordem do contrato; os extras (costuras de UV) mapeiam para o canonico via extras.anigo_canonical_of_extra.",
                        }
                    }),
                );
            }
        }
        if let Some(Value::Array(meshes)) = root.get_mut("meshes") {
            if let Some(Value::Object(m)) = meshes.get_mut(0) {
                if let Some(Value::Array(prims)) = m.get_mut("primitives") {
                    if let Some(Value::Object(p0)) = prims.get_mut(0) {
                        p0.insert("extras".into(), json!({ "anigo_canonical_of_extra": built.extra_canonical }));
                    }
                }
            }
        }
        if opts.vrm {
            let mut used: Vec<Value> = match root.remove("extensionsUsed") {
                Some(Value::Array(a)) => a,
                _ => Vec::new(),
            };
            used.push(Value::from("VRMC_vrm"));
            used.push(Value::from("VRMC_materials_mtoon"));
            root.insert("extensionsUsed".into(), Value::Array(used));
            if let Some(Value::Array(mats)) = root.get_mut("materials") {
                for (mi, m) in mats.iter_mut().enumerate() {
                    if let Value::Object(mo) = m {
                        let c = MATERIALS[mi].1;
                        let shade = [c[0] * 0.78, c[1] * 0.74, c[2] * 0.82];
                        let mut ext = Map::new();
                        ext.insert(
                            "VRMC_materials_mtoon".into(),
                            json!({
                                "specVersion": "1.0",
                                "shadeColorFactor": shade,
                                "shadingShiftFactor": -0.05,
                                "shadingToonyFactor": 0.92,
                                "outlineWidthMode": "none",
                            }),
                        );
                        mo.insert("extensions".into(), Value::Object(ext));
                    }
                }
            }
            let mut exts = match root.remove("extensions") {
                Some(Value::Object(o)) => o,
                _ => Map::new(),
            };
            exts.insert("VRMC_vrm".into(), vrm_extension(ch, &built, opts));
            root.insert("extensions".into(), Value::Object(exts));
        }
    }
    build_glb(&json, &bin)
}

/// Contagens do que será exportado (para o relatório e para testes).
pub fn summarize(ch: &BaseCharacter, morphs: &MorphSet) -> Value {
    let built = build_scene(ch, morphs);
    let prims: Vec<Value> = built.scene.meshes[0]
        .primitives
        .iter()
        .map(|p| json!({"vertices": p.vertices.len(), "triangles": p.indices.len() / 3, "material": MATERIALS[p.material_index as usize].0}))
        .collect();
    let _ = QuadMesh::new();
    json!({
        "bones": ch.skeleton.len(),
        "morph_targets": built.target_names.len(),
        "primitives": prims,
        "skin_canonical_vertices": built.skin_canonical,
        "skin_uv_seam_duplicates": built.extra_canonical.len(),
    })
}
