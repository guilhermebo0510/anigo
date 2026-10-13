//! SNAP do Cenário (algoritmos puros, sem egui): grade de posição/rotação/escala, incremental ×
//! absoluto, snap ao chão (tecla End) e alinhamento à superfície (Z).
//!
//! Unidades: tudo em **metros** internamente; a UI formata pela `LengthUnit` do projeto.

use anigo_core::project::LengthUnit;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Passos de grade de posição oferecidos (metros). Em cm: 1, 5, 10, 25, 50, 100, 200, 500, 1000.
pub const GRID_STEPS_M: [f32; 9] = [0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0];
/// Passos de rotação (graus).
pub const ROT_STEPS_DEG: [f32; 8] = [5.0, 10.0, 15.0, 22.5, 30.0, 45.0, 90.0, 180.0];
/// Passos de escala (aditivos).
pub const SCALE_STEPS: [f32; 6] = [0.0625, 0.125, 0.25, 0.5, 1.0, 10.0];

/// Texto de um comprimento na unidade do projeto (`0.5 m`, `50 cm`).
pub fn format_length(meters: f32, unit: LengthUnit) -> String {
    match unit {
        LengthUnit::Meters => format!("{} m", trim_float(meters)),
        LengthUnit::Centimeters => format!("{} cm", trim_float(meters * 100.0)),
    }
}

fn trim_float(v: f32) -> String {
    let s = format!("{:.3}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SnapSettings {
    pub grid_on: bool,
    pub grid_m: f32,
    pub rot_on: bool,
    pub rot_deg: f32,
    pub scale_on: bool,
    pub scale_step: f32,
    /// Surface snapping: arrastar pelo centro cola o objeto na superfície sob o cursor.
    pub surface_on: bool,
    /// Com surface snapping, alinha o "para cima" do objeto à normal da superfície.
    pub align_to_normal: bool,
    /// Distância entre o objeto e a superfície (m).
    pub surface_offset: f32,
    /// Absoluto (a pose final cai na grade) × incremental (o deslocamento cai na grade).
    pub absolute: bool,
}

impl Default for SnapSettings {
    fn default() -> Self {
        Self {
            grid_on: true,
            // 10 cm como no Unreal: com 0,5 m o objeto "pulava" de meio em meio metro ao arrastar (parecia travado)
            grid_m: 0.1,
            rot_on: true,
            rot_deg: 15.0,
            scale_on: false,
            scale_step: 0.25,
            surface_on: false,
            align_to_normal: true,
            surface_offset: 0.0,
            absolute: false,
        }
    }
}

impl SnapSettings {
    /// Configuração efetiva: `invert` (Ctrl segurado) liga o que está desligado e vice-versa.
    pub fn effective(&self, invert: bool) -> SnapSettings {
        if !invert {
            return *self;
        }
        SnapSettings { grid_on: !self.grid_on, rot_on: !self.rot_on, scale_on: !self.scale_on, ..*self }
    }
}

/// `round(v / step) * step` sem produzir `-0.0`.
pub fn snap_value(v: f32, step: f32) -> f32 {
    if step <= 1e-9 || !v.is_finite() {
        return v;
    }
    let r = (v / step).round() * step;
    if r == 0.0 {
        0.0
    } else {
        r
    }
}

/// Snap de um deslocamento `delta` expresso numa base ortonormal `basis` (1 a 3 vetores).
///
/// * incremental: cada coordenada do deslocamento na base cai na grade;
/// * absoluto: se a base é alinhada ao mundo, a posição **final** (`start + delta`) cai na grade nas
///   componentes cobertas pela base; bases rotacionadas caem para incremental (não existe grade
///   do mundo nelas).
pub fn snap_translation_delta(start: Vec3, delta: Vec3, basis: &[Vec3], step: f32, absolute: bool) -> Vec3 {
    if step <= 1e-9 {
        return delta;
    }
    let world_aligned = basis.iter().all(|b| {
        let a = b.abs();
        (a.x > 0.9999 && a.y < 1e-3 && a.z < 1e-3) || (a.y > 0.9999 && a.x < 1e-3 && a.z < 1e-3) || (a.z > 0.9999 && a.x < 1e-3 && a.y < 1e-3)
    });
    if absolute && world_aligned {
        let mut out = delta;
        for b in basis {
            // eixo do mundo que esta base cobre
            let axis = if b.x.abs() > 0.5 {
                0
            } else if b.y.abs() > 0.5 {
                1
            } else {
                2
            };
            let target = snap_value(start[axis] + delta[axis], step);
            out[axis] = target - start[axis];
        }
        return out;
    }
    let mut out = Vec3::ZERO;
    for b in basis {
        out += *b * snap_value(delta.dot(*b), step);
    }
    out
}

/// Snap do ângulo ACUMULADO do arrasto (graus → radianos de saída), não do euler resultante.
pub fn snap_angle(angle_rad: f32, step_deg: f32) -> f32 {
    if step_deg <= 1e-6 {
        return angle_rad;
    }
    snap_value(angle_rad.to_degrees(), step_deg).to_radians()
}

/// Snap de escala: `absolute` = o valor final cai na grade; senão `s0 + round(Δ/step)·step` (aditivo).
pub fn snap_scale_value(start: f32, new: f32, step: f32, absolute: bool) -> f32 {
    if step <= 1e-9 {
        return new;
    }
    let v = if absolute { snap_value(new, step) } else { start + snap_value(new - start, step) };
    v.max(step.min(0.01))
}

/// Rotação que alinha o "para cima" (+Y local) do objeto à normal da superfície, com o menor giro
/// possível (o rumo do objeto é preservado).
pub fn align_up_to_normal(rotation: Quat, normal: Vec3) -> Quat {
    let n = normal.normalize_or_zero();
    if n == Vec3::ZERO {
        return rotation;
    }
    let up = rotation * Vec3::Y;
    if up.dot(n) > 0.999_999 {
        return rotation;
    }
    if up.dot(n) < -0.999_999 {
        // oposto exato: gira 180 graus em torno do eixo local X
        return (Quat::from_axis_angle(rotation * Vec3::X, std::f32::consts::PI) * rotation).normalize();
    }
    (Quat::from_rotation_arc(up, n) * rotation).normalize()
}

/// Caixa local do ativo (mínimo, máximo).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalBox {
    pub min: Vec3,
    pub max: Vec3,
}

