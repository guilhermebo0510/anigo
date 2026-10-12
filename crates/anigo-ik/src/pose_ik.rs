//! IK de pose sobre o modelo canônico de `anigo_core::pose` (Posing): two-bone com pole, FABRIK para cadeias
//! longas, olhar da cabeça e travamento de pés.
//!
//! Tudo opera em **espaço de mundo** (como o Basic IK do Control Rig) e devolve uma [`PoseState`] nova — os
//! planejadores são puros; o app transforma a diferença em UM `Command::SetPose`.
//!
//! * [`solve_limb`]: ombro/cotovelo/punho ou quadril/joelho/tornozelo. Lei dos cossenos (fechado, sem
//!   iteração); o plano de dobra vem do `pole` (ponto de mundo) ou da dobra atual; comprimentos preservados.
//! * [`fabrik`] + [`solve_chain`]: Forward And Backward Reaching IK (Aristidou & Lasenby, 2011) para a coluna.
//! * [`look_at_head`]: rotação sem *roll* que aponta a face (+Z da cabeça) para o alvo, repartida pescoço/cabeça.
//! * [`lock_feet`]: refaz as pernas para que os tornozelos fiquem onde estavam (foot lock).

use anigo_core::pose::{
    bone, bone_limit, clamp_to_limit, Limb, LimitProfile, PoseRig, PoseState, BONE_COUNT, BONE_PARENTS,
};
use glam::{Mat3, Quat, Vec3};

/// Folga numérica nos extremos do alcance (evita a singularidade do membro totalmente esticado).
const REACH_EPS: f32 = 1e-4;

/// Resultado de uma resolução de IK.
#[derive(Debug, Clone, PartialEq)]
pub struct IkResult {
    pub pose: PoseState,
    /// O efetuador chegou ao alvo (distância < 1 mm) ao final, depois dos limites.
    pub reached: bool,
    /// Distância final efetuador → alvo (m).
    pub error: f32,
}

/// Opções comuns.
#[derive(Debug, Clone, Copy)]
pub struct IkOptions {
    /// Perfil de limites aplicado às rotações resultantes (`Free` = nenhum).
    pub limits: LimitProfile,
    /// Mantém a orientação de mundo do efetuador (mão/pé) depois do IK.
    pub keep_end_orientation: bool,
}

impl Default for IkOptions {
    fn default() -> Self {
        IkOptions { limits: LimitProfile::Free, keep_end_orientation: true }
    }
}

/// Rotação que leva o par (eixo `dir_rest`, eixo lateral `side_rest`) ao par (`dir_new`, `side_new`): `dir` exato e o
/// lateral projetado no plano perpendicular a `dir` (os dois pares viram bases ortonormais).
fn frame_rotation(dir_rest: Vec3, side_rest: Vec3, dir_new: Vec3, side_new: Vec3) -> Quat {
    let basis = |dir: Vec3, side: Vec3| -> Option<Mat3> {
        let e1 = dir.normalize_or_zero();
        let e2 = (side - e1 * side.dot(e1)).normalize_or_zero();
        if e1 == Vec3::ZERO || e2 == Vec3::ZERO {
            return None;
        }
        Some(Mat3::from_cols(e1, e2, e1.cross(e2)))
    };
    match (basis(dir_rest, side_rest), basis(dir_new, side_new)) {
        (Some(r), Some(n)) => Quat::from_mat3(&(n * r.transpose())).normalize(),
        _ => arc(dir_rest, dir_new),
    }
}

/// Rotação mínima que leva `a` a `b` (vetores não necessariamente unitários).
pub fn arc(a: Vec3, b: Vec3) -> Quat {
    let (a, b) = (a.normalize_or_zero(), b.normalize_or_zero());
    if a == Vec3::ZERO || b == Vec3::ZERO {
        return Quat::IDENTITY;
    }
    let d = a.dot(b);
    if d > 1.0 - 1e-7 {
        return Quat::IDENTITY;
    }
    if d < -1.0 + 1e-7 {
        // oposto: qualquer eixo perpendicular
        let axis = if a.x.abs() < 0.9 { Vec3::X } else { Vec3::Y }.cross(a).normalize();
        return Quat::from_axis_angle(axis, std::f32::consts::PI);
    }
    Quat::from_rotation_arc(a, b)
}

/// Rotação de mundo de cada osso e suas posições (avaliação completa da pose).
struct Eval {
    rot: [Quat; BONE_COUNT],
    pos: [Vec3; BONE_COUNT],
}

