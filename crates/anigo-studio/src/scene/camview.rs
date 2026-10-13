//! Visão de câmera do viewport do Cenário: projeção/raios (sem egui) e a navegação estilo Unreal
//! (voar com RMB+WASD, velocidade por distância, focar, vistas ortográficas).

use crate::viewport::camera::{project_with, OrbitCamera, ViewPreset};
use glam::{Mat4, Vec2, Vec3};

/// Câmera + viewport em pixels: tudo o que o gizmo, o picking e os overlays precisam.
#[derive(Debug, Clone, Copy)]
pub struct CamView {
    pub view_proj: Mat4,
    pub inv_view_proj: Mat4,
    pub eye: Vec3,
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub viewport: Vec2,
    pub ortho: bool,
    pub fov_y: f32,
    /// Altura (mundo) do volume ortográfico, quando `ortho`.
    pub ortho_height: f32,
}

impl CamView {
    pub fn from_orbit(cam: &OrbitCamera, viewport: Vec2) -> Self {
        let aspect = viewport.x / viewport.y.max(1.0);
        let view_proj = cam.view_proj(aspect);
        Self {
            view_proj,
            inv_view_proj: view_proj.inverse(),
            eye: cam.eye(),
            forward: cam.forward(),
            right: cam.right(),
            up: cam.up(),
            viewport,
            ortho: cam.orthographic,
            fov_y: cam.fov_y,
            ortho_height: 2.0 * cam.distance * (cam.fov_y * 0.5).tan(),
        }
    }

    /// Pixel (origem no canto superior esquerdo) e profundidade NDC; `None` atrás da câmera.
    pub fn project(&self, p: Vec3) -> Option<(Vec2, f32)> {
        project_with(self.view_proj, p, self.viewport)
    }

    /// Raio de mundo (origem, direção unitária) a partir de um pixel.
    pub fn ray(&self, px: Vec2) -> (Vec3, Vec3) {
        let ndc = Vec2::new(px.x / self.viewport.x.max(1.0) * 2.0 - 1.0, 1.0 - px.y / self.viewport.y.max(1.0) * 2.0);
        let near = self.inv_view_proj.project_point3(Vec3::new(ndc.x, ndc.y, 0.0));
        let far = self.inv_view_proj.project_point3(Vec3::new(ndc.x, ndc.y, 1.0));
        if self.ortho {
            (near, self.forward)
        } else {
            (self.eye, (far - self.eye).normalize_or_zero())
        }
    }

    /// Distância (mundo) que um pixel cobre na profundidade de `p`.
    pub fn world_per_pixel(&self, p: Vec3) -> f32 {
        let h = if self.ortho {
            self.ortho_height
        } else {
            let depth = (p - self.eye).dot(self.forward).abs().max(0.05);
            2.0 * depth * (self.fov_y * 0.5).tan()
        };
        h / self.viewport.y.max(1.0)
    }

    /// Plano de tela que passa por `p` (normal = direção de olhar).
    pub fn screen_plane_normal(&self) -> Vec3 {
        self.forward
    }
}

/// Velocidade-base da câmera de voo (m/s) para uma distância de foco `distance` — a ideia do
/// "Distance Based Camera Speed" do UE: cena pequena = devagar, cena grande = rápido.
pub fn fly_speed(base: f32, scalar: f32, distance: f32) -> f32 {
    let d = distance.clamp(0.2, 400.0);
    (base * scalar * d.powf(0.6)).clamp(0.05, 200.0)
}

/// Aplica "olhar em volta" mantendo o OLHO parado (RMB+arrastar): gira yaw/pitch e reposiciona o alvo.
pub fn fly_look(cam: &mut OrbitCamera, dx_px: f32, dy_px: f32) {
    const SENS: f32 = 0.005;
    let eye = cam.eye();
    cam.yaw -= dx_px * SENS;
    cam.pitch = (cam.pitch - dy_px * SENS).clamp(-1.5, 1.5);
    // eye = target + back*distance → target = eye - back*distance
    let back = Vec3::new(cam.pitch.cos() * cam.yaw.sin(), cam.pitch.sin(), cam.pitch.cos() * cam.yaw.cos());
    cam.target = eye - back * cam.distance;
}

/// Move o olho por `local` (x = direita, y = cima do mundo, z = frente) × `dt` × `speed`.
pub fn fly_move(cam: &mut OrbitCamera, local: Vec3, speed: f32, dt: f32) {
    if local == Vec3::ZERO {
        return;
    }
    let delta = (cam.right() * local.x + Vec3::Y * local.y + cam.forward() * local.z) * speed * dt;
    cam.target += delta;
}

/// Enquadra uma caixa mundial mantendo a direção de olhar (tecla F). Respeita a projeção ortográfica.
pub fn frame_box(cam: &mut OrbitCamera, lo: Vec3, hi: Vec3) {
    let center = (lo + hi) * 0.5;
    let radius = ((hi - lo).length() * 0.5).max(0.25);
    cam.target = center;
    cam.distance = (radius / (cam.fov_y * 0.5).sin().max(0.1) * 1.15).clamp(0.3, 400.0);
}

