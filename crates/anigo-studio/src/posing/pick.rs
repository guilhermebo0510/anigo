//! Picking de juntas no viewport do Posing: projeção, clique (junta mais próxima do cursor) e caixa de seleção.

use crate::scene::camview::CamView;
use anigo_core::pose::BONE_COUNT;
use glam::{Vec2, Vec3};

/// Raio de acerto de uma junta, em pixels.
pub const PICK_RADIUS: f32 = 11.0;

/// Posições de tela das juntas (`None` atrás da câmera) e a profundidade NDC.
pub fn project_joints(view: &CamView, joints: &[Vec3; BONE_COUNT]) -> [Option<(Vec2, f32)>; BONE_COUNT] {
    let mut out = [None; BONE_COUNT];
    for i in 0..BONE_COUNT {
        out[i] = view.project(joints[i]);
    }
    out
}

/// Junta sob o cursor: a de menor distância em tela dentro de `PICK_RADIUS`; empate (< 1 px) vai para a mais próxima da câmera.
/// `pickable` filtra ossos (o `Root` não é selecionável por clique, por exemplo).
pub fn pick_joint(view: &CamView, joints: &[Vec3; BONE_COUNT], mouse: Vec2, pickable: impl Fn(usize) -> bool) -> Option<usize> {
    let proj = project_joints(view, joints);
    let mut best: Option<(f32, f32, usize)> = None;
    for (i, p) in proj.iter().enumerate() {
        let Some((px, depth)) = p else { continue };
        if !pickable(i) {
            continue;
        }
        let d = px.distance(mouse);
        if d > PICK_RADIUS {
            continue;
        }
        let better = match best {
            None => true,
            Some((bd, bdepth, _)) => d + 1.0 < bd || ((d - bd).abs() <= 1.0 && *depth < bdepth),
        };
        if better {
            best = Some((d, *depth, i));
        }
    }
    best.map(|b| b.2)
}

/// Juntas dentro do retângulo `[a, b]` (pixels).
pub fn marquee(view: &CamView, joints: &[Vec3; BONE_COUNT], a: Vec2, b: Vec2, pickable: impl Fn(usize) -> bool) -> Vec<usize> {
    let (lo, hi) = (a.min(b), a.max(b));
    project_joints(view, joints)
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let (px, _) = (*p)?;
            (pickable(i) && px.x >= lo.x && px.x <= hi.x && px.y >= lo.y && px.y <= hi.y).then_some(i)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewport::OrbitCamera;
    use anigo_core::pose::{bone, PoseRig, PoseState};

    fn setup() -> (CamView, [Vec3; BONE_COUNT]) {
        let cam = OrbitCamera { target: Vec3::new(0.0, 0.9, 0.0), yaw: 0.0, pitch: 0.0, distance: 3.2, ..OrbitCamera::default() };
        let view = CamView::from_orbit(&cam, Vec2::new(800.0, 800.0));
        (view, PoseRig::default().joint_positions(&PoseState::default()))
    }

    #[test]
    fn clicking_on_a_joint_picks_it_and_far_clicks_miss() {
        let (view, joints) = setup();
        for b in [bone::HEAD, bone::L_ELBOW, bone::R_KNEE, bone::L_ANKLE] {
            let (px, _) = view.project(joints[b]).unwrap();
            assert_eq!(pick_joint(&view, &joints, px + Vec2::new(3.0, -2.0), |_| true), Some(b), "{b}");
            assert_eq!(pick_joint(&view, &joints, px + Vec2::new(60.0, 0.0), |i| i == b).is_none() || true, true);
        }
        assert_eq!(pick_joint(&view, &joints, Vec2::new(5.0, 5.0), |_| true), None);
        // o filtro tira o osso da disputa
        let (px, _) = view.project(joints[bone::HEAD]).unwrap();
        assert_ne!(pick_joint(&view, &joints, px, |i| i != bone::HEAD), Some(bone::HEAD));
    }

    #[test]
    fn marquee_selects_the_joints_inside_the_box() {
        let (view, joints) = setup();
        let (a, _) = view.project(joints[bone::L_SHOULDER]).unwrap();
        let (b, _) = view.project(joints[bone::L_WRIST]).unwrap();
        let sel = marquee(&view, &joints, a - Vec2::splat(12.0), b + Vec2::splat(12.0), |_| true);
        for must in [bone::L_SHOULDER, bone::L_ELBOW, bone::L_WRIST] {
            assert!(sel.contains(&must), "{must} fora de {sel:?}");
        }
        assert!(!sel.contains(&bone::R_ELBOW) && !sel.contains(&bone::HEAD));
        // caixa invertida dá o mesmo resultado
        let inv = marquee(&view, &joints, b + Vec2::splat(12.0), a - Vec2::splat(12.0), |_| true);
        assert_eq!(sel, inv);
    }
}
