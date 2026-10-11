//! Bridge Edge Loops: conecta dois loops de mesmo comprimento com quads.
//!
//! O alinhamento (deslocamento cíclico e sentido do segundo loop) é escolhido
//! automaticamente pela menor soma de distâncias ao quadrado; o enrolamento
//! das faces novas é deduzido das faces que já usam as arestas dos loops, de
//! modo que a malha resultante tenha normais consistentes. Com `twist` o
//! usuário gira o alinhamento manualmente (só para loops fechados).

use super::extrude::corner_uv;
use super::loops::EdgePath;
use crate::error::{ModelError, ModelResult};
use crate::mesh::{EVert, EditMesh, Face};
use crate::ops::OpOutput;
use crate::select::Selection;
use crate::topology::Topology;
use glam::{Vec2, Vec3};
use std::collections::HashSet;

/// Interpolação das fileiras intermediárias (`segments > 1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeInterpolation {
    /// Reta entre os vértices correspondentes.
    Linear,
    /// Curva de Hermite cujas tangentes saem de cada loop "para fora" das
    /// faces que o tocam (equivalente ao *Blend Path/Surface* do Blender).
    /// Sem faces vizinhas cai para a interpolação linear.
    Smooth,
}

/// Nº máximo de candidatos de deslocamento avaliados para loops grandes.
const MAX_SHIFT_CANDIDATES: usize = 48;

/// Direção (no sentido dos índices) em que a primeira face encontrada percorre
/// o loop: `Some(true)` = `v[i] -> v[i+1]`.
fn face_direction(mesh: &EditMesh, topo: &Topology, verts: &[u32], closed: bool) -> Option<bool> {
    let n = verts.len();
    let count = if closed { n } else { n - 1 };
    for i in 0..count {
        let (a, b) = (verts[i], verts[(i + 1) % n]);
        if let Some(&f) = topo.edge_faces(a, b).first() {
            for (x, y) in mesh.faces[f as usize].edges() {
                if (x, y) == (a, b) {
                    return Some(true);
                }
                if (x, y) == (b, a) {
                    return Some(false);
                }
            }
        }
    }
    None
}

fn shift_cost(mesh: &EditMesh, a: &[u32], b: &[u32], shift: usize) -> f32 {
    let n = a.len();
    (0..n)
        .map(|i| mesh.co(a[i]).distance_squared(mesh.co(b[(i + shift) % n])))
        .sum()
}

/// Melhor deslocamento cíclico de `b` (já na direção escolhida) e seu custo.
fn best_shift_closed(mesh: &EditMesh, a: &[u32], b: &[u32]) -> (usize, f32) {
    let n = a.len();
    let mut candidates: Vec<usize> = (0..n).collect();
    if n > MAX_SHIFT_CANDIDATES {
        let a0 = mesh.co(a[0]);
        candidates.sort_by(|&i, &j| {
            let (di, dj) = (
                mesh.co(b[i]).distance_squared(a0),
                mesh.co(b[j]).distance_squared(a0),
            );
            di.partial_cmp(&dj)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(i.cmp(&j))
        });
        candidates.truncate(MAX_SHIFT_CANDIDATES);
        candidates.sort_unstable();
    }
    let mut best = (0usize, f32::MAX);
    for s in candidates {
        let c = shift_cost(mesh, a, b, s);
        if c < best.1 {
            best = (s, c);
        }
    }
    best
}

/// Direção "para fora" das faces que tocam `v` (soma de `pos - centro da face`).
fn outward_dir(mesh: &EditMesh, topo: &Topology, v: u32) -> Vec3 {
    let mut d = Vec3::ZERO;
    for &f in &topo.vert_faces[v as usize] {
        d += mesh.co(v) - mesh.face_center(f);
    }
    d.try_normalize().unwrap_or(Vec3::ZERO)
}

/// UV de `v` na face que usa a aresta `(v, w)`, se existir.
fn edge_uv(mesh: &EditMesh, topo: &Topology, v: u32, w: u32) -> Option<Vec2> {
    let f = *topo.edge_faces(v, w).first()?;
    corner_uv(&mesh.faces[f as usize], v)
}

