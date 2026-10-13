//! Tronco: anéis de 16 vértices (sem vértice em x = 0: o plano de simetria passa pelo meio
//! das arestas frontal/traseira) do pescoço até a virilha, com os buracos dos ombros.

use super::head::HeadOut;
use super::{best_perm, Region, B};
use crate::dims::{superellipse, trunk_at, Dims};
use crate::params::lerp;
use glam::Vec3;
use std::f32::consts::TAU;

pub(crate) const RING_N: usize = 16;
/// Linha (índice em `rows`) do topo do buraco do ombro.
pub(crate) const ARM_HOLE_ROW: usize = 4;

pub(crate) struct TrunkOut {
    pub rows: Vec<Vec<u32>>,
    /// contorno (12) do buraco do ombro: [esquerdo (+x), direito (-x)]
    pub arm_holes: [Vec<u32>; 2],
}

/// Frações y/H das linhas do tronco (a linha 0 é o anel do pescoço, vem da cabeça).
const ROW_U: [f32; 18] = [
    0.834, 0.821, 0.8075, 0.7925, 0.772, 0.751, 0.7306, 0.7094, 0.6875, 0.6625, 0.6406, 0.622, 0.600, 0.578, 0.556, 0.534, 0.5125, 0.4906,
];

pub(crate) fn theta(j: usize) -> f32 {
    (j as f32 + 0.5) * TAU / RING_N as f32
}

/// Anel do tronco na altura `y`, 16 pontos em ordem canônica (j = 0 logo à esquerda do centro frontal).
pub(crate) fn trunk_ring(d: &Dims, knots: &[crate::dims::TrunkKnot], y: f32) -> Vec<Vec3> {
    let k = trunk_at(knots, y / d.h);
    let mut pts = Vec::with_capacity(RING_N);
    for j in 0..RING_N {
        let (x, z) = superellipse(k.rx, k.rz, k.n, theta(j));
        let mut p = Vec3::new(x, y, z + k.zc);
        // busto / peitoral (frente) e glúteos (costas)
        let g = d.g;
        let bust_amp = lerp(0.008, 0.034, g) * (1.0 + 0.45 * d.p.weight.max(0.0) + 0.5 * d.p.muscle * (1.0 - g)) * (1.0 - 0.35 * d.p.age * g);
        let by = d.y_bust - 0.012 * d.age_sag();
        let bust = bust_amp * (-((x.abs() - 0.064 * d.sc) / 0.034).powi(2)).exp() * (-((y - by) / 0.040).powi(2)).exp();
        let glute_amp = lerp(0.010, 0.020, g) * (1.0 + 0.3 * d.p.weight.max(0.0)) * (1.0 - 0.3 * d.p.age);
        let glute = glute_amp * (-((x.abs() - 0.062 * d.sc) / 0.046).powi(2)).exp() * (-((y - (d.y_hip - 0.035)) / 0.058).powi(2)).exp();
        if z > 0.0 {
            p.z += bust;
        } else {
            p.z -= glute;
        }
        pts.push(p);
    }
    pts
}

impl Dims {
    /// Deslocamento (m) do busto por idade.
    pub fn age_sag(&self) -> f32 {
        self.p.age * self.g
    }
}

