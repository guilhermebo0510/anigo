//! Testes do modelo de pose: álgebra, limites, rig ajustado à malha, paleta de skinning em poses extremas,
//! comandos com inverso, ativo `.anigopose.json` e presets.

use super::*;
use crate::command::{Command, CommandHistory};
use crate::deformation::{canonical_base_mesh, interpolate_gender, prepare_base_mesh};
use crate::mesh::BaseGender;
use crate::project::ProjectState;

fn q(axis: Vec3, deg: f32) -> Quat {
    Quat::from_axis_angle(axis, deg.to_radians())
}

fn default_rig_and_mesh() -> (PoseRig, Mesh, Mesh) {
    let project = ProjectState::default();
    let male = canonical_base_mesh(BaseGender::Male);
    let female = canonical_base_mesh(BaseGender::Female);
    let reference = interpolate_gender(&male, &female, project.character.gender_dimorphism).unwrap();
    let deformed = prepare_base_mesh(&project).unwrap();
    let rig = PoseRig::fit(&reference, &deformed, project.character.gender_dimorphism);
    (rig, reference, deformed)
}

/// Aplica a paleta à malha por LBS (a mesma conta do vertex shader).
fn skin_mesh(mesh: &Mesh, palette: &[Mat4; BONE_COUNT]) -> Vec<Vec3> {
    mesh.vertices
        .iter()
        .map(|v| {
            let p = Vec3::from_array(v.position);
            let mut acc = Vec3::ZERO;
            let mut wsum = 0.0;
            for k in 0..4 {
                let w = v.weights[k];
                if w > 0.0 {
                    acc += palette[v.joints[k] as usize].transform_point3(p) * w;
                    wsum += w;
                }
            }
            if wsum > 0.0 {
                acc / wsum
            } else {
                p
            }
        })
        .collect()
}

#[test]
fn bone_tables_are_consistent() {
    assert_eq!(BONE_NAMES.len(), BONE_COUNT);
    for (i, name) in BONE_NAMES.iter().enumerate() {
        assert_eq!(bone_index(name), Some(i));
        // o pai vem antes do filho (a paleta é calculada em ordem)
        if let Some(p) = BONE_PARENTS[i] {
            assert!(p < i, "{name}: pai {p} depois do filho");
        }
        // espelho é involução e preserva o tipo
        assert_eq!(mirror_bone(mirror_bone(i)), i);
    }
    // a mesma ordem do BondSyncManager (fonte do esqueleto canônico)
    let mgr = crate::bone_sync::BondSyncManager::create_canonical_humanoid();
    for (i, joint) in mgr.joints.iter().enumerate() {
        assert_eq!(joint.name, BONE_NAMES[i]);
        assert_eq!(joint.parent_index, BONE_PARENTS[i]);
    }
    assert_eq!(Limb::LeftArm.chain(), [bone::L_SHOULDER, bone::L_ELBOW, bone::L_WRIST]);
    assert_eq!(Limb::of_bone(bone::R_KNEE), Some(Limb::RightLeg));
    assert_eq!(bone_subtree(bone::L_SHOULDER), vec![9, 10, 11]);
}

#[test]
fn rest_pose_gives_exact_identity_palette() {
    let rig = PoseRig::default();
    let pose = PoseState::default();
    assert!(pose.is_rest());
    let floats = rig.palette_floats(&pose);
    assert_eq!(floats.len(), 384);
    assert!(crate::skinning::flat_palette_is_identity(&floats));
    // exatamente a identidade, bit a bit (o frame dourado não pode mudar)
    let ident = crate::skinning::identity_palette_floats(BONE_COUNT);
    assert_eq!(floats, ident);
    // poses que só TÊM identidade explícita também são repouso
    let mut p = PoseState::default();
    p.set_rotation(bone::HEAD, Quat::IDENTITY);
    assert!(p.is_rest());
}

#[test]
fn pose_state_serialization_is_additive_and_round_trips() {
    // em repouso o JSON do personagem NÃO ganha campo algum (fixtures e projetos antigos intactos)
    let project = ProjectState::default();
    let json = project.to_canonical_json().unwrap();
    assert!(!json.contains("\"pose\""), "pose em repouso não pode aparecer no JSON canônico");
    let reloaded = ProjectState::from_json(&json).unwrap();
    assert_eq!(project, reloaded);

    let mut posed = project.clone();
    posed.character.pose.set_rotation(bone::L_ELBOW, q(Vec3::X, -70.0));
    posed.character.pose.hips_translation = [0.0, -0.1, 0.02];
    posed.character.pose.hands.left = [0.5, 1.0, 1.0, 1.0, 1.0];
    let j = posed.to_canonical_json().unwrap();
    assert!(j.contains("LeftElbow"));
    assert!(!j.contains("root_translation"), "campos zerados são omitidos");
    let back = ProjectState::from_json(&j).unwrap();
    assert_eq!(posed, back);

    // um projeto antigo (sem o campo) carrega como repouso
    let old: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(old["character"].get("pose").is_none());
}

