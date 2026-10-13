//! Testes da Gen-0: topologia, simetria, isomorfismo, esqueleto e pesos. Determinísticos e sem GPU.

use anigo_charbase::character::skin_positions;
use anigo_charbase::params::{FaceParams, Params, FACE_PARAM_NAMES};
use anigo_charbase::qmesh::{edge_key, QuadMesh};
use anigo_charbase::skeleton::{bone_defs, humanoid_names, Skeleton};
use anigo_charbase::topology::{hash_faces, quad_mesh_of, reference, symmetrize};
use anigo_charbase::{generate, generate_with, Gender};
use glam::Vec3;
use std::collections::BTreeSet;

fn extremes() -> Vec<(String, Params)> {
    let mut v: Vec<(String, Params)> = Vec::new();
    for g in [0.0f32, 0.3, 0.5, 1.0] {
        v.push((format!("gender={g}"), Params { gender: g, ..Params::default() }));
    }
    let base = Params::default();
    for s in [-1.0f32, 1.0] {
        v.push((format!("weight={s}"), Params { weight: s, ..base }));
        v.push((format!("muscle={s}"), Params { muscle: s, ..base }));
        v.push((format!("height={s}"), Params { height: s, ..base }));
        v.push((format!("head={s}"), Params { head_size: s, ..base }));
    }
    v.push(("age=1".into(), Params { age: 1.0, ..base }));
    v.push(("age=0.5".into(), Params { age: 0.5, ..base }));
    for (i, name) in FACE_PARAM_NAMES.iter().enumerate() {
        for s in [-1.0f32, 1.0] {
            let mut f = FaceParams::default();
            f.set(i, s);
            v.push((format!("face:{name}={s}"), Params { face: f, ..base }));
        }
    }
    // todos os extremos ao mesmo tempo (magro + velho, gordo + musculoso, etc.)
    v.push((
        "combo_thin_old_male".into(),
        Params { gender: 0.0, age: 1.0, weight: -1.0, muscle: -1.0, height: 1.0, ..base },
    ));
    v.push((
        "combo_fat_muscle_female".into(),
        Params { gender: 1.0, age: 0.7, weight: 1.0, muscle: 1.0, height: -1.0, head_size: 1.0, ..base },
    ));
    v
}

#[test]
fn skin_is_pure_quad_manifold_with_three_boundary_loops() {
    let topo = reference();
    let m = quad_mesh_of(topo, &skin_positions(&Params::default()));
    // só quads, sem índice fora da faixa, sem vértice repetido na mesma face
    for f in &m.faces {
        let s: BTreeSet<u32> = f.iter().copied().collect();
        assert_eq!(s.len(), 4, "face degenerada {f:?}");
        assert!(f.iter().all(|&i| (i as usize) < m.pos.len()));
    }
    let ef = m.edge_faces();
    assert!(ef.values().all(|v| v.len() <= 2), "aresta não-manifold");
    // fronteiras: olho L, olho R, boca (as demais aberturas são fechadas por tampas/pescoço contínuo)
    let loops = m.boundary_loops();
    assert_eq!(loops.len(), 3, "esperava 3 laços de fronteira (olhos e boca)");
    // esfera com 3 discos removidos: chi = 2 - 3
    assert_eq!(m.euler(), -1);
    // todo vértice é usado
    let used: BTreeSet<u32> = m.faces.iter().flatten().copied().collect();
    assert_eq!(used.len(), m.pos.len());
    // alvo de 10k-14k vértices
    assert!((10_000..=14_000).contains(&m.pos.len()), "vértices do Skin: {}", m.pos.len());
}

#[test]
fn valence_is_mostly_regular() {
    let topo = reference();
    let m = quad_mesh_of(topo, &skin_positions(&Params::default()));
    let val = m.valences();
    let reg = val.iter().filter(|&&v| v == 4).count() as f32 / val.len() as f32;
    let max = *val.iter().max().unwrap();
    assert!(reg > 0.90, "fração de vértices regulares (valência 4) = {reg}");
    assert!(max <= 8, "valência máxima {max}");
}

