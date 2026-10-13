//! Expressões VRM por deformação de landmarks (qualidade "média": a topologia é correta, o
//! acabamento artístico de cantos de boca e pálpebras é do usuário). Cada expressão é um
//! `ExprSpec`; `deform` gera as posições deformadas do Skin a partir de uma pose de repouso,
//! usando só landmarks, laços e máscaras do contrato (funciona para qualquer rosto esculpido).

use crate::cage::Region;
use crate::params::smoothstep;
use crate::parts::eye_geom;
use crate::skeleton::Skeleton;
use crate::topology::Topology;
use glam::{Quat, Vec3};

#[derive(Debug, Clone, Copy, Default)]
pub struct ExprSpec {
    /// 0..1: fecha as pálpebras do olho esquerdo (+x) e/ou direito.
    pub close_l: f32,
    pub close_r: f32,
    /// Fração da altura da abertura: + abre (pálpebra superior sobe), - fecha.
    pub upper_shift: f32,
    /// + sobe a pálpebra inferior (bochecha levanta).
    pub lower_shift: f32,
    /// Desloca as duas pálpebras juntas (olhar para cima/baixo), fração da altura.
    pub lid_follow: f32,
    pub cheek_up: f32,
    /// Rotação da mandíbula em radianos (abre a boca).
    pub jaw: f32,
    /// Fator em x dos lábios (1 = igual).
    pub mouth_x: f32,
    /// Lábios para frente (m).
    pub mouth_fwd: f32,
    pub corner_up: f32,
    pub corner_dx: f32,
    pub brow_dy: f32,
    pub brow_inner_dy: f32,
    pub brow_outer_dy: f32,
    /// Sobrancelhas para o centro (m).
    pub brow_in: f32,
}

impl ExprSpec {
    pub const NEUTRAL: ExprSpec = ExprSpec {
        close_l: 0.0,
        close_r: 0.0,
        upper_shift: 0.0,
        lower_shift: 0.0,
        lid_follow: 0.0,
        cheek_up: 0.0,
        jaw: 0.0,
        mouth_x: 1.0,
        mouth_fwd: 0.0,
        corner_up: 0.0,
        corner_dx: 0.0,
        brow_dy: 0.0,
        brow_inner_dy: 0.0,
        brow_outer_dy: 0.0,
        brow_in: 0.0,
    };
}

/// As expressões do contrato: 16 (neutral + 15 deformações). Nomes seguem os presets VRM 1.0.
pub fn expression_specs() -> Vec<(&'static str, ExprSpec)> {
    let n = ExprSpec::NEUTRAL;
    vec![
        ("neutral", n),
        (
            "happy",
            ExprSpec { upper_shift: -0.16, lower_shift: 0.38, cheek_up: 0.7, corner_up: 0.011, corner_dx: 0.006, mouth_x: 1.06, jaw: 0.05, brow_dy: 0.002, ..n },
        ),
        (
            "angry",
            ExprSpec { upper_shift: -0.24, lower_shift: 0.10, brow_dy: -0.004, brow_in: 0.006, brow_inner_dy: -0.003, corner_up: -0.004, mouth_x: 0.96, ..n },
        ),
        (
            "sad",
            ExprSpec { upper_shift: -0.12, brow_inner_dy: 0.006, brow_outer_dy: -0.003, corner_up: -0.008, mouth_x: 0.97, jaw: 0.02, ..n },
        ),
        (
            "relaxed",
            ExprSpec { upper_shift: -0.30, lower_shift: 0.12, corner_up: 0.005, mouth_x: 1.02, brow_dy: 0.001, ..n },
        ),
        (
            "surprised",
            ExprSpec { upper_shift: 0.30, brow_dy: 0.007, jaw: 0.20, mouth_x: 0.90, ..n },
        ),
        ("aa", ExprSpec { jaw: 0.30, mouth_x: 1.0, corner_dx: -0.002, ..n }),
        ("ih", ExprSpec { jaw: 0.09, mouth_x: 1.12, corner_dx: 0.005, ..n }),
        ("ou", ExprSpec { jaw: 0.12, mouth_x: 0.66, mouth_fwd: 0.008, ..n }),
        ("ee", ExprSpec { jaw: 0.10, mouth_x: 1.10, corner_dx: 0.008, corner_up: 0.003, ..n }),
        ("oh", ExprSpec { jaw: 0.22, mouth_x: 0.78, mouth_fwd: 0.005, ..n }),
        ("blink", ExprSpec { close_l: 1.0, close_r: 1.0, ..n }),
        ("blinkLeft", ExprSpec { close_l: 1.0, ..n }),
        ("blinkRight", ExprSpec { close_r: 1.0, ..n }),
        ("lookUp", ExprSpec { lid_follow: 0.14, ..n }),
        ("lookDown", ExprSpec { lid_follow: -0.20, ..n }),
    ]
}

struct EyeRig {
    c: Vec3,
    right: Vec3,
    up: Vec3,
    n: Vec3,
    a: f32,
    b: f32,
    cen: Vec3,
    rad: f32,
}

fn eye_rig(topo: &Topology, rest: &[Vec3], s: &str) -> EyeRig {
    let ids = topo.loop_ids(&format!("eye_{s}_ring3"));
    let g = eye_geom(topo, rest, s);
    let c = ids.iter().map(|&i| rest[i as usize]).sum::<Vec3>() / ids.len().max(1) as f32;
    EyeRig {
        c,
        right: g.right,
        up: g.up,
        n: g.n,
        a: g.half_w.max(1e-4),
        b: g.half_h.max(1e-4),
        cen: g.center,
        rad: g.radius,
    }
}

