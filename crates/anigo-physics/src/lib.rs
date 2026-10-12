//! ANIGO physics: movimento secundário em tempo real (cabelo, roupa, acessórios), Rust puro,
//! determinístico e testável sem GPU.
//!
//! * [`spring_bone`]: spring bone EXATO do VRM (`VRMC_springBone-1.0`), Verlet + colliders.
//! * [`xpbd`]: núcleo XPBD (compliance, sub-passos, distância/direção/anexo/tether/limites/colisão).
//! * [`hair`]: cadeias de cabelo/acessórios sobre o XPBD (rigidez decrescente raiz -> ponta).
//! * [`cloth`]: roupa por malha-proxy (depth, tether, backstop, angle restoration) + geradores.
//! * [`binding`]: proxy -> malha de render (3 vértices + baricêntricas + offset) e cards <-> cadeias.
//! * [`body`]: cápsulas do corpo nos 24 ossos canônicos.
//! * [`world`]: `PhysicsWorld`, a interface única (passo fixo, pausa, reset, serde do projeto).
//! * [`preset`]: parâmetros típicos de anime.
//!
//! Convenções: metros, Y para cima, mão direita; `glam` f32; nenhuma API entra em pânico com
//! NaN/`dt <= 0`/índices inválidos (construção devolve `Result`, `step` descarta e reporta).

pub mod binding;
pub mod body;
pub mod cloth;
pub mod collider;
pub mod error;
pub mod hair;
pub mod math;
pub mod preset;
pub mod skeleton;
pub mod spring_bone;
pub mod stepper;
pub mod world;
pub mod xpbd;

pub use binding::{apply_chain_bindings, apply_proxy_bindings, bind_to_chains, bind_to_proxy, ChainBinding, ChainFrames, ProxyBinding};
pub use body::{build_body_colliders, canonical_body_specs, BodyCapsuleSpec, CapsuleEnd};
pub use cloth::{cape_proxy, skirt_proxy, vertex_normals, ClothParams, ClothProxyDef, ClothSim, VertexSkin};
pub use collider::{Collider, ColliderSet, ColliderShape};
pub use error::{PhysicsError, PhysicsResult};
pub use hair::{compliance_from_stiffness, demo_strands, ChainGuide, HairChainDef, HairGroupDef, HairParams, HairSim};
pub use preset::PhysicsPreset;
pub use skeleton::{Skeleton, SkeletonNode};
pub use spring_bone::{
    SpringBoneDef, SpringBoneSystem, SpringChainDef, SpringColliderDef, SpringColliderGroupDef, SpringJointParams,
};
pub use stepper::{FixedStepper, StepPlan};
pub use world::{
    AccessoryHandle, BodyColliderSetup, ClothHandle, HairHandle, PhysicsSetup, PhysicsWorld, SpringHandle, StepReport,
    WorldConfig,
};
pub use xpbd::{ParticleLimit, XpbdConfig, XpbdStats, XpbdSystem};
