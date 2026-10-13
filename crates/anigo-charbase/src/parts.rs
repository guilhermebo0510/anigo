//! Peças auxiliares do rosto, como submalhas separadas: olhos em camadas (esclera, íris côncava,
//! brilho), cílios, sobrancelhas e interior da boca (cavidade, dentes, língua).
//!
//! Cada peça é uma função das posições do Skin (âncoras pelos mesmos localizadores/laços do
//! contrato), então acompanha qualquer morph ou escultura. `rest` posiciona os olhos e o
//! interior da boca (não dependem da expressão); `current` posiciona cílios, sobrancelhas e a
//! borda da boca (seguem as pálpebras e os lábios deformados). `jaw` (rad) gira os dentes
//! inferiores e a língua em torno da articulação da mandíbula.

use crate::params::smoothstep;
use crate::topology::{Locator, Topology};
use glam::{Quat, Vec3};
use std::f32::consts::{PI, TAU};
use std::sync::OnceLock;

pub const PART_NAMES: [&str; 14] = [
    "eye_white_L",
    "eye_white_R",
    "eye_iris_L",
    "eye_iris_R",
    "eye_highlight_L",
    "eye_highlight_R",
    "lashes_L",
    "lashes_R",
    "brow_L",
    "brow_R",
    "mouth_inside",
    "teeth_upper",
    "teeth_lower",
    "tongue",
];

pub fn part_material(name: &str) -> &'static str {
    if name.starts_with("eye_white") {
        "eye_white"
    } else if name.starts_with("eye_iris") {
        "eye_iris"
    } else if name.starts_with("eye_highlight") {
        "eye_highlight"
    } else if name.starts_with("lashes") {
        "lash"
    } else if name.starts_with("brow") {
        "brow"
    } else if name == "mouth_inside" {
        "mouth_inside"
    } else if name.starts_with("teeth") {
        "teeth"
    } else {
        "tongue"
    }
}

#[derive(Debug, Clone)]
pub struct Part {
    pub name: String,
    pub material: &'static str,
    pub positions: Vec<Vec3>,
    pub tris: Vec<[u32; 3]>,
    pub uv: Vec<[f32; 2]>,
}

struct Frame {
    c: Vec3,
    right: Vec3,
    up: Vec3,
    n: Vec3,
}

fn loop_pos(pos: &[Vec3], ids: &[u32]) -> Vec<Vec3> {
    ids.iter().map(|&i| pos[i as usize]).collect()
}

fn frame_of(pts: &[Vec3]) -> Frame {
    let c = pts.iter().copied().sum::<Vec3>() / pts.len().max(1) as f32;
    let mut n = Vec3::ZERO;
    for k in 0..pts.len() {
        let (a, b) = (pts[k] - c, pts[(k + 1) % pts.len()] - c);
        n += a.cross(b);
    }
    let mut n = n.normalize_or_zero();
    if n.z < 0.0 {
        n = -n;
    }
    if n == Vec3::ZERO {
        n = Vec3::Z;
    }
    let right = Vec3::Y.cross(n).normalize_or_zero();
    let right = if right == Vec3::ZERO { Vec3::X } else { right };
    let up = n.cross(right).normalize_or_zero();
    Frame { c, right, up, n }
}

struct Builder {
    positions: Vec<Vec3>,
    tris: Vec<[u32; 3]>,
    uv: Vec<[f32; 2]>,
}

impl Builder {
    fn new() -> Self {
        Builder {
            positions: Vec::new(),
            tris: Vec::new(),
            uv: Vec::new(),
        }
    }
    fn v(&mut self, p: Vec3, uv: [f32; 2]) -> u32 {
        self.positions.push(p);
        self.uv.push(uv);
        (self.positions.len() - 1) as u32
    }
    fn quad(&mut self, a: u32, b: u32, c: u32, d: u32) {
        self.tris.push([a, b, c]);
        self.tris.push([a, c, d]);
    }
    fn finish(self, name: &str) -> Part {
        Part {
            name: name.to_string(),
            material: part_material(name),
            positions: self.positions,
            tris: self.tris,
            uv: self.uv,
        }
    }
}

