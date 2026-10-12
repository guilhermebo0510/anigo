//! UV autorais da Gen-0: cada região é um "chart" cilíndrico (cabeça, tronco, braços, pernas,
//! pés, dedos) encaixado num retângulo fixo do atlas [0, 1]². É um layout correto e sem
//! sobreposição entre charts, não um layout artístico: o usuário pode redesenhar os UV
//! depois (os UV não entram no hash de conectividade, só no hash de UV do contrato).

use crate::cage::Region;
use crate::topology::Topology;
use glam::Vec3;
use std::f32::consts::TAU;

pub const CHART_COUNT: usize = 18;

pub const CHART_NAMES: [&str; CHART_COUNT] = [
    "head", "trunk", "arm_L", "arm_R", "leg_L", "leg_R", "foot_L", "foot_R", "thumb_L", "index_L", "middle_L", "ring_L", "little_L", "thumb_R",
    "index_R", "middle_R", "ring_R", "little_R",
];

/// Retângulo (u0, v0, u1, v1) de cada chart no atlas.
pub fn chart_rect(c: usize) -> [f32; 4] {
    let pad = 0.004;
    let r = match c {
        0 => [0.0, 0.5, 0.5, 1.0],
        1 => [0.5, 0.5, 1.0, 1.0],
        2 => [0.0, 0.3, 0.5, 0.5],
        3 => [0.5, 0.3, 1.0, 0.5],
        4 => [0.0, 0.0, 0.25, 0.15],
        5 => [0.25, 0.0, 0.5, 0.15],
        6 => [0.5, 0.0, 0.75, 0.15],
        7 => [0.75, 0.0, 1.0, 0.15],
        8..=12 => {
            let k = (c - 8) as f32;
            [k * 0.1, 0.15, k * 0.1 + 0.1, 0.3]
        }
        _ => {
            let k = (c - 13) as f32;
            [0.5 + k * 0.1, 0.15, 0.6 + k * 0.1, 0.3]
        }
    };
    [r[0] + pad, r[1] + pad, r[2] - pad, r[3] - pad]
}

fn finger_slot(r: Region) -> usize {
    match r {
        Region::Thumb => 0,
        Region::Index => 1,
        Region::Middle => 2,
        Region::Ring => 3,
        _ => 4,
    }
}

