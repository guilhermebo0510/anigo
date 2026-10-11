//! Ferramentas diversas do Edit Mode: Subdivide (arestas/faces), Poke, Rip,
//! Separate/Split, marcação em lote (sharp/seam/crease/bevel weight), sombreamento
//! suave/chapado, hide/reveal e helpers de seleção (`Select → Select All by Trait`).
//!
//! # Subdivide
//!
//! Cada aresta cortada ganha `cuts` vértices igualmente espaçados (compartilhados
//! pelas faces vizinhas, logo sem T-junctions). As faces que tocam arestas cortadas
//! são re-tesseladas conforme o padrão:
//!
//! | face | arestas cortadas | resultado |
//! |---|---|---|
//! | quad | as 4 | grade `(cuts+1)²` de quads (interior por superfície de Coons) |
//! | quad | 2 opostas | `cuts+1` tiras de quads (escada) |
//! | triângulo | as 3 | `(cuts+1)²` triângulos (malha triangular) |
//! | qualquer outra combinação | — | a face continua uma só, agora um n-gon com os vértices novos (colineares) no contorno |
//!
//! `subdivide_faces` ainda divide triângulos e n-gons selecionados em quads
//! (estilo Catmull-Clark: centro + pontos médios das arestas), exigindo `cuts`
//! ímpar nesses casos.
//!
//! Atributos: vértices novos interpolam cor/skin/máscara/crease dos extremos
//! (`EVert::lerp`; o interior das grades, bilinear); UV e normais customizadas por corner
//! são interpoladas dentro de cada face; sharp/seam/crease/bevel de uma aresta
//! cortada passam para todos os seus segmentos.

use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, Corner, EVert, EdgeAttr, EdgeKey, EditMesh, Face, Remap};
use crate::normals::compute_vertex_normals;
use crate::ops::delete::{apply_delete, unique_edges, unique_faces, unique_verts, DeletePlan};
use crate::ops::fill::coons_point;
use crate::ops::smooth::XorShift64;
use crate::ops::OpOutput;
use crate::select::Selection;
use crate::topology::Topology;
use glam::{Vec2, Vec3};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Máximo de cortes por aresta (limite de sanidade).
pub const MAX_CUTS: usize = 64;

// ---------------------------------------------------------------- interpolação helpers

fn lerp_normal(a: Option<Vec3>, b: Option<Vec3>, t: f32) -> Option<Vec3> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.lerp(y, t).try_normalize().unwrap_or(x)),
        _ => None,
    }
}

fn lerp_corner(a: &Corner, b: &Corner, t: f32, vert: u32) -> Corner {
    Corner {
        vert,
        uv: a.uv.lerp(b.uv, t),
        normal: lerp_normal(a.normal, b.normal, t),
    }
}

/// Interpolação bilinear de cantos: `c00` (u=0,v=0), `c10` (u=1,v=0), `c01` (u=0,v=1), `c11`.
fn bilerp_corner(
    c00: &Corner,
    c10: &Corner,
    c01: &Corner,
    c11: &Corner,
    u: f32,
    v: f32,
    vert: u32,
) -> Corner {
    let uv = c00.uv * ((1.0 - u) * (1.0 - v))
        + c10.uv * (u * (1.0 - v))
        + c01.uv * ((1.0 - u) * v)
        + c11.uv * (u * v);
    let normal = match (c00.normal, c10.normal, c01.normal, c11.normal) {
        (Some(a), Some(b), Some(c), Some(d)) => {
            let n = a * ((1.0 - u) * (1.0 - v))
                + b * (u * (1.0 - v))
                + c * ((1.0 - u) * v)
                + d * (u * v);
            Some(n.try_normalize().unwrap_or(a))
        }
        _ => None,
    };
    Corner { vert, uv, normal }
}

fn bilerp_vert(a: &EVert, b: &EVert, c: &EVert, d: &EVert, u: f32, v: f32) -> EVert {
    // a(0,0) b(1,0) c(0,1) d(1,1)
    let low = EVert::lerp(a, b, u);
    let high = EVert::lerp(c, d, u);
    EVert::lerp(&low, &high, v)
}

/// Média de vértices (cor/máscara/crease médios; skin do primeiro).
fn average_verts(mesh: &EditMesh, ids: &[u32], co: Vec3) -> EVert {
    let mut out = EVert::derived(&mesh.verts[ids[0] as usize], co);
    let n = ids.len() as f32;
    out.color = [0.0; 4];
    out.mask = 0.0;
    out.crease = 0.0;
    for &i in ids {
        let v = &mesh.verts[i as usize];
        for k in 0..4 {
            out.color[k] += v.color[k] / n;
        }
        out.mask += v.mask / n;
        out.crease += v.crease / n;
    }
    out.hidden = ids.iter().all(|&i| mesh.verts[i as usize].hidden);
    out
}

fn mean_uv(cs: &[Corner]) -> Vec2 {
    cs.iter().map(|c| c.uv).sum::<Vec2>() / cs.len() as f32
}

// ------------------------------------------------------------------------ subdivide

struct Plan {
    cuts: usize,
    /// novos vértices por aresta cortada, ordenados da menor para a maior ponta
    edge_verts: HashMap<EdgeKey, Vec<u32>>,
}

impl Plan {
    fn is_cut(&self, a: u32, b: u32) -> bool {
        self.edge_verts.contains_key(&edge_key(a, b))
    }

    /// Vértices novos na direção `a → b`.
    fn along(&self, a: u32, b: u32) -> Vec<u32> {
        match self.edge_verts.get(&edge_key(a, b)) {
            Some(v) if a <= b => v.clone(),
            Some(v) => v.iter().rev().copied().collect(),
            None => Vec::new(),
        }
    }
}

struct SubdivOutcome {
    created_verts: Vec<u32>,
    /// Para cada face original: novos índices (na malha resultante) das faces que a substituem.
    pieces: Vec<Vec<u32>>,
    /// Faces que tocavam arestas cortadas (e portanto foram refeitas).
    rebuilt: Vec<bool>,
    sub_edges: Vec<EdgeKey>,
}

#[allow(clippy::too_many_arguments)]
fn build_grid(
    mesh: &mut EditMesh,
    template: &Face,
    bottom: &[u32],
    top: &[u32],
    left: &[u32],
    right: &[u32],
    pc: [&Corner; 4], // p0 (0,0), p1 (a,0), p2 (a,b), p3 (0,b)
    pv: [&EVert; 4],
    faces_out: &mut Vec<Face>,
    created: &mut Vec<u32>,
) {
    let a = bottom.len() - 1;
    let b = left.len() - 1;
    let pos = |ids: &[u32], m: &EditMesh| -> Vec<Vec3> { ids.iter().map(|&v| m.co(v)).collect() };
    let (pb, pt, pl, pr) = (
        pos(bottom, mesh),
        pos(top, mesh),
        pos(left, mesh),
        pos(right, mesh),
    );
    let mut node = vec![vec![0u32; b + 1]; a + 1];
    for i in 0..=a {
        for j in 0..=b {
            node[i][j] = if j == 0 {
                bottom[i]
            } else if j == b {
                top[i]
            } else if i == 0 {
                left[j]
            } else if i == a {
                right[j]
            } else {
                let (u, v) = (i as f32 / a as f32, j as f32 / b as f32);
                let mut vert = bilerp_vert(pv[0], pv[1], pv[3], pv[2], u, v);
                vert.co = coons_point(&pb, &pt, &pl, &pr, i, j);
                let id = mesh.push_vert(vert);
                created.push(id);
                id
            };
        }
    }
    let corner = |i: usize, j: usize, id: u32| -> Corner {
        bilerp_corner(
            pc[0],
            pc[1],
            pc[3],
            pc[2],
            i as f32 / a as f32,
            j as f32 / b as f32,
            id,
        )
    };
    for i in 0..a {
        for j in 0..b {
            let ids = [
                node[i][j],
                node[i + 1][j],
                node[i + 1][j + 1],
                node[i][j + 1],
            ];
            let idx = [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)];
            faces_out.push(Face {
                corners: ids
                    .iter()
                    .zip(idx.iter())
                    .map(|(&id, &(ii, jj))| corner(ii, jj, id))
                    .collect(),
                ..template.clone()
            });
        }
    }
}

