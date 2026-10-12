//! Shader ball, composição do preview (bola, objeto, A/B) e luz do rig do Shading.
//!
//! Matemática pura (sem GPU): o app monta `LevelNodeIn`s a partir de [`plan_layout`] e o
//! `CelPreview` (renderer cel canônico) desenha.

use crate::viewport::OrbitCamera;
use anigo_core::Mesh;
use glam::{Quat, Vec3};
use std::sync::Arc;

/// Malha da shader ball: esfera UV de raio 0,5 (u = 0,25 olha para +Z — o centro do rosto do mapa SDF).
pub fn ball_mesh() -> Arc<Mesh> {
    Arc::new(Mesh::create_uv_sphere(0.5, 48, 96))
}

/// Luz do preview em coordenadas esféricas: `azimuth` 0 = vinda de trás da câmera (frontal), positivo
/// = pela direita; `elevation` positivo = por cima.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightRig {
    pub azimuth_deg: f32,
    pub elevation_deg: f32,
    /// A luz acompanha a câmera (o azimute é relativo à câmera); desligada, é relativa ao mundo
    /// (azimute 0 = +Z, positivo para +X — a convenção do SDF facial).
    pub follows_camera: bool,
}

impl Default for LightRig {
    fn default() -> Self {
        Self { azimuth_deg: -35.0, elevation_deg: 35.0, follows_camera: true }
    }
}

impl LightRig {
    /// Direção (unitária) da superfície PARA a luz.
    pub fn direction(&self, cam: &OrbitCamera) -> Vec3 {
        let (right, up, back) = if self.follows_camera { (cam.right(), cam.up(), -cam.forward()) } else { (Vec3::X, Vec3::Y, Vec3::Z) };
        let (az, el) = (self.azimuth_deg.to_radians(), self.elevation_deg.to_radians());
        (right * az.sin() * el.cos() + up * el.sin() + back * az.cos() * el.cos()).normalize_or_zero()
    }
}

/// O que o preview mostra.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewMode {
    /// Só a shader ball.
    Ball,
    /// Só o objeto/personagem do alvo.
    Object,
    /// Bola e objeto lado a lado.
    #[default]
    Both,
}

/// Um assunto a desenhar (malha + caixa em espaço local).
#[derive(Clone)]
pub struct Subject {
    pub key: String,
    pub mesh: Arc<Mesh>,
    pub bounds: (Vec3, Vec3),
}

impl Subject {
    pub fn new(key: impl Into<String>, mesh: Arc<Mesh>) -> Self {
        let bounds = mesh_bounds(&mesh).unwrap_or((Vec3::splat(-0.5), Vec3::splat(0.5)));
        Self { key: key.into(), mesh, bounds }
    }
}

/// Caixa envolvente dos vértices finitos.
pub fn mesh_bounds(m: &Mesh) -> Option<(Vec3, Vec3)> {
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    let mut any = false;
    for v in &m.vertices {
        let p = Vec3::from_array(v.position);
        if p.is_finite() {
            lo = lo.min(p);
            hi = hi.max(p);
            any = true;
        }
    }
    any.then_some((lo, hi))
}

/// Qual material usa a cópia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    /// Material atual (o que está sendo editado).
    Current,
    /// Material "antes" (A) do comparador A/B.
    Before,
}

/// Uma cópia posicionada.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub subject: usize,
    pub which: Which,
    pub translation: Vec3,
    pub scale: f32,
}

/// Disposição final: cópias + caixa do conjunto (para enquadrar a câmera).
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub items: Vec<Placed>,
    pub bounds: (Vec3, Vec3),
}

/// Maior lado normalizado do assunto no preview (a bola tem 1,0 de diâmetro).
pub const SUBJECT_SIZE: f32 = 1.0;

/// Dispõe os assuntos em fila (da esquerda para a direita), cada um normalizado para `SUBJECT_SIZE`
/// e apoiado no mesmo plano central. Com `compare`, cada assunto aparece duas vezes: A (antes) à
/// esquerda e o atual à direita.
pub fn plan_layout(subjects: &[Subject], compare: bool) -> Layout {
    let gap = 0.28 * SUBJECT_SIZE;
    let mut items = Vec::new();
    // larguras normalizadas
    let sizes: Vec<(f32, f32)> = subjects
        .iter()
        .map(|s| {
            let e = (s.bounds.1 - s.bounds.0).max(Vec3::splat(1e-4));
            let scale = SUBJECT_SIZE / e.max_element();
            (scale, e.x * scale)
        })
        .collect();
    let copies = if compare { 2 } else { 1 };
    let total: f32 = sizes.iter().map(|(_, w)| *w * copies as f32).sum::<f32>() + gap * ((subjects.len() * copies).saturating_sub(1)) as f32;
    let mut x = -total * 0.5;
    for (i, s) in subjects.iter().enumerate() {
        let (scale, w) = sizes[i];
        let center_local = (s.bounds.0 + s.bounds.1) * 0.5;
        for c in 0..copies {
            let which = if compare && c == 0 { Which::Before } else { Which::Current };
            let target_center = Vec3::new(x + w * 0.5, 0.0, 0.0);
            items.push(Placed { subject: i, which, translation: target_center - center_local * scale, scale });
            x += w + gap;
        }
    }
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for it in &items {
        let b = subjects[it.subject].bounds;
        lo = lo.min(it.translation + b.0 * it.scale);
        hi = hi.max(it.translation + b.1 * it.scale);
    }
    if items.is_empty() {
        lo = Vec3::splat(-0.5);
        hi = Vec3::splat(0.5);
    }
    Layout { items, bounds: (lo, hi) }
}

