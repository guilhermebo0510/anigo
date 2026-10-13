//! Ponte `VRMC_springBone` <-> `anigo-physics`: transforma o modelo lido do arquivo
//! ([`Vrm1SpringBone`] + nós glTF) em um [`SpringBoneDef`] simulável (spring bone VRM-exato) e
//! faz o caminho inverso para exportar cadeias criadas no ANIGO (cabelo/saia/acessórios) como
//! `VRMC_springBone` oficial.

use anigo_physics::{
    ColliderShape, SkeletonNode, SpringBoneDef, SpringChainDef, SpringColliderDef, SpringColliderGroupDef, SpringJointParams,
};
use glam::{Mat4, Quat, Vec3};
use serde_json::Value;

use crate::vrm1::{
    Vrm1Collider, Vrm1ColliderGroup, Vrm1ColliderShape, Vrm1Spring, Vrm1SpringBone, Vrm1SpringJoint, SPRING_BONE_SPEC_VERSION,
};

fn vec3_of(v: Option<&Value>, default: Vec3) -> Vec3 {
    v.and_then(Value::as_array)
        .filter(|a| a.len() == 3)
        .and_then(|a| Some(Vec3::new(a[0].as_f64()? as f32, a[1].as_f64()? as f32, a[2].as_f64()? as f32)))
        .filter(|v| v.is_finite())
        .unwrap_or(default)
}

/// Nós glTF (`nodes[*]`: TRS ou `matrix`, `children`) -> nós do esqueleto de física. Pais vêm de
/// `children`; valores inválidos viram o padrão (identidade) para não derrubar o import.
pub fn skeleton_nodes_from_gltf(nodes: &[Value]) -> Vec<SkeletonNode> {
    let mut out: Vec<SkeletonNode> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let name = node.get("name").and_then(Value::as_str).map(String::from).unwrap_or_else(|| format!("node_{i}"));
            let (mut t, mut r, mut s) = (Vec3::ZERO, Quat::IDENTITY, Vec3::ONE);
            if let Some(m) = node.get("matrix").and_then(Value::as_array).filter(|a| a.len() == 16) {
                let f: Vec<f32> = m.iter().filter_map(|v| v.as_f64()).map(|v| v as f32).collect();
                if f.len() == 16 {
                    let mat = Mat4::from_cols_slice(&f);
                    if mat.is_finite() && mat.determinant().abs() > 1e-12 {
                        let (ds, dr, dt) = mat.to_scale_rotation_translation();
                        (t, r, s) = (dt, dr, ds);
                    }
                }
            } else {
                t = vec3_of(node.get("translation"), Vec3::ZERO);
                s = vec3_of(node.get("scale"), Vec3::ONE);
                if let Some(q) = node.get("rotation").and_then(Value::as_array).filter(|a| a.len() == 4) {
                    let c: Vec<f32> = q.iter().filter_map(|v| v.as_f64()).map(|v| v as f32).collect();
                    if c.len() == 4 {
                        let quat = Quat::from_xyzw(c[0], c[1], c[2], c[3]);
                        if quat.is_finite() && quat.length_squared() > 1e-12 {
                            r = quat.normalize();
                        }
                    }
                }
            }
            SkeletonNode { name, parent: None, translation: t, rotation: r, scale: s }
        })
        .collect();
    for (i, node) in nodes.iter().enumerate() {
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            for c in children.iter().filter_map(Value::as_u64) {
                let c = c as usize;
                if c < out.len() && c != i && out[c].parent.is_none() {
                    out[c].parent = Some(i);
                }
            }
        }
    }
    out
}

/// Detecta e quebra ciclos de `children` malformados (mantém a primeira aresta, derruba as demais).
fn break_cycles(nodes: &mut [SkeletonNode]) {
    for i in 0..nodes.len() {
        let mut cursor = nodes[i].parent;
        let mut hops = 0usize;
        while let Some(p) = cursor {
            hops += 1;
            if p == i || hops > nodes.len() {
                nodes[i].parent = None;
                break;
            }
            cursor = nodes[p].parent;
        }
    }
}