#[test]
fn topology_is_identical_for_all_parameter_extremes() {
    let topo = reference();
    let h0 = hash_faces(&topo.faces, topo.n_verts);
    for (name, p) in extremes() {
        let pos = skin_positions(&p);
        assert_eq!(pos.len(), topo.n_verts, "{name}");
        assert!(pos.iter().all(|v| v.is_finite()), "{name}: posição não finita");
    }
    // as conectividades de masculino e feminino coincidem exatamente
    let cage_m = anigo_charbase::cage::build_cage(&Params { gender: 0.0, ..Params::default() });
    let cage_f = anigo_charbase::cage::build_cage(&Params { gender: 1.0, ..Params::default() });
    assert_eq!(cage_m.mesh.faces, cage_f.mesh.faces);
    let r_m = anigo_charbase::subdiv::catmull_clark(&cage_m.mesh);
    let r_f = anigo_charbase::subdiv::catmull_clark(&cage_f.mesh);
    assert_eq!(hash_faces(&r_m.mesh.faces, r_m.mesh.pos.len()), hash_faces(&r_f.mesh.faces, r_f.mesh.pos.len()));
    assert_eq!(hash_faces(&r_m.mesh.faces, r_m.mesh.pos.len()), h0);
    // e para os extremos de macro
    for (name, p) in extremes() {
        let cage = anigo_charbase::cage::build_cage(&p);
        assert_eq!(cage.mesh.faces, cage_f.mesh.faces, "{name}: gaiola com conectividade diferente");
        assert_eq!(cage.mesh.pos.len(), cage_f.mesh.pos.len(), "{name}");
    }
}

#[test]
fn skin_is_exactly_symmetric_in_x() {
    let topo = reference();
    for (name, p) in extremes().into_iter().take(12) {
        let pos = skin_positions(&p);
        for i in 0..pos.len() {
            let j = topo.mirror[i] as usize;
            assert_eq!(topo.mirror[j] as usize, i, "{name}: espelho não é involução");
            assert_eq!(pos[j].x, -pos[i].x, "{name}: x do vértice {i}");
            assert_eq!(pos[j].y, pos[i].y, "{name}");
            assert_eq!(pos[j].z, pos[i].z, "{name}");
        }
    }
    // o mapa de simetria é um automorfismo das faces
    let set: BTreeSet<[u32; 4]> = topo
        .faces
        .iter()
        .map(|f| {
            let mut s = *f;
            let mn = (0..4).min_by_key(|&k| s[k]).unwrap();
            s.rotate_left(mn);
            s
        })
        .collect();
    for f in &topo.faces {
        let mut m = [topo.mirror[f[0] as usize], topo.mirror[f[1] as usize], topo.mirror[f[2] as usize], topo.mirror[f[3] as usize]];
        m.reverse(); // o espelho inverte o enrolamento
        let mn = (0..4).min_by_key(|&k| m[k]).unwrap();
        m.rotate_left(mn);
        assert!(set.contains(&m), "face espelhada ausente: {f:?}");
    }
}

#[test]
fn normals_point_outward_and_no_flipped_faces() {
    for (name, p) in extremes() {
        let pos = skin_positions(&p);
        let topo = reference();
        let m = quad_mesh_of(topo, &pos);
        assert!(m.signed_volume() > 0.0, "{name}: volume com sinal não positivo");
        let vn = m.vertex_normals();
        let mut flipped = 0usize;
        for f in &m.faces {
            let q: Vec<Vec3> = f.iter().map(|&i| pos[i as usize]).collect();
            let n = (q[2] - q[0]).cross(q[3] - q[1]);
            let avg = f.iter().map(|&i| vn[i as usize]).sum::<Vec3>();
            if n.dot(avg) < 0.0 {
                flipped += 1;
            }
        }
        assert!(flipped as f32 <= m.faces.len() as f32 * 0.002, "{name}: {flipped} faces invertidas");
    }
}

