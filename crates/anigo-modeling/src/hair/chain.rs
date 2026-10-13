//! Física do cabelo: grupos de bounce (como o "Hair Bounce" do VRoid) e cadeias neutras para o motor
//! de física.
//!
//! [`HairChainSpec`] é um tipo NEUTRO, sem dependência de nenhum crate de física: uma cadeia de
//! partículas/ossos por mecha. Quem simula (ex.: uma futura `anigo-physics`) converte cada spec em
//! `SpringBone`/cadeia própria. Contrato:
//!
//! * `points`: posições no **espaço local do objeto-alvo** (a cabeça), da raiz para a ponta.
//!   `points[0]` é o **ponto fixo** (âncora presa em `root_node_hint`, nunca simulado); `bones` ossos
//!   → `bones + 1` pontos. O trecho da guia antes do ponto fixo é rígido (segue a cabeça).
//! * `radius`: raio de colisão (hit radius) das partículas.
//! * `stiffness`, `drag`, `gravity`: 0..1, no sentido do VRoid/`VRMC_springBone`.
//! * A direção dos pontos segue a do traço (raiz → ponta), como no VRoid.
//!
//! Diferente do VRoid, as cadeias são DERIVADAS das guias a cada chamada de
//! [`HairSet::generate_chains`]: editar o penteado depois não deixa os ossos desatualizados.

use super::{HairSet, StrandRef};
use crate::error::{ModelError, ModelResult};
use glam::Vec3;
use serde::{Deserialize, Serialize};

pub const MAX_BOUNCE_GROUPS: usize = 30;
pub const MAX_BONES: u32 = 16;

/// Grupo de bounce: mechas que compartilham os parâmetros de física (faixas do VRoid).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BounceGroup {
    pub id: u32,
    pub name: String,
    pub strands: Vec<StrandRef>,
    /// Ossos por cadeia (1..=16).
    pub bones: u32,
    /// Fração do comprimento (0.01..=0.99) onde o primeiro osso começa; antes dela o cabelo é rígido.
    pub fixed_point: f32,
    pub stiffness: f32,
    pub drag: f32,
    pub gravity: f32,
    /// Raio de colisão (0..=0.1).
    pub hit_radius: f32,
}

impl BounceGroup {
    pub fn new(id: u32, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            strands: Vec::new(),
            bones: 4,
            fixed_point: 0.2,
            stiffness: 0.5,
            drag: 0.4,
            gravity: 0.3,
            hit_radius: 0.02,
        }
    }

    pub fn validate(&self) -> ModelResult<()> {
        if !(1..=MAX_BONES).contains(&self.bones) {
            return Err(ModelError::InvalidParameter(format!("bones fora de 1..={MAX_BONES}")));
        }
        if !self.fixed_point.is_finite() || !(0.01..=0.99).contains(&self.fixed_point) {
            return Err(ModelError::InvalidParameter("fixed_point fora de 0.01..=0.99".into()));
        }
        for (v, what) in [(self.stiffness, "stiffness"), (self.drag, "drag"), (self.gravity, "gravity")] {
            if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                return Err(ModelError::InvalidParameter(format!("{what} fora de [0, 1]")));
            }
        }
        if !self.hit_radius.is_finite() || !(0.0..=0.1).contains(&self.hit_radius) {
            return Err(ModelError::InvalidParameter("hit_radius fora de [0, 0.1]".into()));
        }
        Ok(())
    }
}

/// Cadeia neutra de partículas/ossos de UMA mecha (ver o contrato no topo do módulo).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HairChainSpec {
    /// Nome estável: `"<grupo>/<mecha id>"`.
    pub name: String,
    /// Nó/osso em que a cadeia se prende (dica; vem de [`HairSet::root_node_hint`]).
    pub root_node_hint: String,
    /// `bones + 1` pontos, da âncora (ponto fixo) à ponta, no espaço local do alvo.
    pub points: Vec<Vec3>,
    pub radius: f32,
    pub stiffness: f32,
    pub drag: f32,
    pub gravity: f32,
    pub fixed_point: f32,
    /// Grupo de bounce de origem.
    pub bounce_group: u32,
    /// Mecha de origem.
    pub source: StrandRef,
}

