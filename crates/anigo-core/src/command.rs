//! ANIGO Transactional Command Layer (P0 — ARQUITETURA_CANONICA_ANIGO §2.5).
//!
//! Every persistent change to [`ProjectState`] happens through a [`Command`]:
//!
//! ```text
//! Command
//!  ├── validate(&ProjectState)      → rejects unknown targets / illegal values
//!  ├── apply_unchecked(&mut State)  → mutates a *draft* copy only
//!  ├── inverse(&ProjectState)       → command that restores the previous state
//!  ├── metadata()                   → description + change scope
//!  └── serialize()                  → JSON (shared contract with TypeScript)
//! ```
//!
//! `apply` is transactional: the draft is validated *before* it replaces the
//! live state, so a failing command can never leave the project half-updated.
//! `CommandHistory` is the single undo/redo authority — the UI keeps no history
//! of its own, it only displays `can_undo`/`can_redo` and the descriptions.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::hierarchy::NodeKind;
use crate::ids::{AssetId, LightId, MaterialId, MorphId, NodeId};
use crate::math::{Camera, Transform};
use crate::mesh::BaseGender;
use crate::morph_catalog::find_slider_def;
use crate::project::{
    MeshRef, NodeSlot, ProjectError, ProjectState, SceneState, URI_BASE_FEMALE, URI_BASE_MALE,
    URI_PRESET_CUBE, URI_PRESET_SPHERE,
};
use crate::scene::StylizedMaterial;
use crate::somatotype::SomatotypeCoords;

/// Monotonic revision counter of the project (bumped by every applied command).
pub type Revision = u64;

/// Patch do modo de projeção da câmera (issue #13).
///
/// Três campos opcionais porque as duas pontas têm necessidades diferentes: a UI
/// manda `orthographic` + `ortho_height` (o volume simétrico que preserva o
/// enquadramento) enquanto o **inverso** de um comando precisa restaurar o
/// volume exato, que é o que `ortho_bounds` carrega.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CameraProjectionPatch {
    /// `Some(true)` = ortográfica, `Some(false)` = perspectiva, `None` = mantém.
    pub orthographic: Option<bool>,
    /// Altura da silhueta (unidades de mundo): volume simétrico equivalente.
    pub ortho_height: Option<f32>,
    /// Volume ortográfico explícito — tem precedência sobre `ortho_height`.
    pub ortho_bounds: Option<crate::math::OrthographicBounds>,
}

impl CameraProjectionPatch {
    /// `true` quando o patch não muda nada.
    pub fn is_empty(&self) -> bool {
        self.orthographic.is_none() && self.ortho_height.is_none() && self.ortho_bounds.is_none()
    }
}

/// What a command changed — used to decide which caches must be rebuilt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeScope {
    /// Base geometry: mesh selection, base gender or macro proportions.
    /// Invalidates the *static* part of the viewport snapshot.
    BaseGeometry,
    /// Deformation weights (morph sliders, somatotype, dimorphism).
    /// Only the *dynamic* weights of the snapshot change.
    Deformation,
    /// Material/shading parameters.
    Shading,
    /// Camera parameters.
    Camera,
    /// Presentation-only state (visibility, background, render settings).
    Presentation,
    /// Project-level metadata (name, settings).
    Project,
}

impl ChangeScope {
    /// `true` when the command invalidates the static snapshot payload
    /// (base vertex buffer / sparse morph channel set).
    pub fn requires_static_rebuild(self) -> bool {
        matches!(self, ChangeScope::BaseGeometry)
    }

    /// `true` when the command can change what the viewport draws.
    pub fn is_visual(self) -> bool {
        !matches!(self, ChangeScope::Project)
    }
}

/// Mesh presets shipped with the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeshPreset {
    /// Canonical 4070-vertex anime mannequin (morph-capable).
    Mannequin,
    /// Unit cube (topology probe).
    Cube,
    /// UV sphere (lighting probe).
    Sphere,
}

impl MeshPreset {
    /// Canonical asset URI of the preset for a base gender.
    pub fn uri(self, gender: BaseGender) -> &'static str {
        match self {
            MeshPreset::Cube => URI_PRESET_CUBE,
            MeshPreset::Sphere => URI_PRESET_SPHERE,
            MeshPreset::Mannequin => match gender {
                BaseGender::Male => URI_BASE_MALE,
                BaseGender::Female => URI_BASE_FEMALE,
            },
        }
    }

    /// `true` when canonical sparse morph channels exist for this preset.
    pub fn supports_morphs(self) -> bool {
        matches!(self, MeshPreset::Mannequin)
    }
}

/// Partial update of the stylized material (anime NPR parameters).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MaterialPatch {
    pub base_color: Option<[f32; 4]>,
    pub shade_color: Option<[f32; 4]>,
    pub outline_color: Option<[f32; 4]>,
    pub outline_width: Option<f32>,
    pub shadow_threshold: Option<f32>,
    pub shadow_smoothness: Option<f32>,
    pub spec_intensity: Option<f32>,
    pub spec_power: Option<f32>,
    pub rim_intensity: Option<f32>,
    pub rim_spread: Option<f32>,
    pub hue_shift: Option<f32>,
    pub toon_steps: Option<f32>,
    pub specular_color: Option<[f32; 4]>,
    pub specular_softness: Option<f32>,
    pub specular_offset: Option<f32>,
    pub rim_color: Option<[f32; 4]>,
    pub outline_opacity: Option<f32>,
    pub outline_smoothness: Option<f32>,
    pub outline_depth_bias: Option<f32>,
    pub specular_size: Option<f32>,
    pub ao_intensity: Option<f32>,
}

impl MaterialPatch {
    /// Applies the patch to a material (only `Some` fields are written).
    pub fn apply_to(&self, material: &mut StylizedMaterial) {
        if let Some(value) = self.base_color {
            material.base_color = value;
        }
        if let Some(value) = self.shade_color {
            material.shade_color = value;
        }
        if let Some(value) = self.outline_color {
            material.outline_color = value;
        }
        if let Some(value) = self.outline_width {
            material.outline_width = value;
        }
        if let Some(value) = self.shadow_threshold {
            material.shadow_threshold = value;
        }
        if let Some(value) = self.shadow_smoothness {
            material.shadow_smoothness = value;
        }
        if let Some(value) = self.spec_intensity {
            material.spec_intensity = value;
        }
        if let Some(value) = self.spec_power {
            material.spec_power = value;
        }
        if let Some(value) = self.rim_intensity {
            material.rim_intensity = value;
        }
        if let Some(value) = self.rim_spread {
            material.rim_spread = value;
        }
        if let Some(value) = self.hue_shift {
            material.hue_shift = value;
        }
        if let Some(value) = self.toon_steps {
            material.toon_steps = value;
        }
        if let Some(value) = self.specular_color {
            material.specular_color = value;
        }
        if let Some(value) = self.specular_softness {
            material.specular_softness = value;
        }
        if let Some(value) = self.specular_offset {
            material.specular_offset = value;
        }
        if let Some(value) = self.rim_color {
            material.rim_color = value;
        }
        if let Some(value) = self.outline_opacity {
            material.outline_opacity = value;
        }
        if let Some(value) = self.outline_smoothness {
            material.outline_smoothness = value;
        }
        if let Some(value) = self.outline_depth_bias {
            material.outline_depth_bias = value;
        }
        if let Some(value) = self.specular_size {
            material.specular_size = value;
        }
        if let Some(value) = self.ao_intensity {
            material.ao_intensity = value;
        }
    }

    /// Builds the inverse patch from the current material values.
    pub fn inverse_of(&self, material: &StylizedMaterial) -> MaterialPatch {
        MaterialPatch {
            base_color: self.base_color.map(|_| material.base_color),
            shade_color: self.shade_color.map(|_| material.shade_color),
            outline_color: self.outline_color.map(|_| material.outline_color),
            outline_width: self.outline_width.map(|_| material.outline_width),
            shadow_threshold: self.shadow_threshold.map(|_| material.shadow_threshold),
            shadow_smoothness: self.shadow_smoothness.map(|_| material.shadow_smoothness),
            spec_intensity: self.spec_intensity.map(|_| material.spec_intensity),
            spec_power: self.spec_power.map(|_| material.spec_power),
            rim_intensity: self.rim_intensity.map(|_| material.rim_intensity),
            rim_spread: self.rim_spread.map(|_| material.rim_spread),
            hue_shift: self.hue_shift.map(|_| material.hue_shift),
            toon_steps: self.toon_steps.map(|_| material.toon_steps),
            specular_color: self.specular_color.map(|_| material.specular_color),
            specular_softness: self.specular_softness.map(|_| material.specular_softness),
            specular_offset: self.specular_offset.map(|_| material.specular_offset),
            rim_color: self.rim_color.map(|_| material.rim_color),
            outline_opacity: self.outline_opacity.map(|_| material.outline_opacity),
            outline_smoothness: self.outline_smoothness.map(|_| material.outline_smoothness),
            outline_depth_bias: self.outline_depth_bias.map(|_| material.outline_depth_bias),
            specular_size: self.specular_size.map(|_| material.specular_size),
            ao_intensity: self.ao_intensity.map(|_| material.ao_intensity),
        }
    }

    /// `true` when no field is set (a no-op patch is rejected).
    pub fn is_empty(&self) -> bool {
        *self == MaterialPatch::default()
    }
}

/// Failures produced while validating or applying a command.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CommandError {
    /// The command targets an entity that does not exist.
    #[error("unknown {kind} '{target}'")]
    UnknownTarget {
        kind: &'static str,
        target: String,
    },
    /// A value is outside its canonical domain.
    #[error("invalid value for '{field}': {detail}")]
    InvalidValue { field: String, detail: String },
    /// A value is not finite.
    #[error("value for '{field}' must be finite, found {value}")]
    NonFinite { field: String, value: f32 },
    /// The command was empty (nothing to do).
    #[error("command has no effect: {0}")]
    NoOp(String),
    /// Applying the command produced an invalid project (rolled back).
    #[error("command produced an invalid project: {0}")]
    InvalidResult(#[from] ProjectError),
    /// Undo was requested with an empty undo stack.
    #[error("nothing to undo")]
    NothingToUndo,
    /// Redo was requested with an empty redo stack.
    #[error("nothing to redo")]
    NothingToRedo,
    /// The history could not restore a valid project (state rolled back).
    #[error("history operation failed: {0}")]
    HistoryFailed(String),
}

/// A persistent, undoable operation on the project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Command {
    /// Sets one canonical morph slider value (catalog-clamped).
    SetMorphValue { target: MorphId, value: f32 },
    /// Clears every morph override, restoring catalog defaults.
    ResetMorphs,
    /// Switches the canonical base gender (male/female isomorphic bases).
    SetBaseGender { gender: BaseGender },
    /// Sets the normalized somatotype.
    SetSomatotype {
        endomorph: f32,
        mesomorph: f32,
        ectomorph: f32,
    },
    /// Sets the continuous gender dimorphism (`0.0` female … `1.0` male).
    SetGenderDimorphism { value: f32 },
    /// Patches macro proportions (bone-driven ratios).
    SetProportions {
        #[serde(default)]
        head_scale: Option<f32>,
        #[serde(default)]
        head_ratio: Option<f32>,
        #[serde(default)]
        shoulder_width: Option<f32>,
        #[serde(default)]
        leg_length: Option<f32>,
        #[serde(default)]
        arm_length: Option<f32>,
        #[serde(default)]
        neck_length: Option<f32>,
        #[serde(default)]
        torso_length: Option<f32>,
        #[serde(default)]
        height_overall: Option<f32>,
    },
    /// Sets camera placement (world space).
    SetCamera {
        #[serde(default)]
        eye: Option<[f32; 3]>,
        #[serde(default)]
        target: Option<[f32; 3]>,
        #[serde(default)]
        up: Option<[f32; 3]>,
        #[serde(default)]
        fov_degrees: Option<f32>,
        /// Issue #13: modo de projeção (perspectiva/ortográfica) e volume.
        #[serde(default)]
        projection: Option<CameraProjectionPatch>,
    },
    /// Orbits the camera around its target (radians).
    OrbitCamera { azimuth: f32, elevation: f32 },
    /// Dollies the camera (multiplicative factor).
    ZoomCamera { factor: f32 },
    /// Pans the camera in screen space.
    PanCamera { dx: f32, dy: f32 },
    /// Patches the key light (or an explicit light id).
    SetLight {
        #[serde(default)]
        light_id: Option<LightId>,
        #[serde(default)]
        direction: Option<[f32; 3]>,
        #[serde(default)]
        color: Option<[f32; 3]>,
        #[serde(default)]
        intensity: Option<f32>,
        #[serde(default)]
        shadow_color: Option<[f32; 3]>,
        #[serde(default)]
        ambient_intensity: Option<f32>,
        #[serde(default)]
        shadow_saturation: Option<f32>,
        #[serde(default)]
        ambient_sky: Option<[f32; 3]>,
        #[serde(default)]
        ambient_ground: Option<[f32; 3]>,
    },
    /// Patches a material (defaults to the character material).
    SetMaterialParams {
        #[serde(default)]
        material_id: Option<MaterialId>,
        patch: MaterialPatch,
    },
    /// Toggles node visibility.
    SetNodeVisibility { node_id: NodeId, visible: bool },
    /// Replaces the mesh referenced by a node.
    SetNodeMesh {
        node_id: NodeId,
        #[serde(default)]
        mesh: Option<MeshRef>,
    },
    /// Issue #12: cria um nó na cena (filho de `parent_id` ou nova raiz).
    ///
    /// A posição `index` é explícita para que o undo restaure o nó exatamente
    /// onde ele estava (ordem determinística faz parte do documento canônico).
    AddNode {
        node_id: NodeId,
        name: String,
        #[serde(default)]
        parent_id: Option<NodeId>,
        #[serde(default)]
        node_kind: NodeKind,
        #[serde(default)]
        transform: Transform,
        #[serde(default)]
        mesh: Option<MeshRef>,
        #[serde(default)]
        material_id: Option<MaterialId>,
        index: u32,
    },
    /// Issue #12: remove um nó e toda a sua subárvore.
    ///
    /// Falha se o nó não existir; o inverso recria a subárvore inteira (com os
    /// pais na ordem correta), então nada se perde no undo.
    RemoveNode { node_id: NodeId },
    /// Issue #12: reparenta um nó (ou o devolve à raiz com `parent_id: None`).
    ///
    /// Um parentesco que fecharia ciclo é recusado com
    /// [`CommandError::InvalidValue`] — a árvore nunca fica inconsistente.
    SetNodeParent {
        node_id: NodeId,
        #[serde(default)]
        parent_id: Option<NodeId>,
    },
    /// Issue #12: patcha a transformação **local** de um nó (o que a UI edita).
    /// Os filhos acompanham automaticamente pela composição pai × local.
    SetNodeTransform {
        node_id: NodeId,
        #[serde(default)]
        translation: Option<[f32; 3]>,
        #[serde(default)]
        rotation: Option<[f32; 4]>,
        #[serde(default)]
        scale: Option<[f32; 3]>,
    },
    /// Loads a preset mesh into the character node.
    LoadMeshPreset { preset: MeshPreset },
    /// Sets the render background color.
    SetBackgroundColor { color: [f32; 4] },
    /// Sets MSAA sample count / tonemap operator.
    SetRenderSettings {
        #[serde(default)]
        msaa_samples: Option<u32>,
        #[serde(default)]
        tonemap: Option<crate::project::TonemapOperator>,
    },
    /// Renames the project.
    RenameProject { name: String },
    /// Applies several commands atomically (single undo entry).
    Batch { commands: Vec<Command> },
}

