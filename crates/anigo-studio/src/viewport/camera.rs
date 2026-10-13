//! Câmera orbital estilo "turntable" do Blender (Y para cima).

use glam::{Mat4, Vec2, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewPreset {
    Front,
    Back,
    Right,
    Left,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OrbitCamera {
    pub target: Vec3,
    /// Rotação em torno de Y (radianos). 0 = olhando para -Z a partir de +Z (vista frontal).
    pub yaw: f32,
    /// Elevação (radianos), limitada a ±89,9°.
    pub pitch: f32,
    pub distance: f32,
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
    pub orthographic: bool,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::new(0.0, 1.0, 0.0),
            yaw: 0.6,
            pitch: 0.35,
            distance: 5.0,
            fov_y: 40f32.to_radians(),
            near: 0.05,
            far: 500.0,
            orthographic: false,
        }
    }
}

const MAX_PITCH: f32 = 89.9 * std::f32::consts::PI / 180.0;

impl OrbitCamera {
    /// Direção do alvo para a câmera (unitária).
    fn back(&self) -> Vec3 {
        Vec3::new(
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
            self.pitch.cos() * self.yaw.cos(),
        )
    }

    pub fn eye(&self) -> Vec3 {
        self.target + self.back() * self.distance
    }

    pub fn forward(&self) -> Vec3 {
        -self.back()
    }

    pub fn right(&self) -> Vec3 {
        self.forward().cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X)
    }

    pub fn up(&self) -> Vec3 {
        self.right().cross(self.forward()).normalize_or_zero()
    }

    pub fn view(&self) -> Mat4 {
        // Em visão de topo/baixo o "up" de mundo degenera; usa o up derivado.
        let up = if self.pitch.abs() > MAX_PITCH - 1e-3 {
            Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos()) * self.pitch.signum()
        } else {
            Vec3::Y
        };
        Mat4::look_at_rh(self.eye(), self.target, up)
    }

    pub fn projection(&self, aspect: f32) -> Mat4 {
        let aspect = aspect.max(1e-4);
        if self.orthographic {
            let half_h = self.distance * (self.fov_y * 0.5).tan();
            let half_w = half_h * aspect;
            Mat4::orthographic_rh(-half_w, half_w, -half_h, half_h, -self.far, self.far)
        } else {
            Mat4::perspective_rh(self.fov_y, aspect, self.near, self.far)
        }
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.projection(aspect) * self.view()
    }

    /// Arrasto de orbitar (pixels → radianos).
    pub fn orbit(&mut self, dx_px: f32, dy_px: f32) {
        const SENS: f32 = 0.008;
        self.yaw -= dx_px * SENS;
        self.pitch = (self.pitch + dy_px * SENS).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Arrasto de pan: move o alvo no plano da tela proporcional à distância.
    pub fn pan(&mut self, dx_px: f32, dy_px: f32, viewport_h_px: f32) {
        let world_per_px = 2.0 * self.distance * (self.fov_y * 0.5).tan() / viewport_h_px.max(1.0);
        self.target += (-self.right() * dx_px + self.up() * dy_px) * world_per_px;
    }

    /// Zoom multiplicativo (`factor` > 1 afasta). Distância limitada.
    pub fn zoom(&mut self, factor: f32) {
        self.distance = (self.distance * factor).clamp(0.01, 400.0);
    }

    /// Zoom em direção ao ponteiro (roda do mouse): move o alvo parcialmente.
    pub fn zoom_to_cursor(&mut self, factor: f32, ndc: Vec2, aspect: f32) {
        let before = self.ray_from_ndc(ndc, aspect);
        let plane_point = self.target;
        let old = intersect_plane(before.0, before.1, plane_point, self.forward());
        self.zoom(factor);
        let after = self.ray_from_ndc(ndc, aspect);
        let new = intersect_plane(after.0, after.1, plane_point, self.forward());
        if let (Some(a), Some(b)) = (old, new) {
            self.target += a - b;
        }
    }

    /// Enquadra uma caixa (min, max): ajusta alvo e distância para caber.
    pub fn frame_bounds(&mut self, min: Vec3, max: Vec3) {
        let center = (min + max) * 0.5;
        let radius = ((max - min).length() * 0.5).max(1e-3);
        self.target = center;
        self.distance = (radius / (self.fov_y * 0.5).sin().max(0.1)).clamp(0.05, 400.0);
    }

    pub fn set_preset(&mut self, preset: ViewPreset) {
        use std::f32::consts::{FRAC_PI_2, PI};
        let (yaw, pitch) = match preset {
            ViewPreset::Front => (0.0, 0.0),
            ViewPreset::Back => (PI, 0.0),
            ViewPreset::Right => (FRAC_PI_2, 0.0),
            ViewPreset::Left => (-FRAC_PI_2, 0.0),
            ViewPreset::Top => (self.yaw, MAX_PITCH),
            ViewPreset::Bottom => (self.yaw, -MAX_PITCH),
        };
        self.yaw = yaw;
        self.pitch = pitch;
    }

    /// Raio de mundo a partir de coordenadas NDC (x,y ∈ [-1,1], y para cima).
    pub fn ray_from_ndc(&self, ndc: Vec2, aspect: f32) -> (Vec3, Vec3) {
        let inv = self.view_proj(aspect).inverse();
        let near = inv.project_point3(Vec3::new(ndc.x, ndc.y, 0.0));
        let far = inv.project_point3(Vec3::new(ndc.x, ndc.y, 1.0));
        if self.orthographic {
            (near, self.forward())
        } else {
            (self.eye(), (far - self.eye()).normalize_or_zero())
        }
    }

    /// Raio a partir de um pixel (origem no canto superior esquerdo).
    pub fn ray_from_pixel(&self, px: Vec2, size: Vec2) -> (Vec3, Vec3) {
        let ndc = Vec2::new(px.x / size.x * 2.0 - 1.0, 1.0 - px.y / size.y * 2.0);
        self.ray_from_ndc(ndc, size.x / size.y.max(1.0))
    }

    /// Projeta um ponto de mundo para pixels (origem no canto superior esquerdo).
    /// Devolve `(pixel, profundidade_ndc)`; `None` se estiver atrás da câmera.
    pub fn project(&self, world: Vec3, size: Vec2) -> Option<(Vec2, f32)> {
        project_with(self.view_proj(size.x / size.y.max(1.0)), world, size)
    }
}

