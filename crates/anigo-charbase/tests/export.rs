//! Testes da exportação GLB/VRM e do contrato de topologia (validação e re-derivação).

use anigo_charbase::contract::{
    contract_json_string, nearest_map, rederive, topology_contract, validate_topology, validate_topology_tris, ContractError, RederiveMethod, TopologyStatus,
};
use anigo_charbase::export::{build_scene, export_glb, ExportOptions};
use anigo_charbase::morphs::{build_morphs, MorphSet};
use anigo_charbase::skeleton::{bone_defs, humanoid_names, Skeleton};
use anigo_charbase::topology::reference;
use anigo_charbase::{generate, generate_with, BaseCharacter, Gender, Params};
use anigo_vrm::gltf::parse_glb;
use glam::Vec3;
use serde_json::Value;
use std::sync::OnceLock;

fn female() -> &'static (BaseCharacter, MorphSet) {
    static C: OnceLock<(BaseCharacter, MorphSet)> = OnceLock::new();
    C.get_or_init(|| {
        let c = generate(Gender::Female);
        let m = build_morphs(&c);
        (c, m)
    })
}

fn glb(vrm: bool) -> &'static Vec<u8> {
    static A: OnceLock<Vec<u8>> = OnceLock::new();
    static B: OnceLock<Vec<u8>> = OnceLock::new();
    let (c, m) = female();
    let cell = if vrm { &B } else { &A };
    cell.get_or_init(|| export_glb(c, m, &ExportOptions { vrm, ..Default::default() }))
}

fn accessor_f32(json: &Value, bin: &[u8], acc: usize) -> Vec<f32> {
    let a = &json["accessors"][acc];
    let comps = match a["type"].as_str().unwrap() {
        "VEC3" => 3,
        "VEC2" => 2,
        "VEC4" => 4,
        "MAT4" => 16,
        "SCALAR" => 1,
        other => panic!("tipo {other}"),
    };
    let count = a["count"].as_u64().unwrap() as usize;
    let mut out = vec![0.0f32; count * comps];
    if let Some(bv) = a.get("bufferView").and_then(|v| v.as_u64()) {
        let v = &json["bufferViews"][bv as usize];
        let off = v["byteOffset"].as_u64().unwrap_or(0) as usize + a["byteOffset"].as_u64().unwrap_or(0) as usize;
        for i in 0..count * comps {
            out[i] = f32::from_le_bytes(bin[off + i * 4..off + i * 4 + 4].try_into().unwrap());
        }
    }
    if let Some(sp) = a.get("sparse") {
        let k = sp["count"].as_u64().unwrap() as usize;
        let ib = &json["bufferViews"][sp["indices"]["bufferView"].as_u64().unwrap() as usize];
        let vb = &json["bufferViews"][sp["values"]["bufferView"].as_u64().unwrap() as usize];
        let io = ib["byteOffset"].as_u64().unwrap_or(0) as usize;
        let vo = vb["byteOffset"].as_u64().unwrap_or(0) as usize;
        let ct = sp["indices"]["componentType"].as_u64().unwrap();
        let mut last: i64 = -1;
        for j in 0..k {
            let idx = match ct {
                5123 => u16::from_le_bytes(bin[io + j * 2..io + j * 2 + 2].try_into().unwrap()) as usize,
                5125 => u32::from_le_bytes(bin[io + j * 4..io + j * 4 + 4].try_into().unwrap()) as usize,
                other => panic!("componentType de índice {other}"),
            };
            assert!(idx as i64 > last, "índices esparsos devem ser estritamente crescentes");
            last = idx as i64;
            for c in 0..comps {
                out[idx * comps + c] = f32::from_le_bytes(bin[vo + (j * comps + c) * 4..vo + (j * comps + c) * 4 + 4].try_into().unwrap());
            }
        }
    }
    out
}

