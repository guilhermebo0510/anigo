//! Pincel de "pentear": reorienta os segmentos da guia na direção do pincel, preservando o comprimento
//! de cada segmento e fixando a raiz.
//!
//! Para cada segmento `i` (de `p[i-1]` a `p[i]`) com peso `w ∈ [0, 1]`, a direção vira
//! `normalize(lerp(dir_i, dir_pincel, w))` com o mesmo comprimento; os pontos são reconstruídos da raiz
//! para a ponta, então o trecho depois da região penteada acompanha rigidamente (translação).
//! O peso vem de uma função fornecida pelo chamador (3D esférico via [`SphereBrush`], ou em espaço de
//! tela no studio).

use super::{HairSet, HairStrand, StrandRef};
use crate::curve::CurveKind;
use glam::Vec3;

/// Pincel esférico 3D (espaço local do alvo).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SphereBrush {
    pub center: Vec3,
    pub radius: f32,
    /// Intensidade 0..=1.
    pub strength: f32,
}

impl SphereBrush {
    /// Peso do segmento com ponto médio `mid`: `strength · (1 − d/r)²` dentro do raio.
    pub fn weight(&self, mid: Vec3) -> f32 {
        if self.radius <= 0.0 {
            return 0.0;
        }
        let d = mid.distance(self.center) / self.radius;
        if d >= 1.0 {
            0.0
        } else {
            self.strength.clamp(0.0, 1.0) * (1.0 - d) * (1.0 - d)
        }
    }
}

/// Penteia UMA mecha. `weight(seg_mid, seg_index)` devolve o peso do segmento `seg_index`
/// (`1..n`, o segmento `i` liga `p[i-1]` a `p[i]`). Devolve o deslocamento máximo de um ponto
/// (0 se nada mudou).
pub fn comb_strand(strand: &mut HairStrand, dir: Vec3, weight: &dyn Fn(Vec3, usize) -> f32) -> f32 {
    let Some(dir) = dir.try_normalize() else {
        return 0.0;
    };
    let n = strand.guide.points.len();
    if n < 2 {
        return 0.0;
    }
    let old: Vec<Vec3> = strand.guide.points.iter().map(|p| p.pos).collect();
    let mut new = old.clone();
    let mut changed = false;
    for i in 1..n {
        let seg = old[i] - old[i - 1];
        let len = seg.length();
        let Some(cur) = seg.try_normalize() else {
            new[i] = new[i - 1];
            continue;
        };
        let w = weight(0.5 * (old[i] + old[i - 1]), i).clamp(0.0, 1.0);
        let d = if w > 1e-6 {
            changed = true;
            cur.lerp(dir, w).try_normalize().unwrap_or(cur)
        } else {
            cur
        };
        new[i] = new[i - 1] + d * len;
    }
    if !changed {
        return 0.0;
    }
    let mut max_move = 0.0f32;
    for (p, q) in strand.guide.points.iter_mut().zip(&new) {
        max_move = max_move.max(p.pos.distance(*q));
        p.pos = *q;
        p.handle_in = None;
        p.handle_out = None;
    }
    strand.guide.kind = CurveKind::CatmullRom;
    max_move
}

