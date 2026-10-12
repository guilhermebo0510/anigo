//! Colliders do corpo: cápsulas ao longo dos 24 ossos do esqueleto canônico do ANIGO.
//!
//! O crate não depende de `anigo-core`; os índices abaixo espelham `anigo_core::skinning::bone`
//! (e o teste de integração `tests/canonical_skeleton.rs` confere nome a nome contra o
//! `BondSyncManager` canônico). A paleta de 24 ossos é congelada pelo contrato do renderer.

use glam::{Mat4, Vec3};

use crate::collider::{Collider, ColliderShape};
use crate::error::{check_index, PhysicsError, PhysicsResult};

/// Índices dos 24 ossos canônicos (mesma ordem de `BondSyncManager::create_canonical_humanoid`).
pub mod bone {
    pub const ROOT: usize = 0;
    pub const HIPS: usize = 1;
    pub const PELVIS: usize = 2;
    pub const SPINE: usize = 3;
    pub const CHEST: usize = 4;
    pub const UPPER_CHEST: usize = 5;
    pub const NECK: usize = 6;
    pub const HEAD: usize = 7;
    pub const LEFT_CLAVICLE: usize = 8;
    pub const LEFT_SHOULDER: usize = 9;
    pub const LEFT_ELBOW: usize = 10;
    pub const LEFT_WRIST: usize = 11;
    pub const RIGHT_CLAVICLE: usize = 12;
    pub const RIGHT_SHOULDER: usize = 13;
    pub const RIGHT_ELBOW: usize = 14;
    pub const RIGHT_WRIST: usize = 15;
    pub const LEFT_THIGH: usize = 16;
    pub const LEFT_KNEE: usize = 17;
    pub const LEFT_ANKLE: usize = 18;
    pub const LEFT_TOES: usize = 19;
    pub const RIGHT_THIGH: usize = 20;
    pub const RIGHT_KNEE: usize = 21;
    pub const RIGHT_ANKLE: usize = 22;
    pub const RIGHT_TOES: usize = 23;
    pub const COUNT: usize = 24;

    /// Nomes na ordem canônica (para conferência).
    pub const NAMES: [&str; COUNT] = [
        "Root", "Hips", "Pelvis", "Spine", "Chest", "UpperChest", "Neck", "Head", "LeftClavicle", "LeftShoulder",
        "LeftElbow", "LeftWrist", "RightClavicle", "RightShoulder", "RightElbow", "RightWrist", "LeftThigh", "LeftKnee",
        "LeftAnkle", "LeftToes", "RightThigh", "RightKnee", "RightAnkle", "RightToes",
    ];
}

/// Extremidade de uma cápsula.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CapsuleEnd {
    /// Posição (em repouso) da junta de índice dado, expressa no espaço do osso da cápsula.
    Joint(usize),
    /// Posição fixa no espaço local do osso da cápsula.
    Local(Vec3),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyCapsuleSpec {
    pub name: &'static str,
    /// Osso a que a cápsula está presa (ela é rígida com este osso).
    pub bone: usize,
    pub from: CapsuleEnd,
    pub to: CapsuleEnd,
    /// Raio em metros, para um corpo de altura ~1.65 m (escalável).
    pub radius: f32,
}