/// `VRMC_springBone` + nós -> definição simulável. Referências inválidas (nó fora da faixa,
/// índices de grupo) são descartadas com aviso em vez de falhar.
pub fn spring_def_from_vrm(spring: &Vrm1SpringBone, gltf_nodes: &[Value]) -> (SpringBoneDef, Vec<String>) {
    let mut warnings = Vec::new();
    let mut nodes = skeleton_nodes_from_gltf(gltf_nodes);
    break_cycles(&mut nodes);
    let n = nodes.len();
    if n == 0 {
        warnings.push("modelo sem nós glTF — springBone ignorado".into());
        return (SpringBoneDef::default(), warnings);
    }

    // colliders: mantém o índice (descartados viram esfera inofensiva) para não deslocar os grupos
    let colliders: Vec<SpringColliderDef> = spring
        .colliders
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let shape = match c.shape {
                Vrm1ColliderShape::Sphere { offset, radius, inside: false } => ColliderShape::Sphere { center: offset.into(), radius },
                Vrm1ColliderShape::Sphere { offset, radius, inside: true } => ColliderShape::InsideSphere { center: offset.into(), radius },
                Vrm1ColliderShape::Capsule { offset, radius, tail, inside: false } => {
                    ColliderShape::Capsule { a: offset.into(), b: tail.into(), radius }
                }
                Vrm1ColliderShape::Capsule { offset, radius, tail, inside: true } => {
                    ColliderShape::InsideCapsule { a: offset.into(), b: tail.into(), radius }
                }
                Vrm1ColliderShape::Plane { offset, normal } => ColliderShape::Plane { point: offset.into(), normal: normal.into() },
            };
            if (c.node as usize) >= n {
                warnings.push(format!("collider {i}: nó {} fora da faixa — desativado", c.node));
                return SpringColliderDef { node: 0, shape: ColliderShape::Sphere { center: Vec3::new(0.0, -1.0e4, 0.0), radius: 0.0 } };
            }
            SpringColliderDef { node: c.node as usize, shape }
        })
        .collect();
    let collider_groups: Vec<SpringColliderGroupDef> = spring
        .collider_groups
        .iter()
        .map(|g| SpringColliderGroupDef {
            name: g.name.clone().unwrap_or_default(),
            colliders: g.colliders.iter().map(|c| *c as usize).filter(|c| *c < colliders.len()).collect(),
        })
        .collect();

    let mut chains = Vec::new();
    for (i, s) in spring.springs.iter().enumerate() {
        let joints: Vec<SpringJointParams> = s
            .joints
            .iter()
            .filter(|j| {
                let ok = (j.node as usize) < n;
                if !ok {
                    warnings.push(format!("spring {i}: junta com nó {} fora da faixa — descartada", j.node));
                }
                ok
            })
            .map(|j| SpringJointParams {
                node: j.node as usize,
                hit_radius: j.hit_radius,
                stiffness: j.stiffness,
                gravity_power: j.gravity_power,
                gravity_dir: j.gravity_dir.into(),
                drag_force: j.drag_force,
            })
            .collect();
        let center = s.center.map(|c| c as usize).filter(|c| {
            let ok = *c < n;
            if !ok {
                warnings.push(format!("spring {i}: center {c} fora da faixa — ignorado"));
            }
            ok
        });
        chains.push(SpringChainDef {
            name: s.name.clone().unwrap_or_else(|| format!("spring_{i}")),
            joints,
            collider_groups: s.collider_groups.iter().map(|g| *g as usize).filter(|g| *g < collider_groups.len()).collect(),
            center,
        });
    }
    (SpringBoneDef { nodes, chains, colliders, collider_groups }, warnings)
}