fn eval(rig: &PoseRig, pose: &PoseState) -> Eval {
    Eval { rot: rig.world_rotations(pose), pos: rig.joint_positions(pose) }
}

fn parent_rot(e: &Eval, bone_i: usize) -> Quat {
    BONE_PARENTS[bone_i].map(|p| e.rot[p]).unwrap_or(Quat::IDENTITY)
}

/// Define a rotação de MUNDO desejada de um osso (converte para o delta local sobre o pai já avaliado).
fn set_world_rotation(pose: &mut PoseState, e: &Eval, bone_i: usize, world: Quat) {
    pose.set_rotation(bone_i, (parent_rot(e, bone_i).inverse() * world).normalize());
}

/// Dica da direção de dobra quando o membro está reto e não há pole: joelho para a frente (+Z), cotovelo para trás.
fn default_bend(limb: Limb) -> Vec3 {
    if limb.is_arm() {
        -Vec3::Z
    } else {
        Vec3::Z
    }
}

/// Posição "natural" do pole: à frente do joelho / atrás do cotovelo, a 0,35 m da junta do meio.
pub fn default_pole(rig: &PoseRig, pose: &PoseState, limb: Limb) -> Vec3 {
    let [_, mid, _] = limb.chain();
    let e = eval(rig, pose);
    e.pos[mid] + default_bend(limb) * 0.35
}

/// Resolve um membro de dois ossos até `target` (mundo) com `pole` (ponto de mundo; `None` mantém a dobra atual).
pub fn solve_limb(rig: &PoseRig, pose: &PoseState, limb: Limb, target: Vec3, pole: Option<Vec3>, opts: &IkOptions) -> IkResult {
    let [r, m, t] = limb.chain();
    let e = eval(rig, pose);
    let (a, b, c) = (e.pos[r], e.pos[m], e.pos[t]);
    let len_a = a.distance(b);
    let len_b = b.distance(c);
    if !target.is_finite() || len_a < 1e-6 || len_b < 1e-6 {
        return IkResult { pose: pose.clone(), reached: false, error: f32::INFINITY };
    }
    let to_target = target - a;
    let mut d = to_target.length();
    let u = if d > 1e-6 { to_target / d } else { (c - a).normalize_or_zero() };
    let u = if u == Vec3::ZERO { -Vec3::Y } else { u };
    let (min_d, max_d) = ((len_a - len_b).abs() + REACH_EPS, len_a + len_b - REACH_EPS);
    d = d.clamp(min_d, max_d);

    // direção de dobra: componente do (pole − A) perpendicular a u; senão a dobra atual; senão a dica
    let perp = |v: Vec3| (v - u * v.dot(u)).normalize_or_zero();
    let mut bend = pole.map(|p| perp(p - a)).unwrap_or(Vec3::ZERO);
    if bend == Vec3::ZERO {
        bend = perp(b - a);
    }
    if bend == Vec3::ZERO {
        bend = perp(default_bend(limb));
    }
    if bend == Vec3::ZERO {
        bend = perp(Vec3::X);
    }

    let cos_a = ((len_a * len_a + d * d - len_b * len_b) / (2.0 * len_a * d)).clamp(-1.0, 1.0);
    let sin_a = (1.0 - cos_a * cos_a).max(0.0).sqrt();
    let b_new = a + u * (len_a * cos_a) + bend * (len_a * sin_a);
    let c_new = a + u * d;

    // Dobradiça anatômica: o eixo X local do osso-raiz e do osso do meio fica sobre a normal do plano de dobra
    // (joelho/cotovelo giram só em X, sem torção), e a raiz recebe a torção necessária para isso.
    let d1 = (b_new - a).normalize_or_zero();
    let d2 = (c_new - b_new).normalize_or_zero();
    let mut hinge = d1.cross(d2);
    if hinge.length() < 1e-4 {
        hinge = u.cross(bend);
    }
    let mut hinge = hinge.normalize_or_zero();
    if hinge == Vec3::ZERO {
        hinge = e.rot[r] * Vec3::X;
    }
    // mantém o sentido do eixo X atual do osso (evita inversão de 180° na torção)
    if hinge.dot(e.rot[r] * Vec3::X) < 0.0 {
        hinge = -hinge;
    }
    let rest_r = (rig.bind[m] - rig.bind[r]).normalize_or_zero();
    let rest_m = (rig.bind[t] - rig.bind[m]).normalize_or_zero();
    let root_world = frame_rotation(rest_r, Vec3::X, d1, hinge);
    let mid_world = frame_rotation(rest_m, Vec3::X, d2, hinge);
    let mut out = pose.clone();
    set_world_rotation(&mut out, &e, r, root_world);
    out.set_rotation(m, (root_world.inverse() * mid_world).normalize());
    if opts.keep_end_orientation {
        out.set_rotation(t, (mid_world.inverse() * e.rot[t]).normalize());
    }
    if opts.limits.scale().is_some() {
        for bone_i in limb.chain() {
            if let Some(l) = bone_limit(bone_i, opts.limits) {
                let q = out.rotation(bone_i);
                out.set_rotation(bone_i, clamp_to_limit(q, &l));
            }
        }
    }
    let end = rig.joint_positions(&out)[t];
    let error = end.distance(target);
    IkResult { pose: out, reached: error < 1e-3, error }
}

