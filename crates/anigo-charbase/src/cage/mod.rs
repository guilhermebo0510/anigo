//! A "gaiola" (cage): malha só de quads, de baixa resolução, que define a topologia e a forma
//! aproximada do personagem. O Skin final é a gaiola subdividida uma vez (Catmull-Clark), o
//! que dobra os anéis (articulações, olhos, boca...) e deixa a superfície orgânica.
//!
//! A topologia é independente dos parâmetros: só as posições mudam. Isso é o que torna
//! masculino/feminino isomórficos e os morphs de macro deltas esparsos sobre os mesmos índices.

mod head;
mod limbs;
mod trunk;

use crate::dims::Dims;
use crate::params::Params;
use crate::qmesh::QuadMesh;
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Região grossa do corpo de cada vértice (usada para pesos, grupos e UV).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum Region {
    Head = 0,
    Neck,
    Trunk,
    Arm,
    Hand,
    Thumb,
    Index,
    Middle,
    Ring,
    Little,
    Leg,
    Foot,
    Toes,
}

impl Region {
    pub const ALL: [Region; 13] = [
        Region::Head,
        Region::Neck,
        Region::Trunk,
        Region::Arm,
        Region::Hand,
        Region::Thumb,
        Region::Index,
        Region::Middle,
        Region::Ring,
        Region::Little,
        Region::Leg,
        Region::Foot,
        Region::Toes,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Region::Head => "head",
            Region::Neck => "neck",
            Region::Trunk => "trunk",
            Region::Arm => "arm",
            Region::Hand => "hand",
            Region::Thumb => "thumb",
            Region::Index => "index",
            Region::Middle => "middle",
            Region::Ring => "ring",
            Region::Little => "little",
            Region::Leg => "leg",
            Region::Foot => "foot",
            Region::Toes => "toes",
        }
    }

    /// Região que prevalece num ponto de aresta/face entre duas regiões.
    pub fn merge(a: Region, b: Region) -> Region {
        if a == b {
            return a;
        }
        // limbos e dedos vencem o tronco/cabeça; dedos vencem a mão
        let rank = |r: Region| -> u8 {
            match r {
                Region::Trunk => 0,
                Region::Neck => 1,
                Region::Head => 2,
                Region::Arm | Region::Leg => 3,
                Region::Hand | Region::Foot => 4,
                Region::Thumb | Region::Index | Region::Middle | Region::Ring | Region::Little | Region::Toes => 5,
            }
        };
        if rank(a) >= rank(b) {
            a
        } else {
            b
        }
    }
}

/// Laço (ciclo ou cadeia) de vértices da gaiola com nome estável.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopDef {
    pub name: String,
    pub verts: Vec<u32>,
    pub closed: bool,
}

pub struct Cage {
    pub mesh: QuadMesh,
    pub region: Vec<Region>,
    pub loops: Vec<LoopDef>,
    /// Conjuntos nomeados de vértices (âncoras de articulação, landmarks, máscaras).
    pub marks: BTreeMap<String, Vec<u32>>,
    /// Atributos escalares por vértice da gaiola: [linha do tronco, sin(theta), cos(theta), anel do dedo].
    pub attr: Vec<[f32; 4]>,
    pub dims: Dims,
}

/// Construtor interno: acumula vértices/faces/regiões/laços.
pub(crate) struct B {
    pub pos: Vec<Vec3>,
    pub faces: Vec<[u32; 4]>,
    pub region: Vec<Region>,
    pub attr: Vec<[f32; 4]>,
    pub loops: Vec<LoopDef>,
    pub marks: BTreeMap<String, Vec<u32>>,
}

impl B {
    pub fn new() -> Self {
        B {
            pos: Vec::new(),
            faces: Vec::new(),
            region: Vec::new(),
            attr: Vec::new(),
            loops: Vec::new(),
            marks: BTreeMap::new(),
        }
    }

    pub fn vert(&mut self, p: Vec3, r: Region) -> u32 {
        self.pos.push(p);
        self.region.push(r);
        self.attr.push([0.0; 4]);
        (self.pos.len() - 1) as u32
    }

    pub fn set_attr(&mut self, id: u32, slot: usize, v: f32) {
        self.attr[id as usize][slot] = v;
    }