#[test]
fn required_loops_exist_as_closed_edge_cycles() {
    let topo = reference();
    let m = quad_mesh_of(topo, &skin_positions(&Params::default()));
    let edges: BTreeSet<(u32, u32)> = m.edge_faces().keys().copied().collect();
    let closed_required = [
        "eye_L_ring0", "eye_L_ring1", "eye_L_ring2", "eye_L_ring3", "eye_R_ring0", "eye_R_ring1", "eye_R_ring2", "eye_R_ring3", "mouth_ring0",
        "mouth_ring1", "mouth_ring2", "mouth_ring3", "ear_L_ring0", "ear_L_ring1", "ear_L_ring2", "ear_L_ring3", "ear_R_ring0", "ear_R_ring1",
        "ear_R_ring2", "ear_R_ring3", "neck_ring0", "neck_ring1", "neck_ring2", "clavicle_loop", "axilla_row", "shoulder_hole_L", "shoulder_hole_R",
        "elbow_L_ring0", "elbow_L_ring1", "elbow_L_ring2", "elbow_R_ring0", "elbow_R_ring1", "elbow_R_ring2", "wrist_L_ring0", "wrist_L_ring1",
        "wrist_R_ring0", "wrist_R_ring1", "knee_L_ring0", "knee_L_ring1", "knee_L_ring2", "knee_R_ring0", "knee_R_ring1", "knee_R_ring2",
        "ankle_L_ring0", "ankle_L_ring1", "ankle_R_ring0", "ankle_R_ring1", "waist_loop", "navel_loop", "pelvis_loop_a", "pelvis_loop_b",
        "crotch_loop", "heel_L_loop", "arch_L_loop", "toe_base_L_loop", "heel_R_loop", "arch_R_loop", "toe_base_R_loop", "nose_alar_ring",
        "nose_tip_ring", "jaw_base_loop", "palm_L_knuckle_loop", "palm_R_knuckle_loop", "crotch_floor",
    ];
    for name in closed_required {
        let l = topo.loop_by_name(name).unwrap_or_else(|| panic!("laço ausente: {name}"));
        assert!(l.closed, "{name}");
        assert!(l.verts.len() >= 4, "{name}: laço curto");
        let uniq: BTreeSet<u32> = l.verts.iter().copied().collect();
        assert_eq!(uniq.len(), l.verts.len(), "{name}: vértice repetido");
        for k in 0..l.verts.len() {
            let (a, b) = (l.verts[k], l.verts[(k + 1) % l.verts.len()]);
            assert!(edges.contains(&edge_key(a, b)), "{name}: aresta {a}-{b} não existe");
        }
    }
    // três anéis por falange (junta no meio) nos dedos, ao menos 2 anéis no pulso/tornozelo
    for s in ["L", "R"] {
        for f in ["index", "middle", "ring", "little", "thumb"] {
            let jn = if f == "thumb" { ["cmc", "mcp", "ip"] } else { ["mcp", "pip", "dip"] };
            for j in jn {
                let l = topo.loop_by_name(&format!("finger_{s}_{f}_{j}")).unwrap_or_else(|| panic!("falta junta {s} {f} {j}"));
                for k in 0..l.verts.len() {
                    assert!(edges.contains(&edge_key(l.verts[k], l.verts[(k + 1) % l.verts.len()])));
                }
            }
        }
    }
    // cadeias abertas: sobrancelha e queixo
    for name in ["brow_line", "chin_line"] {
        let l = topo.loop_by_name(name).unwrap();
        assert!(!l.closed);
        for w in l.verts.windows(2) {
            assert!(edges.contains(&edge_key(w[0], w[1])), "{name}");
        }
    }
}

#[test]
fn eye_and_mouth_have_at_least_three_rings_plus_transition() {
    let topo = reference();
    for s in ["eye_L", "eye_R", "mouth"] {
        let n = (0..8).filter(|k| topo.loop_by_name(&format!("{s}_ring{k}")).is_some()).count();
        assert!(n >= 4, "{s}: {n} anéis (>= 3 + transição)");
    }
}

#[test]
fn generation_is_deterministic() {
    let a = generate(Gender::Female);
    let b = generate(Gender::Female);
    assert_eq!(a.positions, b.positions);
    assert_eq!(a.skeleton.bones.len(), b.skeleton.bones.len());
    for (x, y) in a.skeleton.bones.iter().zip(&b.skeleton.bones) {
        assert_eq!(x.world, y.world);
    }
}

