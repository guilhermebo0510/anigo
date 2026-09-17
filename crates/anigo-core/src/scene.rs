use crate::math::{Camera, Transform};
use crate::mesh::Mesh;
use serde::{Deserialize, Serialize};

/// Stylized Anime Directional Light with Hue-Shifting parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StylizedLight {
    pub direction: [f32; 3],
    pub color: [f32; 3],
    pub intensity: f32,
    /// Stylized cool/warm hue-shifted shadow tint
    pub shadow_color: [f32; 3],
    pub ambient_intensity: f32,
}

impl Default for StylizedLight {
    fn default() -> Self {
        Self {
            direction: [0.577, 0.577, 0.577], // normalized (1, 1, 1)
            color: [1.0, 0.98, 0.95],
            intensity: 1.0,
            shadow_color: [0.65, 0.68, 0.85], // cool anime lavender/blue shadow
            ambient_intensity: 0.35,
        }
    }
}

/// Stylized Material parameters for Anime NPR Cel-Shading.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StylizedMaterial {
    pub name: String,
    pub base_color: [f32; 4],
    pub shade_color: [f32; 4],
    pub outline_color: [f32; 4],
    pub outline_width: f32,
    pub shadow_threshold: f32,
    pub shadow_smoothness: f32,
}

impl Default for StylizedMaterial {
    fn default() -> Self {
        Self {
            name: "DefaultAnimeMaterial".into(),
            base_color: [0.98, 0.92, 0.85, 1.0], // warm skin/cloth tone
            shade_color: [0.82, 0.73, 0.78, 1.0], // soft anime shadow tint
            outline_color: [0.25, 0.15, 0.20, 1.0], // dark anime lineart color
            outline_width: 0.0035,
            shadow_threshold: 0.50,
            shadow_smoothness: 0.04,
        }
    }
}

/// A node within the hierarchical scene graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneNode {
    pub id: String,
    pub name: String,
    pub transform: Transform,
    pub mesh: Option<Mesh>,
    pub material: Option<StylizedMaterial>,
    pub visible: bool,
}

impl SceneNode {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            transform: Transform::default(),
            mesh: None,
            material: Some(StylizedMaterial::default()),
            visible: true,
        }
    }

    pub fn with_mesh(mut self, mesh: Mesh) -> Self {
        self.mesh = Some(mesh);
        self
    }
}

/// Complete Scene representation containing nodes, camera, lighting, and global parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    pub nodes: Vec<SceneNode>,
    pub camera: Camera,
    pub light: StylizedLight,
    pub background_color: [f32; 4],
}

impl Default for Scene {
    fn default() -> Self {
        let mannequin_node = SceneNode::new("mannequin_proxy", "Anime Mannequin")
            .with_mesh(Mesh::create_mannequin_proxy());

        Self {
            nodes: vec![mannequin_node],
            camera: Camera::default(),
            light: StylizedLight::default(),
            background_color: [0.12, 0.13, 0.16, 1.0], // modern dark studio background
        }
    }
}

impl Scene {
    pub fn new_empty() -> Self {
        Self {
            nodes: Vec::new(),
            camera: Camera::default(),
            light: StylizedLight::default(),
            background_color: [0.12, 0.13, 0.16, 1.0],
        }
    }

    pub fn add_node(&mut self, node: SceneNode) {
        self.nodes.push(node);
    }

    pub fn get_node_mut(&mut self, id: &str) -> Option<&mut SceneNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    pub fn total_vertices(&self) -> usize {
        self.nodes.iter().filter_map(|n| n.mesh.as_ref()).map(|m| m.vertices.len()).sum()
    }

    pub fn total_triangles(&self) -> usize {
        self.nodes.iter().filter_map(|n| n.mesh.as_ref()).map(|m| m.indices.len() / 3).sum()
    }
}
