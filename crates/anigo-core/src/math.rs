use glam::{Mat4, Quat, Vec3, Vec4};
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
    /// Abertura vertical em radianos (modo perspectiva).
    pub fov_y: f32,
    pub aspect: f32,
    pub z_near: f32,
    pub z_far: f32,
    /// Issue #13: quando presente, a câmera projeta em **ortográfica** com estes
    /// limites (em unidades de mundo). `None` mantém a perspectiva, que continua
    /// sendo o modo padrão de todo o contrato (o frame congelado do renderer é
    /// perspectiva).
    #[serde(default)]
    pub orthographic: Option<OrthographicBounds>,
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
            orthographic: None,
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

    /// Matriz de projeção do modo corrente (issue #13).
    ///
    /// O uniform de câmera carrega só `view_proj`, então perspectiva e
    /// ortográfica percorrem o mesmo caminho nos shaders: trocar de modo muda a
    /// matriz, nunca a malha.
    pub fn build_projection_matrix(&self) -> Mat4 {
        self.projection_mode().matrix(self.z_near, self.z_far)
    }

    pub fn build_view_projection_matrix(&self) -> Mat4 {
        self.build_projection_matrix() * self.build_view_matrix()
    }

    /// Modo de projeção efetivo (issue #13).
    pub fn projection_mode(&self) -> ProjectionMode {
        match self.orthographic {
            Some(bounds) => ProjectionMode::Orthographic {
                left: bounds.left,
                right: bounds.right,
                bottom: bounds.bottom,
                top: bounds.top,
            },
            None => ProjectionMode::Perspective {
                fov_y: self.fov_y,
                aspect: self.aspect,
            },
        }
    }

    /// Troca o modo de projeção **sem mexer em mais nada**: os parâmetros de
    /// perspectiva (`fov_y`/`aspect`) ficam guardados, então voltar ao modo
    /// perspectiva devolve exatamente o enquadramento anterior (é o que evita o
    /// "salto" ao alternar visão).
    pub fn set_projection_mode(&mut self, mode: ProjectionMode) {
        match mode {
            ProjectionMode::Perspective { fov_y, aspect } => {
                self.fov_y = fov_y;
                self.aspect = aspect;
                self.orthographic = None;
            }
            ProjectionMode::Orthographic {
                left,
                right,
                bottom,
                top,
            } => {
                self.orthographic = Some(OrthographicBounds {
                    left,
                    right,
                    bottom,
                    top,
                });
            }
        }
    }

    /// Frustum do volume de visão atual (issue #13).
    pub fn frustum(&self) -> Frustum {
        Frustum::from_view_projection(&self.build_view_projection_matrix())
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

/// Limites da projeção ortográfica (issue #13), em unidades de mundo.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OrthographicBounds {
    pub left: f32,
    pub right: f32,
    pub bottom: f32,
    pub top: f32,
}

impl OrthographicBounds {
    /// Volume simétrico em torno do eixo da câmera, com alvo `height` de
    /// silhueta — o recorte natural de um estúdio de anime.
    pub fn from_height(height: f32, aspect: f32) -> Self {
        let half_height = (height * 0.5).max(0.01);
        let half_width = half_height * aspect.max(0.01);
        Self {
            left: -half_width,
            right: half_width,
            bottom: -half_height,
            top: half_height,
        }
    }

    /// `true` com `left < right` e `bottom < top` (volume não degenerado).
    pub fn is_valid(&self) -> bool {
        self.left < self.right && self.bottom < self.top
    }

    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.top - self.bottom
    }
}

