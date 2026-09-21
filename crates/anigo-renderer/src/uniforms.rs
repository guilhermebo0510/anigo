use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct CameraUniform {
    pub view_proj: [f32; 16],
    pub camera_pos: [f32; 4],
    pub model: [f32; 16],      // P2-14 model matrix
    pub normal_mat: [f32; 16], // P2-14 normal matrix (mat4 padded)
}

impl Default for CameraUniform {
    fn default() -> Self {
        Self {
            view_proj: [0.0; 16],
            camera_pos: [0.0, 0.0, 0.0, 1.0],
            model: [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0],
            normal_mat: [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct LightUniform {
    pub direction: [f32; 4],
    pub color: [f32; 4],
    pub shadow_color: [f32; 4], // rgb: shadow tint, w: saturation multiplier (P0-02 unified neutral)
    pub ambient_sky: [f32; 4],   // P2-04 sky hemisphere
    pub ambient_ground: [f32; 4],// P2-04 ground hemisphere
}

impl Default for LightUniform {
    fn default() -> Self {
        Self {
            direction: [0.577, 0.577, 0.577, 1.0],
            color: [1.0, 0.98, 0.95, 0.35],
            shadow_color: [1.0, 1.0, 1.0, 1.0], // P0-02 neutral white (was 0.65,0.68,0.85,1.0 double tint)
            ambient_sky: [0.52, 0.60, 0.78, 1.0],
            ambient_ground: [0.25, 0.20, 0.18, 1.0],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct MaterialUniform {
    pub base_color: [f32; 4],
    pub shade_color: [f32; 4],
    pub specular_color: [f32; 4],
    pub rim_color: [f32; 4],
    pub params: [f32; 4],
    pub params2: [f32; 4],
    pub params3: [f32; 4],
}

impl Default for MaterialUniform {
    fn default() -> Self {
        Self {
            base_color: [0.98, 0.92, 0.85, 1.0],
            shade_color: [0.82, 0.73, 0.78, 1.0],
            specular_color: [1.0, 1.0, 1.0, 1.0],
            rim_color: [0.576, 0.773, 0.992, 1.0],
            params: [0.50, 0.02, 0.40, 32.0],
            params2: [0.80, 0.40, -15.0f32.to_radians(), 1.0],
            params3: [0.05, 0.0, 0.45, 0.85], // P2-07 spec_size 0.45, P2-05 ao_intensity 0.85
        }
    }
}

impl From<&anigo_core::StylizedMaterial> for MaterialUniform {
    fn from(m: &anigo_core::StylizedMaterial) -> Self {
        Self {
            base_color: m.base_color,
            shade_color: m.shade_color,
            specular_color: m.specular_color,
            rim_color: m.rim_color,
            params: [
                m.shadow_threshold,
                m.shadow_smoothness,
                m.spec_intensity,
                m.spec_power,
            ],
            params2: [
                m.rim_intensity,
                m.rim_spread,
                m.hue_shift.to_radians(),
                m.toon_steps,
            ],
            params3: [m.specular_softness, m.specular_offset, m.specular_size, m.ao_intensity],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct OutlineUniform {
    pub color: [f32; 4],
    pub params: [f32; 4],  // x: width, y: aspect, z: depth_bias, w: opacity
    pub params2: [f32; 4], // x: smoothness, yzw: unused (P0-09 dead control fix)
}

impl Default for OutlineUniform {
    fn default() -> Self {
        Self {
            color: [0.25, 0.15, 0.20, 1.0],
            params: [0.0035, 1.0, 0.0, 1.0],
            params2: [0.0, 0.0, 0.0, 0.0],
        }
    }
}

/// P1-04: paleta de skinning — `joint_count` matrizes de 16 floats, no layout
/// do bloco `bones` do contrato (`array<mat4x4<f32>, 24>` = 1536 B).
pub const MAX_PALETTE_JOINTS: usize = 24;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct BonePaletteUniform {
    pub matrices: [[f32; 16]; MAX_PALETTE_JOINTS],
}

impl BonePaletteUniform {
    /// Paleta neutra: `Σ wᵢ · (I · p) = p` — não deforma nada.
    pub fn identity(joint_count: usize) -> Self {
        let mut matrices = [[0.0f32; 16]; MAX_PALETTE_JOINTS];
        let count = joint_count.min(MAX_PALETTE_JOINTS);
        for matrix in matrices.iter_mut().take(count) {
            matrix[0] = 1.0;
            matrix[5] = 1.0;
            matrix[10] = 1.0;
            matrix[15] = 1.0;
        }
        Self { matrices }
    }

    /// Constrói a partir de `joint_count * 16` floats (ignora o excedente).
    pub fn from_floats(floats: &[f32]) -> Self {
        let mut uniform = Self::identity(0);
        for (index, matrix) in uniform.matrices.iter_mut().enumerate() {
            let start = index * 16;
            if start + 16 > floats.len() {
                break;
            }
            matrix.copy_from_slice(&floats[start..start + 16]);
        }
        uniform
    }

    /// Achata de volta (o que é enviado ao buffer de GPU).
    pub fn to_floats(&self) -> Vec<f32> {
        self.matrices.iter().flat_map(|matrix| matrix.iter().copied()).collect()
    }

    pub fn stride_bytes() -> usize {
        std::mem::size_of::<Self>()
    }
}

impl Default for BonePaletteUniform {
    fn default() -> Self {
        Self::identity(MAX_PALETTE_JOINTS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uniform_sizes_and_alignments() {
        // P2-14 Camera 208 B (added model+normal_mat), Light 80 B (added sky/ground), Material 112 B
        assert_eq!(std::mem::size_of::<CameraUniform>(), 208);
        assert_eq!(std::mem::size_of::<CameraUniform>() % 16, 0);

        assert_eq!(std::mem::size_of::<LightUniform>(), 80);
        assert_eq!(std::mem::size_of::<LightUniform>() % 16, 0);

        assert_eq!(std::mem::size_of::<MaterialUniform>(), 112);
        assert_eq!(std::mem::size_of::<MaterialUniform>() % 16, 0);

        assert_eq!(std::mem::size_of::<OutlineUniform>(), 48);
        assert_eq!(std::mem::size_of::<OutlineUniform>() % 16, 0);
    }

    #[test]
    fn test_bone_palette_uniform_matches_contract_size() {
        // 24 matrizes × 64 B = 1536 B, o mesmo `uniforms.bones.size` do contrato
        assert_eq!(BonePaletteUniform::stride_bytes(), 1536);
        assert_eq!(BonePaletteUniform::stride_bytes() % 16, 0);
        let identity = BonePaletteUniform::identity(MAX_PALETTE_JOINTS);
        let floats = identity.to_floats();
        assert_eq!(floats.len(), MAX_PALETTE_JOINTS * 16);
        for matrix in floats.chunks_exact(16) {
            assert_eq!(matrix, [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
        }
        // ida e volta mantém os valores
        let round_trip = BonePaletteUniform::from_floats(&floats);
        assert_eq!(round_trip.to_floats(), floats);
        // paleta curta não estoura e devolve o resto neutro
        let short = BonePaletteUniform::from_floats(&[2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
        assert_eq!(short.matrices[0][0], 2.0);
        assert_eq!(short.matrices[1][0], 1.0);
        assert_eq!(short.matrices[1][5], 1.0);
    }

    #[test]
    fn test_material_uniform_from_core_material() {
        let core_mat = anigo_core::StylizedMaterial {
            shadow_threshold: 0.58,
            spec_intensity: 0.75,
            hue_shift: -30.0,
            ..anigo_core::StylizedMaterial::default()
        };

        let u = MaterialUniform::from(&core_mat);
        assert_eq!(u.params[0], 0.58);
        assert_eq!(u.params[2], 0.75);
        assert!((u.params2[2] - (-30.0f32.to_radians())).abs() < 1e-5);
    }
}
