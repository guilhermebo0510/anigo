//! Visualização dos limites articulares: o cone de swing (onde a ponta do osso pode ir) e as faixas
//! swing/twist do osso ativo. Tudo calculado a partir do MESMO `BoneLimit` que o núcleo usa para limitar.

use anigo_core::pose::{bone, bone_children, bone_limit, swing_twist, within_limit, BoneLimit, LimitProfile, PoseRig, PoseState, BONE_PARENTS};
use glam::{Quat, Vec3};

/// Cone de swing de um osso, em espaço de mundo.
#[derive(Debug, Clone, PartialEq)]
pub struct LimitCone {
    /// Origem do cone (junta do osso, posada).
    pub origin: Vec3,
    /// Pontos da borda (direções unitárias de mundo da ponta do osso quando o swing está no limite).
    pub boundary: Vec<Vec3>,
    /// Direção atual da ponta do osso (unitária, mundo).
    pub current: Vec3,
    /// A rotação atual respeita o limite.
    pub inside: bool,
    pub limit: BoneLimit,
}

/// Direção de repouso do osso no espaço do pai: do osso ao filho principal (ou do pai ao osso nas folhas).
pub fn rest_direction(rig: &PoseRig, bone_i: usize) -> Vec3 {
    let tip = match bone_i {
        bone::HIPS => Some(bone::SPINE),
        bone::L_ANKLE => Some(bone::L_TOES),
        bone::R_ANKLE => Some(bone::R_TOES),
        _ => bone_children(bone_i).first().copied(),
    };
    let d = match tip {
        Some(t) => rig.bind[t] - rig.bind[bone_i],
        None => BONE_PARENTS[bone_i].map(|p| rig.bind[bone_i] - rig.bind[p]).unwrap_or(Vec3::Y),
    };
    let n = d.normalize_or_zero();
    if n == Vec3::ZERO {
        Vec3::Y
    } else {
        n
    }
}

/// Cone de limites do osso na pose `pose` (`None` para ossos sem limite no perfil).
pub fn limit_cone(rig: &PoseRig, pose: &PoseState, bone_i: usize, profile: LimitProfile, segments: usize) -> Option<LimitCone> {
    let limit = bone_limit(bone_i, profile)?;
    let world = rig.world_rotations(pose);
    let joints = rig.joint_positions(pose);
    let parent = BONE_PARENTS[bone_i].map(|p| world[p]).unwrap_or(Quat::IDENTITY);
    let rest = rest_direction(rig, bone_i);
    let n = segments.max(8);
    let mut boundary = Vec::with_capacity(n + 1);
    for k in 0..=n {
        let t = std::f32::consts::TAU * k as f32 / n as f32;
        let (u, v) = (t.cos(), t.sin());
        let tx = u * if u >= 0.0 { limit.sx[1] } else { -limit.sx[0] };
        let tz = v * if v >= 0.0 { limit.sz[1] } else { -limit.sz[0] };
        let (tx, tz) = (tx.to_radians(), tz.to_radians());
        let a = (tx * tx + tz * tz).sqrt();
        let swing = if a > 1e-6 { Quat::from_axis_angle(Vec3::new(tx, 0.0, tz) / a, a) } else { Quat::IDENTITY };
        boundary.push((parent * swing * rest).normalize());
    }
    let local = pose.rotation(bone_i);
    let current = (parent * local * rest).normalize();
    Some(LimitCone { origin: joints[bone_i], boundary, current, inside: within_limit(local, &limit), limit })
}

