//! Pesos de skin iniciais: posse "dura" de cada vértice por um osso (região + geometria de
//! referência), suavizada por difusão no grafo da malha (Jacobi) com orçamento de iterações por
//! região (ombros/quadris largos, dedos curtos), mais redistribuição para os ossos de twist.
//! Resultado: soma 1, no máximo 4 influências, transições suaves nas articulações e nenhum
//! vazamento entre membros (a difusão só alcança poucas arestas e os lados esquerdo/direito
//! são isolados no final).

use crate::cage::Region;
use crate::params::smoothstep;
use crate::skeleton::Skeleton;
use crate::topology::Topology;
use glam::Vec3;

pub const MAX_INFLUENCES: usize = 4;

fn is_left_bone(name: &str) -> bool {
    name.starts_with("left") || name.starts_with("tw_left") || name == "bust_L"
}
fn is_right_bone(name: &str) -> bool {
    name.starts_with("right") || name.starts_with("tw_right") || name == "bust_R"
}

pub fn compute_weights(t: &Topology, pos: &[Vec3], skel: &Skeleton) -> Vec<[(u16, f32); MAX_INFLUENCES]> {
    let n = t.n_verts;
    let nb = skel.len();
    let idx = |name: &str| -> usize { skel.index(name).unwrap_or(0) };
    let jp = |name: &str| -> Vec3 { skel.pos(idx(name)) };

    let hips_y = jp("hips").y;
    let spine_y = jp("spine").y;
    let chest_y = jp("chest").y;
    let upper_y = jp("upperChest").y;
    let _ = hips_y;
    let elbow_x = jp("leftLowerArm").x;
    let knee_y = jp("leftLowerLeg").y;
    let mouth_y = t
        .landmark("lm_mouth_center")
        .map(|l| l.eval(pos).y)
        .unwrap_or(jp("head").y + 0.1);
    let jaw_p = jp("jaw");

    // posse dura + orçamento de difusão
    let mut owner = vec![0usize; n];
    let mut budget = vec![0usize; n];
    for v in 0..n {
        let p = pos[v];
        let r = t.region[v];
        let a = t.attr[v];
        let left = p.x >= 0.0;
        let side = if left { "left" } else { "right" };
        let (o, b): (usize, usize) = match r {
            Region::Head => {
                if p.y < mouth_y + 0.004 && p.z > jaw_p.z + 0.012 {
                    (idx("jaw"), 6)
                } else {
                    (idx("head"), 6)
                }
            }
            Region::Neck => (idx("neck"), 10),
            Region::Trunk => {
                let row = a[0];
                let lateral = a[1].abs() > 0.55;
                if (row == 3.0 || row == 4.0) && lateral {
                    (idx(&format!("{side}Shoulder")), 12)
                } else if p.y < spine_y {
                    (idx("hips"), 14)
                } else if p.y < chest_y {
                    (idx("spine"), 14)
                } else if p.y < upper_y {
                    (idx("chest"), 14)
                } else {
                    (idx("upperChest"), 14)
                }
            }
            Region::Arm => {
                if p.x.abs() < elbow_x.abs() {
                    (idx(&format!("{side}UpperArm")), 10)
                } else {
                    (idx(&format!("{side}LowerArm")), 10)
                }
            }
            Region::Hand => (idx(&format!("{side}Hand")), 8),
            Region::Thumb => {
                let k = a[3];
                let nm = if k < 2.0 {
                    "ThumbMetacarpal"
                } else if k < 4.0 {
                    "ThumbProximal"
                } else {
                    "ThumbDistal"
                };
                (idx(&format!("{side}{nm}")), 3)
            }
            Region::Index | Region::Middle | Region::Ring | Region::Little => {
                let k = a[3];
                let part = if k < 2.0 {
                    "Proximal"
                } else if k < 4.0 {
                    "Intermediate"
                } else {
                    "Distal"
                };
                let fname = match r {
                    Region::Index => "Index",
                    Region::Middle => "Middle",
                    Region::Ring => "Ring",
                    _ => "Little",
                };
                (idx(&format!("{side}{fname}{part}")), 3)
            }
            Region::Leg => {
                if p.y > knee_y {
                    (idx(&format!("{side}UpperLeg")), 10)
                } else {
                    (idx(&format!("{side}LowerLeg")), 10)
                }
            }
            Region::Foot => (idx(&format!("{side}Foot")), 6),
            Region::Toes => (idx(&format!("{side}Toes")), 3),
        };
        owner[v] = o;
        budget[v] = b;
    }

    // difusão de Jacobi
    let mut w: Vec<Vec<f32>> = (0..n)
        .map(|v| {
            let mut x = vec![0.0f32; nb];
            x[owner[v]] = 1.0;
            x
        })
        .collect();
    let max_iter = *budget.iter().max().unwrap_or(&0);
    for it in 0..max_iter {
        let prev = w.clone();
        for v in 0..n {
            if it >= budget[v] {
                continue;
            }
            let nbrs = &t.adjacency[v];
            if nbrs.is_empty() {
                continue;
            }
            let inv = 1.0 / nbrs.len() as f32;
            for k in 0..nb {
                let mut s = 0.0;
                for &u in nbrs {
                    s += prev[u as usize][k];
                }
                w[v][k] = 0.5 * prev[v][k] + 0.5 * s * inv;
            }
        }
    }

    // isolamento esquerda/direita e twist
    let tw = |name: &str| -> Option<usize> { skel.index(name) };
    for v in 0..n {
        let p = pos[v];
        if p.x.abs() > 1e-4 {
            for k in 0..nb {
                let name = &skel.bones[k].name;
                let wrong = if p.x > 0.0 { is_right_bone(name) } else { is_left_bone(name) };
                if wrong {
                    w[v][k] = 0.0;
                }
            }
        }
        // twist: a parcela do segmento vira twist conforme a posição ao longo do segmento
        for side in ["left", "right"] {
            let segs: [(&str, &str); 4] = [("UpperArm", "LowerArm"), ("LowerArm", "Hand"), ("UpperLeg", "LowerLeg"), ("LowerLeg", "Foot")];
            for (a, b) in segs {
                let ia = idx(&format!("{side}{a}"));
                let ib = idx(&format!("{side}{b}"));
                let Some(itw) = tw(&format!("tw_{side}{a}")) else { continue };
                let (pa, pb) = (skel.pos(ia), skel.pos(ib));
                let ab = pb - pa;
                let len2 = ab.length_squared().max(1e-9);
                let tpar = ((p - pa).dot(ab) / len2).clamp(0.0, 1.0);
                let m = smoothstep(0.15, 0.65, tpar) * 0.92;
                let share = w[v][ia];
                if share > 0.0 {
                    w[v][itw] += share * m;
                    w[v][ia] = share * (1.0 - m);
                }
            }
        }
    }

    // limpeza: influências desprezíveis somem; vértices da linha central não recebem ossos laterais
    // (evita empates esquerda/direita na poda e mantém a simetria exata)
    for v in 0..n {
        let central = pos[v].x.abs() <= 1e-4;
        for k in 0..nb {
            let name = &skel.bones[k].name;
            if w[v][k] < 0.01 || (central && (is_left_bone(name) || is_right_bone(name))) {
                w[v][k] = 0.0;
            }
        }
        if w[v].iter().sum::<f32>() <= 1e-9 {
            w[v][owner[v]] = 1.0;
        }
    }

    // poda para 4 influências, normaliza
    let mut out = Vec::with_capacity(n);
    for v in 0..n {
        let mut ix: Vec<usize> = (0..nb).collect();
        ix.sort_by(|&a, &b| w[v][b].partial_cmp(&w[v][a]).unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(&b)));
        let mut res = [(0u16, 0.0f32); MAX_INFLUENCES];
        let mut sum = 0.0;
        for k in 0..MAX_INFLUENCES {
            res[k] = (ix[k] as u16, w[v][ix[k]]);
            sum += w[v][ix[k]];
        }
        if sum <= 1e-9 {
            res = [(owner[v] as u16, 1.0), (0, 0.0), (0, 0.0), (0, 0.0)];
        } else {
            for r in res.iter_mut() {
                r.1 /= sum;
            }
        }
        // zera influências nulas apontando para o osso do dono (índice válido)
        for r in res.iter_mut() {
            if r.1 <= 0.0 {
                r.0 = owner[v] as u16;
                r.1 = 0.0;
            }
        }
        out.push(res);
    }
    out
}
