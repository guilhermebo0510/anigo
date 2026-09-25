use std::collections::BTreeMap;

use crate::hierarchy::{self, HierarchyError, NodeKind};
use crate::math::{Camera, Transform};
use crate::mesh::Mesh;
use crate::snapshot::SkinPayload;
use glam::Mat4;
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
fn default_gaze_saccade_amplitude() -> f32 { 2.5 } // graus (Faixa 2–5 do issue #43)
fn default_gaze_damping() -> f32 { 6.0 }
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
    // Fase 2 (#43): olho anime (parallax + highlights) — off por padrão.
    /// "Recalada" da íris: profundidade do parallax (UV + V_tangent × scale).
    #[serde(default)]
    pub eye_depth_scale: f32,
    /// Intensidade dos highlights desenhados à mão (desacoplados da luz).
    #[serde(default)]
    pub eye_highlight_intensity: f32,
    /// Ativa o shader do olho anime (parallax + highlights).
    #[serde(default)]
    pub eye_enabled: bool,
    // Fase 2 (#43): solver de olhar (CPU — anigo-ik / eye_tracking.ts, não GPU).
    /// Rastreamento do olhar para a câmera/alvo (micro-sacadas incluídas).
    #[serde(default)]
    pub gaze_tracking_enabled: bool,
    /// Amplitude das micro-sacadas em graus (faixa 2–5 do issue).
    #[serde(default = "default_gaze_saccade_amplitude")]
    pub gaze_saccade_amplitude: f32,
    /// Damping do tracking (exponencial, 1/s) — quanto maior, mais rígido.
    #[serde(default = "default_gaze_damping")]
    pub gaze_damping: f32,
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
            eye_depth_scale: 0.0,
            eye_highlight_intensity: 0.0,
            eye_enabled: false,
            gaze_tracking_enabled: false,
            gaze_saccade_amplitude: default_gaze_saccade_amplitude(),
            gaze_damping: default_gaze_damping(),
        }
    }
}

/// A node within the hierarchical scene graph.
///
/// Issue #12: o nó carrega a **transformação local** e a sua posição na árvore
/// (`parent_id` + `children`). A matriz mundial é derivada — `parent × local` —
/// e nunca digitada à mão: `Scene::resolve_world_transforms` é a única fonte.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneNode {
    pub id: String,
    pub name: String,
    pub transform: Transform,
    pub mesh: Option<Mesh>,
    pub material: Option<StylizedMaterial>,
    pub visible: bool,
    /// Pai na árvore de transformações (`None` = raiz).
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Filhos diretos, em ordem de exibição (derivado de `parent_id` por
    /// `Scene::rebuild_children`; existe materializado para travessias baratas
    /// e para telemetria/UI).
    #[serde(default)]
    pub children: Vec<String>,
    /// Tipo especializado do nó (`character_root`, `clothing`, `hair`, …).
    #[serde(default)]
    pub kind: NodeKind,
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
            parent_id: None,
            children: Vec::new(),
            kind: NodeKind::default(),
        }
    }

    pub fn with_mesh(mut self, mesh: Mesh) -> Self {
        self.mesh = Some(mesh);
        self
    }

    /// Nó preso a um pai (acessório/vestuário/cabelo).
    pub fn child_of(mut self, parent: impl Into<String>, kind: NodeKind) -> Self {
        self.parent_id = Some(parent.into());
        self.kind = kind;
        self
    }

    /// `true` quando o nó tem geometria própria e visível para desenhar.
    pub fn is_drawable(&self) -> bool {
        self.visible && self.mesh.is_some()
    }
}

/// Fase 2 (#53): Depth of Field cinematográfico (Anime Bokeh DoF).
///
/// Vem no JSON da cena (default = desligado — o passe de pós é pulado e a
/// imagem é idêntica ao frame congelado). Os mesmos valores chegam ao shader
/// `postprocess_dof.wgsl` (uniform `DofUniform`) no viewport e no headless.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DofSettings {
    /// Ativa o passe de DoF pós-projeto.
    #[serde(default)]
    pub enabled: bool,
    /// Distância de foco (m) — o plano milimetricamente nítido (olhos/rosto).
    #[serde(default = "default_dof_focus_distance")]
    pub focus_distance: f32,
    /// Número f (abertura) — menor = mais bokeh.
    #[serde(default = "default_dof_f_number")]
    pub f_number: f32,
    /// Distância focal em mm (lente ativa) — entra na fórmula do CoC.
    #[serde(default = "default_dof_focal_mm")]
    pub focal_mm: f32,
    /// Formato da abertura: 0 = bokeh circular, 1 = hexagonal clássico de anime.
    #[serde(default)]
    pub bokeh_shape: u32,
    /// Raio máximo do bokeh em pixels (teto para não custar além do necessário).
    #[serde(default = "default_dof_max_radius_px")]
    pub max_radius_px: f32,
}

