pub mod bone_sync;
pub mod command;
pub mod deformation;
pub mod export;
pub mod hierarchy;
pub mod ids;
pub mod math;
pub mod mesh;
pub mod morph;
pub mod morph_catalog;
pub mod project;
pub mod scene;
pub mod skinning;
pub mod snapshot;
pub mod somatotype;
pub mod tactile;

pub use bone_sync::{BondJoint, BondProportions, BondSyncManager, CANONICAL_JOINT_COUNT};
pub use command::{
    execute_command, restore_from_log, AppliedCommand, ChangeScope, Command, CommandError,
    CommandHistory, CommandLog, CommandLogEntry, CommandOutcome, HistoryEntry, MaterialPatch,
    MeshPreset, ReplayedLog, Revision, COMMAND_LOG_VERSION,
};
pub use export::{
    buffer_checksum, build_export_bundle, manifest_from_json, ExportBundle, ExportDeformationInfo,
    ExportError, ExportGeometryInfo, ExportManifest, ExportProjectInfo, ExportRenderInfo,
    ExportRequest, ExportSkinInfo, EXPORT_FORMAT_VERSION, EXPORT_GENERATOR,
};
pub use deformation::{
    apply_proportions, canonical_base_mesh, catalog_weights, interpolate_gender, prepare_base_mesh,
    recompute_normals, vertical_bounds, DeformationError, DeformationInputs, HEAD_BAND_START,
    HIP_BAND, PROPORTION_POLICY_VERSION, SHOULDER_BAND, SOMATOTYPE_POLICY_VERSION,
};
pub use hierarchy::{
    ancestors_of, descendants_of, depth_of, hierarchy_order, reparent_checked,
    resolve_world_transforms, validate_hierarchy, HierarchyError, NodeKind,
};
pub use ids::{
    fnv1a64, is_valid_slug, normalize_uri, slugify, AnimationClipId, AssetId, CameraId, CharacterId,
    IdError, LightId, MaterialId, MorphId, NodeId, ProjectId, SceneId, StableId, ID_MAX_LEN,
};
pub use math::{
    Aabb, BoundingSphere, Camera, Frustum, FrustumPlane, OrthographicBounds, ProjectionError,
    ProjectionMode, Transform,
};
pub use mesh::{BaseGender, Mesh, Vertex};
pub use morph::{MorphChannel, MorphTarget, SparseMorphDelta, SparseMorphHeader, SparseMorphSet};
pub use morph_catalog::{
    ALL_MORPH_SLIDERS, AnatomicalZone, GenderDimorphism, MeshIntegrityReport, MorphCatalog,
    MorphSliderDef, SliderMechanism, build_canonical_sparse_morph_set, find_slider_def,
    get_sliders_for_zone,
};
pub use project::{
    migrate_legacy_snapshot, AccessoryAttachment, AnimationClip, AnimationState, AnimationTrack,
    AssetEntry, AssetKind, CameraSlot, CharacterProportions, CharacterState, ClothParameters,
    ColorManagement, ColorSpace, HairParameters, LengthUnit, LightSlot, MaterialEntry, MeshRef,
    NodeSlot, ProjectError, ProjectSettings, ProjectState, RenderState, SceneState,
    TonemapOperator, TransformKeyframe, DEFAULT_AUTOSAVE_INTERVAL_MINUTES, DEFAULT_HISTORY_LIMIT,
    PROJECT_SCHEMA_VERSION, URI_BASE_FEMALE, URI_BASE_MALE, URI_PRESET_CUBE, URI_PRESET_SPHERE,
};
pub use scene::{Scene, SceneNode, StylizedLight, StylizedMaterial};
pub use snapshot::{
    build_dynamic_payload, build_snapshot, build_snapshot_for_project, build_static_payload,
    catalog_fingerprint, coverage_of, sliders_without_geometry,
    decode_buffer, decode_f32_buffer, encode_buffer, mesh_topology_hash, pack_indices,
    pack_vertices, CameraSnapshot, CoreSnapshot, DeformationAuthority, DeformationCoverage,
    DynamicStatePayload, LightSnapshot, MaterialSnapshot, MorphChannelDescriptor, MorphWeight,
    NodeSnapshot, RenderSnapshot, SkinPayload, StaticGeometryPayload, MORPH_CHANNEL_STRIDE_BYTES,
    MORPH_DELTA_STRIDE_BYTES, SNAPSHOT_FORMAT_VERSION, VERTEX_STRIDE_BYTES,
};
pub use somatotype::{SomatotypeCoords, SomatotypeState, V_ECTO, V_ENDO, V_MESO};
pub use tactile::{
    AnatomicalSegment, Capsule, Ray, RaycastHit, TactileDragResult, TactileHullSet,
    project_tactile_drag,
};
