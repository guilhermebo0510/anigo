//! ANIGO Core Snapshot — the canonical data hand-off to every renderer
//! (P0 — ARQUITETURA_CANONICA_ANIGO §5 and §6).
//!
//! The viewport, the headless render and the export path must all consume the
//! *same* description of the scene. This module defines that description and
//! nothing else: no state, no caches, no rendering decisions.
//!
//! ```text
//! ProjectState ──(commands)──► CoreSnapshot
//!                                 ├── static_payload  : base mesh + sparse morph channels
//!                                 └── dynamic         : weights, camera, lights, materials, render
//!                                       ▲                     ▲
//!                                   viewport              headless / MCP export
//! ```
//!
//! The static payload is expensive (vertex + delta buffers) and only changes
//! when base geometry changes, so the client asks for a snapshot quoting the
//! `static_revision` it already has: the buffers are re-sent only when stale.
//!
//! Binary layouts are frozen by contract and asserted by tests on both sides of
//! the boundary (`contracts/fixtures/core_snapshot_v1.json`):
//!
//! ```text
//! vertex : 72 bytes  pos f32x3 | normal f32x3 | uv f32x2 | color f32x4 | joints u16x4 | weights f32x4
//! delta  : 32 bytes  vertex_index u32 | dpos f32x3 | dnormal f32x3 | pad f32
//! channel: 16 bytes  weight f32 | start_offset u32 | delta_count u32 | pad u32
//! ```

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::deformation::{
    catalog_weights, DeformationInputs, DeformationError, PROPORTION_POLICY_VERSION,
    SOMATOTYPE_POLICY_VERSION,
};
use crate::ids::{AssetId, CharacterId, LightId, MaterialId, MorphId, ProjectId};
use crate::bone_sync::BondSyncManager;
use crate::mesh::{BaseGender, Mesh, Vertex};
use crate::morph::SparseMorphSet;
use crate::morph_catalog::ALL_MORPH_SLIDERS;
use crate::project::{ColorManagement, ProjectState, RenderState, TonemapOperator};
use crate::scene::StylizedLight;
use crate::somatotype::SomatotypeCoords;

/// Version of the snapshot wire format.
pub const SNAPSHOT_FORMAT_VERSION: u32 = 1;
/// Bytes per packed vertex (see module docs).
pub const VERTEX_STRIDE_BYTES: u32 = 72;
/// Bytes per sparse morph delta.
pub const MORPH_DELTA_STRIDE_BYTES: u32 = 32;
/// Bytes per morph channel descriptor.
pub const MORPH_CHANNEL_STRIDE_BYTES: u32 = 16;

/// Who is allowed to compute deformation for this snapshot.
///
/// `Core` means the payload carries a complete deformation model and the client
/// must only accumulate it. `ReferenceTs` is the *degraded* mode used while the
/// core model is incomplete: the client is told, in the payload itself, that it
/// must not treat its own reference engine as authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeformationAuthority {
    /// Rust core is authoritative (production).
    Core,
    /// Reference TypeScript engine only (browser preview / incomplete core).
    ReferenceTs,
}

/// Honest report of what the core deformation model covers.
///
/// The client is required to consult this report before using its own
/// deformation: `is_complete() == false` means the payload cannot reproduce the
/// whole character and the session must be marked as degraded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeformationCoverage {
    /// Canonical catalog sliders (currently 157).
    pub total_sliders: u32,
    /// Sliders that have at least one sparse delta in the core model.
    pub sliders_with_geometry: u32,
    /// Total morph targets in the model (catalog length).
    pub morph_targets: u32,
    /// `true` when the macro proportion policy is baked into the base geometry.
    pub bakes_proportions: bool,
    /// `true` when gender dimorphism is baked via base mesh interpolation.
    pub bakes_gender: bool,
    /// `true` when the somatotype is expanded into canonical macro sliders.
    pub somatotype_via_macro_sliders: bool,
    /// Version of the proportion policy applied to the base geometry.
    pub proportion_policy_version: u32,
    /// Version of the somatotype policy applied to the weights.
    pub somatotype_policy_version: u32,
}

impl DeformationCoverage {
    /// `true` only when the core can reproduce the full character without any
    /// client-side deformation implementation.
    pub fn is_complete(&self) -> bool {
        self.bakes_proportions
            && self.bakes_gender
            && self.somatotype_via_macro_sliders
            && self.total_sliders > 0
            && self.sliders_with_geometry == self.total_sliders
    }

    /// Sliders without core geometry (used by diagnostics and tests).
    pub fn missing_sliders(&self) -> u32 {
        self.total_sliders
            .saturating_sub(self.sliders_with_geometry)
    }

    /// Authority implied by this coverage report.
    pub fn authority(&self) -> DeformationAuthority {
        if self.is_complete() {
            DeformationAuthority::Core
        } else {
            DeformationAuthority::ReferenceTs
        }
    }

    /// Fraction of the catalog backed by core geometry, in `[0, 1]`.
    pub fn ratio(&self) -> f32 {
        if self.total_sliders == 0 {
            0.0
        } else {
            self.sliders_with_geometry as f32 / self.total_sliders as f32
        }
    }
}

/// Descriptor of one sparse morph channel in the static payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MorphChannelDescriptor {
    /// Stable morph id (`mrf_<slider>`).
    pub target: MorphId,
    /// Catalog slider id (`<slider>`).
    pub slider_id: String,
    /// Index of the first delta inside the delta buffer.
    pub start_offset: u32,
    /// Number of deltas of this channel.
    pub delta_count: u32,
}