/// Conecta `loop_a` e `loop_b` com `segments` fileiras de quads.
///
/// * Os dois loops devem ter o mesmo número de vértices e ambos serem
///   abertos ou ambos fechados (caso contrário `InvalidParameter`), sem
///   vértices repetidos nem compartilhados entre si.
/// * `segments >= 1`: número de fileiras de quads (`segments - 1` anéis
///   intermediários de vértices novos).
/// * `twist`: rotação adicional (em vértices) sobre o alinhamento automático;
///   só vale para loops fechados.
/// * `interpolation`: forma dos anéis intermediários.
///
/// O alinhamento automático escolhe o sentido do segundo loop e o
/// deslocamento cíclico que minimizam a distância total — a menos que as
/// faces já existentes imponham o sentido (para manter as normais
/// consistentes). Para loops com mais de 48 vértices só os 48 deslocamentos
/// cujo vértice de `loop_b` está mais perto do primeiro de `loop_a` são avaliados.
///
/// UVs: as bordas herdam a UV das faces vizinhas (se existirem; senão uma
/// parametrização planar) e as fileiras intermediárias interpolam. Material e
/// suavidade vêm da face vizinha de `loop_a` (ou `loop_b`).
///
/// Seleção devolvida: as faces criadas.
pub fn bridge_edge_loops(
    mesh: &mut EditMesh,
    loop_a: &EdgePath,
    loop_b: &EdgePath,
    segments: u32,
    twist: i32,
    interpolation: BridgeInterpolation,
) -> ModelResult<OpOutput> {
    if segments == 0 || segments > 256 {
        return Err(ModelError::InvalidParameter(format!(
            "segments deve estar em 1..=256 (recebeu {segments})"
        )));
    }
    let (la, lb) = (&loop_a.verts, &loop_b.verts);
    if la.len() != lb.len() {
        return Err(ModelError::InvalidParameter(format!(
            "os loops precisam ter o mesmo comprimento ({} vs {})",
            la.len(),
            lb.len()
        )));
    }
    if loop_a.closed != loop_b.closed {
        return Err(ModelError::InvalidParameter(
            "um loop é aberto e o outro fechado".into(),
        ));
    }
    let closed = loop_a.closed;
    let n = la.len();
    let min_len = if closed { 3 } else { 2 };
    if n < min_len {
        return Err(ModelError::InvalidParameter(format!(
            "loops muito curtos: {n} vértices (mínimo {min_len})"
        )));
    }
    if twist != 0 && !closed {
        return Err(ModelError::InvalidParameter(
            "twist só se aplica a loops fechados".into(),
        ));
    }
    let mut seen = HashSet::new();
    for &v in la.iter().chain(lb.iter()) {
        if v as usize >= mesh.verts.len() {
            return Err(ModelError::VertexOutOfRange(v, mesh.verts.len()));
        }
        if !seen.insert(v) {
            return Err(ModelError::InvalidParameter(format!(
                "vértice {v} repetido nos loops (loops devem ser disjuntos e sem repetição)"
            )));
        }
    }
    let topo = Topology::build(mesh);

    // ---- sentido/alinhamento
    let sa = face_direction(mesh, &topo, la, closed);
    let sb = face_direction(mesh, &topo, lb, closed);
    let b_fwd: Vec<u32> = lb.clone();
    let b_rev: Vec<u32> = lb.iter().rev().copied().collect();
    // custo de cada orientação (com melhor deslocamento, se fechado)
    let eval = |b: &[u32]| -> (usize, f32) {
        if closed {
            best_shift_closed(mesh, la, b)
        } else {
            (0, shift_cost(mesh, la, b, 0))
        }
    };
    // `flip`: o quad percorre a_{i+1} -> a_i (a face de A percorre a_i -> a_{i+1})
    let (rev_b, flip) = match (sa, sb) {
        (Some(a_fwd), Some(b_fwd_dir)) => {
            let flip = a_fwd;
            let required_b_forward = !flip;
            (b_fwd_dir != required_b_forward, flip)
        }
        (Some(a_fwd), None) => {
            let flip = a_fwd;
            let rev = eval(&b_rev).1 < eval(&b_fwd).1;
            (rev, flip)
        }
        (None, Some(b_dir)) => {
            let rev = eval(&b_rev).1 < eval(&b_fwd).1;
            let required_b_forward = b_dir != rev;
            (rev, !required_b_forward)
        }
        (None, None) => (eval(&b_rev).1 < eval(&b_fwd).1, false),
    };
    let b_seq: &[u32] = if rev_b { &b_rev } else { &b_fwd };
    let shift = if closed {
        let (s, _) = eval(b_seq);
        ((s as i64 + twist as i64).rem_euclid(n as i64)) as usize
    } else {
        0
    };
    let b_al: Vec<u32> = (0..n).map(|i| b_seq[(i + shift) % n]).collect();

    // ---- fileiras
    let rows = segments as usize + 1;
    let mut grid: Vec<Vec<u32>> = vec![la.clone()];
    let mut created_verts = Vec::new();
    let out_a: Vec<Vec3> = la.iter().map(|&v| outward_dir(mesh, &topo, v)).collect();
    let out_b: Vec<Vec3> = b_al.iter().map(|&v| outward_dir(mesh, &topo, v)).collect();
    for r in 1..rows - 1 {
        let t = r as f32 / segments as f32;
        let mut row = Vec::with_capacity(n);
        for i in 0..n {
            let (va, vb) = (&mesh.verts[la[i] as usize], &mesh.verts[b_al[i] as usize]);
            let mut ev = EVert::lerp(va, vb, t);
            if interpolation == BridgeInterpolation::Smooth {
                let (pa, pb) = (va.co, vb.co);
                let d = pa.distance(pb);
                let (ta, tb) = (out_a[i], out_b[i]);
                if ta != Vec3::ZERO && tb != Vec3::ZERO {
                    let (t2, t3) = (t * t, t * t * t);
                    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
                    let h10 = t3 - 2.0 * t2 + t;
                    let h01 = -2.0 * t3 + 3.0 * t2;
                    let h11 = t3 - t2;
                    ev.co = pa * h00 + ta * d * h10 + pb * h01 + (-tb) * d * h11;
                }
            }
            let id = mesh.push_vert(ev);
            created_verts.push(id);
            row.push(id);
        }
        grid.push(row);
    }
    grid.push(b_al.clone());

    // ---- faces
    let count = if closed { n } else { n - 1 };
    let template: Option<Face> = (0..count)
        .find_map(|i| {
            topo.edge_faces(la[i], la[(i + 1) % n])
                .first()
                .map(|&f| mesh.faces[f as usize].clone())
        })
        .or_else(|| {
            (0..count).find_map(|i| {
                topo.edge_faces(b_al[i], b_al[(i + 1) % n])
                    .first()
                    .map(|&f| mesh.faces[f as usize].clone())
            })
        });
    let mut created_faces = Vec::new();
    for i in 0..count {
        let j = (i + 1) % n;
        let (u_i, u_j) = (i as f32 / count as f32, (i + 1) as f32 / count as f32);
        let uv_a = (
            edge_uv(mesh, &topo, la[i], la[j]).unwrap_or(Vec2::new(u_i, 0.0)),
            edge_uv(mesh, &topo, la[j], la[i]).unwrap_or(Vec2::new(u_j, 0.0)),
        );
        let uv_b = (
            edge_uv(mesh, &topo, b_al[i], b_al[j]).unwrap_or(Vec2::new(u_i, 1.0)),
            edge_uv(mesh, &topo, b_al[j], b_al[i]).unwrap_or(Vec2::new(u_j, 1.0)),
        );
        for r in 0..rows - 1 {
            let (t0, t1) = (r as f32 / segments as f32, (r + 1) as f32 / segments as f32);
            // cantos no sentido [i,r] [j,r] [j,r+1] [i,r+1]
            let verts = [grid[r][i], grid[r][j], grid[r + 1][j], grid[r + 1][i]];
            let uvs = [
                uv_a.0.lerp(uv_b.0, t0),
                uv_a.1.lerp(uv_b.1, t0),
                uv_a.1.lerp(uv_b.1, t1),
                uv_a.0.lerp(uv_b.0, t1),
            ];
            let mut face = match &template {
                Some(t) => t.with_verts_from(&verts),
                None => Face::new(&verts),
            };
            for (c, uv) in face.corners.iter_mut().zip(uvs) {
                c.uv = uv;
            }
            if flip {
                // [j,r] [i,r] [i,r+1] [j,r+1]
                face.corners.swap(0, 1);
                face.corners.swap(2, 3);
            }
            mesh.faces.push(face);
            created_faces.push((mesh.faces.len() - 1) as u32);
        }
    }
    Ok(OpOutput {
        selection: Selection::from_faces(mesh, created_faces.iter().copied()),
        created_verts,
        created_faces,
        remap: None,
    })
}

