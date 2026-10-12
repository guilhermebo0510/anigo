//! Cabeça: cubo-esfera 13x13 por face (equiangular), com buracos para pescoço, olhos e boca,
//! "plugs" anelares (pálpebras, lábios) e orelhas. Os anéis concêntricos dos olhos/boca são
//! construídos a partir do contorno do buraco, então a conectividade é sempre a mesma.

use super::{Region, B};
use crate::dims::Dims;
use crate::params::{lerp, smoothstep};
use glam::Vec3;

pub(crate) const C: usize = 13;
const N1: usize = C + 1;

// Ângulos equiangulares (graus) por índice do reticulado. Simétricos em x e z.
const AX: [f32; N1] = [-45.0, -43.0, -38.5, -28.5, -19.0, -11.0, -5.0, 5.0, 11.0, 19.0, 28.5, 38.5, 43.0, 45.0];
const AY: [f32; N1] = [-45.0, -38.3, -31.6, -25.2, -19.3, -12.7, -5.7, 1.1, 8.6, 16.3, 23.6, 31.2, 38.3, 45.0];
const AZ: [f32; N1] = [-45.0, -38.0, -31.0, -24.5, -18.5, -12.0, -4.5, 4.5, 12.0, 18.5, 24.5, 31.0, 38.0, 45.0];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Front,
    Back,
    Right,
    Left,
    Top,
    Bottom,
}

/// Chave do reticulado (i, j, k) de uma célula/vértice de face.
fn key(side: Side, a: usize, b: usize) -> (usize, usize, usize) {
    match side {
        Side::Front => (a, b, C),
        Side::Back => (a, b, 0),
        Side::Right => (C, b, a),
        Side::Left => (0, b, a),
        Side::Top => (a, C, b),
        Side::Bottom => (a, 0, b),
    }
}

#[derive(Clone, Copy)]
struct Hole {
    side: Side,
    a0: usize,
    b0: usize,
    w: usize,
    h: usize,
}

impl Hole {
    fn contains_cell(&self, side: Side, a: usize, b: usize) -> bool {
        side == self.side && a >= self.a0 && a < self.a0 + self.w && b >= self.b0 && b < self.b0 + self.h
    }

    /// Contorno do buraco em ordem cíclica: (a, b) relativos ao canto do bloco.
    fn boundary(&self) -> Vec<(usize, usize)> {
        let (w, h) = (self.w, self.h);
        let mut v = Vec::new();
        for a in 0..w {
            v.push((a, 0));
        }
        for b in 0..h {
            v.push((w, b));
        }
        for a in (1..=w).rev() {
            v.push((a, h));
        }
        for b in (1..=h).rev() {
            v.push((0, b));
        }
        v
    }
}

pub(crate) struct HeadShape {
    pub c: Vec3,
    pub r: Vec3,
    taper: f32,
    chin_fwd: f32,
    chin_down: f32,
    face_len: f32,
    cheek: f32,
    nose_amp: f32,
    nose_sx: f32,
    nose_y: f32,
    lip_amp: f32,
    y_mouth: f32,
    y_brow: f32,
    y_chin_edge: f32,
    chin_pt: f32,
    age: f32,
}