/// Result of applying a command: what changed, how to undo it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppliedCommand {
    /// The command that was applied.
    pub command: Command,
    /// Command that restores the previous project state.
    pub inverse: Command,
    /// Human-readable description (used by the undo UI).
    pub description: String,
    /// What kind of data changed.
    pub scope: ChangeScope,
    /// Stable ids touched by the command (telemetry + tests).
    pub affected: Vec<String>,
}

/// Result of a history operation, returned to the caller (UI/MCP).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandOutcome {
    /// Monotonic sequence number of the applied command.
    pub sequence: u64,
    /// Project revision after the operation.
    pub revision: Revision,
    /// Revision of base geometry after the operation (static snapshot part).
    pub base_geometry_revision: Revision,
    /// Description of the applied command.
    pub description: String,
    /// Change scope of the applied command.
    pub scope: ChangeScope,
    /// Ids touched by the command.
    pub affected: Vec<String>,
    /// Whether undo is available after the operation.
    pub can_undo: bool,
    /// Whether redo is available after the operation.
    pub can_redo: bool,
    /// Undo stack depth after the operation.
    pub undo_depth: usize,
    /// Redo stack depth after the operation.
    pub redo_depth: usize,
}

impl Command {
    /// Human-readable description shown in the undo UI.
    pub fn description(&self) -> String {
        match self {
            Command::SetMorphValue { target, .. } => {
                format!("Morph {}", target.slider_id())
            }
            Command::ResetMorphs => "Reset morphs".to_string(),
            Command::SetBaseGender { gender } => match gender {
                BaseGender::Male => "Base gender: male".to_string(),
                BaseGender::Female => "Base gender: female".to_string(),
            },
            Command::SetSomatotype { .. } => "Somatotype".to_string(),
            Command::SetGenderDimorphism { .. } => "Gender dimorphism".to_string(),
            Command::SetProportions { .. } => "Proportions".to_string(),
            Command::SetCamera { .. } => "Camera".to_string(),
            Command::OrbitCamera { .. } => "Orbit".to_string(),
            Command::ZoomCamera { .. } => "Zoom".to_string(),
            Command::PanCamera { .. } => "Pan".to_string(),
            Command::SetLight { .. } => "Lighting".to_string(),
            Command::SetMaterialParams { .. } => "Material".to_string(),
            Command::SetNodeVisibility { .. } => "Visibility".to_string(),
            Command::SetNodeMesh { .. } => "Mesh".to_string(),
            Command::AddNode { node_id, .. } => format!("Add node {node_id}"),
            Command::RemoveNode { node_id } => format!("Remove node {node_id}"),
            Command::SetNodeParent { node_id, parent_id } => match parent_id {
                Some(parent) => format!("Parent {node_id} under {parent}"),
                None => format!("Unparent {node_id}"),
            },
            Command::SetNodeTransform { node_id, .. } => format!("Transform {node_id}"),
            Command::LoadMeshPreset { preset } => format!("Preset {preset:?}").to_lowercase(),
            Command::SetBackgroundColor { .. } => "Background".to_string(),
            Command::SetRenderSettings { .. } => "Render settings".to_string(),
            Command::RenameProject { .. } => "Rename project".to_string(),
            Command::Batch { commands } => {
                format!("Batch of {} commands", commands.len())
            }
        }
    }

    /// Change scope used to invalidate derived caches.
    pub fn scope(&self) -> ChangeScope {
        match self {
            Command::SetMorphValue { .. }
            | Command::ResetMorphs
            | Command::SetSomatotype { .. }
            | Command::SetGenderDimorphism { .. } => ChangeScope::Deformation,
            Command::SetBaseGender { .. }
            | Command::SetProportions { .. }
            | Command::SetNodeMesh { .. }
            | Command::LoadMeshPreset { .. } => ChangeScope::BaseGeometry,
            Command::SetMaterialParams { .. } => ChangeScope::Shading,
            Command::SetCamera { .. }
            | Command::OrbitCamera { .. }
            | Command::ZoomCamera { .. }
            | Command::PanCamera { .. } => ChangeScope::Camera,
            // Issue #12: topologia e transformação de nós são apresentação —
            // não invalidam a geometria base (a malha canônica é a mesma).
            Command::SetLight { .. }
            | Command::SetNodeVisibility { .. }
            | Command::SetBackgroundColor { .. }
            | Command::SetRenderSettings { .. }
            | Command::AddNode { .. }
            | Command::RemoveNode { .. }
            | Command::SetNodeParent { .. }
            | Command::SetNodeTransform { .. } => ChangeScope::Presentation,
            Command::RenameProject { .. } => ChangeScope::Project,
            Command::Batch { commands } => commands
                .iter()
                .map(Command::scope)
                .max_by_key(|scope| scope_priority(*scope))
                .unwrap_or(ChangeScope::Project),
        }
    }

    /// Stable ids affected by the command.
    pub fn affected(&self) -> Vec<String> {
        match self {
            Command::SetMorphValue { target, .. } => vec![target.to_string()],
            Command::ResetMorphs => vec!["mrf_*".to_string()],
            Command::SetBaseGender { .. } => vec!["chr_canonical".to_string()],
            Command::SetSomatotype { .. } | Command::SetGenderDimorphism { .. } => {
                vec!["chr_canonical".to_string()]
            }
            Command::SetProportions { .. } => vec!["chr_canonical".to_string()],
            Command::SetCamera { .. } | Command::OrbitCamera { .. } | Command::ZoomCamera { .. }
            | Command::PanCamera { .. } => vec!["cam_viewport".to_string()],
            Command::SetLight { light_id, .. } => vec![light_id
                .clone()
                .unwrap_or_else(LightId::canonical_key)
                .to_string()],
            Command::SetMaterialParams { material_id, .. } => vec![material_id
                .clone()
                .unwrap_or_else(MaterialId::canonical_default)
                .to_string()],
            Command::SetNodeVisibility { node_id, .. } | Command::SetNodeMesh { node_id, .. } => {
                vec![node_id.to_string()]
            }
            // Issue #12: a remoção de um nó leva a subárvore junto; o alvo
            // reportado é a raiz removida (o `ProjectState` é quem sabe listar
            // os descendentes, e ele já mudou quando o outcome é montado).
            Command::AddNode { node_id, .. } => vec![node_id.to_string()],
            Command::RemoveNode { node_id } => vec![node_id.to_string()],
            Command::SetNodeParent { node_id, parent_id } => match parent_id {
                Some(parent) => vec![node_id.to_string(), parent.to_string()],
                None => vec![node_id.to_string()],
            },
            Command::SetNodeTransform { node_id, .. } => vec![node_id.to_string()],
            Command::LoadMeshPreset { .. } => vec![NodeId::canonical_character().to_string()],
            Command::SetBackgroundColor { .. } | Command::SetRenderSettings { .. } => {
                vec!["rnd_main".to_string()]
            }
            Command::RenameProject { .. } => vec!["prj_*".to_string()],
            Command::Batch { commands } => {
                commands.iter().flat_map(Command::affected).collect()
            }
        }
    }

