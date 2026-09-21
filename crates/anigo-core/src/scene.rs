use crate::math::{Camera, Transform};
use crate::mesh::Mesh;
use crate::snapshot::SkinPayload;
use serde::{Deserialize, Serialize};

fn default_shadow_saturation() -> f32 { 1.0 }
/// P2-04: ambiente hemisférico (céu claro/frio, chão escuro/quente).
/// Espelha `ambientSky` do domínio TS (`project_persistence.ts`).
fn default_ambient_sky() -> [f32; 3] { [0.52, 0.60, 0.78] }
/// P2-04: idem para o hemisfério inferior. Espelha `ambientGround`.
fn default_ambient_ground() -> [f32; 3] { [0.25, 0.20, 0.18] }
/// P2-07: tamanho do especular separado da intensidade. Espelha `specularSize`.
fn default_spec_size() -> f32 { 0.45 }
/// P2-05: intensidade do AO (antes mistura fixa de 0.85). Espelha `aoIntensity`.
fn default_ao_intensity() -> f32 { 0.85 }
/// Stylized Anime Directional Light with Hue-Shifting parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StylizedLight {
    pub direction: [f32; 3],
    pub color: [f32; 3],
    pub intensity: f32,
    /// Stylized cool/warm hue-shifted shadow tint — neutral white so shade_color alone defines shadow (P0-02)
    pub shadow_color: [f32; 3],
    pub ambient_intensity: f32,
    /// Shadow saturation multiplier applied in HSV hue-shift (default neutral 1.0 to avoid double tint)
    #[serde(default = "default_shadow_saturation")]
    pub shadow_saturation: f32,
    // P2-04 hemisphere ambient (was single intensity scaling shadow color)
    #[serde(default = "default_ambient_sky")]
    pub ambient_sky: [f32; 3],
    #[serde(default = "default_ambient_ground")]
    pub ambient_ground: [f32; 3],
}

impl Default for StylizedLight {
    fn default() -> Self {
        Self {
            direction: [0.577, 0.577, 0.577], // normalized (1, 1, 1)
            color: [1.0, 0.98, 0.95],
            intensity: 1.0,
            shadow_color: [1.0, 1.0, 1.0], // P0-02: neutral white (was 0.65,0.68,0.85 — double tint with shade_color)
            ambient_intensity: 0.35,
            shadow_saturation: default_shadow_saturation(),
            ambient_sky: default_ambient_sky(),
            ambient_ground: default_ambient_ground(),
        }
    }
}

fn default_face_shadow_smoothness() -> f32 { 0.05 }
fn default_mtoon_emission_color() -> [f32; 4] { [0.0, 0.0, 0.0, 0.0] }
fn default_mtoon_emission_intensity() -> f32 { 0.0 }
fn default_mtoon_second_shade_shift() -> f32 { 0.0 }
fn default_mtoon_second_shade_softness() -> f32 { 0.05 }
fn default_mtoon_matcap_intensity() -> f32 { 0.0 }
fn default_mtoon_matcap_mode() -> u8 { 0 } // 0 = normal (mult), 1 = additive
fn default_mtoon_shade_toony() -> bool { true }
fn default_spec_color() -> [f32; 4] { [1.0, 1.0, 1.0, 1.0] }
fn default_spec_softness() -> f32 { 0.05 }
fn default_spec_offset() -> f32 { 0.0 }
fn default_rim_color() -> [f32; 4] { [0.576, 0.773, 0.992, 1.0] } // #93c5fd — P0-09 separate rim tint (was incorrectly sharing shadow_color)
fn default_outline_opacity() -> f32 { 1.0 }
fn default_outline_smoothness() -> f32 { 0.0 }
fn default_outline_depth_bias() -> f32 { 0.0 }