impl HeadShape {
    fn new(d: &Dims) -> Self {
        let f = &d.p.face;
        let g = d.g;
        let ry = d.head_h * 0.5;
        let rx = lerp(0.1000, 0.0975, g) * (1.0 + 0.09 * f.face_width) * (d.head_h / 0.25);
        let rz = lerp(0.0960, 0.0925, g) * (d.head_h / 0.25);
        let cy = d.y_chin + ry;
        let cz = 0.004 + d.stoop(cy) * 0.9 + 0.03 * d.p.age * d.sc;
        let nose_scale = 1.0 + 0.30 * f.nose_size;
        HeadShape {
            c: Vec3::new(0.0, cy, cz),
            r: Vec3::new(rx, ry, rz),
            taper: lerp(0.20, 0.29, g) * (1.0 - 0.45 * f.jaw_width) * (1.0 - 0.10 * d.p.muscle) + 0.03 * f.chin_point,
            chin_fwd: 0.010 * (1.0 + 0.5 * f.chin_point) * d.sc,
            chin_down: 0.012 * f.chin_length * d.sc,
            face_len: 0.10 * f.face_length,
            cheek: 0.006 * (1.0 + 0.8 * f.cheek_fullness + 0.7 * d.p.weight.max(0.0) * 0.6 - 0.5 * d.p.age) * d.sc,
            nose_amp: lerp(0.0230, 0.0205, g) * nose_scale * d.sc,
            nose_sx: 0.0105 * (1.0 + 0.25 * f.nose_size),
            nose_y: d.y_chin + (0.352 + 0.030 * f.nose_height) * d.head_h,
            lip_amp: (0.0042 + 0.0022 * f.lip_fullness) * d.sc,
            y_mouth: d.y_chin + 0.235 * d.head_h,
            y_brow: d.y_chin + 0.645 * d.head_h,
            y_chin_edge: d.y_chin,
            chin_pt: f.chin_point,
            age: d.p.age,
        }
    }

    pub fn normal_at(&self, d: Vec3) -> Vec3 {
        Vec3::new(d.x / self.r.x, d.y / self.r.y, d.z / self.r.z).normalize_or_zero()
    }

    /// Ponto da superfície da cabeça na direção `d` (unitária) a partir do centro.
    pub fn surface(&self, d: Vec3) -> Vec3 {
        let up = smoothstep(0.1, 0.95, d.y);
        let low = smoothstep(0.15, 1.0, -d.y);
        let front = smoothstep(-0.1, 0.8, d.z);
        let back = smoothstep(0.0, 0.9, -d.z);
        let mut x = d.x * self.r.x;
        let mut y = d.y * self.r.y;
        let mut z = d.z * self.r.z;
        y *= 1.0 + 0.020 * up - 0.050 * smoothstep(0.62, 1.0, d.y);
        x *= 1.0 + 0.045 * smoothstep(0.0, 0.7, d.y) * (1.0 - smoothstep(0.7, 1.0, d.y));
        z *= 1.0 + 0.05 * back;
        // comprimento do rosto: estica a metade de baixo
        y *= 1.0 + self.face_len * low;
        // afunila a mandíbula (queixo pontudo no estilo anime)
        x *= 1.0 - self.taper * low.powf(1.25) * (0.30 + 0.70 * front);
        // queixo para frente/baixo
        z += self.chin_fwd * low * front * d.z.max(0.0) * 1.4;
        y -= self.chin_down * low * front;
        // bochechas: alargam o terço médio-baixo
        let cheek_w = smoothstep(-0.55, -0.1, d.y) * (1.0 - smoothstep(0.0, 0.5, d.y)) * smoothstep(0.1, 0.7, d.x.abs()) * (1.0 - smoothstep(0.7, 1.0, d.x.abs()));
        x += x.signum() * self.cheek * cheek_w * 0.8;
        z += self.cheek * 0.3 * cheek_w * front;
        // relevos do rosto (só na frente)
        let fm = smoothstep(0.50, 0.85, d.z);
        if fm > 0.0 {
            let yw = self.c.y + y; // altura absoluta aproximada
            let g = |t: f32| (-t * t).exp();
            // nariz: ponta + asas + ponte
            let tip = self.nose_amp * g(x / self.nose_sx) * g((yw - self.nose_y) / 0.0175);
            let wings = 0.55 * self.nose_amp * (g((x.abs() - 0.0105) / 0.0075)) * g((yw - (self.nose_y - 0.0045)) / 0.0075);
            let bridge = 0.40 * self.nose_amp * g(x / 0.0075) * g((yw - (self.nose_y + 0.032)) / 0.022);
            // sobrancelha (crista) e lábios
            let brow = 0.0030 * g((yw - self.y_brow) / 0.0085) * smoothstep(0.0, 0.03, x.abs() + 0.02);
            let lips = self.lip_amp * g(x / 0.021) * g((yw - self.y_mouth) / 0.0125);
            let chin = (0.0045 + 0.003 * self.chin_pt) * g(x / 0.016) * g((yw - (self.y_chin_edge + 0.020)) / 0.012);
            // olheiras/idade: depressão leve sob os olhos
            z += fm * (tip + wings + bridge + brow + lips + chin);
            z -= fm * self.age * 0.0025 * g((x.abs() - 0.03) / 0.015) * g((yw - (self.nose_y + 0.022)) / 0.010);
        }
        self.c + Vec3::new(x, y, z)
    }

