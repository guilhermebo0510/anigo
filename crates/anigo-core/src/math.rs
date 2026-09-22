use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

/// 3D Transform representing position, rotation, and uniform/non-uniform scale.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}

impl Transform {
    pub fn from_translation(translation: Vec3) -> Self {
        Self {
            translation,
            ..Default::default()
        }
    }

    pub fn to_matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

/// Camera representation with orbital controls and perspective projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    pub eye: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub fov_y: f32,
    pub aspect: f32,
    pub z_near: f32,
    pub z_far: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            eye: Vec3::new(0.0, 1.5, 3.5),
            target: Vec3::new(0.0, 1.0, 0.0),
            up: Vec3::Y,
            fov_y: 45.0_f32.to_radians(),
            aspect: 16.0 / 9.0,
            z_near: 0.05,
            z_far: 100.0,
        }
    }
}

impl Camera {
    pub fn new(eye: Vec3, target: Vec3, aspect: f32) -> Self {
        Self {
            eye,
            target,
            aspect,
            ..Default::default()
        }
    }

    pub fn build_view_matrix(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye, self.target, self.up)
    }

    pub fn build_projection_matrix(&self) -> Mat4 {
        Mat4::perspective_rh(self.fov_y, self.aspect, self.z_near, self.z_far)
    }

    pub fn build_view_projection_matrix(&self) -> Mat4 {
        self.build_projection_matrix() * self.build_view_matrix()
    }

    /// Orbit camera around target by azimuth and elevation deltas (in radians).
    pub fn orbit(&mut self, delta_azimuth: f32, delta_elevation: f32) {
        let offset = self.eye - self.target;
        let radius = offset.length();
        if radius < 0.001 {
            return;
        }

        let mut azimuth = offset.z.atan2(offset.x);
        let mut elevation = (offset.y / radius).clamp(-1.0, 1.0).asin();

        azimuth += delta_azimuth;
        elevation = (elevation + delta_elevation).clamp(-89.0_f32.to_radians(), 89.0_f32.to_radians());

        let x = radius * elevation.cos() * azimuth.cos();
        let y = radius * elevation.sin();
        let z = radius * elevation.cos() * azimuth.sin();

        self.eye = self.target + Vec3::new(x, y, z);
    }

    /// Zoom in/out by adjusting distance to target.
    pub fn zoom(&mut self, factor: f32) {
        let offset = self.eye - self.target;
        let new_radius = (offset.length() * factor).clamp(0.2, 50.0);
        self.eye = self.target + offset.normalize() * new_radius;
    }

    /// Pan camera along its local right and up axes.
    pub fn pan(&mut self, delta_x: f32, delta_y: f32) {
        let forward = (self.target - self.eye).normalize();
        let right = forward.cross(self.up).normalize();
        let up = right.cross(forward).normalize();

        let shift = right * delta_x + up * delta_y;
        self.eye += shift;
        self.target += shift;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Fase 2 (#53): Câmera cinematográfica — lentes, CoC/DoF e tracking
// ─────────────────────────────────────────────────────────────────────────────

/// Altura do sensor full-frame (mm) — padrão do formato 35 mm que os
/// presets de lente cinematográficos assumem.
pub const SENSOR_HEIGHT_MM: f32 = 24.0;

/// FOV vertical (radianos) de uma distância focal em mm:
///   fov_y = 2 · atan(sensor_height / 2 / focal)
/// 24 mm → 53.13°, 35 → 37.85°, 50 → 27.00°, 85 → 16.13°, 135 → 10.13°.
pub fn lens_fov_y(focal_mm: f32, sensor_height_mm: f32) -> f32 {
    2.0 * ((sensor_height_mm * 0.5) / focal_mm).atan()
}

/// Raio do orbitador que **mantém o enquadramento do sujeito** ao trocar de
/// lente (aceite do issue: alternar de 24 → 85 mm muda a perspectiva sem
/// mover o orbitador bruscamente):
///   r' = r · tan(fov_from/2) / tan(fov_to/2)
pub fn reframe_radius(radius: f32, fov_from: f32, fov_to: f32) -> f32 {
    radius * (fov_from * 0.5).tan() / (fov_to * 0.5).tan()
}

/// Circle of Confusion (CoC, metros no sensor) de um ponto a `frag_dist` da
/// lente com foco em `focus_dist` — fórmula do issue:
///   CoC = |(D − F_dist) / D| × F² / (N × (F_dist − F))
/// onde D = distância do ponto, F_dist = distância de foco, F = focal (m) e
/// N = número f. CoC = 0 no plano de foco; cresce com abertura (N baixo),
/// focal longa e defasagem do ponto em relação ao plano.
pub fn circle_of_confusion(frag_dist: f32, focus_dist: f32, focal_m: f32, f_number: f32) -> f32 {
    let d = frag_dist.max(1e-4);
    let fd = focus_dist.max(1e-4);
    let f = focal_m.max(1e-4);
    let n = f_number.max(0.05);
    ((fd - d).abs() / d) * (f * f) / (n * (fd - f).max(1e-4))
}

/// Raio do bokeh em pixels para um CoC (m): fração da altura do sensor
/// convertida em pixels da imagem.
pub fn bokeh_radius_px(coc_m: f32, image_height_px: u32, sensor_height_mm: f32) -> f32 {
    (coc_m / (sensor_height_mm * 0.001)) * image_height_px as f32
}

/// Suavização exponencial (criticamente amortecida) — tracking de câmera e
/// olhar: `x += (t − x) × (1 − e^(−damping × dt))`. damping 1/s.
pub fn damp_value(current: f32, target: f32, damping_per_second: f32, delta_seconds: f32) -> f32 {
    let k = 1.0 - (-damping_per_second.max(0.0) * delta_seconds.max(0.0)).exp();
    current + (target - current) * k
}

/// Suavização exponencial de um vetor (componente a componente).
pub fn damp_vec3(
    current: Vec3,
    target: Vec3,
    damping_per_second: f32,
    delta_seconds: f32,
) -> Vec3 {
    Vec3::new(
        damp_value(current.x, target.x, damping_per_second, delta_seconds),
        damp_value(current.y, target.y, damping_per_second, delta_seconds),
        damp_value(current.z, target.z, damping_per_second, delta_seconds),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transform_matrix() {
        let t = Transform {
            translation: Vec3::new(1.0, 2.0, 3.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::new(2.0, 2.0, 2.0),
        };
        let mat = t.to_matrix();
        let transformed = mat.transform_point3(Vec3::new(1.0, 0.0, 0.0));
        assert!((transformed.x - 3.0).abs() < 1e-5);
        assert!((transformed.y - 2.0).abs() < 1e-5);
        assert!((transformed.z - 3.0).abs() < 1e-5);
    }

    #[test]
    fn test_camera_projection_and_view() {
        let cam = Camera::default();
        let view = cam.build_view_matrix();
        let proj = cam.build_projection_matrix();
        let vp = cam.build_view_projection_matrix();
        assert_eq!(vp, proj * view);
    }

    #[test]
    fn test_camera_orbit_preserves_radius() {
        let mut cam = Camera::default();
        let initial_radius = (cam.eye - cam.target).length();
        cam.orbit(0.5, 0.2);
        let new_radius = (cam.eye - cam.target).length();
        assert!((initial_radius - new_radius).abs() < 1e-4);
    }

    #[test]
    fn test_camera_zoom() {
        let mut cam = Camera::default();
        let initial_dist = (cam.eye - cam.target).length();
        cam.zoom(1.5);
        let zoomed_dist = (cam.eye - cam.target).length();
        assert!((zoomed_dist - initial_dist * 1.5).abs() < 1e-4);
    }

    #[test]
    fn test_camera_pan_preserves_distance() {
        let mut cam = Camera::default();
        let initial_dist = (cam.eye - cam.target).length();
        cam.pan(1.0, 2.0);
        let new_dist = (cam.eye - cam.target).length();
        assert!((new_dist - initial_dist).abs() < 1e-4);
    }

    #[test]
    fn test_lens_fov_presets_full_frame() {
        // 36x24 mm: fov_y = 2*atan(12/focal) - valores do contrato
        // (cinematography.lenses golden).
        let cases = [
            (24.0, 53.130102),
            (35.0, 37.849289),
            (50.0, 26.991467),
            (85.0, 16.071421),
            (135.0, 10.159216),
        ];
        for (focal, expected_deg) in cases {
            let fov = lens_fov_y(focal, SENSOR_HEIGHT_MM).to_degrees();
            assert!((fov - expected_deg).abs() < 1e-4, "focal {focal}: {fov}");
        }
        // Lente mais longa -> FOV sempre menor (monotonico).
        assert!(lens_fov_y(135.0, SENSOR_HEIGHT_MM) < lens_fov_y(85.0, SENSOR_HEIGHT_MM));
        assert!(lens_fov_y(85.0, SENSOR_HEIGHT_MM) < lens_fov_y(24.0, SENSOR_HEIGHT_MM));
    }

    #[test]
    fn test_reframe_radius_keeps_framing() {
        // Trocar 50->85 mm aprofunda o FOV: o orbitador afasta na razao dos
        // tan(fov/2) para o sujeito manter o tamanho em tela (aceite 2).
        let fov50 = lens_fov_y(50.0, SENSOR_HEIGHT_MM);
        let fov85 = lens_fov_y(85.0, SENSOR_HEIGHT_MM);
        let r = reframe_radius(3.0, fov50, fov85);
        assert!(r > 3.0, "telefoto precisa afastar o orbitador");
        // Enquadramento identico: tan(fov/2) * raio e conservado.
        let framing = |fov: f32, radius: f32| (fov * 0.5).tan() * radius;
        assert!(
            (framing(fov50, 3.0) - framing(fov85, r)).abs() < 1e-4,
            "enquadramento nao conservado no reframe"
        );
    }

    #[test]
    fn test_circle_of_confusion() {
        // No plano de foco, CoC = 0 (olhos/rosto nitidos - aceite 1).
        assert!(circle_of_confusion(2.0, 2.0, 0.05, 2.0) < 1e-6);
        // 50 mm f/2 focado em 2 m: ponto a 1 m (frente) -> ~0.64 mm de CoC.
        let front = circle_of_confusion(1.0, 2.0, 0.05, 2.0);
        assert!((front - 0.0006410256).abs() < 1e-6, "CoC frente: {front}");
        // Ponto a 3 m (atras) -> ~0.21 mm (menor - fisica do fino-lente).
        let back = circle_of_confusion(3.0, 2.0, 0.05, 2.0);
        assert!((back - 0.0002136752).abs() < 1e-6, "CoC atras: {back}");
        // Abertura maior (f/1.4) -> mais bokeh; focal mais longa -> mais bokeh.
        assert!(circle_of_confusion(1.0, 2.0, 0.05, 1.4) > front);
        assert!(circle_of_confusion(1.0, 2.0, 0.085, 2.0) > circle_of_confusion(1.0, 2.0, 0.05, 2.0));
    }

    #[test]
    fn test_damping_converges_and_is_smooth() {
        // Suavizacao exponencial: converge ao alvo, nunca o ultrapassa.
        let mut x = 0.0;
        let target = 1.0;
        for _ in 0..300 {
            x = damp_value(x, target, 6.0, 1.0 / 60.0);
            assert!(x < target, "damping nao pode ultrapassar o alvo");
        }
        assert!((x - target).abs() < 1e-3);
        // damping 0 -> imutavel; dt 0 -> imutavel.
        assert_eq!(damp_value(0.5, 1.0, 0.0, 1.0), 0.5);
        assert_eq!(damp_value(0.5, 1.0, 6.0, 0.0), 0.5);
    }
}
