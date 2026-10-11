//! Gizmo de transformação (mover/rotacionar/escalar) desenhado sobre o viewport.
//! Geometria e hit-test são puros (testáveis); o desenho fica em `viewport_ui`.

use crate::modeling::{Axis, TransformKind};
use crate::viewport::OrbitCamera;
use glam::{Vec2, Vec3};

pub const HANDLE_RADIUS_PX: f32 = 9.0;
const RING_SEGMENTS: usize = 48;

/// Tamanho do gizmo em unidades de mundo para ter ~110 px na tela.
pub fn world_size(camera: &OrbitCamera, pivot: Vec3, viewport_h: f32) -> f32 {
    let dist = if camera.orthographic {
        camera.distance
    } else {
        (camera.eye() - pivot).dot(-camera.forward()).abs().max(0.05)
    };
    110.0 * 2.0 * dist * (camera.fov_y * 0.5).tan() / viewport_h.max(1.0)
}

/// Pontos (mundo) do anel de rotação de um eixo.
pub fn ring_points(pivot: Vec3, axis: Axis, radius: f32) -> Vec<Vec3> {
    let (u, v) = match axis {
        Axis::X => (Vec3::Y, Vec3::Z),
        Axis::Y => (Vec3::Z, Vec3::X),
        Axis::Z => (Vec3::X, Vec3::Y),
    };
    (0..=RING_SEGMENTS)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / RING_SEGMENTS as f32;
            pivot + (u * a.cos() + v * a.sin()) * radius
        })
        .collect()
}

fn dist_to_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let l2 = ab.length_squared();
    if l2 < 1e-9 {
        return p.distance(a);
    }
    p.distance(a + ab * ((p - a).dot(ab) / l2).clamp(0.0, 1.0))
}

/// Qual alça está sob o cursor? Move/Escala: ponta e haste do eixo; Rotação: anel.
pub fn hit_test(
    kind: TransformKind,
    camera: &OrbitCamera,
    viewport: Vec2,
    pivot: Vec3,
    mouse: Vec2,
) -> Option<Axis> {
    let size = world_size(camera, pivot, viewport.y);
    let (origin_px, _) = camera.project(pivot, viewport)?;
    let mut best: Option<(f32, Axis)> = None;
    for axis in [Axis::X, Axis::Y, Axis::Z] {
        let d = match kind {
            TransformKind::Grab | TransformKind::Scale => {
                let (end_px, _) = camera.project(pivot + axis.vector() * size, viewport)?;
                dist_to_segment(mouse, origin_px, end_px)
            }
            TransformKind::Rotate => {
                let pts = ring_points(pivot, axis, size * 0.9);
                let mut best_d = f32::MAX;
                let mut prev: Option<Vec2> = None;
                for p in pts {
                    if let Some((px, _)) = camera.project(p, viewport) {
                        if let Some(q) = prev {
                            best_d = best_d.min(dist_to_segment(mouse, q, px));
                        }
                        prev = Some(px);
                    } else {
                        prev = None;
                    }
                }
                best_d
            }
        };
        if d <= HANDLE_RADIUS_PX && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, axis));
        }
    }
    best.map(|(_, a)| a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> OrbitCamera {
        OrbitCamera {
            target: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            distance: 6.0,
            ..Default::default()
        }
    }

    const VP: Vec2 = Vec2::new(800.0, 600.0);

    #[test]
    fn gizmo_size_is_about_110_px_on_screen() {
        let c = cam();
        let s = world_size(&c, Vec3::ZERO, VP.y);
        let (a, _) = c.project(Vec3::ZERO, VP).unwrap();
        let (b, _) = c.project(Vec3::X * s, VP).unwrap();
        assert!((a.distance(b) - 110.0).abs() < 4.0, "{}", a.distance(b));
    }

    #[test]
    fn move_handle_hit_on_x_axis_shaft() {
        let c = cam();
        let s = world_size(&c, Vec3::ZERO, VP.y);
        let (mid, _) = c.project(Vec3::X * s * 0.6, VP).unwrap();
        assert_eq!(hit_test(TransformKind::Grab, &c, VP, Vec3::ZERO, mid), Some(Axis::X));
        let (up, _) = c.project(Vec3::Y * s * 0.7, VP).unwrap();
        assert_eq!(hit_test(TransformKind::Scale, &c, VP, Vec3::ZERO, up), Some(Axis::Y));
        assert_eq!(hit_test(TransformKind::Grab, &c, VP, Vec3::ZERO, Vec2::new(10.0, 10.0)), None);
    }

    #[test]
    fn rotate_ring_hit_for_z_axis() {
        let c = cam();
        let s = world_size(&c, Vec3::ZERO, VP.y);
        // anel Z fica no plano XY, de frente para a câmera frontal
        let d = s * 0.9 * std::f32::consts::FRAC_1_SQRT_2;
        let p = Vec3::new(d, d, 0.0); // (r,0,0) pertence aos anéis Y e Z; 45° é exclusivo do Z
        let (px, _) = c.project(p, VP).unwrap();
        assert_eq!(hit_test(TransformKind::Rotate, &c, VP, Vec3::ZERO, px), Some(Axis::Z));
    }

    #[test]
    fn ring_points_are_equidistant() {
        for p in ring_points(Vec3::new(1.0, 2.0, 3.0), Axis::Y, 2.0) {
            assert!(((p - Vec3::new(1.0, 2.0, 3.0)).length() - 2.0).abs() < 1e-5);
        }
    }
}