    pub fn project(&self, p: Vec3) -> (Vec3, Vec3) {
        let q = (p - self.c) / self.r;
        let d = q.normalize_or_zero();
        (self.surface(d), self.normal_at(d))
    }
}

fn lattice_dir(i: usize, j: usize, k: usize) -> Vec3 {
    let q = Vec3::new(AX[i].to_radians().tan(), AY[j].to_radians().tan(), AZ[k].to_radians().tan());
    q.normalize()
}

pub(crate) struct HeadOut {
    pub neck_loop: Vec<u32>,
    pub shape_center: Vec3,
}

struct RingSpec {
    t: f32,
    sa: f32,
    sb: f32,
    delta: f32,
    /// peso do deslocamento do centro e do cisalhamento
    w: f32,
}

/// Anéis concêntricos dentro de um contorno de buraco. Devolve os anéis (índice 0 = contorno).
#[allow(clippy::too_many_arguments)]
fn annulus(
    b: &mut B,
    shape: &HeadShape,
    r0: &[u32],
    specs: &[RingSpec],
    center_shift: (f32, f32),
    shear: f32,
    delta_fn: &dyn Fn(usize, f32, f32) -> f32,
    back_slope: f32,
) -> Vec<Vec<u32>> {
    let pts = b.positions(r0);
    let e = pts.iter().copied().sum::<Vec3>() / pts.len() as f32;
    let n = shape.normal_at(((e - shape.c) / shape.r).normalize_or_zero());
    let hint = if e.x.abs() > 0.05 && n.x.abs() > 0.7 { Vec3::Z } else { Vec3::X };
    let mut t1 = (hint - n * n.dot(hint)).normalize_or_zero();
    let mut t2 = n.cross(t1).normalize_or_zero();
    if t2.y < 0.0 {
        t2 = -t2;
        t1 = -t1;
    }
    // orientação "para fora" (u > 0 = canto externo) para olhos; para a cabeça lateral usa a frente
    let out_sign = if e.x.abs() > 0.05 && n.x.abs() < 0.7 {
        e.x.signum() * t1.x.signum()
    } else if n.x.abs() > 0.7 {
        t1.z.signum()
    } else {
        1.0
    };
    let loc: Vec<(f32, f32)> = pts.iter().map(|p| ((*p - e).dot(t1), (*p - e).dot(t2))).collect();
    let a0 = loc.iter().map(|l| l.0.abs()).fold(1e-6, f32::max);
    let b0 = loc.iter().map(|l| l.1.abs()).fold(1e-6, f32::max);
    let mut rings = vec![r0.to_vec()];
    for (k, s) in specs.iter().enumerate() {
        let mut ids = Vec::with_capacity(pts.len());
        for (j, &(u, v)) in loc.iter().enumerate() {
            let (nu, nv) = (u / a0, v / b0);
            let len = (nu * nu + nv * nv).sqrt().max(1e-6);
            let (cu, cv) = (nu / len, nv / len);
            let (qu, qv) = (lerp(nu, cu, s.t), lerp(nv, cv, s.t));
            let u_out = qu * out_sign;
            let ru = qu * s.sa * a0 + center_shift.0 * s.w * out_sign;
            let rv = qv * s.sb * b0 + center_shift.1 * s.w + shear * u_out * a0 * s.w;
            let plan = e + t1 * ru + t2 * rv;
            let (sp, nrm) = shape.project(plan);
            let dl = delta_fn(k, qu * out_sign, qv) + s.delta;
            let mut p = sp + nrm * dl;
            if back_slope != 0.0 {
                p.z -= back_slope * dl;
            }
            let _ = j;
            ids.push(b.vert(p, Region::Head));
        }
        rings.push(ids);
    }
    rings
}