/// Modo de projeção da câmera (issue #13).
///
/// O tipo é **completo**: carrega os parâmetros que a matriz precisa, então quem
/// consome não depende de ler a câmera por fora. `Perspective` reaproveita
/// `fov_y`/`aspect` da câmera (`Camera::projection_mode`), `Orthographic`
/// carrega os limites próprios.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ProjectionMode {
    /// Perspectiva: `fov_y` vertical em radianos e `aspect` (largura/altura).
    Perspective { fov_y: f32, aspect: f32 },
    /// Ortográfica: limites do volume em unidades de mundo.
    Orthographic {
        left: f32,
        right: f32,
        bottom: f32,
        top: f32,
    },
}

impl Default for ProjectionMode {
    fn default() -> Self {
        Self::Perspective {
            fov_y: 45.0_f32.to_radians(),
            aspect: 16.0 / 9.0,
        }
    }
}

impl ProjectionMode {
    /// Nome canônico (`perspective`/`orthographic`) — o mesmo do contrato.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Perspective { .. } => "perspective",
            Self::Orthographic { .. } => "orthographic",
        }
    }

    /// `true` quando a projeção é ortográfica.
    pub fn is_orthographic(self) -> bool {
        matches!(self, Self::Orthographic { .. })
    }

    /// Valida os parâmetros do modo (fov implausível ou volume invertido são
    /// erro de dados, não um caso a tratar em silêncio).
    pub fn validate(self) -> Result<(), ProjectionError> {
        match self {
            Self::Perspective { fov_y, aspect } => {
                if !fov_y.is_finite() || !aspect.is_finite() || fov_y <= 0.0 || fov_y >= std::f32::consts::PI {
                    return Err(ProjectionError::InvalidPerspective);
                }
                if aspect <= 0.0 {
                    return Err(ProjectionError::InvalidPerspective);
                }
                Ok(())
            }
            Self::Orthographic {
                left,
                right,
                bottom,
                top,
            } => {
                let finite = [left, right, bottom, top]
                    .iter()
                    .all(|value| value.is_finite());
                if !finite || left >= right || bottom >= top {
                    return Err(ProjectionError::InvalidOrthographic);
                }
                Ok(())
            }
        }
    }

    /// Matriz de projeção na convenção do renderer (mão direita, clip 0..1).
    pub fn matrix(self, z_near: f32, z_far: f32) -> Mat4 {
        match self {
            Self::Perspective { fov_y, aspect } => Mat4::perspective_rh(fov_y, aspect, z_near, z_far),
            Self::Orthographic {
                left,
                right,
                bottom,
                top,
            } => Mat4::orthographic_rh(left, right, bottom, top, z_near, z_far),
        }
    }
}

impl From<OrthographicBounds> for ProjectionMode {
    fn from(bounds: OrthographicBounds) -> Self {
        Self::Orthographic {
            left: bounds.left,
            right: bounds.right,
            bottom: bounds.bottom,
            top: bounds.top,
        }
    }
}

/// Erro de parâmetro de projeção (issue #13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ProjectionError {
    /// `fov_y` fora de `(0, π)` ou `aspect` não positivo.
    #[error("perspective projection needs fov_y in (0, pi) and a positive aspect")]
    InvalidPerspective,
    /// Limites ortográficos invertidos ou não finitos.
    #[error("orthographic projection needs left < right and bottom < top")]
    InvalidOrthographic,
}