/// Valores atuais de swing X, swing Z e twist (graus) para as barras do painel.
pub fn swing_twist_deg(pose: &PoseState, bone_i: usize) -> [f32; 3] {
    let (tx, tz, tw) = swing_twist(pose.rotation(bone_i));
    [tx.to_degrees(), tz.to_degrees(), tw.to_degrees()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::pose::{clamp_to_limit, BONE_COUNT};

    #[test]
    fn the_cone_boundary_is_the_edge_of_the_legal_directions() {
        let rig = PoseRig::default();
        let rest = PoseState::default();
        for b in 0..BONE_COUNT {
            let Some(c) = limit_cone(&rig, &rest, b, LimitProfile::Human, 48) else { continue };
            assert!(c.inside, "{b}: repouso sempre dentro");
            assert!(c.boundary.iter().all(|d| d.is_finite() && (d.length() - 1.0).abs() < 1e-4));
            // para ossos cuja direção de repouso é ±Y (swing puro): o arco mínimo até um ponto da borda é legal;
            // um pouco além da borda não é
            let r = rest_direction(&rig, b);
            if r.y.abs() < 0.99 || c.limit.sx.iter().chain(c.limit.sz.iter()).any(|v| v.abs() >= 150.0) {
                continue; // arcos mínimos são ambíguos perto de 180°
            }
            let parent = BONE_PARENTS[b].map(|p| rig.world_rotations(&rest)[p]).unwrap_or(Quat::IDENTITY);
            for dir in c.boundary.iter().step_by(5) {
                let target = parent.inverse() * *dir;
                let edge = Quat::from_rotation_arc(r, target);
                assert!(within_limit(edge, &c.limit) || clamp_to_limit(edge, &c.limit).dot(edge).abs() > 0.99999, "{b}: borda ilegal");
                let beyond = Quat::from_axis_angle(edge.to_axis_angle().0, edge.to_axis_angle().1 * 1.25);
                if edge.to_axis_angle().1 > 0.2 && c.limit.twist[1] > 0.0 {
                    assert!(!within_limit(beyond, &c.limit), "{b}: 25% além da borda deveria ser ilegal");
                }
            }
        }
        // cotovelo: dobrar 90° para a frente fica dentro; ultrapassar 150° sai
        let mut p = PoseState::default();
        p.set_rotation(bone::L_ELBOW, Quat::from_rotation_x(-90f32.to_radians()));
        let c = limit_cone(&rig, &p, bone::L_ELBOW, LimitProfile::Human, 32).unwrap();
        assert!(c.inside);
        p.set_rotation(bone::L_ELBOW, Quat::from_rotation_x(-170f32.to_radians()));
        let c = limit_cone(&rig, &p, bone::L_ELBOW, LimitProfile::Human, 32).unwrap();
        assert!(!c.inside);
        // sem limite no perfil livre e na raiz
        assert!(limit_cone(&rig, &p, bone::L_ELBOW, LimitProfile::Free, 32).is_none());
        assert!(limit_cone(&rig, &p, bone::ROOT, LimitProfile::Human, 32).is_none());
    }

    #[test]
    fn current_direction_matches_the_posed_bone() {
        let rig = PoseRig::default();
        let mut p = PoseState::default();
        p.set_rotation(bone::L_SHOULDER, Quat::from_rotation_z(80f32.to_radians()));
        let c = limit_cone(&rig, &p, bone::L_SHOULDER, LimitProfile::Human, 32).unwrap();
        let j = rig.joint_positions(&p);
        let along = (j[bone::L_ELBOW] - j[bone::L_SHOULDER]).normalize();
        assert!(c.current.dot(along) > 0.9999, "{:?} vs {along:?}", c.current);
        assert!(c.origin.distance(j[bone::L_SHOULDER]) < 1e-6);
        // o ponto da borda mais próximo do braço está a poucos graus de distância: 80° < 180° de abdução
        assert!(c.inside);
        let lim = c.limit;
        let clamped = clamp_to_limit(Quat::from_rotation_z(-120f32.to_radians()), &lim);
        assert!(within_limit(clamped, &lim));
        let d = swing_twist_deg(&p, bone::L_SHOULDER);
        assert!((d[1] - 80.0).abs() < 1e-3 && d[0].abs() < 1e-3 && d[2].abs() < 1e-3);
    }
}
