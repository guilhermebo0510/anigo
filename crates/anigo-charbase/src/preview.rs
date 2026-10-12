//! Pré-visualização SEM mudar o `render_contract`: a paleta de skinning do renderer continua com
//! 24 ossos. `collapse_to_canonical24` reduz os 65 ossos do personagem (55 humanoid + twist + busto)
//! aos 24 do núcleo (dedos -> mão, twist -> pai, olhos/mandíbula -> cabeça) só para visualizar e
//! gerar miniaturas. A decisão de levar o renderer a N ossos (storage buffer) é do usuário.

use crate::character::BaseCharacter;
use crate::parts::Part;
use crate::skeleton::Skeleton;
use crate::topology::reference;
use anigo_core::mesh::{Mesh, Vertex};
use glam::Vec3;

/// Nomes dos 24 ossos canônicos do núcleo (`BondSyncManager::create_canonical_humanoid`), na ordem.
pub const CANONICAL24: [&str; 24] = [
    "Root", "Hips", "Pelvis", "Spine", "Chest", "UpperChest", "Neck", "Head", "LeftClavicle", "LeftShoulder", "LeftElbow", "LeftWrist",
    "RightClavicle", "RightShoulder", "RightElbow", "RightWrist", "LeftThigh", "LeftKnee", "LeftAnkle", "LeftToes", "RightThigh", "RightKnee",
    "RightAnkle", "RightToes",
];

fn canon_index(name: &str) -> u16 {
    let find = |n: &str| CANONICAL24.iter().position(|c| *c == n).unwrap_or(1) as u16;
    // lado
    let (side, rest) = if let Some(r) = name.strip_prefix("tw_left") {
        ("Left", r)
    } else if let Some(r) = name.strip_prefix("tw_right") {
        ("Right", r)
    } else if let Some(r) = name.strip_prefix("left") {
        ("Left", r)
    } else if let Some(r) = name.strip_prefix("right") {
        ("Right", r)
    } else {
        ("", name)
    };
    match (side, rest) {
        ("", "hips") => find("Hips"),
        ("", "spine") => find("Spine"),
        ("", "chest") | ("", "bust_L") | ("", "bust_R") => find("Chest"),
        ("", "upperChest") => find("UpperChest"),
        ("", "neck") => find("Neck"),
        ("", "head") | ("", "jaw") => find("Head"),
        ("", n) if n.starts_with("bust") => find("Chest"),
        (s, "Eye") if !s.is_empty() => find("Head"),
        (s, "Shoulder") => find(&format!("{s}Clavicle")),
        (s, "UpperArm") => find(&format!("{s}Shoulder")),
        (s, "LowerArm") => find(&format!("{s}Elbow")),
        (s, n) if n == "Hand" || n.starts_with("Thumb") || n.starts_with("Index") || n.starts_with("Middle") || n.starts_with("Ring") || n.starts_with("Little") => {
            find(&format!("{s}Wrist"))
        }
        (s, "UpperLeg") => find(&format!("{s}Thigh")),
        (s, "LowerLeg") => find(&format!("{s}Knee")),
        (s, "Foot") => find(&format!("{s}Ankle")),
        (s, "Toes") => find(&format!("{s}Toes")),
        _ => find("Hips"),
    }
}

/// Reduz pesos de 65 ossos para os 24 canônicos (soma 1, no máximo 4 influências).
pub fn collapse_to_canonical24(skel: &Skeleton, weights: &[[(u16, f32); 4]]) -> Vec<[(u16, f32); 4]> {
    let map: Vec<u16> = skel.bones.iter().map(|b| canon_index(&b.name)).collect();
    weights
        .iter()
        .map(|w| {
            let mut acc: Vec<(u16, f32)> = Vec::new();
            for &(b, x) in w {
                if x <= 0.0 {
                    continue;
                }
                let c = map.get(b as usize).copied().unwrap_or(1);
                if let Some(e) = acc.iter_mut().find(|e| e.0 == c) {
                    e.1 += x;
                } else {
                    acc.push((c, x));
                }
            }
            acc.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
            let mut out = [(0u16, 0.0f32); 4];
            let sum: f32 = acc.iter().take(4).map(|e| e.1).sum();
            for (i, e) in acc.iter().take(4).enumerate() {
                out[i] = (e.0, e.1 / sum.max(1e-9));
            }
            if acc.is_empty() {
                out[0] = (1, 1.0);
            }
            out
        })
        .collect()
}

/// Malha de render (formato de 72 B do núcleo) do Skin, com a paleta colapsada em 24 ossos.
pub fn skin_core_mesh(ch: &BaseCharacter) -> Mesh {
    let topo = reference();
    let w24 = collapse_to_canonical24(&ch.skeleton, &topo.weights);
    let vertices: Vec<Vertex> = (0..topo.n_verts)
        .map(|v| {
            let w = w24[v];
            Vertex::with_skinning(
                ch.positions[v].to_array(),
                ch.normals[v].to_array(),
                [0.0, 0.0],
                [1.0, 0.5, 1.0, 1.0],
                [w[0].0, w[1].0, w[2].0, w[3].0],
                [w[0].1, w[1].1, w[2].1, w[3].1],
            )
        })
        .collect();
    let mut indices = Vec::with_capacity(topo.faces.len() * 6);
    for f in &topo.faces {
        indices.extend_from_slice(&[f[0], f[1], f[2], f[0], f[2], f[3]]);
    }
    Mesh::new("charbase_skin", vertices, indices)
}

/// Malha de render de uma peça auxiliar (dupla face), presa a um osso canônico.
pub fn part_core_mesh(part: &Part, canonical_bone: u16) -> Mesh {
    let mut normals = vec![Vec3::ZERO; part.positions.len()];
    for t in &part.tris {
        let (a, b, c) = (part.positions[t[0] as usize], part.positions[t[1] as usize], part.positions[t[2] as usize]);
        let n = (b - a).cross(c - a);
        for &i in t {
            normals[i as usize] += n;
        }
    }
    let vertices: Vec<Vertex> = part
        .positions
        .iter()
        .zip(&normals)
        .zip(&part.uv)
        .map(|((p, n), uv)| {
            Vertex::with_skinning(
                p.to_array(),
                n.normalize_or_zero().to_array(),
                *uv,
                [1.0, 0.5, 0.0, 1.0],
                [canonical_bone, 0, 0, 0],
                [1.0, 0.0, 0.0, 0.0],
            )
        })
        .collect();
    let mut indices = Vec::with_capacity(part.tris.len() * 6);
    for t in &part.tris {
        indices.extend_from_slice(&[t[0], t[1], t[2], t[0], t[2], t[1]]);
    }
    Mesh::new(format!("charbase_{}", part.name), vertices, indices)
}

/// Osso canônico (índice) de cada peça auxiliar.
pub fn part_bone24(part_name: &str) -> u16 {
    match part_name {
        "teeth_lower" | "tongue" => 7,
        _ => 7,
    }
}