#[test]
fn malformed_parameters_never_panic() {
    let bad = Params {
        gender: f32::NAN,
        age: f32::INFINITY,
        weight: -1e30,
        muscle: 7.0,
        height: f32::NEG_INFINITY,
        head_size: f32::NAN,
        face: FaceParams { eye_size: 1e9, nose_size: f32::NAN, ..FaceParams::default() },
    };
    let c = generate_with(&bad);
    assert!(c.positions.iter().all(|p| p.is_finite()));
    let _ = generate(Gender::Blend(f32::NAN));
    let _ = generate(Gender::Blend(-5.0));
    let _ = generate(Gender::Blend(5.0));
}

#[test]
fn gender_blend_is_continuous() {
    let m = skin_positions(&Params { gender: 0.0, ..Params::default() });
    let f = skin_positions(&Params { gender: 1.0, ..Params::default() });
    let mid = skin_positions(&Params { gender: 0.5, ..Params::default() });
    let near = skin_positions(&Params { gender: 0.52, ..Params::default() });
    let dm: f32 = m.iter().zip(&f).map(|(a, b)| (*a - *b).length()).fold(0.0, f32::max);
    assert!(dm > 0.02, "masculino e feminino praticamente iguais");
    let dn: f32 = mid.iter().zip(&near).map(|(a, b)| (*a - *b).length()).fold(0.0, f32::max);
    assert!(dn < 0.15 * dm, "mudança de gênero descontínua: {dn} vs {dm}");
}

// ----------------------------------------------------------------- esqueleto

#[test]
fn skeleton_has_exact_vrm_humanoid_hierarchy() {
    let c = generate(Gender::Female);
    let sk = &c.skeleton;
    assert_eq!(sk.len(), 65);
    let names = humanoid_names();
    assert_eq!(names.len(), 55);
    for (i, n) in names.iter().enumerate() {
        assert_eq!(sk.bones[i].name, *n, "ordem dos 55 ossos humanoid");
        assert!(sk.bones[i].humanoid);
    }
    let parent = |n: &str| -> Option<String> { sk.index(n).and_then(|i| sk.bones[i].parent).map(|p| sk.bones[p].name.clone()) };
    assert_eq!(parent("hips"), None);
    assert_eq!(parent("spine").as_deref(), Some("hips"));
    assert_eq!(parent("chest").as_deref(), Some("spine"));
    assert_eq!(parent("upperChest").as_deref(), Some("chest"));
    assert_eq!(parent("neck").as_deref(), Some("upperChest"));
    assert_eq!(parent("head").as_deref(), Some("neck"));
    for e in ["leftEye", "rightEye", "jaw"] {
        assert_eq!(parent(e).as_deref(), Some("head"));
    }
    for side in ["left", "right"] {
        assert_eq!(parent(&format!("{side}Shoulder")).as_deref(), Some("upperChest"));
        assert_eq!(parent(&format!("{side}UpperArm")), Some(format!("{side}Shoulder")));
        assert_eq!(parent(&format!("{side}LowerArm")), Some(format!("{side}UpperArm")));
        assert_eq!(parent(&format!("{side}Hand")), Some(format!("{side}LowerArm")));
        assert_eq!(parent(&format!("{side}ThumbMetacarpal")), Some(format!("{side}Hand")));
        assert_eq!(parent(&format!("{side}ThumbProximal")), Some(format!("{side}ThumbMetacarpal")));
        assert_eq!(parent(&format!("{side}ThumbDistal")), Some(format!("{side}ThumbProximal")));
        for f in ["Index", "Middle", "Ring", "Little"] {
            assert_eq!(parent(&format!("{side}{f}Proximal")), Some(format!("{side}Hand")));
            assert_eq!(parent(&format!("{side}{f}Intermediate")), Some(format!("{side}{f}Proximal")));
            assert_eq!(parent(&format!("{side}{f}Distal")), Some(format!("{side}{f}Intermediate")));
        }
        assert_eq!(parent(&format!("{side}UpperLeg")).as_deref(), Some("hips"));
        assert_eq!(parent(&format!("{side}LowerLeg")), Some(format!("{side}UpperLeg")));
        assert_eq!(parent(&format!("{side}Foot")), Some(format!("{side}LowerLeg")));
        assert_eq!(parent(&format!("{side}Toes")), Some(format!("{side}Foot")));
    }
    // extras: 8 twist + 2 busto, não humanoid
    for n in anigo_charbase::skeleton::TWIST_BONES.iter().chain(anigo_charbase::skeleton::BUST_BONES.iter()) {
        let i = sk.index(n).unwrap_or_else(|| panic!("osso extra ausente {n}"));
        assert!(!sk.bones[i].humanoid);
    }
    // pais sempre antes dos filhos
    for (i, b) in sk.bones.iter().enumerate() {
        if let Some(p) = b.parent {
            assert!(p < i);
        }
    }
}

