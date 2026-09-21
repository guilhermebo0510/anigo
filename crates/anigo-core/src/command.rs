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

use crate::ids::{LightId, MaterialId, MorphId, NodeId};
use crate::mesh::BaseGender;
use crate::morph_catalog::find_slider_def;
use crate::project::{
    AssetEntry, AssetKind, MeshRef, NodeSlot, ProjectError, ProjectState, SceneState,
    URI_BASE_FEMALE, URI_BASE_MALE, URI_PRESET_CUBE, URI_PRESET_SPHERE,
};
use crate::scene::StylizedMaterial;
use crate::somatotype::SomatotypeCoords;

/// Monotonic revision counter of the project (bumped by every applied command).
pub type Revision = u64;

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
            Command::SetLight { .. }
            | Command::SetNodeVisibility { .. }
            | Command::SetBackgroundColor { .. }
            | Command::SetRenderSettings { .. } => ChangeScope::Presentation,
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
                Ok(())
            }
            Command::SetProportions { .. } => Ok(()),
            Command::SetCamera { .. } => Ok(()),
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
            Command::SetLight { light_id, .. } => {
                let id = light_id.clone().unwrap_or_else(LightId::canonical_key);
                if state.light(&id).is_none() && id != LightId::canonical_key() {
                    return Err(CommandError::UnknownTarget {
                        kind: "light",
                        target: id.to_string(),
                    });
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
                if !state.scene.nodes.iter().any(|node| &node.node_id == node_id) {
                    return Err(CommandError::UnknownTarget {
                        kind: "scene node",
                        target: node_id.to_string(),
                    });
                }
                Ok(())
            }
            Command::LoadMeshPreset { .. } => Ok(()),
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
                Ok(())
            }
            Command::SetRenderSettings { msaa_samples, .. } => {
                if let Some(samples) = msaa_samples {
                    if !matches!(samples, 1 | 2 | 4 | 8 | 16) {
                        return Err(CommandError::InvalidValue {
                            field: "msaa_samples".to_string(),
                            detail: format!("must be one of 1, 2, 4, 8, 16, found {samples}"),
                        });
                    }
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
                Ok(())
            }
            Command::Batch { commands } => {
                if commands.is_empty() {
                    return Err(CommandError::NoOp("empty batch".to_string()));
                }
                for command in commands {
                    command.validate(state)?;
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
            } => {
                let camera = &state.scene.camera.camera;
                Ok(Command::SetCamera {
                    eye: eye.map(|_| camera.eye.to_array()),
                    target: target.map(|_| camera.target.to_array()),
                    up: up.map(|_| camera.up.to_array()),
                    fov_degrees: fov_degrees.map(|_| camera.fov_y.to_degrees()),
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
            Command::LoadMeshPreset { .. } => {
                let node_id = NodeId::canonical_character();
                let node = find_node(state, &node_id)?;
                Ok(Command::Batch {
                    commands: vec![
                        Command::SetNodeMesh {
                            node_id: node_id.clone(),
                            mesh: node.mesh.clone(),
                        },
                        Command::SetBaseGender {
                            gender: state.character.base_gender,
                        },
                    ],
                })
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
                state.character.active_preset_id = Some(match gender {
                    BaseGender::Male => "mannequin_male".to_string(),
                    BaseGender::Female => "mannequin_female".to_string(),
                });
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
                let node = find_node_mut(state, node_id)?;
                node.mesh = mesh.clone();
                if let Some(reference) = mesh {
                    state
                        .assets
                        .entry(reference.asset_id.clone())
                        .or_insert_with(|| AssetEntry::from_uri(AssetKind::Mesh, format!("anigo://asset/{}", reference.asset_id)));
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
                state.character.active_preset_id = Some(match preset {
                    MeshPreset::Cube => "cube".to_string(),
                    MeshPreset::Sphere => "sphere".to_string(),
                    MeshPreset::Mannequin => match gender {
                        BaseGender::Male => "mannequin_male".to_string(),
                        BaseGender::Female => "mannequin_female".to_string(),
                    },
                });
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

    /// Serializes the history log (deterministic; used by tests + autosave).
    pub fn log_json(&self) -> Result<String, CommandError> {
        serde_json::to_string_pretty(&self.undo).map_err(|e| CommandError::HistoryFailed(e.to_string()))
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

        Ok(CommandOutcome {
            description: format!("Undo {}", entry.description),
            scope: applied.scope,
            affected: applied.affected,
            ..self.outcome(&applied)
        })
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

        Ok(CommandOutcome {
            description: format!("Redo {}", entry.description),
            scope: applied.scope,
            affected: applied.affected,
            ..self.outcome(&applied)
        })
    }

    /// Clears both stacks (project load / new project).
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
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
            Command::RenameProject {
                name: "Projeto de Teste".to_string(),
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
