//! ANIGO Canonical Project State (P0 — ARQUITETURA_CANONICA_ANIGO §3).
//!
//! `ProjectState` is the single authoritative document of a project: character,
//! scene, materials, animation, render/color-management settings and asset
//! references. It is:
//!
//! - **versioned** (`schema_version`, currently [`PROJECT_SCHEMA_VERSION`]);
//! - **id-addressed** (every entity is referenced by a stable id from
//!   [`crate::ids`], never by array position);
//! - **validated before use** ([`ProjectState::validate`] rejects dangling or
//!   duplicated references, non-finite numbers and out-of-range values);
//! - **migratable** ([`ProjectState::from_json`] accepts legacy TypeScript
//!   autosave payloads and upgrades them to the current schema);
//! - **forward-compatible** (unknown JSON keys are preserved in
//!   [`ProjectState::extensions`] so a newer build never silently drops data);
//! - **deterministically serializable** ([`ProjectState::to_canonical_json`]),
//!   which is what the contract/golden tests compare.
//!
//! Derived data (GPU buffers, deformed meshes, thumbnails) is *never* stored
//! here — see §3.2 of the architecture document.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::ids::{
    fnv1a64, AnimationClipId, AssetId, CameraId, CharacterId, IdError, LightId, MaterialId,
    MorphId, NodeId, ProjectId, SceneId,
};
use crate::hierarchy::{self, NodeKind};
use crate::math::{Camera, Transform};
use crate::mesh::{BaseGender, Mesh};
use crate::morph_catalog::{find_slider_def, ALL_MORPH_SLIDERS};
use crate::scene::{Scene, SceneNode, StylizedLight, StylizedMaterial};
use crate::somatotype::SomatotypeCoords;

/// Current schema version of the project document. Bumping this requires
/// adding an explicit migration in [`ProjectState::migrate_value`].
pub const PROJECT_SCHEMA_VERSION: u32 = 1;

/// Default autosave interval (minutes) used by [`ProjectSettings::default`].
pub const DEFAULT_AUTOSAVE_INTERVAL_MINUTES: u32 = 5;
/// Default undo/redo depth used by [`ProjectSettings::default`].
pub const DEFAULT_HISTORY_LIMIT: u32 = 60;

/// Canonical URI of the procedurally generated male base mesh asset.
pub const URI_BASE_MALE: &str = "anigo://base/anigo_base_male.glb";
/// Canonical URI of the procedurally generated female base mesh asset.
pub const URI_BASE_FEMALE: &str = "anigo://base/anigo_base_female.glb";
/// Canonical URI of the unit cube preset mesh.
pub const URI_PRESET_CUBE: &str = "anigo://preset/cube";
/// Canonical URI of the UV sphere preset mesh.
pub const URI_PRESET_SPHERE: &str = "anigo://preset/uv_sphere";

/// Built-in (procedural) mesh assets that ship with the editor.
///
/// They are registered by [`ProjectState::default`] and repaired by
/// [`ProjectState::sanitize`] so that every `MeshRef` of a well-formed project
/// resolves *without a command ever mutating the asset registry*: a command
/// that fabricated an entry could not remove it again on undo, which would
/// break the "undo reproduces the exact previous state" invariant of the
/// history. Loading a preset or pointing a node at a built-in mesh is
/// therefore a pure node/gender change.
pub const BUILTIN_MESH_URIS: [&str; 4] = [
    URI_BASE_MALE,
    URI_BASE_FEMALE,
    URI_PRESET_CUBE,
    URI_PRESET_SPHERE,
];

/// Everything that can go wrong while loading, validating or migrating a project.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProjectError {
    /// The payload is not valid JSON.
    #[error("project JSON is malformed: {0}")]
    Json(String),
    /// The payload is not a JSON object.
    #[error("project root must be a JSON object, found {found}")]
    NotAnObject { found: String },
    /// `schema_version` is newer than this build understands.
    #[error("project schema_version {found} is newer than the supported version {supported}")]
    UnsupportedSchemaVersion { found: u32, supported: u32 },
    /// `schema_version` is missing on a payload that claims to be canonical.
    #[error("project is missing the required 'schema_version' field")]
    MissingSchemaVersion,
    /// An identifier failed validation.
    #[error("invalid id in field '{field}': {source}")]
    InvalidId {
        field: &'static str,
        #[source]
        source: IdError,
    },
    /// Two entities of the same kind share an id.
    #[error("duplicated {kind} id '{id}'")]
    DuplicateId { kind: &'static str, id: String },
    /// A reference points at an entity that does not exist.
    #[error("field '{field}' references unknown {kind} '{target}'")]
    DanglingReference {
        field: &'static str,
        kind: &'static str,
        target: String,
    },
    /// A value is outside its canonical domain.
    #[error("invalid value for '{field}': {detail}")]
    InvalidValue { field: String, detail: String },
    /// A morph id is not part of the canonical catalog.
    #[error("unknown morph slider '{0}'")]
    UnknownMorph(String),
    /// The project contains no scene.
    #[error("project must contain at least one scene node")]
    EmptyScene,
    /// Issue #12: a árvore de transformações é inválida (pai ausente, pai
    /// próprio ou ciclo de parentesco).
    #[error("invalid scene hierarchy: {detail}")]
    InvalidHierarchy { detail: String },
}

/// Root document of a saved project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectState {
    /// Schema version — always present, always first.
    pub schema_version: u32,
    /// Stable project id (`prj_<slug>`).
    pub project_id: ProjectId,
    /// Display name of the project.
    pub name: String,
    /// Character domain (identity, somatotype, morph weights, clothing).
    pub character: CharacterState,
    /// Scene domain (nodes, camera, lights).
    pub scene: SceneState,
    /// Materials, keyed by stable id.
    #[serde(default)]
    pub materials: BTreeMap<MaterialId, MaterialEntry>,
    /// Animation domain.
    #[serde(default)]
    pub animation: AnimationState,
    /// Render + color management settings.
    #[serde(default)]
    pub render: RenderState,
    /// Referenced assets, keyed by stable id.
    #[serde(default)]
    pub assets: BTreeMap<AssetId, AssetEntry>,
    /// Persistent application settings bound to the project.
    #[serde(default)]
    pub settings: ProjectSettings,
    /// Unknown keys preserved verbatim from the loaded payload (forward compat).
    #[serde(flatten, default)]
    pub extensions: BTreeMap<String, Value>,
}

// ---------------------------------------------------------------------------
// Character
// ---------------------------------------------------------------------------

/// Character domain of the project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterState {
    /// Stable character id (`chr_<slug>`).
    pub character_id: CharacterId,
    /// Canonical base mesh archetype.
    pub base_gender: BaseGender,
    /// Continuous gender dimorphism: `0.0` female, `1.0` male.
    pub gender_dimorphism: f32,
    /// Heath-Carter somatotype (normalized barycentric components).
    pub somatotype: SomatotypeCoords,
    /// Macro proportions (bone-driven ratios).
    pub proportions: CharacterProportions,
    /// Sparse morph weights keyed by stable morph id — only sliders that differ
    /// from the catalog default are stored.
    #[serde(default)]
    pub morph_values: BTreeMap<MorphId, f32>,
    /// Id of the preset the character was last built from, when any.
    #[serde(default)]
    pub active_preset_id: Option<String>,
    /// Hair parameters.
    #[serde(default)]
    pub hair: HairParameters,
    /// Cloth parameters.
    #[serde(default)]
    pub cloth: ClothParameters,
    /// Accessory attachment.
    #[serde(default)]
    pub accessory: AccessoryAttachment,
}

/// Bone-driven macro proportions of the character.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CharacterProportions {
    pub head_scale: f32,
    pub head_ratio: f32,
    pub shoulder_width: f32,
    pub leg_length: f32,
    pub arm_length: f32,
    pub neck_length: f32,
    pub torso_length: f32,
    pub height_overall: f32,
}

impl Default for CharacterProportions {
    fn default() -> Self {
        Self {
            head_scale: 1.0,
            head_ratio: 6.5,
            shoulder_width: 1.0,
            leg_length: 1.0,
            arm_length: 1.0,
            neck_length: 1.0,
            torso_length: 1.0,
            height_overall: 1.0,
        }
    }
}

/// Hair domain parameters.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HairParameters {
    pub volume: f32,
    pub thickness: f32,
    pub curvature: f32,
    pub strands: u32,
}

impl Default for HairParameters {
    fn default() -> Self {
        Self {
            volume: 1.2,
            thickness: 0.05,
            curvature: 0.4,
            strands: 16,
        }
    }
}

/// Cloth domain parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClothParameters {
    pub layer: String,
    pub tension: f32,
    pub rigidity: f32,
    pub gravity: f32,
}

impl Default for ClothParameters {
    fn default() -> Self {
        Self {
            layer: "uniforme".to_string(),
            tension: 0.5,
            rigidity: 0.3,
            gravity: 1.0,
        }
    }
}

/// Accessory attachment socket + offset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccessoryAttachment {
    pub socket: String,
    pub scale: f32,
    pub offset: [f32; 3],
}

impl Default for AccessoryAttachment {
    fn default() -> Self {
        Self {
            socket: "head".to_string(),
            scale: 1.0,
            offset: [0.0, 0.0, 0.0],
        }
    }
}

// ---------------------------------------------------------------------------
// Scene
// ---------------------------------------------------------------------------