/// Esfera UV (polo frontal = `n`). Devolve os vértices por anel para ligar o resto.
fn uv_sphere(c: Vec3, r: f32, f: &Frame, lat: usize, lon: usize) -> Builder {
    let mut b = Builder::new();
    let front = b.v(c + f.n * r, [0.5, 0.0]);
    let mut rings: Vec<Vec<u32>> = Vec::new();
    for i in 1..lat {
        let th = PI * i as f32 / lat as f32;
        let mut ring = Vec::new();
        for j in 0..lon {
            let ph = TAU * j as f32 / lon as f32;
            let d = f.n * th.cos() + (f.right * ph.cos() + f.up * ph.sin()) * th.sin();
            ring.push(b.v(c + d * r, [j as f32 / lon as f32, th / PI]));
        }
        rings.push(ring);
    }
    let back = b.v(c - f.n * r, [0.5, 1.0]);
    for j in 0..lon {
        let j2 = (j + 1) % lon;
        b.tris.push([front, rings[0][j2], rings[0][j]]);
        for i in 0..rings.len() - 1 {
            b.quad(rings[i][j], rings[i][j2], rings[i + 1][j2], rings[i + 1][j]);
        }
        let last = rings.len() - 1;
        b.tris.push([back, rings[last][j], rings[last][j2]]);
    }
    for t in b.tris.iter_mut() {
        t.swap(1, 2);
    }
    b
}

fn eye_center_locator(topo: &Topology, s: &str) -> Locator {
    topo.eye_center_locator(s)
}

#[derive(Debug, Clone, Copy)]
pub struct EyeGeom {
    pub center: Vec3,
    pub radius: f32,
    pub n: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub half_w: f32,
    pub half_h: f32,
}

/// Geometria do globo ocular (centro, raio, eixos) para o olho `L` ou `R`.
pub fn eye_geom(topo: &Topology, rest: &[Vec3], s: &str) -> EyeGeom {
    let ids3 = topo.loop_ids(&format!("eye_{s}_ring3"));
    let pts = loop_pos(rest, &ids3);
    let fr = frame_of(&pts);
    let center = eye_center_locator(topo, s).eval(rest);
    let radius = pts.iter().map(|p| (*p - center).length()).fold(0.0f32, f32::max) * 1.03;
    let n = (fr.c - center).normalize_or_zero();
    let n = if n == Vec3::ZERO { Vec3::Z } else { n };
    let right = Vec3::Y.cross(n).normalize_or_zero();
    let up = n.cross(right).normalize_or_zero();
    let half_w = pts.iter().map(|p| (*p - fr.c).dot(right).abs()).fold(0.0f32, f32::max);
    let half_h = pts.iter().map(|p| (*p - fr.c).dot(up).abs()).fold(0.0f32, f32::max);
    EyeGeom {
        center,
        radius,
        n,
        right,
        up,
        half_w,
        half_h,
    }
}