fn rebuild_face(
    mesh: &mut EditMesh,
    face: &Face,
    plan: &Plan,
    spoke: bool,
    out: &mut Vec<Face>,
    created: &mut Vec<u32>,
) {
    let n = face.corners.len();
    let c = plan.cuts;
    let vids: Vec<u32> = face.verts().collect();
    let flags: Vec<bool> = (0..n)
        .map(|i| plan.is_cut(vids[i], vids[(i + 1) % n]))
        .collect();
    if !flags.iter().any(|&f| f) {
        out.push(face.clone());
        return;
    }
    // anel: vértices (originais + novos) com seus corners interpolados
    let mut ring: Vec<Corner> = Vec::with_capacity(n * (c + 1));
    for i in 0..n {
        let (ci, cj) = (face.corners[i], face.corners[(i + 1) % n]);
        ring.push(ci);
        if flags[i] {
            for (k, id) in plan
                .along(vids[i], vids[(i + 1) % n])
                .into_iter()
                .enumerate()
            {
                ring.push(lerp_corner(&ci, &cj, (k + 1) as f32 / (c + 1) as f32, id));
            }
        }
    }
    let ring_ids: Vec<u32> = ring.iter().map(|k| k.vert).collect();
    let all_cut = flags.iter().all(|&f| f);
    let stride = c + 1;

    if n == 4 && all_cut {
        let rid = |k: usize| ring_ids[k % ring_ids.len()];
        let bottom: Vec<u32> = (0..=stride).map(&rid).collect();
        let right: Vec<u32> = (stride..=2 * stride).map(&rid).collect();
        let top: Vec<u32> = (2 * stride..=3 * stride).rev().map(&rid).collect();
        let left: Vec<u32> = (3 * stride..=4 * stride).rev().map(&rid).collect();
        let pv: Vec<EVert> = face
            .corners
            .iter()
            .map(|k| mesh.verts[k.vert as usize].clone())
            .collect();
        build_grid(
            mesh,
            face,
            &bottom,
            &top,
            &left,
            &right,
            [
                &face.corners[0],
                &face.corners[1],
                &face.corners[2],
                &face.corners[3],
            ],
            [&pv[0], &pv[1], &pv[2], &pv[3]],
            out,
            created,
        );
        return;
    }
    if n == 4 && flags[0] == flags[2] && flags[1] == flags[3] && flags[0] != flags[1] {
        // escada: o par cortado é (0,2) ou (1,3); gira para que seja (0,2)
        let r = if flags[0] { 0 } else { 1 };
        let q = |i: usize| face.corners[(i + r) % 4];
        let e0: Vec<Corner> = {
            let (ci, cj) = (q(0), q(1));
            let mut v = vec![ci];
            for (k, id) in plan.along(ci.vert, cj.vert).into_iter().enumerate() {
                v.push(lerp_corner(&ci, &cj, (k + 1) as f32 / (c + 1) as f32, id));
            }
            v.push(cj);
            v
        };
        let e2: Vec<Corner> = {
            // q3 → q2 (reverso de q2 → q3)
            let (ci, cj) = (q(3), q(2));
            let mut v = vec![ci];
            for (k, id) in plan.along(ci.vert, cj.vert).into_iter().enumerate() {
                v.push(lerp_corner(&ci, &cj, (k + 1) as f32 / (c + 1) as f32, id));
            }
            v.push(cj);
            v
        };
        for j in 0..=c {
            out.push(Face {
                corners: vec![e0[j], e0[j + 1], e2[j + 1], e2[j]],
                ..face.clone()
            });
        }
        return;
    }
    if spoke && all_cut && c % 2 == 1 {
        // centro + pontos médios: n sub-quads, cada um com grade m × m
        let m = stride / 2;
        let cpos: Vec3 = face.corners.iter().map(|k| mesh.co(k.vert)).sum::<Vec3>() / n as f32;
        let center_vert = average_verts(mesh, &vids, cpos);
        let center_id = mesh.push_vert(center_vert.clone());
        created.push(center_id);
        let center_corner = Corner {
            vert: center_id,
            uv: mean_uv(&face.corners),
            normal: None,
        };
        let mid_idx = |k: usize| k * stride + m;
        // espinhas M_k → C
        let mut spokes: Vec<Vec<u32>> = Vec::with_capacity(n);
        for k in 0..n {
            let mid = ring_ids[mid_idx(k)];
            let mp = mesh.co(mid);
            let mv = mesh.verts[mid as usize].clone();
            let mut path = vec![mid];
            for s in 1..m {
                let t = s as f32 / m as f32;
                let mut v = EVert::lerp(&mv, &center_vert, t);
                v.co = mp.lerp(cpos, t);
                let id = mesh.push_vert(v);
                created.push(id);
                path.push(id);
            }
            path.push(center_id);
            spokes.push(path);
        }
        for k in 0..n {
            let kp = (k + n - 1) % n;
            let rid = |i: usize| ring_ids[i % ring_ids.len()];
            // M_{k-1} -> canto k (o índice dá a volta no anel quando k == 0)
            let start = mid_idx(kp);
            let end = if k == 0 { n * stride } else { k * stride };
            let bottom: Vec<u32> = (start..=end).map(&rid).collect();
            let right: Vec<u32> = (k * stride..=mid_idx(k)).map(&rid).collect();
            let top: Vec<u32> = spokes[k].iter().rev().copied().collect();
            let left: Vec<u32> = spokes[kp].clone();
            let c_prev_mid = ring[mid_idx(kp)];
            let c_mid = ring[mid_idx(k)];
            let p = [c_prev_mid, face.corners[k], c_mid, center_corner];
            let pv = [
                mesh.verts[c_prev_mid.vert as usize].clone(),
                mesh.verts[face.corners[k].vert as usize].clone(),
                mesh.verts[c_mid.vert as usize].clone(),
                center_vert.clone(),
            ];
            build_grid(
                mesh,
                face,
                &bottom,
                &top,
                &left,
                &right,
                [&p[0], &p[1], &p[2], &p[3]],
                [&pv[0], &pv[1], &pv[2], &pv[3]],
                out,
                created,
            );
        }
        return;
    }
    if n == 3 && all_cut {
        let m = c + 1;
        let (ca, cb, cc) = (face.corners[0], face.corners[1], face.corners[2]);
        let ab = plan.along(ca.vert, cb.vert);
        let bc = plan.along(cb.vert, cc.vert);
        let ac = plan.along(ca.vert, cc.vert);
        let (va, vb, vcv) = (
            mesh.verts[ca.vert as usize].clone(),
            mesh.verts[cb.vert as usize].clone(),
            mesh.verts[cc.vert as usize].clone(),
        );
        let bary = |i: usize, j: usize, id: u32| -> Corner {
            let (wb, wc) = (i as f32 / m as f32, j as f32 / m as f32);
            let wa = 1.0 - wb - wc;
            let normal = match (ca.normal, cb.normal, cc.normal) {
                (Some(x), Some(y), Some(z)) => {
                    (x * wa + y * wb + z * wc).try_normalize().or(Some(x))
                }
                _ => None,
            };
            Corner {
                vert: id,
                uv: ca.uv * wa + cb.uv * wb + cc.uv * wc,
                normal,
            }
        };
        let mut node = vec![vec![u32::MAX; m + 1]; m + 1];
        for i in 0..=m {
            for j in 0..=(m - i) {
                node[i][j] = if i == 0 && j == 0 {
                    ca.vert
                } else if i == m && j == 0 {
                    cb.vert
                } else if i == 0 && j == m {
                    cc.vert
                } else if j == 0 {
                    ab[i - 1]
                } else if i == 0 {
                    ac[j - 1]
                } else if i + j == m {
                    bc[j - 1]
                } else {
                    let (wb, wc) = (i as f32 / m as f32, j as f32 / m as f32);
                    let wa = 1.0 - wb - wc;
                    let mut v = EVert::derived(&va, Vec3::ZERO);
                    v.co = va.co * wa + vb.co * wb + vcv.co * wc;
                    for k in 0..4 {
                        v.color[k] = va.color[k] * wa + vb.color[k] * wb + vcv.color[k] * wc;
                    }
                    v.mask = va.mask * wa + vb.mask * wb + vcv.mask * wc;
                    v.crease = va.crease * wa + vb.crease * wb + vcv.crease * wc;
                    let src = if wa >= wb && wa >= wc {
                        &va
                    } else if wb >= wc {
                        &vb
                    } else {
                        &vcv
                    };
                    v.joints = src.joints;
                    v.weights = src.weights;
                    let id = mesh.push_vert(v);
                    created.push(id);
                    id
                };
            }
        }
        for i in 0..m {
            for j in 0..(m - i) {
                let up = [(i, j), (i + 1, j), (i, j + 1)];
                out.push(Face {
                    corners: up.iter().map(|&(a, b)| bary(a, b, node[a][b])).collect(),
                    ..face.clone()
                });
                if i + j + 2 <= m {
                    let down = [(i + 1, j), (i + 1, j + 1), (i, j + 1)];
                    out.push(Face {
                        corners: down.iter().map(|&(a, b)| bary(a, b, node[a][b])).collect(),
                        ..face.clone()
                    });
                }
            }
        }
        return;
    }
    // padrão parcial: um único n-gon com os vértices novos no contorno
    out.push(Face {
        corners: ring,
        ..face.clone()
    });
}

#[allow(clippy::too_many_arguments)]
fn subdivide_core(
    mesh: &mut EditMesh,
    topo: &Topology,
    cut: &[EdgeKey],
    cuts: usize,
    smooth: f32,
    fractal: f32,
    seed: u64,
    spoke_faces: &BTreeSet<u32>,
) -> SubdivOutcome {
    let normals = compute_vertex_normals(mesh);
    let mut plan = Plan {
        cuts,
        edge_verts: HashMap::new(),
    };
    let mut created: Vec<u32> = Vec::new();
    let mut sub_edges: Vec<EdgeKey> = Vec::new();
    for &(a, b) in cut {
        let (va, vb) = (
            mesh.verts[a as usize].clone(),
            mesh.verts[b as usize].clone(),
        );
        let (pa, pb) = (va.co, vb.co);
        let len = (pb - pa).length();
        let has_faces = !topo.edge(a, b).map(|e| e.faces.is_empty()).unwrap_or(true);
        let (na, nb) = (normals[a as usize], normals[b as usize]);
        let theta = na.dot(nb).clamp(-1.0, 1.0).acos();
        let sign = if (nb - na).dot(pb - pa) >= 0.0 {
            1.0
        } else {
            -1.0
        };
        let sag = len * 0.5 * (theta * 0.25).tan();
        let mut ids = Vec::with_capacity(cuts);
        for k in 1..=cuts {
            let t = k as f32 / (cuts + 1) as f32;
            let mut v = EVert::lerp(&va, &vb, t);
            let mut co = pa.lerp(pb, t);
            if has_faces {
                let dir = na.lerp(nb, t).try_normalize().unwrap_or(na);
                if smooth > 0.0 {
                    co += dir * (smooth * 4.0 * sag * t * (1.0 - t) * sign);
                }
                if fractal > 0.0 {
                    // determinístico por (semente, aresta, índice do corte)
                    let key = ((a as u64) << 32 | b as u64)
                        .wrapping_mul(31)
                        .wrapping_add(k as u64);
                    let mut rng = XorShift64::for_index(seed, key);
                    co += dir * (fractal * len * rng.next_signed() * 0.5);
                }
            }
            v.co = co;
            let id = mesh.push_vert(v);
            created.push(id);
            ids.push(id);
        }
        // arestas filhas
        let mut chain = vec![a];
        chain.extend(ids.iter().copied());
        chain.push(b);
        for w in chain.windows(2) {
            sub_edges.push(edge_key(w[0], w[1]));
        }
        plan.edge_verts.insert((a, b), ids);
    }

    // refaz as faces
    let nf = mesh.faces.len();
    let old = std::mem::take(&mut mesh.faces);
    let mut pieces: Vec<Vec<u32>> = vec![Vec::new(); nf];
    let mut rebuilt: Vec<bool> = vec![false; nf];
    let mut new_faces: Vec<Face> = Vec::with_capacity(old.len());
    for (fi, face) in old.iter().enumerate() {
        let before = new_faces.len();
        rebuilt[fi] = face.edges().any(|(a, b)| plan.is_cut(a, b));
        rebuild_face(
            mesh,
            face,
            &plan,
            spoke_faces.contains(&(fi as u32)),
            &mut new_faces,
            &mut created,
        );
        pieces[fi] = (before as u32..new_faces.len() as u32).collect();
    }
    mesh.faces = new_faces;

    // atributos de aresta herdados pelos segmentos
    for &(a, b) in cut {
        let attr = mesh.edge_attrs.remove(&(a, b));
        let ids = &plan.edge_verts[&(a, b)];
        let mut chain = vec![a];
        chain.extend(ids.iter().copied());
        chain.push(b);
        let wire = topo.edge(a, b).map(|e| e.faces.is_empty()).unwrap_or(false);
        for w in chain.windows(2) {
            let k = edge_key(w[0], w[1]);
            match attr {
                Some(at) => {
                    mesh.edge_attrs.insert(k, at);
                }
                None if wire => {
                    mesh.edge_attrs.entry(k).or_default();
                }
                None => {}
            }
        }
    }
    SubdivOutcome {
        created_verts: created,
        pieces,
        rebuilt,
        sub_edges,
    }
}

fn check_subdivide_params(cuts: usize, smooth: f32, fractal: f32) -> ModelResult<()> {
    if cuts == 0 || cuts > MAX_CUTS {
        return Err(ModelError::InvalidParameter(format!(
            "cuts deve estar em 1..={MAX_CUTS}, recebeu {cuts}"
        )));
    }
    if !smooth.is_finite() || !(0.0..=10.0).contains(&smooth) {
        return Err(ModelError::InvalidParameter(format!(
            "smooth deve estar em [0, 10], recebeu {smooth}"
        )));
    }
    if !fractal.is_finite() || !(0.0..=10.0).contains(&fractal) {
        return Err(ModelError::InvalidParameter(format!(
            "fractal deve estar em [0, 10], recebeu {fractal}"
        )));
    }
    Ok(())
}

/// Índices (na malha nova) de todas as faces resultantes de faces refeitas.
fn rebuilt_faces(out: &SubdivOutcome) -> Vec<u32> {
    out.pieces
        .iter()
        .zip(out.rebuilt.iter())
        .filter(|(_, &r)| r)
        .flat_map(|(p, _)| p.iter().copied())
        .collect()
}

fn face_remap(old_verts: usize, pieces: &[Vec<u32>]) -> Remap {
    Remap {
        vert: (0..old_verts as u32).map(Some).collect(),
        face: pieces.iter().map(|p| p.first().copied()).collect(),
    }
}

/// *Subdivide* (`Edge → Subdivide`): corta cada aresta indicada em `cuts + 1`
/// segmentos e re-tessela as faces afetadas (ver tabela de padrões no topo do módulo).
///
/// * `smooth` (`0..=10`): arredonda — empurra os vértices novos ao longo da normal
///   interpolada para que a aresta siga o arco de círculo definido pelas normais das
///   pontas (1 = esfera exata para superfícies esféricas; côncavo afunda);
/// * `fractal` (`0..=10`) e `seed`: ruído determinístico ao longo da normal, proporcional ao
///   comprimento da aresta;
/// * arestas wire (sem face) também podem ser cortadas.
///
/// Seleção devolvida: arestas filhas (e seus vértices); `created_verts`: todos os
/// vértices novos; `created_faces`: faces resultantes de faces re-tesseladas;
/// `remap.face` leva cada face antiga ao primeiro pedaço. Erros (malha intacta):
/// seleção vazia, aresta inexistente, `cuts` fora de `1..=64`, `smooth`/`fractal` fora de faixa.
pub fn subdivide_edges(
    mesh: &mut EditMesh,
    edges: &[EdgeKey],
    cuts: usize,
    smooth: f32,
    fractal: f32,
    seed: u64,
) -> ModelResult<OpOutput> {
    let topo = Topology::build(mesh);
    let edges = unique_edges(&topo, edges, "arestas")?;
    check_subdivide_params(cuts, smooth, fractal)?;
    let old_verts = mesh.verts.len();
    let out = subdivide_core(
        mesh,
        &topo,
        &edges,
        cuts,
        smooth,
        fractal,
        seed,
        &BTreeSet::new(),
    );
    let produced: Vec<u32> = rebuilt_faces(&out);
    let remap = face_remap(old_verts, &out.pieces);
    let mut selection = Selection::from_edges(out.sub_edges.iter().copied());
    selection.verts.extend(out.created_verts.iter().copied());
    Ok(OpOutput {
        selection,
        created_verts: out.created_verts,
        created_faces: produced,
        remap: Some(remap),
    })
}

