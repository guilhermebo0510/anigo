//! Morphs de macro e de rosto como alvos esparsos aditivos, mais as expressões VRM.
//!
//! Os alvos de forma (gênero, idade, peso, músculo, altura, cabeça, 18 parâmetros de rosto
//! em duas polaridades) são a diferença entre a geração base e a geração com o parâmetro movido:
//! o acoplamento entre macros é só o que o gerador já faz (por exemplo, peso engorda também as
//! bochechas). As expressões vêm de `expressions::deform`. Peças auxiliares (olhos, cílios,
//! sobrancelhas, interior da boca) recebem deltas densos por peça.

use crate::character::{skin_positions, BaseCharacter};
use crate::expressions::{deform, expression_specs};
use crate::params::{Params, FACE_PARAM_NAMES};
use crate::parts::{build_parts, Part};
use crate::topology::reference;
use anigo_core::morph::{SparseMorphDelta, SparseMorphSet};
use glam::Vec3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MorphGroup {
    Macro,
    Face,
    Expression,
}

#[derive(Debug, Clone)]
pub struct MorphTarget {
    pub name: String,
    pub group: MorphGroup,
    /// Deltas esparsos do Skin: (vértice, delta de posição).
    pub deltas: Vec<(u32, [f32; 3])>,
    /// Deltas densos por peça auxiliar (vazio = sem movimento), na ordem de `PART_NAMES`.
    pub part_deltas: Vec<Vec<[f32; 3]>>,
}

impl MorphTarget {
    pub fn max_delta(&self) -> f32 {
        self.deltas.iter().map(|(_, d)| Vec3::from_array(*d).length()).fold(0.0, f32::max)
    }
}

#[derive(Debug, Clone, Default)]
pub struct MorphSet {
    pub targets: Vec<MorphTarget>,
}

pub const EPS: f32 = 1e-6;

impl MorphSet {
    pub fn find(&self, name: &str) -> Option<usize> {
        self.targets.iter().position(|t| t.name == name)
    }

    pub fn names(&self) -> Vec<String> {
        self.targets.iter().map(|t| t.name.clone()).collect()
    }

    /// Aplica `base + soma(peso_i * delta_i)`.
    pub fn apply(&self, base: &[Vec3], weights: &[(&str, f32)]) -> Vec<Vec3> {
        let mut out = base.to_vec();
        for (name, w) in weights {
            if let Some(i) = self.find(name) {
                for (v, d) in &self.targets[i].deltas {
                    if let Some(p) = out.get_mut(*v as usize) {
                        *p += Vec3::from_array(*d) * *w;
                    }
                }
            }
        }
        out
    }

    /// Aplica os deltas densos às peças auxiliares.
    pub fn apply_parts(&self, base: &[Part], weights: &[(&str, f32)]) -> Vec<Part> {
        let mut out: Vec<Part> = base.to_vec();
        for (name, w) in weights {
            if let Some(i) = self.find(name) {
                for (pi, d) in self.targets[i].part_deltas.iter().enumerate() {
                    if d.is_empty() {
                        continue;
                    }
                    for (p, dd) in out[pi].positions.iter_mut().zip(d) {
                        *p += Vec3::from_array(*dd) * *w;
                    }
                }
            }
        }
        out
    }

    /// Exporta para o `SparseMorphSet` do core (normais zeradas; o renderer recompleta).
    pub fn to_core_sparse(&self) -> SparseMorphSet {
        let mut set = SparseMorphSet::new();
        for t in &self.targets {
            let deltas = t.deltas.iter().map(|(v, d)| SparseMorphDelta::new(*v, *d, [0.0, 0.0, 0.0])).collect();
            set.add_target(t.name.clone(), deltas);
        }
        set
    }
}

fn sparsify(base: &[Vec3], new: &[Vec3]) -> Vec<(u32, [f32; 3])> {
    base.iter()
        .zip(new)
        .enumerate()
        .filter_map(|(i, (b, n))| {
            let d = *n - *b;
            (d.length() > EPS).then(|| (i as u32, d.to_array()))
        })
        .collect()
}