    /// Validates the command against the current project (no mutation).
    pub fn validate(&self, state: &ProjectState) -> Result<(), CommandError> {
        match self {
            Command::SetMorphValue { target, value } => {
                let slider = slider_def_or_err(target)?;
                require_finite("value", *value)?;
                if *value < slider.min || *value > slider.max {
                    return Err(CommandError::InvalidValue {
                        field: format!("mrf_{}", slider.id),
                        detail: format!("must be within [{}, {}], found {}", slider.min, slider.max, value),
                    });
                }
                // P0 undo/redo: a command that changes nothing must not enter the
                // history (a slider drag that returns to its previous value, a
                // panel that re-emits the current value, …).
                if (state.morph_value(slider.id) - *value).abs() <= 1e-6 {
                    return Err(CommandError::NoOp(format!(
                        "morph '{}' already has the value {value}",
                        slider.id
                    )));
                }
                Ok(())
            }
            Command::ResetMorphs => {
                if state.character.morph_values.is_empty() {
                    return Err(CommandError::NoOp("no morph overrides to reset".to_string()));
                }
                Ok(())
            }
            Command::SetBaseGender { .. } => Ok(()),
            Command::SetSomatotype {
                endomorph,
                mesomorph,
                ectomorph,
            } => {
                for (field, value) in [
                    ("endomorph", *endomorph),
                    ("mesomorph", *mesomorph),
                    ("ectomorph", *ectomorph),
                ] {
                    require_finite(field, value)?;
                    if value < 0.0 || value > 1.0 {
                        return Err(CommandError::InvalidValue {
                            field: format!("somatotype.{field}"),
                            detail: format!("must be within [0, 1], found {value}"),
                        });
                    }
                }
                if endomorph + mesomorph + ectomorph <= 1e-6 {
                    return Err(CommandError::InvalidValue {
                        field: "somatotype".to_string(),
                        detail: "components must not all be zero".to_string(),
                    });
                }
                Ok(())
            }
            Command::SetGenderDimorphism { value } => {
                require_finite("gender_dimorphism", *value)?;
                if !(0.0..=1.0).contains(value) {
                    return Err(CommandError::InvalidValue {
                        field: "gender_dimorphism".to_string(),
                        detail: format!("must be within [0, 1], found {value}"),
                    });
                }
                if (state.character.gender_dimorphism - *value).abs() <= 1e-6 {
                    return Err(CommandError::NoOp(format!(
                        "gender dimorphism is already {value}"
                    )));
                }
                Ok(())
            }
            Command::SetProportions {
                head_scale,
                head_ratio,
                shoulder_width,
                leg_length,
                arm_length,
                neck_length,
                torso_length,
                height_overall,
            } => {
                if [
                    head_scale,
                    head_ratio,
                    shoulder_width,
                    leg_length,
                    arm_length,
                    neck_length,
                    torso_length,
                    height_overall,
                ]
                .iter()
                .all(|field| field.is_none())
                {
                    return Err(CommandError::NoOp("empty proportions patch".to_string()));
                }
                Ok(())
            }
            Command::SetCamera {
                eye,
                target,
                up,
                fov_degrees,
                projection,
            } => {
                if eye.is_none()
                    && target.is_none()
                    && up.is_none()
                    && fov_degrees.is_none()
                    && projection.is_none()
                {
                    return Err(CommandError::NoOp("empty camera patch".to_string()));
                }
                if let Some(patch) = projection {
                    if patch.is_empty() {
                        return Err(CommandError::NoOp("empty projection patch".to_string()));
                    }
                    if let Some(bounds) = patch.ortho_bounds {
                        if !bounds.is_valid()
                            || ![bounds.left, bounds.right, bounds.bottom, bounds.top]
                                .iter()
                                .all(|value| value.is_finite())
                        {
                            return Err(CommandError::InvalidValue {
                                field: "projection.ortho_bounds".to_string(),
                                detail: "left < right and bottom < top, all finite".to_string(),
                            });
                        }
                    }
                    if let Some(height) = patch.ortho_height {
                        require_finite("projection.ortho_height", height)?;
                        if height <= 0.0 {
                            return Err(CommandError::InvalidValue {
                                field: "projection.ortho_height".to_string(),
                                detail: "must be greater than zero".to_string(),
                            });
                        }
                    }
                }
                Ok(())
            }
            Command::OrbitCamera { azimuth, elevation } => {
                require_finite("azimuth", *azimuth)?;
                require_finite("elevation", *elevation)
            }
            Command::ZoomCamera { factor } => {
                require_finite("factor", *factor)?;
                if *factor <= 0.0 {
                    return Err(CommandError::InvalidValue {
                        field: "factor".to_string(),
                        detail: "must be greater than 0".to_string(),
                    });
                }
                Ok(())
            }
            Command::PanCamera { dx, dy } => {
                require_finite("dx", *dx)?;
                require_finite("dy", *dy)
            }
            Command::SetLight {
                light_id,
                direction,
                color,
                intensity,
                shadow_color,
                ambient_intensity,
                shadow_saturation,
                ambient_sky,
                ambient_ground,
            } => {
                let id = light_id.clone().unwrap_or_else(LightId::canonical_key);
                if state.light(&id).is_none() && id != LightId::canonical_key() {
                    return Err(CommandError::UnknownTarget {
                        kind: "light",
                        target: id.to_string(),
                    });
                }
                if direction.is_none()
                    && color.is_none()
                    && intensity.is_none()
                    && shadow_color.is_none()
                    && ambient_intensity.is_none()
                    && shadow_saturation.is_none()
                    && ambient_sky.is_none()
                    && ambient_ground.is_none()
                {
                    return Err(CommandError::NoOp("empty light patch".to_string()));
                }
                Ok(())
            }
            Command::SetMaterialParams {
                material_id,
                patch,
            } => {
                let id = material_id
                    .clone()
                    .unwrap_or_else(MaterialId::canonical_default);
                if !state.materials.contains_key(&id) {
                    return Err(CommandError::UnknownTarget {
                        kind: "material",
                        target: id.to_string(),
                    });
                }
                if patch.is_empty() {
                    return Err(CommandError::NoOp("empty material patch".to_string()));
                }
                Ok(())
            }
            Command::SetNodeVisibility { node_id, .. } | Command::SetNodeMesh { node_id, .. } => {
                let node = state
                    .scene
                    .nodes
                    .iter()
                    .find(|node| &node.node_id == node_id)
                    .ok_or_else(|| CommandError::UnknownTarget {
                        kind: "scene node",
                        target: node_id.to_string(),
                    })?;
                if let Command::SetNodeVisibility { visible, .. } = self {
                    if node.visible == *visible {
                        return Err(CommandError::NoOp(format!(
                            "node '{node_id}' is already {}",
                            if *visible { "visible" } else { "hidden" }
                        )));
                    }
                }
                if let Command::SetNodeMesh { mesh, .. } = self {
                    if let Some(reference) = mesh {
                        // The registry is authoritative project data: a command
                        // *references* an asset, it never fabricates one. An
                        // entry created during `apply` could not be removed on
                        // undo, which would break the exactness of the history.
                        require_known_asset(state, &reference.asset_id)?;
                    }
                }
                Ok(())
            }
            // ── Issue #12: árvore de transformações ─────────────────────────
            Command::AddNode {
                node_id,
                name,
                parent_id,
                transform,
                mesh,
                material_id,
                ..
            } => {
                if state.scene.nodes.iter().any(|node| &node.node_id == node_id) {
                    return Err(CommandError::InvalidValue {
                        field: "node_id".to_string(),
                        detail: format!("node '{node_id}' already exists"),
                    });
                }
                if name.trim().is_empty() {
                    return Err(CommandError::InvalidValue {
                        field: "name".to_string(),
                        detail: "node name must not be empty".to_string(),
                    });
                }
                if let Some(parent) = parent_id {
                    if parent == node_id {
                        return Err(CommandError::InvalidValue {
                            field: "parent_id".to_string(),
                            detail: "a node cannot be its own parent".to_string(),
                        });
                    }
                    if state.scene.node(parent).is_none() {
                        return Err(CommandError::UnknownTarget {
                            kind: "scene node",
                            target: parent.to_string(),
                        });
                    }
                }
                if let Some(reference) = mesh {
                    require_known_asset(state, &reference.asset_id)?;
                }
                if let Some(material_id) = material_id {
                    if !state.materials.contains_key(material_id) {
                        return Err(CommandError::UnknownTarget {
                            kind: "material",
                            target: material_id.to_string(),
                        });
                    }
                }
                validate_transform("transform", *transform)?;
                Ok(())
            }
            Command::RemoveNode { node_id } => {
                find_node(state, node_id)?;
                Ok(())
            }
            Command::SetNodeParent { node_id, parent_id } => {
                let node = find_node(state, node_id)?;
                if let Some(parent_id) = parent_id {
                    if state.scene.node(parent_id).is_none() {
                        return Err(CommandError::UnknownTarget {
                            kind: "scene node",
                            target: parent_id.to_string(),
                        });
                    }
                    if parent_id == &node.node_id {
                        return Err(CommandError::InvalidValue {
                            field: "parent_id".to_string(),
                            detail: "a node cannot be its own parent".to_string(),
                        });
                    }
                    let subtree = crate::hierarchy::descendants_of(&state.scene.nodes, node_id);
                    if subtree.iter().any(|descendant| descendant == parent_id) {
                        return Err(CommandError::InvalidValue {
                            field: "parent_id".to_string(),
                            detail: format!(
                                "'{parent_id}' is a descendant of '{node_id}' (would create a cycle)"
                            ),
                        });
                    }
                }
                if node.parent_id == *parent_id {
                    let current = match parent_id {
                        Some(parent) => format!("'{parent}'"),
                        None => "the scene root".to_string(),
                    };
                    return Err(CommandError::NoOp(format!(
                        "node '{node_id}' is already parented to {current}"
                    )));
                }
                Ok(())
            }
            Command::SetNodeTransform {
                node_id,
                translation,
                rotation,
                scale,
            } => {
                let node = find_node(state, node_id)?;
                // Um patch vazio não é comando; um patch que não muda nada é NoOp.
                if translation.is_none() && rotation.is_none() && scale.is_none() {
                    return Err(CommandError::NoOp("empty transform patch".to_string()));
                }
                let mut transform = node.transform;
                let mut changed = false;
                if let Some(translation) = translation {
                    validate_vector("translation", *translation)?;
                    if transform.translation.to_array() != *translation {
                        transform.translation = glam::Vec3::from_array(*translation);
                        changed = true;
                    }
                }
                if let Some(rotation) = rotation {
                    validate_quaternion(*rotation)?;
                    let quat = glam::Quat::from_array(*rotation);
                    if transform.rotation != quat {
                        transform.rotation = quat;
                        changed = true;
                    }
                }
                if let Some(scale) = scale {
                    validate_vector("scale", *scale)?;
                    if transform.scale.to_array() != *scale {
                        transform.scale = glam::Vec3::from_array(*scale);
                        changed = true;
                    }
                }
                if !changed {
                    return Err(CommandError::NoOp(format!(
                        "transform of '{node_id}' already has those values"
                    )));
                }
                Ok(())
            }
            Command::LoadMeshPreset { preset } => {
                // Presets resolve to their canonical built-in asset, which every
                // well-formed project registers (`BUILTIN_MESH_URIS`).
                require_known_asset(
                    state,
                    &AssetId::for_uri(preset.uri(state.character.base_gender)),
                )
            }
            Command::SetBackgroundColor { color } => {
                for component in color {
                    require_finite("background_color", *component)?;
                    if *component < 0.0 || *component > 1.0 {
                        return Err(CommandError::InvalidValue {
                            field: "background_color".to_string(),
                            detail: format!("must be within [0, 1], found {component}"),
                        });
                    }
                }
                if state.render.background_color == *color {
                    return Err(CommandError::NoOp(
                        "background color is already set to that value".to_string(),
                    ));
                }
                Ok(())
            }
            Command::SetRenderSettings {
                msaa_samples,
                tonemap,
            } => {
                if let Some(samples) = msaa_samples {
                    if !matches!(samples, 1 | 2 | 4 | 8 | 16) {
                        return Err(CommandError::InvalidValue {
                            field: "msaa_samples".to_string(),
                            detail: format!("must be one of 1, 2, 4, 8, 16, found {samples}"),
                        });
                    }
                }
                if msaa_samples.is_none() && tonemap.is_none() {
                    return Err(CommandError::NoOp("empty render settings patch".to_string()));
                }
                Ok(())
            }
            Command::RenameProject { name } => {
                if name.trim().is_empty() {
                    return Err(CommandError::InvalidValue {
                        field: "name".to_string(),
                        detail: "must not be empty".to_string(),
                    });
                }
                if state.name == *name {
                    return Err(CommandError::NoOp(format!(
                        "project is already named '{name}'"
                    )));
                }
                Ok(())
            }
            Command::Batch { commands } => {
                if commands.is_empty() {
                    return Err(CommandError::NoOp("empty batch".to_string()));
                }
                // Um batch é atômico, então cada comando é validado contra o
                // estado **intermediário** (replay num rascunho), não contra o
                // estado inicial: é o que permite, por exemplo, recriar um pai e
                // o filho no mesmo lote — o undo de uma remoção de subárvore.
                let mut scratch = state.clone();
                for command in commands {
                    command.validate(&scratch)?;
                    command.apply_unchecked(&mut scratch)?;
                }
                Ok(())
            }
        }
    }

    /// Builds the command that undoes `self` given the *current* state.
    pub fn inverse(&self, state: &ProjectState) -> Result<Command, CommandError> {
        match self {
            Command::SetMorphValue { target, .. } => Ok(Command::SetMorphValue {
                target: target.clone(),
                value: state.morph_value(target.slider_id()),
            }),
            Command::ResetMorphs => {
                let mut commands = Vec::new();
                for (slider_id, value) in state.active_morph_values() {
                    commands.push(Command::SetMorphValue {
                        target: MorphId::for_slider(slider_id),
                        value,
                    });
                }
                Ok(Command::Batch { commands })
            }
            Command::SetBaseGender { .. } => Ok(Command::SetBaseGender {
                gender: state.character.base_gender,
            }),
            Command::SetSomatotype { .. } => {
                let somatotype = state.character.somatotype;
                Ok(Command::SetSomatotype {
                    endomorph: somatotype.endomorph,
                    mesomorph: somatotype.mesomorph,
                    ectomorph: somatotype.ectomorph,
                })
            }
            Command::SetGenderDimorphism { .. } => Ok(Command::SetGenderDimorphism {
                value: state.character.gender_dimorphism,
            }),
            Command::SetProportions {
                head_scale,
                head_ratio,
                shoulder_width,
                leg_length,
                arm_length,
                neck_length,
                torso_length,
                height_overall,
            } => {
                let current = state.character.proportions;
                Ok(Command::SetProportions {
                    head_scale: head_scale.map(|_| current.head_scale),
                    head_ratio: head_ratio.map(|_| current.head_ratio),
                    shoulder_width: shoulder_width.map(|_| current.shoulder_width),
                    leg_length: leg_length.map(|_| current.leg_length),
                    arm_length: arm_length.map(|_| current.arm_length),
                    neck_length: neck_length.map(|_| current.neck_length),
                    torso_length: torso_length.map(|_| current.torso_length),
                    height_overall: height_overall.map(|_| current.height_overall),
                })
            }
            Command::SetCamera {
                eye,
                target,
                up,
                fov_degrees,
                projection,
            } => {
                let camera = &state.scene.camera.camera;
                Ok(Command::SetCamera {
                    eye: eye.map(|_| camera.eye.to_array()),
                    target: target.map(|_| camera.target.to_array()),
                    up: up.map(|_| camera.up.to_array()),
                    fov_degrees: fov_degrees.map(|_| camera.fov_y.to_degrees()),
                    // O inverso restaura o volume **exato** (não só a altura):
                    // um undo não pode recentralizar um volume assimétrico.
                    projection: projection.map(|_| CameraProjectionPatch {
                        orthographic: Some(camera.orthographic.is_some()),
                        ortho_height: camera.orthographic.map(|bounds| bounds.height()),
                        ortho_bounds: camera.orthographic,
                    }),
                })
            }
            Command::OrbitCamera { azimuth, elevation } => Ok(Command::OrbitCamera {
                azimuth: -*azimuth,
                elevation: -*elevation,
            }),
            Command::ZoomCamera { factor } => Ok(Command::ZoomCamera {
                factor: 1.0 / *factor,
            }),
            Command::PanCamera { dx, dy } => Ok(Command::PanCamera {
                dx: -*dx,
                dy: -*dy,
            }),
            Command::SetLight { light_id, .. } => {
                let id = light_id.clone().unwrap_or_else(LightId::canonical_key);
                let light = state
                    .light(&id)
                    .cloned()
                    .unwrap_or_default();
                Ok(Command::SetLight {
                    light_id: Some(id),
                    direction: Some(light.direction),
                    color: Some(light.color),
                    intensity: Some(light.intensity),
                    shadow_color: Some(light.shadow_color),
                    ambient_intensity: Some(light.ambient_intensity),
                    shadow_saturation: Some(light.shadow_saturation),
                    ambient_sky: Some(light.ambient_sky),
                    ambient_ground: Some(light.ambient_ground),
                })
            }
            Command::SetMaterialParams {
                material_id,
                patch,
            } => {
                let id = material_id
                    .clone()
                    .unwrap_or_else(MaterialId::canonical_default);
                let current = state
                    .materials
                    .get(&id)
                    .map(|entry| entry.material.clone())
                    .ok_or_else(|| CommandError::UnknownTarget {
                        kind: "material",
                        target: id.to_string(),
                    })?;
                Ok(Command::SetMaterialParams {
                    material_id: Some(id),
                    patch: patch.inverse_of(&current),
                })
            }
            Command::SetNodeVisibility { node_id, .. } => {
                let node = find_node(state, node_id)?;
                Ok(Command::SetNodeVisibility {
                    node_id: node_id.clone(),
                    visible: node.visible,
                })
            }
            Command::SetNodeMesh { node_id, .. } => {
                let node = find_node(state, node_id)?;
                Ok(Command::SetNodeMesh {
                    node_id: node_id.clone(),
                    mesh: node.mesh.clone(),
                })
            }
            // ── Issue #12: árvore de transformações ─────────────────────────
            Command::AddNode { node_id, .. } => Ok(Command::RemoveNode {
                node_id: node_id.clone(),
            }),
            Command::RemoveNode { node_id } => {
                // O inverso da remoção recria a subárvore inteira. A ordem é a
                // das posições **originais** (ascendente), o que reproduz o
                // array exatamente; um nó cujo pai também foi removido entra
                // como raiz e recebe o vínculo no `SetNodeParent` do fim do
                // lote — referência para frente seria um estado inválido.
                find_node(state, node_id)?;
                let removed: std::collections::BTreeSet<NodeId> = std::iter::once(node_id.clone())
                    .chain(crate::hierarchy::descendants_of(&state.scene.nodes, node_id))
                    .collect();
                let mut adds: Vec<Command> = Vec::new();
                let mut links: Vec<Command> = Vec::new();
                for (index, slot) in state.scene.nodes.iter().enumerate() {
                    if !removed.contains(&slot.node_id) {
                        continue;
                    }
                    let parent_inside = slot
                        .parent_id
                        .as_ref()
                        .map(|parent| removed.contains(parent))
                        .unwrap_or(false);
                    adds.push(Command::AddNode {
                        node_id: slot.node_id.clone(),
                        name: slot.name.clone(),
                        parent_id: if parent_inside {
                            None
                        } else {
                            slot.parent_id.clone()
                        },
                        node_kind: slot.kind,
                        transform: slot.transform,
                        mesh: slot.mesh.clone(),
                        material_id: slot.material_id.clone(),
                        index: index as u32,
                    });
                    if parent_inside {
                        links.push(Command::SetNodeParent {
                            node_id: slot.node_id.clone(),
                            parent_id: slot.parent_id.clone(),
                        });
                    }
                }
                let mut commands = adds;
                commands.extend(links);
                if commands.len() == 1 {
                    Ok(commands.remove(0))
                } else {
                    Ok(Command::Batch { commands })
                }
            }
            Command::SetNodeParent { node_id, .. } => {
                let node = find_node(state, node_id)?;
                Ok(Command::SetNodeParent {
                    node_id: node_id.clone(),
                    parent_id: node.parent_id.clone(),
                })
            }
            Command::SetNodeTransform { node_id, .. } => {
                let node = find_node(state, node_id)?;
                Ok(Command::SetNodeTransform {
                    node_id: node_id.clone(),
                    translation: Some(node.transform.translation.to_array()),
                    rotation: Some(node.transform.rotation.to_array()),
                    scale: Some(node.transform.scale.to_array()),
                })
            }
            Command::LoadMeshPreset { preset } => {
                let node_id = NodeId::canonical_character();
                let node = find_node(state, &node_id)?;
                let mut commands = Vec::new();
                // The gender command may rewrite the node mesh when the node
                // still points at a canonical base (`SetBaseGender` keeps the
                // mesh and the archetype in sync), so it must run *before* the
                // explicit mesh restore below, which has the last word.
                commands.push(Command::SetBaseGender {
                    gender: state.character.base_gender,
                });
                if !preset.supports_morphs() {
                    // Loading a non-mannequin preset clears every morph override
                    // (those presets have no sparse morph channels), so the
                    // inverse has to put all of them back — otherwise undoing
                    // "load preset" would silently discard the user's sliders.
                    for (slider_id, value) in state.active_morph_values() {
                        commands.push(Command::SetMorphValue {
                            target: MorphId::for_slider(slider_id),
                            value,
                        });
                    }
                }
                commands.push(Command::SetNodeMesh {
                    node_id: node_id.clone(),
                    mesh: node.mesh.clone(),
                });
                Ok(Command::Batch { commands })
            }
            Command::SetBackgroundColor { .. } => Ok(Command::SetBackgroundColor {
                color: state.render.background_color,
            }),
            Command::SetRenderSettings {
                msaa_samples,
                tonemap,
            } => Ok(Command::SetRenderSettings {
                msaa_samples: msaa_samples.map(|_| state.render.msaa_samples),
                tonemap: tonemap.map(|_| state.render.tonemap),
            }),
            Command::RenameProject { .. } => Ok(Command::RenameProject {
                name: state.name.clone(),
            }),
            Command::Batch { commands } => {
                // Inverse of a batch is the reversed batch of inverses. The
                // intermediate states are replayed on a scratch copy so each
                // inverse sees the state it actually undoes.
                let mut scratch = state.clone();
                let mut inverses = Vec::with_capacity(commands.len());
                for command in commands {
                    inverses.push(command.inverse(&scratch)?);
                    command.apply_unchecked(&mut scratch)?;
                }
                inverses.reverse();
                Ok(Command::Batch { commands: inverses })
            }
        }
    }