/// Cápsulas padrão do corpo: cabeça, pescoço, tronco, quadril, ombros, braços, mãos, pernas e pés.
pub fn canonical_body_specs() -> Vec<BodyCapsuleSpec> {
    use bone::*;
    use CapsuleEnd::{Joint, Local};
    let s = |name, bone, from, to, radius| BodyCapsuleSpec { name, bone, from, to, radius };
    vec![
        s("head", HEAD, Local(Vec3::new(0.0, 0.04, 0.0)), Local(Vec3::new(0.0, 0.12, 0.0)), 0.095),
        s("neck", NECK, Local(Vec3::ZERO), Joint(HEAD), 0.045),
        s("upper_chest", UPPER_CHEST, Local(Vec3::ZERO), Joint(NECK), 0.13),
        s("chest", CHEST, Local(Vec3::ZERO), Joint(UPPER_CHEST), 0.13),
        s("spine", SPINE, Local(Vec3::ZERO), Joint(CHEST), 0.12),
        s("belly", HIPS, Local(Vec3::ZERO), Joint(SPINE), 0.12),
        s("hips", HIPS, Joint(LEFT_THIGH), Joint(RIGHT_THIGH), 0.11),
        s("shoulder_line", UPPER_CHEST, Joint(LEFT_SHOULDER), Joint(RIGHT_SHOULDER), 0.05),
        s("left_upper_arm", LEFT_SHOULDER, Local(Vec3::ZERO), Joint(LEFT_ELBOW), 0.045),
        s("left_forearm", LEFT_ELBOW, Local(Vec3::ZERO), Joint(LEFT_WRIST), 0.035),
        s("left_hand", LEFT_WRIST, Local(Vec3::ZERO), Local(Vec3::new(0.08, 0.0, 0.0)), 0.035),
        s("right_upper_arm", RIGHT_SHOULDER, Local(Vec3::ZERO), Joint(RIGHT_ELBOW), 0.045),
        s("right_forearm", RIGHT_ELBOW, Local(Vec3::ZERO), Joint(RIGHT_WRIST), 0.035),
        s("right_hand", RIGHT_WRIST, Local(Vec3::ZERO), Local(Vec3::new(-0.08, 0.0, 0.0)), 0.035),
        s("left_thigh", LEFT_THIGH, Local(Vec3::ZERO), Joint(LEFT_KNEE), 0.08),
        s("left_shin", LEFT_KNEE, Local(Vec3::ZERO), Joint(LEFT_ANKLE), 0.055),
        s("left_foot", LEFT_ANKLE, Local(Vec3::ZERO), Joint(LEFT_TOES), 0.045),
        s("right_thigh", RIGHT_THIGH, Local(Vec3::ZERO), Joint(RIGHT_KNEE), 0.08),
        s("right_shin", RIGHT_KNEE, Local(Vec3::ZERO), Joint(RIGHT_ANKLE), 0.055),
        s("right_foot", RIGHT_ANKLE, Local(Vec3::ZERO), Joint(RIGHT_TOES), 0.045),
    ]
}

/// Constrói os colliders (cápsulas presas aos ossos) a partir da pose de REPOUSO em coordenadas de
/// mundo (`rest_world[i]` = matriz do osso `i`, 24 entradas). `radius_scale` escala os raios
/// (sliders de corpo: gordo/magro). Cada cápsula recebe o atrito `friction`.
pub fn build_body_colliders(rest_world: &[Mat4], radius_scale: f32, friction: f32) -> PhysicsResult<Vec<(&'static str, Collider)>> {
    if !(radius_scale.is_finite() && radius_scale > 0.0) {
        return Err(PhysicsError::BadValue { what: "radius_scale".into() });
    }
    if rest_world.len() < bone::COUNT {
        return Err(PhysicsError::BadIndex { what: "rest_world", index: bone::COUNT, len: rest_world.len() });
    }
    if rest_world.iter().any(|m| !m.is_finite()) {
        return Err(PhysicsError::BadValue { what: "rest_world".into() });
    }
    let mut out = Vec::new();
    for spec in canonical_body_specs() {
        check_index("body.bone", spec.bone, rest_world.len())?;
        let inv = rest_world[spec.bone].inverse();
        let resolve = |end: CapsuleEnd| -> PhysicsResult<Vec3> {
            match end {
                CapsuleEnd::Local(v) => Ok(v),
                CapsuleEnd::Joint(j) => {
                    check_index("body.joint", j, rest_world.len())?;
                    Ok(inv.transform_point3(rest_world[j].w_axis.truncate()))
                }
            }
        };
        let (a, b) = (resolve(spec.from)?, resolve(spec.to)?);
        let mut collider = Collider::on_bone(spec.bone, ColliderShape::Capsule { a, b, radius: spec.radius * radius_scale });
        collider.friction = friction;
        out.push((spec.name, collider));
    }
    Ok(out)
}