// -------------------------------------------------------------------- testes

#[cfg(test)]
mod tests {
    use super::super::extrude::test_util::*;
    use super::super::loops::boundary_loop;
    use super::*;
    use crate::mesh::edge_key;
    use crate::primitives;
    use std::collections::HashSet;

    /// Cada aresta dirigida (a->b) aparece no máximo uma vez: orientação consistente.
    fn assert_consistent_orientation(m: &EditMesh) {
        let mut seen = HashSet::new();
        for f in &m.faces {
            for (a, b) in f.edges() {
                assert!(
                    seen.insert((a, b)),
                    "aresta dirigida ({a},{b}) repetida: enrolamento inconsistente"
                );
            }
        }
    }

    fn rim_loops(m: &EditMesh) -> (EdgePath, EdgePath) {
        let t = Topology::build(m);
        let mut loops: Vec<EdgePath> = Vec::new();
        let mut covered: HashSet<u32> = HashSet::new();
        for e in &t.edges {
            if e.faces.len() == 1 && !covered.contains(&e.verts.0) {
                let lp = boundary_loop(m, &t, e.verts).unwrap();
                covered.extend(lp.verts.iter().copied());
                loops.push(lp);
            }
        }
        assert_eq!(loops.len(), 2);
        (loops.remove(0), loops.remove(0))
    }