/// Scene domain: hierarchical nodes, camera and lights, all id-addressed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneState {
    pub scene_id: SceneId,
    pub camera: CameraSlot,
    #[serde(default)]
    pub lights: Vec<LightSlot>,
    #[serde(default)]
    pub nodes: Vec<NodeSlot>,
}

impl SceneState {
    /// Nó pelo id estável.
    pub fn node(&self, id: &NodeId) -> Option<&NodeSlot> {
        self.nodes.iter().find(|node| &node.node_id == id)
    }

    /// Nó mutável pelo id estável.
    pub fn node_mut(&mut self, id: &NodeId) -> Option<&mut NodeSlot> {
        self.nodes.iter_mut().find(|node| &node.node_id == id)
    }

    /// Filhos diretos de `parent` (issue #12), em ordem de declaração —
    /// `None` devolve as raízes. A lista é **derivada** de `parent_id`: uma
    /// única autoridade sobre a topologia.
    pub fn children_of(&self, parent: Option<&NodeId>) -> Vec<&NodeSlot> {
        self.nodes
            .iter()
            .filter(|node| node.parent_id.as_ref() == parent)
            .collect()
    }

    /// Nós raiz (sem pai declarado).
    pub fn roots(&self) -> Vec<&NodeSlot> {
        self.children_of(None)
    }

    /// Profundidade de um nó na árvore (raiz = 0); `None` quando não existe.
    pub fn depth_of(&self, id: &NodeId) -> Option<usize> {
        self.node(id).map(|node| hierarchy::depth_of(&self.nodes, &node.node_id))
    }

    /// Descendentes de um nó (ordem de largura, declaração dentro do nível).
    pub fn descendants_of(&self, id: &NodeId) -> Vec<&NodeSlot> {
        hierarchy::descendants_of(&self.nodes, id)
            .into_iter()
            .filter_map(|descendant| self.node(&descendant))
            .collect()
    }

    /// Matrizes mundiais resolvidas de toda a cena, indexadas por id.
    pub fn world_transforms(&self) -> Result<BTreeMap<NodeId, glam::Mat4>, hierarchy::HierarchyError> {
        hierarchy::resolve_world_transforms(&self.nodes)
    }

    /// Ordem topológica dos nós (pais antes de filhos) como índices.
    pub fn hierarchy_order(&self) -> Result<Vec<usize>, hierarchy::HierarchyError> {
        hierarchy::hierarchy_order(&self.nodes)
    }
}

/// Project camera slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraSlot {
    pub camera_id: CameraId,
    pub camera: Camera,
}

/// Project light slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LightSlot {
    pub light_id: LightId,
    pub light: StylizedLight,
}

/// Scene node referencing assets + materials by id (never by index).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeSlot {
    pub node_id: NodeId,
    pub name: String,
    #[serde(default)]
    pub transform: Transform,
    #[serde(default)]
    pub mesh: Option<MeshRef>,
    #[serde(default)]
    pub material_id: Option<MaterialId>,
    #[serde(default = "default_true")]
    pub visible: bool,
    /// Issue #12: pai na árvore de transformações (`None` = raiz da cena).
    ///
    /// A lista de filhos **não** é persistida aqui: ela é derivada deste campo
    /// (`SceneState::children_of`), de modo que exista uma única autoridade
    /// sobre a topologia — duas listas (pai e filhos) só dariam margem a
    /// divergirem entre si depois de um undo.
    #[serde(default)]
    pub parent_id: Option<NodeId>,
    /// Issue #12: tipo especializado do nó (`character_root`, `clothing`,
    /// `hair`, `accessory`, `humanoid_bone`, `mesh`, `light`, `camera`, `group`).
    #[serde(default)]
    pub kind: NodeKind,
}

impl NodeSlot {
    /// Nó folha de um pai opcional (ordem de declaração = ordem de exibição).
    pub fn new(node_id: NodeId, name: impl Into<String>, kind: NodeKind) -> Self {
        Self {
            node_id,
            name: name.into(),
            transform: Transform::default(),
            mesh: None,
            material_id: None,
            visible: true,
            parent_id: None,
            kind,
        }
    }

    /// Nó preso a um pai (acessório/vestuário/cabelo seguindo o corpo).
    pub fn child_of(mut self, parent: NodeId) -> Self {
        self.parent_id = Some(parent);
        self
    }

    /// `true` quando este nó é o pai direto de `candidate`.
    pub fn is_parent_of(&self, candidate: &NodeSlot) -> bool {
        candidate.parent_id.as_ref() == Some(&self.node_id)
    }
}

/// Reference to a mesh primitive inside an asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshRef {
    pub asset_id: AssetId,
    #[serde(default)]
    pub primitive_index: u32,
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Materials
// ---------------------------------------------------------------------------

/// Material entry: stable id + canonical stylized parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialEntry {
    pub material_id: MaterialId,
    pub name: String,
    pub material: StylizedMaterial,
}

// ---------------------------------------------------------------------------
// Animation
// ---------------------------------------------------------------------------

/// Animation domain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationState {
    #[serde(default)]
    pub clips: BTreeMap<AnimationClipId, AnimationClip>,
    #[serde(default)]
    pub active_clip: Option<AnimationClipId>,
    #[serde(default)]
    pub playhead_seconds: f32,
}

impl Default for AnimationState {
    fn default() -> Self {
        Self {
            clips: BTreeMap::new(),
            active_clip: None,
            playhead_seconds: 0.0,
        }
    }
}

/// Animation clip with per-node transform tracks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationClip {
    pub clip_id: AnimationClipId,
    pub name: String,
    pub duration_seconds: f32,
    #[serde(default)]
    pub tracks: Vec<AnimationTrack>,
}

/// Track binding keyframes to a node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationTrack {
    pub node_id: NodeId,
    #[serde(default)]
    pub keyframes: Vec<TransformKeyframe>,
}

/// One transform keyframe.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TransformKeyframe {
    pub time_seconds: f32,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

// ---------------------------------------------------------------------------
// Render + color management (§6.2)
// ---------------------------------------------------------------------------

/// Render settings for viewport and headless export (same struct, §2.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderState {
    /// Version of the render/color-management block itself.
    pub settings_version: u32,
    pub msaa_samples: u32,
    pub background_color: [f32; 4],
    pub color: ColorManagement,
    pub tonemap: TonemapOperator,
}

impl Default for RenderState {
    fn default() -> Self {
        Self {
            settings_version: 1,
            msaa_samples: 4,
            background_color: [0.08, 0.09, 0.13, 1.0],
            color: ColorManagement::default(),
            tonemap: TonemapOperator::None,
        }
    }
}

/// Explicit color-management definition (its absence is treated as a visual
/// parity failure by the architecture document).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorManagement {
    /// Space of textures/materials as authored (asset files).
    pub input_texture_space: ColorSpace,
    /// Working space of the renderer.
    pub working_space: ColorSpace,
    /// Space of the framebuffer presented to the user / written to disk.
    pub display_space: ColorSpace,
}

impl Default for ColorManagement {
    fn default() -> Self {
        Self {
            input_texture_space: ColorSpace::Srgb,
            working_space: ColorSpace::LinearSrgb,
            display_space: ColorSpace::Srgb,
        }
    }
}

/// Supported color spaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorSpace {
    /// sRGB (non-linear, display referred).
    Srgb,
    /// Linear sRGB (scene referred working space).
    LinearSrgb,
    /// Display P3 (wide gamut presentation).
    DisplayP3,
}

/// Tonemapping operator applied between working and display space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TonemapOperator {
    /// No tonemapping (NPR cel shading default).
    None,
    /// Reinhard `x / (1 + x)`.
    Reinhard,
    /// Neutral (Khronos) tonemap.
    Neutral,
}

// ---------------------------------------------------------------------------
// Assets + settings
// ---------------------------------------------------------------------------

/// Referenced asset entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetEntry {
    pub asset_id: AssetId,
    pub kind: AssetKind,
    /// Relative or `anigo://` URI — never an absolute machine path.
    pub uri: String,
    #[serde(default)]
    pub byte_len: Option<u64>,
    #[serde(default)]
    pub content_hash: Option<String>,
}

impl AssetEntry {
    /// Builds an entry deriving its id from the canonical URI.
    pub fn from_uri(kind: AssetKind, uri: impl Into<String>) -> Self {
        let uri = uri.into();
        Self {
            asset_id: AssetId::for_uri(&uri),
            kind,
            uri,
            byte_len: None,
            content_hash: None,
        }
    }
}

/// Asset kinds tracked by the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    /// glTF/GLB or procedural mesh.
    Mesh,
    /// Image texture.
    Texture,
    /// Hair groom asset.
    Hair,
    /// Cloth asset.
    Cloth,
    /// Audio clip.
    Audio,
    /// Anything else (kept so unknown assets never disappear).
    Other,
}

/// Persistent project settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSettings {
    pub autosave_interval_minutes: u32,
    pub history_limit: u32,
    pub locale: String,
    pub units: LengthUnit,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            autosave_interval_minutes: DEFAULT_AUTOSAVE_INTERVAL_MINUTES,
            history_limit: DEFAULT_HISTORY_LIMIT,
            locale: "pt_BR".to_string(),
            units: LengthUnit::Meters,
        }
    }
}

/// Unit system used by the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LengthUnit {
    Meters,
    Centimeters,
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

