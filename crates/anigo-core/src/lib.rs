pub mod bone_sync;
pub mod math;
pub mod mesh;
pub mod morph;
pub mod morph_catalog;
pub mod scene;
pub mod somatotype;
pub mod tactile;

pub use bone_sync::{BondJoint, BondProportions, BondSyncManager};
pub use math::{Camera, Transform};
pub use mesh::{BaseGender, Mesh, Vertex};
pub use morph::{MorphChannel, MorphTarget, SparseMorphDelta, SparseMorphHeader, SparseMorphSet};
pub use morph_catalog::{
    ALL_MORPH_SLIDERS, AnatomicalZone, GenderDimorphism, MeshIntegrityReport, MorphCatalog,
    MorphSliderDef, SliderMechanism, build_canonical_sparse_morph_set, find_slider_def,
    get_sliders_for_zone,
};
pub use scene::{Scene, SceneNode, StylizedLight, StylizedMaterial};
pub use somatotype::{SomatotypeCoords, SomatotypeState, V_ECTO, V_ENDO, V_MESO};
pub use tactile::{
    AnatomicalSegment, Capsule, Ray, RaycastHit, TactileDragResult, TactileHullSet,
    project_tactile_drag,
};
