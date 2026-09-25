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
    // Fase 2 (#18): material anime VRoid/MToon — emission sub-color,
    // intensidades/offsets do segundo shade, matcap (sphere add) e os flags
    // de habilitação dos 5 slots de textura (main/shade/second/emission/sphere).
    pub emission_color: [f32; 4],
    pub params4: [f32; 4], // emission_intensity, second_shade_shift, second_shade_softness, matcap_intensity
    pub params5: [f32; 4], // main_tex, shade_tex, second_shade, emission (0/1)
    pub params6: [f32; 4], // matcap_enabled, matcap_mode (0 mult/1 add), shade_toony, reserved
    // Fase 2 (#17): sombra facial SDF — offset do threshold, suavidade,
    // ativação (o mapa SDF é o binding 16 do cel, ancorado no neutro 1x1).
    pub params7: [f32; 4], // face_shadow_offset, face_shadow_smoothness, face_sdf_enabled, reserved
    // Fase 2 (#43): olho anime — recalada do parallax da íris, intensidade
    // dos highlights desenhados à mão (desacoplados da luz) e ativação.
    // (Os settings do solver de olhar são CPU — anigo-ik/eye_tracking.ts.)
    pub params8: [f32; 4], // eye_depth_scale, eye_highlight_intensity, eye_enabled, reserved
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
            // MToon off por padrão: sem emissão, sem matcap, sem texturas —
            // o frame congelado do render contract mantém 28 floats idênticos.
            emission_color: [0.0, 0.0, 0.0, 0.0],
            params4: [0.0, 0.0, 0.05, 0.0],
            params5: [0.0, 0.0, 0.0, 0.0],
            params6: [0.0, 0.0, 1.0, 0.0],
            // Fase 2 (#17): SDF facial off por padrão (frame congelado intacto)
            params7: [0.0, 0.05, 0.0, 0.0],
            // Fase 2 (#43): olho anime off por padrão (frame congelado intacto)
            params8: [0.0, 0.0, 0.0, 0.0],
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
            // Fase 2 (#18): slots MToon. Texturas desabilitadas → o renderer
            // ancora o neutro 1x1 branco e o shader ignora o slot.
            emission_color: m.mtoon_emission_color,
            params4: [
                m.mtoon_emission_intensity,
                m.mtoon_second_shade_shift,
                m.mtoon_second_shade_softness,
                m.mtoon_matcap_intensity,
            ],
            params5: [
                (m.mtoon_main_texture_enabled as u32) as f32,
                (m.mtoon_shade_texture_enabled as u32) as f32,
                (m.mtoon_second_shade_texture_enabled as u32) as f32,
                (m.mtoon_emission_texture_enabled as u32) as f32,
            ],
            params6: [
                (m.mtoon_matcap_enabled as u32) as f32,
                m.mtoon_matcap_mode as f32,
                (m.mtoon_shade_toony as u32) as f32,
                0.0,
            ],
            // Fase 2 (#17): sombra facial SDF
            params7: [
                m.face_shadow_offset,
                m.face_shadow_smoothness,
                (m.face_sdf_enabled as u32) as f32,
                0.0,
            ],
            // Fase 2 (#43): olho anime (parallax + highlights)
            params8: [
                m.eye_depth_scale,
                m.eye_highlight_intensity,
                (m.eye_enabled as u32) as f32,
                0.0,
            ],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct OutlineUniform {
    pub color: [f32; 4],
    pub params: [f32; 4],  // x: width, y: aspect, z: depth_bias, w: opacity
    // x: smoothness, y: width_tex_enabled (Fase 2 #18 — mapa de espessura MToon),
    // z/w: reserved (P0-09 dead control fix)
    pub params2: [f32; 4],
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

/// Fase 2 (#53): Depth of Field cinematográfico (Anime Bokeh DoF) — 48 B,
/// layout do bloco `dof` do contrato. O MESMO empacotamento roda no
/// viewport (`dofUniformFloats` em render_uniforms.ts); o passe só existe
/// quando `Scene.dof.enabled`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct DofUniform {
    // x: focus_distance (m), y: f_number, z: bokeh_shape (0 círculo/1 hex), w: focal_m (m)
    pub params: [f32; 4],
    // x: width_px, y: height_px, z: z_near (m), w: z_far (m)
    pub resolution: [f32; 4],
    // x: max_radius_px, y: sensor_height_m (0.024), z/w: reserved
    pub limits: [f32; 4],
}

impl DofUniform {
    /// Empacota os settings da cena + dimensões/planos da câmera nos 48 B
    /// do bloco `dof` (a ordem dos floats é a do `postprocess_dof.wgsl`).
    pub fn from_settings(
        focus_distance: f32,
        f_number: f32,
        bokeh_shape: u32,
        focal_mm: f32,
        max_radius_px: f32,
        width_px: u32,
        height_px: u32,
        z_near: f32,
        z_far: f32,
    ) -> Self {
        Self {
            params: [focus_distance, f_number, bokeh_shape as f32, focal_mm / 1000.0],
            resolution: [width_px as f32, height_px as f32, z_near, z_far],
            limits: [max_radius_px, 0.024, 0.0, 0.0],
        }
    }
}

impl Default for DofUniform {
    fn default() -> Self {
        Self {
            params: [2.0, 2.0, 0.0, 0.05],
            resolution: [1.0, 1.0, 0.05, 100.0],
            limits: [16.0, 0.024, 0.0, 0.0],
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
    ///
    /// O que a fatia não cobre fica com a matriz **identidade**, não com zeros:
    /// uma paleta curta (por exemplo um buffer de tamanho errado) somaria zeros
    /// e colapsaria o vértice para a origem, enquanto a identidade mantém o
    /// vértice no lugar (mesma escolha da paleta neutra de `skinning`).
    pub fn from_floats(floats: &[f32]) -> Self {
        let mut uniform = Self::identity(MAX_PALETTE_JOINTS);
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
        // P2-14 Camera 208 B (added model+normal_mat), Light 80 B (added sky/ground),
        // Material 208 B (Fase 2 #18: +4 vec4s de MToon; #17: +params7 face SDF;
        // #43: +params8 olho anime — 52 floats)
        assert_eq!(std::mem::size_of::<CameraUniform>(), 208);
        assert_eq!(std::mem::size_of::<CameraUniform>() % 16, 0);

        assert_eq!(std::mem::size_of::<LightUniform>(), 80);
        assert_eq!(std::mem::size_of::<LightUniform>() % 16, 0);

        assert_eq!(std::mem::size_of::<MaterialUniform>(), 208);
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
