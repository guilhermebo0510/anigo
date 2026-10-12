//! Braços (com mãos de cinco dedos) e pernas (com pés e dedos agrupados), todos como tubos
//! que se ligam ao tronco por ponte (ombro) ou por divisão em "calça" (pernas).

use super::trunk::{TrunkOut, RING_N};
use super::{best_perm, Region, B};
use crate::dims::Dims;
use crate::params::{lerp, smoothstep};
use glam::Vec3;
use std::f32::consts::PI;

fn catmull(ts: &[(f32, f32, f32)], t: f32) -> (f32, f32) {
    // interpolação linear suavizada (smoothstep entre nós) para raios
    let n = ts.len();
    if t <= ts[0].0 {
        return (ts[0].1, ts[0].2);
    }
    if t >= ts[n - 1].0 {
        return (ts[n - 1].1, ts[n - 1].2);
    }
    let mut i = 0;
    while i + 1 < n && ts[i + 1].0 < t {
        i += 1;
    }
    let (a, b) = (ts[i], ts[i + 1]);
    let u = (t - a.0) / (b.0 - a.0);
    let s = u * u * (3.0 - 2.0 * u) * 0.5 + u * 0.5; // mistura linear/suave
    (lerp(a.1, b.1, s), lerp(a.2, b.2, s))
}

fn sup(rx: f32, rz: f32, n: f32, a: f32) -> (f32, f32) {
    let (s, c) = a.sin_cos();
    let e = 2.0 / n;
    (rx * s.signum() * s.abs().powf(e), rz * c.signum() * c.abs().powf(e))
}

fn side_name(s: f32) -> &'static str {
    if s > 0.0 {
        "L"
    } else {
        "R"
    }
}

// ---------------------------------------------------------------------------------------------
// BRAÇOS E MÃOS
// ---------------------------------------------------------------------------------------------

pub(crate) fn build_arms(b: &mut B, d: &Dims, t: &TrunkOut) {
    for (si, s) in [1.0f32, -1.0f32].into_iter().enumerate() {
        build_arm(b, d, &t.arm_holes[si], s);
    }
}

/// Layout retangular da seção da palma: k -> (c, r). c = 0 é o lado do polegar (+z), r = 0 é a palma.
const PALM_LAYOUT: [(usize, usize); 12] = [
    (0, 0),
    (1, 0),
    (2, 0),
    (3, 0),
    (4, 0),
    (4, 1),
    (4, 2),
    (3, 2),
    (2, 2),
    (1, 2),
    (0, 2),
    (0, 1),
];