impl Default for CharacterState {
    fn default() -> Self {
        Self {
            character_id: CharacterId::canonical(),
            base_gender: BaseGender::Male,
            gender_dimorphism: 1.0,
            somatotype: SomatotypeCoords::default(),
            proportions: CharacterProportions::default(),
            morph_values: BTreeMap::new(),
            active_preset_id: None,
            hair: HairParameters::default(),
            cloth: ClothParameters::default(),
            accessory: AccessoryAttachment::default(),
        }
    }
}

impl Default for SceneState {
    fn default() -> Self {
        let material_id = MaterialId::canonical_default();
        let node = NodeSlot {
            node_id: NodeId::canonical_character(),
            name: "Anime Mannequin".to_string(),
            transform: Transform::default(),
            mesh: Some(MeshRef {
                asset_id: AssetId::for_uri(URI_BASE_MALE),
                primitive_index: 0,
            }),
            material_id: Some(material_id),
            visible: true,
            // Issue #12: o manequim é a raiz do personagem na cena canônica.
            parent_id: None,
            kind: NodeKind::CharacterRoot,
        };

        Self {
            scene_id: SceneId::canonical(),
            camera: CameraSlot {
                camera_id: CameraId::canonical(),
                camera: Camera::default(),
            },
            lights: vec![LightSlot {
                light_id: LightId::canonical_key(),
                light: StylizedLight::default(),
            }],
            nodes: vec![node],
        }
    }
}

impl Default for ProjectState {
    fn default() -> Self {
        let scene = SceneState::default();
        let mut materials = BTreeMap::new();
        materials.insert(
            MaterialId::canonical_default(),
            MaterialEntry {
                material_id: MaterialId::canonical_default(),
                name: "DefaultAnimeMaterial".to_string(),
                material: StylizedMaterial::default(),
            },
        );

        let mut assets = BTreeMap::new();
        // Every built-in asset is registered from the start, so presets and
        // canonical bases are referenced without a command having to touch the
        // registry (see `BUILTIN_MESH_URIS`).
        for uri in BUILTIN_MESH_URIS {
            let entry = AssetEntry::from_uri(AssetKind::Mesh, uri);
            assets.insert(entry.asset_id.clone(), entry);
        }

        Self {
            schema_version: PROJECT_SCHEMA_VERSION,
            project_id: ProjectId::from_slug("untitled"),
            name: "Untitled Project".to_string(),
            character: CharacterState::default(),
            scene,
            materials,
            animation: AnimationState::default(),
            render: RenderState::default(),
            assets,
            settings: ProjectSettings::default(),
            extensions: BTreeMap::new(),
        }
    }
}

impl ProjectState {
    /// Creates a project with an explicit project id.
    pub fn with_project_id(project_id: ProjectId, name: impl Into<String>) -> Self {
        Self {
            project_id,
            name: name.into(),
            ..Self::default()
        }
    }

    /// Resolves the asset id of the canonical base mesh for a gender.
    pub fn base_mesh_asset_id(gender: BaseGender) -> AssetId {
        AssetId::for_uri(match gender {
            BaseGender::Male => URI_BASE_MALE,
            BaseGender::Female => URI_BASE_FEMALE,
        })
    }

    /// Returns the authoritative morph value of a catalog slider (catalog
    /// default when the project does not override it).
    pub fn morph_value(&self, slider_id: &str) -> f32 {
        let morph_id = MorphId::for_slider(slider_id);
        if let Some(value) = self.character.morph_values.get(&morph_id) {
            return *value;
        }
        find_slider_def(slider_id).map_or(0.0, |def| def.default_value)
    }

    /// Sets a morph value in place (used by the command layer only).
    pub(crate) fn set_morph_value(&mut self, slider_id: &str, value: f32) {
        let morph_id = MorphId::for_slider(slider_id);
        let default = find_slider_def(slider_id).map_or(0.0, |def| def.default_value);
        if (value - default).abs() <= f32::EPSILON {
            self.character.morph_values.remove(&morph_id);
        } else {
            self.character.morph_values.insert(morph_id, value);
        }
    }

    /// Active (non-default) morph values as `(slider_id, value)` pairs in
    /// canonical catalog order — the deterministic order used by snapshots and
    /// golden tests.
    pub fn active_morph_values(&self) -> Vec<(&'static str, f32)> {
        let mut out = Vec::new();
        for def in ALL_MORPH_SLIDERS.iter() {
            if let Some(value) = self.character.morph_values.get(&MorphId::for_slider(def.id)) {
                if (value - def.default_value).abs() > 1e-6 {
                    out.push((def.id, *value));
                }
            }
        }
        out
    }

    /// Catalog-normalized morph weights (`value - default`) keyed by slider id.
    pub fn morph_weights(&self) -> Vec<(&'static str, f32)> {
        let mut out = Vec::new();
        for def in ALL_MORPH_SLIDERS.iter() {
            let value = self.morph_value(def.id);
            let weight = value - def.default_value;
            if weight.abs() > 1e-6 {
                out.push((def.id, weight));
            }
        }
        out
    }

    /// Scene node holding the character mesh, if any.
    pub fn character_node(&self) -> Option<&NodeSlot> {
        self.scene.nodes.iter().find(|node| {
            node.mesh
                .as_ref()
                .is_some_and(|mesh| mesh.asset_id == Self::base_mesh_asset_id(self.character.base_gender))
        })
    }

    /// Material used by the character node (falls back to the first material).
    pub fn character_material(&self) -> Option<&MaterialEntry> {
        let node_material = self
            .character_node()
            .and_then(|node| node.material_id.as_ref())
            .and_then(|id| self.materials.get(id));
        node_material.or_else(|| self.materials.values().next())
    }

    /// Resolves a light by canonical id.
    pub fn light(&self, id: &LightId) -> Option<&StylizedLight> {
        self.scene
            .lights
            .iter()
            .find(|slot| &slot.light_id == id)
            .map(|slot| &slot.light)
    }

    /// Mutable access to the key light, falling back to inserting the default.
    pub fn key_light_mut(&mut self) -> &mut StylizedLight {
        let key = LightId::canonical_key();
        if self.scene.lights.iter().all(|slot| slot.light_id != key) {
            self.scene.lights.push(LightSlot {
                light_id: key.clone(),
                light: StylizedLight::default(),
            });
        }
        let index = self
            .scene
            .lights
            .iter()
            .position(|slot| slot.light_id == key)
            .unwrap_or(0);
        &mut self.scene.lights[index].light
    }
}

// ---------------------------------------------------------------------------
// Validation + sanitization
// ---------------------------------------------------------------------------

fn require_finite(field: &str, value: f32) -> Result<(), ProjectError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(ProjectError::InvalidValue {
            field: field.to_string(),
            detail: "must be a finite number".to_string(),
        })
    }
}

fn require_range(field: &str, value: f32, min: f32, max: f32) -> Result<(), ProjectError> {
    require_finite(field, value)?;
    if value < min || value > max {
        Err(ProjectError::InvalidValue {
            field: field.to_string(),
            detail: format!("must be within [{min}, {max}], found {value}"),
        })
    } else {
        Ok(())
    }
}

fn require_nonzero_vec(field: &str, value: [f32; 3]) -> Result<(), ProjectError> {
    for component in value {
        require_finite(field, component)?;
    }
    let length_sq = value[0] * value[0] + value[1] * value[1] + value[2] * value[2];
    if length_sq <= 1e-9 {
        return Err(ProjectError::InvalidValue {
            field: field.to_string(),
            detail: "must be a non-zero vector".to_string(),
        });
    }
    Ok(())
}