/// Caminho inverso: cadeias/colliders da física -> `VRMC_springBone` oficial (para exportar). Os
/// índices de nó são os do esqueleto de física (devem coincidir com os nós glTF exportados).
pub fn vrm_from_spring_def(def: &SpringBoneDef) -> Vrm1SpringBone {
    let colliders = def
        .colliders
        .iter()
        .map(|c| {
            let shape = match c.shape {
                ColliderShape::Sphere { center, radius } => Vrm1ColliderShape::Sphere { offset: center.into(), radius, inside: false },
                ColliderShape::InsideSphere { center, radius } => Vrm1ColliderShape::Sphere { offset: center.into(), radius, inside: true },
                ColliderShape::Capsule { a, b, radius } => Vrm1ColliderShape::Capsule { offset: a.into(), radius, tail: b.into(), inside: false },
                ColliderShape::InsideCapsule { a, b, radius } => {
                    Vrm1ColliderShape::Capsule { offset: a.into(), radius, tail: b.into(), inside: true }
                }
                ColliderShape::Plane { point, normal } => Vrm1ColliderShape::Plane { offset: point.into(), normal: normal.into() },
            };
            Vrm1Collider { node: c.node as u64, shape }
        })
        .collect();
    let collider_groups = def
        .collider_groups
        .iter()
        .map(|g| Vrm1ColliderGroup {
            name: if g.name.is_empty() { None } else { Some(g.name.clone()) },
            colliders: g.colliders.iter().map(|c| *c as u64).collect(),
        })
        .collect();
    let springs = def
        .chains
        .iter()
        .map(|c| Vrm1Spring {
            name: if c.name.is_empty() { None } else { Some(c.name.clone()) },
            joints: c
                .joints
                .iter()
                .map(|j| Vrm1SpringJoint {
                    node: j.node as u64,
                    hit_radius: j.hit_radius,
                    stiffness: j.stiffness,
                    gravity_power: j.gravity_power,
                    gravity_dir: j.gravity_dir.into(),
                    drag_force: j.drag_force,
                })
                .collect(),
            collider_groups: c.collider_groups.iter().map(|g| *g as u64).collect(),
            center: c.center.map(|c| c as u64),
        })
        .collect();
    Vrm1SpringBone { spec_version: SPRING_BONE_SPEC_VERSION.to_string(), colliders, collider_groups, springs }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exporter::{ExportNode, ExportScene, VrmExportData};
    use crate::{export_gltf, export_model, parse_model};
    use anigo_physics::{SpringBoneSystem, SpringColliderGroupDef};

    const DT: f32 = 1.0 / 60.0;

    /// root(0) -> a(1) -> b(2) -> c(3), cada um 0.1 m abaixo do anterior; colisor no nó 4 (cabeça).
    fn scene() -> ExportScene {
        let mut nodes = vec![
            ExportNode { name: Some("root".into()), translation: Some([0.0, 1.5, 0.0]), children: vec![1], ..Default::default() },
            ExportNode { name: Some("a".into()), translation: Some([0.1, 0.0, 0.0]), children: vec![2], ..Default::default() },
            ExportNode { name: Some("b".into()), translation: Some([0.1, 0.0, 0.0]), children: vec![3], ..Default::default() },
            ExportNode { name: Some("c".into()), translation: Some([0.1, 0.0, 0.0]), ..Default::default() },
            ExportNode { name: Some("head".into()), translation: Some([0.0, -0.2, 0.0]), ..Default::default() },
        ];
        nodes[0].children = vec![1, 4];
        ExportScene { root_nodes: vec![0], nodes, ..Default::default() }
    }

    fn def() -> SpringBoneDef {
        let (d, w) = spring_def_from_vrm(&Vrm1SpringBone::default(), &[]);
        assert!(w.iter().any(|x| x.contains("sem nós")));
        let mut d = d;
        d.chains.push(SpringChainDef {
            name: "hair".into(),
            joints: vec![
                SpringJointParams { node: 1, stiffness: 0.4, gravity_power: 1.0, drag_force: 0.3, hit_radius: 0.01, ..Default::default() },
                SpringJointParams { node: 2, stiffness: 0.3, gravity_power: 1.0, drag_force: 0.3, hit_radius: 0.01, ..Default::default() },
                SpringJointParams::new(3),
            ],
            collider_groups: vec![0],
            center: Some(0),
        });
        d.colliders.push(SpringColliderDef { node: 4, shape: ColliderShape::Sphere { center: Vec3::ZERO, radius: 0.1 } });
        d.collider_groups.push(SpringColliderGroupDef { name: "head".into(), colliders: vec![0] });
        d
    }

    #[test]
    fn fisica_para_vrm_para_fisica_ida_e_volta() {
        let d = def();
        let vrm = vrm_from_spring_def(&d);
        let data = VrmExportData { spring_bone: Some(vrm.clone()), ..Default::default() };
        let bytes = export_model(&scene(), Some(&data));
        let parsed = parse_model(&bytes).expect("arquivo parseia");
        let sb = parsed.vrm.unwrap().spring_bone.expect("springBone");
        assert_eq!(sb, vrm, "VRM exportado e relido e identico");
        let (back, warnings) = spring_def_from_vrm(&sb, parsed.model.json["nodes"].as_array().unwrap());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(back.chains, d.chains);
        assert_eq!(back.colliders, d.colliders);
        assert_eq!(back.collider_groups, d.collider_groups);
        // nos: pais e translacoes vindos do glTF
        assert_eq!(back.nodes[2].parent, Some(1));
        assert!((back.nodes[1].translation - Vec3::new(0.1, 0.0, 0.0)).length() < 1e-6);
    }

    #[test]
    fn vrm_exportado_pelo_anigo_simula_no_motor_de_spring_bone() {
        let d = def();
        let data = VrmExportData { spring_bone: Some(vrm_from_spring_def(&d)), ..Default::default() };
        let (json, _) = export_gltf(&scene(), Some(&data));
        let nodes = json["nodes"].as_array().unwrap();
        let sb = crate::vrm1::parse_spring_bone(json.pointer("/extensions/VRMC_springBone"), nodes, &mut Vec::new()).unwrap();
        let (def_back, warnings) = spring_def_from_vrm(&sb, nodes);
        assert!(warnings.is_empty());
        let mut sys = SpringBoneSystem::new(&def_back).expect("rig valido");
        assert_eq!(sys.pair_count(), 2);
        let mut max_len_err = 0.0f32;
        let mut max_swing = 0.0f32;
        for f in 0..300 {
            let t = f as f32 * DT;
            sys.skeleton_mut().set_local_translation(0, Vec3::new((t * 4.0).sin() * 0.3, 1.5, 0.0));
            sys.step(DT);
            for i in 0..sys.pair_count() {
                let (h, tl) = sys.pair_nodes(i).unwrap();
                let len = (sys.skeleton().world_position(tl) - sys.skeleton().world_position(h)).length();
                max_len_err = max_len_err.max((len - sys.pair_bone_length(i).unwrap()).abs());
            }
            max_swing = max_swing.max(sys.node_rotation(1).angle_between(Quat::IDENTITY));
        }
        eprintln!("VRM exportado simulado: erro max de comprimento {max_len_err:.2e}, swing max {max_swing:.3} rad");
        assert!(max_len_err < 1e-4);
        assert!(max_swing > 0.05, "a cadeia reage ao movimento do personagem");
    }

    #[test]
    fn referencias_invalidas_sao_descartadas_com_aviso() {
        let mut sb = vrm_from_spring_def(&def());
        sb.springs[0].joints.push(Vrm1SpringJoint { node: 77, ..Default::default() });
        sb.springs[0].center = Some(90);
        sb.colliders[0].node = 55;
        let nodes = scene();
        let (_, json_nodes) = {
            let (json, _) = export_gltf(&nodes, None);
            let list = json["nodes"].as_array().unwrap().clone();
            ((), list)
        };
        let (d, warnings) = spring_def_from_vrm(&sb, &json_nodes);
        assert!(warnings.len() >= 3, "{warnings:?}");
        assert!(SpringBoneSystem::new(&d).is_ok(), "o resultado sempre constroi");
    }

    #[test]
    fn gltf_malformado_nao_causa_panic() {
        let nodes: Vec<Value> = vec![
            serde_json::json!({ "children": [0, 1, 99], "translation": [1, 2] }),
            serde_json::json!({ "children": [0], "rotation": [0, 0, 0, 0], "scale": ["x", 1, 1] }),
            serde_json::json!({ "matrix": [1, 2, 3] }),
            serde_json::json!({ "matrix": [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0] }),
        ];
        let (d, _) = spring_def_from_vrm(&vrm_from_spring_def(&def()), &nodes);
        assert!(SpringBoneSystem::new(&d).is_ok());
        let sk = skeleton_nodes_from_gltf(&nodes);
        assert_eq!(sk.len(), 4);
        assert!(sk.iter().all(|n| n.rotation.is_finite() && n.translation.is_finite()));
    }
}
