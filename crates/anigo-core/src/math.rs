use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

/// 3D Transform representing position, rotation, and uniform/non-uniform scale.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}