    #[test]
    fn two_open_tubes_bridged_twice_make_a_closed_torus() {
        let mut lower = primitives::cylinder(1.0, 1.0, 8, 1, false);
        let mut upper = primitives::cylinder(1.0, 1.0, 8, 1, false);
        lower.transform_positions(|p| p + Vec3::new(0.0, -2.0, 0.0));
        upper.transform_positions(|p| p + Vec3::new(0.0, 2.0, 0.0));
        let mut m = lower.clone();
        let voff = m.vert_count() as u32;
        m.append(&upper);
        let t = Topology::build(&m);
        let rim = |upper_half: bool, top: bool| -> EdgePath {
            let e = t
                .edges
                .iter()
                .find(|e| {
                    e.faces.len() == 1
                        && (e.verts.0 >= voff) == upper_half
                        && ((m.co(e.verts.0).y > if upper_half { 2.0 } else { -2.0 }) == top)
                })
                .unwrap()
                .verts;
            boundary_loop(&m, &t, e).unwrap()
        };
        let (l_bot, l_top, u_bot, u_top) = (
            rim(false, false),
            rim(false, true),
            rim(true, false),
            rim(true, true),
        );
        let o1 =
            bridge_edge_loops(&mut m, &l_top, &u_bot, 1, 0, BridgeInterpolation::Linear).unwrap();
        assert_eq!(o1.created_faces.len(), 8);
        let tm = Topology::build(&m);
        assert_eq!(tm.boundary_edge_count(), 16);
        assert_eq!(tm.non_manifold_edge_count(), 0);
        assert_consistent_orientation(&m);
        let o2 =
            bridge_edge_loops(&mut m, &u_top, &l_bot, 2, 0, BridgeInterpolation::Linear).unwrap();
        assert_eq!(o2.created_faces.len(), 16);
        assert_closed(&m, Some(0));
        assert_consistent_orientation(&m);
        assert_eq!(o2.selection.faces.len(), 16);
    }