impl LocalBox {
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// Centro da base (face de baixo): o ponto que "senta" no chão.
    pub fn bottom_center(&self) -> Vec3 {
        Vec3::new((self.min.x + self.max.x) * 0.5, self.min.y, (self.min.z + self.max.z) * 0.5)
    }

    /// Os 8 cantos.
    pub fn corners(&self) -> [Vec3; 8] {
        let (a, b) = (self.min, self.max);
        [
            Vec3::new(a.x, a.y, a.z),
            Vec3::new(b.x, a.y, a.z),
            Vec3::new(b.x, a.y, b.z),
            Vec3::new(a.x, a.y, b.z),
            Vec3::new(a.x, b.y, a.z),
            Vec3::new(b.x, b.y, a.z),
            Vec3::new(b.x, b.y, b.z),
            Vec3::new(a.x, b.y, b.z),
        ]
    }
}

/// Translação do nó para que o CENTRO DA BASE do ativo toque `hit` (mais `offset` ao longo de `normal`).
/// A rotação/escala já devem estar definidas (inclusive o alinhamento à normal).
pub fn place_on_surface(hit: Vec3, normal: Vec3, rotation: Quat, scale: Vec3, local: &LocalBox, offset: f32) -> Vec3 {
    let base = rotation * (local.bottom_center() * scale);
    hit + normal.normalize_or_zero() * offset - base
}

/// Os 5 pontos da base de uma caixa mundial alinhada aos eixos (centro + 4 cantos), na altura `y`.
pub fn base_points(lo: Vec3, hi: Vec3) -> [Vec3; 5] {
    let y = lo.y;
    let c = Vec3::new((lo.x + hi.x) * 0.5, y, (lo.z + hi.z) * 0.5);
    [c, Vec3::new(lo.x, y, lo.z), Vec3::new(hi.x, y, lo.z), Vec3::new(hi.x, y, hi.z), Vec3::new(lo.x, y, hi.z)]
}