pub fn compute_uvs(t: &Topology, pos: &[Vec3]) -> (Vec<[[f32; 2]; 4]>, Vec<u8>) {
    let nf = t.faces.len();
    // referências por chart
    let head_z = {
        let mut s = 0.0;
        let mut n: f32 = 0.0;
        for (i, r) in t.region.iter().enumerate() {
            if *r == Region::Head {
                s += pos[i].z;
                n += 1.0;
            }
        }
        s / n.max(1.0)
    };
    let centroid = |name: &str| -> Vec3 {
        let ids = t.mark(name);
        ids.iter().map(|&i| pos[i as usize]).sum::<Vec3>() / ids.len().max(1) as f32
    };
    let arm_c = [centroid("arm_L_ring3"), centroid("arm_R_ring3")];
    let leg_c = [centroid("leg_L_ring5"), centroid("leg_R_ring5")];
    let foot_c = [centroid("foot_L_arch"), centroid("foot_R_arch")];

    struct Finger {
        base: Vec3,
        dir: Vec3,
    }
    let mut fingers: Vec<Option<Finger>> = (0..10).map(|_| None).collect();
    for (si, s) in ["L", "R"].iter().enumerate() {
        for (fi, name) in ["thumb", "index", "middle", "ring", "little"].iter().enumerate() {
            let base = centroid(&format!("finger_{s}_{name}_ring0"));
            let tip_ids = t.mark(&format!("finger_{s}_{name}_tip"));
            if let Some(&tip) = tip_ids.first() {
                let dir = (pos[tip as usize] - base).normalize_or_zero();
                fingers[si * 5 + fi] = Some(Finger { base, dir });
            }
        }
    }

    let mut chart_of_face = vec![0u8; nf];
    let mut raw: Vec<[[f32; 2]; 4]> = vec![[[0.0; 2]; 4]; nf];
    for (fi, f) in t.faces.iter().enumerate() {
        let regs: Vec<Region> = f.iter().map(|&v| t.region[v as usize]).collect();
        let reg = regs.iter().copied().fold(regs[0], Region::merge);
        let center = f.iter().map(|&v| pos[v as usize]).sum::<Vec3>() * 0.25;
        let side = if center.x >= 0.0 { 0usize } else { 1usize };
        let chart = match reg {
            Region::Head | Region::Neck => 0,
            Region::Trunk => 1,
            Region::Arm | Region::Hand => 2 + side,
            Region::Leg => 4 + side,
            Region::Foot | Region::Toes => 6 + side,
            Region::Thumb | Region::Index | Region::Middle | Region::Ring | Region::Little => 8 + side * 5 + finger_slot(reg),
        };
        chart_of_face[fi] = chart as u8;
        let proj = |p: Vec3| -> (f32, f32) {
            match chart {
                0 => ((p.x).atan2(p.z - head_z) / TAU + 0.5, p.y),
                1 => ((p.x).atan2(p.z) / TAU + 0.5, p.y),
                2 | 3 => {
                    let c = arm_c[chart - 2];
                    ((p.z - c.z).atan2(p.y - c.y) / TAU + 0.5, p.x.abs())
                }
                4 | 5 => {
                    let c = leg_c[chart - 4];
                    ((p.x - c.x).atan2(p.z - c.z) / TAU + 0.5, p.y)
                }
                6 | 7 => {
                    let c = foot_c[chart - 6];
                    ((p.x - c.x).atan2(p.y - 0.03) / TAU + 0.5, p.z)
                }
                _ => {
                    let idx = chart - 8;
                    if let Some(fg) = &fingers[idx] {
                        let q = p - fg.base;
                        let along = q.dot(fg.dir);
                        let rad = q - fg.dir * along;
                        let a1 = (Vec3::Y - fg.dir * fg.dir.dot(Vec3::Y)).normalize_or_zero();
                        let a2 = fg.dir.cross(a1);
                        ((rad.dot(a2)).atan2(rad.dot(a1)) / TAU + 0.5, along)
                    } else {
                        (0.0, 0.0)
                    }
                }
            }
        };
        let cp = proj(center);
        for k in 0..4 {
            let (mut u, v) = proj(pos[f[k] as usize]);
            // desembrulha em relação ao centro da face
            let mut d = u - cp.0;
            d -= d.round();
            u = cp.0 + d;
            raw[fi][k] = [u, v];
        }
    }
    // normaliza por chart
    let mut mn = vec![[f32::MAX; 2]; CHART_COUNT];
    let mut mx = vec![[f32::MIN; 2]; CHART_COUNT];
    for fi in 0..nf {
        let c = chart_of_face[fi] as usize;
        for k in 0..4 {
            for a in 0..2 {
                mn[c][a] = mn[c][a].min(raw[fi][k][a]);
                mx[c][a] = mx[c][a].max(raw[fi][k][a]);
            }
        }
    }
    let mut uv = vec![[[0.0f32; 2]; 4]; nf];
    for fi in 0..nf {
        let c = chart_of_face[fi] as usize;
        let rect = chart_rect(c);
        for k in 0..4 {
            let ur = (mx[c][0] - mn[c][0]).max(1e-6);
            let vr = (mx[c][1] - mn[c][1]).max(1e-6);
            let uu = (raw[fi][k][0] - mn[c][0]) / ur;
            let vv = (raw[fi][k][1] - mn[c][1]) / vr;
            uv[fi][k] = [rect[0] + uu * (rect[2] - rect[0]), rect[1] + vv * (rect[3] - rect[1])];
        }
    }
    (uv, chart_of_face)
}