#[test]
fn project_validation_rejects_bad_poses() {
    let mut p = ProjectState::default();
    p.character.pose.rotations.insert("NotABone".into(), [0.0, 0.0, 0.0, 1.0]);
    assert!(p.validate().is_err());
    let mut p = ProjectState::default();
    p.character.pose.rotations.insert("Head".into(), [f32::NAN, 0.0, 0.0, 1.0]);
    assert!(p.validate().is_err());
    let mut p = ProjectState::default();
    p.character.pose.hips_translation = [0.0, 1.0e6, 0.0];
    assert!(p.validate().is_err());
    // sanitize repara em vez de falhar
    let mut p = ProjectState::default();
    p.character.pose.rotations.insert("NotABone".into(), [0.0, 0.0, 0.0, 1.0]);
    p.character.pose.rotations.insert("Head".into(), [0.0, 2.0, 0.0, 0.0]);
    p.character.pose.hips_translation = [f32::NAN, 99.0, 0.0];
    let p = p.sanitize();
    p.validate().unwrap();
    assert!(!p.character.pose.rotations.contains_key("NotABone"));
    let head = p.character.pose.rotation(bone::HEAD);
    assert!((head.length() - 1.0).abs() < 1e-5);
}

#[test]
fn mirror_is_an_involution_and_swaps_sides() {
    let mut p = PoseState::default();
    p.set_rotation(bone::L_SHOULDER, q(Vec3::Z, 70.0) * q(Vec3::X, -30.0));
    p.set_rotation(bone::L_KNEE, q(Vec3::X, 90.0));
    p.set_rotation(bone::SPINE, q(Vec3::Y, 25.0) * q(Vec3::Z, 10.0));
    p.hips_translation = [0.05, -0.1, 0.02];
    p.hands.left = [0.1, 0.2, 0.3, 0.4, 0.5];
    let m = p.mirrored();
    assert_ne!(m, p);
    assert_eq!(m.mirrored().differing_bones(&p), BTreeSet::new());
    assert_eq!(m.mirrored().hips_translation, p.hips_translation);
    // lado trocado
    assert!(m.rotation(bone::L_SHOULDER).dot(Quat::IDENTITY).abs() > 0.999999);
    assert!(m.rotation(bone::R_KNEE).dot(q(Vec3::X, 90.0)).abs() > 0.99999);
    assert_eq!(m.hands.right, [0.1, 0.2, 0.3, 0.4, 0.5]);
    assert_eq!(m.hips_translation[0], -0.05);

    // geometricamente: as juntas espelhadas ficam em posições espelhadas em X
    let rig = PoseRig::default();
    let (a, b) = (rig.joint_positions(&p), rig.joint_positions(&m));
    for i in 0..BONE_COUNT {
        let j = mirror_bone(i);
        let (pa, pb) = (a[i], b[j]);
        assert!((pa.x + pb.x).abs() < 1e-4 && (pa.y - pb.y).abs() < 1e-4 && (pa.z - pb.z).abs() < 1e-4, "{}: {pa} vs {pb}", BONE_NAMES[i]);
    }
}

#[test]
fn blend_and_masks() {
    let mut a = PoseState::default();
    a.set_rotation(bone::L_ELBOW, q(Vec3::X, -100.0));
    let rest = PoseState::default();
    let half = PoseState::blend(&rest, &a, 0.5);
    let ang = half.rotation(bone::L_ELBOW).to_axis_angle().1.to_degrees();
    assert!((ang - 50.0).abs() < 0.01, "{ang}");
    // extrapola
    let over = PoseState::blend(&rest, &a, 1.5);
    assert!((over.rotation(bone::L_ELBOW).to_axis_angle().1.to_degrees() - 150.0).abs() < 0.01);
    assert!((over.rotation(bone::L_ELBOW).length() - 1.0).abs() < 1e-5);
    // blend 0 = origem; 1 = destino
    assert!(PoseState::blend(&rest, &a, 0.0).is_rest());
    assert!(PoseState::blend(&rest, &a, 1.0).differing_bones(&a).is_empty());
    // máscara
    let mut b = a.clone();
    b.set_rotation(bone::HEAD, q(Vec3::Y, 30.0));
    let only_head: BTreeSet<usize> = [bone::HEAD].into_iter().collect();
    let m = b.masked(&only_head);
    assert_eq!(m.posed_bones(), vec![bone::HEAD]);
    let o = a.overlay(&b, &only_head);
    assert!(o.rotation(bone::HEAD).dot(b.rotation(bone::HEAD)).abs() > 0.99999);
    assert!(o.rotation(bone::L_ELBOW).dot(a.rotation(bone::L_ELBOW)).abs() > 0.99999);
}