    /// Applies the command to a project without validation (used on drafts).
    pub fn apply_unchecked(&self, state: &mut ProjectState) -> Result<(), CommandError> {
        match self {
            Command::SetMorphValue { target, value } => {
                state.set_morph_value(target.slider_id(), *value);
                Ok(())
            }
            Command::ResetMorphs => {
                state.character.morph_values.clear();
                Ok(())
            }
            Command::SetBaseGender { gender } => {
                state.character.base_gender = *gender;
                let node_id = NodeId::canonical_character();
                if let Some(node) = state.scene.nodes.iter_mut().find(|n| n.node_id == node_id) {
                    if node
                        .mesh
                        .as_ref()
                        .is_some_and(|mesh| mesh.asset_id == ProjectState::base_mesh_asset_id(BaseGender::Male)
                            || mesh.asset_id == ProjectState::base_mesh_asset_id(BaseGender::Female))
                    {
                        node.mesh = Some(MeshRef {
                            asset_id: ProjectState::base_mesh_asset_id(*gender),
                            primitive_index: 0,
                        });
                    }
                }
                // NOTE: `active_preset_id` is intentionally NOT mutated here. It is
                // a higher-level concept (which preset is loaded) that this
                // low-level geometry command cannot restore on undo, so writing
                // it would break the "undo reproduces the exact prior state"
                // invariant of the history. Loading a preset is the caller's
                // responsibility if they want the id updated.
                Ok(())
            }
            Command::SetSomatotype {
                endomorph,
                mesomorph,
                ectomorph,
            } => {
                state.character.somatotype =
                    SomatotypeCoords::new(*endomorph, *mesomorph, *ectomorph).normalized();
                Ok(())
            }
            Command::SetGenderDimorphism { value } => {
                state.character.gender_dimorphism = *value;
                Ok(())
            }
            Command::SetProportions {
                head_scale,
                head_ratio,
                shoulder_width,
                leg_length,
                arm_length,
                neck_length,
                torso_length,
                height_overall,
            } => {
                let proportions = &mut state.character.proportions;
                if let Some(value) = head_scale {
                    proportions.head_scale = *value;
                }
                if let Some(value) = head_ratio {
                    proportions.head_ratio = *value;
                }
                if let Some(value) = shoulder_width {
                    proportions.shoulder_width = *value;
                }
                if let Some(value) = leg_length {
                    proportions.leg_length = *value;
                }
                if let Some(value) = arm_length {
                    proportions.arm_length = *value;
                }
                if let Some(value) = neck_length {
                    proportions.neck_length = *value;
                }
                if let Some(value) = torso_length {
                    proportions.torso_length = *value;
                }
                if let Some(value) = height_overall {
                    proportions.height_overall = *value;
                }
                Ok(())
            }
            Command::SetCamera {
                eye,
                target,
                up,
                fov_degrees,
                projection,
            } => {
                let camera = &mut state.scene.camera.camera;
                if let Some(value) = eye {
                    camera.eye = glam::Vec3::from_array(*value);
                }
                if let Some(value) = target {
                    camera.target = glam::Vec3::from_array(*value);
                }
                if let Some(value) = up {
                    camera.up = glam::Vec3::from_array(*value);
                }
                if let Some(value) = fov_degrees {
                    camera.fov_y = value.to_radians();
                }
                if let Some(patch) = projection {
                    apply_camera_projection(camera, *patch);
                }
                Ok(())
            }
            Command::OrbitCamera { azimuth, elevation } => {
                state.scene.camera.camera.orbit(*azimuth, *elevation);
                Ok(())
            }
            Command::ZoomCamera { factor } => {
                state.scene.camera.camera.zoom(*factor);
                Ok(())
            }
            Command::PanCamera { dx, dy } => {
                state.scene.camera.camera.pan(*dx, *dy);
                Ok(())
            }
            Command::SetLight {
                light_id,
                direction,
                color,
                intensity,
                shadow_color,
                ambient_intensity,
                shadow_saturation,
                ambient_sky,
                ambient_ground,
            } => {
                let id = light_id.clone().unwrap_or_else(LightId::canonical_key);
                let light = state.key_light_mut_for(&id);
                if let Some(value) = direction {
                    light.direction = *value;
                }
                if let Some(value) = color {
                    light.color = *value;
                }
                if let Some(value) = intensity {
                    light.intensity = *value;
                }
                if let Some(value) = shadow_color {
                    light.shadow_color = *value;
                }
                if let Some(value) = ambient_intensity {
                    light.ambient_intensity = *value;
                }
                if let Some(value) = shadow_saturation {
                    light.shadow_saturation = *value;
                }
                if let Some(value) = ambient_sky {
                    light.ambient_sky = *value;
                }
                if let Some(value) = ambient_ground {
                    light.ambient_ground = *value;
                }
                Ok(())
            }
            Command::SetMaterialParams {
                material_id,
                patch,
            } => {
                let id = material_id
                    .clone()
                    .unwrap_or_else(MaterialId::canonical_default);
                let entry = state
                    .materials
                    .entry(id.clone())
                    .or_insert_with(|| crate::project::MaterialEntry {
                        material_id: id.clone(),
                        name: "DefaultAnimeMaterial".to_string(),
                        material: StylizedMaterial::default(),
                    });
                patch.apply_to(&mut entry.material);
                Ok(())
            }
            Command::SetNodeVisibility { node_id, visible } => {
                let node = find_node_mut(state, node_id)?;
                node.visible = *visible;
                Ok(())
            }
            Command::SetNodeMesh { node_id, mesh } => {
                // Pure node mutation: the asset registry is never written here.
                // Registering an asset under its own id but with a URI-derived
                // key would either invalidate the project (`asset_id` and the
                // map key must match) or leave an entry behind after undo.
                let node = find_node_mut(state, node_id)?;
                node.mesh = mesh.clone();
                Ok(())
            }
            // ── Issue #12: árvore de transformações ─────────────────────────
            Command::AddNode {
                node_id,
                name,
                parent_id,
                node_kind,
                transform,
                mesh,
                material_id,
                index,
            } => {
                let slot = NodeSlot {
                    node_id: node_id.clone(),
                    name: name.clone(),
                    transform: *transform,
                    mesh: mesh.clone(),
                    material_id: material_id.clone(),
                    visible: true,
                    parent_id: parent_id.clone(),
                    kind: *node_kind,
                };
                let position = (*index as usize).min(state.scene.nodes.len());
                state.scene.nodes.insert(position, slot);
                Ok(())
            }
            Command::RemoveNode { node_id } => {
                // Remove a subárvore inteira: um filho órfão seria inválido, e o
                // inverse recria tudo de uma vez.
                let doomed: Vec<NodeId> = std::iter::once(node_id.clone())
                    .chain(crate::hierarchy::descendants_of(&state.scene.nodes, node_id))
                    .collect();
                state
                    .scene
                    .nodes
                    .retain(|node| !doomed.contains(&node.node_id));
                Ok(())
            }
            Command::SetNodeParent { node_id, parent_id } => {
                let node = find_node_mut(state, node_id)?;
                node.parent_id = parent_id.clone();
                Ok(())
            }
            Command::SetNodeTransform {
                node_id,
                translation,
                rotation,
                scale,
            } => {
                let node = find_node_mut(state, node_id)?;
                if let Some(translation) = translation {
                    node.transform.translation = glam::Vec3::from_array(*translation);
                }
                if let Some(rotation) = rotation {
                    node.transform.rotation = glam::Quat::from_array(*rotation).normalize();
                }
                if let Some(scale) = scale {
                    node.transform.scale = glam::Vec3::from_array(*scale);
                }
                Ok(())
            }
            Command::LoadMeshPreset { preset } => {
                let node_id = NodeId::canonical_character();
                let gender = state.character.base_gender;
                let node = find_node_mut(state, &node_id)?;
                node.mesh = Some(MeshRef {
                    asset_id: crate::ids::AssetId::for_uri(preset.uri(gender)),
                    primitive_index: 0,
                });
                if !preset.supports_morphs() {
                    // Sparse morph channels exist only for the canonical base.
                    state.character.morph_values.clear();
                }
                // `active_preset_id` is intentionally left untouched: this
                // command's inverse (`SetNodeMesh` + `SetBaseGender`) cannot
                // restore it, so mutating it here would make undo asymmetric.
                Ok(())
            }
            Command::SetBackgroundColor { color } => {
                state.render.background_color = *color;
                Ok(())
            }
            Command::SetRenderSettings {
                msaa_samples,
                tonemap,
            } => {
                if let Some(samples) = msaa_samples {
                    state.render.msaa_samples = *samples;
                }
                if let Some(operator) = tonemap {
                    state.render.tonemap = *operator;
                }
                Ok(())
            }
            Command::RenameProject { name } => {
                state.name = name.clone();
                Ok(())
            }
            Command::Batch { commands } => {
                for command in commands {
                    command.apply_unchecked(state)?;
                }
                Ok(())
            }
        }
    }

    /// Transactional application: validate → mutate a draft → validate draft →
    /// swap. A failing command leaves `state` untouched.
    pub fn apply(&self, state: &mut ProjectState) -> Result<AppliedCommand, CommandError> {
        self.validate(state)?;
        let inverse = self.inverse(state)?;

        let mut draft = state.clone();
        self.apply_unchecked(&mut draft)?;
        draft.validate().map_err(CommandError::InvalidResult)?;
        *state = draft;

        Ok(AppliedCommand {
            command: self.clone(),
            inverse,
            description: self.description(),
            scope: self.scope(),
            affected: self.affected(),
        })
    }

    /// Serializes the command (shared contract with TypeScript).
    pub fn to_json(&self) -> Result<String, CommandError> {
        serde_json::to_string(self)
            .map_err(|e| CommandError::InvalidValue {
                field: "command".to_string(),
                detail: e.to_string(),
            })
    }

    /// Deserializes a command coming from the UI/MCP bridge.
    pub fn from_json(raw: &str) -> Result<Command, CommandError> {
        serde_json::from_str(raw).map_err(|e| CommandError::InvalidValue {
            field: "command".to_string(),
            detail: e.to_string(),
        })
    }
}

impl ProjectState {
    /// Mutable access to a light by id (inserting the key light when missing).
    fn key_light_mut_for(&mut self, id: &LightId) -> &mut crate::scene::StylizedLight {
        if self.scene.lights.iter().all(|slot| &slot.light_id != id) {
            if id == &LightId::canonical_key() {
                self.scene.lights.push(crate::project::LightSlot {
                    light_id: id.clone(),
                    light: crate::scene::StylizedLight::default(),
                });
            } else {
                // Unknown ids are rejected in `validate`; this path is only
                // reachable from sanitize/repair flows, so reuse the key light.
                return self.key_light_mut();
            }
        }
        let index = self
            .scene
            .lights
            .iter()
            .position(|slot| &slot.light_id == id)
            .unwrap_or(0);
        &mut self.scene.lights[index].light
    }
}

