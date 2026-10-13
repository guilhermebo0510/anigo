//! Edições de pose puras: cada função recebe a pose do INÍCIO do gesto e devolve a pose desejada.
//! Sem estado oculto: repetir a chamada com o mesmo gesto dá o mesmo resultado (sem deriva no arrasto).

use anigo_core::pose::{
    bone, bone_limit, clamp_to_limit, euler_deg_to_quat, is_left_bone, is_right_bone, mirror_bone, Limb, LimitProfile, PoseRig, PoseState,
    BONE_COUNT, BONE_PARENTS,
};
use anigo_ik::pose_ik::{lock_feet, solve_limb, IkOptions};
use glam::{Quat, Vec3};
use std::collections::BTreeSet;

/// Opções de edição vindas da UI.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EditOpts {
    pub profile: LimitProfile,
    /// Editar um osso edita o par do outro lado (X-Axis Mirror do Blender).
    pub symmetry: bool,
    /// Passo de snap de rotação em graus.
    pub snap_deg: Option<f32>,
}

impl Default for EditOpts {
    fn default() -> Self {
        EditOpts { profile: LimitProfile::Human, symmetry: false, snap_deg: None }
    }
}

/// Snap de ângulo (radianos) ao passo em graus.
pub fn snap_angle(angle: f32, step_deg: Option<f32>) -> f32 {
    match step_deg {
        Some(s) if s > 0.0 => {
            let step = s.to_radians();
            (angle / step).round() * step
        }
        _ => angle,
    }
}

/// Reflexo no plano X de uma rotação: `(x, −y, −z, w)`.
pub fn mirror_quat(q: Quat) -> Quat {
    Quat::from_xyzw(q.x, -q.y, -q.z, q.w)
}

fn clamp_bone(q: Quat, bone_i: usize, profile: LimitProfile) -> Quat {
    match bone_limit(bone_i, profile) {
        Some(l) => clamp_to_limit(q, &l),
        None => q.normalize(),
    }
}

/// Aplica `local` ao osso e, se `symmetry`, o reflexo ao osso espelhado.
fn put_local(pose: &mut PoseState, bone_i: usize, local: Quat, opts: &EditOpts) {
    let c = clamp_bone(local, bone_i, opts.profile);
    pose.set_rotation(bone_i, c);
    let m = mirror_bone(bone_i);
    if opts.symmetry && m != bone_i {
        pose.set_rotation(m, clamp_bone(mirror_quat(c), m, opts.profile));
    }
}

/// Gira o osso `bone_i` em torno de `axis_world` por `angle` partindo da pose `start` (gizmo FK).
pub fn rotate_bone_world(rig: &PoseRig, start: &PoseState, bone_i: usize, axis_world: Vec3, angle: f32, opts: &EditOpts) -> PoseState {
    let world = rig.world_rotations(start);
    let parent = BONE_PARENTS[bone_i].map(|p| world[p]).unwrap_or(Quat::IDENTITY);
    let a = snap_angle(angle, opts.snap_deg);
    let delta = if axis_world.length_squared() > 1e-12 { Quat::from_axis_angle(axis_world.normalize(), a) } else { Quat::IDENTITY };
    let local = (parent.inverse() * delta * world[bone_i]).normalize();
    let mut out = start.clone();
    put_local(&mut out, bone_i, local, opts);
    out
}

/// Define a rotação local do osso diretamente (painel numérico).
pub fn set_bone_local(start: &PoseState, bone_i: usize, local: Quat, opts: &EditOpts) -> PoseState {
    let mut out = start.clone();
    put_local(&mut out, bone_i, local, opts);
    out
}

/// Define a rotação local por Euler XYZ em graus (painel numérico).
pub fn set_bone_euler(start: &PoseState, bone_i: usize, euler_deg: [f32; 3], opts: &EditOpts) -> PoseState {
    set_bone_local(start, bone_i, euler_deg_to_quat(euler_deg), opts)
}

/// Zera os ossos de `bones` (e a translação do root/quadril se eles estiverem na seleção).
pub fn reset_bones(start: &PoseState, bones: &BTreeSet<usize>) -> PoseState {
    let mut out = start.clone();
    for &b in bones {
        out.set_rotation(b, Quat::IDENTITY);
    }
    if bones.contains(&bone::ROOT) {
        out.root_translation = [0.0; 3];
    }
    if bones.contains(&bone::HIPS) {
        out.hips_translation = [0.0; 3];
    }
    out
}

