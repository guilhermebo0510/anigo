//! `BaseCharacter`: a geração completa (Skin, esqueleto) para um conjunto de `Params`.

use crate::cage::build_cage;
use crate::params::{Gender, Params};
use crate::skeleton::{bone_defs, Skeleton};
use crate::subdiv::catmull_clark;
use crate::topology::{quad_mesh_of, reference, symmetrize, Topology};
use glam::Vec3;

#[derive(Debug, Clone)]
pub struct BaseCharacter {
    pub params: Params,
    /// Posições do Skin (mesmos índices do contrato de topologia).
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub skeleton: Skeleton,
}

impl BaseCharacter {
    pub fn topology(&self) -> &'static Topology {
        reference()
    }
}

/// Posições do Skin para `params` (só a forma: a topologia é a de referência).
pub fn skin_positions(params: &Params) -> Vec<Vec3> {
    let topo = reference();
    let cage = build_cage(params);
    let refined = catmull_clark(&cage.mesh);
    debug_assert_eq!(refined.mesh.faces, topo.faces, "a topologia não pode depender dos parâmetros");
    let mut pos = refined.mesh.pos;
    symmetrize(&mut pos, &topo.mirror);
    pos
}

/// Gera o personagem para um gênero (masculino, feminino ou mistura contínua).
pub fn generate(g: Gender) -> BaseCharacter {
    generate_with(&Params::for_gender(g))
}

pub fn generate_with(params: &Params) -> BaseCharacter {
    let p = params.sanitized();
    let topo = reference();
    let positions = skin_positions(&p);
    let normals = quad_mesh_of(topo, &positions).vertex_normals();
    let skeleton = Skeleton::derive(&bone_defs(topo), &positions);
    BaseCharacter {
        params: p,
        positions,
        normals,
        skeleton,
    }
}