/// *Subdivide* de faces: corta todas as arestas das faces selecionadas `cuts` vezes.
///
/// * quads viram grades `(cuts+1)²`;
/// * triângulos e n-gons viram quads ao redor de um vértice central (estilo
///   Catmull-Clark: pontos médios das arestas + centro, cada quarto com grade
///   `((cuts+1)/2)²`) — por isso exigem `cuts` **ímpar**;
/// * faces vizinhas não selecionadas seguem a tabela do módulo (escadas, n-gons).
///
/// `smooth` arredonda as arestas cortadas como em [`subdivide_edges`]. Seleção
/// devolvida: as faces resultantes das faces selecionadas. Erros: seleção vazia/inválida,
/// parâmetros fora de faixa, `cuts` par com triângulo/n-gon selecionado.
pub fn subdivide_faces(
    mesh: &mut EditMesh,
    faces: &[u32],
    cuts: usize,
    smooth: f32,
) -> ModelResult<OpOutput> {
    let sel = unique_faces(mesh, faces, "faces")?;
    check_subdivide_params(cuts, smooth, 0.0)?;
    if cuts & 1 == 0 {
        if let Some(&f) = sel.iter().find(|&&f| mesh.faces[f as usize].len() != 4) {
            return Err(ModelError::InvalidParameter(format!(
                "a face {f} tem {} lados: triângulos e n-gons exigem cuts ímpar (recebeu {cuts})",
                mesh.faces[f as usize].len()
            )));
        }
    }
    let topo = Topology::build(mesh);
    let mut cut: BTreeSet<EdgeKey> = BTreeSet::new();
    for &f in &sel {
        for (a, b) in mesh.faces[f as usize].edges() {
            cut.insert(edge_key(a, b));
        }
    }
    let cut: Vec<EdgeKey> = cut.into_iter().collect();
    let spoke: BTreeSet<u32> = sel
        .iter()
        .copied()
        .filter(|&f| mesh.faces[f as usize].len() != 4)
        .collect();
    let old_verts = mesh.verts.len();
    let out = subdivide_core(mesh, &topo, &cut, cuts, smooth, 0.0, 0, &spoke);
    let remap = face_remap(old_verts, &out.pieces);
    let mut new_sel: Vec<u32> = Vec::new();
    for &f in &sel {
        new_sel.extend(out.pieces[f as usize].iter().copied());
    }
    let produced: Vec<u32> = rebuilt_faces(&out);
    let mut selection = Selection::from_faces(mesh, new_sel);
    selection.verts.extend(out.created_verts.iter().copied());
    Ok(OpOutput {
        selection,
        created_verts: out.created_verts,
        created_faces: produced,
        remap: Some(remap),
    })
}

// ---------------------------------------------------------------------------- poke

/// *Poke Faces*: cada face selecionada vira um leque de triângulos ao redor de um
/// vértice novo no centro (média dos vértices), deslocado ao longo da normal da face.
///
/// * `offset`: deslocamento; com `use_relative_offset`, multiplica o raio médio da
///   face (distância média do centro aos vértices), senão é absoluto;
/// * o vértice central herda cor/máscara/crease médios; o corner central recebe a UV
///   média; os demais corners mantêm UV e normais; atributos de face e de aresta
///   permanecem.
///
/// Seleção: os triângulos criados; `created_verts`: os centros. Erros: seleção
/// vazia/inválida, `offset` não finito.
pub fn poke_faces(
    mesh: &mut EditMesh,
    faces: &[u32],
    offset: f32,
    use_relative_offset: bool,
) -> ModelResult<OpOutput> {
    let sel = unique_faces(mesh, faces, "faces")?;
    if !offset.is_finite() {
        return Err(ModelError::InvalidParameter(format!(
            "offset não finito: {offset}"
        )));
    }
    let selected: BTreeSet<u32> = sel.iter().copied().collect();
    let nf = mesh.faces.len();
    let old = std::mem::take(&mut mesh.faces);
    let mut map: Vec<Option<u32>> = vec![None; nf];
    let mut produced = Vec::new();
    let mut centers = Vec::new();
    let mut new_faces: Vec<Face> = Vec::with_capacity(old.len() + sel.len() * 3);
    for (fi, face) in old.iter().enumerate() {
        map[fi] = Some(new_faces.len() as u32);
        if !selected.contains(&(fi as u32)) {
            new_faces.push(face.clone());
            continue;
        }
        let pts: Vec<Vec3> = face.corners.iter().map(|c| mesh.co(c.vert)).collect();
        let center = pts.iter().copied().sum::<Vec3>() / pts.len() as f32;
        let normal = crate::mesh::newell_normal(&pts);
        let radius = pts.iter().map(|p| p.distance(center)).sum::<f32>() / pts.len() as f32;
        let height = if use_relative_offset {
            offset * radius
        } else {
            offset
        };
        let ids: Vec<u32> = face.verts().collect();
        let cv = average_verts(mesh, &ids, center + normal * height);
        let cid = mesh.push_vert(cv);
        centers.push(cid);
        let cc = Corner {
            vert: cid,
            uv: mean_uv(&face.corners),
            normal: None,
        };
        let n = face.corners.len();
        for i in 0..n {
            produced.push(new_faces.len() as u32);
            new_faces.push(Face {
                corners: vec![cc, face.corners[i], face.corners[(i + 1) % n]],
                ..face.clone()
            });
        }
    }
    mesh.faces = new_faces;
    let remap = Remap {
        vert: (0..mesh.verts.len() as u32 - centers.len() as u32)
            .map(Some)
            .collect(),
        face: map,
    };
    let mut selection = Selection::from_faces(mesh, produced.iter().copied());
    selection.verts.extend(centers.iter().copied());
    Ok(OpOutput {
        selection,
        created_verts: centers,
        created_faces: produced,
        remap: Some(remap),
    })
}

// ---------------------------------------------------------------------------- rip

/// Alvo do [`rip_vertex`].
#[derive(Debug, Clone, PartialEq)]
pub enum RipTarget {
    /// Destaca estas faces (as que tocam o vértice) do restante: elas passam a usar um
    /// vértice duplicado.
    Faces(Vec<u32>),
    /// Corta ao longo destas arestas (incidentes ao vértice): as faces ao redor são
    /// separadas em grupos conexos sem cruzar as arestas e cada grupo, exceto o da menor
    /// face, ganha um vértice duplicado. Em vértice interior exige ≥ 2 arestas; em
    /// vértice de fronteira basta 1.
    Edges(Vec<EdgeKey>),
}

/// *Rip* (`V`): duplica o vértice e distribui as faces ao redor entre as cópias,
/// abrindo uma fenda na malha. Atributos de aresta incidentes são copiados para as
/// novas arestas (e movidos quando todas as faces da aresta migram).
///
/// Seleção devolvida: o vértice original e as cópias; `created_verts`: as cópias.
/// Erros (malha intacta): vértice inválido ou sem faces, alvo que não separa nada (todas
/// ou nenhuma face do vértice; arestas que não dividem o leque), aresta que não é
/// incidente ao vértice.
pub fn rip_vertex(mesh: &mut EditMesh, vert: u32, target: &RipTarget) -> ModelResult<OpOutput> {
    if vert as usize >= mesh.verts.len() {
        return Err(ModelError::VertexOutOfRange(vert, mesh.verts.len()));
    }
    let topo = Topology::build(mesh);
    let faces_v: Vec<u32> = topo.vert_faces[vert as usize].clone();
    if faces_v.is_empty() {
        return Err(ModelError::InvalidParameter(format!(
            "o vértice {vert} não pertence a nenhuma face"
        )));
    }
    // grupos de faces: groups[0] mantém o vértice; os demais recebem cópias
    let mut groups: Vec<Vec<u32>> = Vec::new();
    match target {
        RipTarget::Faces(sel) => {
            let sel = unique_faces(mesh, sel, "faces")?;
            let inside: Vec<u32> = faces_v
                .iter()
                .copied()
                .filter(|f| sel.contains(f))
                .collect();
            let outside: Vec<u32> = faces_v
                .iter()
                .copied()
                .filter(|f| !sel.contains(f))
                .collect();
            if inside.is_empty() || outside.is_empty() {
                return Err(ModelError::InvalidParameter(
                    "as faces escolhidas precisam dividir as faces do vértice em dois grupos não vazios".into(),
                ));
            }
            groups.push(outside);
            groups.push(inside);
        }
        RipTarget::Edges(es) => {
            let es = unique_edges(&topo, es, "arestas")?;
            for &(a, b) in &es {
                if a != vert && b != vert {
                    return Err(ModelError::InvalidParameter(format!(
                        "a aresta ({a}, {b}) não é incidente ao vértice {vert}"
                    )));
                }
            }
            let cut: BTreeSet<EdgeKey> = es.into_iter().collect();
            // componentes das faces do vértice sem atravessar arestas cortadas
            let mut comp: BTreeMap<u32, u32> = BTreeMap::new();
            let mut next = 0u32;
            for &start in &faces_v {
                if comp.contains_key(&start) {
                    continue;
                }
                comp.insert(start, next);
                let mut stack = vec![start];
                while let Some(f) = stack.pop() {
                    for &e in &topo.vert_edges[vert as usize] {
                        let info = &topo.edges[e as usize];
                        if cut.contains(&info.verts) || !info.faces.contains(&f) {
                            continue;
                        }
                        for &g in &info.faces {
                            if faces_v.contains(&g) && !comp.contains_key(&g) {
                                comp.insert(g, next);
                                stack.push(g);
                            }
                        }
                    }
                }
                next += 1;
            }
            if next < 2 {
                return Err(ModelError::InvalidParameter(
                    "as arestas escolhidas não separam as faces ao redor do vértice (vértice interior exige 2 ou mais arestas)".into(),
                ));
            }
            for c in 0..next {
                groups.push(
                    comp.iter()
                        .filter(|(_, &g)| g == c)
                        .map(|(&f, _)| f)
                        .collect(),
                );
            }
        }
    }
    // aplica
    let mut created = Vec::new();
    let mut group_of_face: HashMap<u32, usize> = HashMap::new();
    let mut group_vert: Vec<u32> = vec![vert];
    for (gi, g) in groups.iter().enumerate() {
        for &f in g {
            group_of_face.insert(f, gi);
        }
        if gi > 0 {
            let nv = mesh.push_vert(mesh.verts[vert as usize].clone());
            created.push(nv);
            group_vert.push(nv);
        }
    }
    for (&f, &gi) in &group_of_face {
        if gi == 0 {
            continue;
        }
        for c in &mut mesh.faces[f as usize].corners {
            if c.vert == vert {
                c.vert = group_vert[gi];
            }
        }
    }
    // atributos de aresta incidentes
    let incident: Vec<(EdgeKey, EdgeAttr)> = mesh
        .edge_attrs
        .iter()
        .filter(|((a, b), _)| *a == vert || *b == vert)
        .map(|(k, v)| (*k, *v))
        .collect();
    for ((a, b), attr) in incident {
        let other = if a == vert { b } else { a };
        let efaces = topo.edge_faces(a, b);
        if efaces.is_empty() {
            continue;
        }
        let mut used_groups: BTreeSet<usize> = BTreeSet::new();
        for f in efaces {
            if let Some(&gi) = group_of_face.get(f) {
                used_groups.insert(gi);
            }
        }
        for &gi in &used_groups {
            if gi > 0 {
                mesh.edge_attrs
                    .insert(edge_key(group_vert[gi], other), attr);
            }
        }
        if !used_groups.contains(&0) {
            mesh.edge_attrs.remove(&(a, b));
        }
    }
    Ok(OpOutput {
        selection: Selection::from_verts(std::iter::once(vert).chain(created.iter().copied())),
        created_verts: created,
        created_faces: Vec::new(),
        remap: None,
    })
}