/// Morph value + normalized weight (`value - catalog_default`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MorphWeight {
    /// Stable morph id.
    pub target: MorphId,
    /// Catalog slider id.
    pub slider_id: String,
    /// Authored slider value.
    pub value: f32,
    /// Accumulation weight used by the vertex shader/compute pass.
    pub weight: f32,
}

/// Static (expensive) part of the snapshot: base geometry + morph channels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StaticGeometryPayload {
    /// Wire format version.
    pub format_version: u32,
    /// Revision this payload corresponds to (client cache key).
    pub static_revision: u64,
    /// Base gender the geometry was built for.
    pub base_gender: BaseGender,
    /// Asset id of the mesh this geometry came from.
    pub mesh_asset_id: AssetId,
    /// Stable id of the mesh inside the project.
    pub mesh_uri: String,
    /// Vertex count.
    pub vertex_count: u32,
    /// Index count (3 per triangle).
    pub index_count: u32,
    /// Bytes per packed vertex (always [`VERTEX_STRIDE_BYTES`]).
    pub vertex_stride_bytes: u32,
    /// Packed vertex buffer (base64 of the little-endian bytes).
    pub vertex_buffer_base64: String,
    /// Index buffer (base64, `u32` little-endian).
    pub index_buffer_base64: String,
    /// Stable hash of topology + base positions (cache validation).
    pub topology_hash: String,
    /// Bytes per morph delta (always [`MORPH_DELTA_STRIDE_BYTES`]).
    pub morph_delta_stride_bytes: u32,
    /// Channels present in the delta buffer, in catalog order.
    pub morph_channels: Vec<MorphChannelDescriptor>,
    /// Concatenated sparse deltas (base64).
    pub morph_deltas_base64: String,
    /// Total number of deltas in the buffer.
    pub morph_total_deltas: u32,
    /// Fingerprint of the canonical morph catalog used to build the channels.
    pub catalog_fingerprint: String,
    /// P1-04: paleta de skinning + estado do rig (a malha já vem atribuída).
    pub skin: SkinPayload,
}

/// Skinning entregue junto da geometria estática (P1-04).
///
/// A paleta é `world × inverse bind` por osso, 16 floats por osso (ordem de
/// colunas), pronta para o bloco `bones` do contrato. A malha carrega até 4
/// influências por vértice nos atributos 4 e 5 do layout de 72 B.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkinPayload {
    /// Ossos na paleta (24 no esqueleto canônico).
    pub bone_count: u32,
    /// Matrizes de skinning: `bone_count * 16` floats.
    pub palette: Vec<f32>,
    /// Nome da pose de bind (`canonical_rest`).
    pub bind_pose: String,
    /// As proporções estão assadas na malha base (o bake está ligado).
    pub proportions_baked: bool,
    /// A paleta entregue é a identidade — skinning não deforma duas vezes.
    pub palette_is_identity: bool,
}

impl SkinPayload {
    /// Paleta canônica enquanto o núcleo assa as proporções na malha base.
    pub fn canonical_base() -> Self {
        let skeleton = BondSyncManager::create_canonical_humanoid();
        let palette = crate::skinning::identity_palette_floats(skeleton.joints.len());
        Self {
            bone_count: skeleton.joints.len() as u32,
            palette,
            bind_pose: "canonical_rest".to_string(),
            proportions_baked: crate::skinning::PROPORTIONS_BAKED_INTO_BASE_MESH,
            palette_is_identity: true,
        }
    }
}

/// Camera state handed to the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraSnapshot {
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub up: [f32; 3],
    pub fov_y_radians: f32,
    pub z_near: f32,
    pub z_far: f32,
    pub aspect: f32,
}

/// Light state handed to the renderer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LightSnapshot {
    pub light_id: LightId,
    pub direction: [f32; 3],
    pub color: [f32; 3],
    pub intensity: f32,
    pub shadow_color: [f32; 3],
    pub ambient_intensity: f32,
    pub shadow_saturation: f32,
    pub ambient_sky: [f32; 3],
    pub ambient_ground: [f32; 3],
}

impl LightSnapshot {
    /// Builds the snapshot of a stylized light.
    pub fn from_light(light_id: &LightId, light: &StylizedLight) -> Self {
        Self {
            light_id: light_id.clone(),
            direction: light.direction,
            color: light.color,
            intensity: light.intensity,
            shadow_color: light.shadow_color,
            ambient_intensity: light.ambient_intensity,
            shadow_saturation: light.shadow_saturation,
            ambient_sky: light.ambient_sky,
            ambient_ground: light.ambient_ground,
        }
    }
}

fn default_material_snapshot_emission_color() -> [f32; 4] { [0.0, 0.0, 0.0, 0.0] }
fn default_material_snapshot_shade_toony() -> bool { true }
fn default_material_snapshot_face_smoothness() -> f32 { 0.05 }