    #[test]
    fn two_circles_with_caps_become_a_closed_cylinder() {
        // base com normal -Y e topo com normal +Y
        let mut bottom = primitives::circle(1.0, 12, false);
        bottom.faces[0].corners.reverse();
        let mut top = primitives::circle(1.0, 12, false);
        top.transform_positions(|p| p + Vec3::Y);
        let (n_bottom, n_top) = (bottom.face_normal(0), top.face_normal(0));
        assert!(n_bottom.y < 0.0 && n_top.y > 0.0);
        let mut m = bottom.clone();
        let (voff, _) = m.append(&top);
        let t = Topology::build(&m);
        let a = boundary_loop(
            &m,
            &t,
            t.edges
                .iter()
                .find(|e| e.faces.len() == 1 && e.verts.0 < voff)
                .unwrap()
                .verts,
        )
        .unwrap();
        let b = boundary_loop(
            &m,
            &t,
            t.edges
                .iter()
                .find(|e| e.faces.len() == 1 && e.verts.0 >= voff)
                .unwrap()
                .verts,
        )
        .unwrap();
        bridge_edge_loops(&mut m, &a, &b, 1, 0, BridgeInterpolation::Linear).unwrap();
        assert_closed(&m, Some(2));
        assert_consistent_orientation(&m);
        assert_outward_star(&m);
        // cilindro de raio 1 e altura 1 inscrito em polígono de 12 lados
        let expected = 0.5 * 12.0 * (std::f32::consts::TAU / 12.0).sin();
        assert!(
            (signed_volume(&m) - expected).abs() < 1e-3,
            "{}",
            signed_volume(&m)
        );
    }

    #[test]
    fn auto_alignment_undoes_rotation_and_reversal() {
        let mut m = EditMesh::new("wires");
        let n = 10u32;
        let ring = |m: &mut EditMesh, y: f32| -> Vec<u32> {
            (0..n)
                .map(|i| {
                    let a = std::f32::consts::TAU * i as f32 / n as f32;
                    m.add_vert(Vec3::new(a.cos(), y, -a.sin()))
                })
                .collect()
        };
        let a = ring(&mut m, 0.0);
        let mut b = ring(&mut m, 1.0);
        b.rotate_left(3);
        b.reverse();
        let la = EdgePath::from_verts(a.clone(), true);
        let lb = EdgePath::from_verts(b, true);
        let out = bridge_edge_loops(&mut m, &la, &lb, 1, 0, BridgeInterpolation::Linear).unwrap();
        assert_eq!(out.created_faces.len(), 10);
        // cada degrau liga pontos de mesma (x, z): nada torcido
        for &f in &out.created_faces {
            let vs: Vec<u32> = m.faces[f as usize].verts().collect();
            let mut by_y: Vec<Vec3> = vs.iter().map(|&v| m.co(v)).collect();
            by_y.sort_by(|p, q| p.y.partial_cmp(&q.y).unwrap());
            let (lo, hi) = ((by_y[0], by_y[1]), (by_y[2], by_y[3]));
            let close = |p: Vec3, q: Vec3| (p.x - q.x).abs() < 1e-5 && (p.z - q.z).abs() < 1e-5;
            assert!(
                (close(lo.0, hi.0) && close(lo.1, hi.1))
                    || (close(lo.0, hi.1) && close(lo.1, hi.0)),
                "quad torcido"
            );
        }
        let t = Topology::build(&m);
        assert_eq!(t.non_manifold_edge_count(), 0);
        assert_eq!(t.boundary_edge_count(), 20);
    }