/// Cola `clip` em `current`: só os ossos de `mask` (como o Paste Pose do Blender, por nome); `flipped` espelha antes.
pub fn paste_pose(current: &PoseState, clip: &PoseState, mask: &BTreeSet<usize>, flipped: bool) -> PoseState {
    let src = if flipped { clip.mirrored() } else { clip.clone() };
    let eff: BTreeSet<usize> = if flipped { mask.iter().map(|b| mirror_bone(*b)).collect() } else { mask.clone() };
    current.overlay(&src, &eff)
}

/// Movimento do quadril/raiz em um arrasto.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HipsMove {
    /// Deslocamento de mundo desde o início do gesto.
    pub delta: Vec3,
    /// `true` move o `Root` (personagem inteiro), `false` só o `Hips` (corpo; pés ficam se `foot_lock`).
    pub whole_body: bool,
    /// Mantém os tornozelos onde estavam no início do gesto (foot lock por IK de perna).
    pub foot_lock: bool,
    /// Depois da edição, apoia o ponto de contato mais baixo no chão.
    pub auto_ground: bool,
}

/// Move o quadril (ou o corpo todo) com foot lock opcional.
pub fn move_hips(rig: &PoseRig, start: &PoseState, mv: HipsMove, opts: &EditOpts) -> PoseState {
    let mut out = start.clone();
    let key = if mv.whole_body { &mut out.root_translation } else { &mut out.hips_translation };
    let base = if mv.whole_body { start.root_translation } else { start.hips_translation };
    for i in 0..3 {
        key[i] = base[i] + mv.delta[i];
    }
    if mv.foot_lock && !mv.whole_body {
        let j0 = rig.joint_positions(start);
        let planted = [(Limb::LeftLeg, j0[bone::L_ANKLE]), (Limb::RightLeg, j0[bone::R_ANKLE])];
        let ik = IkOptions { limits: opts.profile, keep_end_orientation: true };
        out = lock_feet(rig, &out, &planted, &ik).pose;
    }
    if mv.auto_ground {
        out = rig.grounded(&out, 0.0);
    }
    out
}

/// Arrasta o efetuador de um membro até `target` (mundo) com `pole`; com simetria o membro oposto acompanha espelhado.
pub fn drag_effector(rig: &PoseRig, start: &PoseState, limb: Limb, target: Vec3, pole: Option<Vec3>, opts: &EditOpts) -> PoseState {
    let ik = IkOptions { limits: opts.profile, keep_end_orientation: true };
    let mut out = solve_limb(rig, start, limb, target, pole, &ik).pose;
    if opts.symmetry {
        let flip = |v: Vec3| Vec3::new(-v.x, v.y, v.z);
        out = solve_limb(rig, &out, limb.mirror(), flip(target), pole.map(flip), &ik).pose;
    }
    out
}