/// Material state handed to the renderer (canonical NPR parameters only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialSnapshot {
    pub material_id: MaterialId,
    pub name: String,
    pub base_color: [f32; 4],
    pub shade_color: [f32; 4],
    pub outline_color: [f32; 4],
    pub outline_width: f32,
    pub outline_opacity: f32,
    pub outline_smoothness: f32,
    pub outline_depth_bias: f32,
    pub shadow_threshold: f32,
    pub shadow_smoothness: f32,
    pub specular_color: [f32; 4],
    pub spec_intensity: f32,
    pub spec_power: f32,
    pub specular_softness: f32,
    pub specular_offset: f32,
    pub specular_size: f32,
    pub rim_color: [f32; 4],
    pub rim_intensity: f32,
    pub rim_spread: f32,
    pub hue_shift: f32,
    pub toon_steps: f32,
    pub ao_intensity: f32,
    // Fase 2 (#18): parâmetros MToon (VRMC_materials_mtoon) — com defaults
    // para snapshots antigos continuam abrindo intactos.
    #[serde(default = "default_material_snapshot_emission_color")]
    pub mtoon_emission_color: [f32; 4],
    #[serde(default)]
    pub mtoon_emission_intensity: f32,
    #[serde(default)]
    pub mtoon_second_shade_shift: f32,
    #[serde(default)]
    pub mtoon_second_shade_softness: f32,
    #[serde(default)]
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
    #[serde(default)]
    pub mtoon_matcap_mode: u8,
    #[serde(default = "default_material_snapshot_shade_toony")]
    pub mtoon_shade_toony: bool,
    // Fase 2 (#17): sombra facial SDF — defaults para snapshots antigos.
    #[serde(default)]
    pub face_shadow_offset: f32,
    #[serde(default = "default_material_snapshot_face_smoothness")]
    pub face_shadow_smoothness: f32,
    #[serde(default)]
    pub face_sdf_enabled: bool,
}

impl MaterialSnapshot {
    /// Builds the snapshot of a stylized material.
    pub fn from_material(
        material_id: &MaterialId,
        entry: &crate::project::MaterialEntry,
    ) -> Self {
        let material = &entry.material;
        Self {
            material_id: material_id.clone(),
            name: entry.name.clone(),
            base_color: material.base_color,
            shade_color: material.shade_color,
            outline_color: material.outline_color,
            outline_width: material.outline_width,
            outline_opacity: material.outline_opacity,
            outline_smoothness: material.outline_smoothness,
            outline_depth_bias: material.outline_depth_bias,
            shadow_threshold: material.shadow_threshold,
            shadow_smoothness: material.shadow_smoothness,
            specular_color: material.specular_color,
            spec_intensity: material.spec_intensity,
            spec_power: material.spec_power,
            specular_softness: material.specular_softness,
            specular_offset: material.specular_offset,
            specular_size: material.specular_size,
            rim_color: material.rim_color,
            rim_intensity: material.rim_intensity,
            rim_spread: material.rim_spread,
            hue_shift: material.hue_shift,
            toon_steps: material.toon_steps,
            ao_intensity: material.ao_intensity,
            mtoon_emission_color: material.mtoon_emission_color,
            mtoon_emission_intensity: material.mtoon_emission_intensity,
            mtoon_second_shade_shift: material.mtoon_second_shade_shift,
            mtoon_second_shade_softness: material.mtoon_second_shade_softness,
            mtoon_matcap_intensity: material.mtoon_matcap_intensity,
            mtoon_main_texture_enabled: material.mtoon_main_texture_enabled,
            mtoon_shade_texture_enabled: material.mtoon_shade_texture_enabled,
            mtoon_second_shade_texture_enabled: material.mtoon_second_shade_texture_enabled,
            mtoon_emission_texture_enabled: material.mtoon_emission_texture_enabled,
            mtoon_matcap_enabled: material.mtoon_matcap_enabled,
            mtoon_matcap_mode: material.mtoon_matcap_mode,
            mtoon_shade_toony: material.mtoon_shade_toony,
            // Fase 2 (#17): sombra facial SDF
            face_shadow_offset: material.face_shadow_offset,
            face_shadow_smoothness: material.face_shadow_smoothness,
            face_sdf_enabled: material.face_sdf_enabled,
        }
    }
}

/// Render + color management block of the snapshot (§6.2).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RenderSnapshot {
    pub settings_version: u32,
    pub msaa_samples: u32,
    pub background_color: [f32; 4],
    pub color: ColorManagement,
    pub tonemap: TonemapOperator,
}

impl From<&RenderState> for RenderSnapshot {
    fn from(render: &RenderState) -> Self {
        Self {
            settings_version: render.settings_version,
            msaa_samples: render.msaa_samples,
            background_color: render.background_color,
            color: render.color,
            tonemap: render.tonemap,
        }
    }
}

/// Per-node visibility/material binding handed to the renderer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeSnapshot {
    pub node_id: String,
    pub name: String,
    pub visible: bool,
    pub material_id: Option<MaterialId>,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

/// Dynamic (cheap) part of the snapshot: everything that changes per edit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DynamicStatePayload {
    /// Monotonic revision of the dynamic state.
    pub dynamic_revision: u64,
    /// Revision of the static payload this dynamic state belongs to.
    pub static_revision: u64,
    /// Project id.
    pub project_id: ProjectId,
    /// Project display name.
    pub project_name: String,
    /// Character id.
    pub character_id: CharacterId,
    /// Base gender of the character.
    pub base_gender: BaseGender,
    /// Continuous gender dimorphism.
    pub gender_dimorphism: f32,
    /// Normalized somatotype.
    pub somatotype: SomatotypeCoords,
    /// Morph weights (`value - default`) in catalog order, sparse.
    pub morph_weights: Vec<MorphWeight>,
    /// Camera.
    pub camera: CameraSnapshot,
    /// Lights.
    pub lights: Vec<LightSnapshot>,
    /// Materials.
    pub materials: Vec<MaterialSnapshot>,
    /// Scene nodes.
    pub nodes: Vec<NodeSnapshot>,
    /// Render + color management.
    pub render: RenderSnapshot,
    /// Who must compute deformation for this payload.
    pub deformation_authority: DeformationAuthority,
    /// What the core model actually covers.
    pub deformation_coverage: DeformationCoverage,
}

