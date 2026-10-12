//! Geometria (mundo) do gizmo de direção da luz: esfera de direção com paralelos/meridianos, anel do
//! horizonte, arcos de azimute e elevação e o raio de luz. Só pontos 3D — a UI projeta e desenha.

use super::dir_from_az_el;
use glam::Vec3;

/// Papel de uma polilinha (a UI escolhe cor/espessura por papel).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// Paralelo (círculo de elevação constante) ou meridiano (grande círculo por Y).
    Grid,
    /// Equador (elevação 0°): o horizonte do personagem.
    Horizon,
    /// Arco no plano do horizonte da frente (+Z) até o azimute da luz.
    AzimuthArc,
    /// Arco vertical do horizonte até a elevação da luz.
    ElevationArc,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Polyline {
    pub kind: LineKind,
    pub points: Vec<Vec3>,
}

const SEGMENTS: usize = 72;

fn circle_at_elevation(center: Vec3, r: f32, el_deg: f32) -> Vec<Vec3> {
    (0..=SEGMENTS).map(|i| center + dir_from_az_el(i as f32 * 360.0 / SEGMENTS as f32, el_deg) * r).collect()
}

/// Grande círculo que contém o eixo Y e a direção horizontal de azimute `az_deg` (um meridiano e o oposto).
fn meridian(center: Vec3, r: f32, az_deg: f32) -> Vec<Vec3> {
    let h = dir_from_az_el(az_deg, 0.0);
    (0..=SEGMENTS)
        .map(|i| {
            let t = i as f32 * std::f32::consts::TAU / SEGMENTS as f32;
            center + (h * t.cos() + Vec3::Y * t.sin()) * r
        })
        .collect()
}

/// Arco no equador do azimute 0° (frente) até `az_deg` pelo lado curto.
pub fn azimuth_arc(center: Vec3, r: f32, az_deg: f32) -> Vec<Vec3> {
    let n = ((az_deg.abs() / 5.0).ceil() as usize).max(1);
    (0..=n).map(|i| center + dir_from_az_el(az_deg * i as f32 / n as f32, 0.0) * r).collect()
}

/// Arco vertical no azimute `az_deg` do horizonte (el 0°) até `el_deg`.
pub fn elevation_arc(center: Vec3, r: f32, az_deg: f32, el_deg: f32) -> Vec<Vec3> {
    let n = ((el_deg.abs() / 5.0).ceil() as usize).max(1);
    (0..=n).map(|i| center + dir_from_az_el(az_deg, el_deg * i as f32 / n as f32) * r).collect()
}

/// Todas as polilinhas do gizmo para a luz em (`az_deg`, `el_deg`).
pub fn sphere_lines(center: Vec3, r: f32, az_deg: f32, el_deg: f32) -> Vec<Polyline> {
    let mut out = Vec::new();
    for el in [-60.0, -30.0, 30.0, 60.0] {
        out.push(Polyline { kind: LineKind::Grid, points: circle_at_elevation(center, r, el) });
    }
    for az in [0.0, 30.0, 60.0, 90.0, 120.0, 150.0] {
        out.push(Polyline { kind: LineKind::Grid, points: meridian(center, r, az) });
    }
    out.push(Polyline { kind: LineKind::Horizon, points: circle_at_elevation(center, r, 0.0) });
    // os arcos ficam ligeiramente DENTRO da esfera para não brigarem com o horizonte
    let ra = r * 0.96;
    out.push(Polyline { kind: LineKind::AzimuthArc, points: azimuth_arc(center, ra, az_deg) });
    out.push(Polyline { kind: LineKind::ElevationArc, points: elevation_arc(center, ra, az_deg, el_deg) });
    out
}

/// Posição do disco do sol na esfera.
pub fn sun_position(center: Vec3, r: f32, dir: Vec3) -> Vec3 {
    center + dir.normalize_or_zero() * r
}

/// Pontas do raio de luz: do disco até o centro (a luz viaja do sol para o personagem). Devolve
/// (início, fim) e a base da ponta de seta a `head` metros do fim.
pub fn light_ray(center: Vec3, r: f32, dir: Vec3, head: f32) -> (Vec3, Vec3, Vec3) {
    let d = dir.normalize_or_zero();
    let start = center + d * r;
    let end = center + d * (r * 0.18);
    let base = end + d * head;
    (start, end, base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_point_lies_on_the_sphere() {
        let c = Vec3::new(1.0, 2.0, -3.0);
        let r = 1.7;
        for line in sphere_lines(c, r, 35.0, 40.0) {
            let rr = if matches!(line.kind, LineKind::AzimuthArc | LineKind::ElevationArc) { r * 0.96 } else { r };
            assert!(line.points.len() >= 2);
            for p in &line.points {
                assert!(((*p - c).length() - rr).abs() < 1e-4, "{:?} fora da esfera", line.kind);
            }
        }
    }

    #[test]
    fn parallels_have_constant_elevation_and_the_horizon_is_flat() {
        let c = Vec3::ZERO;
        let lines = sphere_lines(c, 1.0, 0.0, 0.0);
        let horizon = lines.iter().find(|l| l.kind == LineKind::Horizon).unwrap();
        assert!(horizon.points.iter().all(|p| p.y.abs() < 1e-5));
        let grid: Vec<_> = lines.iter().filter(|l| l.kind == LineKind::Grid).collect();
        assert_eq!(grid.len(), 4 + 6);
        // o primeiro paralelo é a elevação -60°
        assert!(grid[0].points.iter().all(|p| (p.y + 0.8660254).abs() < 1e-4));
    }

    #[test]
    fn azimuth_and_elevation_arcs_end_at_the_light() {
        let c = Vec3::new(0.0, 1.0, 0.0);
        let (az, el) = (50.0, 35.0);
        let a = azimuth_arc(c, 1.0, az);
        assert!((a[0] - (c + Vec3::Z)).length() < 1e-5, "o arco de azimute nasce na frente (+Z)");
        let end = *a.last().unwrap();
        assert!((end - (c + dir_from_az_el(az, 0.0))).length() < 1e-5);
        let e = elevation_arc(c, 1.0, az, el);
        assert!((e[0] - (c + dir_from_az_el(az, 0.0))).length() < 1e-5);
        assert!((*e.last().unwrap() - (c + dir_from_az_el(az, el))).length() < 1e-5);
        // negativo: vai pelo lado curto
        let neg = azimuth_arc(c, 1.0, -50.0);
        assert!(neg.iter().all(|p| p.x <= 1e-5));
        // arco degenerado (0°) ainda tem 2 pontos
        assert!(azimuth_arc(c, 1.0, 0.0).len() >= 2);
    }

    #[test]
    fn sun_and_ray_follow_the_direction() {
        let c = Vec3::ZERO;
        let d = dir_from_az_el(30.0, 45.0);
        assert!((sun_position(c, 2.0, d) - d * 2.0).length() < 1e-5);
        let (s, e, b) = light_ray(c, 2.0, d, 0.2);
        assert!((s - d * 2.0).length() < 1e-5);
        assert!(e.length() < s.length(), "o raio aponta para o centro");
        assert!(((b - e).length() - 0.2).abs() < 1e-5);
    }
}
