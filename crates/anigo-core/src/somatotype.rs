use glam::Vec2;
use serde::{Deserialize, Serialize};
use crate::mesh::{Mesh, Vertex};

/// Canonical vertices of the 2D Heath-Carter somatochart triangle in pad coordinate space [-1, 1] x [-1, 1].
pub const V_MESO: Vec2 = Vec2::new(0.0, 1.0);
pub const V_ENDO: Vec2 = Vec2::new(-1.0, -0.866);
pub const V_ECTO: Vec2 = Vec2::new(1.0, -0.866);

/// Heath-Carter somatotype barycentric coordinates $(S_{\text{endo}}, S_{\text{meso}}, S_{\text{ecto}}) \in [0, 1]^3$.
/// Governs global macro corpulency, muscularity, and thinness.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SomatotypeCoords {
    /// Endomorphy: Adiposity, corpulency, soft roundness [0.0 .. 1.0]
    pub endomorph: f32,
    /// Mesomorphy: Musculoskeletal athletic hypertrophy, V-taper [0.0 .. 1.0]
    pub mesomorph: f32,
    /// Ectomorphy: Linearity, bone prominence, slender frame [0.0 .. 1.0]
    pub ectomorph: f32,
}

impl Default for SomatotypeCoords {
    fn default() -> Self {
        // Balanced central somatotype (neutral average)
        Self {
            endomorph: 1.0 / 3.0,
            mesomorph: 1.0 / 3.0,
            ectomorph: 1.0 / 3.0,
        }
    }
}

impl SomatotypeCoords {
    pub fn new(endomorph: f32, mesomorph: f32, ectomorph: f32) -> Self {
        Self {
            endomorph,
            mesomorph,
            ectomorph,
        }
    }

    /// Normalizes the three components so their sum equals 1.0.
    pub fn normalized(&self) -> Self {
        let e = self.endomorph.max(0.0);
        let m = self.mesomorph.max(0.0);
        let ec = self.ectomorph.max(0.0);
        let sum = e + m + ec;
        if sum > 1e-6 {
            Self {
                endomorph: e / sum,
                mesomorph: m / sum,
                ectomorph: ec / sum,
            }
        } else {
            Self::default()
        }
    }

    /// Solves barycentric coordinates from 2D pad coordinates $(x, y) \in [-1, 1]^2$.
    /// Uses Cramer's rule for point-in-triangle projection with boundary clamping.
    pub fn from_pad_2d(x: f32, y: f32) -> Self {
        let p = Vec2::new(x, y);

        let v0 = V_ENDO;
        let v1 = V_MESO;
        let v2 = V_ECTO;

        let denom = (v1.y - v2.y) * (v0.x - v2.x) + (v2.x - v1.x) * (v0.y - v2.y);
        if denom.abs() < 1e-7 {
            return Self::default();
        }

        let lambda0 = ((v1.y - v2.y) * (p.x - v2.x) + (v2.x - v1.x) * (p.y - v2.y)) / denom;
        let lambda1 = ((v2.y - v0.y) * (p.x - v2.x) + (v0.x - v2.x) * (p.y - v2.y)) / denom;
        let lambda2 = 1.0 - lambda0 - lambda1;

        // Clamp to simplex
        let raw = Self {
            endomorph: lambda0.max(0.0),
            mesomorph: lambda1.max(0.0),
            ectomorph: lambda2.max(0.0),
        };

        raw.normalized()
    }

    /// Converts barycentric somatotype coordinates to 2D pad coordinates $(x, y) \in [-1, 1]^2$.
    pub fn to_pad_2d(&self) -> (f32, f32) {
        let norm = self.normalized();
        let p = norm.endomorph * V_ENDO + norm.mesomorph * V_MESO + norm.ectomorph * V_ECTO;
        (p.x, p.y)
    }
}

/// Macro character morphology state combining Heath-Carter somatotype with continuous gender dimorphism.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SomatotypeState {
    pub coords: SomatotypeCoords,
    /// Continuous gender transition:
    /// 0.0 = Canonical Female
    /// 1.0 = Canonical Male
    /// 0.5 = Androgynous / Unisex
    pub gender_dimorphism: f32,
}

