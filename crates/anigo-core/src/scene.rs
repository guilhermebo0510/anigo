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
    /// Anisotropic specular highlight intensity (0.0 to 2.0)
    pub spec_intensity: f32,
    /// Specular power / sharpness exponent (4.0 to 128.0)
    pub spec_power: f32,
    /// Stylized Fresnel rim lighting intensity (0.0 to 3.0)
    pub rim_intensity: f32,
    /// Rim light spread / angular width (0.05 to 1.0)
    pub rim_spread: f32,
    /// Mathematical shadow hue rotation in degrees (-180 to +180)
    pub hue_shift: f32,
    /// Toon ramp steps: 1.0 = hard anime cel, 2.0 = 2-tier Ghibli soft, 0.0 = continuous
    pub toon_steps: f32,
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
            shadow_smoothness: 0.02,
            spec_intensity: 0.40,
            spec_power: 32.0,
            rim_intensity: 0.80,
            rim_spread: 0.40,
            hue_shift: -15.0, // cool lavender shift
            toon_steps: 1.0,
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

    pub fn update_material_for_all(&mut self, material: StylizedMaterial) {
        for node in &mut self.nodes {
            node.material = Some(material.clone());
        }
    }

    pub fn update_light(&mut self, light: StylizedLight) {
        self.light = light;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stylized_material_defaults() {
        let mat = StylizedMaterial::default();
        assert_eq!(mat.shadow_threshold, 0.50);
        assert_eq!(mat.shadow_smoothness, 0.02);
        assert_eq!(mat.spec_intensity, 0.40);
        assert_eq!(mat.spec_power, 32.0);
        assert_eq!(mat.rim_intensity, 0.80);
        assert_eq!(mat.rim_spread, 0.40);
        assert_eq!(mat.hue_shift, -15.0);
        assert_eq!(mat.toon_steps, 1.0);
    }

    #[test]
    fn test_scene_update_material() {
        let mut scene = Scene::default();
        assert_eq!(scene.nodes.len(), 1);
        let new_mat = StylizedMaterial {
            shadow_threshold: 0.65,
            hue_shift: -25.0,
            ..Default::default()
        };
        scene.update_material_for_all(new_mat);

        let node = scene.nodes.first().unwrap();
        let mat = node.material.as_ref().unwrap();
        assert_eq!(mat.shadow_threshold, 0.65);
        assert_eq!(mat.hue_shift, -25.0);
    }

    #[test]
    fn test_scene_update_light() {
        let mut scene = Scene::default();
        let light = StylizedLight {
            intensity: 2.5,
            shadow_color: [0.5, 0.6, 0.9],
            ..Default::default()
        };
        scene.update_light(light);

        assert_eq!(scene.light.intensity, 2.5);
        assert_eq!(scene.light.shadow_color, [0.5, 0.6, 0.9]);
    }
}