impl ProjectState {
    /// Strict validation used before applying a project to the runtime.
    pub fn validate(&self) -> Result<(), ProjectError> {
        if self.schema_version != PROJECT_SCHEMA_VERSION {
            return Err(ProjectError::UnsupportedSchemaVersion {
                found: self.schema_version,
                supported: PROJECT_SCHEMA_VERSION,
            });
        }

        require_range(
            "character.gender_dimorphism",
            self.character.gender_dimorphism,
            0.0,
            1.0,
        )?;
        let somatotype = self.character.somatotype;
        for (field, value) in [
            ("character.somatotype.endomorph", somatotype.endomorph),
            ("character.somatotype.mesomorph", somatotype.mesomorph),
            ("character.somatotype.ectomorph", somatotype.ectomorph),
        ] {
            require_range(field, value, 0.0, 1.0)?;
        }
        if (somatotype.endomorph + somatotype.mesomorph + somatotype.ectomorph - 1.0).abs() > 1e-3 {
            return Err(ProjectError::InvalidValue {
                field: "character.somatotype".to_string(),
                detail: "components must be normalized to sum 1.0".to_string(),
            });
        }

        for (morph_id, value) in &self.character.morph_values {
            let slider_id = morph_id.slider_id();
            let def = find_slider_def(slider_id)
                .ok_or_else(|| ProjectError::UnknownMorph(slider_id.to_string()))?;
            require_range(
                &format!("character.morph_values.{slider_id}"),
                *value,
                def.min,
                def.max,
            )?;
        }

        if self.scene.nodes.is_empty() {
            return Err(ProjectError::EmptyScene);
        }

        let mut node_ids: BTreeSet<&str> = BTreeSet::new();
        for node in &self.scene.nodes {
            if !node_ids.insert(node.node_id.as_str()) {
                return Err(ProjectError::DuplicateId {
                    kind: "scene node",
                    id: node.node_id.to_string(),
                });
            }
            if let Some(mesh) = &node.mesh {
                if !self.assets.contains_key(&mesh.asset_id) {
                    return Err(ProjectError::DanglingReference {
                        field: "scene.nodes[].mesh.asset_id",
                        kind: "asset",
                        target: mesh.asset_id.to_string(),
                    });
                }
            }
            if let Some(material_id) = &node.material_id {
                if !self.materials.contains_key(material_id) {
                    return Err(ProjectError::DanglingReference {
                        field: "scene.nodes[].material_id",
                        kind: "material",
                        target: material_id.to_string(),
                    });
                }
            }
            for component in node.transform.translation.to_array() {
                require_finite("scene.nodes[].transform.translation", component)?;
            }
            for component in node.transform.scale.to_array() {
                require_finite("scene.nodes[].transform.scale", component)?;
            }
            for component in node.transform.rotation.to_array() {
                require_finite("scene.nodes[].transform.rotation", component)?;
            }
        }

        // Issue #12: a topologia é validada como um todo — ids únicos, todo
        // `parent_id` existente, nenhum nó sendo seu próprio pai e nenhum ciclo.
        // Um comando que produza qualquer um desses estados é revertido por
        // `Command::apply` (validate → draft → validate → swap).
        hierarchy::validate_hierarchy(&self.scene.nodes)
            .map_err(|error| ProjectError::InvalidHierarchy {
                detail: error.to_string(),
            })?;

        for (id, entry) in &self.materials {
            if entry.material_id != *id {
                return Err(ProjectError::InvalidValue {
                    field: format!("materials.{id}.material_id"),
                    detail: "map key and embedded id must match".to_string(),
                });
            }
        }
        for (id, entry) in &self.assets {
            if entry.asset_id != *id {
                return Err(ProjectError::InvalidValue {
                    field: format!("assets.{id}.asset_id"),
                    detail: "map key and embedded id must match".to_string(),
                });
            }
            if Path::new(&entry.uri).is_absolute() {
                return Err(ProjectError::InvalidValue {
                    field: format!("assets.{id}.uri"),
                    detail: "absolute machine paths are not portable project data".to_string(),
                });
            }
        }

        for clip in self.animation.clips.values() {
            for track in &clip.tracks {
                if !node_ids.contains(track.node_id.as_str()) {
                    return Err(ProjectError::DanglingReference {
                        field: "animation.clips[].tracks[].node_id",
                        kind: "scene node",
                        target: track.node_id.to_string(),
                    });
                }
            }
        }
        if let Some(active) = &self.animation.active_clip {
            if !self.animation.clips.contains_key(active) {
                return Err(ProjectError::DanglingReference {
                    field: "animation.active_clip",
                    kind: "animation clip",
                    target: active.to_string(),
                });
            }
        }
        require_finite("animation.playhead_seconds", self.animation.playhead_seconds)?;

        let camera = &self.scene.camera.camera;
        for (field, vector) in [
            ("scene.camera.camera.eye", camera.eye.to_array()),
            ("scene.camera.camera.target", camera.target.to_array()),
            ("scene.camera.camera.up", camera.up.to_array()),
        ] {
            for component in vector {
                require_finite(field, component)?;
            }
        }
        if camera.eye == camera.target {
            return Err(ProjectError::InvalidValue {
                field: "scene.camera.camera.eye".to_string(),
                detail: "eye and target must differ".to_string(),
            });
        }
        require_nonzero_vec("scene.camera.camera.up", camera.up.to_array())?;
        require_range("scene.camera.camera.fov_y", camera.fov_y, 1e-3, 3.14159)?;
        require_range("scene.camera.camera.z_near", camera.z_near, 1e-4, 10.0)?;
        require_range("scene.camera.camera.z_far", camera.z_far, 0.01, 10_000.0)?;
        if camera.z_far <= camera.z_near {
            return Err(ProjectError::InvalidValue {
                field: "scene.camera.camera.z_far".to_string(),
                detail: "must be greater than z_near".to_string(),
            });
        }

        require_nonzero_vec(
            "scene.lights[].light.direction",
            self.scene
                .lights
                .first()
                .map_or([0.577, 0.577, 0.577], |slot| slot.light.direction),
        )?;
        for slot in &self.scene.lights {
            require_range(
                &format!("scene.lights.{}.intensity", slot.light_id),
                slot.light.intensity,
                0.0,
                32.0,
            )?;
        }

        require_range("render.msaa_samples", self.render.msaa_samples as f32, 1.0, 16.0)?;
        for component in self.render.background_color {
            require_range("render.background_color", component, 0.0, 1.0)?;
        }

        Ok(())
    }

    /// Lenient normalization used right after loading untrusted data.
    ///
    /// Removes unknown morph ids, clamps out-of-range numbers, drops dangling
    /// references and restores mandatory defaults. Never panics.
    pub fn sanitize(mut self) -> Self {
        self.schema_version = PROJECT_SCHEMA_VERSION;
        if self.name.trim().is_empty() {
            self.name = "Untitled Project".to_string();
        }

        self.character.gender_dimorphism = clamp01(self.character.gender_dimorphism, 1.0);
        self.character.somatotype = SomatotypeCoords::new(
            component_or(self.character.somatotype.endomorph, 1.0 / 3.0),
            component_or(self.character.somatotype.mesomorph, 1.0 / 3.0),
            component_or(self.character.somatotype.ectomorph, 1.0 / 3.0),
        )
        .normalized();

        let mut morph_values = BTreeMap::new();
        for (morph_id, value) in std::mem::take(&mut self.character.morph_values) {
            let Some(def) = find_slider_def(morph_id.slider_id()) else {
                continue; // unknown slider: dropped, never fabricated
            };
            if !value.is_finite() {
                continue;
            }
            let clamped = value.clamp(def.min, def.max);
            if (clamped - def.default_value).abs() > 1e-6 {
                morph_values.insert(MorphId::for_slider(def.id), clamped);
            }
        }
        self.character.morph_values = morph_values;

        // Dangling mesh references become procedural canonical bases.
        for node in &mut self.scene.nodes {
            if let Some(mesh) = &node.mesh {
                if !self.assets.contains_key(&mesh.asset_id) {
                    let fallback = AssetId::for_uri(match self.character.base_gender {
                        BaseGender::Male => URI_BASE_MALE,
                        BaseGender::Female => URI_BASE_FEMALE,
                    });
                    node.mesh = Some(MeshRef {
                        asset_id: fallback,
                        primitive_index: 0,
                    });
                }
            }
            if let Some(material_id) = &node.material_id {
                if !self.materials.contains_key(material_id) {
                    node.material_id = Some(MaterialId::canonical_default());
                }
            }
        }
        if self.scene.nodes.is_empty() {
            self.scene.nodes = SceneState::default().nodes;
        }

        if !self
            .materials
            .contains_key(&MaterialId::canonical_default())
        {
            let entry = MaterialEntry {
                material_id: MaterialId::canonical_default(),
                name: "DefaultAnimeMaterial".to_string(),
                material: StylizedMaterial::default(),
            };
            self.materials.insert(entry.material_id.clone(), entry);
        }
        for (id, entry) in self.materials.iter_mut() {
            entry.material_id = id.clone();
            if !entry.name.trim().is_empty() {
                continue;
            }
            entry.name = id.to_string();
        }

        // Ensure the built-in assets exist so every `MeshRef` stays resolvable
        // (a geometry command never fabricates registry entries — see
        // `BUILTIN_MESH_URIS`).
        for uri in BUILTIN_MESH_URIS {
            let entry = AssetEntry::from_uri(AssetKind::Mesh, uri);
            self.assets.entry(entry.asset_id.clone()).or_insert(entry);
        }
        for (id, entry) in self.assets.iter_mut() {
            entry.asset_id = id.clone();
            if entry.uri.trim().is_empty() {
                entry.uri = format!("anigo://unknown/{id}");
            }
        }

        self.animation.clips.retain(|id, clip| {
            clip.clip_id = id.clone();
            clip.duration_seconds.is_finite() && clip.duration_seconds >= 0.0
        });
        let known_nodes: BTreeSet<NodeId> = self
            .scene
            .nodes
            .iter()
            .map(|node| node.node_id.clone())
            .collect();
        for clip in self.animation.clips.values_mut() {
            clip.tracks.retain(|track| known_nodes.contains(&track.node_id));
        }
        if let Some(active) = self.animation.active_clip.clone() {
            if !self.animation.clips.contains_key(&active) {
                self.animation.active_clip = None;
            }
        }
        if !self.animation.playhead_seconds.is_finite() {
            self.animation.playhead_seconds = 0.0;
        }

        if !self.scene.camera.camera.eye.is_finite()
            || !self.scene.camera.camera.target.is_finite()
            || !self.scene.camera.camera.up.is_finite()
        {
            self.scene.camera.camera = Camera::default();
        }
        if !self.scene.camera.camera.fov_y.is_finite() || self.scene.camera.camera.fov_y <= 0.0 {
            self.scene.camera.camera.fov_y = Camera::default().fov_y;
        }

        self.render.msaa_samples = self.render.msaa_samples.clamp(1, 16);
        for component in self.render.background_color.iter_mut() {
            *component = component_or(*component, 0.0).clamp(0.0, 1.0);
        }

        self.settings.autosave_interval_minutes = self.settings.autosave_interval_minutes.clamp(1, 240);
        self.settings.history_limit = self.settings.history_limit.clamp(1, 1000);

        self
    }

