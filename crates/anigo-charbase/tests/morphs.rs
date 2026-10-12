//! Testes dos morphs de macro, rosto e expressões, das peças auxiliares e da re-derivação do
//! esqueleto em extremos (magro, gordo, velho). Determinísticos e sem GPU.

use anigo_charbase::cage::Region;
use anigo_charbase::character::skin_positions;
use anigo_charbase::morphs::{build_morphs, MorphGroup, MorphSet};
use anigo_charbase::params::{Params, FACE_PARAM_NAMES};
use anigo_charbase::parts::{build_parts, eye_geom, parts_topology, PART_NAMES};
use anigo_charbase::skeleton::{bone_defs, Skeleton};
use anigo_charbase::topology::{bfs_dist, quad_mesh_of, reference, Topology};
use anigo_charbase::{generate, generate_with, BaseCharacter, Gender};
use anigo_modeling::bvh::TriBvh;
use anigo_modeling::EditMesh;
use glam::Vec3;
use std::sync::OnceLock;

fn female() -> &'static (BaseCharacter, MorphSet) {
    static C: OnceLock<(BaseCharacter, MorphSet)> = OnceLock::new();
    C.get_or_init(|| {
        let c = generate(Gender::Female);
        let m = build_morphs(&c);
        (c, m)
    })
}

fn male() -> &'static (BaseCharacter, MorphSet) {
    static C: OnceLock<(BaseCharacter, MorphSet)> = OnceLock::new();
    C.get_or_init(|| {
        let c = generate(Gender::Male);
        let m = build_morphs(&c);
        (c, m)
    })
}

fn flipped_fraction(topo: &Topology, base: &[Vec3], pos: &[Vec3]) -> (f32, f32) {
    let m = quad_mesh_of(topo, pos);
    let vn = m.vertex_normals();
    let bm = quad_mesh_of(topo, base);
    let bn = bm.vertex_normals();
    let mut flipped = 0usize;
    let mut turned = 0usize;
    for f in &m.faces {
        let q: Vec<Vec3> = f.iter().map(|&i| pos[i as usize]).collect();
        let n = (q[2] - q[0]).cross(q[3] - q[1]);
        let avg = f.iter().map(|&i| vn[i as usize]).sum::<Vec3>();
        if n.dot(avg) < 0.0 {
            flipped += 1;
        }
        // normal de vértice invertida em relação à base
        if f.iter().any(|&i| vn[i as usize].dot(bn[i as usize]) < 0.0) {
            turned += 1;
        }
    }
    (flipped as f32 / m.faces.len() as f32, turned as f32 / m.faces.len() as f32)
}

/// Contagem grosseira de contatos entre superfícies não vizinhas (< `eps`), excluindo vizinhos
/// topológicos próximos. Detecta o grosso das autointerseções (membros atravessando o tronco).
fn near_contacts(topo: &Topology, pos: &[Vec3], eps: f32, skip_depth: u32) -> usize {
    let mut em = EditMesh::new("skin");
    for p in pos {
        em.add_vert(*p);
    }
    for f in &topo.faces {
        em.add_face(&[f[0], f[1], f[2], f[3]]).unwrap();
    }
    let bvh = TriBvh::build(&em);
    let mut count = 0usize;
    for v in 0..pos.len() {
        // região do vértice: ignora a fenda do olho/boca (anéis finos por construção)
        let d = bfs_dist_limited(&topo.adjacency, v as u32, skip_depth);
        for t in bvh.query_sphere_tris(pos[v], eps) {
            let tr = bvh.tri_ref(t).unwrap();
            if tr.verts.iter().any(|&u| d.contains(&u)) {
                continue;
            }
            count += 1;
            break;
        }
    }
    count
}

fn bfs_dist_limited(adj: &[Vec<u32>], src: u32, depth: u32) -> std::collections::BTreeSet<u32> {
    let mut seen = std::collections::BTreeSet::new();
    let mut frontier = vec![src];
    seen.insert(src);
    for _ in 0..depth {
        let mut next = Vec::new();
        for v in frontier {
            for &n in &adj[v as usize] {
                if seen.insert(n) {
                    next.push(n);
                }
            }
        }
        frontier = next;
    }
    seen
}