/// Vistas ortográficas estilo UE (Alt+H frente, Alt+K lado, Alt+J topo) e perspectiva (Alt+G).
pub fn set_view(cam: &mut OrbitCamera, view: SceneViewKind) {
    match view {
        SceneViewKind::Perspective => cam.orthographic = false,
        SceneViewKind::Front => {
            cam.set_preset(ViewPreset::Front);
            cam.orthographic = true;
        }
        SceneViewKind::Back => {
            cam.set_preset(ViewPreset::Back);
            cam.orthographic = true;
        }
        SceneViewKind::Right => {
            cam.set_preset(ViewPreset::Right);
            cam.orthographic = true;
        }
        SceneViewKind::Left => {
            cam.set_preset(ViewPreset::Left);
            cam.orthographic = true;
        }
        SceneViewKind::Top => {
            cam.set_preset(ViewPreset::Top);
            cam.orthographic = true;
        }
        SceneViewKind::Bottom => {
            cam.set_preset(ViewPreset::Bottom);
            cam.orthographic = true;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SceneViewKind {
    Perspective,
    Front,
    Back,
    Right,
    Left,
    Top,
    Bottom,
}

impl SceneViewKind {
    pub const ALL: [SceneViewKind; 7] = [
        SceneViewKind::Perspective,
        SceneViewKind::Front,
        SceneViewKind::Back,
        SceneViewKind::Right,
        SceneViewKind::Left,
        SceneViewKind::Top,
        SceneViewKind::Bottom,
    ];

    pub fn key(self) -> &'static str {
        match self {
            SceneViewKind::Perspective => "scene.view.perspective",
            SceneViewKind::Front => "scene.view.front",
            SceneViewKind::Back => "scene.view.back",
            SceneViewKind::Right => "scene.view.right",
            SceneViewKind::Left => "scene.view.left",
            SceneViewKind::Top => "scene.view.top",
            SceneViewKind::Bottom => "scene.view.bottom",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VP: Vec2 = Vec2::new(800.0, 600.0);

    #[test]
    fn rays_and_projection_round_trip() {
        let cam = OrbitCamera::default();
        let view = CamView::from_orbit(&cam, VP);
        let world = Vec3::new(0.4, 1.3, -0.2);
        let (px, _) = view.project(world).unwrap();
        let (o, d) = view.ray(px);
        let to = world - o;
        assert!((to - d * to.dot(d)).length() < 1e-3);
    }

    #[test]
    fn world_per_pixel_matches_the_projection_of_a_unit_step() {
        let cam = OrbitCamera { target: Vec3::ZERO, yaw: 0.0, pitch: 0.0, distance: 6.0, ..Default::default() };
        let view = CamView::from_orbit(&cam, VP);
        let wpp = view.world_per_pixel(Vec3::ZERO);
        let (a, _) = view.project(Vec3::ZERO).unwrap();
        let (b, _) = view.project(Vec3::X * wpp * 100.0).unwrap();
        assert!((a.distance(b) - 100.0).abs() < 1.0, "{}", a.distance(b));
    }

    #[test]
    fn fly_look_keeps_the_eye_in_place() {
        let mut cam = OrbitCamera::default();
        let eye = cam.eye();
        fly_look(&mut cam, 120.0, -40.0);
        assert!((cam.eye() - eye).length() < 1e-4);
        assert!(cam.forward().is_finite());
    }

    #[test]
    fn fly_move_translates_along_the_view_axes() {
        let mut cam = OrbitCamera { yaw: 0.0, pitch: 0.0, ..Default::default() };
        let eye = cam.eye();
        fly_move(&mut cam, Vec3::new(0.0, 0.0, 1.0), 2.0, 1.0);
        assert!(((cam.eye() - eye) - cam.forward() * 2.0).length() < 1e-4);
        let eye = cam.eye();
        fly_move(&mut cam, Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5);
        assert!(((cam.eye() - eye) - Vec3::Y * 0.5).length() < 1e-4, "subir é sempre no eixo global Y");
    }

    #[test]
    fn fly_speed_grows_with_distance_and_is_bounded() {
        let near = fly_speed(2.0, 1.0, 1.0);
        let far = fly_speed(2.0, 1.0, 40.0);
        assert!(far > near * 3.0);
        assert!(fly_speed(2.0, 100.0, 400.0) <= 200.0);
        assert!(fly_speed(0.001, 0.001, 0.2) >= 0.05);
    }

    #[test]
    fn frame_box_fits_the_corners() {
        let mut cam = OrbitCamera::default();
        let (lo, hi) = (Vec3::new(-3.0, 0.0, -2.0), Vec3::new(3.0, 2.0, 2.0));
        frame_box(&mut cam, lo, hi);
        let view = CamView::from_orbit(&cam, VP);
        for c in [lo, hi, Vec3::new(lo.x, hi.y, hi.z), Vec3::new(hi.x, lo.y, lo.z)] {
            let (p, _) = view.project(c).unwrap();
            assert!(p.x > 0.0 && p.x < VP.x && p.y > 0.0 && p.y < VP.y, "{c:?} -> {p:?}");
        }
    }

    #[test]
    fn ortho_views_look_along_the_axes() {
        let mut cam = OrbitCamera::default();
        set_view(&mut cam, SceneViewKind::Top);
        assert!(cam.orthographic && cam.forward().y < -0.999);
        set_view(&mut cam, SceneViewKind::Front);
        assert!((cam.forward() - Vec3::NEG_Z).length() < 1e-5);
        set_view(&mut cam, SceneViewKind::Perspective);
        assert!(!cam.orthographic);
        let view = CamView::from_orbit(&OrbitCamera { orthographic: true, ..Default::default() }, VP);
        let (_, d1) = view.ray(Vec2::new(10.0, 10.0));
        let (_, d2) = view.ray(Vec2::new(700.0, 500.0));
        assert!((d1 - d2).length() < 1e-5);
    }
}