fn build_arm(b: &mut B, d: &Dims, hole: &[u32], s: f32) {
    let sn = side_name(s);
    let sc = d.sc;
    let g = d.g;
    let p = &d.p;
    let hc = b.centroid(hole);
    let x_h = hc.x.abs();
    let yc = hc.y;
    let zc = hc.z;
    let x_sj = x_h - 0.022 * sc;
    let l_u = lerp(0.288, 0.264, g) * sc;
    let l_f = lerp(0.252, 0.236, g) * sc;

    let upper: [(f32, f32, f32); 6] = [
        (0.0, 0.056, 0.052),
        (0.22, 0.054, 0.050),
        (0.44, 0.047, 0.047),
        (0.70, 0.043, 0.045),
        (0.87, 0.038, 0.040),
        (1.0, 0.0345, 0.0365),
    ];
    let fore: [(f32, f32, f32); 6] = [
        (0.0, 0.0345, 0.0365),
        (0.2, 0.0375, 0.0405),
        (0.4, 0.0378, 0.041),
        (0.65, 0.031, 0.036),
        (0.85, 0.024, 0.030),
        (1.0, 0.0165, 0.0255),
    ];
    let wk = if p.weight >= 0.0 { p.weight } else { 0.40 * p.weight };
    let gm = lerp(1.16, 1.07, g);
    let thick = |seg_t: f32, base: f32, is_upper: bool| -> f32 {
        let musc = p.muscle * if is_upper { 0.14 } else { 0.07 } * (0.4 + 0.6 * (std::f32::consts::PI * seg_t).sin());
        base * gm * sc * (1.0 + 0.34 * wk + musc - 0.07 * p.age)
    };

    // estações: (distância desde a junta do ombro, raios, é braço?)
    let mut stations: Vec<(f32, f32, f32)> = Vec::new(); // (dist, ry, rz)
    for &tt in &[0.17f32, 0.31, 0.45, 0.59, 0.73, 0.88, 1.0] {
        let (ry, rz) = catmull(&upper, tt);
        stations.push((tt * l_u, thick(tt, ry, true), thick(tt, rz, true)));
    }
    for &tt in &[0.12f32, 0.30, 0.48, 0.66, 0.84, 1.0] {
        let (ry, rz) = catmull(&fore, tt);
        stations.push((l_u + tt * l_f, thick(tt, ry, false), thick(tt, rz, false)));
    }
    let elbow_idx = 6usize; // índice em `stations` (t = 1.0 do braço)
    let n_st = stations.len();

    let ring_pts = |x_abs: f32, ry: f32, rz: f32| -> Vec<Vec3> {
        (0..12)
            .map(|i| {
                let a = i as f32 * PI / 6.0;
                let (zz, yy) = sup(rz, ry, 2.3, a);
                Vec3::new(s * x_abs, yc + yy, zc + zz)
            })
            .collect()
    };

    // alinhamento do primeiro anel com o buraco do ombro
    let first = ring_pts(x_sj + stations[0].0, stations[0].1, stations[0].2);
    let hole_pos = b.positions(hole);
    let perm = best_perm(&hole_pos, &first);

    let mut rings: Vec<Vec<u32>> = vec![hole.to_vec()];
    for (k, &(dist, ry, rz)) in stations.iter().enumerate() {
        let pts = ring_pts(x_sj + dist, ry, rz);
        let ordered: Vec<Vec3> = (0..12).map(|i| pts[perm[i]]).collect();
        let _ = k;
        rings.push(b.ring(&ordered, Region::Arm));
    }
    for k in 0..rings.len() - 1 {
        let (a, c) = (rings[k].clone(), rings[k + 1].clone());
        b.bridge(&a, &c);
    }

    // laços/âncoras
    b.mark(format!("arm_{sn}_hole"), hole.to_vec());
    for (k, r) in rings.iter().enumerate() {
        b.mark(format!("arm_{sn}_ring{k}"), r.clone());
    }
    // rings[0] = buraco; rings[1 + i] = estação i
    let elbow_ring = rings[1 + elbow_idx].clone();
    b.add_loop(format!("elbow_{sn}_ring0"), rings[elbow_idx].clone(), true);
    b.add_loop(format!("elbow_{sn}_ring1"), elbow_ring.clone(), true);
    b.add_loop(format!("elbow_{sn}_ring2"), rings[2 + elbow_idx].clone(), true);
    b.mark(format!("elbow_{sn}"), elbow_ring);
    let wrist_ring = rings[n_st].clone();
    b.add_loop(format!("wrist_{sn}_ring1"), wrist_ring.clone(), true);
    b.add_loop(format!("wrist_{sn}_ring0"), rings[n_st - 1].clone(), true);
    b.mark(format!("wrist_{sn}"), wrist_ring.clone());
    b.add_loop(format!("forearm_mid_{sn}"), rings[1 + elbow_idx + 3].clone(), true);

    build_hand(b, d, &wrist_ring, s, yc, zc, x_sj + l_u + l_f);
}

