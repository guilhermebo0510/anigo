//! Colocação de ativos soltos da Biblioteca no viewport: raio → superfície (ou plano do chão),
//! encaixe da base da caixa do ativo, snap de grade e alinhamento opcional à normal. Pura.

use super::bvh::SceneBvh;
use super::snap::{align_up_to_normal, place_on_surface, snap_value, LocalBox, SnapSettings};
use anigo_core::ids::NodeId;
use anigo_core::math::Transform;
use glam::{Quat, Vec3};

/// Onde o raio do cursor encontrou o mundo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DropTarget {
    pub point: Vec3,
    pub normal: Vec3,
    /// `true` quando acertou geometria da cena (senão é o plano do chão ou um ponto à frente).
    pub on_geometry: bool,
}

/// Alvo do soltar: geometria da cena → plano y = `floor_y` → ponto a `fallback_dist` à frente.
pub fn drop_target(scene: &SceneBvh, o: Vec3, d: Vec3, floor_y: f32, fallback_dist: f32, skip: impl Fn(&NodeId) -> bool) -> DropTarget {
    if let Some(h) = scene.raycast(o, d, 1e6, skip) {
        return DropTarget { point: h.point, normal: h.normal, on_geometry: true };
    }
    if d.y.abs() > 1e-5 {
        let t = (floor_y - o.y) / d.y;
        if t > 0.0 && t < 1e5 {
            return DropTarget { point: o + d * t, normal: Vec3::Y, on_geometry: false };
        }
    }
    DropTarget { point: o + d * fallback_dist, normal: Vec3::Y, on_geometry: false }
}

/// Transformação de um ativo solto em `target`.
///
/// * a BASE da caixa do ativo (centro da face de baixo) fica no ponto — independente de onde está a
///   origem do modelo;
/// * `snap.surface_on && snap.align_to_normal`: o "para cima" do ativo acompanha a normal;
/// * com a grade ligada, x/z do ponto caem na grade quando a superfície é (quase) horizontal.
pub fn place_asset(target: &DropTarget, local: &LocalBox, yaw: f32, scale: Vec3, snap: &SnapSettings) -> Transform {
    let mut point = target.point;
    let horizontal = target.normal.dot(Vec3::Y) > 0.85;
    if snap.grid_on && horizontal {
        point.x = snap_value(point.x, snap.grid_m);
        point.z = snap_value(point.z, snap.grid_m);
    }
    let mut rotation = Quat::from_rotation_y(yaw);
    let normal = if snap.surface_on && snap.align_to_normal {
        rotation = align_up_to_normal(rotation, target.normal);
        target.normal
    } else {
        Vec3::Y
    };
    let translation = place_on_surface(point, normal, rotation, scale, local, snap.surface_offset);
    Transform { translation, rotation, scale }
}