/// Posições deformadas do Skin para uma expressão. Devolve também a rotação da mandíbula.
pub fn deform(topo: &Topology, rest: &[Vec3], skel: &Skeleton, spec: &ExprSpec) -> (Vec<Vec3>, f32) {
    let mut out = rest.to_vec();
    let eyes = [eye_rig(topo, rest, "L"), eye_rig(topo, rest, "R")];
    let close = [spec.close_l, spec.close_r];
    let mouth_ids = topo.loop_ids("mouth_ring3");
    let mc = mouth_ids.iter().map(|&i| rest[i as usize]).sum::<Vec3>() / mouth_ids.len().max(1) as f32;
    let a_m = mouth_ids.iter().map(|&i| (rest[i as usize].x - mc.x).abs()).fold(1e-4f32, f32::max);
    let jaw_bone = skel.index("jaw").unwrap_or(0) as u16;
    let hinge = skel.pos(jaw_bone as usize);
    let zone_mouth = crate::topology::zone_index(anigo_core::morph_catalog::AnatomicalZone::MouthLips);
    let zone_brow = crate::topology::zone_index(anigo_core::morph_catalog::AnatomicalZone::Eyebrows);
    let brow_x = {
        let ids = topo.loop_ids("brow_line");
        ids.iter().map(|&i| rest[i as usize].x.abs()).fold(1e-4f32, f32::max)
    };

    for v in 0..rest.len() {
        if topo.region[v] != Region::Head {
            continue;
        }
        let p = rest[v];
        let mut d = Vec3::ZERO;

        // pálpebras e bochechas
        for (k, e) in eyes.iter().enumerate() {
            let q = p - e.c;
            let (xe, ye, ze) = (q.dot(e.right), q.dot(e.up), q.dot(e.n));
            if q.length() > 0.09 || ze < -0.02 {
                continue;
            }
            let s = ((xe / e.a).powi(2) + (ye / e.b).powi(2)).sqrt();
            let w = 1.0 - smoothstep(1.0, 2.1, s);
            if w > 0.0 {
                let b_loc = e.b * (1.0 - (xe / e.a).powi(2)).max(0.0).sqrt();
                let line = -0.30 * b_loc;
                let mut dy = 0.0;
                if ye >= 0.0 {
                    dy += close[k] * (line - ye) + spec.upper_shift * b_loc;
                } else {
                    dy += close[k] * (line - ye) + spec.lower_shift * b_loc;
                }
                dy += spec.lid_follow * b_loc;
                d += e.up * (dy * w);
                d += e.n * (0.0008 * close[k] * w);
                // as pálpebras deslizam sobre o globo: nunca entram na esclera
                if d.length() > 1e-7 {
                    let pp = p + d - e.cen;
                    let r = pp.length();
                    let min_r = e.rad + 0.0016;
                    if r < min_r && r > 1e-6 {
                        d += pp / r * (min_r - r) * (w * 1.6).min(1.0);
                    }
                }
            }
            if spec.cheek_up != 0.0 && ye < -0.4 * e.b {
                let wc = 1.0 - smoothstep(1.4, 3.4, s);
                d += e.up * (spec.cheek_up * 0.0045 * wc);
            }
        }

        // boca: máscara de zona (distância no grafo), com corte por região
        let wm = topo.zones[v][zone_mouth];
        if wm > 0.0 {
            // lábios acompanham a rotação da mandíbula pelos pesos de skin (aplicado abaixo)
            d.x += (spec.mouth_x - 1.0) * (p.x - mc.x) * wm;
            d.z += spec.mouth_fwd * wm * (1.0 - (((p.x - mc.x).abs()) / (1.4 * a_m)).clamp(0.0, 1.0));
            let corner_w = {
                let dc = ((p.x.abs() - a_m).powi(2) + (p.y - mc.y).powi(2)).sqrt();
                1.0 - smoothstep(0.004, 0.026, dc)
            };
            d.y += spec.corner_up * corner_w;
            d.x += spec.corner_dx * p.x.signum() * corner_w;
        }

        // sobrancelhas (pele em volta da linha da sobrancelha)
        let wb = topo.zones[v][zone_brow];
        if wb > 0.0 {
            let t = (p.x.abs() / brow_x).clamp(0.0, 1.0);
            d.y += wb * (spec.brow_dy + spec.brow_inner_dy * (1.0 - t) + spec.brow_outer_dy * t);
            d.x -= p.x.signum() * wb * spec.brow_in * (1.0 - t);
        }
        out[v] = p + d;
    }

    // mandíbula: rotação em torno da dobradiça, ponderada pelo peso do osso `jaw`
    if spec.jaw != 0.0 {
        for v in 0..rest.len() {
            let wj: f32 = topo.weights[v].iter().filter(|(b, _)| *b == jaw_bone).map(|(_, w)| *w).sum();
            if wj > 0.0 {
                let q = Quat::from_axis_angle(Vec3::X, spec.jaw * wj);
                out[v] = hinge + q * (out[v] - hinge);
            }
        }
    }
    (out, spec.jaw)
}