fn build_hand(b: &mut B, d: &Dims, wrist: &[u32], s: f32, yc: f32, zc: f32, x_w_abs: f32) {
    let sn = side_name(s);
    let sc = d.sc;
    let g = d.g;
    let p = &d.p;
    let wk = if p.weight >= 0.0 { p.weight } else { 0.3 * p.weight };
    let wid = lerp(0.080, 0.071, g) * sc * (1.0 + 0.16 * wk + 0.04 * p.muscle);
    let th = [0.027f32, 0.0245, 0.0215];
    let th_k = 1.0 + 0.20 * wk;

    // anéis da palma: lerp entre a elipse do pulso e o retângulo da palma
    let wpos = b.positions(wrist);
    let rect_pt = |k: usize, x_abs: f32, w: f32, t: f32, zc_off: f32| -> Vec3 {
        let (c, r) = PALM_LAYOUT[k];
        let zz = zc + zc_off + w * 0.5 - c as f32 * w / 4.0;
        let yy = yc + (r as f32 - 1.0) * t * 0.5;
        // arredonda os cantos
        let corner = (c == 0 || c == 4) && (r == 0 || r == 2);
        let (zz, yy) = if corner {
            (zc + zc_off + (zz - zc - zc_off) * 0.86, yc + (yy - yc) * 0.82)
        } else {
            (zz, yy)
        };
        Vec3::new(s * x_abs, yy, zz)
    };
    // casamento elipse -> layout
    let wr_c = b.centroid(wrist);
    let wrect: Vec<Vec3> = (0..12)
        .map(|k| {
            let r = rect_pt(k, x_w_abs, 0.049, 0.032, 0.0);
            Vec3::new(wr_c.x, r.y, r.z)
        })
        .collect();
    let perm = best_perm(&wpos, &wrect); // wpos[i] <-> wrect[perm[i]]
    let stations = [(0.034f32, 0.62f32), (0.064, 0.92), (0.092, 1.0)];
    let mut palm: Vec<Vec<u32>> = vec![wrist.to_vec()];
    for (m, &(dx, tt)) in stations.iter().enumerate() {
        let x_abs = x_w_abs + dx * sc;
        let w = lerp(0.049, wid, tt);
        let t = lerp(0.032, th[m] * th_k, 0.5 + 0.5 * tt);
        let t = if m == 2 { th[2] * th_k } else { t };
        let mut ids = Vec::with_capacity(12);
        for i in 0..12 {
            let k = perm[i];
            let rp = rect_pt(k, x_abs, w, t, 0.0);
            // parte elíptica (pulso) no início
            let ell_t = (1.0 - tt).max(0.0);
            let wp = wpos[i];
            let ell = Vec3::new(s * x_abs, wp.y, wp.z);
            ids.push(b.vert(ell * ell_t * 0.5 + rp * (1.0 - ell_t * 0.5), Region::Hand));
        }
        palm.push(ids);
    }
    // índice do layout -> vértice, por anel
    let by_layout = |ring: &[u32]| -> [u32; 12] {
        let mut out = [u32::MAX; 12];
        for i in 0..12 {
            out[perm[i]] = ring[i];
        }
        out
    };
    let p1 = by_layout(&palm[1]);
    let p2 = by_layout(&palm[2]);

    // pontes: pulso -> P1 -> P2 -> P3 (a faixa P1-P2 do lado do polegar vira a base do polegar)
    b.bridge(&palm[0].clone(), &palm[1].clone());
    {
        let a = palm[1].clone();
        let c = palm[2].clone();
        for i in 0..12 {
            let j = (i + 1) % 12;
            let (ka, kb) = (perm[i], perm[j]);
            // arestas do lado c = 0 entre r=0..2: pares de layout (11,0)? -> (0,0)-(0,1), (0,1)-(0,2)
            let thumb_edge = {
                let pair = (ka.min(kb), ka.max(kb));
                pair == (0, 11) || pair == (10, 11)
            };
            if thumb_edge {
                continue;
            }
            b.quad(a[i], a[j], c[j], c[i]);
        }
    }
    b.bridge(&palm[2].clone(), &palm[3].clone());
    let p3 = by_layout(&palm[3]);

    // vértices internos da grade distal (c = 1..3, r = 1)
    let mut mid = [u32::MAX; 5];
    for c in 1..=3usize {
        let a = b.pos[p3[PALM_LAYOUT.iter().position(|&x| x == (c, 0)).unwrap()] as usize];
        let t = b.pos[p3[PALM_LAYOUT.iter().position(|&x| x == (c, 2)).unwrap()] as usize];
        mid[c] = b.vert((a + t) * 0.5, Region::Hand);
    }
    let lay = |c: usize, r: usize| -> usize { PALM_LAYOUT.iter().position(|&x| x == (c, r)).unwrap() };
    let grid = |c: usize, r: usize| -> u32 {
        if r == 1 && (1..=3).contains(&c) {
            mid[c]
        } else {
            p3[lay(c, r)]
        }
    };

    b.mark(format!("palm_{sn}_ring1"), palm[1].clone());
    b.mark(format!("palm_{sn}_ring2"), palm[2].clone());
    b.mark(format!("palm_{sn}_ring3"), palm[3].clone());
    b.add_loop(format!("palm_{sn}_knuckle_loop"), palm[3].clone(), true);
    b.add_loop(format!("palm_{sn}_mid_loop"), palm[2].clone(), true);

    // --- dedos ---
    let x_base = x_w_abs + 0.092 * sc;
    let fan = [5.0f32, 1.5, -3.0, -8.5];
    let lens = [0.071f32, 0.079, 0.073, 0.057];
    let fnames = ["index", "middle", "ring", "little"];
    let fregions = [Region::Index, Region::Middle, Region::Ring, Region::Little];
    let hs = sc * lerp(1.04, 0.97, g);
    let fr = [0.0f32, 0.225, 0.45, 0.60, 0.75, 0.875, 1.0];
    let ry_t = [0.0108f32, 0.0099, 0.0092, 0.0085, 0.0079, 0.0073, 0.0060];
    let rz_t = [0.0096f32, 0.0090, 0.0087, 0.0083, 0.0078, 0.0072, 0.0058];
    let fat = 1.0 + 0.10 * wk;
    for f in 0..4usize {
        let base_ring = vec![grid(f, 0), grid(f + 1, 0), grid(f + 1, 1), grid(f + 1, 2), grid(f, 2), grid(f, 1)];
        let bc = b.centroid(&base_ring);
        let ang = fan[f].to_radians();
        let dir = Vec3::new(s * ang.cos(), 0.0, ang.sin());
        let len = lens[f] * hs;
        let hex = [(0.90f32, -0.85f32), (-0.90, -0.85), (-1.08, 0.0), (-0.90, 0.85), (0.90, 0.85), (1.08, 0.0)];
        let mut rings = vec![base_ring];
        for k in 1..7 {
            let center = bc + dir * (len * fr[k]);
            let (ry, rz) = (ry_t[k] * hs * fat, rz_t[k] * hs * fat);
            let pts: Vec<Vec3> = hex.iter().map(|&(sz, sy)| center + Vec3::new(0.0, sy * ry, sz * rz)).collect();
            let ids = b.ring(&pts, fregions[f]);
            for &i in &ids {
                b.set_attr(i, 3, k as f32);
            }
            rings.push(ids);
        }
        for k in 0..6 {
            let (a, c) = (rings[k].clone(), rings[k + 1].clone());
            b.bridge(&a, &c);
        }
        let tip_c = b.centroid(&rings[6]) + dir * (0.0045 * hs);
        let tip = b.vert(tip_c, fregions[f]);
        b.set_attr(tip, 3, 6.5);
        let r6 = &rings[6];
        b.quad(r6[0], r6[1], r6[2], tip);
        b.quad(r6[2], r6[3], r6[4], tip);
        b.quad(r6[4], r6[5], r6[0], tip);
        for (k, r) in rings.iter().enumerate() {
            b.mark(format!("finger_{sn}_{}_ring{k}", fnames[f]), r.clone());
        }
        b.mark(format!("finger_{sn}_{}_tip", fnames[f]), vec![tip]);
        for (jn, ri) in [("mcp", 0usize), ("pip", 2), ("dip", 4)] {
            b.add_loop(format!("finger_{sn}_{}_{jn}", fnames[f]), rings[ri].clone(), true);
        }
    }

    // --- polegar: base = contorno do buraco aberto na faixa P1-P2 (lado c = 0) ---
    {
        let base_ring = vec![p1[lay(0, 0)], p1[lay(0, 1)], p1[lay(0, 2)], p2[lay(0, 2)], p2[lay(0, 1)], p2[lay(0, 0)]];
        let bc = b.centroid(&base_ring);
        let dir = Vec3::new(s * 0.52, -0.16, 0.84).normalize();
        let up = Vec3::Y;
        let mut lat = dir.cross(up).normalize();
        if lat.x * s < 0.0 {
            lat = -lat;
        }
        let lt = 0.072 * hs;
        let fr_t = [0.0f32, 0.20, 0.40, 0.55, 0.70, 0.85, 1.0];
        let ry_th = [0.0125f32, 0.0118, 0.0108, 0.0098, 0.0090, 0.0082, 0.0066];
        let rl_th = [0.0150f32, 0.0128, 0.0112, 0.0102, 0.0094, 0.0086, 0.0068];
        // ordem do anel: (P1 baixo), (P1 meio), (P1 cima), (P2 cima), (P2 meio), (P2 baixo)
        let pat = [(-0.88f32, -0.85f32), (-1.06, 0.0), (-0.88, 0.85), (0.88, 0.85), (1.06, 0.0), (0.88, -0.85)];
        let mut rings = vec![base_ring];
        for k in 1..7 {
            let center = bc + dir * (lt * fr_t[k]);
            let pts: Vec<Vec3> = pat.iter().map(|&(a, v)| center + lat * (a * rl_th[k] * hs * fat) + up * (v * ry_th[k] * hs * fat)).collect();
            let ids = b.ring(&pts, Region::Thumb);
            for &i in &ids {
                b.set_attr(i, 3, k as f32);
            }
            rings.push(ids);
        }
        for k in 0..6 {
            let (a, c) = (rings[k].clone(), rings[k + 1].clone());
            b.bridge(&a, &c);
        }
        let tip_c = b.centroid(&rings[6]) + dir * (0.005 * hs);
        let tip = b.vert(tip_c, Region::Thumb);
        b.set_attr(tip, 3, 6.5);
        let r6 = &rings[6];
        b.quad(r6[0], r6[1], r6[2], tip);
        b.quad(r6[2], r6[3], r6[4], tip);
        b.quad(r6[4], r6[5], r6[0], tip);
        for (k, r) in rings.iter().enumerate() {
            b.mark(format!("finger_{sn}_thumb_ring{k}"), r.clone());
        }
        b.mark(format!("finger_{sn}_thumb_tip"), vec![tip]);
        for (jn, ri) in [("cmc", 0usize), ("mcp", 2), ("ip", 4)] {
            b.add_loop(format!("finger_{sn}_thumb_{jn}"), rings[ri].clone(), true);
        }
    }
    let _ = smoothstep(0.0, 1.0, 0.5);
}