/// Foot lock: refaz as pernas para que os tornozelos voltem a `targets` (mundo) com o joelho para onde já estava.
pub fn lock_feet(rig: &PoseRig, pose: &PoseState, targets: &[(Limb, Vec3)], opts: &IkOptions) -> IkResult {
    let mut cur = pose.clone();
    let mut worst = 0.0f32;
    for (limb, target) in targets {
        let e = eval(rig, &cur);
        let knee = e.pos[limb.chain()[1]];
        // pole = o joelho atual empurrado para a frente do plano quadril→tornozelo (mantém o lado da dobra)
        let pole = knee + Vec3::Z * 0.1;
        let r = solve_limb(rig, &cur, *limb, *target, Some(pole), opts);
        worst = worst.max(r.error);
        cur = r.pose;
    }
    IkResult { pose: cur, reached: worst < 1e-3, error: worst }
}

// ---------------------------------------------------------------------------------------------
// FABRIK
// ---------------------------------------------------------------------------------------------

/// FABRIK sobre posições: preserva os comprimentos dos segmentos, mantém `pts[0]` fixo e leva o último ponto a
/// `target` (ou o mais perto possível, esticando a cadeia na direção dele).
pub fn fabrik(pts: &[Vec3], target: Vec3, iterations: usize, tolerance: f32) -> Vec<Vec3> {
    let n = pts.len();
    if n < 2 || !target.is_finite() {
        return pts.to_vec();
    }
    let lens: Vec<f32> = pts.windows(2).map(|w| w[0].distance(w[1])).collect();
    let total: f32 = lens.iter().sum();
    let root = pts[0];
    let mut p = pts.to_vec();
    if root.distance(target) >= total {
        // fora do alcance: estica em direção ao alvo
        let dir = (target - root).normalize_or_zero();
        for i in 1..n {
            p[i] = p[i - 1] + dir * lens[i - 1];
        }
        return p;
    }
    for _ in 0..iterations.max(1) {
        // backward: do efetuador para a raiz
        p[n - 1] = target;
        for i in (0..n - 1).rev() {
            let dir = (p[i] - p[i + 1]).normalize_or_zero();
            p[i] = p[i + 1] + dir * lens[i];
        }
        // forward: da raiz para o efetuador
        p[0] = root;
        for i in 1..n {
            let dir = (p[i] - p[i - 1]).normalize_or_zero();
            p[i] = p[i - 1] + dir * lens[i - 1];
        }
        if p[n - 1].distance(target) < tolerance {
            break;
        }
    }
    p
}

/// Cadeia de ossos (em ordem raiz→ponta, cada um filho do anterior) resolvida com FABRIK e convertida em
/// rotações: cada osso gira pelo arco mínimo que alinha seu segmento ao novo. O último osso conserva a orientação de mundo.
pub fn solve_chain(rig: &PoseRig, pose: &PoseState, chain: &[usize], target: Vec3, opts: &IkOptions) -> IkResult {
    if chain.len() < 2 {
        return IkResult { pose: pose.clone(), reached: false, error: f32::INFINITY };
    }
    let e0 = eval(rig, pose);
    let pts: Vec<Vec3> = chain.iter().map(|b| e0.pos[*b]).collect();
    let solved = fabrik(&pts, target, 24, 1e-4);
    let mut out = pose.clone();
    let tip_world = e0.rot[*chain.last().unwrap_or(&0)];
    for i in 0..chain.len() - 1 {
        let e = eval(rig, &out);
        let bone_i = chain[i];
        let cur = e.pos[chain[i + 1]] - e.pos[bone_i];
        let want = solved[i + 1] - solved[i];
        let delta = arc(cur, want);
        set_world_rotation(&mut out, &e, bone_i, delta * e.rot[bone_i]);
    }
    if opts.keep_end_orientation {
        let e = eval(rig, &out);
        let tip = *chain.last().unwrap_or(&0);
        set_world_rotation(&mut out, &e, tip, tip_world);
    }
    if opts.limits.scale().is_some() {
        for &b in chain {
            if let Some(l) = bone_limit(b, opts.limits) {
                let q = out.rotation(b);
                out.set_rotation(b, clamp_to_limit(q, &l));
            }
        }
    }
    let end = rig.joint_positions(&out)[*chain.last().unwrap_or(&0)];
    let error = end.distance(target);
    IkResult { pose: out, reached: error < 1e-3, error }
}