#[test]
fn skeleton_is_a_symmetric_t_pose() {
    for g in [Gender::Male, Gender::Female] {
        let c = generate(g);
        let sk = &c.skeleton;
        for b in &sk.bones {
            let (n, other) = if let Some(r) = b.name.strip_prefix("left") {
                (b, format!("right{r}"))
            } else if let Some(r) = b.name.strip_prefix("tw_left") {
                (b, format!("tw_right{r}"))
            } else if b.name == "bust_L" {
                (b, "bust_R".to_string())
            } else {
                continue;
            };
            let o = &sk.bones[sk.index(&other).unwrap()];
            assert!((n.world[0] + o.world[0]).abs() < 1e-4, "{} x", n.name);
            assert!((n.world[1] - o.world[1]).abs() < 1e-4, "{} y", n.name);
            assert!((n.world[2] - o.world[2]).abs() < 1e-4, "{} z", n.name);
        }
        // T-pose: o braço é horizontal (y e z do cotovelo e punho ~ ombro)
        let p = |n: &str| sk.pos(sk.index(n).unwrap());
        let sh = p("leftUpperArm");
        for n in ["leftLowerArm", "leftHand", "leftMiddleProximal", "leftMiddleDistal"] {
            let q = p(n);
            assert!((q.y - sh.y).abs() < 0.03, "{n}: braço fora da T-pose (dy={})", q.y - sh.y);
            assert!(q.x > sh.x, "{n}");
        }
        // pernas verticais, pés no chão
        let hip = p("leftUpperLeg");
        let knee = p("leftLowerLeg");
        let ank = p("leftFoot");
        assert!(hip.y > knee.y && knee.y > ank.y && ank.y > 0.0);
        assert!((hip.x - ank.x).abs() < 0.03);
        // comprimentos plausíveis (membros simétricos, nada degenerado)
        for b in &sk.bones {
            if let Some(par) = b.parent {
                let d = (Vec3::from_array(b.world) - sk.pos(par)).length();
                assert!(d < 0.8, "{}: osso muito longo {d}", b.name);
            }
        }
        let height = c.positions.iter().map(|v| v.y).fold(f32::MIN, f32::max);
        assert!((1.4..2.0).contains(&height), "altura {height}");
        assert!(c.positions.iter().map(|v| v.y).fold(f32::MAX, f32::min) > -0.01);
    }
}

#[test]
fn skeleton_rederives_from_any_morphed_mesh() {
    let topo = reference();
    let defs = bone_defs(topo);
    for g in [0.0f32, 1.0] {
        let a = generate_with(&Params { gender: g, ..Params::default() });
        for locator_sum in defs.iter().map(|d| d.locator.weight_sum()) {
            assert!((locator_sum - 1.0).abs() < 1e-4, "localizador não afim: soma {locator_sum}");
        }
        let again = Skeleton::derive(&defs, &a.positions);
        for (x, y) in a.skeleton.bones.iter().zip(&again.bones) {
            assert_eq!(x.world, y.world);
        }
    }
}

// ----------------------------------------------------------------- pesos