impl HairSet {
    /// Penteia as mechas do grupo `group`. `weight_for(strand)` devolve o peso por segmento
    /// (como em [`comb_strand`]). Em grupos com simetria, cada par recebe o pincel nos DOIS lados e o
    /// lado que mais se moveu é espelhado sobre o outro (resultado simétrico e determinístico).
    /// Devolve as mechas alteradas.
    pub fn comb(&mut self, group: u32, dir: Vec3, weight_for: &dyn Fn(&HairStrand, Vec3, usize) -> f32) -> Vec<StrandRef> {
        let Some(g) = self.group_mut(group) else {
            return Vec::new();
        };
        let symmetric = g.symmetry;
        let mut moved: Vec<(u32, f32)> = Vec::new();
        let ids: Vec<u32> = g.strands.iter().map(|s| s.id).collect();
        let mut handled = std::collections::HashSet::new();
        for id in ids {
            if handled.contains(&id) {
                continue;
            }
            let Some(strand) = g.strand(id).cloned() else {
                continue;
            };
            let partner = if symmetric { strand.mirror_of } else { None };
            let mut a = strand.clone();
            let da = comb_strand(&mut a, dir, &|mid, i| weight_for(&strand, mid, i));
            match partner.and_then(|p| g.strand(p).cloned()) {
                Some(pstrand) => {
                    let mut b = pstrand.clone();
                    let db = comb_strand(&mut b, dir, &|mid, i| weight_for(&pstrand, mid, i));
                    handled.insert(pstrand.id);
                    if da <= 0.0 && db <= 0.0 {
                        continue;
                    }
                    // o lado que mais se moveu manda; o outro é o espelho dele
                    let (src, dst_id) = if da >= db { (a, pstrand.id) } else { (b, id) };
                    let mut m = src.mirrored_x();
                    m.id = dst_id;
                    m.mirror_of = Some(src.id);
                    let src_id = src.id;
                    if let Some(s) = g.strand_mut(src_id) {
                        *s = src;
                    }
                    if let Some(s) = g.strand_mut(dst_id) {
                        *s = m;
                    }
                    moved.push((id, da.max(db)));
                    moved.push((pstrand.id, da.max(db)));
                }
                None => {
                    if da > 0.0 {
                        if let Some(s) = g.strand_mut(id) {
                            *s = a;
                        }
                        moved.push((id, da));
                    }
                }
            }
        }
        moved.into_iter().map(|(strand, _)| StrandRef { group, strand }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::drop_guide;
    use super::super::StrandParams;
    use super::*;

    fn seg_lengths(s: &HairStrand) -> Vec<f32> {
        s.guide.points.windows(2).map(|w| w[0].pos.distance(w[1].pos)).collect()
    }

    #[test]
    fn comb_keeps_root_and_segment_lengths_and_turns_toward_brush() {
        let mut s = HairStrand::new(1, drop_guide(Vec3::new(0.2, 1.0, 0.0), 1.0, 6), StrandParams::default(), Vec3::Y);
        let lens = seg_lengths(&s);
        let root = s.root();
        let brush = SphereBrush {
            center: Vec3::new(0.2, 0.5, 0.0),
            radius: 0.6,
            strength: 1.0,
        };
        let moved = comb_strand(&mut s, Vec3::X, &|mid, _| brush.weight(mid));
        assert!(moved > 0.01);
        assert_eq!(s.root(), root, "raiz fixa");
        for (a, b) in lens.iter().zip(seg_lengths(&s)) {
            assert!((a - b).abs() < 1e-5, "comprimento do segmento preservado");
        }
        assert!(s.tip().x > 0.2 + 0.05, "a ponta foi empurrada para +X: {}", s.tip().x);
    }

    #[test]
    fn strands_outside_the_brush_are_untouched() {
        let mut set = HairSet::new();
        let g = set.add_group("A").unwrap();
        let near = set.add_strand(g, drop_guide(Vec3::new(0.0, 1.0, 0.0), 0.6, 5), StrandParams::default(), Vec3::Y).unwrap()[0];
        let far = set.add_strand(g, drop_guide(Vec3::new(3.0, 1.0, 0.0), 0.6, 5), StrandParams::default(), Vec3::Y).unwrap()[0];
        let before_far = set.strand(StrandRef { group: g, strand: far }).unwrap().clone();
        let brush = SphereBrush {
            center: Vec3::new(0.0, 0.7, 0.0),
            radius: 0.5,
            strength: 1.0,
        };
        let changed = set.comb(g, Vec3::Z, &|_, mid, _| brush.weight(mid));
        assert_eq!(changed, vec![StrandRef { group: g, strand: near }]);
        assert_eq!(set.strand(StrandRef { group: g, strand: far }).unwrap(), &before_far);
    }

    #[test]
    fn symmetric_comb_keeps_the_pair_mirrored() {
        let mut set = HairSet::new();
        let g = set.add_group("Franja").unwrap();
        set.group_mut(g).unwrap().symmetry = true;
        let ids = set.add_strand(g, drop_guide(Vec3::new(0.3, 1.0, 0.0), 0.8, 6), StrandParams::default(), Vec3::Y).unwrap();
        // pincel perto do lado +X só
        let brush = SphereBrush {
            center: Vec3::new(0.3, 0.6, 0.0),
            radius: 0.4,
            strength: 1.0,
        };
        let changed = set.comb(g, Vec3::Z, &|_, mid, _| brush.weight(mid));
        assert_eq!(changed.len(), 2);
        let a = set.strand(StrandRef { group: g, strand: ids[0] }).unwrap();
        let b = set.strand(StrandRef { group: g, strand: ids[1] }).unwrap();
        for (pa, pb) in a.guide.points.iter().zip(&b.guide.points) {
            assert!((pa.pos.x + pb.pos.x).abs() < 1e-5 && (pa.pos.z - pb.pos.z).abs() < 1e-5);
        }
        assert!(a.tip().z > 0.05, "foi penteado para +Z");
        set.validate().unwrap();
    }

    #[test]
    fn zero_direction_or_zero_weight_is_a_noop() {
        let mut s = HairStrand::new(1, drop_guide(Vec3::Y, 1.0, 5), StrandParams::default(), Vec3::Y);
        let before = s.clone();
        assert_eq!(comb_strand(&mut s, Vec3::ZERO, &|_, _| 1.0), 0.0);
        assert_eq!(comb_strand(&mut s, Vec3::X, &|_, _| 0.0), 0.0);
        assert_eq!(s, before);
    }
}