// ------------------------------------------------------------- separate / split

/// *Separate → Selection*: extrai as faces dadas para uma **nova malha** e as remove
/// da original. A nova malha leva só os vértices usados (renumerados em ordem
/// crescente do índice original), com atributos de vértice, de face, de corner e das
/// arestas pertencentes às faces. Na original, somem as faces e os vértices/arestas
/// que só elas usavam (vértices compartilhados com faces restantes ficam nas duas).
///
/// Devolve `(nova_malha, remap_da_original)`. A nova malha se chama `"<nome>.separate"`.
/// Erros (original intacta): seleção vazia ou índice inválido.
pub fn separate_selected(mesh: &mut EditMesh, faces: &[u32]) -> ModelResult<(EditMesh, Remap)> {
    let sel = unique_faces(mesh, faces, "faces")?;
    let mut used = vec![false; mesh.verts.len()];
    let mut face_edges: BTreeSet<EdgeKey> = BTreeSet::new();
    for &f in &sel {
        for (a, b) in mesh.faces[f as usize].edges() {
            used[a as usize] = true;
            used[b as usize] = true;
            face_edges.insert(edge_key(a, b));
        }
    }
    let mut vmap: Vec<Option<u32>> = vec![None; mesh.verts.len()];
    let mut out = EditMesh::new(format!("{}.separate", mesh.name));
    for (i, &u) in used.iter().enumerate() {
        if u {
            vmap[i] = Some(out.push_vert(mesh.verts[i].clone()));
        }
    }
    for &f in &sel {
        let mut face = mesh.faces[f as usize].clone();
        for c in &mut face.corners {
            c.vert = vmap[c.vert as usize].unwrap_or(c.vert);
        }
        out.faces.push(face);
    }
    for (&(a, b), &attr) in &mesh.edge_attrs {
        if face_edges.contains(&(a, b)) {
            if let (Some(na), Some(nb)) = (vmap[a as usize], vmap[b as usize]) {
                out.edge_attrs.insert(edge_key(na, nb), attr);
            }
        }
    }
    let topo = Topology::build(mesh);
    let cleanup: Vec<u32> = (0..used.len() as u32)
        .filter(|&v| used[v as usize])
        .collect();
    let remap = apply_delete(
        mesh,
        &topo,
        &DeletePlan {
            kill_faces: &sel,
            kill_verts: &[],
            drop_edges: &[],
            promote_wire: false,
            cleanup_verts: &cleanup,
        },
    );
    Ok((out, remap))
}

/// *Split* (`Y`): destaca as faces dadas do resto **dentro da mesma malha**,
/// duplicando os vértices que elas compartilham com faces não selecionadas (as
/// faces selecionadas passam a usar as cópias). Atributos de aresta nas fronteiras
/// são copiados para os dois lados. Sem vértices compartilhados é um no-op.
///
/// Seleção devolvida: as faces destacadas; `created_verts`: as cópias. Erros:
/// seleção vazia/inválida.
pub fn split_selected(mesh: &mut EditMesh, faces: &[u32]) -> ModelResult<OpOutput> {
    let sel = unique_faces(mesh, faces, "faces")?;
    let in_sel: BTreeSet<u32> = sel.iter().copied().collect();
    let topo = Topology::build(mesh);
    // vértices compartilhados entre selecionadas e não selecionadas
    let mut dup: BTreeMap<u32, u32> = BTreeMap::new();
    let mut shared: BTreeSet<u32> = BTreeSet::new();
    for &f in &sel {
        for v in mesh.faces[f as usize].verts() {
            if topo.vert_faces[v as usize]
                .iter()
                .any(|g| !in_sel.contains(g))
            {
                shared.insert(v);
            }
        }
    }
    for &v in &shared {
        let nv = mesh.push_vert(mesh.verts[v as usize].clone());
        dup.insert(v, nv);
    }
    let map = |v: u32| dup.get(&v).copied().unwrap_or(v);
    // atributos de aresta
    let mut add: Vec<(EdgeKey, EdgeAttr)> = Vec::new();
    let mut remove: Vec<EdgeKey> = Vec::new();
    for (&(a, b), &attr) in &mesh.edge_attrs {
        let Some(info) = topo.edge(a, b) else {
            continue;
        };
        if !info.faces.iter().any(|f| in_sel.contains(f)) {
            continue;
        }
        let k = edge_key(map(a), map(b));
        if k != (a, b) {
            add.push((k, attr));
            if info.faces.iter().all(|f| in_sel.contains(f)) {
                remove.push((a, b));
            }
        }
    }
    for k in remove {
        mesh.edge_attrs.remove(&k);
    }
    for (k, v) in add {
        mesh.edge_attrs.insert(k, v);
    }
    for &f in &sel {
        for c in &mut mesh.faces[f as usize].corners {
            c.vert = map(c.vert);
        }
    }
    let created: Vec<u32> = dup.values().copied().collect();
    Ok(OpOutput {
        selection: Selection::from_faces(mesh, sel.iter().copied()),
        created_verts: created,
        created_faces: Vec::new(),
        remap: None,
    })
}

// ------------------------------------------------------------- marcação em lote

fn edit_edges(
    mesh: &mut EditMesh,
    edges: &[EdgeKey],
    f: impl Fn(&mut EdgeAttr),
) -> ModelResult<OpOutput> {
    let topo = Topology::build(mesh);
    let edges = unique_edges(&topo, edges, "arestas")?;
    for &(a, b) in &edges {
        let wire = topo.is_wire_edge(a, b);
        mesh.edit_edge_attr(a, b, &f);
        if wire {
            // uma aresta wire só existe enquanto tiver entrada no mapa
            mesh.edge_attrs.entry((a, b)).or_default();
        }
    }
    Ok(OpOutput {
        selection: Selection::from_edges(edges),
        ..Default::default()
    })
}

/// Marca/desmarca arestas como *sharp* (Blender: *Mark Sharp / Clear Sharp*).
/// Arestas wire continuam existindo mesmo que o atributo volte ao padrão. Seleção
/// devolvida: as arestas. Erros: lista vazia ou aresta inexistente (nada é alterado).
pub fn mark_sharp(mesh: &mut EditMesh, edges: &[EdgeKey], sharp: bool) -> ModelResult<OpOutput> {
    edit_edges(mesh, edges, |e| e.sharp = sharp)
}

/// Marca/desmarca costuras de UV (*Mark Seam / Clear Seam*). Mesmas regras de [`mark_sharp`].
pub fn mark_seam(mesh: &mut EditMesh, edges: &[EdgeKey], seam: bool) -> ModelResult<OpOutput> {
    edit_edges(mesh, edges, |e| e.seam = seam)
}

fn check_01(name: &str, v: f32) -> ModelResult<()> {
    if !v.is_finite() || !(0.0..=1.0).contains(&v) {
        return Err(ModelError::InvalidParameter(format!(
            "{name} deve estar em [0, 1], recebeu {v}"
        )));
    }
    Ok(())
}

/// Define o vinco de subdivisão (`0..=1`) das arestas (*Edge Crease*). Erros: valor
/// fora de `[0, 1]` ou não finito, lista vazia, aresta inexistente.
pub fn set_crease(mesh: &mut EditMesh, edges: &[EdgeKey], crease: f32) -> ModelResult<OpOutput> {
    check_01("crease", crease)?;
    edit_edges(mesh, edges, |e| e.crease = crease)
}

/// Define o *bevel weight* (`0..=1`) das arestas. Mesmas regras de [`set_crease`].
pub fn set_bevel_weight(
    mesh: &mut EditMesh,
    edges: &[EdgeKey],
    weight: f32,
) -> ModelResult<OpOutput> {
    check_01("weight", weight)?;
    edit_edges(mesh, edges, |e| e.bevel_weight = weight)
}

/// *Shade Smooth / Shade Flat* nas faces dadas. Seleção devolvida: as faces. Erros:
/// seleção vazia/inválida.
pub fn set_faces_smooth(mesh: &mut EditMesh, faces: &[u32], smooth: bool) -> ModelResult<OpOutput> {
    let sel = unique_faces(mesh, faces, "faces")?;
    for &f in &sel {
        mesh.faces[f as usize].smooth = smooth;
    }
    Ok(OpOutput {
        selection: Selection::from_faces(mesh, sel),
        ..Default::default()
    })
}

// -------------------------------------------------------------------- hide / reveal

fn refresh_vert_visibility(mesh: &mut EditMesh) {
    let topo = Topology::build(mesh);
    for (v, faces) in topo.vert_faces.iter().enumerate() {
        if !faces.is_empty() && faces.iter().all(|&f| mesh.faces[f as usize].hidden) {
            mesh.verts[v].hidden = true;
        }
    }
}

/// *Hide* (`H`) para faces: marca as faces como ocultas e oculta os vértices cujas
/// faces estão todas ocultas (vértices compartilhados com faces visíveis continuam
/// visíveis). Devolve seleção vazia (o que some não fica selecionado). Erros:
/// seleção vazia/inválida.
pub fn hide_faces(mesh: &mut EditMesh, faces: &[u32]) -> ModelResult<OpOutput> {
    let sel = unique_faces(mesh, faces, "faces")?;
    for &f in &sel {
        mesh.faces[f as usize].hidden = true;
    }
    refresh_vert_visibility(mesh);
    Ok(OpOutput::default())
}

/// *Hide* para vértices: oculta os vértices e todas as faces que os usam.
pub fn hide_verts(mesh: &mut EditMesh, verts: &[u32]) -> ModelResult<OpOutput> {
    let sel = unique_verts(mesh, verts, "vértices")?;
    let set: BTreeSet<u32> = sel.iter().copied().collect();
    for &v in &sel {
        mesh.verts[v as usize].hidden = true;
    }
    for f in &mut mesh.faces {
        if f.verts().any(|v| set.contains(&v)) {
            f.hidden = true;
        }
    }
    refresh_vert_visibility(mesh);
    Ok(OpOutput::default())
}

/// *Reveal* (`Alt+H`): mostra tudo que estava oculto e devolve como seleção os
/// elementos revelados (comportamento do Blender). Nada oculto: seleção vazia.
pub fn reveal_all(mesh: &mut EditMesh) -> OpOutput {
    let mut faces = Vec::new();
    for (i, f) in mesh.faces.iter_mut().enumerate() {
        if f.hidden {
            f.hidden = false;
            faces.push(i as u32);
        }
    }
    let mut verts = Vec::new();
    for (i, v) in mesh.verts.iter_mut().enumerate() {
        if v.hidden {
            v.hidden = false;
            verts.push(i as u32);
        }
    }
    let mut selection = Selection::from_faces(mesh, faces);
    selection.verts.extend(verts);
    OpOutput {
        selection,
        ..Default::default()
    }
}

// -------------------------------------------------------------------- seleção por traço

/// *Select Sharp Edges*: arestas manifold cujas duas faces formam ângulo diedral
/// (entre normais) **maior** que `angle` rad (`0..=π`). Arestas de fronteira/wire/não-manifold
/// nunca entram. Erro: ângulo inválido.
pub fn select_sharp_edges(mesh: &EditMesh, angle: f32) -> ModelResult<Selection> {
    if !angle.is_finite() || !(0.0..=std::f32::consts::PI + 1e-4).contains(&angle) {
        return Err(ModelError::InvalidParameter(format!(
            "angle deve estar em [0, π], recebeu {angle}"
        )));
    }
    let topo = Topology::build(mesh);
    let cos_limit = angle.cos();
    let edges = topo.edges.iter().filter(|e| {
        e.faces.len() == 2
            && e.faces[0] != e.faces[1]
            && mesh
                .face_normal(e.faces[0])
                .dot(mesh.face_normal(e.faces[1]))
                < cos_limit - 1e-6
    });
    Ok(Selection::from_edges(edges.map(|e| e.verts)))
}