#[test]
fn skin_weights_are_normalized_with_at_most_four_influences() {
    let topo = reference();
    assert_eq!(topo.weights.len(), topo.n_verts);
    for (v, w) in topo.weights.iter().enumerate() {
        let sum: f32 = w.iter().map(|x| x.1).sum();
        assert!((sum - 1.0).abs() < 1e-4, "vértice {v}: soma {sum}");
        assert!(w.iter().all(|x| x.1 >= 0.0 && x.1 <= 1.0 + 1e-6));
        let nonzero = w.iter().filter(|x| x.1 > 0.0).count();
        assert!(nonzero <= 4);
        assert!(w.iter().all(|x| (x.0 as usize) < 65));
    }
}

fn bone_weights(topo: &anigo_charbase::topology::Topology, sk: &Skeleton, v: usize) -> Vec<(String, f32)> {
    topo.weights[v].iter().filter(|x| x.1 > 1e-6).map(|x| (sk.bones[x.0 as usize].name.clone(), x.1)).collect()
}

#[test]
fn weights_do_not_leak_between_arms_trunk_and_legs() {
    let topo = reference();
    let c = generate(Gender::Female);
    let sk = &c.skeleton;
    let elbow = sk.pos(sk.index("leftLowerArm").unwrap());
    let armpit_y = sk.pos(sk.index("leftUpperArm").unwrap()).y - 0.12;
    let arm_set = |n: &str, side: &str| -> bool {
        let s = side;
        n.starts_with(s) && !n.contains("UpperLeg") && !n.contains("LowerLeg") && !n.contains("Foot") && !n.contains("Toes") && !n.contains("Eye")
            || n.starts_with(&format!("tw_{s}")) && !n.contains("Leg")
    };
    let mut checked_arm = 0;
    let mut checked_trunk = 0;
    for v in 0..topo.n_verts {
        let p = c.positions[v];
        let ws = bone_weights(topo, sk, v);
        // antebraço, mão e dedos: só ossos do próprio braço
        if p.x.abs() > elbow.x.abs() + 0.02 {
            let side = if p.x > 0.0 { "left" } else { "right" };
            for (n, w) in &ws {
                assert!(arm_set(n, side), "vértice {v} do braço {side} (x={}) pesa em {n} ({w})", p.x);
            }
            checked_arm += 1;
        }
        // tronco longe do ombro: nada de braço, perna ou cabeça
        if p.x.abs() < 0.07 && p.y < armpit_y && p.y > sk.pos(sk.index("hips").unwrap()).y + 0.02 {
            for (n, _) in &ws {
                assert!(
                    matches!(n.as_str(), "hips" | "spine" | "chest" | "upperChest"),
                    "vértice de tronco {v} (y={}) pesa em {n}",
                    p.y
                );
            }
            checked_trunk += 1;
        }
        // pernas: esquerda só ossos esquerdos (e hips perto da virilha)
        if p.y < sk.pos(sk.index("leftLowerLeg").unwrap()).y + 0.1 {
            let side = if p.x > 0.0 { "left" } else { "right" };
            for (n, _) in &ws {
                assert!(
                    n.contains(side) && (n.contains("Leg") || n.contains("Foot") || n.contains("Toes")),
                    "vértice de perna/pé {v} pesa em {n}"
                );
            }
        }
        // cabeça: só head/jaw/neck
        if p.y > sk.pos(sk.index("head").unwrap()).y + 0.03 {
            for (n, _) in &ws {
                assert!(matches!(n.as_str(), "head" | "jaw" | "neck"), "vértice de cabeça {v} pesa em {n}");
            }
        }
    }
    assert!(checked_arm > 500 && checked_trunk > 60, "{checked_arm} {checked_trunk}");
}