impl Default for SomatotypeState {
    fn default() -> Self {
        Self {
            coords: SomatotypeCoords::default(),
            gender_dimorphism: 1.0, // Default masculine canonical base
        }
    }
}

impl SomatotypeState {
    pub fn new(coords: SomatotypeCoords, gender_dimorphism: f32) -> Self {
        Self {
            coords: coords.normalized(),
            gender_dimorphism: gender_dimorphism.clamp(0.0, 1.0),
        }
    }

    /// Evaluates continuous interpolation between isomorphic canonical Male and Female base meshes.
    ///
    /// Preserves 1:1 topology, face indices, UV layouts, joint indices, and bone weights while
    /// performing continuous geometric morphing of vertex positions, outward normals and vertex AO colors.
    pub fn interpolate_canonical_mesh(
        &self,
        male_mesh: &Mesh,
        female_mesh: &Mesh,
    ) -> Result<Mesh, String> {
        if male_mesh.vertices.len() != female_mesh.vertices.len() {
            return Err(format!(
                "Mesh vertex count mismatch: male has {}, female has {}",
                male_mesh.vertices.len(),
                female_mesh.vertices.len()
            ));
        }

        let g = self.gender_dimorphism;
        let mut morphed_vertices = Vec::with_capacity(male_mesh.vertices.len());

        for (v_m, v_f) in male_mesh.vertices.iter().zip(female_mesh.vertices.iter()) {
            let pos = [
                (1.0 - g) * v_f.position[0] + g * v_m.position[0],
                (1.0 - g) * v_f.position[1] + g * v_m.position[1],
                (1.0 - g) * v_f.position[2] + g * v_m.position[2],
            ];

            let n_raw = [
                (1.0 - g) * v_f.normal[0] + g * v_m.normal[0],
                (1.0 - g) * v_f.normal[1] + g * v_m.normal[1],
                (1.0 - g) * v_f.normal[2] + g * v_m.normal[2],
            ];
            let len_sq = n_raw[0] * n_raw[0] + n_raw[1] * n_raw[1] + n_raw[2] * n_raw[2];
            let normal = if len_sq > 1e-12 {
                let inv_len = 1.0 / len_sq.sqrt();
                [n_raw[0] * inv_len, n_raw[1] * inv_len, n_raw[2] * inv_len]
            } else {
                v_m.normal
            };

            let color = [
                (1.0 - g) * v_f.color[0] + g * v_m.color[0],
                (1.0 - g) * v_f.color[1] + g * v_m.color[1],
                (1.0 - g) * v_f.color[2] + g * v_m.color[2],
                (1.0 - g) * v_f.color[3] + g * v_m.color[3],
            ];

            morphed_vertices.push(Vertex {
                position: pos,
                normal,
                uv: v_m.uv,
                color,
                joints: v_m.joints,
                weights: v_m.weights,
            });
        }

        Ok(Mesh {
            name: format!("CanonicalMorphed_Gender_{:.2}", g),
            vertices: morphed_vertices,
            indices: male_mesh.indices.clone(),
            custom_attributes: male_mesh.custom_attributes.clone(),
        })
    }