// ---------------------------------------------------------------------------------------------
// PERNAS E PÉS
// ---------------------------------------------------------------------------------------------

pub(crate) fn build_legs(b: &mut B, d: &Dims, t: &TrunkOut) {
    let crotch = t.rows.last().unwrap().clone();
    let nr = RING_N;
    // cadeias (frente -> trás): esquerda j = 0..7, direita j = 15..8
    let chain_l: Vec<u32> = (0..nr / 2).map(|j| crotch[j]).collect();
    let chain_r: Vec<u32> = (0..nr / 2).map(|j| crotch[nr - 1 - j]).collect();
    let (ll, ll_rings) = build_leg(b, d, &chain_l, 1.0);
    let (rr, rr_rings) = build_leg(b, d, &chain_r, -1.0);
    let _ = (ll, rr, ll_rings, rr_rings);
    // quad do assoalho da virilha: (c0 esq, c0 dir, c7 dir, c7 esq)
    b.quad(chain_l[0], chain_r[0], chain_r[nr / 2 - 1], chain_l[nr / 2 - 1]);
    b.add_loop("crotch_floor", vec![chain_l[0], chain_r[0], chain_r[nr / 2 - 1], chain_l[nr / 2 - 1]], true);
}

fn leg_angles() -> [f32; 8] {
    let mut a = [0.0f32; 8];
    for (j, v) in a.iter_mut().enumerate() {
        *v = (-55.0 + j as f32 * 290.0 / 7.0).to_radians();
    }
    a
}