#[test]
fn limits_are_idempotent_respected_and_continuous() {
    let mut rng = 0x1234_5678_u64;
    let mut next = || {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((rng >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
    };
    for profile in [LimitProfile::Human, LimitProfile::Anime] {
        for bone_i in 0..BONE_COUNT {
            let Some(lim) = bone_limit(bone_i, profile) else { continue };
            for _ in 0..200 {
                let qr = Quat::from_euler(glam::EulerRot::XYZ, next() * 3.0, next() * 3.0, next() * 3.0).normalize();
                let c = clamp_to_limit(qr, &lim);
                assert!(c.is_finite() && (c.length() - 1.0).abs() < 1e-4);
                // idempotente
                let c2 = clamp_to_limit(c, &lim);
                assert!(c2.dot(c).abs() > 1.0 - 1e-5, "{} não idempotente", BONE_NAMES[bone_i]);
                // respeita: swing e twist dentro das faixas (com a tolerância do cone elíptico)
                let (tx, tz, tw) = swing_twist(c);
                let (txd, tzd, twd) = (tx.to_degrees(), tz.to_degrees(), tw.to_degrees());
                assert!(twd >= lim.twist[0] - 0.01 && twd <= lim.twist[1] + 0.01, "{} twist {twd}", BONE_NAMES[bone_i]);
                assert!(txd >= lim.sx[0] - 0.01 && txd <= lim.sx[1] + 0.01, "{} sx {txd}", BONE_NAMES[bone_i]);
                assert!(tzd >= lim.sz[0] - 0.01 && tzd <= lim.sz[1] + 0.01, "{} sz {tzd}", BONE_NAMES[bone_i]);
            }
            // contínuo: uma entrada ligeiramente diferente muda a saída pouco
            let base = Quat::from_euler(glam::EulerRot::XYZ, 0.9, -0.4, 0.7).normalize();
            let near = (base * Quat::from_axis_angle(Vec3::new(0.3, 0.9, 0.1).normalize(), 0.002)).normalize();
            let (a, b) = (clamp_to_limit(base, &lim), clamp_to_limit(near, &lim));
            assert!(a.dot(b).abs() > 0.9999, "{} descontínuo", BONE_NAMES[bone_i]);
        }
    }
    // identidade e rotações dentro do limite voltam intactas
    let lim = bone_limit(bone::L_ELBOW, LimitProfile::Human).unwrap();
    let inside = q(Vec3::X, -90.0);
    assert!(clamp_to_limit(inside, &lim).dot(inside).abs() > 0.999999);
    assert!(clamp_to_limit(Quat::IDENTITY, &lim).dot(Quat::IDENTITY).abs() > 0.999999);
    // hiperextensão do cotovelo é cortada; a flexão excessiva também
    let hyper = q(Vec3::X, 40.0);
    let c = clamp_to_limit(hyper, &lim);
    assert!(swing_twist(c).0.to_degrees() <= 3.01);
    let flex = q(Vec3::X, -170.0);
    assert!((swing_twist(clamp_to_limit(flex, &lim)).0.to_degrees() + 150.0).abs() < 0.01);
    // anime é mais folgado que humano; livre não limita
    let human = bone_limit(bone::L_KNEE, LimitProfile::Human).unwrap();
    let anime = bone_limit(bone::L_KNEE, LimitProfile::Anime).unwrap();
    assert!(anime.sx[1] > human.sx[1]);
    assert!(bone_limit(bone::L_KNEE, LimitProfile::Free).is_none());
    // o lado direito é o espelho do esquerdo
    let (l, r) = (bone_limit(bone::L_SHOULDER, LimitProfile::Human).unwrap(), bone_limit(bone::R_SHOULDER, LimitProfile::Human).unwrap());
    assert_eq!(l.sx, r.sx);
    assert_eq!([-l.sz[1], -l.sz[0]], r.sz);
    // euler <-> quat
    let e = [30.0, -20.0, 45.0];
    let back = quat_to_euler_deg(euler_deg_to_quat(e));
    for i in 0..3 {
        assert!((back[i] - e[i]).abs() < 1e-3);
    }
}

#[test]
fn clamp_pose_makes_every_bone_legal() {
    let mut p = PoseState::default();
    for b in 0..BONE_COUNT {
        p.set_rotation(b, Quat::from_euler(glam::EulerRot::XYZ, 2.0, 1.5, -2.2));
    }
    let c = clamp_pose(&p, LimitProfile::Human);
    for b in 0..BONE_COUNT {
        if let Some(l) = bone_limit(b, LimitProfile::Human) {
            assert!(within_limit(c.rotation(b), &l), "{}", BONE_NAMES[b]);
        }
    }
    // livre não muda nada
    assert_eq!(clamp_pose(&p, LimitProfile::Free), p);
}

#[test]
fn rig_fit_keeps_joints_inside_the_body_and_follows_proportions() {
    let (rig, reference, deformed) = default_rig_and_mesh();
    // sem proportions nem morphs, o ajuste é o canônico
    let canon = PoseRig::canonical(1.0);
    for i in 0..BONE_COUNT {
        assert!(rig.bind[i].distance(canon.bind[i]) < 1e-4, "{}: {} vs {}", BONE_NAMES[i], rig.bind[i], canon.bind[i]);
    }
    // cada junta fica perto da superfície da malha (dentro do corpo): vizinho mais próximo < 8 cm
    for i in 1..BONE_COUNT {
        let p = rig.bind[i];
        let nearest = deformed.vertices.iter().map(|v| Vec3::from_array(v.position).distance(p)).fold(f32::INFINITY, f32::min);
        let interior = matches!(i, bone::HIPS | bone::PELVIS | bone::SPINE | bone::CHEST | bone::UPPER_CHEST);
        let max = if interior { 0.14 } else { 0.09 };
        assert!(nearest < max, "{}: junta a {nearest} m da superfície", BONE_NAMES[i]);
    }
    // o osso esquerdo está em +X
    for (l, r) in [(bone::L_SHOULDER, bone::R_SHOULDER), (bone::L_KNEE, bone::R_KNEE), (bone::L_ANKLE, bone::R_ANKLE)] {
        assert!(rig.bind[l].x > 0.0 && rig.bind[r].x < 0.0);
        assert!((rig.bind[l].x + rig.bind[r].x).abs() < 1e-4);
    }
    // alongar as pernas e o braço desloca as juntas na mesma direção que a malha
    let mut project = ProjectState::default();
    project.character.proportions.leg_length = 1.3;
    project.character.proportions.arm_length = 1.25;
    project.character.proportions.height_overall = 1.1;
    let tall = prepare_base_mesh(&project).unwrap();
    let rig2 = PoseRig::fit(&reference, &tall, 1.0);
    assert!(rig2.bind[bone::L_ANKLE].y > rig.bind[bone::L_ANKLE].y - 0.2, "tornozelo acompanha o piso: {}", rig2.bind[bone::L_ANKLE].y);
    assert!(rig2.bind[bone::L_KNEE].y > rig.bind[bone::L_KNEE].y, "joelho sobe com a perna longa");
    assert!(rig2.bind[bone::HEAD].y > rig.bind[bone::HEAD].y, "cabeça sobe com a altura");
    assert!(rig2.bind[bone::L_WRIST].y < rig.bind[bone::L_WRIST].y + 0.05);
    for p in rig2.bind {
        assert!(p.is_finite());
    }
    // malhas incompatíveis voltam ao canônico sem pânico
    let tiny = Mesh::create_cube(1.0);
    assert_eq!(PoseRig::fit(&tiny, &deformed, 1.0).bind, canon.bind);
    // gênero interpola
    let mid = PoseRig::canonical(0.5);
    let (m, f) = (PoseRig::canonical(1.0), PoseRig::canonical(0.0));
    assert!(mid.bind[bone::HEAD].y < m.bind[bone::HEAD].y && mid.bind[bone::HEAD].y > f.bind[bone::HEAD].y);
}

#[test]
fn skeleton_is_rigid_in_any_pose_bone_lengths_preserved() {
    let (rig, _, _) = default_rig_and_mesh();
    let rest = rig.joint_positions(&PoseState::default());
    let mut rng = 99u64;
    let mut next = || {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((rng >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
    };
    for _ in 0..300 {
        let mut p = PoseState::default();
        for b in 0..BONE_COUNT {
            p.set_rotation(b, Quat::from_euler(glam::EulerRot::XYZ, next() * 3.1, next() * 3.1, next() * 3.1));
        }
        p.root_translation = [next(), next(), next()];
        p.hips_translation = [next() * 0.3, next() * 0.3, next() * 0.3];
        let pos = rig.joint_positions(&p);
        for i in 2..BONE_COUNT {
            // Root e Hips têm translação própria; os demais ossos só giram
            let parent = BONE_PARENTS[i].unwrap();
            let (l0, l1) = (rest[i].distance(rest[parent]), pos[i].distance(pos[parent]));
            assert!((l0 - l1).abs() < 2e-4, "{}: {l0} -> {l1}", BONE_NAMES[i]);
            assert!(pos[i].is_finite());
        }
        // cada matriz de skin é rígida: base ortonormal, det = +1, sem cisalhamento
        for m in rig.skin_palette(&p) {
            let (c0, c1, c2) = (m.x_axis.truncate(), m.y_axis.truncate(), m.z_axis.truncate());
            assert!((c0.length() - 1.0).abs() < 1e-4 && (c1.length() - 1.0).abs() < 1e-4 && (c2.length() - 1.0).abs() < 1e-4);
            assert!(c0.dot(c1).abs() < 1e-4 && c1.dot(c2).abs() < 1e-4 && c0.dot(c2).abs() < 1e-4);
            assert!((c0.dot(c1.cross(c2)) - 1.0).abs() < 1e-4);
            assert!(m.to_cols_array().iter().all(|v| v.is_finite()));
        }
    }
}

#[test]
fn hips_translation_moves_the_body_but_not_the_root() {
    let rig = PoseRig::default();
    let mut p = PoseState::default();
    p.hips_translation = [0.0, -0.2, 0.0];
    let (a, b) = (rig.joint_positions(&PoseState::default()), rig.joint_positions(&p));
    assert!((b[bone::HIPS].y - (a[bone::HIPS].y - 0.2)).abs() < 1e-5);
    assert!((b[bone::HEAD].y - (a[bone::HEAD].y - 0.2)).abs() < 1e-5, "a cabeça acompanha o quadril");
    assert!((b[bone::L_KNEE].y - (a[bone::L_KNEE].y - 0.2)).abs() < 1e-5, "sem IK as pernas vão junto");
    assert_eq!(b[bone::ROOT], a[bone::ROOT]);
    let mut p = PoseState::default();
    p.root_translation = [1.0, 0.0, 2.0];
    let b = rig.joint_positions(&p);
    assert!((b[bone::L_ANKLE] - rig.bind[bone::L_ANKLE] - Vec3::new(1.0, 0.0, 2.0)).length() < 1e-5);
}

#[test]
fn extreme_poses_skin_without_nan_or_explosions() {
    let (rig, _, deformed) = default_rig_and_mesh();
    let rest_bounds = bounds(&deformed.vertices.iter().map(|v| Vec3::from_array(v.position)).collect::<Vec<_>>());
    let extremes: Vec<PoseState> = vec![
        // braço sobre a cabeça, tronco torcido, cabeça virada
        {
            let mut p = PoseState::default();
            p.set_rotation(bone::L_SHOULDER, q(Vec3::Z, 175.0));
            p.set_rotation(bone::R_SHOULDER, q(Vec3::X, -170.0) * q(Vec3::Z, -20.0));
            p.set_rotation(bone::SPINE, q(Vec3::Y, 40.0) * q(Vec3::X, 25.0));
            p.set_rotation(bone::HEAD, q(Vec3::Y, 80.0));
            p
        },
        // agachamento profundo
        {
            let mut p = PoseState::default();
            for (t, k, a) in [(bone::L_THIGH, bone::L_KNEE, bone::L_ANKLE), (bone::R_THIGH, bone::R_KNEE, bone::R_ANKLE)] {
                p.set_rotation(t, q(Vec3::X, -120.0));
                p.set_rotation(k, q(Vec3::X, 145.0));
                p.set_rotation(a, q(Vec3::X, -20.0));
            }
            p.set_rotation(bone::SPINE, q(Vec3::X, 30.0));
            p
        },
        // ponte / dobrado para frente e para trás, rotações aleatórias grandes
        {
            let mut p = PoseState::default();
            p.set_rotation(bone::HIPS, q(Vec3::X, 100.0));
            p.set_rotation(bone::SPINE, q(Vec3::X, -40.0));
            p.set_rotation(bone::NECK, q(Vec3::X, -50.0));
            p
        },
        presets::body("action", &rig).unwrap(),
        presets::body("t", &rig).unwrap(),
    ];
    for (n, p) in extremes.iter().enumerate() {
        let pal = rig.skin_palette(p);
        let skinned = skin_mesh(&deformed, &pal);
        assert!(skinned.iter().all(|v| v.is_finite()), "pose {n}: NaN/Inf");
        // nenhum vértice vai parar longe do corpo (o corpo tem ~1,8 m): caixa limitada
        let b = bounds(&skinned);
        for axis in 0..3 {
            assert!(b.1[axis] - b.0[axis] < 4.0, "pose {n}: caixa explodiu {b:?}");
        }
        // soma de pesos = 1 (LBS convexo): o deslocamento máximo é limitado pelo maior deslocamento de osso
        let max_bone_disp = (0..BONE_COUNT)
            .map(|i| (pal[i].transform_point3(rig.bind[i]) - rig.bind[i]).length())
            .fold(0.0f32, f32::max);
        for (v, s) in deformed.vertices.iter().zip(&skinned) {
            let p0 = Vec3::from_array(v.position);
            let d = (*s - p0).length();
            let bound = (0..BONE_COUNT).map(|i| (pal[i].transform_point3(p0) - p0).length()).fold(0.0f32, f32::max);
            assert!(d <= bound + 1e-4, "pose {n}: deslocamento acima do envelope convexo");
            assert!(bound.is_finite() && max_bone_disp.is_finite());
        }
        // volume do corpo: a caixa de repouso é ~(0,56 x 1,8 x 0,3); nenhuma pose achata tudo num ponto
        let ext = b.1 - b.0;
        assert!(ext.length() > 0.8, "pose {n} colapsou a malha: {ext}");
        // rest bounds is kept for reference
        assert!(rest_bounds.1.y > 1.5);
    }
}

fn bounds(points: &[Vec3]) -> (Vec3, Vec3) {
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for p in points {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    (lo, hi)
}

#[test]
fn dominant_vertices_follow_their_bone_rigidly() {
    // um vértice com peso 1 num osso acompanha exatamente a matriz de skin desse osso
    let (rig, _, deformed) = default_rig_and_mesh();
    let mut p = PoseState::default();
    p.set_rotation(bone::L_ELBOW, q(Vec3::X, -90.0));
    let pal = rig.skin_palette(&p);
    let skinned = skin_mesh(&deformed, &pal);
    let mut checked = 0;
    for (v, s) in deformed.vertices.iter().zip(&skinned) {
        if v.weights[0] > 0.9999 && v.joints[0] as usize == bone::L_ELBOW {
            let expect = pal[bone::L_ELBOW].transform_point3(Vec3::from_array(v.position));
            assert!(expect.distance(*s) < 1e-4);
            checked += 1;
        }
    }
    assert!(checked > 20, "poucos vértices do antebraço: {checked}");
    // a mão (descendente) rodou em volta do cotovelo: a distância cotovelo→pulso é preservada
    let j = rig.joint_positions(&p);
    let rest = rig.joint_positions(&PoseState::default());
    assert!((j[bone::L_WRIST].distance(j[bone::L_ELBOW]) - rest[bone::L_WRIST].distance(rest[bone::L_ELBOW])).abs() < 1e-5);
    // antebraço sobe: o pulso passou a ficar na frente do cotovelo (+Z)
    assert!(j[bone::L_WRIST].z > j[bone::L_ELBOW].z + 0.2, "flexão de cotovelo leva a mão para a frente");
}

#[test]
fn center_of_mass_balance_and_hull() {
    let rig = PoseRig::default();
    let rest = PoseState::default();
    let (com, hull, inside) = balance(&rig, &rest, 0.02);
    assert!(inside, "de pé em repouso o CoM está dentro da base de apoio: {com} {hull:?}");
    assert!(com.y > 0.8 && com.y < 1.2, "CoM em altura de quadril/abdômen: {}", com.y);
    // inclinar muito o tronco para a frente sem mover os pés tira o CoM do polígono
    let mut lean = PoseState::default();
    lean.set_rotation(bone::HIPS, q(Vec3::X, 80.0));
    let (_, _, inside2) = balance(&rig, &lean, 0.02);
    assert!(!inside2, "inclinação de 80° deixa o CoM fora da base");
    // uma perna só: a base encolhe
    assert!(convex_hull_xz(&[Vec3::ZERO, Vec3::X, Vec3::Z, Vec3::new(1.0, 0.0, 1.0), Vec3::new(0.5, 0.0, 0.5)]).len() == 4);
    assert!(point_in_hull(&convex_hull_xz(&[Vec3::ZERO, Vec3::X, Vec3::Z, Vec3::new(1.0, 0.0, 1.0)]), [0.5, 0.5]));
    assert!(!point_in_hull(&convex_hull_xz(&[Vec3::ZERO, Vec3::X, Vec3::Z, Vec3::new(1.0, 0.0, 1.0)]), [1.5, 0.5]));
}

#[test]
fn grounding_puts_the_lowest_foot_point_on_the_floor() {
    let (rig, _, _) = default_rig_and_mesh();
    let rest_low = rig.lowest_contact_y(&PoseState::default());
    assert!(rest_low.abs() < 1e-5, "em repouso os pés tocam y=0 (sole_y): {rest_low}");
    let mut p = PoseState::default();
    p.set_rotation(bone::L_THIGH, q(Vec3::X, -80.0));
    p.set_rotation(bone::L_KNEE, q(Vec3::X, 90.0));
    p.hips_translation = [0.0, -0.1, 0.0];
    let g = rig.grounded(&p, 0.0);
    assert!(rig.lowest_contact_y(&g).abs() < 1e-4);
    // aterrar duas vezes não muda mais nada
    let g2 = rig.grounded(&g, 0.0);
    assert!((g2.root_translation[1] - g.root_translation[1]).abs() < 1e-5);
}

#[test]
fn set_pose_command_round_trips_and_is_one_undo_step() {
    let mut state = ProjectState::default();
    let mut history = CommandHistory::new(100);
    let before = state.clone();

    let mut target = PoseState::default();
    target.set_rotation(bone::L_ELBOW, q(Vec3::X, -80.0));
    target.set_rotation(bone::HEAD, q(Vec3::Y, 30.0));
    target.hips_translation = [0.0, -0.05, 0.0];
    let cmd = plan_pose_change(&state.character.pose, &target, PoseOp::Preset).unwrap();
    let outcome = history.execute(&mut state, cmd.clone()).unwrap();
    assert_eq!(outcome.description, "Pose preset");
    assert_eq!(outcome.undo_depth, 1);
    assert_eq!(state.character.pose, target);

    // re-aplicar o mesmo alvo não muda nada: não entra no histórico
    assert!(plan_pose_change(&state.character.pose, &target, PoseOp::Edit).is_none());
    assert!(history.execute(&mut state, cmd).is_err());
    assert_eq!(history.undo_depth(), 1);

    // editar um osso: descrição nomeia o osso
    let mut t2 = state.character.pose.clone();
    t2.set_rotation(bone::L_ELBOW, q(Vec3::X, -20.0));
    let c2 = plan_pose_change(&state.character.pose, &t2, PoseOp::Edit).unwrap();
    assert_eq!(history.execute(&mut state, c2).unwrap().description, "Pose LeftElbow");

    history.undo(&mut state).unwrap();
    assert_eq!(state.character.pose, target);
    history.undo(&mut state).unwrap();
    assert_eq!(state, before);
    history.redo(&mut state).unwrap();
    history.redo(&mut state).unwrap();
    assert_eq!(state.character.pose, t2);

    // JSON do comando é estável e volta igual
    let json = plan_pose_change(&PoseState::default(), &target, PoseOp::Mirror).unwrap().to_json().unwrap();
    assert!(json.contains("\"kind\":\"set_pose\"") && json.contains("\"op\":\"mirror\""), "{json}");
    assert_eq!(Command::from_json(&json).unwrap().to_json().unwrap(), json);
}

#[test]
fn set_pose_validation_rejects_bad_input_and_keeps_state_intact() {
    let mut state = ProjectState::default();
    let mut h = CommandHistory::new(10);
    let bad = [
        Command::SetPose { bones: vec![], root_translation: None, hips_translation: None, hands: None, op: PoseOp::Edit },
        Command::SetPose { bones: vec![PoseBoneEntry { bone: "Tail".into(), rotation: [0.0, 0.0, 0.0, 1.0] }], root_translation: None, hips_translation: None, hands: None, op: PoseOp::Edit },
        Command::SetPose { bones: vec![PoseBoneEntry { bone: "Head".into(), rotation: [f32::NAN, 0.0, 0.0, 1.0] }], root_translation: None, hips_translation: None, hands: None, op: PoseOp::Edit },
        Command::SetPose { bones: vec![PoseBoneEntry { bone: "Head".into(), rotation: [0.0, 0.0, 0.0, 0.0] }], root_translation: None, hips_translation: None, hands: None, op: PoseOp::Edit },
        Command::SetPose { bones: vec![], root_translation: Some([0.0, 1.0e9, 0.0]), hips_translation: None, hands: None, op: PoseOp::Edit },
        Command::SetPose { bones: vec![], root_translation: None, hips_translation: None, hands: Some(HandsPose { left: [2.0; 5], right: [0.0; 5] }), op: PoseOp::Hand },
        Command::SetPose {
            bones: vec![
                PoseBoneEntry { bone: "Head".into(), rotation: [0.0, 0.7071, 0.0, 0.7071] },
                PoseBoneEntry { bone: "Head".into(), rotation: [0.0, 0.0, 0.0, 1.0] },
            ],
            root_translation: None,
            hips_translation: None,
            hands: None,
            op: PoseOp::Edit,
        },
        // identidade sobre repouso = sem efeito
        Command::SetPose { bones: vec![PoseBoneEntry { bone: "Head".into(), rotation: [0.0, 0.0, 0.0, 1.0] }], root_translation: None, hips_translation: None, hands: None, op: PoseOp::Edit },
    ];
    for c in bad {
        let before = state.clone();
        assert!(h.execute(&mut state, c.clone()).is_err(), "{c:?} deveria falhar");
        assert_eq!(state, before);
    }
    assert_eq!(h.undo_depth(), 0);
    // quaternion não normalizado é aceito e normalizado
    let c = Command::SetPose { bones: vec![PoseBoneEntry { bone: "Head".into(), rotation: [0.0, 1.4142, 0.0, 1.4142] }], root_translation: None, hips_translation: None, hands: None, op: PoseOp::Edit };
    h.execute(&mut state, c).unwrap();
    let r = state.character.pose.rotation(bone::HEAD);
    assert!((r.length() - 1.0).abs() < 1e-5 && (r.to_axis_angle().1.to_degrees() - 90.0).abs() < 0.1);
    // aninhado em batch
    let mut t = state.character.pose.clone();
    t.set_rotation(bone::R_KNEE, q(Vec3::X, 60.0));
    let inner = plan_pose_change(&state.character.pose, &t, PoseOp::Edit).unwrap();
    let before = state.clone();
    h.execute(&mut state, Command::Batch { commands: vec![inner] }).unwrap();
    h.undo(&mut state).unwrap();
    assert_eq!(state, before);
}

#[test]
fn pose_asset_round_trip_apply_flip_blend_and_mask() {
    let rig = PoseRig::default();
    let mut pose = PoseState::default();
    pose.set_rotation(bone::L_SHOULDER, q(Vec3::Z, 60.0));
    pose.set_rotation(bone::L_ELBOW, q(Vec3::X, -90.0));
    pose.set_rotation(bone::HEAD, q(Vec3::Y, 20.0));
    pose.hands.left = [0.2; 5];
    let mask: BTreeSet<usize> = [bone::L_SHOULDER, bone::L_ELBOW].into_iter().collect();
    let asset = PoseAsset::from_mask("saudação", &pose, &mask, &rig);
    assert_eq!(asset.bones, vec!["LeftShoulder", "LeftElbow"]);
    assert!(asset.pose.rotation(bone::HEAD).dot(Quat::IDENTITY).abs() > 0.99999, "fora da máscara não vai pro arquivo");
    let json = asset.to_json().unwrap();
    let back = PoseAsset::from_json(json.as_bytes()).unwrap();
    assert_eq!(back, asset);

    // aplicar toca só nos ossos do ativo e preserva o resto
    let mut current = PoseState::default();
    current.set_rotation(bone::HEAD, q(Vec3::Y, -10.0));
    let applied = back.apply_to(&current, 1.0, false, None);
    assert!(applied.rotation(bone::L_ELBOW).dot(pose.rotation(bone::L_ELBOW)).abs() > 0.99999);
    assert!(applied.rotation(bone::HEAD).dot(current.rotation(bone::HEAD)).abs() > 0.99999);
    // espelhado: cai no lado direito
    let flipped = back.apply_to(&current, 1.0, true, None);
    assert!(flipped.rotation(bone::R_ELBOW).dot(q(Vec3::X, -90.0)).abs() > 0.99999);
    assert!(flipped.rotation(bone::L_ELBOW).dot(Quat::IDENTITY).abs() > 0.99999);
    // só nos ossos selecionados
    let sel: BTreeSet<usize> = [bone::L_ELBOW].into_iter().collect();
    let partial = back.apply_to(&current, 1.0, false, Some(&sel));
    assert!(partial.rotation(bone::L_SHOULDER).dot(Quat::IDENTITY).abs() > 0.99999);
    assert!(partial.rotation(bone::L_ELBOW).dot(pose.rotation(bone::L_ELBOW)).abs() > 0.99999);
    // blend 50 %
    let half = back.apply_to(&current, 0.5, false, None);
    assert!((half.rotation(bone::L_ELBOW).to_axis_angle().1.to_degrees() - 45.0).abs() < 0.01);
    // ativo vira comando e o comando é desfazível
    let mut state = ProjectState::default();
    let cmd = plan_pose_change(&state.character.pose, &applied, PoseOp::Apply).unwrap();
    let mut h = CommandHistory::new(10);
    h.execute(&mut state, cmd).unwrap();
    h.undo(&mut state).unwrap();
    assert!(state.character.pose.is_rest());
}

#[test]
fn pose_asset_rejects_garbage() {
    assert!(matches!(PoseAsset::from_json(b"not json"), Err(PoseAssetError::Json(_))));
    assert!(matches!(PoseAsset::from_json(&vec![b' '; POSE_ASSET_MAX_BYTES + 1]), Err(PoseAssetError::TooLarge(_))));
    let mut a = PoseAsset::new("x", PoseState::default());
    a.format = "other".into();
    assert!(matches!(PoseAsset::from_json(a.to_json().unwrap().as_bytes()), Err(PoseAssetError::Format(_))));
    let mut a = PoseAsset::new("x", PoseState::default());
    a.version = 99;
    assert!(matches!(PoseAsset::from_json(a.to_json().unwrap().as_bytes()), Err(PoseAssetError::Version(99))));
    let mut a = PoseAsset::new("x", PoseState::default());
    a.bones = vec!["Wings".into()];
    assert!(matches!(PoseAsset::from_json(a.to_json().unwrap().as_bytes()), Err(PoseAssetError::Invalid(_))));
    let mut a = PoseAsset::new("x", PoseState::default());
    a.pose.rotations.insert("Head".into(), [f32::INFINITY, 0.0, 0.0, 1.0]);
    // serde_json não escreve Infinity: o JSON vira `null` e a leitura falha com erro tratado
    let json = a.to_json().unwrap();
    assert!(PoseAsset::from_json(json.as_bytes()).is_err());
}

#[test]
fn presets_are_legal_grounded_and_distinct() {
    let (rig, _, _) = default_rig_and_mesh();
    let mut seen = Vec::new();
    for key in presets::BODY {
        let p = presets::body(key, &rig).unwrap_or_else(|| panic!("{key}"));
        p.validate().unwrap();
        assert!(rig.lowest_contact_y(&p).abs() < 2e-3, "{key}: pés fora do chão ({})", rig.lowest_contact_y(&p));
        // todo preset respeita os limites do perfil ANIME (o mais folgado)
        for b in 0..BONE_COUNT {
            if let Some(l) = bone_limit(b, LimitProfile::Anime) {
                assert!(within_limit(p.rotation(b), &l), "{key}: {} fora do limite anime", BONE_NAMES[b]);
            }
        }
        assert!(!seen.contains(&p), "{key} igual a outro preset");
        seen.push(p);
    }
    assert!(presets::body("nope", &rig).is_none());
    // T-pose: os braços ficam na horizontal (pulso na altura do ombro)
    let t = presets::body("t", &rig).unwrap();
    let j = rig.joint_positions(&t);
    assert!((j[bone::L_WRIST].y - j[bone::L_SHOULDER].y).abs() < 0.02);
    assert!(j[bone::L_WRIST].x > j[bone::L_SHOULDER].x + 0.4);
    assert!(j[bone::R_WRIST].x < j[bone::R_SHOULDER].x - 0.4);
    // sentado: o quadril desce e os joelhos ficam à frente
    let s = presets::body("sitting", &rig).unwrap();
    let js = rig.joint_positions(&s);
    assert!(js[bone::HIPS].y < rig.bind[bone::HIPS].y - 0.25);
    assert!(js[bone::L_KNEE].z > js[bone::HIPS].z + 0.25);
    for key in presets::HAND {
        let c = presets::hand(key).unwrap();
        assert!(c.iter().all(|v| (0.0..=1.0).contains(v)));
    }
    assert_ne!(presets::hand("fist"), presets::hand("open"));
}

#[test]
fn finger_curl_bends_only_the_finger_vertices() {
    let mut mesh = canonical_base_mesh(BaseGender::Male);
    let original = mesh.clone();
    assert!(finger_curl_mesh(&mut mesh, &HandsPose::default()));
    assert_eq!(mesh, original, "mão aberta não muda a malha");
    let mut h = HandsPose::default();
    h.left = [1.0; 5];
    assert!(finger_curl_mesh(&mut mesh, &h));
    let mut moved_left = 0;
    let mut moved_right = 0;
    let mut moved_other = 0;
    for (i, (a, b)) in original.vertices.iter().zip(&mesh.vertices).enumerate() {
        let d = Vec3::from_array(a.position).distance(Vec3::from_array(b.position));
        if d > 1e-6 {
            if (FINGERS_START..FINGERS_START + FINGER_BLOCK).contains(&i) {
                if a.position[0] > 0.0 {
                    moved_left += 1;
                } else {
                    moved_right += 1;
                }
            } else {
                moved_other += 1;
            }
            assert!(d < 0.12, "dedo curvou demais: {d}");
            assert!(b.position.iter().all(|v| v.is_finite()) && b.normal.iter().all(|v| v.is_finite()));
        }
    }
    assert!(moved_left > 100, "dedos esquerdos (x>0) deviam se mover: {moved_left}");
    assert_eq!(moved_right, 0, "mão direita aberta não se move");
    assert_eq!(moved_other, 0);
    // a ponta do dedo avançou para +Z (curvou para a frente) e subiu em relação ao repouso
    let tip = FINGERS_START + (2 * 2 + 1) * FINGER_VERTS + 4 * 7; // dedo médio, lado x>0, último anel
    assert!(mesh.vertices[tip].position[2] > original.vertices[tip].position[2] + 0.02);
    // malha que não é a canônica é recusada sem tocar
    let mut cube = Mesh::create_cube(1.0);
    assert!(!finger_curl_mesh(&mut cube, &h));
}