/// Stylized Material parameters for Anime NPR Cel-Shading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Toon ramp steps: 1.0 = hard anime cel, 2.0 = 2-tier Ghibli soft, 0.0 = continuous, 3.0 = high-key multi-band (P0-09)
    pub toon_steps: f32,
    // P0-09: previously dead controls / missing params now persisted
    #[serde(default = "default_spec_color")]
    pub specular_color: [f32; 4],
    #[serde(default = "default_spec_softness")]
    pub specular_softness: f32,
    #[serde(default = "default_spec_offset")]
    pub specular_offset: f32,
    #[serde(default = "default_rim_color")]
    pub rim_color: [f32; 4],
    #[serde(default = "default_outline_opacity")]
    pub outline_opacity: f32,
    #[serde(default = "default_outline_smoothness")]
    pub outline_smoothness: f32,
    #[serde(default = "default_outline_depth_bias")]
    pub outline_depth_bias: f32,
    // P2-07 separate spec_size (was coupled 0.65-0.12*intensity)
    #[serde(default = "default_spec_size")]
    pub specular_size: f32,
    // P2-05 AO intensity (was hardcoded 0.85 mix)
    #[serde(default = "default_ao_intensity")]
    pub ao_intensity: f32,
    // Fase 2 (#18): material anime VRoid/MToon — slots de textura e
    // parâmetros VRMC_materials_mtoon. Todos off por padrão: um material
    // sem texturas renderiza exatamente como antes (frame congelado).
    #[serde(default = "default_mtoon_emission_color")]
    pub mtoon_emission_color: [f32; 4],
    #[serde(default = "default_mtoon_emission_intensity")]
    pub mtoon_emission_intensity: f32,
    #[serde(default = "default_mtoon_second_shade_shift")]
    pub mtoon_second_shade_shift: f32,
    #[serde(default = "default_mtoon_second_shade_softness")]
    pub mtoon_second_shade_softness: f32,
    #[serde(default = "default_mtoon_matcap_intensity")]
    pub mtoon_matcap_intensity: f32,
    #[serde(default)]
    pub mtoon_main_texture_enabled: bool,
    #[serde(default)]
    pub mtoon_shade_texture_enabled: bool,
    #[serde(default)]
    pub mtoon_second_shade_texture_enabled: bool,
    #[serde(default)]
    pub mtoon_emission_texture_enabled: bool,
    #[serde(default)]
    pub mtoon_matcap_enabled: bool,
    #[serde(default = "default_mtoon_matcap_mode")]
    pub mtoon_matcap_mode: u8,
    #[serde(default = "default_mtoon_shade_toony")]
    pub mtoon_shade_toony: bool,
    // Fase 2 (#17): sombra facial SDF (Genshin style) — off por padrão.
    /// Deslocamento manual do threshold do SDF facial (-0.25 a 0.25).
    #[serde(default)]
    pub face_shadow_offset: f32,
    /// Suavidade da penumbra da sombra facial.
    #[serde(default = "default_face_shadow_smoothness")]
    pub face_shadow_smoothness: f32,
    /// Ativa a sombra facial (mapa SDF ancorado no renderer).
    #[serde(default)]
    pub face_sdf_enabled: bool,
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
            specular_color: default_spec_color(),
            specular_softness: default_spec_softness(),
            specular_offset: default_spec_offset(),
            rim_color: default_rim_color(),
            outline_opacity: default_outline_opacity(),
            outline_smoothness: default_outline_smoothness(),
            outline_depth_bias: default_outline_depth_bias(),
            specular_size: default_spec_size(),
            ao_intensity: default_ao_intensity(),
            mtoon_emission_color: default_mtoon_emission_color(),
            mtoon_emission_intensity: default_mtoon_emission_intensity(),
            mtoon_second_shade_shift: default_mtoon_second_shade_shift(),
            mtoon_second_shade_softness: default_mtoon_second_shade_softness(),
            mtoon_matcap_intensity: default_mtoon_matcap_intensity(),
            mtoon_main_texture_enabled: false,
            mtoon_shade_texture_enabled: false,
            mtoon_second_shade_texture_enabled: false,
            mtoon_emission_texture_enabled: false,
            mtoon_matcap_enabled: false,
            mtoon_matcap_mode: default_mtoon_matcap_mode(),
            mtoon_shade_toony: default_mtoon_shade_toony(),
            face_shadow_offset: 0.0,
            face_shadow_smoothness: default_face_shadow_smoothness(),
            face_sdf_enabled: false,
        }
    }
}

/// A node within the hierarchical scene graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub nodes: Vec<SceneNode>,
    pub camera: Camera,
    pub light: StylizedLight,
    pub background_color: [f32; 4],
    /// P1-04: paleta de skinning da cena. Sai do núcleo (a mesma que o snapshot
    /// entrega), então viewport e headless deformam com a mesma matriz. Sem o
    /// campo no JSON, volta para a paleta canônica (identidade enquanto as
    /// proporções são assadas na malha base).
    #[serde(default = "SkinPayload::canonical_base")]
    pub skin: SkinPayload,
}

impl Default for Scene {
    fn default() -> Self {
        // P0-05/P0-08: use canonical base (was proxy 156 tris → real 6880 tris for telemetry parity)
        let mannequin_node = SceneNode::new("mannequin_proxy", "Anime Mannequin")
            .with_mesh(Mesh::create_canonical_base(crate::mesh::BaseGender::Male));

        Self {
            nodes: vec![mannequin_node],
            camera: Camera::default(),
            light: StylizedLight::default(),
            background_color: [0.08, 0.09, 0.13, 1.0], // P0-04: unified with viewport clearColor (was 0.12,0.13,0.16)
            skin: SkinPayload::canonical_base(),
        }
    }
}

impl Scene {
    pub fn new_empty() -> Self {
        Self {
            nodes: Vec::new(),
            camera: Camera::default(),
            light: StylizedLight::default(),
            background_color: [0.08, 0.09, 0.13, 1.0],
            skin: SkinPayload::canonical_base(),
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