/// *Select Loose Geometry*: vértices sem aresta (`use_verts`), arestas sem face
/// (`use_edges`) e faces cujas arestas pertencem só a elas (`use_faces`).
pub fn select_loose(
    mesh: &EditMesh,
    use_verts: bool,
    use_edges: bool,
    use_faces: bool,
) -> Selection {
    let topo = Topology::build(mesh);
    let mut s = Selection::default();
    if use_verts {
        for v in 0..mesh.verts.len() {
            if topo.vert_edges[v].is_empty() {
                s.verts.insert(v as u32);
            }
        }
    }
    if use_edges {
        for e in topo.edges.iter().filter(|e| e.faces.is_empty()) {
            s.edges.insert(e.verts);
            s.verts.insert(e.verts.0);
            s.verts.insert(e.verts.1);
        }
    }
    if use_faces {
        for (fi, es) in topo.face_edges.iter().enumerate() {
            if es.iter().all(|&e| topo.edges[e as usize].faces.len() == 1) {
                let f = Selection::from_faces(mesh, [fi as u32]);
                s.faces.extend(f.faces);
                s.edges.extend(f.edges);
                s.verts.extend(f.verts);
            }
        }
    }
    s
}

/// Quais tipos de não-manifold selecionar em [`select_non_manifold`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NonManifoldOptions {
    /// Arestas sem face.
    pub wire: bool,
    /// Arestas com uma só face.
    pub boundary: bool,
    /// Arestas com 3+ faces.
    pub multi_face: bool,
    /// Arestas de 2 faces com enrolamentos inconsistentes (normais opostas).
    pub non_contiguous: bool,
    /// Vértices onde as faces formam mais de um "leque" separado (pinçamento).
    pub verts: bool,
}

impl Default for NonManifoldOptions {
    fn default() -> Self {
        Self {
            wire: true,
            boundary: true,
            multi_face: true,
            non_contiguous: true,
            verts: true,
        }
    }
}

/// *Select Non-Manifold*: arestas e vértices conforme `opts`. Seleciona também os
/// extremos das arestas escolhidas.
pub fn select_non_manifold(mesh: &EditMesh, opts: NonManifoldOptions) -> Selection {
    let topo = Topology::build(mesh);
    let mut s = Selection::default();
    for (ei, e) in topo.edges.iter().enumerate() {
        let n = e.faces.len();
        let hit = match n {
            0 => opts.wire,
            1 => opts.boundary,
            2 => {
                opts.non_contiguous && {
                    let dir = |f: u32| -> Option<bool> {
                        let pos = topo.face_edges[f as usize]
                            .iter()
                            .position(|&x| x as usize == ei)?;
                        let face = &mesh.faces[f as usize];
                        let m = face.corners.len();
                        Some(face.corners[pos].vert < face.corners[(pos + 1) % m].vert)
                    };
                    e.faces[0] != e.faces[1] && dir(e.faces[0]) == dir(e.faces[1])
                }
            }
            _ => opts.multi_face,
        };
        if hit {
            s.edges.insert(e.verts);
            s.verts.insert(e.verts.0);
            s.verts.insert(e.verts.1);
        }
    }
    if opts.verts {
        for v in 0..mesh.verts.len() as u32 {
            let faces = &topo.vert_faces[v as usize];
            if faces.len() < 2 {
                continue;
            }
            let mut parent: BTreeMap<u32, u32> = faces.iter().map(|&f| (f, f)).collect();
            fn find(p: &mut BTreeMap<u32, u32>, x: u32) -> u32 {
                let mut r = x;
                while p[&r] != r {
                    r = p[&r];
                }
                r
            }
            for &e in &topo.vert_edges[v as usize] {
                let info = &topo.edges[e as usize];
                let fs: Vec<u32> = info
                    .faces
                    .iter()
                    .copied()
                    .filter(|f| parent.contains_key(f))
                    .collect();
                for w in fs.windows(2) {
                    let (ra, rb) = (find(&mut parent, w[0]), find(&mut parent, w[1]));
                    if ra != rb {
                        parent.insert(ra.max(rb), ra.min(rb));
                    }
                }
            }
            let roots: BTreeSet<u32> = faces.iter().map(|&f| find(&mut parent, f)).collect();
            if roots.len() > 1 {
                s.verts.insert(v);
            }
        }
    }
    s
}