impl Default for DofSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            focus_distance: default_dof_focus_distance(),
            f_number: default_dof_f_number(),
            focal_mm: default_dof_focal_mm(),
            bokeh_shape: 0,
            max_radius_px: default_dof_max_radius_px(),
        }
    }
}

fn default_dof_focus_distance() -> f32 {
    2.0
}
fn default_dof_f_number() -> f32 {
    2.0
}
fn default_dof_focal_mm() -> f32 {
    50.0
}
fn default_dof_max_radius_px() -> f32 {
    16.0
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
    /// Fase 2 (#53): DoF cinematográfico — default off (sem campo, sem passe).
    #[serde(default)]
    pub dof: DofSettings,
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
            dof: DofSettings::default(),
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
            dof: DofSettings::default(),
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

    /// Reconstrói as listas de filhos a partir dos `parent_id` (issue #12).
    ///
    /// Chamado depois de qualquer mudança de topologia: assim `children` nunca
    /// é uma segunda autoridade sobre a árvore — é só o cache ordenado da
    /// relação declarada em `parent_id`.
    pub fn rebuild_children(&mut self) {
        for node in &mut self.nodes {
            node.children.clear();
        }
        // Índice id → posição para resolver os pais sem varrer a lista por nó.
        let index_of: BTreeMap<String, usize> = self
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id.clone(), index))
            .collect();
        let mut children: Vec<Vec<String>> = vec![Vec::new(); self.nodes.len()];
        for node in &self.nodes {
            if let Some(parent) = &node.parent_id {
                if let Some(parent_index) = index_of.get(parent) {
                    children[*parent_index].push(node.id.clone());
                }
            }
        }
        for (index, list) in children.into_iter().enumerate() {
            self.nodes[index].children = list;
        }
    }

    /// Valida a árvore (ids únicos, pais existentes, sem ciclo).
    pub fn validate_hierarchy(&self) -> Result<(), HierarchyError> {
        let ids: Vec<&str> = self.nodes.iter().map(|node| node.id.as_str()).collect();
        let parents: Vec<Option<&str>> = self
            .nodes
            .iter()
            .map(|node| node.parent_id.as_deref())
            .collect();
        let locals: Vec<Mat4> = self
            .nodes
            .iter()
            .map(|node| node.transform.to_matrix())
            .collect();
        hierarchy::resolve_indexed(&ids, &parents, &locals).map(|_| ())
    }

    /// Matrizes mundiais de todos os nós (pais antes de filhos), por id.
    pub fn resolve_world_transforms(&self) -> Result<BTreeMap<String, Mat4>, HierarchyError> {
        let ids: Vec<&str> = self.nodes.iter().map(|node| node.id.as_str()).collect();
        let parents: Vec<Option<&str>> = self
            .nodes
            .iter()
            .map(|node| node.parent_id.as_deref())
            .collect();
        let locals: Vec<Mat4> = self
            .nodes
            .iter()
            .map(|node| node.transform.to_matrix())
            .collect();
        let world = hierarchy::resolve_indexed(&ids, &parents, &locals)?;
        Ok(self
            .nodes
            .iter()
            .zip(world)
            .map(|(node, matrix)| (node.id.clone(), matrix))
            .collect())
    }

    /// Matriz mundial de um nó (ou `None` se o id não existe / a árvore é
    /// inválida). O renderer usa isto para o `model` do uniform de câmera.
    pub fn world_matrix(&self, id: &str) -> Option<Mat4> {
        self.resolve_world_transforms()
            .ok()
            .and_then(|world| world.get(id).copied())
    }

    /// Filhos diretos de um nó (por id), em ordem de declaração.
    pub fn children_of(&self, parent: Option<&str>) -> Vec<&SceneNode> {
        self.nodes
            .iter()
            .filter(|node| node.parent_id.as_deref() == parent)
            .collect()
    }

    /// Nó pelo id.
    pub fn node(&self, id: &str) -> Option<&SceneNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// Nó mutável pelo id.
    pub fn node_mut(&mut self, id: &str) -> Option<&mut SceneNode> {
        self.nodes.iter_mut().find(|node| node.id == id)
    }

    /// Profundidade do nó (raiz = 0); `None` quando o id não existe.
    pub fn depth_of(&self, id: &str) -> Option<usize> {
        let mut depth = 0usize;
        let mut cursor = self.node(id)?.parent_id.clone();
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        while let Some(parent) = cursor {
            if !seen.insert(parent.clone()) {
                break;
            }
            depth += 1;
            cursor = self.node(&parent)?.parent_id.clone();
        }
        Some(depth)
    }

    /// Ordem topológica dos nós (para desenho/telemetria determinística).
    pub fn hierarchy_order(&self) -> Result<Vec<usize>, HierarchyError> {
        let ids: Vec<&str> = self.nodes.iter().map(|node| node.id.as_str()).collect();
        let parents: Vec<Option<&str>> = self
            .nodes
            .iter()
            .map(|node| node.parent_id.as_deref())
            .collect();
        let locals: Vec<Mat4> = self
            .nodes
            .iter()
            .map(|node| node.transform.to_matrix())
            .collect();
        hierarchy::resolve_indexed(&ids, &parents, &locals)?;
        let index_of: BTreeMap<&str, usize> = ids
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, index))
            .collect();
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); ids.len()];
        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        for (index, parent) in parents.iter().enumerate() {
            match parent.and_then(|parent| index_of.get(parent).copied()) {
                Some(parent) => children[parent].push(index),
                None => queue.push_back(index),
            }
        }
        let mut order = Vec::with_capacity(ids.len());
        while let Some(index) = queue.pop_front() {
            order.push(index);
            for child in &children[index] {
                queue.push_back(*child);
            }
        }
        Ok(order)
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
    fn child_world_matrix_follows_the_parent_transform() {
        // Issue #12, aceitação #1: mexer no pai move o filho sem tocar no filho.
        let mut scene = Scene::default();
        scene.nodes.clear();
        let mut root = SceneNode::new("nod_root", "Root");
        root.transform.translation = glam::Vec3::new(1.0, 0.0, 0.0);
        let child = SceneNode::new("nod_hat", "Hat")
            .child_of("nod_root", NodeKind::Accessory)
            .with_mesh(Mesh::create_cube(0.2));
        scene.nodes.push(root);
        scene.nodes.push(child);
        scene.rebuild_children();

        let before = scene.world_matrix("nod_hat").expect("matriz do filho");
        assert!((before.transform_point3(glam::Vec3::ZERO).x - 1.0).abs() < 1e-5);

        // Move o pai: a posição mundial do filho acompanha na hora.
        scene
            .node_mut("nod_root")
            .expect("raiz")
            .transform
            .translation = glam::Vec3::new(3.5, 0.0, 0.0);
        let after = scene.world_matrix("nod_hat").expect("matriz do filho");
        assert!((after.transform_point3(glam::Vec3::ZERO).x - 3.5).abs() < 1e-5);

        // O filho continua com a transformação local intacta.
        let child = scene.node("nod_hat").expect("filho");
        assert!(child.transform.translation.length() < 1e-6);
        assert_eq!(child.parent_id.as_deref(), Some("nod_root"));
        assert_eq!(scene.node("nod_root").expect("raiz").children, vec!["nod_hat".to_string()]);
        assert_eq!(scene.depth_of("nod_hat"), Some(1));
    }

    #[test]
    fn rebuild_children_is_derived_and_deterministic() {
        let mut scene = Scene::new_empty();
        scene.nodes.push(SceneNode::new("nod_a", "A"));
        scene.nodes.push(SceneNode::new("nod_b", "B"));
        scene.nodes.push(SceneNode::new("nod_c", "C").child_of("nod_a", NodeKind::Clothing));
        scene.nodes.push(SceneNode::new("nod_d", "D").child_of("nod_a", NodeKind::Hair));
        scene.nodes.push(SceneNode::new("nod_e", "E").child_of("nod_d", NodeKind::Accessory));
        scene.rebuild_children();

        // Ordem de declaração dentro do nível, filhos antes dos netos na lista
        // do pai (o que a UI usa para desenhar a árvore).
        assert_eq!(
            scene.node("nod_a").expect("nod_a").children,
            vec!["nod_c".to_string(), "nod_d".to_string()]
        );
        assert_eq!(
            scene.node("nod_d").expect("nod_d").children,
            vec!["nod_e".to_string()]
        );
        assert!(scene.node("nod_e").expect("nod_e").children.is_empty());
        assert_eq!(scene.children_of(None).len(), 2);

        // Filho declarado antes do pai ainda resolve (ordem topológica).
        let mut scene = Scene::new_empty();
        scene.nodes.push(SceneNode::new("nod_child", "Child").child_of("nod_parent", NodeKind::Accessory));
        scene.nodes.push(SceneNode::new("nod_parent", "Parent"));
        assert!(scene.validate_hierarchy().is_ok());
        let order = scene.hierarchy_order().expect("ordem topológica");
        assert_eq!(order, vec![1, 0]);
    }

    #[test]
    fn cyclic_scene_is_rejected() {
        let mut scene = Scene::new_empty();
        scene.nodes.push(SceneNode::new("nod_a", "A").child_of("nod_b", NodeKind::Mesh));
        scene.nodes.push(SceneNode::new("nod_b", "B").child_of("nod_a", NodeKind::Mesh));
        assert!(matches!(
            scene.validate_hierarchy(),
            Err(HierarchyError::Cycle { .. })
        ));
        assert!(scene.resolve_world_transforms().is_err());
        assert!(scene.world_matrix("nod_a").is_none());
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
