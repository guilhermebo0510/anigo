//! ANIGO Real-Time Cloth and Tailoring Simulation Engine.
//!
//! Camada de AUTORIA da roupa (proxy, pintura de fixo/móvel, vínculo com a malha de render); a
//! simulação (XPBD com compliance, tether, backstop, angle restoration, colisão com o corpo) mora
//! em `anigo-physics` e é reexportada aqui.
//!
//! Os tipos antigos (`ClothParticle`, `DistanceConstraint` com `stiffness`, `ClothMesh`) seguem
//! existindo por compatibilidade, mas o `stiffness` por iteração depende de iterações e `dt`:
//! use [`ClothMesh::to_xpbd`] (que converte para compliance) ou, melhor, [`ClothProxyDef`].

use glam::Vec3;
use serde::{Deserialize, Serialize};

pub use anigo_physics::{
    apply_proxy_bindings, bind_to_proxy, cape_proxy, skirt_proxy, vertex_normals, ClothParams, ClothProxyDef, ClothSim,
    PhysicsPreset, ProxyBinding, VertexSkin,
};
use anigo_physics::{compliance_from_stiffness, XpbdConfig, XpbdSystem};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClothParticle {
    pub position: Vec3,
    pub prev_position: Vec3,
    pub acceleration: Vec3,
    pub inv_mass: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistanceConstraint {
    pub p1: usize,
    pub p2: usize,
    pub rest_length: f32,
    /// Rigidez artística 0..1 (1 = rígido). Convertida para compliance XPBD em [`ClothMesh::to_xpbd`].
    pub stiffness: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClothMesh {
    pub name: String,
    pub particles: Vec<ClothParticle>,
    pub constraints: Vec<DistanceConstraint>,
}

impl ClothMesh {
    /// Converte a malha antiga para um sistema XPBD (compliance a partir de `stiffness`).
    /// Partículas com `inv_mass <= 0` ficam fixas. Entradas inválidas são ignoradas.
    pub fn to_xpbd(&self, config: XpbdConfig) -> XpbdSystem {
        let mut sys = XpbdSystem::new(config);
        let mut map = Vec::with_capacity(self.particles.len());
        for p in &self.particles {
            let mass = if p.inv_mass.is_finite() && p.inv_mass > 0.0 { 1.0 / p.inv_mass } else { 0.0 };
            map.push(sys.add_particle(p.position, mass, 0.0));
        }
        for c in &self.constraints {
            if let (Some(Some(a)), Some(Some(b))) = (map.get(c.p1), map.get(c.p2)) {
                sys.add_distance(*a, *b, c.rest_length, compliance_from_stiffness(c.stiffness));
            }
        }
        sys.settle_here();
        sys
    }
}

/// Pinta os vértices fixos: `true` onde `positions[i].y >= min_y` (cintura/ombro). Atalho de autoria.
pub fn fixed_mask_above(positions: &[Vec3], min_y: f32) -> Vec<bool> {
    positions.iter().map(|p| p.is_finite() && p.y >= min_y).collect()
}

/// Pinta os vértices fixos a partir de uma lista de índices (grupo de vértices "cloth_attach_*").
pub fn fixed_mask_from_indices(vertex_count: usize, fixed: &[usize]) -> Vec<bool> {
    let mut mask = vec![false; vertex_count];
    for &i in fixed {
        if let Some(slot) = mask.get_mut(i) {
            *slot = true;
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_physics::{ColliderSet, PhysicsWorld};
    use glam::{Mat4, Quat};

    #[test]
    fn malha_antiga_vira_xpbd_e_pende() {
        let mesh = ClothMesh {
            name: "corda".into(),
            particles: (0..5)
                .map(|k| ClothParticle {
                    position: Vec3::new(0.0, -(k as f32) * 0.1, 0.0),
                    prev_position: Vec3::ZERO,
                    acceleration: Vec3::ZERO,
                    inv_mass: if k == 0 { 0.0 } else { 1.0 },
                })
                .collect(),
            constraints: (1..5).map(|k| DistanceConstraint { p1: k - 1, p2: k, rest_length: 0.1, stiffness: 1.0 }).collect(),
        };
        let mut sys = mesh.to_xpbd(XpbdConfig::default());
        let set = ColliderSet::new();
        for _ in 0..300 {
            sys.step(1.0 / 60.0, &set);
        }
        assert!((sys.pos[4].y + 0.4).abs() < 5e-3, "{:?}", sys.pos[4]);
        // restricoes invalidas sao ignoradas
        let mut bad = mesh.clone();
        bad.constraints.push(DistanceConstraint { p1: 0, p2: 99, rest_length: 0.1, stiffness: 1.0 });
        bad.constraints.push(DistanceConstraint { p1: 0, p2: 1, rest_length: f32::NAN, stiffness: 1.0 });
        let _ = bad.to_xpbd(XpbdConfig::default());
    }

    #[test]
    fn mascaras_de_vertices_fixos() {
        let pos = [Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.5, 0.0), Vec3::splat(f32::NAN)];
        assert_eq!(fixed_mask_above(&pos, 0.9), vec![true, false, false]);
        assert_eq!(fixed_mask_from_indices(3, &[0, 2, 77]), vec![true, false, true]);
    }

    #[test]
    fn saia_via_anigo_cloth_roda_no_physics_world() {
        let waist = Vec3::new(0.0, 0.9, 0.0);
        let def = skirt_proxy(waist, 0.19, 0.3, 0.45, 12, 4, 0);
        let rest = [Mat4::from_translation(waist)];
        let mut world = PhysicsWorld::default();
        let h = world.add_cloth(&def, &rest).unwrap();
        for f in 0..120 {
            let t = f as f32 / 60.0;
            let pose = Mat4::from_rotation_translation(Quat::from_rotation_y(t.sin()), waist);
            world.step(1.0 / 60.0, &[pose]);
        }
        assert!(world.cloth(h).unwrap().max_edge_stretch() < 0.08);
    }
}