fn part_diff(base: &[Part], new: &[Part]) -> Vec<Vec<[f32; 3]>> {
    base.iter()
        .zip(new)
        .map(|(b, n)| {
            let d: Vec<[f32; 3]> = b.positions.iter().zip(&n.positions).map(|(p, q)| (*q - *p).to_array()).collect();
            if d.iter().any(|v| Vec3::from_array(*v).length() > EPS) {
                d
            } else {
                Vec::new()
            }
        })
        .collect()
}

/// Definições dos morphs de forma: (nome, grupo, modificador de parâmetros).
pub fn shape_defs() -> Vec<(String, MorphGroup, Box<dyn Fn(&mut Params)>)> {
    let mut v: Vec<(String, MorphGroup, Box<dyn Fn(&mut Params)>)> = Vec::new();
    v.push(("gender_male".into(), MorphGroup::Macro, Box::new(|p| p.gender = 0.0)));
    v.push(("gender_female".into(), MorphGroup::Macro, Box::new(|p| p.gender = 1.0)));
    // idade: faixa ADULTA. 0 = jovem adulto (base), 0.5 = adulto, 1 = idoso. Não há alvo infantil.
    v.push(("age_adult".into(), MorphGroup::Macro, Box::new(|p| p.age = 0.5)));
    v.push(("age_elderly".into(), MorphGroup::Macro, Box::new(|p| p.age = 1.0)));
    v.push(("weight_heavy".into(), MorphGroup::Macro, Box::new(|p| p.weight = 1.0)));
    v.push(("weight_thin".into(), MorphGroup::Macro, Box::new(|p| p.weight = -1.0)));
    v.push(("muscle_high".into(), MorphGroup::Macro, Box::new(|p| p.muscle = 1.0)));
    v.push(("muscle_low".into(), MorphGroup::Macro, Box::new(|p| p.muscle = -1.0)));
    v.push(("height_tall".into(), MorphGroup::Macro, Box::new(|p| p.height = 1.0)));
    v.push(("height_short".into(), MorphGroup::Macro, Box::new(|p| p.height = -1.0)));
    v.push(("head_big".into(), MorphGroup::Macro, Box::new(|p| p.head_size = 1.0)));
    v.push(("head_small".into(), MorphGroup::Macro, Box::new(|p| p.head_size = -1.0)));
    for (i, name) in FACE_PARAM_NAMES.iter().enumerate() {
        v.push((format!("{name}_pos"), MorphGroup::Face, Box::new(move |p| p.face.set(i, 1.0))));
        v.push((format!("{name}_neg"), MorphGroup::Face, Box::new(move |p| p.face.set(i, -1.0))));
    }
    v
}

/// Conjunto completo de morphs do personagem base: macros, rosto e expressões.
pub fn build_morphs(ch: &BaseCharacter) -> MorphSet {
    let topo = reference();
    let base_parts = build_parts(topo, &ch.positions, &ch.positions, 0.0);
    let mut set = MorphSet::default();
    for (name, group, f) in shape_defs() {
        let mut p = ch.params;
        f(&mut p);
        let p = p.sanitized();
        let pos = skin_positions(&p);
        let parts = build_parts(topo, &pos, &pos, 0.0);
        set.targets.push(MorphTarget {
            name,
            group,
            deltas: sparsify(&ch.positions, &pos),
            part_deltas: part_diff(&base_parts, &parts),
        });
    }
    for (name, spec) in expression_specs() {
        let (def, jaw) = deform(topo, &ch.positions, &ch.skeleton, &spec);
        let parts = build_parts(topo, &ch.positions, &def, jaw);
        set.targets.push(MorphTarget {
            name: name.to_string(),
            group: MorphGroup::Expression,
            deltas: sparsify(&ch.positions, &def),
            part_deltas: part_diff(&base_parts, &parts),
        });
    }
    set
}