    /// Stable fingerprint of the authoritative content (FNV-1a over the
    /// canonical JSON). Used by tests and by the snapshot static cache to
    /// detect real changes without diffing whole documents.
    pub fn content_fingerprint(&self) -> u64 {
        match self.to_canonical_json() {
            Ok(json) => fnv1a64(&json),
            Err(_) => 0,
        }
    }

    /// Deterministic serialization (sorted maps, pretty printed).
    pub fn to_canonical_json(&self) -> Result<String, ProjectError> {
        serde_json::to_string_pretty(self).map_err(|e| ProjectError::Json(e.to_string()))
    }

    /// Returns the JSON value of this project (canonical key order).
    pub fn to_value(&self) -> Result<Value, ProjectError> {
        serde_json::to_value(self).map_err(|e| ProjectError::Json(e.to_string()))
    }

    /// Parses a project payload: detects legacy/foreign schemas, migrates them,
    /// sanitizes hostile values and validates the result.
    pub fn from_json(raw: &str) -> Result<Self, ProjectError> {
        let value: Value = serde_json::from_str(raw).map_err(|e| ProjectError::Json(e.to_string()))?;
        Self::from_value(value)
    }

    /// Same as [`ProjectState::from_json`] for an already parsed value.
    pub fn from_value(value: Value) -> Result<Self, ProjectError> {
        let migrated = Self::migrate_value(value)?;
        // O tipo do `from_value` precisa ser explícito: com `.sanitize()` no
        // encadeamento o parâmetro `T` fica ambíguo (E0282) — o alvo do `let`
        // não o restringe, porque quem define a saída é o método.
        let parsed: ProjectState = serde_json::from_value(migrated)
            .map_err(|e| ProjectError::Json(e.to_string()))?;
        let project = parsed.sanitize();
        project.validate()?;
        Ok(project)
    }

    /// Explicit migration entry point.
    ///
    /// Currently supported inputs:
    /// - canonical v1 documents (`project_id` + `schema_version = 1`);
    /// - legacy TypeScript autosave snapshots (no `project_id`; flattened
    ///   `character`/`preset`/`cameraEye`/`lightDir` fields, TS schema
    ///   versions 1 and 2), preserved under `extensions.legacy_ts_snapshot`.
    pub fn migrate_value(value: Value) -> Result<Value, ProjectError> {
        let object = value
            .as_object()
            .ok_or_else(|| ProjectError::NotAnObject {
                found: describe_value(&value),
            })?
            .clone();

        let has_project_id = object.contains_key("project_id");
        if !has_project_id {
            return migrate_legacy_snapshot(object);
        }

        let schema_version = match object.get("schema_version") {
            Some(Value::Number(number)) => number
                .as_u64()
                .map(|v| v as u32)
                .ok_or(ProjectError::MissingSchemaVersion)?,
            Some(_) => return Err(ProjectError::MissingSchemaVersion),
            None => return Err(ProjectError::MissingSchemaVersion),
        };

        if schema_version > PROJECT_SCHEMA_VERSION {
            return Err(ProjectError::UnsupportedSchemaVersion {
                found: schema_version,
                supported: PROJECT_SCHEMA_VERSION,
            });
        }
        if schema_version < PROJECT_SCHEMA_VERSION {
            return Err(ProjectError::UnsupportedSchemaVersion {
                found: schema_version,
                supported: PROJECT_SCHEMA_VERSION,
            });
        }

        Ok(Value::Object(object))
    }
}

fn describe_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(_) => "boolean".to_string(),
        Value::Number(_) => "number".to_string(),
        Value::String(_) => "string".to_string(),
        Value::Array(_) => "array".to_string(),
        Value::Object(_) => "object".to_string(),
    }
}

fn clamp01(value: f32, fallback: f32) -> f32 {
    component_or(value, fallback).clamp(0.0, 1.0)
}

fn component_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

// ---------------------------------------------------------------------------
// Legacy TypeScript snapshot migration (schema v0 → v1)
// ---------------------------------------------------------------------------

fn number_at(object: &Map<String, Value>, key: &str, fallback: f32) -> f32 {
    match object.get(key) {
        Some(Value::Number(number)) => number.as_f64().map(|v| v as f32).unwrap_or(fallback),
        _ => fallback,
    }
}

/// Igual a [`string_at`], mas para um `Value` que pode não ser objeto (o
/// "preservado" da migração é um objeto, mas a função não deve assumir).
fn string_in(value: &Value, key: &str, fallback: &str) -> String {
    match value.as_object() {
        Some(object) => string_at(object, key, fallback),
        None => fallback.to_string(),
    }
}

fn string_at(object: &Map<String, Value>, key: &str, fallback: &str) -> String {
    match object.get(key) {
        Some(Value::String(value)) => value.clone(),
        _ => fallback.to_string(),
    }
}

fn vec3_at(object: &Map<String, Value>, key: &str, fallback: [f32; 3]) -> [f32; 3] {
    match object.get(key) {
        Some(Value::Array(items)) if items.len() >= 3 => {
            let mut out = fallback;
            for (index, slot) in out.iter_mut().enumerate() {
                if let Some(Value::Number(number)) = items.get(index) {
                    if let Some(number) = number.as_f64() {
                        *slot = number as f32;
                    }
                }
            }
            out
        }
        _ => fallback,
    }
}

/// Parses `#rrggbb` / `#rrggbbaa` (legacy TS payloads store colors as hex).
fn parse_hex_rgba(raw: &str, fallback: [f32; 4]) -> [f32; 4] {
    let hex = raw.trim().trim_start_matches('#');
    let parse_pair = |index: usize| -> Option<f32> {
        let start = index * 2;
        let slice = hex.get(start..start + 2)?;
        u8::from_str_radix(slice, 16).ok().map(|v| v as f32 / 255.0)
    };
    match hex.len() {
        6 => {
            let (Some(r), Some(g), Some(b)) = (parse_pair(0), parse_pair(1), parse_pair(2)) else {
                return fallback;
            };
            [r, g, b, 1.0]
        }
        8 => {
            let (Some(r), Some(g), Some(b), Some(a)) =
                (parse_pair(0), parse_pair(1), parse_pair(2), parse_pair(3))
            else {
                return fallback;
            };
            [r, g, b, a]
        }
        _ => fallback,
    }
}

/// Converts a TS `preset` value into the canonical mesh reference.
fn preset_mesh_ref(preset: &str, gender: BaseGender) -> MeshRef {
    let uri = match preset {
        "cube" => URI_PRESET_CUBE,
        "sphere" => URI_PRESET_SPHERE,
        _ => match gender {
            BaseGender::Male => URI_BASE_MALE,
            BaseGender::Female => URI_BASE_FEMALE,
        },
    };
    MeshRef {
        asset_id: AssetId::for_uri(uri),
        primitive_index: 0,
    }
}

