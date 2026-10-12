use super::*;
use anigo_core::pose::{self, bone, bone_limit, within_limit, BONE_PARENTS};

fn rig() -> PoseRig {
    PoseRig::default()
}

fn q(axis: Vec3, deg: f32) -> Quat {
    Quat::from_axis_angle(axis, deg.to_radians())
}

fn lens_ok(rig: &PoseRig, a: &PoseState, b: &PoseState) {
    let (pa, pb) = (rig.joint_positions(a), rig.joint_positions(b));
    for i in 2..BONE_COUNT {
        let p = BONE_PARENTS[i].unwrap();
        assert!((pa[i].distance(pa[p]) - pb[i].distance(pb[p])).abs() < 2e-4, "{}: comprimento mudou", pose::BONE_NAMES[i]);
    }
}

#[test]
fn two_bone_reaches_every_reachable_target_for_every_limb() {
    let rig = rig();
    let rest = PoseState::default();
    for limb in Limb::ALL {
        let [r, m, t] = limb.chain();
        let j = rig.joint_positions(&rest);
        let (len_a, len_b) = (j[r].distance(j[m]), j[m].distance(j[t]));
        let mut rng = 7u64;
        let mut next = || {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((rng >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
        };
        for _ in 0..200 {
            // alvo aleatório numa casca entre 35 % e 95 % do alcance
            let dir = Vec3::new(next(), next(), next()).normalize_or_zero();
            if dir == Vec3::ZERO {
                continue;
            }
            let dist = (len_a + len_b) * (0.35 + 0.6 * (next() * 0.5 + 0.5));
            let target = j[r] + dir * dist;
            let pole = j[m] + Vec3::new(next(), next(), next()) * 0.5;
            let res = solve_limb(&rig, &rest, limb, target, Some(pole), &IkOptions::default());
            assert!(res.reached, "{limb:?} errou o alvo por {}", res.error);
            let pos = rig.joint_positions(&res.pose);
            // comprimentos preservados
            assert!((pos[r].distance(pos[m]) - len_a).abs() < 2e-4);
            assert!((pos[m].distance(pos[t]) - len_b).abs() < 2e-4);
            assert!(pos[t].distance(target) < 1e-3);
            assert!(pos.iter().all(|p| p.is_finite()));
            // só mexe nos ossos da cadeia (o resto da pose fica intacto)
            let changed = res.pose.differing_bones(&rest);
            assert!(changed.iter().all(|b| limb.chain().contains(b)), "{limb:?} mexeu em {changed:?}");
            lens_ok(&rig, &rest, &res.pose);
        }
    }
}

#[test]
fn unreachable_targets_stretch_straight_toward_the_target() {
    let rig = rig();
    let rest = PoseState::default();
    let limb = Limb::LeftArm;
    let [r, m, t] = limb.chain();
    let j = rig.joint_positions(&rest);
    let target = j[r] + Vec3::new(3.0, 1.0, 1.0);
    let res = solve_limb(&rig, &rest, limb, target, None, &IkOptions::default());
    assert!(!res.reached);
    let pos = rig.joint_positions(&res.pose);
    let dir = (target - j[r]).normalize();
    // quase reto e apontando para o alvo
    assert!((pos[t] - j[r]).normalize().dot(dir) > 0.9999);
    assert!((pos[m] - j[r]).normalize().dot(dir) > 0.999);
    let reach = j[r].distance(j[m]) + j[m].distance(j[t]);
    assert!((pos[t].distance(pos[r]) - reach).abs() < 2e-3);
    assert!(pos.iter().all(|p| p.is_finite()));
    // alvo exatamente na raiz (distância zero) não gera NaN
    let res = solve_limb(&rig, &rest, limb, j[r], None, &IkOptions::default());
    assert!(rig.joint_positions(&res.pose).iter().all(|p| p.is_finite()));
    // alvo NaN é recusado sem alterar a pose
    let res = solve_limb(&rig, &rest, limb, Vec3::NAN, None, &IkOptions::default());
    assert_eq!(res.pose, rest);
}

#[test]
fn the_pole_decides_which_way_the_joint_bends() {
    let rig = rig();
    let rest = PoseState::default();
    let j = rig.joint_positions(&rest);
    for limb in [Limb::LeftArm, Limb::LeftLeg] {
        let [r, m, t] = limb.chain();
        // alvo moderadamente perto para ter dobra pronunciada
        let target = j[r] + (j[t] - j[r]) * 0.75 + Vec3::new(0.0, 0.0, 0.05);
        for pole_dir in [Vec3::Z, -Vec3::Z, Vec3::X, -Vec3::X] {
            let pole = j[m] + pole_dir * 0.6;
            let res = solve_limb(&rig, &rest, limb, target, Some(pole), &IkOptions::default());
            let pos = rig.joint_positions(&res.pose);
            let axis = (pos[t] - pos[r]).normalize();
            let bend = (pos[m] - pos[r]) - axis * (pos[m] - pos[r]).dot(axis);
            let want = (pole_dir - axis * pole_dir.dot(axis)).normalize_or_zero();
            assert!(bend.normalize().dot(want) > 0.98, "{limb:?}: dobra {bend} não segue o pole {pole_dir}");
        }
    }
    // pole colinear com o alvo (degenerado) cai na dobra atual, sem NaN
    let [r, _, t] = Limb::LeftLeg.chain();
    let target = j[r] + (j[t] - j[r]) * 0.8;
    let res = solve_limb(&rig, &rest, Limb::LeftLeg, target, Some(j[r] + (target - j[r]) * 3.0), &IkOptions::default());
    assert!(rig.joint_positions(&res.pose).iter().all(|p| p.is_finite()));
}

#[test]
fn ik_with_limits_stays_legal_and_reports_the_miss() {
    let rig = rig();
    let rest = PoseState::default();
    let j = rig.joint_positions(&rest);
    // joelho não dobra para trás: um alvo que exigiria hiperextensão é recortado
    let [r, m, t] = Limb::LeftLeg.chain();
    let target = j[r] + (j[t] - j[r]) * 0.6;
    let pole_back = j[m] - Vec3::Z * 0.6; // força o joelho para trás
    let opts = IkOptions { limits: LimitProfile::Human, keep_end_orientation: true };
    let res = solve_limb(&rig, &rest, Limb::LeftLeg, target, Some(pole_back), &opts);
    for b in Limb::LeftLeg.chain() {
        if let Some(l) = bone_limit(b, LimitProfile::Human) {
            assert!(within_limit(res.pose.rotation(b), &l), "{} fora do limite", pose::BONE_NAMES[b]);
        }
    }
    assert!(res.error > 1e-3, "o recorte de limite deve ser reportado como erro");
    // sem limites alcança
    let free = solve_limb(&rig, &rest, Limb::LeftLeg, target, Some(pole_back), &IkOptions::default());
    assert!(free.reached);
}

#[test]
fn end_orientation_is_kept_or_released() {
    let rig = rig();
    let mut base = PoseState::default();
    base.set_rotation(bone::L_ANKLE, q(Vec3::X, 20.0));
    let before = rig.world_rotations(&base)[bone::L_ANKLE];
    let j = rig.joint_positions(&base);
    let target = j[bone::L_ANKLE] + Vec3::new(0.0, 0.15, 0.25);
    let keep = solve_limb(&rig, &base, Limb::LeftLeg, target, None, &IkOptions::default());
    let after = rig.world_rotations(&keep.pose)[bone::L_ANKLE];
    assert!(before.dot(after).abs() > 0.99999, "orientação do pé mantida");
    let free = solve_limb(&rig, &base, Limb::LeftLeg, target, None, &IkOptions { keep_end_orientation: false, ..Default::default() });
    let after2 = rig.world_rotations(&free.pose)[bone::L_ANKLE];
    assert!(before.dot(after2).abs() < 0.9999);
}

#[test]
fn fabrik_preserves_lengths_and_reaches() {
    let pts = vec![Vec3::ZERO, Vec3::new(0.0, 0.2, 0.0), Vec3::new(0.0, 0.45, 0.0), Vec3::new(0.0, 0.65, 0.0), Vec3::new(0.0, 0.85, 0.0)];
    let lens: Vec<f32> = pts.windows(2).map(|w| w[0].distance(w[1])).collect();
    let mut rng = 3u64;
    let mut next = || {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((rng >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
    };
    for _ in 0..300 {
        let target = Vec3::new(next(), next(), next()) * 0.9;
        let out = fabrik(&pts, target, 32, 1e-5);
        assert_eq!(out[0], Vec3::ZERO, "a raiz fica");
        for (w, l) in out.windows(2).zip(&lens) {
            assert!((w[0].distance(w[1]) - l).abs() < 1e-4, "comprimento mudou");
        }
        if target.length() < 0.85 * 0.97 {
            assert!(out[4].distance(target) < 2e-3, "FABRIK não convergiu: {}", out[4].distance(target));
        } else {
            // fora do alcance: cadeia reta na direção do alvo
            assert!((out[4].normalize() - target.normalize()).length() < 1e-3);
        }
        assert!(out.iter().all(|p| p.is_finite()));
    }
    // degenerados
    assert_eq!(fabrik(&pts[..1], Vec3::X, 10, 1e-4), pts[..1].to_vec());
    assert_eq!(fabrik(&pts, Vec3::NAN, 10, 1e-4), pts);
}

#[test]
fn spine_chain_follows_a_head_target_without_stretching() {
    let rig = rig();
    let rest = PoseState::default();
    let j = rig.joint_positions(&rest);
    let target = j[bone::HEAD] + Vec3::new(0.08, -0.07, 0.15);
    let res = solve_chain(&rig, &rest, &UPPER_BODY_CHAIN, target, &IkOptions::default());
    assert!(res.reached, "erro {}", res.error);
    lens_ok(&rig, &rest, &res.pose);
    // o quadril e as pernas não se mexem
    let changed = res.pose.differing_bones(&rest);
    assert!(changed.iter().all(|b| UPPER_BODY_CHAIN.contains(b)), "{changed:?}");
    // a cadeia inteira (com o quadril fixo) também preserva comprimentos
    let res2 = solve_chain(&rig, &rest, &SPINE_CHAIN, target, &IkOptions::default());
    lens_ok(&rig, &rest, &res2.pose);
    assert!(res2.error < 0.01);
    // com limites do perfil humano a coluna não passa dos ângulos permitidos
    let limited = solve_chain(&rig, &rest, &UPPER_BODY_CHAIN, j[bone::HEAD] + Vec3::new(0.0, -0.5, 0.5), &IkOptions { limits: LimitProfile::Human, keep_end_orientation: false });
    for b in UPPER_BODY_CHAIN {
        let l = bone_limit(b, LimitProfile::Human).unwrap();
        assert!(within_limit(limited.pose.rotation(b), &l));
    }
}

#[test]
fn look_at_points_the_face_at_the_target_without_roll() {
    let rig = rig();
    let rest = PoseState::default();
    let eye = eye_center(&rig, &rest);
    assert!(face_direction(&rig, &rest).dot(Vec3::Z) > 0.9999);
    // alvos dentro da amplitude de cabeça + pescoço
    for target in [eye + Vec3::new(1.0, 0.0, 1.0), eye + Vec3::new(-1.0, 0.2, 1.0), eye + Vec3::new(0.3, 0.6, 1.0), eye + Vec3::new(0.0, -0.4, 1.0)] {
        let res = look_at_head(&rig, &rest, target, 1.0, 0.35, LimitProfile::Free);
        assert!(res.reached, "erro angular {}", res.error);
        // sem roll: o eixo da direita da cabeça fica no plano horizontal
        let e = rig.world_rotations(&res.pose);
        let right = e[bone::HEAD] * Vec3::X;
        assert!(right.y.abs() < 0.02, "head roll {}", right.y);
        // só pescoço e cabeça mexem
        assert!(res.pose.differing_bones(&rest).iter().all(|b| *b == bone::NECK || *b == bone::HEAD));
    }
    // peso 0 = nada; 0.5 = metade do caminho
    let t = eye + Vec3::new(1.0, 0.0, 1.0);
    assert!(look_at_head(&rig, &rest, t, 0.0, 0.35, LimitProfile::Free).pose.is_rest());
    let half = look_at_head(&rig, &rest, t, 0.5, 0.35, LimitProfile::Free);
    let yaw_half = face_direction(&rig, &half.pose).x.atan2(face_direction(&rig, &half.pose).z).to_degrees();
    assert!((yaw_half - 22.5).abs() < 2.0, "{yaw_half}");
    // alvo atrás é recortado pelos limites (a cabeça não gira 180°)
    let behind = look_at_head(&rig, &rest, eye + Vec3::new(0.0, 0.0, -2.0), 1.0, 0.35, LimitProfile::Human);
    for b in [bone::NECK, bone::HEAD] {
        assert!(within_limit(behind.pose.rotation(b), &bone_limit(b, LimitProfile::Human).unwrap()));
    }
    assert!(!behind.reached);
    assert!(rig.joint_positions(&behind.pose).iter().all(|p| p.is_finite()));
}

#[test]
fn foot_lock_keeps_the_ankles_planted_while_the_hips_move() {
    let rig = rig();
    let rest = PoseState::default();
    let j0 = rig.joint_positions(&rest);
    let planted = [(Limb::LeftLeg, j0[bone::L_ANKLE]), (Limb::RightLeg, j0[bone::R_ANKLE])];
    // abaixa o quadril 18 cm e o desloca 5 cm: sem IK os pés iriam junto
    for (dy, dx, dz) in [(-0.18, 0.0, 0.0), (-0.25, 0.05, 0.03), (-0.1, -0.06, 0.0)] {
        let mut moved = rest.clone();
        moved.hips_translation = [dx, dy, dz];
        let naive = rig.joint_positions(&moved);
        assert!(naive[bone::L_ANKLE].distance(j0[bone::L_ANKLE]) > 0.05);
        let res = lock_feet(&rig, &moved, &planted, &IkOptions::default());
        assert!(res.reached, "foot lock errou {} m", res.error);
        let pos = rig.joint_positions(&res.pose);
        assert!(pos[bone::L_ANKLE].distance(j0[bone::L_ANKLE]) < 1e-3);
        assert!(pos[bone::R_ANKLE].distance(j0[bone::R_ANKLE]) < 1e-3);
        // o quadril realmente desceu
        assert!(pos[bone::HIPS].y < j0[bone::HIPS].y - 0.05);
        // joelhos dobram para a frente (+Z) ao agachar
        if dy < -0.15 {
            assert!(pos[bone::L_KNEE].z > j0[bone::L_KNEE].z, "joelho deve ir para a frente");
        }
        lens_ok(&rig, &moved, &res.pose);
    }
}