/// Caixa alinhada aos eixos (issue #13) — o volume local de uma malha.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    /// Caixa degenerada no ponto dado.
    pub fn point(point: Vec3) -> Self {
        Self {
            min: point,
            max: point,
        }
    }

    /// Caixa que contém todos os pontos (vazia quando não há nenhum).
    pub fn from_points<I: IntoIterator<Item = Vec3>>(points: I) -> Option<Self> {
        let mut iter = points.into_iter();
        let first = iter.next()?;
        let mut bounds = Self::point(first);
        for point in iter {
            bounds.min = bounds.min.min(point);
            bounds.max = bounds.max.max(point);
        }
        Some(bounds)
    }

    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn half_extents(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }

    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }

    pub fn contains_point(&self, point: Vec3) -> bool {
        point.cmpge(self.min).all() && point.cmple(self.max).all()
    }

    /// A caixa que contém os 8 cantos transformados por `matrix`.
    ///
    /// A rotação entra pelo transform dos cantos (e não pelo raio), então a
    /// caixa resultante é justa para o volume alinhado aos eixos do mundo.
    pub fn transformed(&self, matrix: &Mat4) -> Self {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for corner in 0..8 {
            let point = Vec3::new(
                if corner & 1 == 0 { self.min.x } else { self.max.x },
                if corner & 2 == 0 { self.min.y } else { self.max.y },
                if corner & 4 == 0 { self.min.z } else { self.max.z },
            );
            let world = matrix.transform_point3(point);
            min = min.min(world);
            max = max.max(world);
        }
        Self { min, max }
    }

    /// Esfera circunscrita da caixa.
    pub fn bounding_sphere(&self) -> BoundingSphere {
        let center = self.center();
        BoundingSphere {
            center,
            radius: self.half_extents().length(),
        }
    }
}

/// Esfera envolvente (issue #13) — o volume de teste rápido do culling.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoundingSphere {
    pub center: Vec3,
    pub radius: f32,
}

impl BoundingSphere {
    pub fn new(center: Vec3, radius: f32) -> Self {
        Self {
            center,
            radius: radius.max(0.0),
        }
    }

    pub fn from_points<I: IntoIterator<Item = Vec3>>(points: I) -> Option<Self> {
        Aabb::from_points(points).map(|bounds| bounds.bounding_sphere())
    }

    /// Transforma a esfera pela matriz mundial.
    ///
    /// O raio escala pela **maior** coluna da matriz: uma esfera envolvente
    /// nunca pode encolher (um falso "fora do frustum" apagaria geometria).
    pub fn transformed(&self, matrix: &Mat4) -> Self {
        let center = matrix.transform_point3(self.center);
        let scale_x = matrix.x_axis.truncate().length();
        let scale_y = matrix.y_axis.truncate().length();
        let scale_z = matrix.z_axis.truncate().length();
        let scale = scale_x.max(scale_y).max(scale_z);
        Self {
            center,
            radius: self.radius * scale,
        }
    }
}

/// Frustum da câmera (issue #13): 6 planos normalizados apontando para dentro.
///
/// A extração é a de Gribb–Hartmann sobre a matriz `view_proj` combinada, na
/// convenção de clip do renderer (profundidade 0..1, mão direita).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Frustum {
    planes: [Vec4; 6],
}

/// Índice dos planos na ordem documentada (L, R, B, T, N, F).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrustumPlane {
    Left = 0,
    Right = 1,
    Bottom = 2,
    Top = 3,
    Near = 4,
    Far = 5,
}

impl Frustum {
    /// Quantidade de planos do frustum.
    pub const PLANE_COUNT: usize = 6;

    /// Extrai os planos de uma matriz `view_proj` (clip 0..1).
    pub fn from_view_projection(view_proj: &Mat4) -> Self {
        let row0 = view_proj.row(0);
        let row1 = view_proj.row(1);
        let row2 = view_proj.row(2);
        let row3 = view_proj.row(3);
        let raw = [
            row3 + row0, // left
            row3 - row0, // right
            row3 + row1, // bottom
            row3 - row1, // top
            row2,        // near (clip 0..1)
            row3 - row2, // far
        ];
        let planes = raw.map(|plane| {
            let length = plane.truncate().length();
            if length < 1e-9 {
                plane
            } else {
                plane / length
            }
        });
        Self { planes }
    }

    pub fn planes(&self) -> &[Vec4; 6] {
        &self.planes
    }

    pub fn plane(&self, plane: FrustumPlane) -> Vec4 {
        self.planes[plane as usize]
    }

    /// Distância assinada de um ponto a um plano (`> 0` = dentro).
    fn distance(plane: Vec4, point: Vec3) -> f32 {
        plane.x * point.x + plane.y * point.y + plane.z * point.z + plane.w
    }