impl HairSet {
    /// Cadeias de física de todas as mechas que pertencem a algum grupo de bounce.
    pub fn generate_chains(&self) -> ModelResult<Vec<HairChainSpec>> {
        let mut out = Vec::new();
        for b in &self.bounce {
            b.validate()?;
            for r in &b.strands {
                let Some(strand) = self.strand(*r) else {
                    continue;
                };
                let total = strand.length();
                if !(total > 1e-6) {
                    continue;
                }
                // pontos uniformes em arco de s0 = fixed_point·L até L
                let n = b.bones as usize + 1;
                let dense = strand.guide.sample_uniform(256)?;
                let at = |s: f32| -> Vec3 {
                    let x = (s / total).clamp(0.0, 1.0) * (dense.len() - 1) as f32;
                    let i = (x.floor() as usize).min(dense.len() - 2);
                    dense[i].pos.lerp(dense[i + 1].pos, x - i as f32)
                };
                let s0 = b.fixed_point * total;
                let points: Vec<Vec3> = (0..n).map(|k| at(s0 + (total - s0) * k as f32 / (n - 1) as f32)).collect();
                out.push(HairChainSpec {
                    name: format!("{}/{}", r.group, r.strand),
                    root_node_hint: self.root_node_hint.clone(),
                    points,
                    radius: b.hit_radius,
                    stiffness: b.stiffness,
                    drag: b.drag,
                    gravity: b.gravity,
                    fixed_point: b.fixed_point,
                    bounce_group: b.id,
                    source: *r,
                });
            }
        }
        Ok(out)
    }

    /// Todas as mechas do penteado, na ordem dos grupos.
    pub fn all_strand_refs(&self) -> Vec<StrandRef> {
        self.groups
            .iter()
            .flat_map(|g| g.strands.iter().map(move |s| StrandRef { group: g.id, strand: s.id }))
            .collect()
    }

    /// "Auto-generate Bone Group" do VRoid: agrupa as mechas por proximidade das raízes em
    /// `n_groups` (1..=30) grupos — farthest-point + 6 iterações de k-means, determinístico.
    /// Substitui os grupos de bounce atuais. Devolve quantos grupos foram criados (≤ nº de mechas).
    pub fn auto_bounce(&mut self, n_groups: usize, bones: u32) -> ModelResult<usize> {
        if !(1..=MAX_BOUNCE_GROUPS).contains(&n_groups) {
            return Err(ModelError::InvalidParameter(format!("n_groups fora de 1..={MAX_BOUNCE_GROUPS}")));
        }
        let probe = BounceGroup {
            bones,
            ..BounceGroup::new(0, "")
        };
        probe.validate()?;
        let refs = self.all_strand_refs();
        if refs.is_empty() {
            return Err(ModelError::EmptySelection("sem mechas para agrupar"));
        }
        let roots: Vec<Vec3> = refs.iter().map(|r| self.strand(*r).map(|s| s.root()).unwrap_or(Vec3::ZERO)).collect();
        let k = n_groups.min(refs.len());
        // farthest-point: começa pela raiz mais alta (maior y, depois menor índice)
        let first = (0..roots.len()).fold(0, |best, i| if roots[i].y > roots[best].y + 1e-9 { i } else { best });
        let mut centers = vec![roots[first]];
        while centers.len() < k {
            let mut best = (0usize, -1.0f32);
            for (i, r) in roots.iter().enumerate() {
                let d = centers.iter().map(|c| c.distance_squared(*r)).fold(f32::INFINITY, f32::min);
                if d > best.1 + 1e-12 {
                    best = (i, d);
                }
            }
            centers.push(roots[best.0]);
        }
        let assign = |centers: &[Vec3]| -> Vec<usize> {
            roots
                .iter()
                .map(|r| {
                    let mut best = (0usize, f32::INFINITY);
                    for (c, cp) in centers.iter().enumerate() {
                        let d = cp.distance_squared(*r);
                        if d < best.1 - 1e-12 {
                            best = (c, d);
                        }
                    }
                    best.0
                })
                .collect()
        };
        let mut owner = assign(&centers);
        for _ in 0..6 {
            let mut sum = vec![Vec3::ZERO; k];
            let mut cnt = vec![0usize; k];
            for (i, &o) in owner.iter().enumerate() {
                sum[o] += roots[i];
                cnt[o] += 1;
            }
            for c in 0..k {
                if cnt[c] > 0 {
                    centers[c] = sum[c] / cnt[c] as f32;
                }
            }
            owner = assign(&centers);
        }
        self.bounce.clear();
        let mut created = 0;
        for c in 0..k {
            let members: Vec<StrandRef> = owner.iter().enumerate().filter(|(_, &o)| o == c).map(|(i, _)| refs[i]).collect();
            if members.is_empty() {
                continue;
            }
            let id = self.alloc_bounce_id();
            let mut b = BounceGroup::new(id, format!("Bounce {}", created + 1));
            b.bones = bones;
            b.strands = members;
            self.bounce.push(b);
            created += 1;
        }
        Ok(created)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::drop_guide;
    use super::super::StrandParams;
    use super::*;

    fn comb_set(n: usize) -> HairSet {
        let mut s = HairSet::new();
        let g = s.add_group("Tudo").unwrap();
        for i in 0..n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            let root = Vec3::new(a.cos() * 0.5, 1.0, a.sin() * 0.5);
            s.add_strand(g, drop_guide(root, 0.7, 5), StrandParams::default(), Vec3::Y).unwrap();
        }
        s
    }