/// Complete snapshot returned to a renderer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreSnapshot {
    /// Wire format version.
    pub snapshot_version: u32,
    /// Static payload — `None` when the caller already has this revision.
    pub static_payload: Option<StaticGeometryPayload>,
    /// Dynamic payload — always present.
    pub dynamic: DynamicStatePayload,
}

impl CoreSnapshot {
    /// Serializes the snapshot for the Tauri/MCP boundary.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Parses a snapshot (contract tests + headless consumers).
    pub fn from_json(raw: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(raw)
    }
}

// ---------------------------------------------------------------------------
// Binary helpers (frozen layouts)
// ---------------------------------------------------------------------------

fn encode_bytes(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode_bytes(raw: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD
        .decode(raw)
        .map_err(|e| format!("invalid base64 buffer: {e}"))
}

/// Packs vertices into the frozen 72-byte layout.
pub fn pack_vertices(vertices: &[Vertex]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vertices.len() * VERTEX_STRIDE_BYTES as usize);
    for vertex in vertices {
        for component in vertex.position {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
        for component in vertex.normal {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
        for component in vertex.uv {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
        for component in vertex.color {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
        for joint in vertex.joints {
            bytes.extend_from_slice(&joint.to_le_bytes());
        }
        for weight in vertex.weights {
            bytes.extend_from_slice(&weight.to_le_bytes());
        }
    }
    bytes
}

/// Packs `u32` indices little-endian.
pub fn pack_indices(indices: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(indices.len() * 4);
    for index in indices {
        bytes.extend_from_slice(&index.to_le_bytes());
    }
    bytes
}

/// Packs sparse morph deltas into the frozen 32-byte layout (vertex ordered).
pub fn pack_deltas(deltas: &[crate::morph::SparseMorphDelta]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(deltas.len() * MORPH_DELTA_STRIDE_BYTES as usize);
    for delta in deltas {
        bytes.extend_from_slice(&delta.vertex_index.to_le_bytes());
        for component in delta.delta_position {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
        for component in delta.delta_normal {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
        bytes.extend_from_slice(&delta._pad.to_le_bytes());
    }
    bytes
}

/// Encodes a byte buffer as base64 (wire format of the snapshot).
pub fn encode_buffer(bytes: &[u8]) -> String {
    encode_bytes(bytes)
}

/// Decodes a base64 buffer, validating its length against a stride.
pub fn decode_buffer(raw: &str, stride: u32) -> Result<Vec<u8>, String> {
    let bytes = decode_bytes(raw)?;
    if stride == 0 {
        return Ok(bytes);
    }
    if bytes.len() % stride as usize != 0 {
        return Err(format!(
            "buffer length {} is not a multiple of the stride {stride}",
            bytes.len()
        ));
    }
    Ok(bytes)
}

/// Decodes a base64 `f32` buffer.
pub fn decode_f32_buffer(raw: &str, stride: u32) -> Result<Vec<f32>, String> {
    let bytes = decode_buffer(raw, stride)?;
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

/// Stable fingerprint of a mesh (topology + base positions), FNV-1a over the
/// packed vertex buffer and indices.
pub fn mesh_topology_hash(mesh: &Mesh) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |byte: u8| {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for vertex in &mesh.vertices {
        for component in vertex.position {
            for byte in component.to_le_bytes() {
                mix(byte);
            }
        }
    }
    for index in &mesh.indices {
        for byte in index.to_le_bytes() {
            mix(byte);
        }
    }
    format!("{hash:016x}")
}

/// Fingerprint of the canonical morph catalog (ids + zones + ranges), so a
/// snapshot built against a different catalog is detectable.
pub fn catalog_fingerprint() -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |byte: u8| {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for def in ALL_MORPH_SLIDERS.iter() {
        for byte in def.id.as_bytes() {
            mix(*byte);
        }
        for value in [def.min, def.default_value, def.max] {
            for byte in value.to_le_bytes() {
                mix(byte);
            }
        }
    }
    format!("{hash:016x}")
}

/// Builds the static payload (base geometry + sparse morph channels).
pub fn build_static_payload(
    mesh: &Mesh,
    morph_set: &SparseMorphSet,
    weights: &[f32],
    static_revision: u64,
    base_gender: BaseGender,
    mesh_uri: &str,
) -> StaticGeometryPayload {
    let (_, channels, deltas) = morph_set.pack_all_channels(weights, mesh.vertices.len() as u32);

    let descriptors: Vec<MorphChannelDescriptor> = morph_set
        .targets
        .iter()
        .enumerate()
        .map(|(index, target)| {
            let channel = channels.get(index);
            MorphChannelDescriptor {
                target: MorphId::for_slider(&target.name),
                slider_id: target.name.clone(),
                start_offset: channel.map_or(0, |channel| channel.start_offset),
                delta_count: channel.map_or(0, |channel| channel.delta_count),
            }
        })
        .collect();

    StaticGeometryPayload {
        format_version: SNAPSHOT_FORMAT_VERSION,
        static_revision,
        base_gender,
        mesh_asset_id: AssetId::for_uri(mesh_uri),
        mesh_uri: mesh_uri.to_string(),
        vertex_count: mesh.vertices.len() as u32,
        index_count: mesh.indices.len() as u32,
        vertex_stride_bytes: VERTEX_STRIDE_BYTES,
        vertex_buffer_base64: encode_bytes(&pack_vertices(&mesh.vertices)),
        index_buffer_base64: encode_bytes(&pack_indices(&mesh.indices)),
        topology_hash: mesh_topology_hash(mesh),
        morph_delta_stride_bytes: MORPH_DELTA_STRIDE_BYTES,
        morph_channels: descriptors,
        morph_deltas_base64: encode_bytes(&pack_deltas(&deltas)),
        morph_total_deltas: deltas.len() as u32,
        catalog_fingerprint: catalog_fingerprint(),
        skin: SkinPayload::canonical_base(),
    }
}

/// Computes the coverage report of a morph set (how many sliders deform).
///
/// `bakes_*` describe the core pipeline: [`build_snapshot_for_project`] applies
/// the gender interpolation and the proportion policy to the base mesh, and
/// [`DeformationInputs`] expands the somatotype into macro sliders — so those
/// inputs are part of the model by construction.
pub fn coverage_of(morph_set: &SparseMorphSet) -> DeformationCoverage {
    let total = ALL_MORPH_SLIDERS.len() as u32;
    let with_geometry = morph_set
        .targets
        .iter()
        .filter(|target| !target.deltas.is_empty())
        .count() as u32;
    DeformationCoverage {
        total_sliders: total,
        sliders_with_geometry: with_geometry,
        morph_targets: morph_set.targets.len() as u32,
        bakes_proportions: true,
        bakes_gender: true,
        somatotype_via_macro_sliders: true,
        proportion_policy_version: PROPORTION_POLICY_VERSION,
        somatotype_policy_version: SOMATOTYPE_POLICY_VERSION,
    }
}

/// Lists the canonical sliders that received no geometry (diagnostics/tests).
pub fn sliders_without_geometry(morph_set: &SparseMorphSet) -> Vec<&str> {
    morph_set
        .targets
        .iter()
        .filter(|target| target.deltas.is_empty())
        .map(|target| target.name.as_str())
        .collect()
}

/// Builds the dynamic payload from the authoritative project state.
pub fn build_dynamic_payload(
    project: &ProjectState,
    inputs: &DeformationInputs,
    dynamic_revision: u64,
    static_revision: u64,
    coverage: DeformationCoverage,
) -> DynamicStatePayload {
    // Weights come from the deformation inputs (morph overrides + somatotype
    // expanded into macro sliders), never from a client-side reconstruction.
    let mut morph_weights = Vec::new();
    for def in ALL_MORPH_SLIDERS.iter() {
        let weight = inputs.weight_of(def.id);
        if weight.abs() <= 1e-6 {
            continue; // sparse: only active channels are transmitted
        }
        morph_weights.push(MorphWeight {
            target: MorphId::for_slider(def.id),
            slider_id: def.id.to_string(),
            value: def.default_value + weight,
            weight,
        });
    }

    let camera = &project.scene.camera.camera;
    DynamicStatePayload {
        dynamic_revision,
        static_revision,
        project_id: project.project_id.clone(),
        project_name: project.name.clone(),
        character_id: project.character.character_id.clone(),
        base_gender: project.character.base_gender,
        gender_dimorphism: project.character.gender_dimorphism,
        somatotype: project.character.somatotype,
        morph_weights,
        camera: CameraSnapshot {
            eye: camera.eye.to_array(),
            target: camera.target.to_array(),
            up: camera.up.to_array(),
            fov_y_radians: camera.fov_y,
            z_near: camera.z_near,
            z_far: camera.z_far,
            aspect: camera.aspect,
        },
        lights: project
            .scene
            .lights
            .iter()
            .map(|slot| LightSnapshot::from_light(&slot.light_id, &slot.light))
            .collect(),
        materials: project
            .materials
            .iter()
            .map(|(id, entry)| MaterialSnapshot::from_material(id, entry))
            .collect(),
        nodes: project
            .scene
            .nodes
            .iter()
            .map(|node| NodeSnapshot {
                node_id: node.node_id.to_string(),
                name: node.name.clone(),
                visible: node.visible,
                material_id: node.material_id.clone(),
                translation: node.transform.translation.to_array(),
                rotation: [
                    node.transform.rotation.x,
                    node.transform.rotation.y,
                    node.transform.rotation.z,
                    node.transform.rotation.w,
                ],
                scale: node.transform.scale.to_array(),
            })
            .collect(),
        render: RenderSnapshot::from(&project.render),
        deformation_authority: coverage.authority(),
        deformation_coverage: coverage,
    }
}

/// Builds a snapshot for a project, preparing the base geometry with the
/// canonical deformation pipeline (gender interpolation + proportions).
///
/// This is the API the app shell, the MCP tools and the headless renderer must
/// use: it is impossible to obtain a snapshot whose base geometry did not come
/// from [`crate::deformation::prepare_base_mesh`].
pub fn build_snapshot_for_project(
    project: &ProjectState,
    morph_set: &SparseMorphSet,
    dynamic_revision: u64,
    static_revision: u64,
    include_static: bool,
    mesh_uri: &str,
) -> Result<CoreSnapshot, DeformationError> {
    let base_mesh = crate::deformation::prepare_base_mesh(project)?;
    Ok(build_snapshot(
        project,
        &base_mesh,
        morph_set,
        dynamic_revision,
        static_revision,
        include_static,
        mesh_uri,
    ))
}

/// Low-level builder for callers that already own a prepared base mesh.
///
/// `mesh` must be the output of [`crate::deformation::prepare_base_mesh`] for
/// the same project, otherwise the coverage report would overstate the model
/// ([`build_snapshot_for_project`] is the safe entry point).
pub fn build_snapshot(
    project: &ProjectState,
    mesh: &Mesh,
    morph_set: &SparseMorphSet,
    dynamic_revision: u64,
    static_revision: u64,
    include_static: bool,
    mesh_uri: &str,
) -> CoreSnapshot {
    let coverage = coverage_of(morph_set);
    let inputs = DeformationInputs::from_project(project);
    let weights = catalog_weights(&inputs);

    let static_payload = if include_static {
        Some(build_static_payload(
            mesh,
            morph_set,
            &weights,
            static_revision,
            project.character.base_gender,
            mesh_uri,
        ))
    } else {
        None
    };

    CoreSnapshot {
        snapshot_version: SNAPSHOT_FORMAT_VERSION,
        static_payload,
        dynamic: build_dynamic_payload(
            project,
            &inputs,
            dynamic_revision,
            static_revision,
            coverage,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::BaseGender;
    use crate::morph_catalog::{build_canonical_sparse_morph_set, MorphCatalog};

    fn fixture_json() -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("contracts")
            .join("fixtures")
            .join("core_snapshot_v1.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("fixture {} unreadable: {e}", path.display()));
        serde_json::from_str(&raw).expect("fixture must be valid JSON")
    }

    #[test]
    fn frozen_layouts_match_the_contract_fixture() {
        // The fixture is produced by the TypeScript contract test and MUST
        // decode to the same bytes/values on the Rust side.
        let fixture = fixture_json();
        let static_payload = &fixture["static_payload"];

        let stride = static_payload["vertex_stride_bytes"].as_u64().unwrap() as u32;
        assert_eq!(stride, VERTEX_STRIDE_BYTES);
        let vertex_count = static_payload["vertex_count"].as_u64().unwrap() as u32;
        let vertices = decode_buffer(
            static_payload["vertex_buffer_base64"].as_str().unwrap(),
            stride,
        )
        .expect("vertex buffer must decode");
        assert_eq!(vertices.len() as u32, vertex_count * stride);

        let deltas = decode_buffer(
            static_payload["morph_deltas_base64"].as_str().unwrap(),
            MORPH_DELTA_STRIDE_BYTES,
        )
        .expect("delta buffer must decode");
        let total_deltas = static_payload["morph_total_deltas"].as_u64().unwrap() as u32;
        assert_eq!(deltas.len() as u32, total_deltas * MORPH_DELTA_STRIDE_BYTES);

        // First vertex position must match the fixture's documented values.
        let position = [
            f32::from_le_bytes([vertices[0], vertices[1], vertices[2], vertices[3]]),
            f32::from_le_bytes([vertices[4], vertices[5], vertices[6], vertices[7]]),
            f32::from_le_bytes([vertices[8], vertices[9], vertices[10], vertices[11]]),
        ];
        let expected = &fixture["expected_first_position"];
        for (index, component) in position.iter().enumerate() {
            let expected_value = expected[index].as_f64().unwrap() as f32;
            assert!(
                (component - expected_value).abs() < 1e-6,
                "vertex[{index}] = {component}, expected {expected_value}"
            );
        }

        // First delta: vertex_index + delta position.
        let vertex_index = u32::from_le_bytes([deltas[0], deltas[1], deltas[2], deltas[3]]);
        assert_eq!(
            vertex_index,
            fixture["expected_first_delta_vertex_index"].as_u64().unwrap() as u32
        );
        let delta_position = [
            f32::from_le_bytes([deltas[4], deltas[5], deltas[6], deltas[7]]),
            f32::from_le_bytes([deltas[8], deltas[9], deltas[10], deltas[11]]),
            f32::from_le_bytes([deltas[12], deltas[13], deltas[14], deltas[15]]),
        ];
        let expected_delta = fixture["expected_first_delta_position"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_f64().unwrap() as f32)
            .collect::<Vec<f32>>();
        for (index, component) in delta_position.iter().enumerate() {
            assert!((component - expected_delta[index]).abs() < 1e-6);
        }
    }

    #[test]
    fn production_snapshot_carries_the_bone_palette() {
        // P1-04: a paleta sai do núcleo junto da geometria estática, então o
        // renderer nunca precisa inventar ossos.
        let project = ProjectState::default();
        let mesh = crate::deformation::prepare_base_mesh(&project).expect("base mesh");
        let morph_set = build_canonical_sparse_morph_set(&mesh);
        let snapshot = build_snapshot_for_project(
            &project,
            &morph_set,
            1,
            1,
            true,
            "anigo://canonical/base",
        )
        .expect("snapshot do projeto padrão");

        let payload = snapshot
            .static_payload
            .as_ref()
            .expect("snapshot com geometria estática");
        assert_eq!(payload.vertex_count, 4070);
        assert_eq!(
            payload.morph_channels.len(),
            crate::morph_catalog::ALL_MORPH_SLIDERS.len()
        );
        assert_eq!(
            payload.skin.bone_count as usize,
            crate::bone_sync::CANONICAL_JOINT_COUNT
        );
        assert_eq!(payload.skin.palette.len(), 384);
        assert!(crate::skinning::flat_palette_is_identity(&payload.skin.palette));
        assert!(payload.skin.palette_is_identity);
        assert_eq!(payload.skin.bind_pose, "canonical_rest");

        // Os 16 B de skin do vértice já significam algo: nenhum vértice fica sem
        // osso, e o buffer continua múltiplo do stride de 72 B.
        assert!(mesh
            .vertices
            .iter()
            .all(|vertex| (vertex.weights.iter().sum::<f32>() - 1.0).abs() < 1e-5));
        let vertex_bytes = decode_buffer(&payload.vertex_buffer_base64, VERTEX_STRIDE_BYTES)
            .expect("o buffer de vértices do snapshot decodifica");
        assert_eq!(
            vertex_bytes.len(),
            mesh.vertices.len() * VERTEX_STRIDE_BYTES as usize
        );
    }

    #[test]
    fn fixture_deserializes_into_the_rust_contract_types() {
        // Guards the fixture against drifting away from the Rust structs: a
        // renamed/removed field or a wrong enum spelling fails here.
        let fixture = fixture_json();
        let snapshot: CoreSnapshot =
            serde_json::from_value(fixture.clone()).expect("fixture must match the Rust contract");

        assert_eq!(snapshot.snapshot_version, SNAPSHOT_FORMAT_VERSION);
        let static_payload = snapshot.static_payload.as_ref().expect("fixture has static payload");
        assert_eq!(static_payload.vertex_stride_bytes, VERTEX_STRIDE_BYTES);
        assert_eq!(static_payload.morph_delta_stride_bytes, MORPH_DELTA_STRIDE_BYTES);
        assert_eq!(static_payload.morph_channels.len(), 2);
        assert_eq!(static_payload.morph_channels[0].target.as_str(), "mrf_head_width");
        assert_eq!(
            static_payload.morph_channels[0].start_offset
                + static_payload.morph_channels[0].delta_count,
            static_payload.morph_channels[1].start_offset
        );

        // P1-04: o fixture carrega a paleta de skinning (a mesma que o
        // viewport/headless consomem no bloco `bones` do render contract).
        let skin = &static_payload.skin;
        assert_eq!(skin.bone_count as usize, crate::bone_sync::CANONICAL_JOINT_COUNT);
        assert_eq!(skin.palette.len(), skin.bone_count as usize * 16);
        assert_eq!(skin.palette.len(), 384);
        assert!(skin.proportions_baked);
        assert!(skin.palette_is_identity);
        assert_eq!(skin.bind_pose, "canonical_rest");
        assert!(crate::skinning::flat_palette_is_identity(&skin.palette));

        let dynamic = &snapshot.dynamic;
        assert_eq!(dynamic.project_id.as_str(), "prj_fixture");
        assert_eq!(dynamic.character_id.as_str(), "chr_canonical");
        assert_eq!(dynamic.base_gender, BaseGender::Male);
        assert_eq!(dynamic.morph_weights.len(), 2);
        assert_eq!(dynamic.lights[0].light_id.as_str(), "lgt_key");
        assert_eq!(dynamic.materials[0].material_id.as_str(), "mat_default_anime");
        assert_eq!(dynamic.nodes[0].node_id, "nod_character_base");
        assert_eq!(dynamic.render.msaa_samples, 4);
        assert_eq!(dynamic.deformation_authority, DeformationAuthority::ReferenceTs);
        assert_eq!(dynamic.deformation_coverage.total_sliders, 157);
        assert!(!dynamic.deformation_coverage.is_complete());
        assert!(!dynamic.deformation_coverage.bakes_gender);
        assert_eq!(dynamic.deformation_coverage.missing_sliders(), 0);

        // Round-tripping the fixture through Rust must be lossless.
        assert_eq!(snapshot.to_json().unwrap(), snapshot.to_json().unwrap());
        assert_eq!(
            CoreSnapshot::from_json(&snapshot.to_json().unwrap()).unwrap(),
            snapshot
        );
        assert_eq!(
            CoreSnapshot::from_json(&snapshot.to_json().unwrap())
                .unwrap()
                .to_json()
                .unwrap(),
            snapshot.to_json().unwrap()
        );
    }

    #[test]
    fn packed_vertex_buffer_round_trips_with_72_byte_stride() {
        let mesh = Mesh::create_canonical_base(BaseGender::Male);
        let bytes = pack_vertices(&mesh.vertices);
        assert_eq!(bytes.len(), mesh.vertices.len() * 72);

        // position (12) + normal (12) + uv (8) + color (16) + joints (8) + weights (16)
        let first = &mesh.vertices[0];
        let read_f32 = |offset: usize| {
            f32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ])
        };
        assert_eq!(read_f32(0), first.position[0]);
        assert_eq!(read_f32(12), first.normal[0]);
        assert_eq!(read_f32(24), first.uv[0]);
        assert_eq!(read_f32(32), first.color[0]);
        let joints = u16::from_le_bytes([bytes[48], bytes[49]]);
        assert_eq!(joints, first.joints[0]);
        assert_eq!(read_f32(56), first.weights[0]);
    }

    #[test]
    fn snapshots_are_deterministic_and_round_trip_through_json() {
        let mut project = ProjectState::default();
        project.set_morph_value("head_width", 1.2);
        let mesh = Mesh::create_canonical_base(project.character.base_gender);
        let catalog = MorphCatalog::new(project.character.base_gender);
        let snapshot = build_snapshot(
            &project,
            &mesh,
            &catalog.morph_set,
            7,
            3,
            true,
            crate::project::URI_BASE_MALE,
        );

        assert_eq!(snapshot.snapshot_version, SNAPSHOT_FORMAT_VERSION);
        assert_eq!(snapshot.dynamic.dynamic_revision, 7);
        assert_eq!(snapshot.dynamic.static_revision, 3);
        // The canonical somatotype (1/3, 1/3, 1/3) expands into three macro
        // slider weights, plus the explicit head_width override.
        assert_eq!(snapshot.dynamic.morph_weights.len(), 4);
        let head_width = snapshot
            .dynamic
            .morph_weights
            .iter()
            .find(|weight| weight.slider_id == "head_width")
            .expect("head_width override must be transmitted");
        assert!((head_width.weight - 0.2).abs() < 1e-6);
        assert!((head_width.value - 1.2).abs() < 1e-6);
        let endo = snapshot
            .dynamic
            .morph_weights
            .iter()
            .find(|weight| weight.slider_id == "somatotype_endomorph")
            .expect("somatotype must expand into macro sliders");
        assert!((endo.weight - 1.0 / 3.0).abs() < 1e-5);
        assert_eq!(
            snapshot.dynamic.deformation_authority,
            DeformationAuthority::Core,
            "the core model covers the canonical character; the payload says so"
        );

        let json = snapshot.to_json().unwrap();
        let parsed = CoreSnapshot::from_json(&json).unwrap();
        assert_eq!(parsed, snapshot);
        assert_eq!(parsed.to_json().unwrap(), json, "serialization is deterministic");

        let without_static = build_snapshot(
            &project,
            &mesh,
            &catalog.morph_set,
            8,
            3,
            false,
            crate::project::URI_BASE_MALE,
        );
        assert!(without_static.static_payload.is_none());
        assert_eq!(without_static.dynamic.dynamic_revision, 8);
    }

    #[test]
    fn static_payload_carries_every_canonical_channel() {
        let project = ProjectState::default();
        let mesh = Mesh::create_canonical_base(project.character.base_gender);
        let morph_set = build_canonical_sparse_morph_set(&mesh);
        let payload = build_static_payload(
            &mesh,
            &morph_set,
            &vec![0.0; morph_set.targets.len()],
            1,
            project.character.base_gender,
            crate::project::URI_BASE_MALE,
        );

        assert_eq!(payload.vertex_count, 4070);
        assert_eq!(payload.index_count, 20640);
        assert_eq!(payload.morph_channels.len(), ALL_MORPH_SLIDERS.len());
        assert!(!payload.catalog_fingerprint.is_empty());
        assert_eq!(payload.topology_hash.len(), 16);

        // Channel offsets are contiguous and consistent with the delta buffer.
        let mut expected_offset = 0;
        for channel in &payload.morph_channels {
            assert_eq!(channel.start_offset, expected_offset);
            expected_offset += channel.delta_count;
        }
        assert_eq!(expected_offset, payload.morph_total_deltas);
    }

    #[test]
    fn coverage_is_complete_for_the_canonical_catalog() {
        let mesh = Mesh::create_canonical_base(BaseGender::Female);
        let morph_set = build_canonical_sparse_morph_set(&mesh);
        let coverage = coverage_of(&morph_set);

        assert_eq!(coverage.total_sliders as usize, ALL_MORPH_SLIDERS.len());
        assert_eq!(coverage.morph_targets as usize, ALL_MORPH_SLIDERS.len());
        assert!(
            coverage.sliders_with_geometry <= coverage.total_sliders,
            "coverage can never exceed the catalog"
        );
        assert_eq!(
            sliders_without_geometry(&morph_set),
            Vec::<&str>::new(),
            "every canonical slider must produce geometry (P0: geometry-only model)"
        );
        assert_eq!(coverage.sliders_with_geometry, coverage.total_sliders);
        assert!(coverage.is_complete());
        assert_eq!(coverage.missing_sliders(), 0);
        assert_eq!(coverage.authority(), DeformationAuthority::Core);
        assert!((coverage.ratio() - 1.0).abs() < 1e-6);
        assert_eq!(coverage.proportion_policy_version, PROPORTION_POLICY_VERSION);
        assert_eq!(coverage.somatotype_policy_version, SOMATOTYPE_POLICY_VERSION);
    }

    #[test]
    fn snapshot_for_project_is_deterministic_and_carries_prepared_geometry() {
        let mut project = ProjectState::default();
        project.character.proportions.head_scale = 1.25;
        project.set_morph_value("jaw_v_line_taper", 0.75);
        let morph_set =
            build_canonical_sparse_morph_set(&Mesh::create_canonical_base(BaseGender::Male));

        let first = build_snapshot_for_project(
            &project,
            &morph_set,
            4,
            2,
            true,
            crate::project::URI_BASE_MALE,
        )
        .expect("snapshot must build");
        let second = build_snapshot_for_project(
            &project,
            &morph_set,
            4,
            2,
            true,
            crate::project::URI_BASE_MALE,
        )
        .expect("snapshot must build");
        assert_eq!(
            first.to_json().unwrap(),
            second.to_json().unwrap(),
            "the same project must always produce the same snapshot"
        );

        let payload = first.static_payload.expect("static payload requested");
        // Prepared geometry differs from the raw canonical base (head scaled).
        let raw = Mesh::create_canonical_base(BaseGender::Male);
        assert_ne!(payload.topology_hash, mesh_topology_hash(&raw));
        assert_eq!(payload.vertex_count, 4070);
        assert_eq!(
            first.dynamic.deformation_authority,
            DeformationAuthority::Core
        );
    }

    #[test]
    fn decode_rejects_buffers_with_wrong_alignment() {
        let bad = encode_buffer(&[1, 2, 3]);
        assert!(decode_buffer(&bad, 4).is_err());
        assert!(decode_f32_buffer("not-base64!!", 4).is_err());
    }

    #[test]
    fn mesh_and_catalog_fingerprints_are_stable() {
        let male = Mesh::create_canonical_base(BaseGender::Male);
        let female = Mesh::create_canonical_base(BaseGender::Female);
        assert_ne!(mesh_topology_hash(&male), mesh_topology_hash(&female));
        assert_eq!(mesh_topology_hash(&male), mesh_topology_hash(&male));
        assert_eq!(catalog_fingerprint(), catalog_fingerprint());
        assert_eq!(catalog_fingerprint().len(), 16);
    }
}