/// Snap ao chão (tecla End): raios para -Y a partir dos 5 pontos da base; devolve o deslocamento
/// vertical (negativo = desce). `cast(origin, dir)` devolve a distância `t` do primeiro acerto.
///
/// O objeto "senta" sem atravessar: vale o MENOR deslocamento não negativo entre os acertos de cena e
/// o plano `floor_y`. Sem nenhum candidato não negativo (objeto abaixo do piso), sobe até o piso.
pub fn ground_snap_delta(points: &[Vec3], floor_y: f32, mut cast: impl FnMut(Vec3, Vec3) -> Option<f32>) -> f32 {
    const EPS: f32 = 0.002;
    let mut best: Option<f32> = None;
    let mut plane_fallback: Option<f32> = None;
    for p in points {
        // piso lógico
        let d_plane = p.y - floor_y;
        if d_plane >= 0.0 {
            best = Some(best.map_or(d_plane, |b: f32| b.min(d_plane)));
        } else {
            plane_fallback = Some(plane_fallback.map_or(d_plane, |b: f32| b.max(d_plane)));
        }
        // geometria da cena
        if let Some(t) = cast(*p + Vec3::Y * EPS, Vec3::NEG_Y) {
            let d = (t - EPS).max(0.0);
            best = Some(best.map_or(d, |b: f32| b.min(d)));
        }
    }
    match (best, plane_fallback) {
        (Some(d), _) => -d,
        (None, Some(up)) => -up,
        (None, None) => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_value_rounds_to_the_grid_without_negative_zero() {
        assert_eq!(snap_value(0.74, 0.5), 0.5);
        assert_eq!(snap_value(0.76, 0.5), 1.0);
        assert_eq!(snap_value(-0.24, 0.5).to_bits(), 0.0f32.to_bits());
        assert_eq!(snap_value(1.3, 0.0), 1.3, "passo zero desliga");
        assert!(snap_value(f32::NAN, 0.5).is_nan());
    }

    #[test]
    fn incremental_snap_moves_in_whole_steps_from_the_start() {
        // objeto fora da grade (1.23): incremental mantém o desvio, absoluto cai na grade
        let start = Vec3::new(1.23, 0.0, 0.0);
        let d = snap_translation_delta(start, Vec3::new(0.74, 0.0, 0.0), &[Vec3::X], 0.5, false);
        assert!((d.x - 0.5).abs() < 1e-6, "{d:?}");
        let d = snap_translation_delta(start, Vec3::new(0.74, 0.0, 0.0), &[Vec3::X], 0.5, true);
        assert!(((start.x + d.x) - 2.0).abs() < 1e-5, "posição final na grade: {}", start.x + d.x);
    }

    #[test]
    fn absolute_snap_only_touches_the_constrained_axes() {
        let start = Vec3::new(0.3, 0.3, 0.3);
        let delta = Vec3::new(0.9, 0.0, 0.9);
        let d = snap_translation_delta(start, delta, &[Vec3::X, Vec3::Z], 0.5, true);
        assert!(((start + d).x - 1.0).abs() < 1e-5 && ((start + d).z - 1.0).abs() < 1e-5);
        assert_eq!(d.y, 0.0, "o eixo Y não foi arrastado");
    }

    #[test]
    fn local_space_basis_snaps_incrementally_along_its_own_axes() {
        let yaw = Quat::from_rotation_y(0.6);
        let axis = yaw * Vec3::X;
        let d = snap_translation_delta(Vec3::ZERO, axis * 1.3, &[axis], 0.5, true);
        assert!((d.dot(axis) - 1.5).abs() < 1e-5, "{}", d.dot(axis));
        assert!((d - axis * d.dot(axis)).length() < 1e-5, "continua sobre o eixo local");
    }

    #[test]
    fn angle_snap_acts_on_the_accumulated_angle() {
        assert!((snap_angle(37f32.to_radians(), 15.0).to_degrees() - 30.0).abs() < 1e-4);
        assert!((snap_angle(38f32.to_radians(), 15.0).to_degrees() - 45.0).abs() < 1e-4);
        assert!((snap_angle(-100f32.to_radians(), 90.0).to_degrees() + 90.0).abs() < 1e-4);
        assert!((snap_angle(0.3, 0.0) - 0.3).abs() < 1e-9);
    }

    #[test]
    fn scale_snap_is_additive_or_absolute() {
        assert!((snap_scale_value(1.0, 1.3, 0.25, false) - 1.25).abs() < 1e-6);
        assert!((snap_scale_value(1.1, 1.4, 0.25, false) - 1.35).abs() < 1e-5, "aditivo preserva o desvio inicial");
        assert!((snap_scale_value(1.1, 1.4, 0.25, true) - 1.5).abs() < 1e-6, "absoluto cai na grade");
        assert!(snap_scale_value(1.0, -3.0, 0.25, true) > 0.0, "nunca zero/negativo");
    }

    #[test]
    fn ctrl_inverts_the_three_grids_but_not_the_mode_flags() {
        let s = SnapSettings { grid_on: true, rot_on: false, scale_on: false, absolute: true, ..Default::default() };
        let e = s.effective(true);
        assert!(!e.grid_on && e.rot_on && e.scale_on);
        assert!(e.absolute, "absoluto/incremental não inverte");
        assert_eq!(s.effective(false), s);
    }

    #[test]
    fn align_up_to_normal_makes_up_match_and_keeps_heading() {
        let yaw = Quat::from_rotation_y(0.8);
        let n = Vec3::new(0.3, 1.0, -0.2).normalize();
        let r = align_up_to_normal(yaw, n);
        assert!((r * Vec3::Y - n).length() < 1e-4);
        // plano horizontal: nada muda
        assert!(align_up_to_normal(yaw, Vec3::Y).abs_diff_eq(yaw, 1e-6));
        // normal oposta: vira de cabeça para baixo sem NaN
        let flipped = align_up_to_normal(Quat::IDENTITY, Vec3::NEG_Y);
        assert!((flipped * Vec3::Y - Vec3::NEG_Y).length() < 1e-4 && flipped.is_finite());
    }

    #[test]
    fn placement_puts_the_bottom_center_on_the_hit_point() {
        // ativo com origem no centro geométrico: 2 m de altura (y -1..1), 1 m de lado
        let local = LocalBox { min: Vec3::new(-0.5, -1.0, -0.5), max: Vec3::new(0.5, 1.0, 0.5) };
        let hit = Vec3::new(3.0, 0.4, -2.0);
        let t = place_on_surface(hit, Vec3::Y, Quat::IDENTITY, Vec3::ONE, &local, 0.0);
        assert!((t - Vec3::new(3.0, 1.4, -2.0)).length() < 1e-5, "{t:?}");
        // escala 2: a base (y=-1*2) fica a 2 m abaixo da origem
        let t = place_on_surface(hit, Vec3::Y, Quat::IDENTITY, Vec3::splat(2.0), &local, 0.1);
        assert!((t.y - (0.4 + 0.1 + 2.0)).abs() < 1e-5, "{t:?}");
        // numa parede (normal +X) com alinhamento à normal: a base encosta na parede
        let rot = align_up_to_normal(Quat::IDENTITY, Vec3::X);
        let t = place_on_surface(Vec3::new(5.0, 1.0, 0.0), Vec3::X, rot, Vec3::ONE, &local, 0.0);
        assert!((t.x - 6.0).abs() < 1e-4, "origem a 1 m da parede: {t:?}");
    }

    #[test]
    fn ground_snap_sits_on_the_floor_plane_and_on_geometry_below() {
        let pts = base_points(Vec3::new(-1.0, 3.0, -1.0), Vec3::new(1.0, 5.0, 1.0));
        // sem geometria: desce até y=0
        let d = ground_snap_delta(&pts, 0.0, |_, _| None);
        assert!((d + 3.0).abs() < 1e-5, "{d}");
        // uma mesa de 0.75 m embaixo de só UM canto: o menor deslocamento vence (não atravessa)
        let d = ground_snap_delta(&pts, 0.0, |o, _| (o.x < -0.5 && o.z < -0.5).then_some(o.y - 0.75));
        assert!((d + 2.25).abs() < 1e-3, "{d}");
        // abaixo do piso: sobe até ele
        let low = base_points(Vec3::new(-1.0, -2.0, -1.0), Vec3::new(1.0, 0.0, 1.0));
        let d = ground_snap_delta(&low, 0.0, |_, _| None);
        assert!((d - 2.0).abs() < 1e-5, "{d}");
        // já no chão: zero
        let on = base_points(Vec3::new(-1.0, 0.0, -1.0), Vec3::ONE);
        assert!(ground_snap_delta(&on, 0.0, |_, _| None).abs() < 1e-6);
    }

    #[test]
    fn lengths_follow_the_project_unit() {
        assert_eq!(format_length(0.5, LengthUnit::Meters), "0.5 m");
        assert_eq!(format_length(1.0, LengthUnit::Meters), "1 m");
        assert_eq!(format_length(0.5, LengthUnit::Centimeters), "50 cm");
        assert_eq!(format_length(0.01, LengthUnit::Centimeters), "1 cm");
        assert_eq!(GRID_STEPS_M.len(), 9);
    }
}