/// Projeta com uma matriz view-proj já calculada (evita recomputar por vértice).
pub fn project_with(view_proj: Mat4, world: Vec3, size: Vec2) -> Option<(Vec2, f32)> {
    let clip = view_proj * world.extend(1.0);
    if clip.w <= 1e-6 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    Some((
        Vec2::new((ndc.x * 0.5 + 0.5) * size.x, (1.0 - (ndc.y * 0.5 + 0.5)) * size.y),
        ndc.z,
    ))
}

pub fn intersect_plane(origin: Vec3, dir: Vec3, plane_point: Vec3, plane_normal: Vec3) -> Option<Vec3> {
    let denom = dir.dot(plane_normal);
    if denom.abs() < 1e-8 {
        return None;
    }
    let t = (plane_point - origin).dot(plane_normal) / denom;
    Some(origin + dir * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: Vec2 = Vec2::new(800.0, 600.0);

    #[test]
    fn target_projects_to_viewport_center() {
        let cam = OrbitCamera::default();
        let (p, depth) = cam.project(cam.target, SIZE).unwrap();
        assert!((p - SIZE * 0.5).length() < 0.5, "{p:?}");
        assert!((0.0..1.0).contains(&depth));
    }

    #[test]
    fn pixel_ray_roundtrips_projection() {
        let cam = OrbitCamera::default();
        let world = Vec3::new(0.7, 1.4, -0.3);
        let (px, _) = cam.project(world, SIZE).unwrap();
        let (o, d) = cam.ray_from_pixel(px, SIZE);
        // distância do ponto ao raio ~ 0
        let to = world - o;
        let dist = (to - d * to.dot(d)).length();
        assert!(dist < 1e-3, "dist {dist}");
    }

    #[test]
    fn orbit_changes_eye_but_keeps_distance_and_clamps_pitch() {
        let mut cam = OrbitCamera::default();
        let d0 = (cam.eye() - cam.target).length();
        cam.orbit(120.0, 5000.0);
        let d1 = (cam.eye() - cam.target).length();
        assert!((d0 - d1).abs() < 1e-4);
        assert!(cam.pitch <= MAX_PITCH + 1e-6);
        assert!(cam.view().is_finite());
    }

    #[test]
    fn pan_moves_target_in_screen_plane() {
        let mut cam = OrbitCamera::default();
        let t0 = cam.target;
        cam.pan(100.0, 0.0, 600.0);
        let delta = cam.target - t0;
        assert!(delta.dot(cam.forward()).abs() < 1e-5);
        assert!(delta.dot(cam.right()) < 0.0, "arrastar para a direita move o alvo para a esquerda");
    }

    #[test]
    fn frame_bounds_fits_sphere_in_view() {
        let mut cam = OrbitCamera::default();
        cam.frame_bounds(Vec3::splat(-2.0), Vec3::splat(2.0));
        for corner in [Vec3::splat(-2.0), Vec3::splat(2.0), Vec3::new(-2.0, 2.0, -2.0)] {
            let (p, _) = cam.project(corner, SIZE).unwrap();
            assert!(p.x > 0.0 && p.x < SIZE.x && p.y > 0.0 && p.y < SIZE.y, "{corner:?} → {p:?}");
        }
    }

    #[test]
    fn presets_look_along_axes() {
        let mut cam = OrbitCamera::default();
        cam.set_preset(ViewPreset::Front);
        assert!((cam.forward() - Vec3::NEG_Z).length() < 1e-5);
        cam.set_preset(ViewPreset::Right);
        assert!((cam.forward() - Vec3::NEG_X).length() < 1e-5);
        cam.set_preset(ViewPreset::Top);
        assert!(cam.forward().y < -0.999);
        assert!(cam.view().is_finite() && cam.up().is_finite());
    }

    #[test]
    fn zoom_to_cursor_keeps_point_under_cursor() {
        let mut cam = OrbitCamera::default();
        let ndc = Vec2::new(0.4, -0.2);
        let (o0, d0) = cam.ray_from_ndc(ndc, 4.0 / 3.0);
        let p0 = intersect_plane(o0, d0, cam.target, cam.forward()).unwrap();
        cam.zoom_to_cursor(0.7, ndc, 4.0 / 3.0);
        let (o1, d1) = cam.ray_from_ndc(ndc, 4.0 / 3.0);
        let p1 = intersect_plane(o1, d1, cam.target, cam.forward()).unwrap();
        assert!((p0 - p1).length() < 1e-3);
    }

    #[test]
    fn orthographic_rays_are_parallel() {
        let cam = OrbitCamera {
            orthographic: true,
            ..Default::default()
        };
        let (_, d1) = cam.ray_from_pixel(Vec2::new(10.0, 10.0), SIZE);
        let (_, d2) = cam.ray_from_pixel(Vec2::new(700.0, 500.0), SIZE);
        assert!((d1 - d2).length() < 1e-5);
    }

    #[test]
    fn points_behind_camera_do_not_project() {
        let cam = OrbitCamera::default();
        let behind = cam.eye() - cam.forward() * 3.0;
        assert!(cam.project(behind, SIZE).is_none());
    }
}