    #[test]
    fn auto_bounce_assigns_every_strand_exactly_once() {
        for k in [1usize, 3, 8, 30] {
            let mut s = comb_set(24);
            let made = s.auto_bounce(k, 5).unwrap();
            assert_eq!(made, k.min(24), "k={k}");
            let mut all: Vec<StrandRef> = s.bounce.iter().flat_map(|b| b.strands.clone()).collect();
            all.sort();
            let mut want = s.all_strand_refs();
            want.sort();
            assert_eq!(all, want, "cada mecha em exatamente um grupo (k={k})");
            assert!(s.bounce.iter().all(|b| b.bones == 5));
            s.validate().unwrap();
        }
        // mais grupos que mechas: limitado ao nº de mechas
        let mut few = comb_set(3);
        assert_eq!(few.auto_bounce(30, 4).unwrap(), 3);
    }

    #[test]
    fn auto_bounce_is_deterministic_and_groups_neighbours() {
        let mut a = comb_set(20);
        let mut b = comb_set(20);
        a.auto_bounce(4, 4).unwrap();
        b.auto_bounce(4, 4).unwrap();
        assert_eq!(a.bounce, b.bounce);
        // as raízes de um grupo ficam mais próximas entre si do que o diâmetro da coroa
        for g in &a.bounce {
            let roots: Vec<Vec3> = g.strands.iter().map(|r| a.strand(*r).unwrap().root()).collect();
            let c = roots.iter().copied().sum::<Vec3>() / roots.len() as f32;
            assert!(roots.iter().all(|r| r.distance(c) < 0.75), "grupo coeso");
        }
    }

    #[test]
    fn invalid_auto_bounce_arguments_are_rejected() {
        let mut s = comb_set(4);
        assert!(s.auto_bounce(0, 4).is_err());
        assert!(s.auto_bounce(31, 4).is_err());
        assert!(s.auto_bounce(2, 0).is_err());
        assert!(s.auto_bounce(2, 17).is_err());
        assert!(s.bounce.is_empty());
        let mut empty = HairSet::new();
        assert!(empty.auto_bounce(2, 4).is_err());
    }

    #[test]
    fn chains_follow_bones_fixed_point_and_stroke_direction() {
        let mut s = comb_set(6);
        s.auto_bounce(2, 4).unwrap();
        for b in &mut s.bounce {
            b.fixed_point = 0.25;
            b.hit_radius = 0.03;
        }
        let chains = s.generate_chains().unwrap();
        assert_eq!(chains.len(), 6);
        for c in &chains {
            assert_eq!(c.points.len(), 5, "4 ossos = 5 pontos");
            assert_eq!(c.root_node_hint, "head");
            assert_eq!(c.radius, 0.03);
            let strand = s.strand(c.source).unwrap();
            // a âncora fica a 25% do comprimento: y = root.y - 0.25·0.7
            assert!((c.points[0].y - (strand.root().y - 0.25 * 0.7)).abs() < 0.02, "{:?}", c.points[0]);
            assert!(c.points.last().unwrap().distance(strand.tip()) < 1e-3, "termina na ponta");
            assert!(c.points.windows(2).all(|w| w[1].y < w[0].y), "raiz -> ponta");
        }
        // serde estável
        let json = serde_json::to_string(&chains).unwrap();
        let back: Vec<HairChainSpec> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, chains);
    }

    #[test]
    fn editing_the_hairstyle_updates_the_chains_and_removal_prunes_groups() {
        let mut s = comb_set(4);
        s.auto_bounce(1, 3).unwrap();
        let before = s.generate_chains().unwrap();
        let r = s.all_strand_refs()[0];
        s.set_point_pos(r, 4, Vec3::new(2.0, 0.0, 0.0)).unwrap();
        let after = s.generate_chains().unwrap();
        assert_ne!(before[0].points, after[0].points, "cadeia derivada da guia atual");
        assert_eq!(s.remove_strand(r), 1);
        s.validate().unwrap();
        assert_eq!(s.generate_chains().unwrap().len(), 3);
        // grupo de bounce com valores fora da faixa é recusado
        s.bounce[0].fixed_point = 1.5;
        assert!(s.generate_chains().is_err());
        assert!(s.validate().is_err());
    }
}