/// Cadeia do tronco até a cabeça: quadril → coluna → peito → peito superior → pescoço → cabeça.
pub const SPINE_CHAIN: [usize; 6] = [bone::HIPS, bone::SPINE, bone::CHEST, bone::UPPER_CHEST, bone::NECK, bone::HEAD];

/// Cadeia da coluna sem o quadril (o quadril fica onde está; só tronco/pescoço/cabeça seguem o alvo).
pub const UPPER_BODY_CHAIN: [usize; 5] = [bone::SPINE, bone::CHEST, bone::UPPER_CHEST, bone::NECK, bone::HEAD];

// ---------------------------------------------------------------------------------------------
// Olhar
// ---------------------------------------------------------------------------------------------

/// Posição do "olho" (entre os olhos) em mundo: a junta da cabeça + deslocamento local do rosto.
pub fn eye_center(rig: &PoseRig, pose: &PoseState) -> Vec3 {
    let e = eval(rig, pose);
    e.pos[bone::HEAD] + e.rot[bone::HEAD] * Vec3::new(0.0, 0.11, 0.09)
}

/// Direção para a qual a face aponta (eixo +Z local da cabeça) em mundo.
pub fn face_direction(rig: &PoseRig, pose: &PoseState) -> Vec3 {
    let e = eval(rig, pose);
    e.rot[bone::HEAD] * Vec3::Z
}

/// Rotação sem *roll* que aponta +Z para `dir` mantendo o "para cima" do mundo (+Y).
fn look_rotation(dir: Vec3) -> Quat {
    let f = dir.normalize_or_zero();
    if f == Vec3::ZERO {
        return Quat::IDENTITY;
    }
    let up = if f.y.abs() > 0.999 { Vec3::Z } else { Vec3::Y };
    let right = up.cross(f).normalize();
    let upv = f.cross(right);
    Quat::from_mat3(&Mat3::from_cols(right, upv, f)).normalize()
}

/// Cabeça e pescoço olham para `target`. `weight` (0–1) mistura com a pose atual; `neck_share` é a parcela do
/// pescoço (o resto é da cabeça). Os limites do perfil recortam o resultado (alvo atrás do personagem não gira 180°).
pub fn look_at_head(rig: &PoseRig, pose: &PoseState, target: Vec3, weight: f32, neck_share: f32, limits: LimitProfile) -> IkResult {
    let e0 = eval(rig, pose);
    let w = weight.clamp(0.0, 1.0);
    let share = neck_share.clamp(0.0, 1.0);
    let mut out = pose.clone();
    // o "olho" se desloca quando a cabeça gira: 4 refinamentos convergem para <0,1°
    for _ in 0..4 {
        let eye = eye_center(rig, &out);
        let want = look_rotation(target - eye);
        let head_goal = e0.rot[bone::HEAD].slerp(want, w);
        let neck_goal = e0.rot[bone::NECK].slerp(want, w * share);
        set_world_rotation(&mut out, &e0, bone::NECK, neck_goal);
        out.set_rotation(bone::HEAD, (neck_goal.inverse() * head_goal).normalize());
    }
    if limits.scale().is_some() {
        for b in [bone::NECK, bone::HEAD] {
            if let Some(l) = bone_limit(b, limits) {
                let q = out.rotation(b);
                out.set_rotation(b, clamp_to_limit(q, &l));
            }
        }
    }
    // erro angular entre a face e o alvo (rad → m a 1 m só para um número comparável)
    let f = face_direction(rig, &out);
    let eye2 = eye_center(rig, &out);
    let ang = f.angle_between((target - eye2).normalize_or_zero());
    IkResult { pose: out, reached: ang < 0.02, error: ang }
}

#[cfg(test)]
mod tests;
