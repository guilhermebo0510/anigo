use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};

/// Standard Anime NPR vertex definition with channels tailored for Cel-Shading & Inverted Hull.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, Serialize, Deserialize)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// Anime vertex attribute encoding:
    /// R: Stylized Ambient Occlusion
    /// G: Shadow threshold shift
    /// B: Outline extrusion width multiplier
    /// A: Specular / Rim light mask
    pub color: [f32; 4],
}

impl Vertex {
    pub fn new(position: [f32; 3], normal: [f32; 3], uv: [f32; 2]) -> Self {
        Self {
            position,
            normal,
            uv,
            color: [1.0, 0.5, 1.0, 1.0],
        }
    }

    pub fn with_color(position: [f32; 3], normal: [f32; 3], uv: [f32; 2], color: [f32; 4]) -> Self {
        Self {
            position,
            normal,
            uv,
            color,
        }
    }
}

/// 3D Geometry mesh consisting of indexed vertices.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mesh {
    pub name: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn new(name: impl Into<String>, vertices: Vec<Vertex>, indices: Vec<u32>) -> Self {
        Self {
            name: name.into(),
            vertices,
            indices,
        }
    }

    /// Creates a unit cube with distinct normals per face.
    pub fn create_cube(size: f32) -> Self {
        let h = size * 0.5;
        let mut vertices = Vec::new();
        let mut indices = Vec::new();

        let faces = [
            // Front (+Z)
            ([0.0, 0.0, 1.0], [[-h, -h, h], [h, -h, h], [h, h, h], [-h, h, h]]),
            // Back (-Z)
            ([0.0, 0.0, -1.0], [[h, -h, -h], [-h, -h, -h], [-h, h, -h], [h, h, -h]]),
            // Top (+Y)
            ([0.0, 1.0, 0.0], [[-h, h, h], [h, h, h], [h, h, -h], [-h, h, -h]]),
            // Bottom (-Y)
            ([0.0, -1.0, 0.0], [[-h, -h, -h], [h, -h, -h], [h, -h, h], [-h, -h, h]]),
            // Right (+X)
            ([1.0, 0.0, 0.0], [[h, -h, h], [h, -h, -h], [h, h, -h], [h, h, h]]),
            // Left (-X)
            ([-1.0, 0.0, 0.0], [[-h, -h, -h], [-h, -h, h], [-h, h, h], [-h, h, -h]]),
        ];

        let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];

        for (normal, corners) in faces {
            let base_idx = vertices.len() as u32;
            for (i, &corner) in corners.iter().enumerate() {
                vertices.push(Vertex::with_color(
                    corner,
                    normal,
                    uvs[i],
                    [1.0, 0.5, 1.0, 1.0],
                ));
            }
            indices.extend_from_slice(&[
                base_idx, base_idx + 1, base_idx + 2,
                base_idx, base_idx + 2, base_idx + 3,
            ]);
        }

        Self::new("Cube", vertices, indices)
    }

    /// Creates a stylized Anime Mannequin proxy consisting of head, torso, limbs, and joints.
    pub fn create_mannequin_proxy() -> Self {
        let mut combined_vertices = Vec::new();
        let mut combined_indices = Vec::new();

        let segments = [
            // Head (Sphere/Elongated Box)
            ([0.0, 1.65, 0.0], [0.28, 0.32, 0.28], [1.0, 0.48, 1.2, 1.0]),
            // Neck
            ([0.0, 1.45, 0.0], [0.10, 0.12, 0.10], [0.9, 0.50, 1.0, 1.0]),
            // Torso (Upper Chest)
            ([0.0, 1.25, 0.0], [0.38, 0.30, 0.24], [1.0, 0.50, 1.0, 1.0]),
            // Waist / Pelvis
            ([0.0, 0.95, 0.0], [0.34, 0.25, 0.22], [1.0, 0.50, 1.0, 1.0]),
            // Left Upper Arm
            ([-0.28, 1.20, 0.0], [0.10, 0.32, 0.10], [0.95, 0.50, 1.0, 0.8]),
            // Right Upper Arm
            ([0.28, 1.20, 0.0], [0.10, 0.32, 0.10], [0.95, 0.50, 1.0, 0.8]),
            // Left Forearm
            ([-0.28, 0.82, 0.0], [0.08, 0.30, 0.08], [0.95, 0.50, 1.0, 0.8]),
            // Right Forearm
            ([0.28, 0.82, 0.0], [0.08, 0.30, 0.08], [0.95, 0.50, 1.0, 0.8]),
            // Left Thigh
            ([-0.12, 0.65, 0.0], [0.14, 0.38, 0.14], [1.0, 0.50, 1.0, 0.9]),
            // Right Thigh
            ([0.12, 0.65, 0.0], [0.14, 0.38, 0.14], [1.0, 0.50, 1.0, 0.9]),
            // Left Calf
            ([-0.12, 0.25, 0.0], [0.11, 0.40, 0.11], [1.0, 0.50, 1.0, 0.9]),
            // Right Calf
            ([0.12, 0.25, 0.0], [0.11, 0.40, 0.11], [1.0, 0.50, 1.0, 0.9]),
            // Ground Pedestal (Shadow receiver)
            ([0.0, -0.02, 0.0], [1.2, 0.04, 1.2], [0.7, 0.50, 0.0, 0.3]),
        ];

        for (center, dims, color) in segments {
            let sub_cube = Self::create_cube(1.0);
            let base_idx = combined_vertices.len() as u32;

            for v in sub_cube.vertices {
                let pos = [
                    center[0] + v.position[0] * dims[0],
                    center[1] + v.position[1] * dims[1],
                    center[2] + v.position[2] * dims[2],
                ];
                combined_vertices.push(Vertex::with_color(pos, v.normal, v.uv, color));
            }

            for idx in sub_cube.indices {
                combined_indices.push(base_idx + idx);
            }
        }

        Self::new("AnimeMannequinProxy", combined_vertices, combined_indices)
    }

    /// Creates a smooth UV sphere for testing cel-shading light ramps and smooth normals.
    pub fn create_uv_sphere(radius: f32, rings: u32, sectors: u32) -> Self {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();

        for r in 0..=rings {
            let v = r as f32 / rings as f32;
            let theta = v * std::f32::consts::PI;

            for s in 0..=sectors {
                let u = s as f32 / sectors as f32;
                let phi = u * 2.0 * std::f32::consts::PI;

                let x = theta.sin() * phi.cos();
                let y = theta.cos();
                let z = theta.sin() * phi.sin();

                vertices.push(Vertex::with_color(
                    [x * radius, y * radius, z * radius],
                    [x, y, z],
                    [u, v],
                    [1.0, 0.5, 1.0, 1.0],
                ));
            }
        }

        for r in 0..rings {
            for s in 0..sectors {
                let first = r * (sectors + 1) + s;
                let second = first + sectors + 1;

                indices.push(first);
                indices.push(second);
                indices.push(first + 1);

                indices.push(second);
                indices.push(second + 1);
                indices.push(first + 1);
            }
        }

        Self::new("UvSphere", vertices, indices)
    }
}