/// Upgrades a legacy TypeScript autosave payload (`ProjectStateSnapshot`) into
/// the canonical v1 document. Unknown legacy keys are preserved under
/// `extensions.legacy_ts_snapshot` so no authored data is ever lost.
pub fn migrate_legacy_snapshot(mut object: Map<String, Value>) -> Result<Value, ProjectError> {
    let legacy_schema_version = match object.get("schemaVersion") {
        Some(Value::Number(number)) => number.as_u64().unwrap_or(1) as u32,
        _ => 1,
    };
    // Legacy documents never carried a project id; a *canonical* document with
    // a future schema version is rejected earlier (see `migrate_value`).
    if legacy_schema_version > 9999 {
        return Err(ProjectError::UnsupportedSchemaVersion {
            found: legacy_schema_version,
            supported: PROJECT_SCHEMA_VERSION,
        });
    }

    let character_value = object
        .get("character")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();

    let base_gender = match character_value.get("baseGender") {
        Some(Value::String(value)) if value == "female" => BaseGender::Female,
        _ => BaseGender::Male,
    };

    // --- character -------------------------------------------------------
    let mut morph_values = Map::new();
    if let Some(Value::Object(raw_morphs)) = character_value.get("morphSliders") {
        for (slider_id, raw_value) in raw_morphs {
            let Some(def) = find_slider_def(slider_id) else {
                continue; // unknown slider: dropped by the sanitizer as well
            };
            let value = match raw_value {
                Value::Number(number) => number.as_f64().map(|v| v as f32).unwrap_or(def.default_value),
                _ => continue,
            };
            let clamped = value.clamp(def.min, def.max);
            if (clamped - def.default_value).abs() > 1e-6 {
                morph_values.insert(MorphId::for_slider(slider_id).to_string(), json!(clamped));
            }
        }
    }

    let somatotype = character_value
        .get("somatotype")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();
    let endpoints = [
        ("endo", "endomorph"),
        ("meso", "mesomorph"),
        ("ecto", "ectomorph"),
    ];
    let somatotype_json = {
        let mut map = Map::new();
        for (legacy_key, canonical_key) in endpoints {
            map.insert(canonical_key.to_string(), json!(number_at(&somatotype, legacy_key, 1.0 / 3.0)));
        }
        Value::Object(map)
    };

    let proportions = character_value
        .get("proportions")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();
    let default_proportions = CharacterProportions::default();
    // O payload legado guarda as proporções em camelCase (`headScale`), não no
    // snake_case canônico: ler `head_scale` devolvia sempre o fallback e a
    // migração descartava as proporções autoradas em silêncio.
    let legacy_key = |canonical: &str| camel_key(canonical);
    let proportions_json = {
        let mut map = Map::new();
        let pairs = [
            (default_proportions.head_scale, "head_scale"),
            (default_proportions.head_ratio, "head_ratio"),
            (default_proportions.shoulder_width, "shoulder_width"),
            (default_proportions.leg_length, "leg_length"),
            (default_proportions.arm_length, "arm_length"),
            (default_proportions.neck_length, "neck_length"),
            (default_proportions.torso_length, "torso_length"),
            (default_proportions.height_overall, "height_overall"),
        ];
        for (fallback, key) in pairs {
            map.insert(key.to_string(), json!(number_at(&proportions, &legacy_key(key), fallback)));
        }
        Value::Object(map)
    };

    let hair = character_value
        .get("hair")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();
    let cloth = character_value
        .get("cloth")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();
    let accessory = character_value
        .get("accessory")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();
    let defaults = CharacterState::default();

    let character_json = json!({
        "character_id": CharacterId::canonical().to_string(),
        "base_gender": match base_gender {
            BaseGender::Male => "Male",
            BaseGender::Female => "Female",
        },
        "gender_dimorphism": clamp01(
            number_at(&character_value, "genderDimorphism", defaults.gender_dimorphism),
            defaults.gender_dimorphism,
        ),
        "somatotype": somatotype_json,
        "proportions": proportions_json,
        "morph_values": Value::Object(morph_values),
        "active_preset_id": character_value
            .get("activePresetId")
            .and_then(|value| value.as_str())
            .map(|value| Value::String(value.to_string()))
            .unwrap_or(Value::Null),
        "hair": {
            "volume": number_at(&hair, "volume", defaults.hair.volume),
            "thickness": number_at(&hair, "thickness", defaults.hair.thickness),
            "curvature": number_at(&hair, "curvature", defaults.hair.curvature),
            "strands": number_at(&hair, "strands", defaults.hair.strands as f32).max(0.0) as u32,
        },
        "cloth": {
            "layer": string_at(&cloth, "layer", &defaults.cloth.layer),
            "tension": number_at(&cloth, "tension", defaults.cloth.tension),
            "rigidity": number_at(&cloth, "rigidity", defaults.cloth.rigidity),
            "gravity": number_at(&cloth, "gravity", defaults.cloth.gravity),
        },
        "accessory": {
            "socket": string_at(&accessory, "socket", &defaults.accessory.socket),
            "scale": number_at(&accessory, "scale", defaults.accessory.scale),
            "offset": [
                number_at(&accessory, "offsetX", 0.0),
                number_at(&accessory, "offsetY", 0.0),
                number_at(&accessory, "offsetZ", 0.0),
            ],
        },
    });

    // --- scene -----------------------------------------------------------
    let preset = string_at(&object, "preset", "mannequin");
    let mesh_ref = preset_mesh_ref(&preset, base_gender);
    let material_id = MaterialId::canonical_default();
    let default_material = StylizedMaterial::default();
    let light_default = StylizedLight::default();

    let scene_json = json!({
        "scene_id": SceneId::canonical().to_string(),
        "camera": {
            "camera_id": CameraId::canonical().to_string(),
            "camera": {
                "eye": vec3_at(&object, "cameraEye", [0.0, 1.5, 3.5]),
                "target": vec3_at(&object, "cameraTarget", [0.0, 1.0, 0.0]),
                "up": vec3_at(&object, "cameraUp", [0.0, 1.0, 0.0]),
                "fov_y": number_at(&object, "fov", 45.0).to_radians(),
                "aspect": 16.0 / 9.0,
                "z_near": 0.05,
                "z_far": 100.0,
            },
        },
        "lights": [{
            "light_id": LightId::canonical_key().to_string(),
            "light": {
                "direction": vec3_at(&object, "lightDir", light_default.direction),
                "color": vec3_at(&object, "lightColor", light_default.color),
                "intensity": number_at(&object, "lightIntensity", light_default.intensity),
                "shadow_color": vec3_at(&object, "shadowColor", light_default.shadow_color),
                "ambient_intensity": number_at(&object, "ambientIntensity", light_default.ambient_intensity),
                "shadow_saturation": number_at(&object, "shadowSaturation", light_default.shadow_saturation),
                "ambient_sky": vec3_at(&object, "ambientSky", light_default.ambient_sky),
                "ambient_ground": vec3_at(&object, "ambientGround", light_default.ambient_ground),
            },
        }],
        "nodes": [{
            "node_id": NodeId::canonical_character().to_string(),
            "name": preset_label(&preset),
            "transform": {
                "translation": [0.0, 0.0, 0.0],
                "rotation": [0.0, 0.0, 0.0, 1.0],
                "scale": [1.0, 1.0, 1.0],
            },
            "mesh": {
                "asset_id": mesh_ref.asset_id.to_string(),
                "primitive_index": mesh_ref.primitive_index,
            },
            "material_id": material_id.to_string(),
            "visible": true,
        }],
    });

    // --- materials -------------------------------------------------------
    let material_json = json!({
        "material_id": material_id.to_string(),
        "name": default_material.name,
        "material": {
            "name": default_material.name,
            "base_color": parse_hex_rgba(
                &string_at(&object, "baseColorHex", "#faebd7"),
                default_material.base_color,
            ),
            "shade_color": parse_hex_rgba(
                &string_at(&object, "shadowColorHex", "#d1b8c7"),
                default_material.shade_color,
            ),
            "outline_color": parse_hex_rgba(
                &string_at(&object, "outlineColor", "#402633"),
                default_material.outline_color,
            ),
            "outline_width": number_at(&object, "outlineWidth", default_material.outline_width * 1000.0) / 1000.0,
            "shadow_threshold": number_at(&object, "shadowThreshold", default_material.shadow_threshold),
            "shadow_smoothness": number_at(&object, "toonSmoothness", default_material.shadow_smoothness),
            "spec_intensity": number_at(&object, "specIntensity", default_material.spec_intensity),
            "spec_power": number_at(&object, "specExponent", default_material.spec_power),
            "rim_intensity": number_at(&object, "rimIntensity", default_material.rim_intensity),
            "rim_spread": number_at(&object, "rimSpread", default_material.rim_spread),
            "hue_shift": number_at(&object, "hueShift", default_material.hue_shift),
            "toon_steps": number_at(&object, "toonSteps", default_material.toon_steps),
            "specular_color": parse_hex_rgba(
                &string_at(&object, "specColorHex", "#ffffff"),
                default_material.specular_color,
            ),
            "specular_softness": number_at(&object, "specSoftness", default_material.specular_softness),
            "specular_offset": number_at(&object, "specOffset", default_material.specular_offset),
            "rim_color": parse_hex_rgba(
                &string_at(&object, "rimColor", "#93c5fd"),
                default_material.rim_color,
            ),
            "outline_opacity": number_at(&object, "outlineOpacity", default_material.outline_opacity),
            "outline_smoothness": number_at(&object, "outlineSmoothness", default_material.outline_smoothness),
            "outline_depth_bias": number_at(&object, "outlineDepthBias", default_material.outline_depth_bias),
            "specular_size": number_at(&object, "specularSize", default_material.specular_size),
            "ao_intensity": number_at(&object, "aoIntensity", default_material.ao_intensity),
        },
    });

    // --- assets ----------------------------------------------------------
    let mut assets = Map::new();
    for uri in [
        URI_BASE_MALE,
        URI_BASE_FEMALE,
        URI_PRESET_CUBE,
        URI_PRESET_SPHERE,
        // Preserve the user-facing asset path of the presets that ship with
        // the desktop build (relative, portable, no machine paths).
        "assets/models/anigo_base_male.glb",
        "assets/models/anigo_base_female.glb",
    ] {
        let entry = AssetEntry::from_uri(AssetKind::Mesh, uri);
        assets.insert(
            entry.asset_id.to_string(),
            json!({
                "asset_id": entry.asset_id.to_string(),
                "kind": "mesh",
                "uri": entry.uri,
            }),
        );
    }

    // --- render + settings ----------------------------------------------
    let render_json = json!({
        "settings_version": 1,
        "msaa_samples": 4,
        "background_color": [0.08, 0.09, 0.13, 1.0],
        "color": {
            "input_texture_space": "srgb",
            "working_space": "linear_srgb",
            "display_space": "srgb",
        },
        "tonemap": "none",
    });
    let settings_json = json!({
        "autosave_interval_minutes": DEFAULT_AUTOSAVE_INTERVAL_MINUTES,
        "history_limit": DEFAULT_HISTORY_LIMIT,
        "locale": "pt_BR",
        "units": "meters",
    });

    // Everything that had no canonical home is preserved verbatim.
    let preserved = Value::Object(std::mem::take(&mut object));
    let project_id = ProjectId::from_slug(&format!(
        "legacy_{:016x}",
        fnv1a64(&format!(
            "{}|{}",
            preset,
            string_in(&preserved, "version", "0.1.0")
        ))
    ));
    let name = format!(
        "{} (migrado de schema TS {})",
        preset_label(&preset),
        legacy_schema_version
    );

    let mut materials_map = Map::new();
    materials_map.insert(material_id.to_string(), material_json);

    let mut canonical = Map::new();
    canonical.insert(
        "schema_version".to_string(),
        json!(PROJECT_SCHEMA_VERSION),
    );
    canonical.insert("project_id".to_string(), json!(project_id.to_string()));
    canonical.insert("name".to_string(), json!(name));
    canonical.insert("character".to_string(), character_json);
    canonical.insert("scene".to_string(), scene_json);
    canonical.insert("materials".to_string(), Value::Object(materials_map));
    canonical.insert("animation".to_string(), json!({ "clips": {}, "active_clip": null, "playhead_seconds": 0.0 }));
    canonical.insert("render".to_string(), render_json);
    canonical.insert("assets".to_string(), Value::Object(assets));
    canonical.insert("settings".to_string(), settings_json);
    // `ProjectState::extensions` é `#[serde(flatten)]`: as chaves estendidas
    // vivem **no topo** do documento, não sob uma chave `extensions`. Gravar o
    // objeto nomeado fazia o payload legado ser lido de volta um nível abaixo
    // (`extensions.extensions.legacy_ts_snapshot`), ou seja a preservação
    // prometida por §3.1 se perdia no primeiro save.
    canonical.insert(
        "migrated_from".to_string(),
        json!("legacy_ts_snapshot"),
    );
    canonical.insert(
        "legacy_schema_version".to_string(),
        json!(legacy_schema_version),
    );
    canonical.insert("legacy_ts_snapshot".to_string(), preserved);

    Ok(Value::Object(canonical))
}