/// Ossos do "lado" de um osso (para filtros de UI).
pub fn side_of(bone_i: usize) -> Side {
    if is_left_bone(bone_i) {
        Side::Left
    } else if is_right_bone(bone_i) {
        Side::Right
    } else {
        Side::Center
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Center,
}

/// Todos os ossos da pose (0..24).
pub fn all_bones() -> BTreeSet<usize> {
    (0..BONE_COUNT).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::pose::{within_limit, BONE_NAMES};

    fn rig() -> PoseRig {
        PoseRig::default()
    }

    #[test]
    fn rotate_bone_world_turns_the_bone_about_the_world_axis() {
        let rig = rig();
        let opts = EditOpts { profile: LimitProfile::Free, ..Default::default() };
        let p = rotate_bone_world(&rig, &PoseState::default(), bone::L_SHOULDER, Vec3::Z, 90f32.to_radians(), &opts);
        let j = rig.joint_positions(&p);
        // braço pendurado + 90° em Z (mundo) = braço na horizontal para +X (T-pose)
        assert!((j[bone::L_WRIST].y - j[bone::L_SHOULDER].y).abs() < 0.02);
        assert!(j[bone::L_WRIST].x > j[bone::L_SHOULDER].x + 0.4);
        // nada além do ombro mudou na pose
        assert_eq!(p.posed_bones(), vec![bone::L_SHOULDER]);
    }

    #[test]
    fn world_axis_rotation_composes_with_a_rotated_parent() {
        let rig = rig();
        let opts = EditOpts { profile: LimitProfile::Free, ..Default::default() };
        let mut start = PoseState::default();
        start.set_rotation(bone::SPINE, Quat::from_rotation_y(0.6));
        // girar o peito em torno do eixo Y do MUNDO: a rotação de mundo do peito é exatamente a do pai x delta
        let p = rotate_bone_world(&rig, &start, bone::CHEST, Vec3::Y, 0.4, &opts);
        let w = rig.world_rotations(&p);
        let (_, ang) = w[bone::CHEST].to_axis_angle();
        assert!((ang - 1.0).abs() < 1e-4, "0,6 + 0,4 rad em torno de Y: {ang}");
    }

    #[test]
    fn limits_clamp_fk_edits_and_free_profile_does_not() {
        let rig = rig();
        let limited = EditOpts { profile: LimitProfile::Human, ..Default::default() };
        let p = rotate_bone_world(&rig, &PoseState::default(), bone::L_ELBOW, Vec3::X, 170f32.to_radians(), &limited);
        let lim = bone_limit(bone::L_ELBOW, LimitProfile::Human).unwrap();
        assert!(within_limit(p.rotation(bone::L_ELBOW), &lim), "hiperextensão cortada");
        let free = EditOpts { profile: LimitProfile::Free, ..Default::default() };
        let q = rotate_bone_world(&rig, &PoseState::default(), bone::L_ELBOW, Vec3::X, 170f32.to_radians(), &free);
        assert!(!within_limit(q.rotation(bone::L_ELBOW), &lim));
    }

    #[test]
    fn symmetry_mirrors_the_edit_to_the_opposite_bone() {
        let rig = rig();
        let opts = EditOpts { profile: LimitProfile::Human, symmetry: true, snap_deg: None };
        let p = rotate_bone_world(&rig, &PoseState::default(), bone::L_SHOULDER, Vec3::Z, 60f32.to_radians(), &opts);
        let j = rig.joint_positions(&p);
        assert!((j[bone::L_WRIST].x + j[bone::R_WRIST].x).abs() < 1e-4, "punhos simétricos em X");
        assert!((j[bone::L_WRIST].y - j[bone::R_WRIST].y).abs() < 1e-4);
        assert!((j[bone::L_WRIST].z - j[bone::R_WRIST].z).abs() < 1e-4);
        // osso central não tem par: a simetria não o toca
        let q = rotate_bone_world(&rig, &PoseState::default(), bone::SPINE, Vec3::X, 0.2, &opts);
        assert_eq!(q.posed_bones(), vec![bone::SPINE]);
    }

    #[test]
    fn snap_rounds_the_angle_to_the_step() {
        assert!((snap_angle(0.30, Some(15.0)) - 15f32.to_radians()).abs() < 1e-6);
        assert!((snap_angle(0.12, Some(15.0))).abs() < 1e-6);
        assert!((snap_angle(-0.30, Some(15.0)) + 15f32.to_radians()).abs() < 1e-6);
        assert_eq!(snap_angle(0.3, None), 0.3);
        let rig = rig();
        let opts = EditOpts { profile: LimitProfile::Free, symmetry: false, snap_deg: Some(15.0) };
        let p = rotate_bone_world(&rig, &PoseState::default(), bone::HEAD, Vec3::Y, 0.33, &opts);
        let a = p.rotation(bone::HEAD).to_axis_angle().1.to_degrees();
        assert!((a - 15.0).abs() < 1e-3, "{a}");
    }

    #[test]
    fn numeric_euler_edit_round_trips_through_the_panel() {
        let opts = EditOpts { profile: LimitProfile::Free, ..Default::default() };
        let p = set_bone_euler(&PoseState::default(), bone::L_THIGH, [-40.0, 10.0, 15.0], &opts);
        let e = anigo_core::pose::quat_to_euler_deg(p.rotation(bone::L_THIGH));
        assert!((e[0] + 40.0).abs() < 1e-3 && (e[1] - 10.0).abs() < 1e-3 && (e[2] - 15.0).abs() < 1e-3, "{e:?}");
    }

    #[test]
    fn reset_and_paste() {
        let mut p = PoseState::default();
        p.set_rotation(bone::HEAD, Quat::from_rotation_y(0.5));
        p.set_rotation(bone::L_ELBOW, Quat::from_rotation_x(-1.0));
        p.hips_translation = [0.0, -0.1, 0.0];
        let sel: BTreeSet<usize> = [bone::HEAD, bone::HIPS].into_iter().collect();
        let r = reset_bones(&p, &sel);
        assert!(r.rotation(bone::HEAD).dot(Quat::IDENTITY).abs() > 0.99999);
        assert_eq!(r.hips_translation, [0.0; 3]);
        assert!(r.rotation(bone::L_ELBOW).dot(p.rotation(bone::L_ELBOW)).abs() > 0.99999);
        // colar só nos ossos da máscara, espelhado
        let mask: BTreeSet<usize> = [bone::L_ELBOW].into_iter().collect();
        let pasted = paste_pose(&PoseState::default(), &p, &mask, true);
        assert!(pasted.rotation(bone::R_ELBOW).dot(Quat::from_rotation_x(-1.0)).abs() > 0.99999);
        assert_eq!(pasted.posed_bones(), vec![bone::R_ELBOW]);
        let plain = paste_pose(&PoseState::default(), &p, &mask, false);
        assert_eq!(plain.posed_bones(), vec![bone::L_ELBOW]);
    }

    #[test]
    fn hips_move_with_foot_lock_keeps_feet_and_ground_contact() {
        let rig = rig();
        let opts = EditOpts { profile: LimitProfile::Human, ..Default::default() };
        let start = PoseState::default();
        let j0 = rig.joint_positions(&start);
        let mv = HipsMove { delta: Vec3::new(0.03, -0.2, 0.0), whole_body: false, foot_lock: true, auto_ground: false };
        let p = move_hips(&rig, &start, mv, &opts);
        let j = rig.joint_positions(&p);
        assert!(j[bone::L_ANKLE].distance(j0[bone::L_ANKLE]) < 2e-3 && j[bone::R_ANKLE].distance(j0[bone::R_ANKLE]) < 2e-3, "tornozelos não ficaram");
        assert!(j[bone::HIPS].y < j0[bone::HIPS].y - 0.15);
        // sem foot lock os pés descem junto; com auto-ground voltam ao chão
        let free = move_hips(&rig, &start, HipsMove { foot_lock: false, auto_ground: true, ..mv }, &opts);
        assert!(rig.lowest_contact_y(&free).abs() < 1e-4, "auto-ground encosta no chão");
        // corpo inteiro: pés vão junto, nada de IK
        let whole = move_hips(&rig, &start, HipsMove { whole_body: true, foot_lock: true, auto_ground: false, delta: Vec3::new(1.0, 0.0, 0.5) }, &opts);
        assert_eq!(whole.root_translation, [1.0, 0.0, 0.5]);
        assert_eq!(whole.posed_bones(), Vec::<usize>::new());
    }

    #[test]
    fn effector_drag_respects_limits_and_symmetry() {
        let rig = rig();
        let start = PoseState::default();
        let j0 = rig.joint_positions(&start);
        let target = j0[bone::L_WRIST] + Vec3::new(0.15, 0.4, 0.3);
        let p = drag_effector(&rig, &start, Limb::LeftArm, target, None, &EditOpts { profile: LimitProfile::Free, ..Default::default() });
        assert!(rig.joint_positions(&p)[bone::L_WRIST].distance(target) < 1e-3);
        // com simetria, o outro punho vai ao ponto espelhado
        let s = drag_effector(&rig, &start, Limb::LeftArm, target, None, &EditOpts { profile: LimitProfile::Free, symmetry: true, snap_deg: None });
        let js = rig.joint_positions(&s);
        assert!(js[bone::R_WRIST].distance(Vec3::new(-target.x, target.y, target.z)) < 1e-3);
        for b in 0..BONE_COUNT {
            assert!(s.rotation(b).is_finite(), "{}", BONE_NAMES[b]);
        }
    }
}