    /// `true` quando o ponto está dentro dos 6 planos.
    pub fn contains_point(&self, point: Vec3) -> bool {
        self.planes
            .iter()
            .all(|plane| Self::distance(*plane, point) >= 0.0)
    }

    /// Teste esfera-frustum: rejeita só quando a esfera está **inteiramente**
    /// fora de algum plano (nunca recorta geometria visível por margem).
    pub fn intersects_sphere(&self, sphere: &BoundingSphere) -> bool {
        self.planes.iter().all(|plane| {
            let distance = Self::distance(*plane, sphere.center);
            // Plano degenerado (matriz estranha) não pode rejeitar nada.
            distance + sphere.radius >= 0.0 || !plane.is_finite()
        })
    }

    /// Teste caixa-frustum pelo vértice positivo de cada plano.
    pub fn intersects_aabb(&self, aabb: &Aabb) -> bool {
        self.planes.iter().all(|plane| {
            let positive = Vec3::new(
                if plane.x >= 0.0 { aabb.max.x } else { aabb.min.x },
                if plane.y >= 0.0 { aabb.max.y } else { aabb.min.y },
                if plane.z >= 0.0 { aabb.max.z } else { aabb.min.z },
            );
            Self::distance(*plane, positive) >= 0.0 || !plane.is_finite()
        })
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

    // ── Issue #13: frustum, volumes e modos de projeção ───────────────────────

    fn unit_cube() -> Aabb {
        Aabb {
            min: Vec3::splat(-0.5),
            max: Vec3::splat(0.5),
        }
    }

    #[test]
    fn frustum_contains_the_target_and_rejects_points_behind_the_camera() {
        let camera = Camera::default();
        let frustum = camera.frustum();
        assert_eq!(frustum.planes().len(), Frustum::PLANE_COUNT);

        assert!(frustum.contains_point(camera.target), "o alvo é visível");
        assert!(
            !frustum.intersects_sphere(&BoundingSphere::new(Vec3::new(0.0, 1.0, -100.0), 0.5)),
            "objeto em z = -100 está atrás da câmera"
        );
        assert!(
            frustum.intersects_sphere(&BoundingSphere::new(camera.target, 0.5)),
            "o manequim precisa continuar visível"
        );
    }

    #[test]
    fn frustum_planes_are_normalized_and_point_inward() {
        let camera = Camera::default();
        let frustum = camera.frustum();
        for plane in frustum.planes() {
            assert!(
                (plane.truncate().length() - 1.0).abs() < 1e-4,
                "plano precisa estar normalizado: {plane:?}"
            );
        }
        // O alvo está à frente de todos os planos (distância positiva).
        for plane in frustum.planes() {
            let distance = plane.x * camera.target.x
                + plane.y * camera.target.y
                + plane.z * camera.target.z
                + plane.w;
            assert!(distance > 0.0, "plano {plane:?} aponta para fora");
        }
        assert_eq!(frustum.plane(FrustumPlane::Near), frustum.planes()[4]);
    }

    #[test]
    fn aabb_transformed_by_a_rotated_matrix_stays_conservative() {
        let bounds = unit_cube();
        let rotated = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_4);
        let world = bounds.transformed(&rotated);
        // Os cantos girados cabem na caixa resultante (que é maior que a local).
        for corner in [
            Vec3::new(-0.5, -0.5, -0.5),
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(-0.5, 0.5, -0.5),
        ] {
            assert!(world.contains_point(rotated.transform_point3(corner)));
        }
        assert!(world.size().x > bounds.size().x - 1e-5);
    }