fn scope_priority(scope: ChangeScope) -> u8 {
    match scope {
        ChangeScope::BaseGeometry => 5,
        ChangeScope::Deformation => 4,
        ChangeScope::Shading => 3,
        ChangeScope::Camera => 2,
        ChangeScope::Presentation => 1,
        ChangeScope::Project => 0,
    }
}

fn require_finite(field: &str, value: f32) -> Result<(), CommandError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(CommandError::NonFinite {
            field: field.to_string(),
            value,
        })
    }
}

fn slider_def_or_err(
    target: &MorphId,
) -> Result<&'static crate::morph_catalog::MorphSliderDef, CommandError> {
    find_slider_def(target.slider_id()).ok_or_else(|| CommandError::UnknownTarget {
        kind: "morph slider",
        target: target.to_string(),
    })
}

/// Rejects a mesh reference to an asset that is not part of the project.
fn require_known_asset(state: &ProjectState, asset_id: &AssetId) -> Result<(), CommandError> {
    if state.assets.contains_key(asset_id) {
        Ok(())
    } else {
        Err(CommandError::UnknownTarget {
            kind: "asset",
            target: asset_id.to_string(),
        })
    }
}

/// Issue #13: aplica um patch de projeção à câmera.
///
/// Entrar em ortográfica **sem** informar volume usa a altura que reproduz o
/// enquadramento perspectiva na distância atual do alvo
/// (`2·d·tan(fov/2)`) — é o que faz a troca de modo não dar salto visual nem
/// deformar o modelo, a mesma conta do `camera_math.ts` no viewport.
fn apply_camera_projection(camera: &mut Camera, patch: CameraProjectionPatch) {
    use crate::math::{OrthographicBounds, ProjectionMode};

    if let Some(bounds) = patch.ortho_bounds {
        camera.set_projection_mode(ProjectionMode::from(bounds));
        return;
    }

    match patch.orthographic {
        Some(true) => {
            let height = patch
                .ortho_height
                .unwrap_or_else(|| framing_preserving_height(camera));
            camera.set_projection_mode(ProjectionMode::from(OrthographicBounds::from_height(
                height,
                camera.aspect,
            )));
        }
        Some(false) => {
            camera.set_projection_mode(ProjectionMode::Perspective {
                fov_y: camera.fov_y,
                aspect: camera.aspect,
            });
        }
        // Sem troca de modo, `ortho_height` só ajusta um volume já ortográfico.
        None => {
            if let Some(height) = patch.ortho_height {
                if camera.orthographic.is_some() {
                    camera.set_projection_mode(ProjectionMode::from(
                        OrthographicBounds::from_height(height, camera.aspect),
                    ));
                }
            }
        }
    }
}

/// Altura (unidades de mundo) que casa o enquadramento perspectiva na
/// distância atual do alvo: `2 · d · tan(fov_y / 2)`.
fn framing_preserving_height(camera: &Camera) -> f32 {
    let distance = (camera.eye - camera.target).length().max(0.01);
    (2.0 * distance * (camera.fov_y * 0.5).tan()).max(0.01)
}

/// Issue #12: um vetor de transformação só entra no documento se for finito.
fn validate_vector(field: &str, value: [f32; 3]) -> Result<(), CommandError> {
    for component in value {
        require_finite(field, component)?;
    }
    Ok(())
}

/// Issue #12: quaternion finito e não nulo (uma rotação válida).
fn validate_quaternion(value: [f32; 4]) -> Result<(), CommandError> {
    for component in value {
        require_finite("rotation", component)?;
    }
    let length_squared: f32 = value.iter().map(|component| component * component).sum();
    if length_squared < 1e-12 {
        return Err(CommandError::InvalidValue {
            field: "rotation".to_string(),
            detail: "quaternion must not be zero".to_string(),
        });
    }
    Ok(())
}

/// Issue #12: transformação local completa (translação, rotação e escala).
fn validate_transform(field: &str, transform: Transform) -> Result<(), CommandError> {
    validate_vector(&format!("{field}.translation"), transform.translation.to_array())?;
    validate_quaternion(transform.rotation.to_array())?;
    validate_vector(&format!("{field}.scale"), transform.scale.to_array())
}

fn find_node<'a>(state: &'a ProjectState, node_id: &NodeId) -> Result<&'a NodeSlot, CommandError> {
    state
        .scene
        .nodes
        .iter()
        .find(|node| &node.node_id == node_id)
        .ok_or_else(|| CommandError::UnknownTarget {
            kind: "scene node",
            target: node_id.to_string(),
        })
}

fn find_node_mut<'a>(
    state: &'a mut ProjectState,
    node_id: &NodeId,
) -> Result<&'a mut NodeSlot, CommandError> {
    state
        .scene
        .nodes
        .iter_mut()
        .find(|node| &node.node_id == node_id)
        .ok_or_else(|| CommandError::UnknownTarget {
            kind: "scene node",
            target: node_id.to_string(),
        })
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

/// Undo/redo authority. The UI never owns history; it only renders it.
#[derive(Debug, Clone)]
pub struct CommandHistory {
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    /// Accepted commands of the session, oldest first (§2.5 operation log).
    ///
    /// The undo stack is a *position* in the session; the log is the session
    /// itself. Undoing does not erase it, so a project saved after some undos
    /// still replays as the session that produced it. It is intentionally not
    /// trimmed by `limit` (that bound is the undo depth, not the log).
    log: Vec<CommandLogEntry>,
    limit: usize,
    sequence: u64,
    revision: Revision,
    base_geometry_revision: Revision,
}

/// One applied command in the history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Monotonic sequence number.
    pub sequence: u64,
    /// Revision produced by this entry.
    pub revision: Revision,
    /// Base geometry revision produced by this entry.
    pub base_geometry_revision: Revision,
    /// Description shown in the undo UI.
    pub description: String,
    /// Change scope.
    pub scope: ChangeScope,
    /// Command that undoes this entry.
    pub inverse: Command,
    /// Command that redoes this entry.
    pub command: Command,
}

impl Default for CommandHistory {
    fn default() -> Self {
        Self::new(crate::project::DEFAULT_HISTORY_LIMIT as usize)
    }
}

impl CommandHistory {
    /// Creates a history with an explicit undo depth.
    pub fn new(limit: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            log: Vec::new(),
            limit: limit.max(1),
            sequence: 0,
            revision: 0,
            base_geometry_revision: 0,
        }
    }

    /// Current project revision.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Current base-geometry revision (static snapshot payload).
    pub fn base_geometry_revision(&self) -> Revision {
        self.base_geometry_revision
    }

    /// `true` when a command can be undone.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// `true` when a command can be redone.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Undo depth.
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    /// Redo depth.
    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    /// Description of the next undoable command.
    pub fn last_undo_description(&self) -> Option<&str> {
        self.undo.last().map(|entry| entry.description.as_str())
    }

    /// Description of the next redoable command.
    pub fn last_redo_description(&self) -> Option<&str> {
        self.redo.last().map(|entry| entry.description.as_str())
    }

    /// Serializes the session log (deterministic; used by tests + autosave).
    pub fn log_json(&self) -> Result<String, CommandError> {
        serde_json::to_string_pretty(&self.log).map_err(|e| CommandError::HistoryFailed(e.to_string()))
    }

    /// Executes a command, pushing it onto the undo stack.
    ///
    /// The redo stack is cleared (standard DCC semantics). When the command or
    /// the resulting project is invalid, the state is left untouched.
    pub fn execute(
        &mut self,
        state: &mut ProjectState,
        command: Command,
    ) -> Result<CommandOutcome, CommandError> {
        let applied = command.apply(state)?;

        self.sequence += 1;
        self.revision += 1;
        if applied.scope.requires_static_rebuild() {
            self.base_geometry_revision += 1;
        }

        self.undo.push(HistoryEntry {
            sequence: self.sequence,
            revision: self.revision,
            base_geometry_revision: self.base_geometry_revision,
            description: applied.description.clone(),
            scope: applied.scope,
            inverse: applied.inverse.clone(),
            command: applied.command.clone(),
        });
        while self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        self.redo.clear();
        // Only *accepted* commands enter the operation log, and redo/undo never
        // do: a redone command is the same entry of the session, not a new one.
        self.log.push(CommandLogEntry {
            sequence: self.sequence,
            revision: self.revision,
            description: applied.description.clone(),
            scope: applied.scope,
            command: applied.command.clone(),
        });

        Ok(self.outcome(&applied))
    }

    /// Reverts the last applied command.
    pub fn undo(&mut self, state: &mut ProjectState) -> Result<CommandOutcome, CommandError> {
        let entry = self
            .undo
            .last()
            .cloned()
            .ok_or(CommandError::NothingToUndo)?;

        let applied = entry.inverse.apply(state)?;
        self.undo.pop();
        self.redo.push(entry.clone());
        self.revision += 1;
        if entry.scope.requires_static_rebuild() {
            self.base_geometry_revision += 1;
        }

        let mut outcome = self.outcome(&applied);
        outcome.description = format!("Undo {}", entry.description);
        outcome.scope = applied.scope;
        outcome.affected = applied.affected;
        Ok(outcome)
    }

    /// Re-applies the last undone command.
    pub fn redo(&mut self, state: &mut ProjectState) -> Result<CommandOutcome, CommandError> {
        let entry = self.redo.last().cloned().ok_or(CommandError::NothingToRedo)?;

        let applied = entry.command.apply(state)?;
        self.redo.pop();
        self.undo.push(entry.clone());
        self.revision += 1;
        if entry.scope.requires_static_rebuild() {
            self.base_geometry_revision += 1;
        }

        let mut outcome = self.outcome(&applied);
        outcome.description = format!("Redo {}", entry.description);
        outcome.scope = applied.scope;
        outcome.affected = applied.affected;
        Ok(outcome)
    }

    /// Clears both stacks and the session log (project load / new project).
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.log.clear();
        self.sequence = 0;
    }

    fn outcome(&self, applied: &AppliedCommand) -> CommandOutcome {
        CommandOutcome {
            sequence: self.sequence,
            revision: self.revision,
            base_geometry_revision: self.base_geometry_revision,
            description: applied.description.clone(),
            scope: applied.scope,
            affected: applied.affected.clone(),
            can_undo: self.can_undo(),
            can_redo: self.can_redo(),
            undo_depth: self.undo.len(),
            redo_depth: self.redo.len(),
        }
    }
}

// ---------------------------------------------------------------------------
// Persistent command log (P0 undo/redo item 3: only valid commands are kept)
// ---------------------------------------------------------------------------

/// Version of the persisted command-log format.
pub const COMMAND_LOG_VERSION: u32 = 1;

/// One command that was **accepted** by the history.
///
/// The log stores *commands*, not snapshots: replaying it on a base project
/// rebuilds the session, and because entries only enter the log after a
/// successful apply, a log is valid by construction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandLogEntry {
    /// Monotonic sequence number assigned by the history.
    pub sequence: u64,
    /// Project revision produced by this command.
    pub revision: Revision,
    /// Description shown in the undo UI.
    pub description: String,
    /// Change scope of the command.
    pub scope: ChangeScope,
    /// The command itself.
    pub command: Command,
}

/// Serializable command log (operation log of §2.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandLog {
    /// Wire format version of the log.
    pub version: u32,
    /// Accepted commands, oldest first.
    pub entries: Vec<CommandLogEntry>,
}