/// *Select Interior Faces*: faces cujas arestas têm todas mais de 2 faces (faces
/// internas de volumes soldados, típicas de booleanas mal resolvidas).
pub fn select_interior_faces(mesh: &EditMesh) -> Selection {
    let topo = Topology::build(mesh);
    let faces = topo
        .face_edges
        .iter()
        .enumerate()
        .filter(|(_, es)| es.iter().all(|&e| topo.edges[e as usize].faces.len() > 2))
        .map(|(i, _)| i as u32);
    Selection::from_faces(mesh, faces)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;
    use crate::primitives::assert_outward;

    fn area(m: &EditMesh) -> f32 {
        (0..m.face_count() as u32).map(|f| m.face_area(f)).sum()
    }

    fn all_edges(m: &EditMesh) -> Vec<EdgeKey> {
        Topology::build(m).edges.iter().map(|e| e.verts).collect()
    }

    fn assert_closed(m: &EditMesh, euler: i64) {
        m.validate().unwrap();
        let t = Topology::build(m);
        assert!(
            t.is_closed_manifold(),
            "malha deveria ser fechada e 2-manifold"
        );
        assert_eq!(t.euler_characteristic(m), euler);
    }

    fn triangle() -> EditMesh {
        let mut m = EditMesh::new("tri");
        for p in [Vec3::ZERO, Vec3::X, Vec3::Z * -1.0] {
            m.add_vert(p);
        }
        m.add_face(&[0, 1, 2]).unwrap();
        m
    }

    // ------------------------------------------------------------- subdivide_edges

    #[test]
    fn subdivide_all_cube_edges_gives_quad_grids() {
        for (cuts, faces, verts) in [(1usize, 24usize, 26usize), (2, 54, 56), (3, 96, 98)] {
            let mut m = primitives::cube(2.0);
            let a0 = area(&m);
            let edges = all_edges(&m);
            let out = subdivide_edges(&mut m, &edges, cuts, 0.0, 0.0, 0).unwrap();
            assert_eq!(m.face_count(), faces, "cuts {cuts}");
            assert_eq!(m.vert_count(), verts);
            assert!(m.faces.iter().all(|f| f.len() == 4));
            assert_closed(&m, 2);
            assert_outward(&m);
            assert!((area(&m) - a0).abs() < 1e-3);
            assert_eq!(out.created_verts.len(), verts - 8);
            assert_eq!(out.created_faces.len(), faces);
            assert_eq!(out.selection.edges.len(), 12 * (cuts + 1));
            let r = out.remap.unwrap();
            assert_eq!(r.face.len(), 6);
            assert_eq!(r.vert.len(), 8);
        }
    }

    #[test]
    fn subdivide_single_cube_edge_makes_two_pentagons() {
        let mut m = primitives::cube(1.0);
        let e = Topology::build(&m).edges[0].verts;
        let out = subdivide_edges(&mut m, &[e], 1, 0.0, 0.0, 0).unwrap();
        assert_eq!(m.vert_count(), 9);
        assert_eq!(m.face_count(), 6);
        assert_eq!(m.faces.iter().filter(|f| f.len() == 5).count(), 2);
        assert_closed(&m, 2);
        assert_eq!(out.created_faces.len(), 2);
        let nv = out.created_verts[0];
        let expected = (primitives::cube(1.0).co(e.0) + primitives::cube(1.0).co(e.1)) * 0.5;
        assert!((m.co(nv) - expected).length() < 1e-6);
    }

    #[test]
    fn subdivide_opposite_edges_of_a_quad_makes_ladder() {
        let mut m = primitives::plane(2.0);
        let a0 = area(&m);
        // face [0,2,3,1]: arestas opostas (0,2) e (1,3)
        let out = subdivide_edges(&mut m, &[(0, 2), (1, 3)], 2, 0.0, 0.0, 0).unwrap();
        assert_eq!(m.face_count(), 3);
        assert!(m.faces.iter().all(|f| f.len() == 4));
        assert_eq!(m.vert_count(), 8);
        assert!((area(&m) - a0).abs() < 1e-5);
        for f in 0..3 {
            assert!(m.face_normal(f).y > 0.99);
        }
        assert_eq!(out.created_faces.len(), 3);
        m.validate().unwrap();
        // o outro par também
        let mut n = primitives::plane(2.0);
        subdivide_edges(&mut n, &[(0, 1), (2, 3)], 1, 0.0, 0.0, 0).unwrap();
        assert_eq!(n.face_count(), 2);
        assert!(n.faces.iter().all(|f| f.len() == 4));
        for f in 0..2 {
            assert!(n.face_normal(f).y > 0.99);
        }
    }

    #[test]
    fn subdivide_triangle_lattice() {
        for (cuts, tris, verts) in [(1usize, 4usize, 6usize), (2, 9, 10), (3, 16, 15)] {
            let mut m = triangle();
            let a0 = area(&m);
            let edges = all_edges(&m);
            subdivide_edges(&mut m, &edges, cuts, 0.0, 0.0, 0).unwrap();
            assert_eq!(m.face_count(), tris);
            assert_eq!(m.vert_count(), verts);
            assert!(m.faces.iter().all(|f| f.len() == 3));
            assert!((area(&m) - a0).abs() < 1e-5);
            let n0 = triangle().face_normal(0);
            for f in 0..tris as u32 {
                assert!(
                    (m.face_normal(f) - n0).length() < 1e-4,
                    "orientação do triângulo {f}"
                );
            }
            m.validate().unwrap();
            let t = Topology::build(&m);
            assert_eq!(t.euler_characteristic(&m), 1);
        }
    }

    #[test]
    fn subdivide_triangulated_cube_keeps_closed() {
        let mut m = primitives::cube(1.0);
        let all: Vec<u32> = (0..6).collect();
        crate::ops::triangulate::triangulate_faces(
            &mut m,
            &all,
            crate::ops::triangulate::QuadMethod::Beauty,
            crate::ops::triangulate::NgonMethod::Beauty,
        )
        .unwrap();
        let edges = all_edges(&m);
        subdivide_edges(&mut m, &edges, 1, 0.0, 0.0, 0).unwrap();
        assert_eq!(m.face_count(), 48);
        assert_closed(&m, 2);
        assert_outward(&m);
    }

    #[test]
    fn subdivide_smooth_follows_the_sphere() {
        let base = primitives::quad_sphere(1.0, 3);
        let edges = all_edges(&base);
        let err = |smooth: f32| {
            let mut m = base.clone();
            let out = subdivide_edges(&mut m, &edges, 1, smooth, 0.0, 0).unwrap();
            // só os vértices de aresta (não os interiores das grades)
            out.created_verts
                .iter()
                .take(edges.len())
                .map(|&v| (m.co(v).length() - 1.0).abs())
                .fold(0.0f32, f32::max)
        };
        let flat = err(0.0);
        let round = err(1.0);
        assert!(round < flat * 0.4, "flat {flat} round {round}");
        assert!(round < 0.01, "{round}");
        // cubo: vértices de aresta sobem
        let mut c = primitives::cube(1.0);
        let es = all_edges(&c);
        let out = subdivide_edges(&mut c, &es, 1, 1.0, 0.0, 0).unwrap();
        for &v in out.created_verts.iter().take(12) {
            assert!(c.co(v).length() > std::f32::consts::FRAC_1_SQRT_2 + 0.05);
        }
        c.validate().unwrap();
    }

    #[test]
    fn subdivide_fractal_is_deterministic_and_seed_dependent() {
        let base = primitives::cube(2.0);
        let edges = all_edges(&base);
        let run = |fractal, seed| {
            let mut m = base.clone();
            subdivide_edges(&mut m, &edges, 2, 0.0, fractal, seed).unwrap();
            m
        };
        assert_eq!(run(0.2, 1).content_hash(), run(0.2, 1).content_hash());
        assert_ne!(run(0.2, 1).content_hash(), run(0.2, 2).content_hash());
        assert_ne!(run(0.2, 1).content_hash(), run(0.0, 1).content_hash());
        assert_closed(&run(0.2, 1), 2);
    }

    #[test]
    fn subdivide_interpolates_uv_attributes_and_edge_flags() {
        let mut m = primitives::plane(2.0);
        m.faces[0].material = 3;
        m.faces[0].smooth = false;
        for c in &mut m.faces[0].corners {
            c.normal = Some(Vec3::Y);
        }
        m.verts[0].mask = 1.0;
        m.set_sharp(0, 2, true);
        m.set_crease(0, 2, 0.5);
        m.set_seam(1, 3, true);
        let edges = all_edges(&m);
        let out = subdivide_edges(&mut m, &edges, 1, 0.0, 0.0, 0).unwrap();
        assert_eq!(m.face_count(), 4);
        assert!(m.faces.iter().all(|f| f.material == 3 && !f.smooth));
        let center = *out.created_verts.last().unwrap();
        let c_uv = m
            .faces
            .iter()
            .flat_map(|f| f.corners.iter())
            .find(|c| c.vert == center)
            .unwrap()
            .uv;
        assert!((c_uv - Vec2::splat(0.5)).length() < 1e-5);
        assert!(m
            .faces
            .iter()
            .flat_map(|f| f.corners.iter())
            .all(|c| c.normal == Some(Vec3::Y)));
        let mid = out.created_verts[0];
        assert!(m.verts[mid as usize].mask > 0.0 && m.verts[mid as usize].mask < 1.0);
        assert!(!m.edge_attr(0, 2).sharp);
        assert_eq!(m.edge_attrs.values().filter(|e| e.sharp).count(), 2);
        assert_eq!(m.edge_attrs.values().filter(|e| e.seam).count(), 2);
        assert_eq!(m.edge_attrs.values().filter(|e| e.crease == 0.5).count(), 2);
        m.validate().unwrap();
    }

    #[test]
    fn subdivide_wire_edge_chain() {
        let mut m = EditMesh::new("w");
        let a = m.add_vert(Vec3::ZERO);
        let b = m.add_vert(Vec3::X * 3.0);
        m.edge_attrs.insert(edge_key(a, b), Default::default());
        let out = subdivide_edges(&mut m, &[(a, b)], 2, 1.0, 0.5, 1).unwrap();
        assert_eq!(m.vert_count(), 4);
        assert_eq!(m.edge_attrs.len(), 3);
        assert!(!m.edge_attrs.contains_key(&(a, b)));
        assert_eq!(out.selection.edges.len(), 3);
        // smooth/fractal ignorados em arestas sem face: posições uniformes
        assert!((m.co(2) - Vec3::X).length() < 1e-6 && (m.co(3) - Vec3::X * 2.0).length() < 1e-6);
    }

    #[test]
    fn subdivide_errors_leave_mesh_intact() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        let e = Topology::build(&m).edges[0].verts;
        assert!(matches!(
            subdivide_edges(&mut m, &[], 1, 0.0, 0.0, 0),
            Err(ModelError::EmptySelection(_))
        ));
        assert!(matches!(
            subdivide_edges(&mut m, &[(0, 6)], 1, 0.0, 0.0, 0),
            Err(ModelError::MissingEdge(..))
        ));
        for (c, s, f) in [
            (0, 0.0, 0.0),
            (65, 0.0, 0.0),
            (1, -1.0, 0.0),
            (1, f32::NAN, 0.0),
            (1, 11.0, 0.0),
            (1, 0.0, -0.1),
            (1, 0.0, f32::INFINITY),
        ] {
            assert!(
                matches!(
                    subdivide_edges(&mut m, &[e], c, s, f, 0),
                    Err(ModelError::InvalidParameter(_))
                ),
                "{c} {s} {f}"
            );
        }
        assert!(subdivide_faces(&mut m, &[], 1, 0.0).is_err());
        assert!(subdivide_faces(&mut m, &[9], 1, 0.0).is_err());
        assert!(subdivide_faces(&mut m, &[0], 0, 0.0).is_err());
        assert_eq!(m.content_hash(), h);
    }

    // ------------------------------------------------------------- subdivide_faces

    #[test]
    fn subdivide_faces_quad_grid_and_selection() {
        let mut m = primitives::plane(2.0);
        let out = subdivide_faces(&mut m, &[0], 1, 0.0).unwrap();
        assert_eq!(m.face_count(), 4);
        assert_eq!(m.vert_count(), 9);
        assert_eq!(out.selection.faces.len(), 4);
        assert!((area(&m) - 4.0).abs() < 1e-5);
        let mut g = primitives::plane(2.0);
        subdivide_faces(&mut g, &[0], 3, 0.0).unwrap();
        assert_eq!(g.face_count(), 16);
        g.validate().unwrap();
    }

    #[test]
    fn subdivide_faces_triangles_and_ngons_use_a_center_vertex() {
        let mut t = triangle();
        let a0 = area(&t);
        subdivide_faces(&mut t, &[0], 1, 0.0).unwrap();
        assert_eq!(t.face_count(), 3);
        assert_eq!(t.vert_count(), 7);
        assert!(t.faces.iter().all(|f| f.len() == 4));
        assert!((area(&t) - a0).abs() < 1e-5);
        let n0 = triangle().face_normal(0);
        for f in 0..3 {
            assert!((t.face_normal(f) - n0).length() < 1e-4);
        }
        t.validate().unwrap();

        let mut t3 = triangle();
        subdivide_faces(&mut t3, &[0], 3, 0.0).unwrap();
        assert_eq!(t3.face_count(), 12);
        assert!((area(&t3) - a0).abs() < 1e-5);
        assert!(t3.faces.iter().all(|f| f.len() == 4));

        let mut p = primitives::circle(1.0, 5, false);
        let pa = area(&p);
        subdivide_faces(&mut p, &[0], 1, 0.0).unwrap();
        assert_eq!(p.face_count(), 5);
        assert_eq!(p.vert_count(), 11);
        assert!((area(&p) - pa).abs() < 1e-5);
        for f in 0..5 {
            assert!(p.face_normal(f).y > 0.99);
        }
        let mut h = primitives::circle(1.0, 6, false);
        let ha = area(&h);
        subdivide_faces(&mut h, &[0], 3, 0.0).unwrap();
        assert_eq!(h.face_count(), 24);
        assert!((area(&h) - ha).abs() < 1e-4);
        h.validate().unwrap();
        assert_eq!(Topology::build(&h).euler_characteristic(&h), 1);
    }

    #[test]
    fn subdivide_faces_even_cuts_with_non_quads_is_rejected() {
        let mut t = triangle();
        let h = t.content_hash();
        assert!(matches!(
            subdivide_faces(&mut t, &[0], 2, 0.0),
            Err(ModelError::InvalidParameter(_))
        ));
        assert_eq!(t.content_hash(), h);
        let mut q = primitives::plane(1.0);
        subdivide_faces(&mut q, &[0], 2, 0.0).unwrap();
        assert_eq!(q.face_count(), 9);
    }

    #[test]
    fn subdivide_faces_subset_keeps_mesh_watertight() {
        let mut m = primitives::cube(1.0);
        let out = subdivide_faces(&mut m, &[0], 1, 0.0).unwrap();
        // face 0 -> 4 quads; as 4 vizinhas ganham 1 vértice (pentágonos); oposta intacta
        assert_eq!(m.face_count(), 6 - 1 + 4);
        assert_eq!(m.faces.iter().filter(|f| f.len() == 5).count(), 4);
        assert_closed(&m, 2);
        assert_outward(&m);
        assert_eq!(out.selection.faces.len(), 4);
        let mut n = primitives::quad_sphere(1.0, 4);
        subdivide_faces(&mut n, &[0, 1, 2], 1, 1.0).unwrap();
        assert_closed(&n, 2);
    }

    #[test]
    fn subdivide_whole_sphere_is_a_catmull_clark_style_split() {
        let mut m = primitives::quad_sphere(1.0, 4);
        let faces: Vec<u32> = (0..m.face_count() as u32).collect();
        let before = m.face_count();
        let n_edges = Topology::build(&m).edges.len();
        let out = subdivide_faces(&mut m, &faces, 1, 1.0).unwrap();
        assert_eq!(m.face_count(), before * 4);
        assert_eq!(out.selection.faces.len(), before * 4);
        assert_closed(&m, 2);
        assert_outward(&m);
        assert!(m.faces.iter().all(|f| f.len() == 4));
        for &v in out.created_verts.iter().take(n_edges) {
            assert!((m.co(v).length() - 1.0).abs() < 0.01);
        }
    }

    #[test]
    fn subdivide_is_deterministic() {
        let run = || {
            let mut m = primitives::quad_sphere(1.0, 3);
            let faces: Vec<u32> = (0..m.face_count() as u32).step_by(3).collect();
            subdivide_faces(&mut m, &faces, 1, 0.5).unwrap();
            m.content_hash()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn subdivide_large_mesh() {
        let mut m = primitives::quad_sphere(1.0, 64);
        let faces: Vec<u32> = (0..m.face_count() as u32).collect();
        let t0 = std::time::Instant::now();
        subdivide_faces(&mut m, &faces, 1, 0.0).unwrap();
        assert_eq!(m.face_count(), 6 * 64 * 64 * 4);
        assert!(t0.elapsed().as_secs() < 60, "{:?}", t0.elapsed());
        assert!(Topology::build(&m).is_closed_manifold());
    }

    // ------------------------------------------------------------------------ poke

    #[test]
    fn poke_builds_a_pyramid() {
        let mut m = primitives::cube(2.0);
        let out = poke_faces(&mut m, &[0], 0.5, false).unwrap();
        assert_eq!(m.face_count(), 5 + 4);
        assert_eq!(m.vert_count(), 9);
        assert_closed(&m, 2);
        let c = out.created_verts[0];
        let n = primitives::cube(2.0).face_normal(0);
        let center = primitives::cube(2.0).face_center(0);
        assert!((m.co(c) - (center + n * 0.5)).length() < 1e-5);
        assert_eq!(out.created_faces.len(), 4);
        assert_eq!(out.selection.faces.len(), 4);
        assert!(m.faces.iter().filter(|f| f.len() == 3).count() == 4);
        assert_outward(&m);
        // relativo: raio médio de um quad de lado 2 = sqrt(2)
        let mut r = primitives::cube(2.0);
        let out = poke_faces(&mut r, &[0], 1.0, true).unwrap();
        let c = out.created_verts[0];
        assert!(((r.co(c) - center).length() - 2f32.sqrt()).abs() < 1e-4);
        let mut z = primitives::cube(2.0);
        poke_faces(&mut z, &[0, 1, 2], -0.3, false).unwrap();
        assert_closed(&z, 2);
        assert_eq!(z.face_count(), 3 + 12);
    }

    #[test]
    fn poke_attributes_and_ngons() {
        let mut m = primitives::circle(1.0, 7, false);
        for (i, c) in m.faces[0].corners.iter_mut().enumerate() {
            c.uv = Vec2::new(i as f32, 0.0);
        }
        m.faces[0].material = 2;
        m.verts[0].mask = 0.7;
        let out = poke_faces(&mut m, &[0], 0.0, false).unwrap();
        assert_eq!(m.face_count(), 7);
        assert!(m.faces.iter().all(|f| f.len() == 3 && f.material == 2));
        let cid = out.created_verts[0];
        let cuv = m.faces[0].corners[0].uv;
        assert_eq!(m.faces[0].corners[0].vert, cid);
        assert!((cuv.x - 3.0).abs() < 1e-5);
        assert!((m.verts[cid as usize].mask - 0.1).abs() < 1e-5);
        assert_eq!(Topology::build(&m).euler_characteristic(&m), 1);
        m.validate().unwrap();
    }

    #[test]
    fn poke_errors() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        assert!(poke_faces(&mut m, &[], 0.1, false).is_err());
        assert!(poke_faces(&mut m, &[7], 0.1, false).is_err());
        assert!(poke_faces(&mut m, &[0], f32::NAN, false).is_err());
        assert_eq!(m.content_hash(), h);
    }

    // ------------------------------------------------------------------------- rip

    fn grid_center(m: &EditMesh) -> u32 {
        (0..m.vert_count() as u32)
            .min_by(|&a, &b| m.co(a).length().total_cmp(&m.co(b).length()))
            .unwrap()
    }

    #[test]
    fn rip_by_faces_detaches_them() {
        let mut m = primitives::grid(2, 2, 2.0, 2.0);
        let c = grid_center(&m);
        let t = Topology::build(&m);
        let f = t.vert_faces[c as usize][0];
        let out = rip_vertex(&mut m, c, &RipTarget::Faces(vec![f])).unwrap();
        m.validate().unwrap();
        assert_eq!(m.vert_count(), 10);
        assert_eq!(out.created_verts.len(), 1);
        let nv = out.created_verts[0];
        assert_eq!(m.co(nv), m.co(c));
        assert!(m.faces[f as usize].verts().any(|v| v == nv));
        assert!(!m.faces[f as usize].verts().any(|v| v == c));
        assert!(Topology::build(&m).boundary_edge_count() > 8);
        assert_eq!(out.selection.verts.len(), 2);
    }

    #[test]
    fn rip_by_edges_interior_and_boundary() {
        let base = primitives::grid(2, 2, 2.0, 2.0);
        let c = grid_center(&base);
        let t = Topology::build(&base);
        let nbrs = t.vert_neighbors(c);
        let a = nbrs
            .iter()
            .copied()
            .find(|&n| base.co(n).x < -0.5 && base.co(n).z.abs() < 1e-5)
            .unwrap();
        let b = nbrs
            .iter()
            .copied()
            .find(|&n| base.co(n).x > 0.5 && base.co(n).z.abs() < 1e-5)
            .unwrap();
        let mut m = base.clone();
        let out = rip_vertex(&mut m, c, &RipTarget::Edges(vec![(c, a), (b, c)])).unwrap();
        m.validate().unwrap();
        assert_eq!(m.vert_count(), 10);
        let t2 = Topology::build(&m);
        assert_eq!(t2.vert_faces[c as usize].len(), 2);
        assert_eq!(t2.vert_faces[out.created_verts[0] as usize].len(), 2);
        // 1 aresta em vértice interior: erro
        let mut one = base.clone();
        let h = one.content_hash();
        assert!(matches!(
            rip_vertex(&mut one, c, &RipTarget::Edges(vec![(c, a)])),
            Err(ModelError::InvalidParameter(_))
        ));
        assert_eq!(one.content_hash(), h);
        // vértice de fronteira com 1 aresta interior basta
        let bv = (0..base.vert_count() as u32)
            .find(|&v| (base.co(v) - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-5)
            .unwrap();
        let tb = Topology::build(&base);
        let inner = tb.vert_neighbors(bv).into_iter().find(|&n| n == c).unwrap();
        let mut r = base.clone();
        let out = rip_vertex(&mut r, bv, &RipTarget::Edges(vec![(bv, inner)])).unwrap();
        assert_eq!(out.created_verts.len(), 1);
        r.validate().unwrap();
    }

    #[test]
    fn rip_moves_or_copies_edge_attrs() {
        let mut m = primitives::grid(2, 2, 2.0, 2.0);
        let c = grid_center(&m);
        let t = Topology::build(&m);
        let nbrs = t.vert_neighbors(c);
        for &n in &nbrs {
            m.set_seam(c, n, true);
        }
        let f = t.vert_faces[c as usize][0];
        let before = m.edge_attrs.len();
        rip_vertex(&mut m, c, &RipTarget::Faces(vec![f])).unwrap();
        assert!(m.edge_attrs.len() >= before);
        assert!(m.edge_attrs.values().all(|e| e.seam));
        m.validate().unwrap();
    }

    #[test]
    fn rip_errors() {
        let mut m = primitives::grid(2, 2, 2.0, 2.0);
        let c = grid_center(&m);
        let h = m.content_hash();
        let t = Topology::build(&m);
        let all_faces = t.vert_faces[c as usize].clone();
        assert!(rip_vertex(&mut m, 99, &RipTarget::Faces(vec![0])).is_err());
        assert!(rip_vertex(&mut m, c, &RipTarget::Faces(vec![])).is_err());
        assert!(rip_vertex(&mut m, c, &RipTarget::Faces(all_faces)).is_err());
        assert!(rip_vertex(&mut m, c, &RipTarget::Faces(vec![99])).is_err());
        assert!(rip_vertex(&mut m, c, &RipTarget::Edges(vec![])).is_err());
        assert!(rip_vertex(&mut m, c, &RipTarget::Edges(vec![(0, 99)])).is_err());
        let other = t
            .edges
            .iter()
            .find(|e| e.verts.0 != c && e.verts.1 != c)
            .unwrap()
            .verts;
        assert!(rip_vertex(&mut m, c, &RipTarget::Edges(vec![other])).is_err());
        assert_eq!(m.content_hash(), h);
        let loose = m.add_vert(Vec3::splat(9.0));
        let h2 = m.content_hash();
        assert!(rip_vertex(&mut m, loose, &RipTarget::Faces(vec![0])).is_err());
        assert_eq!(m.content_hash(), h2);
    }

    // ----------------------------------------------------------- separate / split

    #[test]
    fn separate_extracts_faces_into_a_new_mesh() {
        let mut m = primitives::cube(1.0);
        let (part, remap) = separate_selected(&mut m, &[0]).unwrap();
        part.validate().unwrap();
        m.validate().unwrap();
        assert_eq!(part.face_count(), 1);
        assert_eq!(part.vert_count(), 4);
        assert_eq!(m.face_count(), 5);
        assert_eq!(
            m.vert_count(),
            8,
            "vértices compartilhados com outras faces ficam"
        );
        assert_eq!(remap.face[0], None);
        assert_eq!(remap.face[1], Some(0));
        assert!(part.name.ends_with(".separate"));
        // atributo de aresta viaja para a nova malha e permanece na original (aresta compartilhada)
        let f_with_edge = (0..6u32)
            .find(|&f| {
                primitives::cube(1.0).faces[f as usize]
                    .edges()
                    .any(|(a, b)| edge_key(a, b) == (0, 1))
            })
            .unwrap();
        let mut m2 = primitives::cube(1.0);
        m2.set_sharp(0, 1, true);
        let (p2, _) = separate_selected(&mut m2, &[f_with_edge]).unwrap();
        assert_eq!(p2.edge_attrs.len(), 1);
        assert!(p2.edge_attrs.values().next().unwrap().sharp);
        assert_eq!(
            m2.edge_attrs.len(),
            1,
            "a aresta continua em outra face da original"
        );
    }

    #[test]
    fn separate_half_sphere_preserves_total_area() {
        let mut m = primitives::quad_sphere(1.0, 6);
        let a0 = area(&m);
        let top: Vec<u32> = (0..m.face_count() as u32)
            .filter(|&f| m.face_center(f).y > 0.0)
            .collect();
        let (part, remap) = separate_selected(&mut m, &top).unwrap();
        assert!((area(&m) + area(&part) - a0).abs() < 1e-3);
        assert_eq!(part.face_count(), top.len());
        part.validate().unwrap();
        m.validate().unwrap();
        assert!(remap.vert.iter().filter(|v| v.is_none()).count() > 0);
        assert!(Topology::build(&part).boundary_edge_count() > 0);
        let mut all = primitives::cube(1.0);
        let (p, _) = separate_selected(&mut all, &[0, 1, 2, 3, 4, 5]).unwrap();
        assert!(all.is_empty());
        assert!(Topology::build(&p).is_closed_manifold());
    }

    #[test]
    fn separate_errors() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        assert!(separate_selected(&mut m, &[]).is_err());
        assert!(separate_selected(&mut m, &[9]).is_err());
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn split_detaches_boundary_vertices() {
        let mut m = primitives::quad_sphere(1.0, 4);
        let a0 = area(&m);
        let top: Vec<u32> = (0..m.face_count() as u32)
            .filter(|&f| m.face_center(f).y > 0.0)
            .collect();
        m.set_sharp(0, 1, true);
        let nv0 = m.vert_count();
        let out = split_selected(&mut m, &top).unwrap();
        m.validate().unwrap();
        assert!(!out.created_verts.is_empty());
        assert_eq!(m.vert_count(), nv0 + out.created_verts.len());
        assert!((area(&m) - a0).abs() < 1e-4);
        let t = Topology::build(&m);
        assert_eq!(
            t.face_components(&m).1,
            2,
            "as duas metades não compartilham mais vértices"
        );
        assert!(!t.is_closed_manifold());
        assert_eq!(out.selection.faces.len(), top.len());
        let boundary_orig: BTreeSet<u32> = out
            .created_verts
            .iter()
            .map(|&v| (0..nv0 as u32).find(|&o| m.co(o) == m.co(v)).unwrap())
            .collect();
        for &f in &top {
            assert!(m.faces[f as usize]
                .verts()
                .all(|v| !boundary_orig.contains(&v)));
        }
    }

    #[test]
    fn split_noop_cases_and_attr_duplication() {
        let mut m = primitives::cube(1.0);
        let all: Vec<u32> = (0..6).collect();
        let h = m.content_hash();
        let out = split_selected(&mut m, &all).unwrap();
        assert!(out.created_verts.is_empty());
        assert_eq!(m.content_hash(), h);
        assert!(split_selected(&mut m, &[]).is_err());
        assert!(split_selected(&mut m, &[9]).is_err());
        let mut c = primitives::cube(1.0);
        let (a, b) = c.faces[0].edges().next().unwrap();
        c.set_seam(a, b, true);
        let out = split_selected(&mut c, &[0]).unwrap();
        assert_eq!(out.created_verts.len(), 4);
        assert_eq!(c.edge_attrs.values().filter(|e| e.seam).count(), 2);
        c.validate().unwrap();
    }

    // -------------------------------------------------------------------- marcação

    #[test]
    fn mark_attributes_in_batch() {
        let mut m = primitives::cube(1.0);
        let edges = all_edges(&m);
        let out = mark_sharp(&mut m, &edges[..4], true).unwrap();
        assert_eq!(out.selection.edges.len(), 4);
        assert_eq!(m.edge_attrs.len(), 4);
        mark_seam(&mut m, &edges[2..6], true).unwrap();
        assert_eq!(m.edge_attrs.len(), 6);
        set_crease(&mut m, &edges[..2], 0.75).unwrap();
        set_bevel_weight(&mut m, &edges[..1], 0.5).unwrap();
        let e0 = m.edge_attr(edges[0].0, edges[0].1);
        assert!(e0.sharp && e0.crease == 0.75 && e0.bevel_weight == 0.5);
        mark_sharp(&mut m, &edges, false).unwrap();
        mark_seam(&mut m, &edges, false).unwrap();
        set_crease(&mut m, &edges, 0.0).unwrap();
        set_bevel_weight(&mut m, &edges, 0.0).unwrap();
        assert!(m.edge_attrs.is_empty());
        mark_sharp(&mut m, &[(edges[0].1, edges[0].0), edges[0]], true).unwrap();
        assert_eq!(m.edge_attrs.len(), 1);
    }

    #[test]
    fn mark_keeps_wire_edges_alive() {
        let mut m = EditMesh::new("w");
        let a = m.add_vert(Vec3::ZERO);
        let b = m.add_vert(Vec3::X);
        m.edge_attrs.insert(edge_key(a, b), Default::default());
        mark_sharp(&mut m, &[(a, b)], true).unwrap();
        mark_sharp(&mut m, &[(a, b)], false).unwrap();
        assert_eq!(
            m.edge_attrs.len(),
            1,
            "aresta wire não pode sumir ao voltar ao padrão"
        );
    }

    #[test]
    fn mark_errors_are_atomic() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        let e = Topology::build(&m).edges[0].verts;
        assert!(mark_sharp(&mut m, &[], true).is_err());
        assert!(mark_sharp(&mut m, &[e, (0, 6)], true).is_err());
        assert!(set_crease(&mut m, &[e], 1.5).is_err());
        assert!(set_crease(&mut m, &[e], f32::NAN).is_err());
        assert!(set_bevel_weight(&mut m, &[e], -0.1).is_err());
        assert!(mark_seam(&mut m, &[(0, 99)], true).is_err());
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn set_smooth_flat() {
        let mut m = primitives::cube(1.0);
        let out = set_faces_smooth(&mut m, &[0, 2], true).unwrap();
        assert!(m.faces[0].smooth && m.faces[2].smooth && !m.faces[1].smooth);
        assert_eq!(out.selection.faces.len(), 2);
        set_faces_smooth(&mut m, &[0], false).unwrap();
        assert!(!m.faces[0].smooth);
        assert!(set_faces_smooth(&mut m, &[], true).is_err());
        assert!(set_faces_smooth(&mut m, &[10], true).is_err());
    }

    // ------------------------------------------------------------------ hide/reveal

    #[test]
    fn hide_and_reveal_roundtrip() {
        let mut m = primitives::cube(1.0);
        hide_faces(&mut m, &[0]).unwrap();
        assert!(m.faces[0].hidden && !m.faces[1].hidden);
        assert!(
            m.verts.iter().all(|v| !v.hidden),
            "vértices compartilhados continuam visíveis"
        );
        hide_faces(&mut m, &[1, 2, 3, 4, 5]).unwrap();
        assert!(m.verts.iter().all(|v| v.hidden));
        let out = reveal_all(&mut m);
        assert_eq!(out.selection.faces.len(), 6);
        assert_eq!(out.selection.verts.len(), 8);
        assert!(m.faces.iter().all(|f| !f.hidden) && m.verts.iter().all(|v| !v.hidden));
        assert!(reveal_all(&mut m).selection.is_empty());
        hide_verts(&mut m, &[0]).unwrap();
        assert_eq!(m.faces.iter().filter(|f| f.hidden).count(), 3);
        assert!(m.verts[0].hidden);
        assert!(hide_faces(&mut m, &[]).is_err());
        assert!(hide_faces(&mut m, &[9]).is_err());
        assert!(hide_verts(&mut m, &[]).is_err());
        assert!(hide_verts(&mut m, &[99]).is_err());
    }

    // ---------------------------------------------------------------- seleção por traço

    #[test]
    fn select_sharp_edges_by_angle() {
        let m = primitives::cube(1.0);
        let s = select_sharp_edges(&m, 0.1).unwrap();
        assert_eq!(s.edges.len(), 12);
        assert_eq!(s.verts.len(), 8);
        assert!(select_sharp_edges(&m, 1.6).unwrap().edges.is_empty());
        let sph = primitives::quad_sphere(1.0, 8);
        assert!(select_sharp_edges(&sph, 0.8).unwrap().edges.is_empty());
        assert!(!select_sharp_edges(&sph, 0.01).unwrap().edges.is_empty());
        let g = primitives::grid(2, 2, 1.0, 1.0);
        assert!(select_sharp_edges(&g, 0.0).unwrap().edges.is_empty());
        for bad in [-1.0, f32::NAN, 4.0] {
            assert!(select_sharp_edges(&m, bad).is_err());
        }
    }

    #[test]
    fn select_loose_variants() {
        let mut m = primitives::cube(1.0);
        let loose = m.add_vert(Vec3::splat(5.0));
        let a = m.add_vert(Vec3::splat(6.0));
        let b = m.add_vert(Vec3::splat(7.0));
        m.edge_attrs.insert(edge_key(a, b), Default::default());
        let s = select_loose(&m, true, false, false);
        assert_eq!(s.verts.iter().copied().collect::<Vec<_>>(), vec![loose]);
        let e = select_loose(&m, false, true, false);
        assert_eq!(e.edges.len(), 1);
        assert!(e.verts.contains(&a) && e.verts.contains(&b));
        assert!(select_loose(&m, false, false, true).is_empty());
        m.append(&primitives::plane(1.0));
        let f = select_loose(&m, false, false, true);
        assert_eq!(f.faces.len(), 1);
        assert!(select_loose(&primitives::cube(1.0), true, true, true).is_empty());
    }

    #[test]
    fn select_non_manifold_kinds() {
        let opts = NonManifoldOptions::default();
        assert!(select_non_manifold(&primitives::cube(1.0), opts).is_empty());
        assert!(select_non_manifold(&primitives::quad_sphere(1.0, 3), opts).is_empty());
        let g = primitives::grid(2, 2, 1.0, 1.0);
        let s = select_non_manifold(&g, opts);
        assert_eq!(s.edges.len(), 8);
        let only_wire = NonManifoldOptions {
            wire: true,
            boundary: false,
            multi_face: false,
            non_contiguous: false,
            verts: false,
        };
        assert!(select_non_manifold(&g, only_wire).is_empty());
        let mut w = primitives::cube(1.0);
        let a = w.add_vert(Vec3::splat(6.0));
        let b = w.add_vert(Vec3::splat(7.0));
        w.edge_attrs.insert(edge_key(a, b), Default::default());
        let s = select_non_manifold(&w, only_wire);
        assert_eq!(s.edges.len(), 1);
        let mut book = EditMesh::new("book");
        let (p, q) = (book.add_vert(Vec3::ZERO), book.add_vert(Vec3::X));
        for d in [Vec3::Y, Vec3::Z, -Vec3::Y] {
            let c = book.add_vert(Vec3::X + d);
            let e = book.add_vert(d);
            book.add_face(&[p, q, c, e]).unwrap();
        }
        let multi = NonManifoldOptions {
            wire: false,
            boundary: false,
            multi_face: true,
            non_contiguous: false,
            verts: false,
        };
        let s = select_non_manifold(&book, multi);
        assert_eq!(s.edges.len(), 1);
        let mut flipped = primitives::cube(1.0);
        flipped.flip_face(0).unwrap();
        let nc = NonManifoldOptions {
            wire: false,
            boundary: false,
            multi_face: false,
            non_contiguous: true,
            verts: false,
        };
        let s = select_non_manifold(&flipped, nc);
        assert_eq!(s.edges.len(), 4);
        let mut bow = EditMesh::new("bow");
        let o = bow.add_vert(Vec3::ZERO);
        let vs: Vec<u32> = [Vec3::X, Vec3::Z, -Vec3::X, -Vec3::Z]
            .iter()
            .map(|&p| bow.add_vert(p))
            .collect();
        bow.add_face(&[o, vs[1], vs[0]]).unwrap();
        bow.add_face(&[o, vs[3], vs[2]]).unwrap();
        let only_v = NonManifoldOptions {
            wire: false,
            boundary: false,
            multi_face: false,
            non_contiguous: false,
            verts: true,
        };
        let s = select_non_manifold(&bow, only_v);
        assert_eq!(s.verts.iter().copied().collect::<Vec<_>>(), vec![o]);
    }

    #[test]
    fn select_interior_faces_finds_triple_sheets() {
        let mut m = EditMesh::new("triple");
        let ids: Vec<u32> = [Vec3::ZERO, Vec3::X, Vec3::new(1.0, 0.0, 1.0), Vec3::Z]
            .iter()
            .map(|&p| m.add_vert(p))
            .collect();
        for _ in 0..3 {
            m.add_face(&ids).unwrap();
        }
        let s = select_interior_faces(&m);
        assert_eq!(s.faces.len(), 3);
        assert!(select_interior_faces(&primitives::cube(1.0)).is_empty());
        assert!(select_interior_faces(&primitives::plane(1.0)).is_empty());
    }

    #[test]
    fn operators_chain_without_corrupting_the_mesh() {
        let mut m = primitives::quad_sphere(1.0, 3);
        let faces: Vec<u32> = (0..m.face_count() as u32).step_by(2).collect();
        let out = subdivide_faces(&mut m, &faces, 1, 0.5).unwrap();
        let sel: Vec<u32> = out.selection.faces.iter().copied().take(5).collect();
        let out2 = poke_faces(&mut m, &sel, 0.1, true).unwrap();
        let created: Vec<u32> = out2.created_faces.clone();
        set_faces_smooth(&mut m, &created, false).unwrap();
        assert_closed(&m, 2);
        let part: Vec<u32> = (0..m.face_count() as u32)
            .filter(|&f| m.face_center(f).y > 0.3)
            .collect();
        let (top, _) = separate_selected(&mut m, &part).unwrap();
        top.validate().unwrap();
        m.validate().unwrap();
    }

    /// Sequências aleatórias (determinísticas) de operadores de vários módulos: nenhum
    /// pode entrar em pânico nem deixar a malha inválida (erros `Err` são aceitos).
    #[test]
    fn fuzz_operator_sequences_keep_meshes_valid() {
        use crate::ops::{delete, dissolve, fill, flip, merge, smooth, transform, triangulate};
        let bases = [
            primitives::quad_sphere(1.0, 3),
            primitives::uv_sphere(1.0, 6, 8),
            primitives::capsule(0.5, 1.0, 8, 2),
            primitives::cylinder(1.0, 2.0, 7, 2, true),
            primitives::cube_subdivided(1.0, 2),
            primitives::grid(4, 3, 4.0, 3.0),
        ];
        let mut rng = crate::ops::smooth::XorShift64::new(2024);
        for (bi, base) in bases.iter().enumerate() {
            for round in 0..15 {
                let mut m = base.clone();
                for step in 0..8 {
                    if m.face_count() == 0 {
                        break;
                    }
                    let nf = m.face_count() as u64;
                    let pick_faces =
                        |rng: &mut crate::ops::smooth::XorShift64, m: &EditMesh| -> Vec<u32> {
                            let n = 1 + rng.next_u64() % 4;
                            (0..n)
                                .map(|_| (rng.next_u64() % m.face_count() as u64) as u32)
                                .collect()
                        };
                    let faces = pick_faces(&mut rng, &m);
                    let verts: Vec<u32> = (0..3)
                        .map(|_| (rng.next_u64() % m.vert_count().max(1) as u64) as u32)
                        .collect();
                    let edges: Vec<EdgeKey> = {
                        let t = Topology::build(&m);
                        if t.edges.is_empty() {
                            Vec::new()
                        } else {
                            (0..3)
                                .map(|_| {
                                    t.edges[(rng.next_u64() % t.edges.len() as u64) as usize].verts
                                })
                                .collect()
                        }
                    };
                    let op = rng.next_u64() % 22;
                    let before = m.clone();
                    let res: ModelResult<OpOutput> = match op {
                        0 => delete::delete_faces_keep_verts(&mut m, &faces),
                        1 => delete::delete_faces(&mut m, &faces),
                        2 => delete::delete_edges(&mut m, &edges),
                        3 => delete::delete_verts(&mut m, &verts),
                        4 => dissolve::dissolve_edges(&mut m, &edges, rng.next_u64() & 1 == 0),
                        5 => dissolve::dissolve_verts(
                            &mut m,
                            &verts,
                            rng.next_u64() & 1 == 0,
                            rng.next_u64() & 1 == 0,
                        ),
                        6 => dissolve::dissolve_faces(&mut m, &faces),
                        7 => dissolve::limited_dissolve(
                            &mut m,
                            0.2,
                            true,
                            dissolve::Delimit::default(),
                        ),
                        8 => triangulate::triangulate_faces(
                            &mut m,
                            &faces,
                            triangulate::QuadMethod::Beauty,
                            triangulate::NgonMethod::Beauty,
                        ),
                        9 => {
                            let all: Vec<u32> = (0..m.face_count() as u32).collect();
                            triangulate::tris_to_quads(&mut m, &all, 0.5, true, true, true)
                        }
                        10 => subdivide_faces(&mut m, &faces, 1, 0.3),
                        11 => subdivide_edges(
                            &mut m,
                            &edges,
                            1 + (rng.next_u64() % 2) as usize,
                            0.0,
                            0.0,
                            1,
                        ),
                        12 => poke_faces(&mut m, &faces, 0.1, true),
                        13 => merge::merge_verts(&mut m, &verts, merge::MergeMode::Center),
                        14 => merge::merge_by_distance(
                            &mut m,
                            None,
                            0.2,
                            merge::ByDistanceOptions::default(),
                        ),
                        15 => flip::flip_faces(&mut m, &faces),
                        16 => flip::recalc_normals_outside(&mut m, false),
                        17 => fill::fill_all_holes(&mut m, 8),
                        18 => split_selected(&mut m, &faces),
                        19 => rip_vertex(&mut m, verts[0], &RipTarget::Faces(faces.clone())),
                        20 => smooth::smooth_verts(
                            &mut m,
                            &verts,
                            0.5,
                            2,
                            false,
                            [true; 3],
                            smooth::SmoothWeights::Cotangent,
                        ),
                        _ => transform::translate_verts(&mut m, &verts, Vec3::new(0.01, 0.0, 0.0)),
                    };
                    match res {
                        Ok(_) => m.validate().unwrap_or_else(|e| {
                            panic!("base {bi} rodada {round} passo {step} op {op}: malha inválida: {e}")
                        }),
                        Err(_) => assert_eq!(
                            m.content_hash(),
                            before.content_hash(),
                            "base {bi} rodada {round} passo {step} op {op}: erro deixou a malha alterada"
                        ),
                    }
                    let _ = nf;
                }
            }
        }
    }
}