    #[test]
    fn twist_rotates_the_alignment() {
        let build = |twist: i32| {
            let mut m = primitives::cylinder(1.0, 2.0, 8, 1, false);
            let (a, b) = rim_loops(&m);
            bridge_edge_loops(&mut m, &a, &b, 1, twist, BridgeInterpolation::Linear).unwrap();
            m
        };
        let base = build(0);
        let tw = build(1);
        assert_closed(&tw, Some(0));
        assert_consistent_orientation(&tw);
        assert_ne!(base.content_hash(), tw.content_hash());
        let len_of = |m: &EditMesh| -> f32 {
            // soma dos comprimentos das arestas dos 8 quads novos (diagonais torcidas são maiores)
            (m.face_count() - 8..m.face_count())
                .map(|f| {
                    let c: Vec<Vec3> = m.face_positions(f as u32);
                    c[1].distance(c[2]) + c[3].distance(c[0])
                })
                .sum()
        };
        assert!(len_of(&tw) > len_of(&base) + 0.1);
        // twist de volta ao mesmo: n voltas completas
        let full = build(8);
        assert_eq!(full.content_hash(), base.content_hash());
        assert_eq!(build(-8).content_hash(), base.content_hash());
    }

    #[test]
    fn segments_create_intermediate_rings_with_interpolated_uv() {
        let mut m = primitives::cylinder(1.0, 2.0, 8, 1, false);
        let (a, b) = rim_loops(&m);
        let verts_before = m.vert_count();
        let out = bridge_edge_loops(&mut m, &a, &b, 4, 0, BridgeInterpolation::Linear).unwrap();
        assert_eq!(out.created_faces.len(), 32);
        assert_eq!(out.created_verts.len(), 24);
        assert_eq!(m.vert_count(), verts_before + 24);
        assert_closed(&m, Some(0));
        assert_consistent_orientation(&m);
        // os anéis intermediários ficam entre y=-1 e y=1, em passos regulares
        let mut ys: Vec<f32> = out.created_verts.iter().map(|&v| m.co(v).y).collect();
        ys.sort_by(|p, q| p.partial_cmp(q).unwrap());
        ys.dedup_by(|p, q| (*p - *q).abs() < 1e-5);
        assert_eq!(ys.len(), 3);
        for (y, e) in ys.iter().zip([-0.5, 0.0, 0.5]) {
            assert!((y - e).abs() < 1e-5);
        }
        for &f in &out.created_faces {
            for c in &m.faces[f as usize].corners {
                assert!(c.uv.is_finite());
            }
        }
    }

    #[test]
    fn smooth_interpolation_bulges_along_outward_directions() {
        // dois cilindros abertos separados, alinhados no eixo Y, com aberturas voltadas uma para a outra
        let mut lower = primitives::cylinder(1.0, 1.0, 8, 1, false);
        let mut upper = primitives::cylinder(1.0, 1.0, 8, 1, false);
        lower.transform_positions(|p| p + Vec3::new(0.0, -2.0, 0.0));
        upper.transform_positions(|p| p + Vec3::new(0.0, 2.0, 0.0));
        let mut m = lower.clone();
        let voff = m.vert_count() as u32;
        m.append(&upper);
        let t = Topology::build(&m);
        let pick = |upper_half: bool, top: bool| -> EdgePath {
            let e = t
                .edges
                .iter()
                .find(|e| {
                    e.faces.len() == 1
                        && (e.verts.0 >= voff) == upper_half
                        && ((m.co(e.verts.0).y > if upper_half { 2.0 } else { -2.0 }) == top)
                })
                .unwrap()
                .verts;
            boundary_loop(&m, &t, e).unwrap()
        };
        let a = pick(false, true);
        let b = pick(true, false);
        let mut lin = m.clone();
        let mut smo = m.clone();
        let o1 = bridge_edge_loops(&mut lin, &a, &b, 4, 0, BridgeInterpolation::Linear).unwrap();
        let o2 = bridge_edge_loops(&mut smo, &a, &b, 4, 0, BridgeInterpolation::Smooth).unwrap();
        assert_eq!(o1.created_verts.len(), o2.created_verts.len());
        // linear mantém o raio 1; suave também (tangentes paralelas ao eixo) mas com y não-uniforme
        let ys_lin: Vec<f32> = o1.created_verts.iter().map(|&v| lin.co(v).y).collect();
        let ys_smo: Vec<f32> = o2.created_verts.iter().map(|&v| smo.co(v).y).collect();
        assert!(
            ys_lin
                .iter()
                .zip(&ys_smo)
                .any(|(p, q)| (p - q).abs() > 1e-4)
                || ys_lin == ys_smo
        );
        for v in &smo.verts {
            assert!(v.co.is_finite());
        }
        smo.validate().unwrap();
        assert_consistent_orientation(&smo);
    }