pub(crate) fn build_head(b: &mut B, d: &Dims) -> HeadOut {
    let shape = HeadShape::new(d);
    let fp = d.p.face;

    // --- reticulado ---
    let mut ids = vec![u32::MAX; N1 * N1 * N1];
    let at = |i: usize, j: usize, k: usize| (i * N1 + j) * N1 + k;
    for i in 0..N1 {
        for j in 0..N1 {
            for k in 0..N1 {
                let on_surface = i == 0 || i == C || j == 0 || j == C || k == 0 || k == C;
                if on_surface {
                    let dir = lattice_dir(i, j, k);
                    ids[at(i, j, k)] = b.vert(shape.surface(dir), Region::Head);
                }
            }
        }
    }
    let id = |s: Side, a: usize, bb: usize| -> u32 {
        let (i, j, k) = key(s, a, bb);
        ids[at(i, j, k)]
    };

    // --- buracos ---
    let neck = Hole { side: Side::Bottom, a0: 4, b0: 4, w: 5, h: 3 };
    let eye_l = Hole { side: Side::Front, a0: 8, b0: 5, w: 3, h: 3 };
    let eye_r = Hole { side: Side::Front, a0: 2, b0: 5, w: 3, h: 3 };
    let mouth = Hole { side: Side::Front, a0: 4, b0: 1, w: 5, h: 2 };
    let ear_l = Hole { side: Side::Right, a0: 4, b0: 4, w: 3, h: 3 };
    let ear_r = Hole { side: Side::Left, a0: 4, b0: 4, w: 3, h: 3 };
    let holes = [neck, eye_l, eye_r, mouth, ear_l, ear_r];

    for side in [Side::Front, Side::Back, Side::Right, Side::Left, Side::Top, Side::Bottom] {
        for a in 0..C {
            for bb in 0..C {
                if holes.iter().any(|h| h.contains_cell(side, a, bb)) {
                    continue;
                }
                b.quad(id(side, a, bb), id(side, a + 1, bb), id(side, a + 1, bb + 1), id(side, a, bb + 1));
            }
        }
    }

    let boundary_ids = |h: &Hole| -> Vec<u32> { h.boundary().iter().map(|&(a, bb)| id(h.side, h.a0 + a, h.b0 + bb)).collect() };

    // --- olhos ---
    let eye_size = 1.0 + 0.10 * fp.eye_size;
    let eye_w = 1.0 + 0.10 * fp.eye_width;
    let eye_h = 1.0 + 0.10 * fp.eye_height;
    let lid_specs = |sa: f32, sb: f32| {
        vec![
            RingSpec { t: 0.45, sa: 0.98, sb: 0.96, delta: 0.0013, w: 0.2 },
            RingSpec { t: 0.80, sa: 0.93 * sa, sb: 0.87 * sb, delta: 0.0020, w: 0.7 },
            RingSpec { t: 1.00, sa: 0.86 * sa, sb: 0.74 * sb, delta: -0.0030, w: 1.0 },
        ]
    };
    for (name, hole) in [("L", eye_l), ("R", eye_r)] {
        let r0 = boundary_ids(&hole);
        let shift = (0.0035 * fp.eye_distance, 0.0012 * fp.eye_height);
        let shear = 0.28 * fp.eye_tilt;
        let rings = annulus(
            b,
            &shape,
            &r0,
            &lid_specs(eye_size * eye_w, eye_size * eye_h),
            shift,
            shear,
            &|k, nu, _nv| if k == 2 { -0.0016 * smoothstep(0.55, 1.0, nu.abs()) } else { 0.0 },
            0.0,
        );
        for (k, r) in rings.iter().enumerate() {
            b.add_loop(format!("eye_{name}_ring{k}"), r.clone(), true);
        }
        // faces entre os anéis
        for k in 0..rings.len() - 1 {
            let (a, c) = (rings[k].clone(), rings[k + 1].clone());
            b.bridge(&a, &c);
        }
        b.mark(format!("eye_{name}_center"), rings[3].clone());
        b.mark(format!("eye_{name}_outer"), rings[0].clone());
    }

    // --- boca ---
    {
        let r0 = boundary_ids(&mouth);
        let mw = 1.08 + 0.12 * fp.mouth_width;
        let lf = 1.0 + 0.35 * fp.lip_fullness;
        let specs = vec![
            RingSpec { t: 0.60, sa: 0.86 * mw, sb: 0.80, delta: 0.0030 * lf, w: 0.3 },
            RingSpec { t: 1.00, sa: 0.66 * mw, sb: 0.50 * lf, delta: 0.0048 * lf, w: 0.7 },
            RingSpec { t: 1.00, sa: 0.58 * mw, sb: 0.07, delta: -0.0030, w: 1.0 },
        ];
        let rings = annulus(
            b,
            &shape,
            &r0,
            &specs,
            (0.0, 0.0030 * fp.mouth_height),
            0.0,
            &|k, nu, _nv| if k >= 1 { -0.0035 * smoothstep(0.45, 1.0, nu.abs()) } else { 0.0 },
            0.0,
        );
        for (k, r) in rings.iter().enumerate() {
            b.add_loop(format!("mouth_ring{k}"), r.clone(), true);
        }
        for k in 0..rings.len() - 1 {
            let (a, c) = (rings[k].clone(), rings[k + 1].clone());
            b.bridge(&a, &c);
        }
        b.mark("mouth_center", rings[3].clone());
    }

    // --- orelhas ---
    for (name, hole) in [("L", ear_l), ("R", ear_r)] {
        let r0 = boundary_ids(&hole);
        let sgn = if name == "L" { 1.0 } else { -1.0 };
        let es = 1.0 + 0.18 * fp.ear_size;
        let pt = fp.ear_point;
        let specs = vec![
            RingSpec { t: 0.9, sa: 1.10 * es, sb: 1.20 * es, delta: 0.0045, w: 0.0 },
            RingSpec { t: 1.0, sa: 0.90 * es, sb: 1.30 * es, delta: 0.0110, w: 0.0 },
            RingSpec { t: 1.0, sa: 0.62 * es, sb: 0.98 * es, delta: 0.0075, w: 0.0 },
        ];
        // topo da hélice sobe com ear_point (orelha élfica)
        let rings = annulus(
            b,
            &shape,
            &r0,
            &specs,
            (0.0, 0.0),
            0.0,
            &|k, nu, nv| if k == 1 { 0.0035 * smoothstep(0.2, 1.0, nv) * (1.0 + 2.2 * pt) } else { 0.0 },
            0.45,
        );
        for (k, r) in rings.iter().enumerate() {
            b.add_loop(format!("ear_{name}_ring{k}"), r.clone(), true);
        }
        for k in 0..rings.len() - 1 {
            let (a, c) = (rings[k].clone(), rings[k + 1].clone());
            b.bridge(&a, &c);
        }
        // tampa 3x3 (assoalho da concha)
        let last = rings[3].clone();
        let bd = hole.boundary();
        let mut grid = vec![vec![u32::MAX; 4]; 4];
        for (n, &(a, bb)) in bd.iter().enumerate() {
            grid[a][bb] = last[n];
        }
        let gp: Vec<Vec<Vec3>> = (0..4)
            .map(|a| (0..4).map(|bb| if grid[a][bb] != u32::MAX { b.pos[grid[a][bb] as usize] } else { Vec3::ZERO }).collect())
            .collect();
        let sgn_x = sgn;
        for a in 1..3 {
            for bb in 1..3 {
                let s = a as f32 / 3.0;
                let t = bb as f32 / 3.0;
                // interpolação transfinita (Coons)
                let p = gp[0][bb] * (1.0 - s) + gp[3][bb] * s + gp[a][0] * (1.0 - t) + gp[a][3] * t
                    - (gp[0][0] * (1.0 - s) * (1.0 - t) + gp[3][0] * s * (1.0 - t) + gp[0][3] * (1.0 - s) * t + gp[3][3] * s * t);
                let q = p - Vec3::X * sgn_x * 0.0030;
                grid[a][bb] = b.vert(q, Region::Head);
            }
        }
        for a in 0..3 {
            for bb in 0..3 {
                b.quad(grid[a][bb], grid[a + 1][bb], grid[a + 1][bb + 1], grid[a][bb + 1]);
            }
        }
        b.mark(format!("ear_{name}_center"), vec![grid[1][1], grid[2][1], grid[1][2], grid[2][2]]);
        b.mark(format!("ear_{name}_rim"), rings[2].clone());
    }

    // --- pescoço: contorno do buraco (anel 0 do tronco) ---
    let neck_loop = boundary_ids(&neck);

    // --- laços/landmarks da cabeça (reticulado) ---
    let chain = |pts: Vec<(Side, usize, usize)>| -> Vec<u32> { pts.into_iter().map(|(s, a, bb)| id(s, a, bb)).collect() };
    // linha da sobrancelha: j = 9 sobre a face frontal, colunas 2..11
    b.add_loop("brow_line", chain((2..=11).map(|a| (Side::Front, a, 9)).collect()), false);
    // anel das asas do nariz e anel da ponta
    let mut nose_ring = Vec::new();
    for a in 5..=8 {
        nose_ring.push((Side::Front, a, 3));
    }
    for bb in 4..=5 {
        nose_ring.push((Side::Front, 8, bb));
    }
    for a in (5..=8).rev() {
        nose_ring.push((Side::Front, a, 6));
    }
    for bb in (4..=5).rev() {
        nose_ring.push((Side::Front, 5, bb));
    }
    b.add_loop("nose_alar_ring", chain(nose_ring), true);
    b.add_loop(
        "nose_tip_ring",
        chain(vec![(Side::Front, 6, 4), (Side::Front, 7, 4), (Side::Front, 7, 5), (Side::Front, 6, 5)]),
        true,
    );
    // linha da mandíbula: perímetro da face inferior, sem passar pelo buraco do pescoço
    let mut jaw = Vec::new();
    for a in 0..=C {
        jaw.push((Side::Bottom, a, 0));
    }
    for bb in 1..=C {
        jaw.push((Side::Bottom, C, bb));
    }
    for a in (0..C).rev() {
        jaw.push((Side::Bottom, a, C));
    }
    for bb in (1..C).rev() {
        jaw.push((Side::Bottom, 0, bb));
    }
    b.add_loop("jaw_base_loop", chain(jaw), true);
    // anel do queixo (linha da frente da face inferior)
    b.add_loop("chin_line", chain((0..=C).map(|a| (Side::Front, a, 0)).collect()), false);
    // landmarks
    b.mark("head_top", vec![id(Side::Top, 6, 6), id(Side::Top, 7, 6), id(Side::Top, 6, 7), id(Side::Top, 7, 7)]);
    b.mark("chin", vec![id(Side::Front, 6, 0), id(Side::Front, 7, 0)]);
    b.mark("nose_tip", vec![id(Side::Front, 6, 4), id(Side::Front, 7, 4), id(Side::Front, 6, 5), id(Side::Front, 7, 5)]);
    // couro cabeludo: topo, costas e laterais acima da linha da orelha, testa acima da sobrancelha
    let mut scalp = Vec::new();
    for i in 0..N1 {
        for j in 0..N1 {
            for k in 0..N1 {
                if ids[at(i, j, k)] == u32::MAX {
                    continue;
                }
                let front = k == C;
                let ok = if front { j >= 10 } else { j >= 6 || k < 5 && j >= 4 };
                if ok {
                    scalp.push(ids[at(i, j, k)]);
                }
            }
        }
    }
    b.mark("hair_scalp", scalp);
    HeadOut {
        neck_loop,
        shape_center: shape.c,
    }
}