fn preset_label(preset: &str) -> String {
    match preset {
        "cube" => "Cube".to_string(),
        "sphere" => "UV Sphere".to_string(),
        "female" => "Anime Mannequin (Female)".to_string(),
        _ => "Anime Mannequin".to_string(),
    }
}

/// camelCases a canonical field name (`head_scale` → `headScale`).
///
/// Inverso de [`slug_key`]: o autosave legado do TypeScript grava as proporções
/// em camelCase, então a migração precisa procurar por essa forma.
fn camel_key(canonical: &str) -> String {
    let mut out = String::with_capacity(canonical.len());
    let mut uppercase_next = false;
    for character in canonical.chars() {
        if character == '_' {
            uppercase_next = true;
            continue;
        }
        if uppercase_next {
            out.extend(character.to_uppercase());
            uppercase_next = false;
        } else {
            out.push(character);
        }
    }
    out
}

/// snake_cases a legacy camelCase field name (`headScale` → `head_scale`).
#[allow(dead_code)]
fn slug_key(legacy: &str) -> String {
    let mut out = String::with_capacity(legacy.len() + 4);
    for (index, ch) in legacy.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Derived scene (renderer cache, §4.3)
// ---------------------------------------------------------------------------

impl ProjectState {
    /// Builds the renderer-facing [`Scene`] from the authoritative project.
    ///
    /// The project stores references, never mesh data, so a resolver supplies
    /// the actual geometry (asset cache / procedural base). The returned scene
    /// is a **derived cache**: rebuilding it from `ProjectState` reproduces it
    /// exactly, and it is never written back.
    pub fn to_scene(&self, resolver: &dyn Fn(&MeshRef) -> Option<Mesh>) -> Scene {
        let mut scene = Scene::new_empty();
        scene.camera = self.scene.camera.camera.clone();
        scene.background_color = self.render.background_color;
        scene.light = self
            .light(&LightId::canonical_key())
            .cloned()
            .unwrap_or_default();

        for node in &self.scene.nodes {
            let mesh = node.mesh.as_ref().and_then(|reference| resolver(reference));
            let mut scene_node = SceneNode::new(node.node_id.to_string(), node.name.clone());
            scene_node.transform = node.transform;
            scene_node.visible = node.visible;
            scene_node.mesh = mesh;
            scene_node.material = node
                .material_id
                .as_ref()
                .and_then(|id| self.materials.get(id))
                .map(|entry| entry.material.clone())
                .or_else(|| self.materials.values().next().map(|entry| entry.material.clone()));
            scene.nodes.push(scene_node);
        }

        scene
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical_project() -> ProjectState {
        ProjectState::default()
    }

    #[test]
    fn default_project_is_valid_and_has_stable_ids() {
        let project = canonical_project();
        project.validate().expect("default project must validate");
        assert_eq!(project.schema_version, PROJECT_SCHEMA_VERSION);
        assert_eq!(project.project_id.as_str(), "prj_untitled");
        assert_eq!(project.scene.camera.camera_id.as_str(), "cam_viewport");
        assert_eq!(project.scene.lights[0].light_id.as_str(), "lgt_key");
        assert!(project.scene.nodes[0].mesh.is_some());
        assert!(project.character.morph_values.is_empty(), "defaults are not stored");
    }

    #[test]
    fn hierarchy_validation_rejects_cycles_and_dangling_parents() {
        use crate::hierarchy::NodeKind;

        let mut project = canonical_project();
        let root = project.scene.nodes[0].node_id.clone();
        project.scene.nodes.push(
            NodeSlot::new(NodeId::from_slug("nod_jacket"), "Jacket", NodeKind::Clothing)
                .child_of(root.clone()),
        );
        project.scene.nodes.push(
            NodeSlot::new(NodeId::from_slug("nod_hood"), "Hood", NodeKind::Hair)
                .child_of(NodeId::from_slug("nod_jacket")),
        );
        project.validate().expect("hierarchy is valid");
        assert_eq!(project.scene.children_of(Some(&root)).len(), 1);
        assert_eq!(project.scene.depth_of(&NodeId::from_slug("nod_hood")), Some(2));

        // Pai inexistente.
        let mut dangling = project.clone();
        dangling.scene.nodes[1].parent_id = Some(NodeId::from_slug("nod_missing"));
        assert!(matches!(
            dangling.validate(),
            Err(ProjectError::InvalidHierarchy { .. })
        ));

        // Ciclo (a raiz virando filha do próprio descendente).
        let mut cyclic = project.clone();
        cyclic.scene.nodes[0].parent_id = Some(NodeId::from_slug("nod_hood"));
        assert!(matches!(
            cyclic.validate(),
            Err(ProjectError::InvalidHierarchy { .. })
        ));

        // Nó sendo pai de si mesmo.
        let mut selfish = project.clone();
        let hood_id = selfish.scene.nodes[2].node_id.clone();
        selfish.scene.nodes[2].parent_id = Some(hood_id);
        assert!(matches!(
            selfish.validate(),
            Err(ProjectError::InvalidHierarchy { .. })
        ));

        // Rotação não finita (a escala/translação já eram checadas).
        let mut broken = project;
        broken.scene.nodes[1].transform.rotation = glam::Quat::from_array([f32::NAN, 0.0, 0.0, 1.0]);
        assert!(matches!(
            broken.validate(),
            Err(ProjectError::InvalidValue { .. })
        ));
    }

    #[test]
    fn morph_values_round_trip_through_stable_ids() {
        let mut project = canonical_project();
        project.set_morph_value("head_width", 1.25);
        project.set_morph_value("jaw_v_line_taper", 0.75);

        let stored: Vec<String> = project
            .character
            .morph_values
            .keys()
            .map(|id| id.as_str().to_string())
            .collect();
        assert!(stored.contains(&"mrf_head_width".to_string()));

        let json = project.to_canonical_json().unwrap();
        assert!(json.contains("\"mrf_head_width\""), "ids are persisted, not indexes");
        let reloaded = ProjectState::from_json(&json).unwrap();
        assert_eq!(project, reloaded);
        assert!((reloaded.morph_value("head_width") - 1.25).abs() < 1e-6);

        // Setting back to the catalog default removes the override (sparse).
        let default = find_slider_def("head_width").unwrap().default_value;
        let mut touched = canonical_project();
        touched.set_morph_value("head_width", default);
        assert!(touched.character.morph_values.is_empty());
    }

    #[test]
    fn canonical_json_is_deterministic_and_idempotent() {
        let mut project = canonical_project();
        project.set_morph_value("bust_volume_cup", 0.9);
        project.set_morph_value("head_width", 1.1);
        let first = project.to_canonical_json().unwrap();
        let second = project.to_canonical_json().unwrap();
        assert_eq!(first, second, "serialization must be deterministic");
        let reloaded = ProjectState::from_json(&first).unwrap();
        assert_eq!(reloaded.to_canonical_json().unwrap(), first);
        assert_eq!(reloaded.content_fingerprint(), project.content_fingerprint());
    }

    #[test]
    fn validate_rejects_dangling_material_reference() {
        let mut project = canonical_project();
        project.materials.clear();
        let err = project.validate().unwrap_err();
        assert!(matches!(err, ProjectError::DanglingReference { .. }), "got {err:?}");
    }

    #[test]
    fn validate_rejects_duplicate_node_ids() {
        let mut project = canonical_project();
        let duplicate = project.scene.nodes[0].clone();
        project.scene.nodes.push(duplicate);
        let err = project.validate().unwrap_err();
        assert!(matches!(err, ProjectError::DuplicateId { .. }), "got {err:?}");
    }

    #[test]
    fn validate_rejects_out_of_range_and_unknown_morphs() {
        let mut project = canonical_project();
        project
            .character
            .morph_values
            .insert(MorphId::for_slider("head_width"), 99.0);
        assert!(matches!(
            project.validate().unwrap_err(),
            ProjectError::InvalidValue { .. }
        ));

        let mut project = canonical_project();
        project
            .character
            .morph_values
            .insert(MorphId::for_slider("totally_fake"), 1.0);
        assert!(matches!(
            project.validate().unwrap_err(),
            ProjectError::UnknownMorph(_)
        ));
    }

    #[test]
    fn sanitize_drops_unknown_morphs_clamps_values_and_repairs_references() {
        let mut project = canonical_project();
        project
            .character
            .morph_values
            .insert(MorphId::for_slider("head_width"), 99.0);
        project
            .character
            .morph_values
            .insert(MorphId::for_slider("nope_not_real"), 0.5);
        project
            .character
            .morph_values
            .insert(MorphId::for_slider("head_depth"), f32::NAN);
        project.materials.clear();
        project.scene.camera.camera.eye = glam::Vec3::new(f32::NAN, 0.0, 0.0);

        let sanitized = project.clone().sanitize();
        let head_width = find_slider_def("head_width").unwrap();
        assert_eq!(sanitized.morph_value("head_width"), head_width.max);
        assert!(!sanitized
            .character
            .morph_values
            .contains_key(&MorphId::for_slider("nope_not_real")));
        assert!(!sanitized
            .character
            .morph_values
            .contains_key(&MorphId::for_slider("head_depth")));
        assert!(sanitized.materials.contains_key(&MaterialId::canonical_default()));
        assert!(sanitized.scene.camera.camera.eye.is_finite());
        sanitized.validate().expect("sanitized project must validate");
    }

    #[test]
    fn rejects_future_schema_versions() {
        let mut project = canonical_project();
        project.schema_version = PROJECT_SCHEMA_VERSION + 1;
        let value = project.to_value().unwrap();
        let err = ProjectState::migrate_value(value).unwrap_err();
        assert!(matches!(
            err,
            ProjectError::UnsupportedSchemaVersion { found, supported }
                if found == PROJECT_SCHEMA_VERSION + 1 && supported == PROJECT_SCHEMA_VERSION
        ));
    }

    #[test]
    fn missing_schema_version_is_rejected_for_canonical_documents() {
        let mut value = canonical_project().to_value().unwrap();
        value.as_object_mut().unwrap().remove("schema_version");
        let err = ProjectState::migrate_value(value).unwrap_err();
        assert_eq!(err, ProjectError::MissingSchemaVersion);
    }

    #[test]
    fn unknown_fields_are_preserved_across_load() {
        let mut value = canonical_project().to_value().unwrap();
        value.as_object_mut().unwrap().insert(
            "future_feature".to_string(),
            json!({ "enabled": true, "notes": ["a", "b"] }),
        );
        let project = ProjectState::from_value(value).unwrap();
        assert_eq!(
            project.extensions.get("future_feature"),
            Some(&json!({ "enabled": true, "notes": ["a", "b"] }))
        );
        let json = project.to_canonical_json().unwrap();
        assert!(json.contains("future_feature"), "unknown fields survive a save");
    }

    #[test]
    fn legacy_ts_snapshot_migrates_character_camera_light_and_material() {
        let legacy = json!({
            "preset": "mannequin",
            "headScale": 1.1,
            "headRatio": 6.2,
            "outlineWidth": 3.5,
            "shadowThreshold": 0.45,
            "lightDir": [0.5, 0.5, 0.5],
            "lightIntensity": 1.4,
            "shadowColor": [0.9, 0.9, 1.0],
            "cameraEye": [0.0, 1.7, 3.2],
            "cameraTarget": [0.0, 1.05, 0.0],
            "fov": 50.0,
            "timestamp": 1_748_000_000_000u64,
            "version": "0.2.0",
            "schemaVersion": 2,
            "outlineColor": "#402633",
            "baseColorHex": "#faebd7",
            "hueShift": -22.0,
            "toonSteps": 2.0,
            "rimColor": "#93c5fd",
            "aoIntensity": 0.7,
            "character": {
                "schemaVersion": 2,
                "baseGender": "female",
                "activePresetId": "plus_size",
                "somatotype": { "endo": 0.5, "meso": 0.3, "ecto": 0.2 },
                "genderDimorphism": 0.37,
                "proportions": { "headScale": 1.1, "headRatio": 6.2, "shoulderWidth": 1.05 },
                "morphSliders": { "head_width": 1.3, "bogus_slider": 1.0, "jaw_v_line_taper": 0.8 },
                "hair": { "volume": 1.4, "thickness": 0.06, "curvature": 0.5, "strands": 24 },
                "cloth": { "layer": "uniforme", "tension": 0.6, "rigidity": 0.35, "gravity": 1.0 },
                "accessory": { "socket": "head", "scale": 1.2, "offsetX": 0.0, "offsetY": 0.1, "offsetZ": 0.0 }
            }
        });

        let project = ProjectState::from_value(legacy).expect("legacy payload must migrate");
        project.validate().expect("migrated project must validate");

        assert_eq!(project.character.base_gender, BaseGender::Female);
        assert!((project.character.gender_dimorphism - 0.37).abs() < 1e-6);
        assert_eq!(project.character.active_preset_id.as_deref(), Some("plus_size"));
        assert!((project.morph_value("head_width") - 1.3).abs() < 1e-6);
        assert!((project.morph_value("jaw_v_line_taper") - 0.8).abs() < 1e-6);
        assert_eq!(project.character.morph_values.len(), 2, "unknown slider is dropped");
        assert!((project.character.somatotype.endomorph - 0.5).abs() < 1e-6);
        assert!((project.character.hair.volume - 1.4).abs() < 1e-6);
        assert_eq!(project.character.hair.strands, 24);
        assert!((project.character.proportions.head_scale - 1.1).abs() < 1e-6);
        assert!((project.character.proportions.shoulder_width - 1.05).abs() < 1e-6);

        let camera = &project.scene.camera.camera;
        assert!((camera.eye.y - 1.7).abs() < 1e-6);
        assert!((camera.fov_y.to_degrees() - 50.0).abs() < 1e-3, "fov is stored in radians");

        let light = project.light(&LightId::canonical_key()).unwrap();
        assert!((light.intensity - 1.4).abs() < 1e-6);

        let material = project.character_material().unwrap();
        assert!((material.material.shadow_threshold - 0.45).abs() < 1e-6);
        assert!((material.material.hue_shift + 22.0).abs() < 1e-6);
        assert!((material.material.ao_intensity - 0.7).abs() < 1e-6);
        assert_eq!(material.name, "DefaultAnimeMaterial");

        // The untouched legacy payload survives verbatim.
        let preserved = project
            .extensions
            .get("legacy_ts_snapshot")
            .and_then(|value| value.as_object())
            .expect("legacy payload preserved");
        assert_eq!(preserved.get("headScale"), Some(&json!(1.1)));
        assert_eq!(
            project.extensions.get("migrated_from"),
            Some(&json!("legacy_ts_snapshot"))
        );

        // Migration is idempotent: migrating the canonical result is a no-op.
        let canonical = project.to_value().unwrap();
        let migrated_again = ProjectState::from_value(canonical).unwrap();
        assert_eq!(migrated_again, project);
    }

    #[test]
    fn to_scene_resolves_references_and_never_stores_meshes() {
        let mut project = canonical_project();
        let gender = project.character.base_gender;
        let scene = project.to_scene(&|reference| {
            if reference.asset_id == ProjectState::base_mesh_asset_id(gender) {
                Some(Mesh::create_canonical_base(gender))
            } else {
                None
            }
        });
        assert_eq!(scene.nodes.len(), 1);
        assert_eq!(scene.total_vertices(), 4070);
        assert_eq!(scene.total_triangles(), 6880);
        assert_eq!(scene.background_color, project.render.background_color);

        project.scene.nodes[0].visible = false;
        let hidden = project.to_scene(&|_| Some(Mesh::create_cube(1.0)));
        assert!(!hidden.nodes[0].visible, "node visibility is authoritative");
    }

    #[test]
    fn active_morphs_and_weights_follow_catalog_order() {
        let mut project = canonical_project();
        project.set_morph_value("jaw_v_line_taper", 0.8);
        project.set_morph_value("head_width", 1.2);
        let active = project.active_morph_values();
        let ids: Vec<&str> = active.iter().map(|(id, _)| *id).collect();
        let head = ids.iter().position(|id| *id == "head_width").unwrap();
        let jaw = ids.iter().position(|id| *id == "jaw_v_line_taper").unwrap();
        assert!(head < jaw, "catalog order is authoritative: {ids:?}");
    }

    #[test]
    fn slug_key_matches_legacy_camel_case() {
        assert_eq!(slug_key("headScale"), "head_scale");
        assert_eq!(slug_key("shoulderWidth"), "shoulder_width");
        assert_eq!(slug_key("heightOverall"), "height_overall");
        // `camel_key` é o inverso (é o que a migração do payload legado usa).
        for canonical in [
            "head_scale",
            "head_ratio",
            "shoulder_width",
            "leg_length",
            "arm_length",
            "neck_length",
            "torso_length",
            "height_overall",
        ] {
            assert_eq!(slug_key(&camel_key(canonical)), canonical);
        }
        assert_eq!(camel_key("height_overall"), "heightOverall");
    }

    #[test]
    fn parse_hex_rgba_handles_valid_and_invalid_input() {
        let fallback = [0.0, 0.0, 0.0, 1.0];
        let rgba = parse_hex_rgba("#ff8000", fallback);
        assert!((rgba[0] - 1.0).abs() < 1e-6);
        assert!((rgba[1] - 0.501_960_8).abs() < 1e-4);
        assert_eq!(parse_hex_rgba("nope", fallback), fallback);
        assert_eq!(parse_hex_rgba("#fff", fallback), fallback);
    }
}