    #[test]
    fn open_loops_make_a_strip_and_reverse_properly() {
        let mut m = EditMesh::new("strip");
        let a: Vec<u32> = (0..4)
            .map(|i| m.add_vert(Vec3::new(i as f32, 0.0, 0.0)))
            .collect();
        let b: Vec<u32> = (0..4)
            .map(|i| m.add_vert(Vec3::new(3.0 - i as f32, 1.0, 0.0)))
            .collect();
        let out = bridge_edge_loops(
            &mut m,
            &EdgePath::from_verts(a, false),
            &EdgePath::from_verts(b, false),
            2,
            0,
            BridgeInterpolation::Linear,
        )
        .unwrap();
        assert_eq!(out.created_faces.len(), 6);
        // segmentos paralelos: nenhum quad degenerado nem cruzado
        for &f in &out.created_faces {
            assert!(m.face_area(f) > 0.1);
        }
        let t = Topology::build(&m);
        assert_eq!(t.non_manifold_edge_count(), 0);
        assert_consistent_orientation(&m);
    }

    #[test]
    fn invalid_inputs_leave_mesh_untouched() {
        let mut m = primitives::cylinder(1.0, 2.0, 8, 1, false);
        let (a, b) = rim_loops(&m);
        let h = m.content_hash();
        let short = EdgePath::from_verts(b.verts[..5].to_vec(), false);
        let call = |m: &mut EditMesh, a: &EdgePath, b: &EdgePath, s: u32, tw: i32| {
            bridge_edge_loops(m, a, b, s, tw, BridgeInterpolation::Linear)
        };
        assert!(matches!(
            call(&mut m, &a, &short, 1, 0),
            Err(ModelError::InvalidParameter(_))
        ));
        let open_b = EdgePath::from_verts(b.verts.clone(), false);
        assert!(matches!(
            call(&mut m, &a, &open_b, 1, 0),
            Err(ModelError::InvalidParameter(_))
        ));
        assert!(matches!(
            call(&mut m, &a, &b, 0, 0),
            Err(ModelError::InvalidParameter(_))
        ));
        assert!(matches!(
            call(&mut m, &a, &b, 1000, 0),
            Err(ModelError::InvalidParameter(_))
        ));
        let open_a = EdgePath::from_verts(a.verts.clone(), false);
        assert!(matches!(
            call(&mut m, &open_a, &open_b, 1, 2),
            Err(ModelError::InvalidParameter(_))
        ));
        assert!(matches!(
            call(&mut m, &a, &a, 1, 0),
            Err(ModelError::InvalidParameter(_))
        ));
        let bad = EdgePath::from_verts(vec![100, 101, 102, 103, 104, 105, 106, 107], true);
        assert!(matches!(
            call(&mut m, &a, &bad, 1, 0),
            Err(ModelError::VertexOutOfRange(_, _))
        ));
        let tiny = EdgePath::from_verts(vec![0, 1], true);
        assert!(matches!(
            call(&mut m, &tiny, &tiny, 1, 0),
            Err(ModelError::InvalidParameter(_))
        ));
        assert_eq!(h, m.content_hash());
        let _ = edge_key(0, 1);
    }

    #[test]
    fn large_loops_are_aligned_quickly_and_deterministically() {
        let build = || {
            let mut m = primitives::cylinder(1.0, 2.0, 2000, 1, false);
            let (a, b) = rim_loops(&m);
            let t0 = std::time::Instant::now();
            bridge_edge_loops(&mut m, &a, &b, 2, 0, BridgeInterpolation::Smooth).unwrap();
            assert!(t0.elapsed().as_secs() < 20);
            m
        };
        let x = build();
        assert_closed(&x, Some(0));
        assert_eq!(x.content_hash(), build().content_hash());
    }
}