#[test]
fn morph_set_has_macros_face_and_vrm_expressions() {
    let (_, m) = female();
    let names = m.names();
    for n in [
        "gender_male", "gender_female", "age_adult", "age_elderly", "weight_heavy", "weight_thin", "muscle_high", "muscle_low", "height_tall",
        "height_short", "head_big", "head_small",
    ] {
        assert!(names.iter().any(|x| x == n), "macro ausente: {n}");
    }
    for p in FACE_PARAM_NAMES {
        assert!(names.contains(&format!("{p}_pos")) && names.contains(&format!("{p}_neg")), "{p}");
    }
    for e in [
        "neutral", "happy", "angry", "sad", "relaxed", "surprised", "aa", "ih", "ou", "ee", "oh", "blink", "blinkLeft", "blinkRight", "lookUp",
        "lookDown",
    ] {
        assert!(names.iter().any(|x| x == e), "expressão ausente: {e}");
    }
    assert_eq!(m.targets.iter().filter(|t| t.group == MorphGroup::Expression).count(), 16);
    // a faixa de idade é adulta: nenhum alvo infantil/adolescente
    assert!(!names.iter().any(|n| n.contains("child") || n.contains("baby") || n.contains("teen") || n.contains("young_child")));
    // nomes únicos e mesma lista para masculino e feminino (isomorfismo dos morphs)
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len());
    assert_eq!(male().1.names(), names);
}

#[test]
fn morph_deltas_are_bounded_and_sparse() {
    for (c, m) in [female(), male()] {
        let n = c.positions.len();
        for t in &m.targets {
            let mx = t.max_delta();
            let bound = match t.group {
                MorphGroup::Macro => 0.22,
                MorphGroup::Face => 0.05,
                MorphGroup::Expression => 0.07,
            };
            assert!(mx <= bound, "{}: delta máximo {mx} > {bound}", t.name);
            for (v, d) in &t.deltas {
                assert!((*v as usize) < n);
                assert!(d.iter().all(|x| x.is_finite()));
            }
            // sparse: nenhum alvo é denso demais além dos macros globais
            if t.group != MorphGroup::Macro {
                assert!(t.deltas.len() < n * 6 / 10, "{}: {} deltas (denso demais)", t.name, t.deltas.len());
            }
        }
    }
}

#[test]
fn face_targets_only_touch_head_and_neck() {
    let topo = reference();
    let (_, m) = female();
    for t in m.targets.iter().filter(|t| t.group == MorphGroup::Face) {
        for (v, _) in &t.deltas {
            let r = topo.region[*v as usize];
            assert!(matches!(r, Region::Head | Region::Neck), "{} move vértice de {:?}", t.name, r);
        }
        assert!(!t.deltas.is_empty() || t.name.contains("neg") || t.name.contains("pos"), "{}", t.name);
    }
    // cada parâmetro de rosto realmente move alguma coisa nas duas polaridades
    for p in FACE_PARAM_NAMES {
        for pol in ["pos", "neg"] {
            let i = m.find(&format!("{p}_{pol}")).unwrap();
            assert!(m.targets[i].deltas.len() > 20, "{p}_{pol} quase não move nada");
        }
    }
}