    #[test]
    fn bounding_sphere_grows_with_scale_but_never_shrinks() {
        let sphere = unit_cube().bounding_sphere();
        let scaled = sphere.transformed(&Mat4::from_scale(Vec3::new(1.0, 4.0, 1.0)));
        assert!(
            scaled.radius >= sphere.radius * 4.0 - 1e-4,
            "o raio escala pela maior coluna (nunca encolhe)"
        );
        let translated = sphere.transformed(&Mat4::from_translation(Vec3::new(3.0, 0.0, 0.0)));
        assert!((translated.center.x - 3.0).abs() < 1e-6);
        assert!((translated.radius - sphere.radius).abs() < 1e-6);
    }

    #[test]
    fn orthographic_bounds_keep_the_perspective_framing() {
        let mut camera = Camera::default();
        let distance = (camera.eye - camera.target).length();
        let bounds = OrthographicBounds::from_height(
            2.0 * distance * (camera.fov_y * 0.5).tan(),
            camera.aspect,
        );
        assert!(bounds.is_valid());
        assert!((bounds.height() / bounds.width() - 1.0 / camera.aspect).abs() < 1e-5);

        // Um ponto no plano do alvo projeta no mesmo NDC nos dois modos.
        let forward = (camera.target - camera.eye).normalize();
        let right = camera.up.cross(forward).normalize();
        let up = forward.cross(right).normalize();
        let sample = camera.target + right * 0.2 + up * 0.15;
        let perspective = camera.build_view_projection_matrix();
        let ndc_perspective = perspective.project_point3(sample);

        camera.set_projection_mode(bounds.into());
        assert!(camera.projection_mode().is_orthographic());
        let ndc_orthographic = camera.build_view_projection_matrix().project_point3(sample);
        assert!((ndc_perspective.x - ndc_orthographic.x).abs() < 1e-4);
        assert!((ndc_perspective.y - ndc_orthographic.y).abs() < 1e-4);

        // Voltar para perspectiva devolve exatamente o enquadramento anterior.
        let before = (camera.fov_y, camera.aspect);
        camera.set_projection_mode(ProjectionMode::Perspective {
            fov_y: camera.fov_y,
            aspect: camera.aspect,
        });
        assert_eq!((camera.fov_y, camera.aspect), before);
        assert!(!camera.projection_mode().is_orthographic());
        // A view continua idêntica: só a projeção mudou (nada de mexer na pose
        // da câmera ao alternar o modo).
        assert!(camera
            .build_view_matrix()
            .abs_diff_eq(Mat4::look_at_rh(camera.eye, camera.target, camera.up), 1e-6));
    }

    #[test]
    fn projection_modes_are_validated_and_named() {
        assert_eq!(
            ProjectionMode::Perspective {
                fov_y: 0.7,
                aspect: 1.7
            }
            .as_str(),
            "perspective"
        );
        assert_eq!(
            ProjectionMode::Orthographic {
                left: -1.0,
                right: 1.0,
                bottom: -1.0,
                top: 1.0
            }
            .as_str(),
            "orthographic"
        );
        assert!(ProjectionMode::default().validate().is_ok());
        assert_eq!(
            ProjectionMode::Perspective {
                fov_y: 0.0,
                aspect: 1.0
            }
            .validate(),
            Err(ProjectionError::InvalidPerspective)
        );
        assert_eq!(
            ProjectionMode::Orthographic {
                left: 1.0,
                right: -1.0,
                bottom: -1.0,
                top: 1.0
            }
            .validate(),
            Err(ProjectionError::InvalidOrthographic)
        );

        // A matriz ortográfica usa a convenção do renderer (clip 0..1).
        let matrix = ProjectionMode::Orthographic {
            left: -1.0,
            right: 1.0,
            bottom: -1.0,
            top: 1.0,
        }
        .matrix(0.05, 100.0);
        let near = matrix * Vec4::new(0.0, 0.0, -0.05, 1.0);
        let far = matrix * Vec4::new(0.0, 0.0, -100.0, 1.0);
        assert!(near.z.abs() < 1e-5);
        assert!((far.z - 1.0).abs() < 1e-5);
    }
}