pub fn build_parts(topo: &Topology, rest: &[Vec3], current: &[Vec3], jaw: f32) -> Vec<Part> {
    let mut parts: Vec<Part> = Vec::new();
    let vn = {
        let m = crate::topology::quad_mesh_of(topo, current);
        m.vertex_normals()
    };

    // --- olhos ---
    let mut eye_geoms: Vec<EyeGeom> = Vec::new();
    for s in ["L", "R"] {
        eye_geoms.push(eye_geom(topo, rest, s));
    }
    for (k, s) in ["L", "R"].iter().enumerate() {
        let g = eye_geoms[k];
        let f = Frame {
            c: g.center,
            right: g.right,
            up: g.up,
            n: g.n,
        };
        // esclera
        let b = uv_sphere(g.center, g.radius, &f, 12, 16);
        parts.push(b.finish(&format!("eye_white_{s}")));
        // íris côncava, na frente da esclera
        let r_iris = (0.80 * g.half_w).min(0.95 * g.radius);
        let mut ib = Builder::new();
        let center_v = {
            let p = g.center + g.n * (g.radius + 0.0003);
            ib.v(p, [0.5, 0.5])
        };
        let (rings, segs) = (5usize, 16usize);
        let mut ring_ids: Vec<Vec<u32>> = Vec::new();
        for ri in 1..=rings {
            let rho = r_iris * ri as f32 / rings as f32;
            let tt = rho / g.radius;
            let mut ids = Vec::new();
            for j in 0..segs {
                let ph = TAU * j as f32 / segs as f32;
                let dir = g.n * (1.0 - tt * tt).max(0.0).sqrt() + (g.right * ph.cos() + g.up * ph.sin()) * tt;
                // côncava: a borda fica 1.3 mm acima da esclera e o centro 0.3 mm
                let lift = 0.0003 + 0.0013 * (rho / r_iris).powi(2);
                let p = g.center + dir * (g.radius + lift);
                let q = ri as f32 / rings as f32; // exato: o UV não depende da geometria
                ids.push(ib.v(p, [0.5 + 0.5 * q * ph.cos(), 0.5 + 0.5 * q * ph.sin()]));
            }
            ring_ids.push(ids);
        }
        for j in 0..segs {
            let j2 = (j + 1) % segs;
            ib.tris.push([center_v, ring_ids[0][j], ring_ids[0][j2]]);
            for r in 0..rings - 1 {
                ib.quad(ring_ids[r][j], ring_ids[r + 1][j], ring_ids[r + 1][j2], ring_ids[r][j2]);
            }
        }
        parts.push(ib.finish(&format!("eye_iris_{s}")));
        // brilho: placa 3x3 na frente da íris, deslocada para cima e para o lado interno
        let mut hb = Builder::new();
        let inner = if *s == "L" { -1.0 } else { 1.0 };
        let hc = g.center + g.n * (g.radius + 0.0026) + g.right * (inner * 0.32 * r_iris) + g.up * (0.36 * r_iris);
        let hs = 0.26 * r_iris;
        let mut grid = [[0u32; 4]; 4];
        for (i, row) in grid.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                let (u, v) = (i as f32 / 3.0 - 0.5, j as f32 / 3.0 - 0.5);
                // levemente curvada sobre a esfera do olho
                let curve = -(u * u + v * v) * hs * 0.8;
                *cell = hb.v(hc + g.right * (u * 2.0 * hs) + g.up * (v * 2.0 * hs) + g.n * curve, [i as f32 / 3.0, j as f32 / 3.0]);
            }
        }
        for i in 0..3 {
            for j in 0..3 {
                hb.quad(grid[i][j], grid[i + 1][j], grid[i + 1][j + 1], grid[i][j + 1]);
            }
        }
        parts.push(hb.finish(&format!("eye_highlight_{s}")));
    }

    // --- cílios (seguem as pálpebras deformadas) ---
    for (k, s) in ["L", "R"].iter().enumerate() {
        let g = eye_geoms[k];
        let ids3 = topo.loop_ids(&format!("eye_{s}_ring3"));
        let n3 = ids3.len();
        let mut lb = Builder::new();
        let outer_sign = if *s == "L" { 1.0 } else { -1.0 };
        let c3 = ids3.iter().map(|&i| current[i as usize]).sum::<Vec3>() / n3 as f32;
        let add_chain = |chain: Vec<u32>, upper: bool, len: f32, lb: &mut Builder| {
            let m = chain.len();
            let mut rows: [Vec<u32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
            for (i, &vid) in chain.iter().enumerate() {
                let p = current[vid as usize];
                let t = i as f32 / (m - 1).max(1) as f32;
                let rad = p - g.center;
                let radial = (rad - g.n * rad.dot(g.n)).normalize_or_zero();
                let sx = (p - c3).dot(g.right) * outer_sign / g.half_w.max(1e-4);
                let l = len * (0.55 + 0.45 * (PI * t).sin().max(0.0).powf(0.7)) * (1.0 + 0.5 * smoothstep(0.3, 1.0, sx));
                let vdir = if upper { g.up } else { -g.up };
                let root = p + vn[vid as usize] * 0.0006;
                let d_mid = (radial * 0.85 + g.n * 0.30 + vdir * 0.10).normalize_or_zero();
                let d_tip = (radial * 0.70 + g.n * 0.55 + vdir * (if upper { 0.35 } else { 0.05 })).normalize_or_zero();
                let mid = root + d_mid * (0.55 * l);
                let tip = root + d_mid * (0.55 * l) + d_tip * (0.45 * l);
                rows[0].push(lb.v(root, [t, 0.0]));
                rows[1].push(lb.v(mid, [t, 0.55]));
                rows[2].push(lb.v(tip, [t, 1.0]));
            }
            for r in 0..2 {
                for i in 0..m - 1 {
                    lb.quad(rows[r][i], rows[r][i + 1], rows[r + 1][i + 1], rows[r + 1][i]);
                }
            }
        };
        // cadeia superior = vértices 10..=20 do anel expandido; inferior = 22,23,0..=8
        let upper: Vec<u32> = (10..=20).map(|i| ids3[i % n3]).collect();
        let lower: Vec<u32> = [22usize, 23].iter().chain((0..=8usize).collect::<Vec<_>>().iter()).map(|&i| ids3[i % n3]).collect();
        add_chain(upper, true, 0.0115 * (g.half_w / 0.019), &mut lb);
        add_chain(lower, false, 0.0045 * (g.half_w / 0.019), &mut lb);
        parts.push(lb.finish(&format!("lashes_{s}")));
    }

    // --- sobrancelhas ---
    {
        let brow = topo.loop_ids("brow_line");
        let nb = brow.len(); // 19
        for (s, range) in [("L", 10..19usize), ("R", 0..9usize)] {
            let chain: Vec<u32> = range.filter(|&i| i < nb).map(|i| brow[i]).collect();
            let m = chain.len();
            let mut bb = Builder::new();
            let mut rows: [Vec<u32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
            // da raiz do nariz (interno) para a têmpora (externo)
            let ordered: Vec<u32> = if s == "L" { chain.clone() } else { chain.iter().rev().copied().collect() };
            for (i, &vid) in ordered.iter().enumerate() {
                let t = i as f32 / (m - 1).max(1) as f32;
                let p = current[vid as usize];
                let thick = 0.55 + 0.45 * (PI * (0.15 + 0.7 * t)).sin();
                let arch = 0.0030 * (PI * t).sin();
                let n = vn[vid as usize];
                let slope = -0.0035 * t * t; // a cauda cai
                rows[0].push(bb.v(p + n * 0.0011 + Vec3::Y * (-0.0035 * thick + arch + slope), [t, 0.0]));
                rows[1].push(bb.v(p + n * 0.0016 + Vec3::Y * (0.0045 * thick + arch + slope), [t, 0.5]));
                rows[2].push(bb.v(p + n * 0.0012 + Vec3::Y * (0.0120 * thick + arch + slope), [t, 1.0]));
            }
            for r in 0..2 {
                for i in 0..m - 1 {
                    bb.quad(rows[r][i], rows[r][i + 1], rows[r + 1][i + 1], rows[r + 1][i]);
                }
            }
            parts.push(bb.finish(&format!("brow_{s}")));
        }
    }

    // --- interior da boca ---
    {
        let ids = topo.loop_ids("mouth_ring3");
        let n = ids.len(); // 28
        let cur = loop_pos(current, &ids);
        let fr = frame_of(&cur);
        let a3 = cur.iter().map(|p| (*p - fr.c).dot(fr.right).abs()).fold(1e-4, f32::max);
        // dobradiça da mandíbula: perto das orelhas, abaixo; usa o centro das orelhas e do pescoço
        let hinge = {
            let el = Locator::centroid(topo.mark("ear_L_center")).eval(rest);
            let er = Locator::centroid(topo.mark("ear_R_center")).eval(rest);
            let nt = Locator::centroid(topo.mark("neck_top_ring")).eval(rest);
            (el + er) * 0.4 + nt * 0.2
        };
        let rot = Quat::from_axis_angle(Vec3::X, jaw);
        let rotate = |p: Vec3, k: f32| -> Vec3 { hinge + Quat::from_axis_angle(Vec3::X, jaw * k) * (p - hinge) };
        let _ = rot;
        // cavidade: anéis mais fundos e mais altos que a fenda
        let wk = [1.0f32, 0.98, 0.92, 0.75, 0.42];
        let hk = [0.0f32, 0.008, 0.016, 0.017, 0.010];
        let dk = [0.0f32, 0.007, 0.018, 0.034, 0.048];
        let mut mb = Builder::new();
        let mut rings: Vec<Vec<u32>> = Vec::new();
        // anel 0 = os próprios vértices da borda (cópias na mesma posição, soldam visualmente)
        let mut ring0 = Vec::new();
        for (j, p) in cur.iter().enumerate() {
            ring0.push(mb.v(*p, [j as f32 / n as f32, 0.0]));
        }
        rings.push(ring0);
        for k in 1..wk.len() {
            let mut ring = Vec::new();
            for (j, p) in cur.iter().enumerate() {
                let u = (*p - fr.c).dot(fr.right);
                let v = (*p - fr.c).dot(fr.up);
                let ang = (v / (0.35 * a3).max(1e-4)).atan2(u / a3);
                let (cu, sv) = (ang.cos(), ang.sin());
                let q = fr.c + fr.right * (a3 * wk[k] * cu) + fr.up * (hk[k] * sv + (-0.004 * k as f32)) - fr.n * dk[k];
                let q = if jaw != 0.0 && sv < 0.0 { rotate(q, 1.0) } else { q };
                ring.push(mb.v(q, [j as f32 / n as f32, k as f32 / 4.0]));
            }
            rings.push(ring);
        }
        for k in 0..rings.len() - 1 {
            for j in 0..n {
                let j2 = (j + 1) % n;
                mb.quad(rings[k][j], rings[k][j2], rings[k + 1][j2], rings[k + 1][j]);
            }
        }
        let last = rings.len() - 1;
        let cc = mb.v(fr.c - fr.n * (dk[4] + 0.008) + fr.up * -0.004, [0.5, 1.0]);
        for j in 0..n {
            let j2 = (j + 1) % n;
            mb.tris.push([cc, rings[last][j2], rings[last][j]]);
        }
        parts.push(mb.finish("mouth_inside"));

        // dentes superiores (cadeia da fenda superior, 9 pontos) e inferiores
        let rest_ids = topo.loop_ids("mouth_ring3");
        let rest_pts = loop_pos(rest, &rest_ids);
        let rfr = frame_of(&rest_pts);
        let mk_teeth = |chain: std::ops::RangeInclusive<usize>, upper: bool, name: &str, k_jaw: f32| -> Part {
            let mut tb = Builder::new();
            let idxs: Vec<usize> = chain.collect();
            let m = idxs.len();
            let mut top = Vec::new();
            let mut bot = Vec::new();
            for (i, &ix) in idxs.iter().enumerate() {
                let p = rest_pts[ix % rest_pts.len()];
                let t = i as f32 / (m - 1).max(1) as f32;
                // as pontas da fileira entram na boca (arco)
                let tuck = 0.007 * (1.0 - (PI * t).sin());
                let dir = if upper { -1.0 } else { 1.0 };
                let a = p - rfr.n * (0.0025 + tuck) + rfr.up * (-0.0005 * dir);
                let b = a + rfr.up * (dir * 0.0075) - rfr.n * 0.0008;
                let (a, b) = if k_jaw != 0.0 { (rotate(a, k_jaw), rotate(b, k_jaw)) } else { (a, b) };
                top.push(tb.v(a, [t, 0.0]));
                bot.push(tb.v(b, [t, 1.0]));
            }
            for i in 0..m - 1 {
                tb.quad(top[i], top[i + 1], bot[i + 1], bot[i]);
            }
            tb.finish(name)
        };
        parts.push(mk_teeth(14..=22, true, "teeth_upper", 0.0));
        parts.push(mk_teeth(0..=8, false, "teeth_lower", 1.0));
        // língua: elipsoide achatado
        let mut tg = Builder::new();
        let tf = Frame {
            c: Vec3::ZERO,
            right: rfr.right,
            up: rfr.up,
            n: rfr.n,
        };
        let tc = rfr.c - rfr.n * 0.022 - rfr.up * 0.006;
        let sph = uv_sphere(Vec3::ZERO, 1.0, &tf, 6, 10);
        for (p, uv) in sph.positions.iter().zip(&sph.uv) {
            let q = tc + rfr.right * (p.dot(rfr.right) * a3 * 0.55) + rfr.up * (p.dot(rfr.up) * 0.0055) + rfr.n * (p.dot(rfr.n) * 0.020);
            tg.v(if jaw != 0.0 { rotate(q, 0.6) } else { q }, *uv);
        }
        tg.tris = sph.tris;
        parts.push(tg.finish("tongue"));
    }
    parts
}

static TOPO: OnceLock<Vec<(String, &'static str, Vec<[u32; 3]>, Vec<[f32; 2]>)>> = OnceLock::new();

/// (nome, material, triângulos, UV) de cada peça: idêntico para qualquer parâmetro.
pub fn parts_topology() -> &'static Vec<(String, &'static str, Vec<[u32; 3]>, Vec<[f32; 2]>)> {
    TOPO.get_or_init(|| {
        let topo = crate::topology::reference();
        let pos = crate::character::skin_positions(&crate::topology::reference_params());
        build_parts(topo, &pos, &pos, 0.0)
            .into_iter()
            .map(|p| (p.name, p.material, p.tris, p.uv))
            .collect()
    })
}