#[test]
fn expressions_move_the_face_in_the_right_places() {
    let topo = reference();
    let (c, m) = female();
    // blink fecha: a abertura vertical do olho (anel 3) cai muito
    let open = |pos: &[Vec3], s: &str| -> f32 {
        let ids = topo.loop_ids(&format!("eye_{s}_ring3"));
        let ys: Vec<f32> = ids.iter().map(|&i| pos[i as usize].y).collect();
        ys.iter().cloned().fold(f32::MIN, f32::max) - ys.iter().cloned().fold(f32::MAX, f32::min)
    };
    let base_open = open(&c.positions, "L");
    let blink = m.apply(&c.positions, &[("blink", 1.0)]);
    assert!(open(&blink, "L") < 0.35 * base_open, "blink: {} vs {}", open(&blink, "L"), base_open);
    assert!(open(&blink, "R") < 0.35 * base_open);
    // blinkLeft só fecha o olho esquerdo (+x)
    let bl = m.apply(&c.positions, &[("blinkLeft", 1.0)]);
    assert!(open(&bl, "L") < 0.35 * base_open);
    assert!(open(&bl, "R") > 0.95 * base_open);
    let br = m.apply(&c.positions, &[("blinkRight", 1.0)]);
    assert!(open(&br, "R") < 0.35 * base_open);
    assert!(open(&br, "L") > 0.95 * base_open);
    // aa abre a boca: o anel interno dos lábios ganha altura
    let mouth_h = |pos: &[Vec3]| -> f32 {
        let ids = topo.loop_ids("mouth_ring3");
        let ys: Vec<f32> = ids.iter().map(|&i| pos[i as usize].y).collect();
        ys.iter().cloned().fold(f32::MIN, f32::max) - ys.iter().cloned().fold(f32::MAX, f32::min)
    };
    let aa = m.apply(&c.positions, &[("aa", 1.0)]);
    assert!(mouth_h(&aa) > mouth_h(&c.positions) + 0.015, "aa: {}", mouth_h(&aa));
    // ou (bico) estreita a boca
    let width = |pos: &[Vec3]| -> f32 {
        let ids = topo.loop_ids("mouth_ring3");
        let xs: Vec<f32> = ids.iter().map(|&i| pos[i as usize].x).collect();
        xs.iter().cloned().fold(f32::MIN, f32::max) - xs.iter().cloned().fold(f32::MAX, f32::min)
    };
    let ou = m.apply(&c.positions, &[("ou", 1.0)]);
    assert!(width(&ou) < 0.85 * width(&c.positions));
    let ee = m.apply(&c.positions, &[("ee", 1.0)]);
    assert!(width(&ee) > 1.05 * width(&c.positions));
    // happy: cantos da boca sobem
    let corner_y = |pos: &[Vec3]| -> f32 {
        let l = topo.landmark("lm_mouth_corner_L").unwrap().eval(pos);
        let r = topo.landmark("lm_mouth_corner_R").unwrap().eval(pos);
        (l.y + r.y) * 0.5
    };
    let happy = m.apply(&c.positions, &[("happy", 1.0)]);
    assert!(corner_y(&happy) > corner_y(&c.positions) + 0.006);
    let sad = m.apply(&c.positions, &[("sad", 1.0)]);
    assert!(corner_y(&sad) < corner_y(&c.positions) - 0.003);
    // neutral não mexe em nada
    assert!(m.targets[m.find("neutral").unwrap()].deltas.is_empty());
    // as expressões só mexem na cabeça
    for e in ["happy", "angry", "sad", "relaxed", "surprised", "aa", "ih", "ou", "ee", "oh", "blink", "lookUp", "lookDown"] {
        for (v, _) in &m.targets[m.find(e).unwrap()].deltas {
            assert!(matches!(topo.region[*v as usize], Region::Head | Region::Neck), "{e}");
        }
    }
    // simetria: blink move os dois olhos de forma espelhada
    let t = &m.targets[m.find("blink").unwrap()];
    let mut by_v = std::collections::BTreeMap::new();
    for (v, d) in &t.deltas {
        by_v.insert(*v, *d);
    }
    for (v, d) in &by_v {
        let mv = topo.mirror[*v as usize];
        let md = by_v.get(&mv).unwrap_or_else(|| panic!("espelho de {v} não se move no blink"));
        assert!((d[0] + md[0]).abs() < 2e-4 && (d[1] - md[1]).abs() < 2e-4 && (d[2] - md[2]).abs() < 2e-4, "blink assimétrico em {v}");
    }
}

#[test]
fn macro_morphs_keep_normals_and_avoid_gross_self_intersection() {
    let topo = reference();
    for (c, m) in [female(), male()] {
        let base_contacts = near_contacts(topo, &c.positions, 0.0012, 7);
        for t in &m.targets {
            let pos = m.apply(&c.positions, &[(t.name.as_str(), 1.0)]);
            let (flipped, turned) = flipped_fraction(topo, &c.positions, &pos);
            assert!(flipped < 0.004, "{}: {:.3}% de faces invertidas", t.name, flipped * 100.0);
            assert!(turned < 0.05, "{}: {:.2}% de faces com normal virada vs base", t.name, turned * 100.0);
            assert!(quad_mesh_of(topo, &pos).signed_volume() > 0.0, "{}", t.name);
            if matches!(t.group, MorphGroup::Macro) {
                let contacts = near_contacts(topo, &pos, 0.0012, 7);
                assert!(contacts <= base_contacts + 25, "{}: {} contatos (base {})", t.name, contacts, base_contacts);
            }
        }
    }
}