    pub fn ring(&mut self, pts: &[Vec3], r: Region) -> Vec<u32> {
        pts.iter().map(|&p| self.vert(p, r)).collect()
    }

    pub fn quad(&mut self, a: u32, b: u32, c: u32, d: u32) {
        self.faces.push([a, b, c, d]);
    }

    /// Ponte de quads entre dois anéis fechados de mesmo tamanho.
    pub fn bridge(&mut self, a: &[u32], b: &[u32]) {
        assert_eq!(a.len(), b.len(), "bridge: anéis de tamanhos diferentes");
        let n = a.len();
        for i in 0..n {
            let j = (i + 1) % n;
            self.quad(a[i], a[j], b[j], b[i]);
        }
    }

    pub fn add_loop(&mut self, name: impl Into<String>, verts: Vec<u32>, closed: bool) {
        self.loops.push(LoopDef {
            name: name.into(),
            verts,
            closed,
        });
    }

    pub fn mark(&mut self, name: impl Into<String>, verts: Vec<u32>) {
        self.marks.insert(name.into(), verts);
    }

    pub fn centroid(&self, ids: &[u32]) -> Vec3 {
        ids.iter().map(|&i| self.pos[i as usize]).sum::<Vec3>() / ids.len().max(1) as f32
    }

    pub fn positions(&self, ids: &[u32]) -> Vec<Vec3> {
        ids.iter().map(|&i| self.pos[i as usize]).collect()
    }
}

/// Permutação `perm` tal que `cand[perm[i]]` fica mais perto de `reference[i]`
/// (testa todas as rotações nos dois sentidos; empate resolve para a menor rotação).
pub(crate) fn best_perm(reference: &[Vec3], cand: &[Vec3]) -> Vec<usize> {
    let n = reference.len();
    assert_eq!(n, cand.len());
    let mut best = (f32::MAX, 0usize, 1i32);
    for dir in [1i32, -1i32] {
        for s in 0..n {
            let mut e = 0.0f32;
            for i in 0..n {
                let k = ((s as i32 + dir * i as i32).rem_euclid(n as i32)) as usize;
                e += (reference[i] - cand[k]).length_squared();
            }
            if e < best.0 - 1e-12 {
                best = (e, s, dir);
            }
        }
    }
    (0..n)
        .map(|i| ((best.1 as i32 + best.2 * i as i32).rem_euclid(n as i32)) as usize)
        .collect()
}

/// Gera a gaiola completa. Determinístico.
pub fn build_cage(params: &Params) -> Cage {
    let dims = Dims::new(params);
    let mut b = B::new();
    let head_out = head::build_head(&mut b, &dims);
    let trunk_out = trunk::build_trunk(&mut b, &dims, &head_out);
    limbs::build_arms(&mut b, &dims, &trunk_out);
    limbs::build_legs(&mut b, &dims, &trunk_out);

    let mut mesh = QuadMesh {
        pos: b.pos,
        faces: b.faces,
    };
    // remove vértices órfãos (interior dos buracos) e remapeia tudo
    let remap = mesh.compact();
    let mut region = vec![Region::Head; mesh.pos.len()];
    let mut attr = vec![[0.0f32; 4]; mesh.pos.len()];
    for (old, &new) in remap.iter().enumerate() {
        if new != u32::MAX {
            region[new as usize] = b.region[old];
            attr[new as usize] = b.attr[old];
        }
    }
    let fix = |v: &Vec<u32>| -> Vec<u32> { v.iter().filter_map(|&i| (remap[i as usize] != u32::MAX).then(|| remap[i as usize])).collect() };
    let loops: Vec<LoopDef> = b
        .loops
        .iter()
        .map(|l| LoopDef {
            name: l.name.clone(),
            verts: fix(&l.verts),
            closed: l.closed,
        })
        .collect();
    let marks: BTreeMap<String, Vec<u32>> = b.marks.iter().map(|(k, v)| (k.clone(), fix(v))).collect();
    mesh.orient_consistently().expect("gaiola não-manifold");
    Cage {
        mesh,
        region,
        loops,
        marks,
        attr,
        dims,
    }
}