fn build_leg(b: &mut B, d: &Dims, chain: &[u32], s: f32) -> (Vec<u32>, Vec<Vec<u32>>) {
    let sn = side_name(s);
    let sc = d.sc;
    let g = d.g;
    let p = &d.p;
    let h = d.h;
    let ang = leg_angles();
    let cx = lerp(0.094, 0.088, g) * sc * (1.0 + 0.05 * p.weight.max(0.0));
    let cz0 = 0.0;
    // tabela (u = y/H, rx, rz)
    let table: [(f32, f32, f32); 11] = [
        (0.045, 0.027, 0.030),
        (0.080, 0.029, 0.032),
        (0.130, 0.036, 0.040),
        (0.200, 0.047, 0.054),
        (0.250, 0.049, 0.054),
        (0.272, 0.048, 0.050),
        (0.295, 0.050, 0.053),
        (0.330, 0.058, 0.065),
        (0.390, 0.072, 0.085),
        (0.440, 0.079, 0.093),
        (0.470, 0.080, 0.094),
    ];
    let wk = if p.weight >= 0.0 { p.weight } else { 0.40 * p.weight };
    let hip_knots = d.trunk_knots();
    let gk = lerp(1.04, 1.12, g);
    let radius = |u: f32| -> (f32, f32) {
        let n = table.len();
        let mut i = 0;
        while i + 1 < n && table[i + 1].0 < u {
            i += 1;
        }
        let (a, c) = (table[i], table[(i + 1).min(n - 1)]);
        let t = if c.0 > a.0 { ((u - a.0) / (c.0 - a.0)).clamp(0.0, 1.0) } else { 0.0 };
        let t = t * t * (3.0 - 2.0 * t) * 0.5 + t * 0.5;
        let thigh = smoothstep(0.30, 0.44, u); // coxa
        let calf = smoothstep(0.12, 0.2, u) * (1.0 - smoothstep(0.24, 0.3, u));
        let k = sc * (1.0 + (0.26 * wk + 0.10 * p.muscle * (0.5 * thigh + calf) - 0.06 * p.age) * (0.4 + 0.6 * (thigh + calf).min(1.0)))
            * if u > 0.3 { gk } else { 1.0 + (gk - 1.0) * 0.7 };
        let (mut rx, mut rz) = (lerp(a.1, c.1, t) * k, lerp(a.2, c.2, t) * k);
        // a coxa nunca passa da largura do anel do quadril na virilha (sem degrau com peso alto)
        let blend = smoothstep(0.40, 0.46, u);
        let hip = crate::dims::trunk_at(&hip_knots, 0.4906);
        rx = lerp(rx, rx.min(hip.rx - cx + 0.024 * sc), blend);
        rz = lerp(rz, rz.min(hip.rz + 0.004 * sc), blend);
        (rx, rz)
    };
    let ring_pts = |u: f32| -> Vec<Vec3> {
        let (rx, rz) = radius(u);
        // flat na parte interna perto da virilha: a elipse é deslocada para fora conforme sobe
        let y = u * h;
        let dx = cx + rx * 0.0;
        ang.iter()
            .map(|&a| {
                let (x, z) = sup(rx, rz, 2.2, a);
                Vec3::new(s * (dx + x), y, cz0 + z)
            })
            .collect()
    };
    let us = [0.452f32, 0.420, 0.385, 0.350, 0.318, 0.292, 0.272, 0.252, 0.228, 0.190, 0.150, 0.115, 0.082, 0.058, 0.045];
    let mut rings: Vec<Vec<u32>> = Vec::new();
    for &u in &us {
        let pts = ring_pts(u);
        let reg = Region::Leg;
        rings.push(b.ring(&pts, reg));
    }
    // primeiro anel: faixa aberta (cadeia -> anel) + quad de fechamento (parede interna da coxa)
    let nr = chain.len(); // 8
    for j in 0..nr - 1 {
        b.quad(chain[j], chain[j + 1], rings[0][j + 1], rings[0][j]);
    }
    b.quad(chain[nr - 1], chain[0], rings[0][0], rings[0][nr - 1]);
    for k in 0..rings.len() - 1 {
        let (a, c) = (rings[k].clone(), rings[k + 1].clone());
        b.bridge(&a, &c);
    }
    let knee_i = 6usize;
    let ankle_i = us.len() - 1;
    b.add_loop(format!("knee_{sn}_ring0"), rings[knee_i - 1].clone(), true);
    b.add_loop(format!("knee_{sn}_ring1"), rings[knee_i].clone(), true);
    b.add_loop(format!("knee_{sn}_ring2"), rings[knee_i + 1].clone(), true);
    b.mark(format!("knee_{sn}"), rings[knee_i].clone());
    b.mark(format!("thigh_top_{sn}"), rings[0].clone());
    b.mark(format!("thigh_mid_{sn}"), rings[3].clone());
    b.mark(format!("calf_mid_{sn}"), rings[9].clone());
    b.add_loop(format!("ankle_{sn}_ring0"), rings[ankle_i - 1].clone(), true);
    b.add_loop(format!("ankle_{sn}_ring1"), rings[ankle_i].clone(), true);
    b.mark(format!("ankle_{sn}"), rings[ankle_i].clone());
    for (k, r) in rings.iter().enumerate() {
        b.mark(format!("leg_{sn}_ring{k}"), r.clone());
    }

    // --- pé ---
    let ankle_y = us[ankle_i] * h;
    let (ax, az) = radius(us[ankle_i]);
    let _ = az;
    // (normal φ em graus, centro (y, z), rx, rv, índice da região)
    let fs = sc * lerp(1.04, 1.0, g);
    let spec: [(f32, f32, f32, f32, f32, bool); 6] = [
        (34.0, 0.052, -0.006, 0.031, 0.047, false),
        (68.0, 0.036, 0.022, 0.034, 0.036, false),
        (88.0, 0.030, 0.066, 0.037, 0.031, false),
        (90.0, 0.0225, 0.116, 0.041, 0.0245, true),
        (90.0, 0.0165, 0.164, 0.040, 0.0180, true),
        (90.0, 0.0110, 0.206, 0.0315, 0.0115, true),
    ];
    let mut frings: Vec<Vec<u32>> = vec![rings[ankle_i].clone()];
    let _ = ax;
    for (fk, &(phi, cy, cz, rx, rv, toes)) in spec.iter().enumerate() {
        let ph = phi.to_radians();
        let yhat = Vec3::new(0.0, ph.sin(), ph.cos());
        let center = Vec3::new(s * cx, cy * fs, cz * fs);
        let pts: Vec<Vec3> = ang
            .iter()
            .map(|&a| {
                let (x, t) = sup(rx * fs, rv * fs, 2.6, a);
                let mut q = center + Vec3::new(s * x, 0.0, 0.0) + yhat * t;
                q.y = q.y.max(0.0035 * fs);
                q
            })
            .collect();
        let _ = fk;
        let reg = if toes { Region::Toes } else { Region::Foot };
        frings.push(b.ring(&pts, reg));
    }
    for k in 0..frings.len() - 1 {
        let (a, c) = (frings[k].clone(), frings[k + 1].clone());
        b.bridge(&a, &c);
    }
    // tampa da ponta (anel de 8 -> grade 2x2): ordem do anel angular
    let last = frings.last().unwrap().clone();
    let c8 = b.centroid(&last);
    let tipc = b.vert(c8 + Vec3::new(0.0, 0.0, 0.006 * fs), Region::Toes);
    // 4 quads em torno do centro: (0,1,2,c) (2,3,4,c) (4,5,6,c) (6,7,0,c)
    b.quad(last[0], last[1], last[2], tipc);
    b.quad(last[2], last[3], last[4], tipc);
    b.quad(last[4], last[5], last[6], tipc);
    b.quad(last[6], last[7], last[0], tipc);
    b.mark(format!("foot_{sn}_heel"), frings[1].clone());
    b.mark(format!("foot_{sn}_arch"), frings[3].clone());
    b.mark(format!("foot_{sn}_ball"), frings[4].clone());
    b.mark(format!("foot_{sn}_toebase"), frings[5].clone());
    b.mark(format!("foot_{sn}_toetip"), vec![tipc]);
    b.add_loop(format!("heel_{sn}_loop"), frings[1].clone(), true);
    b.add_loop(format!("arch_{sn}_loop"), frings[3].clone(), true);
    b.add_loop(format!("toe_base_{sn}_loop"), frings[5].clone(), true);
    (rings[0].clone(), rings)
}