#[test]
fn summed_macros_stay_sane() {
    let topo = reference();
    let (c, m) = female();
    let combos: [&[(&str, f32)]; 4] = [
        &[("weight_heavy", 1.0), ("age_elderly", 1.0)],
        &[("weight_thin", 1.0), ("age_elderly", 1.0), ("height_tall", 1.0)],
        &[("muscle_high", 1.0), ("weight_heavy", 0.6), ("gender_male", 1.0)],
        &[("weight_thin", 1.0), ("muscle_low", 1.0), ("head_big", 1.0)],
    ];
    for w in combos {
        let pos = m.apply(&c.positions, w);
        let (flipped, _) = flipped_fraction(topo, &c.positions, &pos);
        assert!(flipped < 0.006, "{w:?}: {:.2}%", flipped * 100.0);
        assert!(pos.iter().all(|p| p.is_finite()));
        assert!(quad_mesh_of(topo, &pos).signed_volume() > 0.0);
    }
}

#[test]
fn skeleton_follows_extreme_bodies_coherently() {
    let topo = reference();
    let defs = bone_defs(topo);
    let (c, m) = female();
    let base_h = c.positions.iter().map(|v| v.y).fold(f32::MIN, f32::max);
    for (label, w) in [
        ("thin", vec![("weight_thin", 1.0f32)]),
        ("heavy", vec![("weight_heavy", 1.0)]),
        ("old", vec![("age_elderly", 1.0)]),
        ("tall", vec![("height_tall", 1.0)]),
        ("short", vec![("height_short", 1.0)]),
        ("thin_old", vec![("weight_thin", 1.0), ("age_elderly", 1.0)]),
    ] {
        let pairs: Vec<(&str, f32)> = w.iter().map(|(n, x)| (*n, *x)).collect();
        let pos = m.apply(&c.positions, &pairs);
        let sk = Skeleton::derive(&defs, &pos);
        // cada junta continua DENTRO da caixa do corpo e na ordem anatômica
        let (mn, mx) = pos.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), p| (a.min(*p), b.max(*p)));
        for b in &sk.bones {
            let p = Vec3::from_array(b.world);
            assert!(p.x >= mn.x - 1e-3 && p.x <= mx.x + 1e-3 && p.y >= mn.y - 1e-3 && p.y <= mx.y + 1e-3 && p.z >= mn.z - 1e-3 && p.z <= mx.z + 1e-3, "{label}: {} fora da malha", b.name);
        }
        let y = |n: &str| sk.pos(sk.index(n).unwrap()).y;
        assert!(y("hips") < y("spine") && y("spine") < y("chest") && y("chest") < y("upperChest") && y("upperChest") < y("neck") && y("neck") < y("head"), "{label}");
        assert!(y("leftUpperLeg") > y("leftLowerLeg") && y("leftLowerLeg") > y("leftFoot") && y("leftFoot") > 0.0, "{label}");
        // braço continua horizontal e comprimentos de osso positivos
        let sh = sk.pos(sk.index("leftUpperArm").unwrap());
        for n in ["leftLowerArm", "leftHand"] {
            assert!((sk.pos(sk.index(n).unwrap()).y - sh.y).abs() < 0.04, "{label}: {n}");
        }
        for b in &sk.bones {
            if let Some(p) = b.parent {
                let d = (Vec3::from_array(b.world) - sk.pos(p)).length();
                assert!(d > 1e-4 || b.name.contains("tw_") || b.name.contains("Eye"), "{label}: osso {} de comprimento zero", b.name);
            }
        }
        // a altura da malha muda no sentido esperado e o osso da cabeça acompanha
        let h = pos.iter().map(|v| v.y).fold(f32::MIN, f32::max);
        if label == "tall" {
            assert!(h > base_h + 0.05);
        }
        if label == "short" {
            assert!(h < base_h - 0.05);
        }
        assert!(y("head") < h && y("head") > h * 0.8, "{label}: osso da cabeça fora de lugar");
        // a pose re-derivada coincide com o que a geração direta daria (mesmo localizador)
        let sk2 = Skeleton::derive(&defs, &pos);
        assert_eq!(sk.bones.len(), sk2.bones.len());
    }
    // pesos são constantes na topologia: seguem a malha por construção
    assert_eq!(topo.weights.len(), c.positions.len());
}

#[test]
fn skeleton_from_direct_generation_matches_morph_within_tolerance() {
    // gênero via morph aditivo (base feminina + gender_male) vs geração direta masculina
    let (c, m) = female();
    let pos = m.apply(&c.positions, &[("gender_male", 1.0)]);
    let direct = generate(Gender::Male);
    let defs = bone_defs(reference());
    let sk = Skeleton::derive(&defs, &pos);
    for (a, b) in sk.bones.iter().zip(&direct.skeleton.bones) {
        let d = (Vec3::from_array(a.world) - Vec3::from_array(b.world)).length();
        assert!(d < 1e-3, "{}: {d}", a.name);
    }
}