/// Enquadra a caixa do layout inteira, mantendo yaw/pitch.
pub fn frame_layout(cam: &mut OrbitCamera, bounds: (Vec3, Vec3), aspect: f32) {
    let (lo, hi) = bounds;
    let center = (lo + hi) * 0.5;
    let half = (hi - lo) * 0.5;
    let half_h = half.y.max(half.z * 0.5).max(0.05);
    let half_w = half.x.max(0.05);
    let t = (cam.fov_y * 0.5).tan().max(1e-3);
    let dist_h = half_h / t;
    let dist_w = half_w / (t * aspect.max(0.2));
    cam.target = center;
    cam.distance = (dist_h.max(dist_w) * 1.3 + half.z).clamp(0.2, 100.0);
}

/// Rotação identidade (os assuntos não giram; o usuário orbita a câmera).
pub fn subject_rotation() -> Quat {
    Quat::IDENTITY
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject(w: f32, h: f32, d: f32, off: Vec3) -> Subject {
        let mut m = Mesh::create_cube(1.0);
        for v in &mut m.vertices {
            v.position = [v.position[0] * w + off.x, v.position[1] * h + off.y, v.position[2] * d + off.z];
        }
        Subject::new("s", Arc::new(m))
    }

    #[test]
    fn ball_is_a_unit_diameter_sphere_facing_plus_z_at_u_quarter() {
        let m = ball_mesh();
        let (lo, hi) = mesh_bounds(&m).unwrap();
        assert!(((hi - lo).max_element() - 1.0).abs() < 1e-3);
        // vértice com u = 0.25, v = 0.5 (equador) aponta para +Z
        let v = m.vertices.iter().min_by(|a, b| ((a.uv[0] - 0.25).abs() + (a.uv[1] - 0.5).abs()).total_cmp(&((b.uv[0] - 0.25).abs() + (b.uv[1] - 0.5).abs()))).unwrap();
        assert!(v.position[2] > 0.49 && v.position[0].abs() < 0.02, "{:?}", v.position);
    }

    #[test]
    fn light_rig_follows_the_camera_or_the_world() {
        let cam = OrbitCamera { yaw: 1.0, pitch: 0.2, ..Default::default() };
        let rig = LightRig { azimuth_deg: 0.0, elevation_deg: 0.0, follows_camera: true };
        // frontal: da superfície para a câmera
        let d = rig.direction(&cam);
        assert!((d - (-cam.forward())).length() < 1e-5);
        let world = LightRig { azimuth_deg: 90.0, elevation_deg: 0.0, follows_camera: false };
        assert!((world.direction(&cam) - Vec3::X).length() < 1e-5, "azimute +90 = +X no mundo");
        let up = LightRig { azimuth_deg: 0.0, elevation_deg: 90.0, follows_camera: false };
        assert!((up.direction(&cam) - Vec3::Y).length() < 1e-5);
        for az in [-170.0, -35.0, 0.0, 77.0] {
            for el in [-60.0, 0.0, 45.0] {
                let l = LightRig { azimuth_deg: az, elevation_deg: el, follows_camera: true };
                assert!((l.direction(&cam).length() - 1.0).abs() < 1e-5);
            }
        }
        // o padrão reproduz a luz de estúdio do viewport: esquerda/acima/frente
        let d = LightRig::default().direction(&cam);
        assert!(d.dot(cam.right()) < 0.0 && d.dot(cam.up()) > 0.0 && d.dot(-cam.forward()) > 0.0);
    }

    #[test]
    fn layout_places_subjects_left_to_right_without_overlap_and_normalizes_size() {
        let big = subject(2.0, 6.0, 1.0, Vec3::new(5.0, 1.0, 0.0));
        let ball = Subject::new("ball", ball_mesh());
        let l = plan_layout(&[ball.clone(), big.clone()], false);
        assert_eq!(l.items.len(), 2);
        let ext = |it: &Placed| {
            let s = [&ball, &big][it.subject];
            (it.translation + s.bounds.0 * it.scale, it.translation + s.bounds.1 * it.scale)
        };
        let (a0, a1) = ext(&l.items[0]);
        let (b0, b1) = ext(&l.items[1]);
        assert!(a1.x < b0.x, "sem sobreposição em X ({} < {})", a1.x, b0.x);
        assert!(((a1 - a0).max_element() - SUBJECT_SIZE).abs() < 1e-3);
        assert!(((b1 - b0).max_element() - SUBJECT_SIZE).abs() < 1e-3);
        // centrado em X
        assert!(((l.bounds.0.x + l.bounds.1.x) * 0.5).abs() < 1e-3);
    }

    #[test]
    fn compare_duplicates_every_subject_with_before_on_the_left() {
        let ball = Subject::new("ball", ball_mesh());
        let l = plan_layout(&[ball], true);
        assert_eq!(l.items.len(), 2);
        assert_eq!(l.items[0].which, Which::Before);
        assert_eq!(l.items[1].which, Which::Current);
        assert!(l.items[0].translation.x < l.items[1].translation.x);
        assert_eq!(plan_layout(&[], true).items.len(), 0);
    }

    #[test]
    fn framing_fits_the_whole_layout_in_view() {
        let ball = Subject::new("ball", ball_mesh());
        let l = plan_layout(&[ball.clone(), ball.clone(), ball], true);
        let mut cam = OrbitCamera { yaw: 0.0, pitch: 0.0, ..Default::default() };
        frame_layout(&mut cam, l.bounds, 16.0 / 9.0);
        let vis_h = 2.0 * cam.distance * (cam.fov_y * 0.5).tan();
        let vis_w = vis_h * 16.0 / 9.0;
        assert!(vis_w > l.bounds.1.x - l.bounds.0.x, "largura cabe: {vis_w}");
        assert!(vis_h > l.bounds.1.y - l.bounds.0.y);
    }
}
