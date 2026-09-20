use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct CameraUniform {
    pub view_proj: [f32; 16],
    pub camera_pos: [f32; 4],
}

impl Default for CameraUniform {
    fn default() -> Self {
        Self {
            view_proj: [0.0; 16],
            camera_pos: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct LightUniform {
    pub direction: [f32; 4],
    pub color: [f32; 4],
    pub shadow_color: [f32; 4], // rgb: shadow tint, w: saturation multiplier (P0-02 unified neutral)
}

impl Default for LightUniform {
    fn default() -> Self {
        Self {
            direction: [0.577, 0.577, 0.577, 1.0],
            color: [1.0, 0.98, 0.95, 0.35],
            shadow_color: [1.0, 1.0, 1.0, 1.0], // P0-02 neutral white (was 0.65,0.68,0.85,1.0 double tint)
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
            params3: [0.05, 0.0, 0.0, 0.0],
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
            params3: [m.specular_softness, m.specular_offset, 0.0, 0.0],
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uniform_sizes_and_alignments() {
        assert_eq!(std::mem::size_of::<CameraUniform>(), 80);
        assert_eq!(std::mem::size_of::<CameraUniform>() % 16, 0);

        assert_eq!(std::mem::size_of::<LightUniform>(), 48);
        assert_eq!(std::mem::size_of::<LightUniform>() % 16, 0);

        // P0-04/09: Material 112 B (added rim_color) and Outline 48 B (added smoothness)
        assert_eq!(std::mem::size_of::<MaterialUniform>(), 112);
        assert_eq!(std::mem::size_of::<MaterialUniform>() % 16, 0);

        assert_eq!(std::mem::size_of::<OutlineUniform>(), 48);
        assert_eq!(std::mem::size_of::<OutlineUniform>() % 16, 0);
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