#[test]
fn glb_has_65_bone_skin_named_morph_targets_and_multiple_materials() {
    let (c, m) = female();
    let (json, bin) = parse_glb(glb(false)).unwrap();
    assert_eq!(json["skins"][0]["joints"].as_array().unwrap().len(), 65);
    assert_eq!(json["skins"].as_array().unwrap().len(), 1);
    // nomes dos morph targets em extras.targetNames, na ordem dos targets de cada primitivo
    let names: Vec<String> = json["meshes"][0]["extras"]["targetNames"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assert_eq!(names, m.names());
    assert_eq!(names.len(), 64);
    let prims = json["meshes"][0]["primitives"].as_array().unwrap();
    assert!(prims.len() >= 15, "pele + 14 peças");
    let mut materials = std::collections::BTreeSet::new();
    for p in prims {
        assert_eq!(p["targets"].as_array().unwrap().len(), names.len(), "todos os primitivos têm o mesmo número de targets");
        materials.insert(p["material"].as_u64().unwrap());
        // POSITION com min/max (exigência do glTF)
        let pos = p["attributes"]["POSITION"].as_u64().unwrap() as usize;
        assert!(json["accessors"][pos]["min"].is_array() && json["accessors"][pos]["max"].is_array());
        assert!(p["attributes"]["JOINTS_0"].is_u64() && p["attributes"]["WEIGHTS_0"].is_u64());
    }
    assert!(materials.len() >= 9, "múltiplos materiais: {materials:?}");
    // inverse bind matrices: translação = -posição de repouso
    let ibm = accessor_f32(&json, &bin, json["skins"][0]["inverseBindMatrices"].as_u64().unwrap() as usize);
    assert_eq!(ibm.len(), 65 * 16);
    for (i, b) in c.skeleton.bones.iter().enumerate() {
        let t = &ibm[i * 16 + 12..i * 16 + 15];
        for k in 0..3 {
            assert!((t[k] + b.world[k]).abs() < 1e-5, "{}", b.name);
        }
    }
    // hierarquia dos nós = hierarquia do esqueleto
    let nodes = json["nodes"].as_array().unwrap();
    for (i, b) in c.skeleton.bones.iter().enumerate() {
        assert_eq!(nodes[i]["name"].as_str().unwrap(), b.name);
        if let Some(p) = b.parent {
            assert!(nodes[p]["children"].as_array().unwrap().iter().any(|x| x.as_u64() == Some(i as u64)));
        }
    }
    // sem VRM: nenhuma extensão VRM
    assert!(json.get("extensions").is_none());
}

#[test]
fn sparse_morph_accessors_round_trip_to_the_expected_positions() {
    let (c, m) = female();
    let (json, bin) = parse_glb(glb(false)).unwrap();
    let prim = &json["meshes"][0]["primitives"][0];
    let names: Vec<String> = json["meshes"][0]["extras"]["targetNames"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    let nv = reference().n_verts;
    for tname in ["blink", "weight_heavy", "eye_size_pos", "neutral"] {
        let ti = names.iter().position(|n| n == tname).unwrap();
        let acc = prim["targets"][ti]["POSITION"].as_u64().unwrap() as usize;
        let delta = accessor_f32(&json, &bin, acc);
        let expect = &m.targets[m.find(tname).unwrap()];
        let mut dense = vec![[0.0f32; 3]; nv];
        for (v, d) in &expect.deltas {
            dense[*v as usize] = *d;
        }
        for v in 0..nv {
            for k in 0..3 {
                assert!((delta[v * 3 + k] - dense[v][k]).abs() < 1e-6, "{tname} v{v}");
            }
        }
    }
    // POSITION do primitivo 0 nos primeiros 10205 vértices = posições do contrato
    let pos = accessor_f32(&json, &bin, prim["attributes"]["POSITION"].as_u64().unwrap() as usize);
    for v in (0..nv).step_by(97) {
        for k in 0..3 {
            assert!((pos[v * 3 + k] - c.positions[v][k]).abs() < 1e-7);
        }
    }
    // vértices extras (costuras de UV) repetem a posição do canônico
    let extra: Vec<u32> = json["meshes"][0]["primitives"][0]["extras"]["anigo_canonical_of_extra"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect();
    assert!(!extra.is_empty());
    for (i, &cv) in extra.iter().enumerate().step_by(31) {
        for k in 0..3 {
            assert_eq!(pos[(nv + i) * 3 + k], pos[cv as usize * 3 + k]);
        }
    }
}

#[test]
fn vrm_extension_has_full_humanoid_expressions_and_lookat() {
    let (_, m) = female();
    let (json, _) = parse_glb(glb(true)).unwrap();
    let vrm = &json["extensions"]["VRMC_vrm"];
    assert_eq!(vrm["specVersion"], "1.0");
    let hb = vrm["humanoid"]["humanBones"].as_object().unwrap();
    assert_eq!(hb.len(), 55);
    for n in humanoid_names() {
        let node = hb[&n]["node"].as_u64().unwrap() as usize;
        assert_eq!(json["nodes"][node]["name"].as_str().unwrap(), n);
    }
    for required in ["hips", "spine", "head", "leftUpperArm", "rightUpperArm", "leftLowerArm", "rightLowerArm", "leftHand", "rightHand", "leftUpperLeg", "rightUpperLeg", "leftLowerLeg", "rightLowerLeg", "leftFoot", "rightFoot"] {
        assert!(hb.contains_key(required), "{required}");
    }
    let preset = vrm["expressions"]["preset"].as_object().unwrap();
    for e in ["happy", "angry", "sad", "relaxed", "surprised", "aa", "ih", "ou", "ee", "oh", "blink", "blinkLeft", "blinkRight"] {
        let binds = preset[e]["morphTargetBinds"].as_array().unwrap();
        assert_eq!(binds.len(), 1);
        let idx = binds[0]["index"].as_u64().unwrap() as usize;
        assert_eq!(m.targets[idx].name, e);
        let node = binds[0]["node"].as_u64().unwrap() as usize;
        assert!(json["nodes"][node]["mesh"].is_u64(), "o nó do bind deve ter a malha");
    }
    assert_eq!(vrm["lookAt"]["type"], "bone");
    assert!(vrm["meta"]["authors"].as_array().unwrap().len() >= 1);
    assert!(json["extensionsUsed"].as_array().unwrap().iter().any(|e| e == "VRMC_vrm"));
    // MToon mínimo em todos os materiais, só em extensionsUsed (não required)
    for mat in json["materials"].as_array().unwrap() {
        assert_eq!(mat["extensions"]["VRMC_materials_mtoon"]["specVersion"], "1.0");
    }
    assert!(json.get("extensionsRequired").is_none());
}

#[test]
fn exports_are_deterministic_and_loadable_by_the_existing_readers() {
    let (c, m) = female();
    let again = export_glb(c, m, &ExportOptions { vrm: false, ..Default::default() });
    assert_eq!(&again, glb(false), "o mesmo personagem deve gerar os mesmos bytes");
    // leitor do core (usado pela Biblioteca): lê o primeiro primitivo = pele
    let mesh = anigo_core::Mesh::from_glb_bytes(glb(false)).expect("anigo-core lê o GLB");
    let built = build_scene(c, m);
    assert_eq!(mesh.vertices.len(), built.scene.meshes[0].primitives[0].vertices.len());
    assert_eq!(mesh.indices.len(), reference().faces.len() * 6);
    assert!(mesh.vertices.iter().all(|v| v.position.iter().all(|x| x.is_finite())));
    // leitor/validador do anigo-vrm
    let r = anigo_vrm::validate_model(glb(false));
    assert!(r.valid, "validate_model: {:?}", r.error);
    anigo_vrm::parse_model(glb(false)).expect("anigo-vrm lê o GLB");
}

#[test]
fn skin_triangles_triangulate_each_quad_in_contract_order() {
    // o GLB exportado pode ser revalidado pelo contrato triangulado (depois de remapear extras)
    let (c, m) = female();
    let built = build_scene(c, m);
    let prim = &built.scene.meshes[0].primitives[0];
    let nv = built.skin_canonical;
    let canon = |i: u32| -> u32 { if (i as usize) < nv { i } else { built.extra_canonical[i as usize - nv] } };
    let tris: Vec<[u32; 3]> = prim.indices.chunks(3).map(|t| [canon(t[0]), canon(t[1]), canon(t[2])]).collect();
    let rep = validate_topology_tris(&c.positions, &tris);
    assert!(rep.is_exact(), "{:?}", rep.messages);
}

// ----------------------------------------------------------------- contrato

#[test]
fn contract_json_is_complete_and_deterministic() {
    let a = contract_json_string();
    let b = contract_json_string();
    assert_eq!(a, b);
    let v = topology_contract();
    assert_eq!(v["skin"]["quads_only"], true);
    assert_eq!(v["skin"]["vertex_count"], reference().n_verts);
    assert_eq!(v["skin"]["boundary_loops"], 3);
    assert_eq!(v["skin"]["euler_characteristic"], -1);
    assert_eq!(v["bone_count"], 65);
    assert_eq!(v["groups"]["zones"].as_array().unwrap().len(), 18);
    assert!(v["groups"]["masks"].as_array().unwrap().len() >= 15);
    assert_eq!(v["parts"].as_array().unwrap().len(), 14);
    assert_eq!(v["morph_targets"].as_array().unwrap().len(), 64);
    assert!(v["landmarks"].as_array().unwrap().len() >= 40);
    let loops = v["loops"].as_array().unwrap();
    assert!(loops.len() >= 60);
    for l in loops {
        assert!(!l["vertices"].as_array().unwrap().is_empty());
    }
    // os 18 nomes de zona do contrato correspondem ao enum do core
    for (_, name) in anigo_charbase::topology::ZONES {
        assert!(v["groups"]["zones"].as_array().unwrap().iter().any(|z| z["name"] == name));
    }
    // contagens e hash conferem com o arquivo versionado (assets/models_v2/topology_contract.json), se existir
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models_v2/topology_contract.json");
    if let Ok(text) = std::fs::read_to_string(&path) {
        let file: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(file["skin"]["connectivity_hash"], v["skin"]["connectivity_hash"], "topologia mudou: regenere assets/models_v2 (export_models)");
        assert_eq!(file["skin"]["vertex_count"], v["skin"]["vertex_count"]);
        assert_eq!(file["skin"]["uv_hash"], v["skin"]["uv_hash"], "UV mudaram: regenere assets/models_v2");
    }
}

#[test]
fn validate_topology_accepts_any_morph_of_the_contract_and_rejects_changes() {
    let t = reference();
    let (c, m) = female();
    let morphed = m.apply(&c.positions, &[("weight_heavy", 1.0), ("age_elderly", 1.0)]);
    let r = validate_topology(&morphed, &t.faces);
    assert!(r.is_exact(), "{:?}", r.messages);
    assert!(r.manifold && r.quads_only && r.finite);
    // esculpir posições arbitrariamente não muda a topologia
    let sculpted: Vec<Vec3> = c.positions.iter().map(|p| *p + Vec3::new(0.01 * (p.y * 9.0).sin(), 0.0, 0.0)).collect();
    assert!(validate_topology(&sculpted, &t.faces).is_exact());

    // vértices reordenados (rotação) -> a conectividade em ordem difere
    let n = t.n_verts as u32;
    let rotated: Vec<[u32; 4]> = t.faces.iter().map(|f| [(f[0] + 1) % n, (f[1] + 1) % n, (f[2] + 1) % n, (f[3] + 1) % n]).collect();
    let r = validate_topology(&c.positions, &rotated);
    assert_eq!(r.status, TopologyStatus::Mismatch);
    assert!(!r.messages.is_empty());
    // face removida
    let r = validate_topology(&c.positions, &t.faces[1..]);
    assert_eq!(r.status, TopologyStatus::Mismatch);
    // vértice extra
    let mut more = c.positions.clone();
    more.push(Vec3::ZERO);
    assert_eq!(validate_topology(&more, &t.faces).status, TopologyStatus::Mismatch);
    // entradas malformadas nunca entram em pânico
    assert_eq!(validate_topology(&[], &[]).status, TopologyStatus::Mismatch);
    assert_eq!(validate_topology(&c.positions, &[[0, 1, 2, u32::MAX]]).status, TopologyStatus::Mismatch);
    let mut nan = c.positions.clone();
    nan[5] = Vec3::NAN;
    let r = validate_topology(&nan, &t.faces);
    assert_eq!(r.status, TopologyStatus::Mismatch);
    assert!(!r.finite);
    // aresta compartilhada por 3 faces
    let mut bad = t.faces.clone();
    bad.push(bad[0]);
    let r = validate_topology(&c.positions, &bad);
    assert!(!r.manifold);
    // triangulada
    assert_eq!(validate_topology_tris(&[], &[]).status, TopologyStatus::Mismatch);
    assert_eq!(validate_topology_tris(&c.positions, &[[0, 1, 99_999_999]]).status, TopologyStatus::Mismatch);
}

#[test]
fn rederive_with_the_contract_topology_matches_direct_generation() {
    let (c, _) = female();
    let r = rederive(&c.positions, Some(&reference().faces)).unwrap();
    assert_eq!(r.method, RederiveMethod::Exact);
    for (a, b) in r.skeleton.bones.iter().zip(&c.skeleton.bones) {
        assert_eq!(a.world, b.world);
    }
    assert_eq!(r.weights.len(), c.positions.len());
    assert!(r.landmarks.iter().any(|(n, _)| n == "lm_nose_tip"));
    // esculpir move as juntas junto: braço mais longo -> cotovelo mais longe
    let mut sculpt = c.positions.clone();
    for (i, p) in sculpt.iter_mut().enumerate() {
        if reference().region[i] == anigo_charbase::cage::Region::Arm && p.x.abs() > 0.4 {
            p.x += p.x.signum() * 0.05;
        }
    }
    let r2 = rederive(&sculpt, None).unwrap();
    let wrist = |sk: &Skeleton| sk.pos(sk.index("leftHand").unwrap()).x;
    assert!(wrist(&r2.skeleton) > wrist(&c.skeleton) + 0.04);
}

#[test]
fn rederive_transfers_by_nearest_neighbor_when_the_topology_changes() {
    let t = reference();
    let (c, _) = female();
    // 1) malha com os vértices embaralhados (mesmos pontos, outra ordem) e escala/posição diferentes
    let n = c.positions.len();
    let perm: Vec<usize> = (0..n).map(|i| (i * 7919 + 13) % n).collect(); // 7919 é primo e n não é múltiplo
    assert_eq!({ let mut s = perm.clone(); s.sort(); s.dedup(); s.len() }, n);
    let shuffled: Vec<Vec3> = perm.iter().map(|&i| c.positions[i] * 1.1 + Vec3::new(0.3, 0.0, -0.2)).collect();
    let mut new_of_old = vec![0u32; n];
    for (new_i, &old_i) in perm.iter().enumerate() {
        new_of_old[old_i] = new_i as u32;
    }
    let shuffled_faces: Vec<[u32; 4]> = t.faces.iter().map(|f| [new_of_old[f[0] as usize], new_of_old[f[1] as usize], new_of_old[f[2] as usize], new_of_old[f[3] as usize]]).collect();
    let r = rederive(&shuffled, Some(&shuffled_faces)).unwrap();
    assert_eq!(r.method, RederiveMethod::NearestNeighbor);
    // esqueleto ~ original (mesma pose, escala e deslocamento aplicados)
    for (a, b) in r.skeleton.bones.iter().zip(&c.skeleton.bones) {
        let expect = Vec3::from_array(b.world) * 1.1 + Vec3::new(0.3, 0.0, -0.2);
        let got = Vec3::from_array(a.world);
        assert!((expect - got).length() < 0.03, "{}: {:?} vs {:?}", a.name, got, expect);
    }
    // pesos: cada vértice novo herda os pesos do vértice de referência correspondente
    let mut same = 0;
    for (new_i, &old_i) in perm.iter().enumerate() {
        if r.weights[new_i] == t.weights[old_i] {
            same += 1;
        }
    }
    assert!(same as f32 > 0.97 * n as f32, "{same}/{n} vértices com pesos idênticos");
    // 2) malha com MENOS vértices (subamostra): ainda deriva esqueleto e pesos válidos
    let sub: Vec<Vec3> = c.positions.iter().step_by(3).copied().collect();
    let r = rederive(&sub, None).unwrap();
    assert_eq!(r.method, RederiveMethod::NearestNeighbor);
    assert_eq!(r.weights.len(), sub.len());
    for w in &r.weights {
        let s: f32 = w.iter().map(|x| x.1).sum();
        assert!((s - 1.0).abs() < 1e-4);
    }
    let head = |sk: &Skeleton| sk.pos(sk.index("head").unwrap());
    assert!((head(&r.skeleton) - head(&c.skeleton)).length() < 0.04);
    // 3) erros tratados
    assert_eq!(rederive(&[], None).unwrap_err(), ContractError::Empty);
    assert_eq!(rederive(&[Vec3::NAN], None).unwrap_err(), ContractError::NotFinite);
    assert!(matches!(rederive(&[Vec3::ZERO, Vec3::X], None).unwrap_err(), ContractError::BadScale(_)));
}

#[test]
fn nearest_map_is_exact_for_identical_point_sets() {
    let (c, _) = female();
    let pts: Vec<Vec3> = c.positions.iter().step_by(11).copied().collect();
    let map = nearest_map(&pts, &pts);
    for (i, &j) in map.iter().enumerate() {
        assert_eq!(i as u32, j);
    }
}

#[test]
fn gender_variants_export_with_identical_structure() {
    let m_ch = generate(Gender::Male);
    let m_m = build_morphs(&m_ch);
    let (f_ch, f_m) = female();
    let a = build_scene(&m_ch, &m_m);
    let b = build_scene(f_ch, f_m);
    assert_eq!(a.target_names, b.target_names);
    assert_eq!(a.scene.meshes[0].primitives.len(), b.scene.meshes[0].primitives.len());
    for (p, q) in a.scene.meshes[0].primitives.iter().zip(&b.scene.meshes[0].primitives) {
        assert_eq!(p.indices, q.indices, "mesmos índices/UV nos dois gêneros");
        assert_eq!(p.vertices.len(), q.vertices.len());
        for (x, y) in p.vertices.iter().zip(&q.vertices) {
            assert_eq!(x.uv, y.uv);
            assert_eq!(x.joints, y.joints);
        }
    }
    let defs = bone_defs(reference());
    let _ = (defs, generate_with(&Params::default()));
}
