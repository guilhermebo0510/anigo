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
    pub shadow_color: [f32; 4],
}

impl Default for LightUniform {
    fn default() -> Self {
        Self {
            direction: [0.577, 0.577, 0.577, 1.0],
            color: [1.0, 0.98, 0.95, 0.35],
            shadow_color: [0.65, 0.68, 0.85, 1.0],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct MaterialUniform {
    pub base_color: [f32; 4],
    pub shade_color: [f32; 4],
    pub params: [f32; 4],
}

impl Default for MaterialUniform {
    fn default() -> Self {
        Self {
            base_color: [0.98, 0.92, 0.85, 1.0],
            shade_color: [0.82, 0.73, 0.78, 1.0],
            params: [0.50, 0.04, 0.0, 0.0],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct OutlineUniform {
    pub color: [f32; 4],
    pub params: [f32; 4],
}

impl Default for OutlineUniform {
    fn default() -> Self {
        Self {
            color: [0.25, 0.15, 0.20, 1.0],
            params: [0.0035, 1.0, 0.0, 0.0],
        }
    }
}