    /// Resolves canonical macro morph slider weights for high-level UI synchronization.
    pub fn resolve_macro_sliders(&self) -> Vec<(&'static str, f32)> {
        let c = self.coords.normalized();
        vec![
            ("somatotype_endomorph", c.endomorph),
            ("somatotype_mesomorph", c.mesomorph),
            ("somatotype_ectomorph", c.ectomorph),
            ("gender_dimorphism", self.gender_dimorphism),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::BaseGender;

    #[test]
    fn test_somatotype_barycentric_triangle_roundtrip() {
        // Test primary vertices
        let (x_meso, y_meso) = SomatotypeCoords::new(0.0, 1.0, 0.0).to_pad_2d();
        assert!((x_meso - V_MESO.x).abs() < 1e-5);
        assert!((y_meso - V_MESO.y).abs() < 1e-5);

        let from_meso = SomatotypeCoords::from_pad_2d(V_MESO.x, V_MESO.y);
        assert!((from_meso.mesomorph - 1.0).abs() < 1e-4);
        assert!(from_meso.endomorph < 1e-4);
        assert!(from_meso.ectomorph < 1e-4);

        let (x_endo, y_endo) = SomatotypeCoords::new(1.0, 0.0, 0.0).to_pad_2d();
        assert!((x_endo - V_ENDO.x).abs() < 1e-5);
        assert!((y_endo - V_ENDO.y).abs() < 1e-5);

        let from_endo = SomatotypeCoords::from_pad_2d(V_ENDO.x, V_ENDO.y);
        assert!((from_endo.endomorph - 1.0).abs() < 1e-4);
        assert!(from_endo.mesomorph < 1e-4);
        assert!(from_endo.ectomorph < 1e-4);

        let (x_ecto, y_ecto) = SomatotypeCoords::new(0.0, 0.0, 1.0).to_pad_2d();
        assert!((x_ecto - V_ECTO.x).abs() < 1e-5);
        assert!((y_ecto - V_ECTO.y).abs() < 1e-5);

        let from_ecto = SomatotypeCoords::from_pad_2d(V_ECTO.x, V_ECTO.y);
        assert!((from_ecto.ectomorph - 1.0).abs() < 1e-4);
        assert!(from_ecto.mesomorph < 1e-4);
        assert!(from_ecto.endomorph < 1e-4);

        // Test central point
        let center = SomatotypeCoords::new(1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0);
        let (cx, cy) = center.to_pad_2d();
        let from_center = SomatotypeCoords::from_pad_2d(cx, cy);
        assert!((from_center.endomorph - 1.0 / 3.0).abs() < 1e-4);
        assert!((from_center.mesomorph - 1.0 / 3.0).abs() < 1e-4);
        assert!((from_center.ectomorph - 1.0 / 3.0).abs() < 1e-4);
    }

    #[test]
    fn test_continuous_gender_dimorphism_mesh_interpolation() {
        let male = Mesh::create_canonical_base(BaseGender::Male);
        let female = Mesh::create_canonical_base(BaseGender::Female);

        // 1. g = 0.0 -> matches female base
        let state_female = SomatotypeState::new(SomatotypeCoords::default(), 0.0);
        let interp_f = state_female
            .interpolate_canonical_mesh(&male, &female)
            .unwrap();
        assert_eq!(interp_f.vertices.len(), female.vertices.len());
        for (iv, fv) in interp_f.vertices.iter().zip(female.vertices.iter()) {
            for axis in 0..3 {
                assert!((iv.position[axis] - fv.position[axis]).abs() < 1e-5);
                assert!((iv.normal[axis] - fv.normal[axis]).abs() < 1e-5);
            }
        }

        // 2. g = 1.0 -> matches male base
        let state_male = SomatotypeState::new(SomatotypeCoords::default(), 1.0);
        let interp_m = state_male.interpolate_canonical_mesh(&male, &female).unwrap();
        assert_eq!(interp_m.vertices.len(), male.vertices.len());
        for (iv, mv) in interp_m.vertices.iter().zip(male.vertices.iter()) {
            for axis in 0..3 {
                assert!((iv.position[axis] - mv.position[axis]).abs() < 1e-5);
                assert!((iv.normal[axis] - mv.normal[axis]).abs() < 1e-5);
            }
        }

        // 3. g = 0.5 -> intermediate androgynous base
        let state_androgynous = SomatotypeState::new(SomatotypeCoords::default(), 0.5);
        let interp_mid = state_androgynous
            .interpolate_canonical_mesh(&male, &female)
            .unwrap();
        assert_eq!(interp_mid.vertices.len(), male.vertices.len());
        for (i, ((iv, mv), fv)) in interp_mid
            .vertices
            .iter()
            .zip(male.vertices.iter())
            .zip(female.vertices.iter())
            .enumerate()
        {
            for axis in 0..3 {
                let expected_pos = 0.5 * (mv.position[axis] + fv.position[axis]);
                assert!(
                    (iv.position[axis] - expected_pos).abs() < 1e-5,
                    "Vertex {} axis {} mismatch",
                    i,
                    axis
                );
            }
            // Check normalized outward normals
            let norm_len = (iv.normal[0] * iv.normal[0]
                + iv.normal[1] * iv.normal[1]
                + iv.normal[2] * iv.normal[2])
                .sqrt();
            assert!((norm_len - 1.0).abs() < 1e-4);
        }
    }
}