#[test]
fn weights_are_symmetric_and_smooth_at_joints() {
    let topo = reference();
    let c = generate(Gender::Female);
    let sk = &c.skeleton;
    let name_of = |b: u16| sk.bones[b as usize].name.clone();
    let mirror_name = |n: &str| -> String {
        if let Some(r) = n.strip_prefix("left") {
            format!("right{r}")
        } else if let Some(r) = n.strip_prefix("right") {
            format!("left{r}")
        } else if let Some(r) = n.strip_prefix("tw_left") {
            format!("tw_right{r}")
        } else if let Some(r) = n.strip_prefix("tw_right") {
            format!("tw_left{r}")
        } else if n == "bust_L" {
            "bust_R".into()
        } else if n == "bust_R" {
            "bust_L".into()
        } else {
            n.to_string()
        }
    };
    // simetria: o espelho de cada vértice pesa nos ossos espelhados
    for v in 0..topo.n_verts {
        let m = topo.mirror[v] as usize;
        let a: Vec<(String, f32)> = {
            let mut x: Vec<(String, f32)> = topo.weights[v].iter().filter(|w| w.1 > 1e-4).map(|w| (mirror_name(&name_of(w.0)), w.1)).collect();
            x.sort_by(|p, q| p.0.cmp(&q.0));
            x
        };
        let mut b: Vec<(String, f32)> = topo.weights[m].iter().filter(|w| w.1 > 1e-4).map(|w| (name_of(w.0), w.1)).collect();
        b.sort_by(|p, q| p.0.cmp(&q.0));
        // a poda de 4 influências pode trocar empates minúsculos; compare só os pesos > 0.02
        let big = |x: &Vec<(String, f32)>| -> Vec<(String, f32)> { x.iter().filter(|p| p.1 > 0.02).cloned().collect() };
        let (ba, bb) = (big(&a), big(&b));
        assert_eq!(ba.len(), bb.len(), "vértice {v}");
        for (p, q) in ba.iter().zip(&bb) {
            assert_eq!(p.0, q.0, "vértice {v}");
            assert!((p.1 - q.1).abs() < 1e-3, "vértice {v}");
        }
    }
    // suavidade: pesos de vértices vizinhos nunca saltam mais que 0.6 em nenhum osso
    let mut worst = 0.0f32;
    for v in 0..topo.n_verts {
        for &u in &topo.adjacency[v] {
            let mut all: BTreeSet<u16> = BTreeSet::new();
            for w in topo.weights[v].iter().chain(topo.weights[u as usize].iter()) {
                if w.1 > 0.0 {
                    all.insert(w.0);
                }
            }
            for b in all {
                let wv: f32 = topo.weights[v].iter().filter(|x| x.0 == b).map(|x| x.1).sum();
                let wu: f32 = topo.weights[u as usize].iter().filter(|x| x.0 == b).map(|x| x.1).sum();
                worst = worst.max((wv - wu).abs());
            }
        }
    }
    assert!(worst < 0.62, "salto de peso entre vizinhos: {worst}");
}

#[test]
fn fingers_are_weighted_by_phalanx() {
    let topo = reference();
    let c = generate(Gender::Female);
    let sk = &c.skeleton;
    // o vértice do anel PIP-ponta do indicador esquerdo domina a falange distal; o do anel 1 a proximal
    let dominant = |ids: &[u32]| -> String {
        let mut acc: std::collections::BTreeMap<String, f32> = Default::default();
        for &v in ids {
            for (n, w) in bone_weights(topo, sk, v as usize) {
                *acc.entry(n).or_insert(0.0) += w;
            }
        }
        acc.into_iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).unwrap().0
    };
    for s in ["L", "R"] {
        let side = if s == "L" { "left" } else { "right" };
        for (f, name) in [("index", "Index"), ("middle", "Middle"), ("ring", "Ring"), ("little", "Little")] {
            let r1 = topo.mark(&format!("finger_{s}_{f}_ring1")).to_vec();
            let r3 = topo.mark(&format!("finger_{s}_{f}_ring3")).to_vec();
            let r6 = topo.mark(&format!("finger_{s}_{f}_ring6")).to_vec();
            assert_eq!(dominant(&r1), format!("{side}{name}Proximal"));
            assert_eq!(dominant(&r3), format!("{side}{name}Intermediate"));
            assert_eq!(dominant(&r6), format!("{side}{name}Distal"));
        }
    }
}

#[test]
fn quad_mesh_helpers_are_consistent() {
    let m = QuadMesh {
        pos: vec![Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y],
        faces: vec![[0, 1, 2, 3]],
    };
    assert_eq!(m.euler(), 1);
    assert_eq!(m.boundary_loops().len(), 1);
    let mut p = m.pos.clone();
    let mirror = vec![1u32, 0, 3, 2];
    symmetrize(&mut p, &mirror);
    assert_eq!(p[0].x, -p[1].x);
}