/// Posições relativas (em X) para soltar vários ativos lado a lado sem sobrepor: largura de cada
/// um + `gap`, centralizados em torno de zero.
pub fn row_offsets(widths: &[f32], gap: f32) -> Vec<f32> {
    let total: f32 = widths.iter().sum::<f32>() + gap * widths.len().saturating_sub(1) as f32;
    let mut x = -total * 0.5;
    widths
        .iter()
        .map(|w| {
            let c = x + w * 0.5;
            x += w + gap;
            c
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::bvh::{Instance, MeshBvh};
    use std::sync::Arc;

    fn floor_scene() -> SceneBvh {
        // "mesa": cubo 2x1x2 com topo em y = 1
        let cube = anigo_core::Mesh::create_cube(1.0);
        let bvh = Arc::new(MeshBvh::build(&cube));
        let m = glam::Mat4::from_scale_rotation_translation(Vec3::new(2.0, 1.0, 2.0), Quat::IDENTITY, Vec3::new(0.0, 0.5, 0.0));
        SceneBvh::new(vec![Instance::new(NodeId::from_slug("mesa"), m, bvh).unwrap()])
    }

    fn lbox() -> LocalBox {
        LocalBox { min: Vec3::new(-0.25, -0.5, -0.25), max: Vec3::new(0.25, 0.5, 0.25) }
    }

    #[test]
    fn dropping_on_empty_space_uses_the_ground_plane() {
        let scene = SceneBvh::default();
        let t = drop_target(&scene, Vec3::new(0.0, 5.0, 5.0), Vec3::new(0.0, -1.0, -1.0).normalize(), 0.0, 8.0, |_| false);
        assert!(!t.on_geometry && t.point.y.abs() < 1e-4 && (t.point.z - 0.0).abs() < 1e-3, "{t:?}");
        // olhando para cima: cai no ponto à frente, nunca em NaN
        let up = drop_target(&scene, Vec3::ZERO, Vec3::Y, 0.0, 8.0, |_| false);
        assert!((up.point - Vec3::new(0.0, 8.0, 0.0)).length() < 1e-4);
    }

    #[test]
    fn dropping_on_a_table_sits_the_base_on_its_top() {
        let scene = floor_scene();
        let t = drop_target(&scene, Vec3::new(0.2, 6.0, 0.1), Vec3::NEG_Y, 0.0, 8.0, |_| false);
        assert!(t.on_geometry && (t.point.y - 1.0).abs() < 1e-4 && t.normal.dot(Vec3::Y) > 0.99);
        let snap = SnapSettings { grid_on: false, ..Default::default() };
        let tr = place_asset(&t, &lbox(), 0.0, Vec3::ONE, &snap);
        // ativo de 1 m de altura, origem no centro → origem 0.5 m acima do topo
        assert!((tr.translation.y - 1.5).abs() < 1e-4, "{:?}", tr.translation);
        assert!((tr.translation.x - 0.2).abs() < 1e-4);
    }

    #[test]
    fn grid_snaps_horizontal_drops_only() {
        let snap = SnapSettings { grid_on: true, grid_m: 0.5, ..Default::default() };
        let floor = DropTarget { point: Vec3::new(1.37, 0.0, -0.8), normal: Vec3::Y, on_geometry: false };
        let tr = place_asset(&floor, &lbox(), 0.0, Vec3::ONE, &snap);
        assert!((tr.translation.x - 1.5).abs() < 1e-5 && (tr.translation.z + 1.0).abs() < 1e-5, "{:?}", tr.translation);
        let wall = DropTarget { point: Vec3::new(1.37, 1.0, -0.8), normal: Vec3::X, on_geometry: true };
        let tr = place_asset(&wall, &lbox(), 0.0, Vec3::ONE, &snap);
        assert!((tr.translation.z + 0.8).abs() < 1e-5, "parede não encaixa na grade horizontal");
    }

    #[test]
    fn surface_alignment_is_opt_in() {
        let wall = DropTarget { point: Vec3::new(3.0, 1.0, 0.0), normal: Vec3::X, on_geometry: true };
        let off = SnapSettings { grid_on: false, surface_on: false, ..Default::default() };
        let on = SnapSettings { grid_on: false, surface_on: true, align_to_normal: true, ..Default::default() };
        let a = place_asset(&wall, &lbox(), 0.0, Vec3::ONE, &off);
        let b = place_asset(&wall, &lbox(), 0.0, Vec3::ONE, &on);
        assert!((a.rotation * Vec3::Y - Vec3::Y).length() < 1e-5, "sem alinhar, fica em pé");
        assert!((b.rotation * Vec3::Y - Vec3::X).length() < 1e-4, "alinhado à parede");
        // com alinhamento a base encosta na parede (origem 0.5 m para fora)
        assert!((b.translation.x - 3.5).abs() < 1e-4, "{:?}", b.translation);
    }

    #[test]
    fn row_offsets_are_centred_and_non_overlapping() {
        let o = row_offsets(&[1.0, 2.0, 1.0], 0.5);
        assert_eq!(o.len(), 3);
        assert!(o.iter().sum::<f32>().abs() < 1e-4 || (o[0] + o[2]).abs() < 1e-4);
        assert!(o[1] - o[0] >= 1.5 - 1e-5 && o[2] - o[1] >= 1.5 - 1e-5);
        assert!(row_offsets(&[], 1.0).is_empty());
        assert_eq!(row_offsets(&[3.0], 1.0), vec![0.0]);
    }
}