impl CommandLog {
    /// Empty log at the current format version.
    pub fn new() -> Self {
        Self {
            version: COMMAND_LOG_VERSION,
            entries: Vec::new(),
        }
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` when no command was recorded.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Serializes the log for autosave.
    pub fn to_json(&self) -> Result<String, CommandError> {
        serde_json::to_string_pretty(self).map_err(|e| CommandError::HistoryFailed(e.to_string()))
    }

    /// Parses a persisted log, refusing unknown versions.
    pub fn from_json(raw: &str) -> Result<Self, CommandError> {
        let log: Self = serde_json::from_str(raw)
            .map_err(|e| CommandError::HistoryFailed(format!("invalid command log: {e}")))?;
        if log.version > COMMAND_LOG_VERSION {
            return Err(CommandError::HistoryFailed(format!(
                "command log version {} is newer than the supported {}",
                log.version, COMMAND_LOG_VERSION
            )));
        }
        for (index, entry) in log.entries.iter().enumerate() {
            if entry.sequence == 0 {
                return Err(CommandError::HistoryFailed(format!(
                    "command log entry {index} has no sequence number"
                )));
            }
        }
        Ok(log)
    }

    /// The command of every entry, in order.
    pub fn commands(&self) -> Vec<Command> {
        self.entries.iter().map(|entry| entry.command.clone()).collect()
    }
}

impl Default for CommandLog {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHistory {
    /// Exports the accepted commands as a persisted log.
    ///
    /// Only *valid* commands can be here: an entry is pushed after a successful
    /// apply and never for a rejected command, and undoing does not remove it
    /// (the log is the session, not the undo stack).
    pub fn export_log(&self) -> CommandLog {
        CommandLog {
            version: COMMAND_LOG_VERSION,
            entries: self.log.clone(),
        }
    }
}

/// Result of replaying a persisted command log.
#[derive(Debug, Clone)]
pub struct ReplayedLog {
    /// Project after every accepted command was applied.
    pub state: ProjectState,
    /// History rebuilt from the replayed commands (undo/redo work on it).
    pub history: CommandHistory,
    /// Outcome of each replayed command, in order.
    pub outcomes: Vec<CommandOutcome>,
}

/// Replays a persisted log onto `base`, validating every entry.
///
/// A log written by this module is valid by construction, so a failure means
/// the file was edited or produced by a different build: the error names the
/// offending index, and the caller can keep the prefix (`index` entries) that
/// was already applied — the recovery path required by §3.1.
pub fn restore_from_log(
    base: &ProjectState,
    log: &CommandLog,
    limit: usize,
) -> Result<ReplayedLog, (usize, CommandError)> {
    let mut state = base.clone();
    let mut history = CommandHistory::new(limit);
    let mut outcomes = Vec::with_capacity(log.entries.len());

    for (index, entry) in log.entries.iter().enumerate() {
        match history.execute(&mut state, entry.command.clone()) {
            Ok(outcome) => outcomes.push(outcome),
            Err(error) => return Err((index, error)),
        }
    }

    Ok(ReplayedLog {
        state,
        history,
        outcomes,
    })
}

/// Convenience helper used by the Tauri layer and tests.
pub fn execute_command(
    state: &mut ProjectState,
    history: &mut CommandHistory,
    command: Command,
) -> Result<CommandOutcome, CommandError> {
    history.execute(state, command)
}

/// Rebuilds a scene mirroring the project structure (used by tests).
pub fn scene_of(state: &ProjectState) -> SceneState {
    state.scene.clone()
}

/// Convenience: number of morph overrides stored in the project.
pub fn morph_override_count(state: &ProjectState) -> usize {
    state.character.morph_values.len()
}

/// Convenience: morph overrides as a plain map (contract tests).
pub fn morph_overrides(state: &ProjectState) -> BTreeMap<String, f32> {
    state
        .character
        .morph_values
        .iter()
        .map(|(id, value)| (id.to_string(), *value))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::AssetId;
    use crate::project::ProjectState;

    fn project() -> ProjectState {
        ProjectState::default()
    }

    /// Camera commands are spherical reconstructions (`Camera::orbit`
    /// re-derives the eye from clamped angles/radius), so their inverse is
    /// exact only up to floating point + clamp tolerance.
    fn assert_camera_close(actual: &ProjectState, expected: &ProjectState, tolerance: f32) {
        let a = &actual.scene.camera.camera;
        let b = &expected.scene.camera.camera;
        assert!(
            (a.eye - b.eye).length() < tolerance,
            "camera eye drifted: {} vs {}",
            a.eye,
            b.eye
        );
        assert!((a.target - b.target).length() < tolerance);
        let mut normalized = actual.clone();
        normalized.scene.camera = expected.scene.camera.clone();
        assert_eq!(
            normalized, *expected,
            "camera undo must not touch any other field"
        );
    }

    #[test]
    fn apply_undo_redo_is_symmetric_for_morph_values() {
        let mut state = project();
        let mut history = CommandHistory::new(64);
        let before = state.clone();

        let outcome = history
            .execute(
                &mut state,
                Command::SetMorphValue {
                    target: MorphId::for_slider("head_width"),
                    value: 1.2,
                },
            )
            .expect("command must apply");

        assert_eq!(outcome.scope, ChangeScope::Deformation);
        assert!(!outcome.scope.requires_static_rebuild());
        assert_eq!(outcome.revision, 1);
        assert_eq!(outcome.base_geometry_revision, 0);
        assert!((state.morph_value("head_width") - 1.2).abs() < 1e-6);

        let undone = history.undo(&mut state).expect("undo must succeed");
        assert_eq!(state, before, "undo restores the exact previous state");
        assert_eq!(undone.revision, 2);
        assert!(undone.can_redo);

        let redone = history.redo(&mut state).expect("redo must succeed");
        assert!((state.morph_value("head_width") - 1.2).abs() < 1e-6);
        assert_eq!(redone.revision, 3);
        assert!(redone.can_undo);
    }

    #[test]
    fn every_persistent_command_kind_round_trips_apply_undo_redo() {
        let commands = vec![
            Command::SetMorphValue {
                target: MorphId::for_slider("jaw_v_line_taper"),
                value: 0.9,
            },
            Command::SetSomatotype {
                endomorph: 0.5,
                mesomorph: 0.3,
                ectomorph: 0.2,
            },
            Command::SetGenderDimorphism { value: 0.25 },
            Command::SetProportions {
                head_scale: Some(1.15),
                head_ratio: None,
                shoulder_width: Some(1.08),
                leg_length: None,
                arm_length: None,
                neck_length: None,
                torso_length: None,
                height_overall: None,
            },
            Command::SetCamera {
                eye: Some([0.4, 1.6, 3.2]),
                target: None,
                up: None,
                fov_degrees: Some(50.0),
                projection: None,
            },
            Command::OrbitCamera {
                azimuth: 0.2,
                elevation: -0.15,
            },
            Command::ZoomCamera { factor: 1.1 },
            Command::PanCamera { dx: 4.0, dy: -2.0 },
            Command::SetLight {
                light_id: None,
                direction: Some([0.2, 0.9, 0.3]),
                color: None,
                intensity: Some(1.8),
                shadow_color: None,
                ambient_intensity: Some(0.2),
                shadow_saturation: None,
                ambient_sky: None,
                ambient_ground: None,
            },
            Command::SetMaterialParams {
                material_id: None,
                patch: MaterialPatch {
                    shadow_threshold: Some(0.62),
                    outline_width: Some(0.004),
                    ..MaterialPatch::default()
                },
            },
            Command::SetNodeVisibility {
                node_id: NodeId::canonical_character(),
                visible: false,
            },
            Command::SetNodeMesh {
                node_id: NodeId::canonical_character(),
                mesh: Some(MeshRef {
                    asset_id: AssetId::for_uri(URI_PRESET_CUBE),
                    primitive_index: 0,
                }),
            },
            Command::LoadMeshPreset {
                preset: MeshPreset::Sphere,
            },
            Command::SetBackgroundColor {
                color: [0.2, 0.1, 0.3, 1.0],
            },
            Command::SetRenderSettings {
                msaa_samples: Some(8),
                tonemap: Some(crate::project::TonemapOperator::Neutral),
            },
            // Issue #12: a árvore também passa pelo mesmo contrato de involution
            // (aplicar, desfazer e refazer sem deixar resíduo). O reparent e a
            // remoção de subárvore precisam de mais de um nó (e a cena canônica
            // não aceita ficar vazia), então vivem no teste dedicado
            // `node_tree_commands_round_trip_with_world_transforms`.
            Command::AddNode {
                node_id: NodeId::from_slug("nod_stage"),
                name: "Stage".to_string(),
                parent_id: None,
                node_kind: crate::hierarchy::NodeKind::Group,
                transform: Transform::default(),
                mesh: None,
                material_id: None,
                index: 1,
            },
            Command::SetNodeTransform {
                node_id: NodeId::canonical_character(),
                translation: Some([0.5, 0.25, -0.5]),
                rotation: Some([0.0, 0.0, 0.0, 1.0]),
                scale: Some([1.0, 1.0, 1.0]),
            },
            Command::RenameProject {
                name: "Projeto de Teste".to_string(),
            },
            // Regression guard for the undo-involution fix: switching the base
            // gender must round-trip exactly. Its inverse is `SetBaseGender`
            // with the previous gender, which now leaves `active_preset_id`
            // untouched — so undo reproduces the prior state byte for byte.
            Command::SetBaseGender {
                gender: BaseGender::Female,
            },
        ];

        for command in commands {
            let mut state = project();
            let mut history = CommandHistory::new(64);
            let before = state.clone();
            let camera_command = matches!(
                command,
                Command::OrbitCamera { .. } | Command::ZoomCamera { .. } | Command::PanCamera { .. }
            );

            history
                .execute(&mut state, command.clone())
                .unwrap_or_else(|e| panic!("apply failed for {command:?}: {e}"));
            state
                .validate()
                .unwrap_or_else(|e| panic!("invalid state after {command:?}: {e}"));

            history
                .undo(&mut state)
                .unwrap_or_else(|e| panic!("undo failed for {command:?}: {e}"));
            if camera_command {
                assert_camera_close(&state, &before, 1e-3);
            } else {
                assert_eq!(
                    state, before,
                    "undo of {command:?} must restore the previous project"
                );
            }

            history
                .redo(&mut state)
                .unwrap_or_else(|e| panic!("redo failed for {command:?}: {e}"));
            history
                .undo(&mut state)
                .expect("undo after redo must succeed");
            if camera_command {
                assert_camera_close(&state, &before, 1e-3);
            } else {
                assert_eq!(state, before, "apply → undo → redo → undo is symmetric");
            }
        }
    }

    #[test]
    fn node_tree_commands_round_trip_with_world_transforms() {
        // Issue #12 — a árvore é editada só por comandos, e o undo devolve
        // topologia + transformações exatamente como estavam.
        let mut state = project();
        let mut history = CommandHistory::new(64);
        let root = NodeId::canonical_character();
        let jacket = NodeId::from_slug("nod_jacket");
        let hood = NodeId::from_slug("nod_hood");

        history
            .execute(
                &mut state,
                Command::AddNode {
                    node_id: jacket.clone(),
                    name: "Jacket".to_string(),
                    parent_id: Some(root.clone()),
                    node_kind: crate::hierarchy::NodeKind::Clothing,
                    transform: crate::math::Transform::default(),
                    mesh: None,
                    material_id: None,
                    index: 1,
                },
            )
            .expect("add node");
        history
            .execute(
                &mut state,
                Command::AddNode {
                    node_id: hood.clone(),
                    name: "Hood".to_string(),
                    parent_id: Some(jacket.clone()),
                    node_kind: crate::hierarchy::NodeKind::Hair,
                    transform: crate::math::Transform::default(),
                    mesh: None,
                    material_id: None,
                    index: 2,
                },
            )
            .expect("add child node");

        assert_eq!(
            state
                .scene
                .children_of(Some(&root))
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>(),
            vec![jacket.clone()]
        );
        assert_eq!(state.scene.depth_of(&hood), Some(2));

        // Mover o tronco move a peça pendurada nele — sem tocar no filho.
        history
            .execute(
                &mut state,
                Command::SetNodeTransform {
                    node_id: root.clone(),
                    translation: Some([2.0, 0.0, 0.0]),
                    rotation: None,
                    scale: None,
                },
            )
            .expect("move root");
        let world = crate::hierarchy::resolve_world_transforms(&state.scene.nodes).expect("world");
        let hood_world = world.get(&hood).expect("hood matrix");
        assert!((hood_world.transform_point3(glam::Vec3::ZERO).x - 2.0).abs() < 1e-5);
        assert!(state.scene.node(&hood).expect("hood").transform.translation.x.abs() < 1e-6);

        // Reparentar para um descendente é recusado (ciclo) e nada muda.
        let before = state.clone();
        let error = history
            .execute(
                &mut state,
                Command::SetNodeParent {
                    node_id: jacket.clone(),
                    parent_id: Some(hood.clone()),
                },
            )
            .expect_err("cycle must be rejected");
        assert!(matches!(error, CommandError::InvalidValue { .. }));
        assert_eq!(state, before, "a rejected command leaves the project untouched");

        // Reparentar a raiz para a jaqueta também fecharia ciclo.
        assert!(history
            .execute(
                &mut state,
                Command::SetNodeParent {
                    node_id: root.clone(),
                    parent_id: Some(jacket.clone()),
                },
            )
            .is_err());

        // NoOp: repetir o mesmo pai é recusado.
        assert!(matches!(
            history.execute(
                &mut state,
                Command::SetNodeParent {
                    node_id: hood.clone(),
                    parent_id: Some(jacket.clone()),
                },
            ),
            Err(CommandError::NoOp(_))
        ));

        // Uma segunda raiz mantém a cena não-vazia: o projeto canônico não
        // aceita remover o último nó (`ProjectError::EmptyScene`).
        history
            .execute(
                &mut state,
                Command::AddNode {
                    node_id: NodeId::from_slug("nod_stage"),
                    name: "Stage".to_string(),
                    parent_id: None,
                    node_kind: crate::hierarchy::NodeKind::Group,
                    transform: crate::math::Transform::default(),
                    mesh: None,
                    material_id: None,
                    index: 3,
                },
            )
            .expect("add second root");

        // Remover a jaqueta leva a subárvore inteira (o capuz) junto...
        let snapshot = state.clone();
        history
            .execute(
                &mut state,
                Command::RemoveNode {
                    node_id: jacket.clone(),
                },
            )
            .expect("remove subtree");
        assert_eq!(
            state
                .scene
                .nodes
                .iter()
                .map(|node| node.node_id.to_string())
                .collect::<Vec<_>>(),
            vec![root.to_string(), "nod_stage".to_string()],
            "children must never survive their parent"
        );

        // ...e o undo recria todos os nós com os vínculos e as transformações.
        history.undo(&mut state).expect("undo restores the subtree");
        assert_eq!(state, snapshot, "undo restores topology and transforms exactly");
        assert_eq!(
            state
                .scene
                .children_of(Some(&root))
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>(),
            vec![jacket.clone()]
        );
        assert_eq!(
            state.scene.node(&hood).expect("hood").parent_id.as_ref(),
            Some(&jacket)
        );
    }

    #[test]
    fn camera_projection_switch_preserves_the_framing_and_round_trips() {
        // Issue #13: entrar em ortográfica sem informar volume preserva o
        // enquadramento perspectiva na distância do alvo, e o undo devolve o
        // volume exato (não um volume recentralizado).
        let mut state = project();
        let mut history = CommandHistory::new(8);
        let before = state.clone();

        let perspective = state.scene.camera.camera.build_view_projection_matrix();
        let sample = state.scene.camera.camera.target
            + glam::Vec3::new(0.2, -0.1, 0.0);
        let ndc_before = perspective.project_point3(sample);

        history
            .execute(
                &mut state,
                Command::SetCamera {
                    eye: None,
                    target: None,
                    up: None,
                    fov_degrees: None,
                    projection: Some(CameraProjectionPatch {
                        orthographic: Some(true),
                        ..CameraProjectionPatch::default()
                    }),
                },
            )
            .expect("switch to orthographic");
        let camera = &state.scene.camera.camera;
        assert!(camera.projection_mode().is_orthographic());
        let bounds = camera.orthographic.expect("volume ortográfico");
        // O volume casa o enquadramento: metade da altura visível a `d` é
        // `d · tan(fov/2)`.
        let distance = (camera.eye - camera.target).length();
        let expected_half_height = distance * (camera.fov_y * 0.5).tan();
        assert!((bounds.height() * 0.5 - expected_half_height).abs() < 1e-4);
        assert!((bounds.width() / bounds.height() - camera.aspect).abs() < 1e-5);

        // O ponto de referência projeta praticamente no mesmo lugar.
        let ndc_after = camera.build_view_projection_matrix().project_point3(sample);
        assert!((ndc_before.x - ndc_after.x).abs() < 1e-3);
        assert!((ndc_before.y - ndc_after.y).abs() < 1e-3);

        history.undo(&mut state).expect("undo restores the projection");
        assert_eq!(state, before, "undo volta à perspectiva exatamente");

        // Sem troca de modo e sem altura, o patch é vazio (NoOp).
        assert!(matches!(
            history.execute(
                &mut state,
                Command::SetCamera {
                    eye: None,
                    target: None,
                    up: None,
                    fov_degrees: None,
                    projection: Some(CameraProjectionPatch::default()),
                },
            ),
            Err(CommandError::NoOp(_))
        ));

        // Altura inválida é recusada antes de tocar no estado.
        assert!(matches!(
            history.execute(
                &mut state,
                Command::SetCamera {
                    eye: None,
                    target: None,
                    up: None,
                    fov_degrees: None,
                    projection: Some(CameraProjectionPatch {
                        orthographic: Some(true),
                        ortho_height: Some(0.0),
                        ortho_bounds: None,
                    }),
                },
            ),
            Err(CommandError::InvalidValue { .. })
        ));

        // Volume explícito assimétrico é preservado pelo inverso.
        let bounds = crate::math::OrthographicBounds {
            left: -1.5,
            right: 2.5,
            bottom: -0.75,
            top: 1.25,
        };
        history
            .execute(
                &mut state,
                Command::SetCamera {
                    eye: None,
                    target: None,
                    up: None,
                    fov_degrees: None,
                    projection: Some(CameraProjectionPatch {
                        orthographic: Some(true),
                        ortho_height: None,
                        ortho_bounds: Some(bounds),
                    }),
                },
            )
            .expect("explicit volume");
        assert_eq!(state.scene.camera.camera.orthographic, Some(bounds));
        history.undo(&mut state).expect("undo");
        assert!(state.scene.camera.camera.orthographic.is_none());
    }

    #[test]
    fn add_node_rejects_duplicates_and_unknown_parents() {
        let mut state = project();
        let mut history = CommandHistory::new(8);
        let root = NodeId::canonical_character();

        assert!(matches!(
            history.execute(
                &mut state,
                Command::AddNode {
                    node_id: root.clone(),
                    name: "Duplicated".to_string(),
                    parent_id: None,
                    node_kind: crate::hierarchy::NodeKind::Group,
                    transform: crate::math::Transform::default(),
                    mesh: None,
                    material_id: None,
                    index: 0,
                },
            ),
            Err(CommandError::InvalidValue { .. })
        ));

        let orphan = history.execute(
            &mut state,
            Command::AddNode {
                node_id: NodeId::from_slug("nod_orphan"),
                name: "Orphan".to_string(),
                parent_id: Some(NodeId::from_slug("nod_missing")),
                node_kind: crate::hierarchy::NodeKind::Accessory,
                transform: crate::math::Transform::default(),
                mesh: None,
                material_id: None,
                index: 1,
            },
        );
        assert!(matches!(orphan, Err(CommandError::UnknownTarget { .. })));

        // Transformação não finita e quaternion nulo também são recusados.
        assert!(matches!(
            history.execute(
                &mut state,
                Command::SetNodeTransform {
                    node_id: root.clone(),
                    translation: Some([f32::NAN, 0.0, 0.0]),
                    rotation: None,
                    scale: None,
                },
            ),
            Err(CommandError::NonFinite { .. })
        ));
        assert!(matches!(
            history.execute(
                &mut state,
                Command::SetNodeTransform {
                    node_id: root.clone(),
                    translation: None,
                    rotation: Some([0.0, 0.0, 0.0, 0.0]),
                    scale: None,
                },
            ),
            Err(CommandError::InvalidValue { .. })
        ));
        assert!(matches!(
            history.execute(
                &mut state,
                Command::SetNodeTransform {
                    node_id: root,
                    translation: None,
                    rotation: None,
                    scale: None,
                },
            ),
            Err(CommandError::NoOp(_))
        ));
    }

    #[test]
    fn reset_morphs_undoes_every_override() {
        let mut state = project();
        let mut history = CommandHistory::new(64);
        history
            .execute(
                &mut state,
                Command::Batch {
                    commands: vec![
                        Command::SetMorphValue {
                            target: MorphId::for_slider("head_width"),
                            value: 1.3,
                        },
                        Command::SetMorphValue {
                            target: MorphId::for_slider("bust_volume_cup"),
                            value: 0.8,
                        },
                    ],
                },
            )
            .expect("batch must apply");
        assert_eq!(morph_override_count(&state), 2);
        let with_overrides = state.clone();

        history
            .execute(&mut state, Command::ResetMorphs)
            .expect("reset must apply");
        assert_eq!(morph_override_count(&state), 0);

        history.undo(&mut state).expect("undo of reset must succeed");
        assert_eq!(state, with_overrides, "every override is restored");
    }

    #[test]
    fn failing_command_leaves_the_project_untouched() {
        let mut state = project();
        let mut history = CommandHistory::new(64);
        let before = state.clone();

        let err = history
            .execute(
                &mut state,
                Command::SetMorphValue {
                    target: MorphId::for_slider("head_width"),
                    value: 99.0,
                },
            )
            .expect_err("out-of-range value must be rejected");
        assert!(matches!(err, CommandError::InvalidValue { .. }), "got {err:?}");
        assert_eq!(state, before);
        assert_eq!(history.undo_depth(), 0);

        let err = history
            .execute(
                &mut state,
                Command::SetMorphValue {
                    target: MorphId::for_slider("not_a_slider"),
                    value: 1.0,
                },
            )
            .expect_err("unknown slider must be rejected");
        assert!(matches!(err, CommandError::UnknownTarget { .. }), "got {err:?}");

        let err = history
            .execute(
                &mut state,
                Command::SetCamera {
                    eye: Some([f32::NAN, 1.0, 1.0]),
                    target: None,
                    up: None,
                    fov_degrees: None,
                    projection: None,
                },
            )
            .expect_err("non-finite camera must be rejected");
        assert!(matches!(err, CommandError::InvalidResult(_)), "got {err:?}");
        assert_eq!(state, before);
    }

    #[test]
    fn batch_is_atomic_and_undoes_as_one_entry() {
        let mut state = project();
        let mut history = CommandHistory::new(64);
        let before = state.clone();

        let outcome = history
            .execute(
                &mut state,
                Command::Batch {
                    commands: vec![
                        Command::SetMorphValue {
                            target: MorphId::for_slider("head_width"),
                            value: 1.25,
                        },
                        Command::SetBaseGender {
                            gender: BaseGender::Female,
                        },
                        Command::SetProportions {
                            head_scale: Some(1.1),
                            head_ratio: None,
                            shoulder_width: None,
                            leg_length: None,
                            arm_length: None,
                            neck_length: None,
                            torso_length: None,
                            height_overall: None,
                        },
                    ],
                },
            )
            .expect("batch must apply");

        assert_eq!(outcome.scope, ChangeScope::BaseGeometry, "batch keeps the strongest scope");
        assert_eq!(outcome.undo_depth, 1, "a batch is a single undo entry");
        assert_eq!(state.character.base_gender, BaseGender::Female);
        assert!(outcome.base_geometry_revision >= 1);

        history.undo(&mut state).expect("undo must succeed");
        assert_eq!(state, before, "batch undo restores everything at once");
    }

    #[test]
    fn history_limit_drops_the_oldest_entries() {
        let mut state = project();
        let mut history = CommandHistory::new(3);
        for value in [1.05_f32, 1.10, 1.15, 1.20] {
            history
                .execute(
                    &mut state,
                    Command::SetMorphValue {
                        target: MorphId::for_slider("head_width"),
                        value,
                    },
                )
                .expect("apply");
        }
        assert_eq!(history.undo_depth(), 3, "oldest entry is dropped");
    }

    #[test]
    fn commands_serialize_with_stable_kinds() {
        let command = Command::SetMorphValue {
            target: MorphId::for_slider("head_width"),
            value: 1.2,
        };
        let json = command.to_json().unwrap();
        assert_eq!(
            json,
            "{\"kind\":\"set_morph_value\",\"target\":\"mrf_head_width\",\"value\":1.2}"
        );
        assert_eq!(Command::from_json(&json).unwrap(), command);

        let camera = Command::OrbitCamera {
            azimuth: 0.1,
            elevation: -0.2,
        };
        let json = camera.to_json().unwrap();
        assert!(json.contains("\"kind\":\"orbit_camera\""), "got {json}");
        assert_eq!(Command::from_json(&json).unwrap(), camera);
    }

    #[test]
    fn empty_and_no_op_commands_are_rejected() {
        let mut state = project();
        let mut history = CommandHistory::new(8);
        assert!(matches!(
            history
                .execute(&mut state, Command::Batch { commands: vec![] })
                .unwrap_err(),
            CommandError::NoOp(_)
        ));
        assert!(matches!(
            history
                .execute(&mut state, Command::ResetMorphs)
                .unwrap_err(),
            CommandError::NoOp(_)
        ));
        assert!(matches!(
            history
                .execute(
                    &mut state,
                    Command::SetMaterialParams {
                        material_id: None,
                        patch: MaterialPatch::default(),
                    },
                )
                .unwrap_err(),
            CommandError::NoOp(_)
        ));
        assert!(matches!(
            history.undo(&mut state).unwrap_err(),
            CommandError::NothingToUndo
        ));
        assert!(matches!(
            history.redo(&mut state).unwrap_err(),
            CommandError::NothingToRedo
        ));
    }

    #[test]
    fn new_command_clears_the_redo_stack() {
        let mut state = project();
        let mut history = CommandHistory::new(8);
        history
            .execute(
                &mut state,
                Command::SetMorphValue {
                    target: MorphId::for_slider("head_width"),
                    value: 1.1,
                },
            )
            .unwrap();
        history.undo(&mut state).unwrap();
        assert!(history.can_redo());

        history
            .execute(
                &mut state,
                Command::SetMorphValue {
                    target: MorphId::for_slider("jaw_v_line_taper"),
                    value: 0.7,
                },
            )
            .unwrap();
        assert!(!history.can_redo(), "a new action invalidates the redo stack");
        assert!(history.last_redo_description().is_none());
    }

    #[test]
    fn material_patch_inverse_restores_only_touched_fields() {
        let mut state = project();
        let mut history = CommandHistory::new(8);
        let untouched = state.character_material().unwrap().material.hue_shift;

        history
            .execute(
                &mut state,
                Command::SetMaterialParams {
                    material_id: None,
                    patch: MaterialPatch {
                        shadow_threshold: Some(0.7),
                        ..MaterialPatch::default()
                    },
                },
            )
            .unwrap();
        let material = &state.character_material().unwrap().material;
        assert!((material.shadow_threshold - 0.7).abs() < 1e-6);
        assert_eq!(material.hue_shift, untouched, "untouched fields stay identical");

        history.undo(&mut state).unwrap();
        let restored = &state.character_material().unwrap().material;
        assert!((restored.shadow_threshold - 0.5).abs() < 1e-6);
        assert_eq!(restored.hue_shift, untouched);
    }

    #[test]
    fn preset_load_clears_morphs_only_for_non_mannequin_presets() {
        let mut state = project();
        let mut history = CommandHistory::new(8);
        history
            .execute(
                &mut state,
                Command::SetMorphValue {
                    target: MorphId::for_slider("head_width"),
                    value: 1.2,
                },
            )
            .unwrap();

        history
            .execute(&mut state, Command::LoadMeshPreset { preset: MeshPreset::Cube })
            .unwrap();
        assert_eq!(morph_override_count(&state), 0, "cube has no morph channels");

        history.undo(&mut state).unwrap();
        assert!((state.morph_value("head_width") - 1.2).abs() < 1e-6);

        history
            .execute(
                &mut state,
                Command::LoadMeshPreset {
                    preset: MeshPreset::Mannequin,
                },
            )
            .unwrap();
        assert!((state.morph_value("head_width") - 1.2).abs() < 1e-6);
    }

    /// P0 undo/redo item 4: the full `apply → undo → redo → undo` sequence, on a
    /// deep stack, must reproduce every intermediate fingerprint exactly.
    #[test]
    fn apply_undo_redo_sequence_restores_every_intermediate_fingerprint() {
        let mut state = project();
        let mut history = CommandHistory::new(64);

        let commands = vec![
            Command::SetMorphValue {
                target: MorphId::for_slider("head_width"),
                value: 1.35,
            },
            Command::SetSomatotype {
                endomorph: 0.5,
                mesomorph: 0.3,
                ectomorph: 0.2,
            },
            Command::SetProportions {
                head_scale: Some(1.2),
                head_ratio: None,
                shoulder_width: Some(1.15),
                leg_length: None,
                arm_length: None,
                neck_length: None,
                torso_length: None,
                height_overall: Some(1.05),
            },
            Command::SetLight {
                light_id: None,
                direction: None,
                color: None,
                intensity: Some(1.4),
                shadow_color: None,
                ambient_intensity: Some(0.2),
                shadow_saturation: None,
                ambient_sky: None,
                ambient_ground: None,
            },
            Command::SetMaterialParams {
                material_id: None,
                patch: MaterialPatch {
                    outline_width: Some(2.0),
                    toon_steps: Some(2.0),
                    ..MaterialPatch::default()
                },
            },
            Command::SetBackgroundColor {
                color: [0.2, 0.3, 0.4, 1.0],
            },
            Command::SetNodeVisibility {
                node_id: NodeId::canonical_character(),
                visible: false,
            },
            Command::RenameProject {
                name: "Sessão 07".to_string(),
            },
        ];

        // apply — record the fingerprint reached by every command
        let mut fingerprints = vec![state.content_fingerprint()];
        let mut descriptions = Vec::new();
        for (index, command) in commands.iter().cloned().enumerate() {
            let before = state.content_fingerprint();
            let outcome = history.execute(&mut state, command).expect("command applies");
            let after = state.content_fingerprint();
            assert_ne!(before, after, "command {index} must change the project");
            assert_eq!(outcome.undo_depth, index + 1, "one undo entry per command");
            assert_eq!(outcome.sequence, index as u64 + 1);
            assert!(!outcome.can_redo, "a new command clears the redo stack");
            descriptions.push(outcome.description);
            fingerprints.push(after);
        }

        // undo — walk back through every intermediate state
        for index in (0..commands.len()).rev() {
            let undone = history.undo(&mut state).expect("undo works");
            assert_eq!(state.content_fingerprint(), fingerprints[index]);
            assert_eq!(undone.description, format!("Undo {}", descriptions[index]));
            assert_eq!(undone.undo_depth, index);
            assert!(undone.can_redo);
        }
        assert!(!history.can_undo());
        assert_eq!(history.redo_depth(), commands.len());
        assert_eq!(state.content_fingerprint(), fingerprints[0]);

        // redo — walk forward again, same fingerprints
        for index in 0..commands.len() {
            let redone = history.redo(&mut state).expect("redo works");
            assert_eq!(state.content_fingerprint(), fingerprints[index + 1]);
            assert_eq!(redone.description, format!("Redo {}", descriptions[index]));
            assert_eq!(redone.redo_depth, commands.len() - index - 1);
        }
        assert!(!history.can_redo());
        assert_eq!(history.undo_depth(), commands.len());

        // undo once more — the sequence is repeatable, not a one-shot
        for index in (0..commands.len()).rev() {
            history.undo(&mut state).expect("second undo pass");
            assert_eq!(state.content_fingerprint(), fingerprints[index]);
        }
        assert_eq!(state.content_fingerprint(), project().content_fingerprint());

        // The log grew one entry per accepted command (never for the undos).
        assert_eq!(history.export_log().len(), commands.len());
    }

    /// P0 undo/redo item 1/3: a command that would change nothing is rejected
    /// and never becomes part of the session history.
    #[test]
    fn no_op_commands_are_rejected_and_never_recorded() {
        let mut state = project();
        let mut history = CommandHistory::new(16);

        let current_name = state.name.clone();
        let current_background = state.render.background_color;
        let current_dimorphism = state.character.gender_dimorphism;
        let current_head_width = state.morph_value("head_width");

        let no_ops = vec![
            Command::SetMorphValue {
                target: MorphId::for_slider("head_width"),
                value: current_head_width,
            },
            Command::SetProportions {
                head_scale: None,
                head_ratio: None,
                shoulder_width: None,
                leg_length: None,
                arm_length: None,
                neck_length: None,
                torso_length: None,
                height_overall: None,
            },
            Command::SetCamera {
                eye: None,
                target: None,
                up: None,
                fov_degrees: None,
                // Issue #13: entrar em ortográfica sem volume usa o
                // enquadramento perspectiva atual (nenhum salto visual).
                projection: Some(CameraProjectionPatch {
                    orthographic: Some(true),
                    ..CameraProjectionPatch::default()
                }),
            },
            Command::SetLight {
                light_id: None,
                direction: None,
                color: None,
                intensity: None,
                shadow_color: None,
                ambient_intensity: None,
                shadow_saturation: None,
                ambient_sky: None,
                ambient_ground: None,
            },
            Command::SetRenderSettings {
                msaa_samples: None,
                tonemap: None,
            },
            Command::SetNodeVisibility {
                node_id: NodeId::canonical_character(),
                visible: true, // the node is visible by default
            },
            Command::SetBackgroundColor {
                color: current_background,
            },
            Command::RenameProject {
                name: current_name,
            },
            Command::SetGenderDimorphism {
                value: current_dimorphism,
            },
        ];

        for command in no_ops {
            let error = history
                .execute(&mut state, command.clone())
                .expect_err(&format!("{command:?} must be rejected as a no-op"));
            assert!(
                matches!(error, CommandError::NoOp(_)),
                "{command:?} produced {error:?}, expected NoOp"
            );
        }

        assert_eq!(history.undo_depth(), 0, "no-ops never enter the history");
        assert_eq!(history.export_log().len(), 0, "no-ops never enter the log");
        assert_eq!(history.revision(), 0);

        // And a command that *does* change something still works afterwards.
        history
            .execute(
                &mut state,
                Command::SetMorphValue {
                    target: MorphId::for_slider("head_width"),
                    value: current_head_width + 0.2,
                },
            )
            .expect("a real change is accepted");
        assert_eq!(history.undo_depth(), 1);
        assert_eq!(history.export_log().len(), 1);
    }

    /// P0 undo/redo item 3: the persisted log holds commands, never snapshots,
    /// and only commands that were actually applied.
    #[test]
    fn command_log_only_contains_accepted_commands() {
        let mut state = project();
        let mut history = CommandHistory::new(64);

        let accepted = Command::SetMorphValue {
            target: MorphId::for_slider("head_width"),
            value: 1.25,
        };
        history.execute(&mut state, accepted.clone()).unwrap();

        // Rejected commands must not enter the log nor the stacks.
        let depth_before = history.undo_depth();
        let rejected: Vec<Command> = vec![
            Command::SetMorphValue {
                target: MorphId::for_slider("not_a_real_slider"),
                value: 1.0,
            },
            Command::SetMorphValue {
                target: MorphId::for_slider("head_width"),
                value: f32::NAN,
            },
            Command::SetMorphValue {
                target: MorphId::for_slider("head_width"),
                value: 1.25, // identical to the current value → NoOp
            },
            Command::Batch { commands: vec![] },
        ];
        for command in rejected {
            assert!(
                history.execute(&mut state, command).is_err(),
                "invalid command must be rejected"
            );
        }
        assert_eq!(history.undo_depth(), depth_before, "rejected commands are not recorded");

        let log = history.export_log();
        assert_eq!(log.version, COMMAND_LOG_VERSION);
        assert_eq!(log.len(), 1, "only the accepted command is logged");
        assert_eq!(log.entries[0].command, accepted);
        assert_eq!(log.entries[0].sequence, 1);
        assert_eq!(log.entries[0].revision, 1);
        assert!(!log.entries[0].description.is_empty());
        assert_eq!(log.entries[0].scope, ChangeScope::Deformation);
    }

    #[test]
    fn command_log_round_trips_and_replays_into_the_same_project() {
        let mut state = project();
        let mut history = CommandHistory::new(64);
        for command in [
            Command::SetMorphValue {
                target: MorphId::for_slider("head_width"),
                value: 1.3,
            },
            Command::SetSomatotype {
                endomorph: 0.4,
                mesomorph: 0.35,
                ectomorph: 0.25,
            },
            Command::SetMaterialParams {
                material_id: None,
                patch: MaterialPatch {
                    shadow_threshold: Some(0.42),
                    ..MaterialPatch::default()
                },
            },
            Command::SetGenderDimorphism { value: 0.35 },
        ] {
            history.execute(&mut state, command).unwrap();
        }

        let json = history.export_log().to_json().unwrap();
        let reloaded = CommandLog::from_json(&json).unwrap();
        assert_eq!(reloaded.len(), 4);

        let replayed = restore_from_log(&project(), &reloaded, 64).unwrap();
        assert_eq!(
            replayed.state.content_fingerprint(),
            state.content_fingerprint(),
            "replaying the log must rebuild the session byte for byte"
        );
        assert_eq!(replayed.history.revision(), history.revision());
        assert_eq!(replayed.history.undo_depth(), history.undo_depth());
        assert_eq!(replayed.outcomes.len(), 4);
        assert!(replayed.history.can_undo());

        // The rebuilt history is fully functional: undo/redo work on it.
        let mut rebuilt_state = replayed.state.clone();
        let mut rebuilt = replayed.history;
        rebuilt.undo(&mut rebuilt_state).unwrap();
        rebuilt.redo(&mut rebuilt_state).unwrap();
        assert_eq!(rebuilt_state.content_fingerprint(), state.content_fingerprint());
    }

    #[test]
    fn command_log_rejects_corrupted_and_newer_logs() {
        assert!(CommandLog::from_json("{not json").is_err());
        assert!(CommandLog::from_json("").is_err());

        let newer = format!(r#"{{"version":{},"entries":[]}}"#, COMMAND_LOG_VERSION + 1);
        let error = CommandLog::from_json(&newer).unwrap_err();
        assert!(matches!(error, CommandError::HistoryFailed(_)));

        let no_sequence = r#"{"version":1,"entries":[{"sequence":0,"revision":1,"description":"x","scope":"deformation","command":{"kind":"reset_morphs"}}]}"#;
        assert!(CommandLog::from_json(no_sequence).is_err());

        // A valid-looking log whose command cannot be applied names the index.
        let corrupt = CommandLog {
            version: COMMAND_LOG_VERSION,
            entries: vec![CommandLogEntry {
                sequence: 1,
                revision: 1,
                description: "morph".to_string(),
                scope: ChangeScope::Deformation,
                command: Command::SetMorphValue {
                    target: MorphId::for_slider("ghost_slider"),
                    value: 1.0,
                },
            }],
        };
        let (index, error) = restore_from_log(&project(), &corrupt, 64).unwrap_err();
        assert_eq!(index, 0, "the failing entry must be identified");
        assert!(matches!(error, CommandError::UnknownTarget { .. }));
    }

    #[test]
    fn restore_from_log_recovers_the_valid_prefix() {
        let log = CommandLog {
            version: COMMAND_LOG_VERSION,
            entries: vec![
                CommandLogEntry {
                    sequence: 1,
                    revision: 1,
                    description: "morph".to_string(),
                    scope: ChangeScope::Deformation,
                    command: Command::SetMorphValue {
                        // Within the catalog range of `head_width` (0.75..1.35):
                        // the *first* entry has to be valid for the test to
                        // reach the corrupted second one.
                        target: MorphId::for_slider("head_width"),
                        value: 1.3,
                    },
                },
                CommandLogEntry {
                    sequence: 2,
                    revision: 2,
                    description: "ghost".to_string(),
                    scope: ChangeScope::Deformation,
                    command: Command::SetMorphValue {
                        target: MorphId::for_slider("ghost_slider"),
                        value: 1.0,
                    },
                },
            ],
        };

        let (index, _) = restore_from_log(&project(), &log, 64).unwrap_err();
        assert_eq!(index, 1);
        // The caller keeps the prefix: everything before `index` replays fine.
        let prefix = CommandLog {
            version: COMMAND_LOG_VERSION,
            entries: log.entries[..index].to_vec(),
        };
        let recovered = restore_from_log(&project(), &prefix, 64).unwrap();
        assert_eq!(
            recovered.state.morph_value("head_width"),
            1.3,
            "the recoverable prefix restores the session"
        );
    }

    #[test]
    fn empty_log_serializes_and_replays_to_the_base_project() {
        let history = CommandHistory::new(8);
        let log = history.export_log();
        assert!(log.is_empty());
        assert!(log.commands().is_empty());
        let json = log.to_json().unwrap();
        let reloaded = CommandLog::from_json(&json).unwrap();
        assert_eq!(reloaded, log);

        let base = project();
        let replayed = restore_from_log(&base, &reloaded, 8).unwrap();
        assert_eq!(replayed.state.content_fingerprint(), base.content_fingerprint());
        assert!(!replayed.history.can_undo());
        assert!(!replayed.history.can_redo());
    }

    /// Cross-language fixture: the persisted log format must decode on both
    /// sides (`src/services/command_history.ts::parseCommandLog`) and replay
    /// into the canonical project.
    #[test]
    fn command_log_matches_the_frozen_fixture() {
        const FIXTURE: &str = include_str!("../../../contracts/fixtures/command_log_v1.json");
        let value: serde_json::Value =
            serde_json::from_str(FIXTURE).expect("command log fixture must be valid JSON");

        let log = CommandLog::from_json(&value.to_string()).expect("fixture must decode as a log");
        assert_eq!(log.version, COMMAND_LOG_VERSION);

        let expected_count = value["expected_entry_count"].as_u64().unwrap_or(0) as usize;
        assert!(expected_count > 0, "fixture must declare its expectations");
        assert_eq!(log.len(), expected_count);

        let kinds: Vec<String> = log
            .entries
            .iter()
            .map(|entry| {
                serde_json::to_value(&entry.command).unwrap()["kind"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        let expected_kinds: Vec<String> = value["expected_kinds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|kind| kind.as_str().unwrap().to_string())
            .collect();
        assert_eq!(kinds, expected_kinds);

        let expected_sequences: Vec<u64> = value["expected_sequences"]
            .as_array()
            .unwrap()
            .iter()
            .map(|sequence| sequence.as_u64().unwrap())
            .collect();
        let sequences: Vec<u64> = log.entries.iter().map(|entry| entry.sequence).collect();
        assert_eq!(sequences, expected_sequences);

        // Replaying the fixture rebuilds the session recorded in it.
        let replayed = restore_from_log(&ProjectState::default(), &log, 64)
            .expect("every fixture command must be applicable");
        assert_eq!(
            replayed.history.undo_depth(),
            value["expected_undo_depth"].as_u64().unwrap() as usize
        );
        assert_eq!(replayed.state.morph_value("head_width"), 1.25);
        assert!(!replayed.state.scene.nodes[0].visible);
        assert_eq!(replayed.outcomes.len(), log.len());
        // Every replayed outcome reports a deeper undo stack than the previous.
        for (index, outcome) in replayed.outcomes.iter().enumerate() {
            assert_eq!(outcome.undo_depth, index + 1);
            assert_eq!(outcome.sequence, index as u64 + 1);
        }
    }

    #[test]
    fn scene_of_and_morph_overrides_helpers_match_the_project() {
        let mut state = project();
        state
            .character
            .morph_values
            .insert(MorphId::for_slider("head_width"), 1.2);
        assert_eq!(scene_of(&state).nodes.len(), state.scene.nodes.len());
        let overrides = morph_overrides(&state);
        assert_eq!(overrides.get("mrf_head_width"), Some(&1.2));
    }
}