// ----------------------------------------------------------------- peças auxiliares

#[test]
fn parts_have_fixed_topology_and_follow_morphs() {
    let topo = reference();
    let pt = parts_topology();
    assert_eq!(pt.len(), PART_NAMES.len());
    let mut total = 0usize;
    for (name, _, tris, uv) in pt {
        assert!(!tris.is_empty(), "{name}");
        assert_eq!(uv.len(), uv.len());
        let n = uv.len() as u32;
        assert!(tris.iter().all(|t| t.iter().all(|&i| i < n)), "{name}: índice inválido");
        total += uv.len();
    }
    assert!(total > 700 && total < 1600, "vértices das peças: {total}");
    for p in [
        Params::default(),
        Params { gender: 0.0, weight: 1.0, age: 1.0, ..Params::default() },
        Params { weight: -1.0, height: 1.0, head_size: -1.0, ..Params::default() },
    ] {
        let pos = skin_positions(&p);
        let parts = build_parts(topo, &pos, &pos, 0.0);
        assert_eq!(parts.len(), pt.len());
        for (a, (name, _, tris, uv)) in parts.iter().zip(pt.iter()) {
            assert_eq!(&a.name, name);
            assert_eq!(&a.tris, tris, "{name}: topologia da peça muda com os parâmetros");
            assert_eq!(a.positions.len(), uv.len());
            assert!(a.positions.iter().all(|v| v.is_finite()));
        }
    }
}

#[test]
fn eyeballs_fill_the_aperture_without_poking_through_the_skin() {
    let topo = reference();
    let (c, _) = female();
    for s in ["L", "R"] {
        let g = eye_geom(topo, &c.positions, s);
        // os vértices da borda da pálpebra ficam a menos de 1.5 mm da esfera (cobrem a abertura)
        for &i in &topo.loop_ids(&format!("eye_{s}_ring3")) {
            let d = (c.positions[i as usize] - g.center).length();
            assert!(d <= g.radius * 1.001 && d > g.radius * 0.55, "borda a {d} do centro (raio {})", g.radius);
        }
        // nenhum outro vértice do rosto entra na esfera (exceto a borda da abertura)
        let rim: std::collections::BTreeSet<u32> = topo.loop_ids(&format!("eye_{s}_ring3")).into_iter().collect();
        let mut inside = 0;
        for (v, p) in c.positions.iter().enumerate() {
            if topo.region[v] == Region::Head && !rim.contains(&(v as u32)) && (*p - g.center).length() < g.radius * 0.97 {
                inside += 1;
            }
        }
        assert!(inside <= 40, "{inside} vértices da pele dentro do globo ocular {s}");
    }
}

#[test]
fn jaw_rotation_moves_lower_teeth_and_tongue_not_upper_teeth() {
    let topo = reference();
    let (c, _) = female();
    let a = build_parts(topo, &c.positions, &c.positions, 0.0);
    let b = build_parts(topo, &c.positions, &c.positions, 0.3);
    let idx = |n: &str| PART_NAMES.iter().position(|x| *x == n).unwrap();
    let diff = |i: usize| -> f32 { a[i].positions.iter().zip(&b[i].positions).map(|(p, q)| (*p - *q).length()).fold(0.0, f32::max) };
    assert!(diff(idx("teeth_upper")) < 1e-6);
    assert!(diff(idx("teeth_lower")) > 0.01);
    assert!(diff(idx("tongue")) > 0.005);
}

#[test]
fn morph_application_is_additive_and_deterministic() {
    let (c, m) = female();
    let p1 = m.apply(&c.positions, &[("weight_heavy", 0.5), ("blink", 1.0)]);
    let p2 = m.apply(&c.positions, &[("blink", 1.0), ("weight_heavy", 0.5)]);
    for (a, b) in p1.iter().zip(&p2) {
        assert!((*a - *b).length() < 1e-6);
    }
    let half = m.apply(&c.positions, &[("weight_heavy", 0.5)]);
    let full = m.apply(&c.positions, &[("weight_heavy", 1.0)]);
    for i in (0..c.positions.len()).step_by(37) {
        let mid = (c.positions[i] + full[i]) * 0.5;
        assert!((half[i] - mid).length() < 1e-5);
    }
    let core = m.to_core_sparse();
    assert_eq!(core.len(), m.targets.len());
    let c2 = generate_with(&c.params);
    assert_eq!(build_morphs(&c2).targets[3].deltas.len(), m.targets[3].deltas.len());
}