pub(crate) fn build_trunk(b: &mut B, d: &Dims, head: &HeadOut) -> TrunkOut {
    let knots = d.trunk_knots();

    // anel 0 = contorno do buraco do pescoço, reordenado para a ordem canônica
    let neck_ref: Vec<Vec3> = b.positions(&head.neck_loop);
    let c = b.centroid(&head.neck_loop);
    let canon: Vec<Vec3> = (0..RING_N)
        .map(|j| {
            let (x, z) = superellipse(0.027, 0.024, 2.0, theta(j));
            Vec3::new(c.x + x, c.y, c.z + z)
        })
        .collect();
    let perm = best_perm(&canon, &neck_ref);
    let ring0: Vec<u32> = (0..RING_N).map(|j| head.neck_loop[perm[j]]).collect();
    for &i in &ring0 {
        b.region[i as usize] = Region::Neck;
    }

    for (j, &i) in ring0.iter().enumerate() {
        b.set_attr(i, 0, 0.0);
        b.set_attr(i, 1, theta(j).sin());
        b.set_attr(i, 2, theta(j).cos());
    }
    let mut rows: Vec<Vec<u32>> = vec![ring0];
    for (r, &u) in ROW_U.iter().enumerate() {
        let y = u * d.h;
        let pts = trunk_ring(d, &knots, y);
        let reg = if r < 2 { Region::Neck } else { Region::Trunk };
        let ids = b.ring(&pts, reg);
        for (j, &i) in ids.iter().enumerate() {
            b.set_attr(i, 0, (r + 1) as f32);
            b.set_attr(i, 1, theta(j).sin());
            b.set_attr(i, 2, theta(j).cos());
        }
        rows.push(ids);
    }
    let nrows = rows.len();

    let in_hole = |r: usize, j: usize| -> bool {
        r >= ARM_HOLE_ROW && r < ARM_HOLE_ROW + 3 && ((2..=4).contains(&j) || (10..=12).contains(&j))
    };
    for r in 0..nrows - 1 {
        for j in 0..RING_N {
            if in_hole(r, j) {
                continue;
            }
            let j2 = (j + 1) % RING_N;
            b.quad(rows[r][j], rows[r][j2], rows[r + 1][j2], rows[r + 1][j]);
        }
    }

    // contornos dos buracos dos ombros (12 vértices, ordem cíclica)
    let hole_loop = |c0: usize| -> Vec<u32> {
        let r0 = ARM_HOLE_ROW;
        let mut v = Vec::new();
        for j in c0..=c0 + 3 {
            v.push(rows[r0][j]);
        }
        for r in r0 + 1..=r0 + 3 {
            v.push(rows[r][c0 + 3]);
        }
        for j in (c0..c0 + 3).rev() {
            v.push(rows[r0 + 3][j]);
        }
        for r in (r0 + 1..r0 + 3).rev() {
            v.push(rows[r][c0]);
        }
        v
    };
    let hole_l = hole_loop(2);
    let hole_r = hole_loop(10);

    // laços nomeados
    for (k, name) in [(0usize, "neck_ring0"), (1, "neck_ring1"), (2, "neck_ring2"), (3, "clavicle_loop")] {
        b.add_loop(name, rows[k].clone(), true);
    }
    b.add_loop("axilla_row", rows[ARM_HOLE_ROW + 3].clone(), true);
    b.add_loop("chest_loop", rows[ARM_HOLE_ROW + 4].clone(), true);
    b.add_loop("ribs_loop", rows[ARM_HOLE_ROW + 6].clone(), true);
    b.add_loop("waist_loop", rows[12].clone(), true);
    b.add_loop("navel_loop", rows[13].clone(), true);
    b.add_loop("pelvis_loop_a", rows[15].clone(), true);
    b.add_loop("pelvis_loop_b", rows[16].clone(), true);
    b.add_loop("crotch_loop", rows[nrows - 1].clone(), true);
    b.add_loop("shoulder_hole_L", hole_l.clone(), true);
    b.add_loop("shoulder_hole_R", hole_r.clone(), true);

    // âncoras para o esqueleto e o busto
    b.mark("hip_ring", rows[15].clone());
    b.mark("spine_ring", rows[12].clone());
    b.mark("chest_ring", rows[ARM_HOLE_ROW + 4].clone());
    b.mark("upperchest_ring", rows[ARM_HOLE_ROW + 1].clone());
    b.mark("neck_base_ring", rows[2].clone());
    b.mark("neck_top_ring", rows[0].clone());
    for (name, side_cols) in [("bust_L", [0usize, 1, 2]), ("bust_R", [13, 14, 15])] {
        let mut v = Vec::new();
        for &r in &[ARM_HOLE_ROW + 3, ARM_HOLE_ROW + 4] {
            for &cidx in &side_cols {
                v.push(rows[r][cidx]);
            }
        }
        b.mark(name, v);
    }
    b.mark("navel", vec![rows[13][0], rows[13][15]]);

    TrunkOut {
        rows,
        arm_holes: [hole_l, hole_r],
    }
}
