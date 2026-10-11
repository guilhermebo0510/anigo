//! Spin / Screw (torno): gera uma superfície de revolução a partir de um perfil.
//!
//! O perfil vem da seleção: as **arestas** selecionadas (ou, sem arestas, as
//! arestas existentes entre os **vértices** selecionados; sem nenhuma aresta,
//! cada vértice vira um anel de arestas soltas). Se a seleção tem **faces**,
//! o perfil é o contorno da região (arestas usadas por uma só face
//! selecionada). Vértices sobre o eixo não são duplicados (viram polos) e os
//! quads degenerados nesses pontos viram triângulos.
//!
//! O enrolamento é decidido por cadeia de arestas: cada cadeia é percorrida de
//! forma consistente e, no fim, orientada de modo que a **maior área** aponte
//! para longe do eixo (voto ponderado). Perfis simétricos ao eixo ou planos
//! perpendiculares a ele mantêm a orientação de percurso; use `angle` negativo
//! para inverter.

use super::extrude::{prune_orphan_edge_attrs, wire_edge_keys};
use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, EdgeAttr, EdgeKey, EditMesh, Face};
use crate::ops::OpOutput;
use crate::select::Selection;
use crate::topology::Topology;
use glam::{Quat, Vec2, Vec3};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::f32::consts::TAU;

const MAX_NEW_VERTS: usize = 20_000_000;

struct Chain {
    /// arestas orientadas (p -> q), em ordem de percurso
    edges: Vec<(u32, u32)>,
}

/// Decompõe as arestas em cadeias maximais de vértices de grau 2.
fn build_chains(edges: &BTreeSet<EdgeKey>) -> Vec<Chain> {
    let mut adj: BTreeMap<u32, Vec<EdgeKey>> = BTreeMap::new();
    for &e in edges {
        adj.entry(e.0).or_default().push(e);
        adj.entry(e.1).or_default().push(e);
    }
    let mut used: BTreeSet<EdgeKey> = BTreeSet::new();
    let mut chains = Vec::new();
    let other = |e: EdgeKey, v: u32| if e.0 == v { e.1 } else { e.0 };
    // começa por extremidades (grau != 2) para cadeias abertas; depois ciclos
    let mut starts: Vec<u32> = adj.iter().filter(|(_, l)| l.len() != 2).map(|(&v, _)| v).collect();
    starts.extend(adj.keys().copied());
    for s in starts {
        for &e0 in &adj[&s] {
            if used.contains(&e0) {
                continue;
            }
            let mut chain = Vec::new();
            let mut cur = s;
            let mut e = e0;
            loop {
                used.insert(e);
                let nx = other(e, cur);
                chain.push((cur, nx));
                cur = nx;
                let l = &adj[&cur];
                if l.len() != 2 {
                    break;
                }
                let next = if l[0] == e { l[1] } else { l[0] };
                if used.contains(&next) {
                    break;
                }
                e = next;
            }
            chains.push(Chain { edges: chain });
        }
    }
    chains
}

#[allow(clippy::too_many_arguments)]
fn spin_impl(
    mesh: &mut EditMesh,
    sel: &Selection,
    center: Vec3,
    axis: Vec3,
    angle: f32,
    steps: u32,
    merge_duplicates: bool,
    offset_per_turn: f32,
) -> ModelResult<OpOutput> {
    if !center.is_finite() || !axis.is_finite() {
        return Err(ModelError::InvalidParameter("centro/eixo não finitos".into()));
    }
    let axis_n = axis
        .try_normalize()
        .ok_or_else(|| ModelError::InvalidParameter("eixo de rotação nulo".into()))?;
    if !angle.is_finite() || angle == 0.0 || angle.abs() > TAU * 64.0 {
        return Err(ModelError::InvalidParameter(format!(
            "ângulo deve ser finito, não nulo e |ângulo| <= 64 voltas (recebeu {angle})"
        )));
    }
    if !offset_per_turn.is_finite() {
        return Err(ModelError::InvalidParameter("deslocamento por volta não finito".into()));
    }
    if steps == 0 || steps > 100_000 {
        return Err(ModelError::InvalidParameter(format!(
            "steps deve estar em 1..=100000 (recebeu {steps})"
        )));
    }
    for &v in &sel.verts {
        if v as usize >= mesh.verts.len() {
            return Err(ModelError::VertexOutOfRange(v, mesh.verts.len()));
        }
    }
    for &f in &sel.faces {
        if f as usize >= mesh.faces.len() {
            return Err(ModelError::FaceOutOfRange(f, mesh.faces.len()));
        }
    }
    let topo = Topology::build(mesh);
    for &(a, b) in &sel.edges {
        for v in [a, b] {
            if v as usize >= mesh.verts.len() {
                return Err(ModelError::VertexOutOfRange(v, mesh.verts.len()));
            }
        }
        if topo.edge(a, b).is_none() {
            return Err(ModelError::MissingEdge(a, b));
        }
    }
    // ---- perfil
    let mut profile_edges: BTreeSet<EdgeKey> = BTreeSet::new();
    if !sel.faces.is_empty() {
        let mut count: HashMap<EdgeKey, u32> = HashMap::new();
        for &f in &sel.faces {
            for (a, b) in mesh.faces[f as usize].edges() {
                *count.entry(edge_key(a, b)).or_insert(0) += 1;
            }
        }
        profile_edges.extend(count.into_iter().filter(|(_, c)| *c == 1).map(|(k, _)| k));
    } else if !sel.edges.is_empty() {
        profile_edges.extend(sel.edges.iter().map(|&(a, b)| edge_key(a, b)));
    } else {
        for e in &topo.edges {
            if sel.verts.contains(&e.verts.0) && sel.verts.contains(&e.verts.1) {
                profile_edges.insert(e.verts);
            }
        }
    }
    let mut profile_verts: BTreeSet<u32> = BTreeSet::new();
    for &(a, b) in &profile_edges {
        profile_verts.insert(a);
        profile_verts.insert(b);
    }
    if profile_edges.is_empty() {
        profile_verts.extend(sel.verts.iter().copied());
    }
    if profile_verts.is_empty() {
        return Err(ModelError::EmptySelection(
            "spin precisa de arestas ou vértices selecionados",
        ));
    }
    let pv: Vec<u32> = profile_verts.iter().copied().collect();
    let pos: Vec<Vec3> = mesh.verts.iter().map(|v| v.co).collect();
    let closed_ring =
        merge_duplicates && offset_per_turn == 0.0 && angle.abs() >= TAU - 1e-4;
    let copies = if closed_ring { steps as usize } else { steps as usize + 1 };
    if pv.len().saturating_mul(copies) > MAX_NEW_VERTS {
        return Err(ModelError::InvalidParameter(format!(
            "spin geraria {} vértices (limite {MAX_NEW_VERTS})",
            pv.len() * copies
        )));
    }

    let radial = |p: Vec3| -> Vec3 {
        let d = p - center;
        d - axis_n * d.dot(axis_n)
    };
    let max_r = pv.iter().map(|&v| radial(pos[v as usize]).length()).fold(0.0f32, f32::max);
    let axis_eps = 1e-5 * max_r.max(1.0);
    let on_axis: HashMap<u32, bool> = pv
        .iter()
        .map(|&v| (v, offset_per_turn == 0.0 && radial(pos[v as usize]).length() <= axis_eps))
        .collect();

    // transformação da cópia k
    let xform = |p: Vec3, k: usize| -> Vec3 {
        let a = angle * k as f32 / steps as f32;
        let q = Quat::from_axis_angle(axis_n, a);
        center + q * (p - center) + axis_n * (offset_per_turn * a / TAU)
    };

    // ---- orientação das cadeias (voto ponderado por área)
    let chains = build_chains(&profile_edges);
    let step_angle = angle / steps as f32;
    let mut oriented: Vec<(Chain, Vec<f32>)> = Vec::new(); // cadeia + parâmetro v acumulado
    for mut ch in chains {
        let mut vote = 0.0f32;
        for &(p, q) in &ch.edges {
            let (pp, qq) = (pos[p as usize], pos[q as usize]);
            let p1 = center
                + Quat::from_axis_angle(axis_n, step_angle) * (pp - center)
                + axis_n * (offset_per_turn * step_angle / TAU);
            let n = (qq - pp).cross(p1 - pp);
            let r = radial((pp + qq) * 0.5);
            if r.length() > axis_eps {
                vote += n.dot(r / r.length());
            }
        }
        if vote < 0.0 {
            ch.edges.reverse();
            for e in &mut ch.edges {
                *e = (e.1, e.0);
            }
        }
        // parâmetro v por comprimento acumulado
        let mut acc = vec![0.0f32];
        let mut total = 0.0f32;
        for &(p, q) in &ch.edges {
            total += pos[p as usize].distance(pos[q as usize]);
            acc.push(total);
        }
        let inv = if total > 1e-12 { 1.0 / total } else { 0.0 };
        for a in &mut acc {
            *a *= inv;
        }
        oriented.push((ch, acc));
    }

    // ---- mutação
    let mut wires = wire_edge_keys(mesh);
    let mut vmap: Vec<HashMap<u32, u32>> = Vec::with_capacity(copies);
    vmap.push(pv.iter().map(|&v| (v, v)).collect());
    let mut created_verts = Vec::new();
    for k in 1..copies {
        let mut m = HashMap::with_capacity(pv.len());
        for &v in &pv {
            if on_axis[&v] {
                m.insert(v, v);
            } else {
                let mut ev = mesh.verts[v as usize].clone();
                ev.co = xform(pos[v as usize], k);
                let id = mesh.push_vert(ev);
                created_verts.push(id);
                m.insert(v, id);
            }
        }
        vmap.push(m);
    }
    let lookup = |k: usize, v: u32| -> u32 {
        // para anel fechado, a cópia `steps` é a cópia 0
        let kk = if closed_ring { k % copies } else { k };
        vmap[kk][&v]
    };
    let mut created_faces = Vec::new();
    let mut attr_new: Vec<(EdgeKey, EdgeAttr)> = Vec::new();
    for (ch, acc) in &oriented {
        for (ei, &(p, q)) in ch.edges.iter().enumerate() {
            let template: Option<Face> = topo
                .edge_faces(p, q)
                .first()
                .map(|&f| mesh.faces[f as usize].clone());
            let attr = mesh.edge_attr(p, q);
            let (vp, vq) = (acc[ei], acc[ei + 1]);
            for k in 0..steps as usize {
                let (u0, u1) = (k as f32 / steps as f32, (k + 1) as f32 / steps as f32);
                let ring = [lookup(k, p), lookup(k, q), lookup(k + 1, q), lookup(k + 1, p)];
                let uvs = [
                    Vec2::new(u0, vp),
                    Vec2::new(u0, vq),
                    Vec2::new(u1, vq),
                    Vec2::new(u1, vp),
                ];
                // remove duplicatas consecutivas mantendo o UV do primeiro
                let mut verts: Vec<u32> = Vec::with_capacity(4);
                let mut vuv: Vec<Vec2> = Vec::with_capacity(4);
                for (i, &v) in ring.iter().enumerate() {
                    if verts.last() != Some(&v) {
                        verts.push(v);
                        vuv.push(uvs[i]);
                    }
                }
                while verts.len() > 1 && verts.first() == verts.last() {
                    verts.pop();
                    vuv.pop();
                }
                if verts.len() < 3 {
                    continue;
                }
                let mut face = match &template {
                    Some(t) => t.with_verts_from(&verts),
                    None => Face::new(&verts),
                };
                for (c, uv) in face.corners.iter_mut().zip(vuv) {
                    c.uv = uv;
                }
                mesh.faces.push(face);
                created_faces.push((mesh.faces.len() - 1) as u32);
            }
            if !attr.is_default() {
                for c in 1..copies {
                    let (a, b) = (lookup(c, p), lookup(c, q));
                    if a != b {
                        attr_new.push((edge_key(a, b), attr));
                    }
                }
            }
        }
    }
    // vértices soltos (sem arestas): anéis de arestas soltas
    if profile_edges.is_empty() {
        for &v in &pv {
            if on_axis[&v] {
                continue;
            }
            for k in 0..steps as usize {
                let (a, b) = (lookup(k, v), lookup(k + 1, v));
                if a != b {
                    mesh.edge_attrs.entry(edge_key(a, b)).or_default();
                    wires.insert(edge_key(a, b));
                }
            }
        }
    }
    for (k, a) in attr_new {
        mesh.edge_attrs.insert(k, a);
    }
    prune_orphan_edge_attrs(mesh, &wires);

    // ---- seleção devolvida
    let selection = if closed_ring && !created_faces.is_empty() {
        Selection::from_faces(mesh, created_faces.iter().copied())
    } else {
        let last = copies - 1;
        let mut s = Selection::from_edges(
            profile_edges
                .iter()
                .map(|&(a, b)| edge_key(lookup(last, a), lookup(last, b)))
                .filter(|&(a, b)| a != b),
        );
        for &v in &pv {
            s.verts.insert(lookup(last, v));
        }
        s
    };
    Ok(OpOutput {
        selection,
        created_verts,
        created_faces,
        remap: None,
    })
}

/// Spin (torno) do perfil selecionado em torno do eixo `(center, axis)`.
///
/// * `selection`: arestas/vértices do perfil (faces → contorno da região).
/// * `angle`: ângulo total em radianos (sinal = sentido, regra da mão direita
///   em torno de `axis`); `|angle| <= 64 voltas`.
/// * `steps`: divisões do ângulo total (>= 1).
/// * `merge_duplicates`: se `|angle| = 2π`, solda a última cópia na primeira
///   (anel fechado); se `false`, mantém a cópia final sobreposta à inicial
///   (útil para costurar UV/ângulo depois).
///
/// Para cada aresta do perfil gera `steps` quads (triângulos junto ao eixo).
/// UVs: `u` = fração do ângulo, `v` = comprimento acumulado ao longo da cadeia
/// do perfil. Material/suavidade vêm da face que já usava a aresta, se houver;
/// seam/sharp/crease da aresta de perfil são copiados para suas cópias.
///
/// Seleção devolvida: se o anel foi fechado, as faces criadas; caso contrário a
/// última cópia do perfil (para continuar o trabalho).
pub fn spin(
    mesh: &mut EditMesh,
    selection: &Selection,
    center: Vec3,
    axis: Vec3,
    angle: f32,
    steps: u32,
    merge_duplicates: bool,
) -> ModelResult<OpOutput> {
    spin_impl(mesh, selection, center, axis, angle, steps, merge_duplicates, 0.0)
}

/// Screw: como [`spin`], mas cada cópia sobe `offset_per_turn` por volta
/// completa ao longo do eixo (hélice/mola/chifre espiral). Nunca fecha o anel.
pub fn screw(
    mesh: &mut EditMesh,
    selection: &Selection,
    center: Vec3,
    axis: Vec3,
    angle: f32,
    steps: u32,
    offset_per_turn: f32,
) -> ModelResult<OpOutput> {
    spin_impl(mesh, selection, center, axis, angle, steps, false, offset_per_turn)
}

// -------------------------------------------------------------------- testes

#[cfg(test)]
mod tests {
    use super::super::bridge::{bridge_edge_loops, BridgeInterpolation};
    use super::super::extrude::test_util::*;
    use super::super::loops::boundary_loop;
    use super::*;
    use std::collections::HashSet;

    fn polyline(points: &[(f32, f32)]) -> (EditMesh, Selection) {
        let mut m = EditMesh::new("profile");
        let vs: Vec<u32> = points.iter().map(|&(r, y)| m.add_vert(Vec3::new(r, y, 0.0))).collect();
        for w in vs.windows(2) {
            m.edge_attrs.insert(edge_key(w[0], w[1]), EdgeAttr::default());
        }
        let sel = Selection::from_edges(m.edge_attrs.keys().copied());
        (m, sel)
    }

    fn assert_consistent_orientation(m: &EditMesh) {
        let mut seen = HashSet::new();
        for f in &m.faces {
            for (a, b) in f.edges() {
                assert!(seen.insert((a, b)), "enrolamento inconsistente em ({a},{b})");
            }
        }
    }

    #[test]
    fn vase_profile_with_poles_makes_closed_outward_solid() {
        let pts = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.5, 1.5), (0.0, 1.5)];
        let (mut m, sel) = polyline(&pts);
        let out = spin(&mut m, &sel, Vec3::ZERO, Vec3::Y, TAU, 12, true).unwrap();
        assert_closed(&m, Some(2));
        assert_consistent_orientation(&m);
        assert_outward_star(&m);
        // 4 arestas: 2 com polo (triângulos) e 2 quadriláteras = 12*4 faces
        assert_eq!(out.created_faces.len(), 48);
        assert_eq!(m.vert_count(), 2 + 3 * 12, "2 polos + 3 vértices fora do eixo por cópia");
        assert_eq!(out.selection.faces.len(), 48);
        // volume do sólido de 12 lados
        let k = 0.5 * 12.0 * (TAU / 12.0).sin();
        let integral = 1.0 + 0.5 / 3.0 * (1.0 + 0.5 + 0.25);
        assert!((signed_volume(&m) - k * integral).abs() < 1e-3, "{}", signed_volume(&m));
        // triângulos nos polos
        assert_eq!(m.faces.iter().filter(|f| f.len() == 3).count(), 24);
    }

    #[test]
    fn u_profile_spin_then_bridge_rims_gives_closed_torus() {
        let pts = [(1.0, 1.0), (1.0, 0.0), (2.0, 0.0), (2.0, 1.0)];
        let (mut m, sel) = polyline(&pts);
        spin(&mut m, &sel, Vec3::ZERO, Vec3::Y, TAU, 16, true).unwrap();
        let t = Topology::build(&m);
        assert_eq!(t.boundary_edge_count(), 32);
        assert_eq!(t.non_manifold_edge_count(), 0);
        assert_eq!(t.euler_characteristic(&m), 0);
        assert_consistent_orientation(&m);
        // normais para fora do sólido: a parede externa aponta para +radial
        let outer = (0..m.face_count() as u32)
            .find(|&f| m.face_center(f).x.abs().max(m.face_center(f).z.abs()) > 1.7 && m.face_center(f).y > 0.1 && m.face_center(f).y < 0.9)
            .unwrap();
        let c = m.face_center(outer);
        assert!(m.face_normal(outer).dot(Vec3::new(c.x, 0.0, c.z)) > 0.0);
        let inner = (0..m.face_count() as u32)
            .find(|&f| m.face_center(f).x.abs().max(m.face_center(f).z.abs()) < 1.2 && m.face_center(f).y > 0.1 && m.face_center(f).y < 0.9)
            .unwrap();
        let c = m.face_center(inner);
        assert!(m.face_normal(inner).dot(Vec3::new(c.x, 0.0, c.z)) < 0.0);
        // fecha a borda: ponte entre os dois anéis do topo
        let topo = Topology::build(&m);
        let mut loops = Vec::new();
        let mut cover: HashSet<u32> = HashSet::new();
        for e in &topo.edges {
            if e.faces.len() == 1 && !cover.contains(&e.verts.0) {
                let lp = boundary_loop(&m, &topo, e.verts).unwrap();
                cover.extend(lp.verts.iter().copied());
                loops.push(lp);
            }
        }
        assert_eq!(loops.len(), 2);
        bridge_edge_loops(&mut m, &loops[0], &loops[1], 1, 0, BridgeInterpolation::Linear).unwrap();
        assert_closed(&m, Some(0));
        assert_consistent_orientation(&m);
        // volume do "pote": anel de seção 1x1 (paredes) fechado pelo topo: > 0
        assert!(signed_volume(&m) > 0.0);
    }

    #[test]
    fn partial_spin_is_open_and_selects_last_copy() {
        let (mut m, sel) = polyline(&[(1.0, 0.0), (1.0, 1.0)]);
        let out = spin(&mut m, &sel, Vec3::ZERO, Vec3::Y, TAU / 4.0, 4, true).unwrap();
        assert_eq!(out.created_faces.len(), 4);
        assert_eq!(out.created_verts.len(), 8);
        let t = Topology::build(&m);
        // 8 degraus (2 vértices x 4) + as duas arestas de perfil das pontas
        assert_eq!(t.boundary_edge_count(), 10);
        assert_eq!(t.non_manifold_edge_count(), 0);
        // última cópia a 90 graus: (1,y,0) -> (0,y,-1)
        for &v in &out.selection.verts {
            assert!((m.co(v) - Vec3::new(0.0, m.co(v).y, -1.0)).length() < 1e-5);
        }
        assert_eq!(out.selection.edges.len(), 1);
        // a normal da parede aponta para longe do eixo
        for &f in &out.created_faces {
            let c = m.face_center(f);
            assert!(m.face_normal(f).dot(Vec3::new(c.x, 0.0, c.z)) > 0.0);
        }
    }

    #[test]
    fn negative_angle_still_orients_outward() {
        let (mut m, sel) = polyline(&[(1.0, 0.0), (1.0, 1.0)]);
        let out = spin(&mut m, &sel, Vec3::ZERO, Vec3::Y, -TAU / 4.0, 4, true).unwrap();
        for &f in &out.created_faces {
            let c = m.face_center(f);
            assert!(m.face_normal(f).dot(Vec3::new(c.x, 0.0, c.z)) > 0.0);
        }
        for &v in &out.selection.verts {
            assert!((m.co(v) - Vec3::new(0.0, m.co(v).y, 1.0)).length() < 1e-5);
        }
    }

    #[test]
    fn full_turn_without_merge_keeps_overlapping_copy() {
        let (mut m, sel) = polyline(&[(1.0, 0.0), (1.0, 1.0)]);
        let out = spin(&mut m, &sel, Vec3::ZERO, Vec3::Y, TAU, 8, false).unwrap();
        assert_eq!(out.created_verts.len(), 16, "8 cópias novas de 2 vértices");
        assert_eq!(out.created_faces.len(), 8);
        let t = Topology::build(&m);
        assert_eq!(t.boundary_edge_count(), 18);
        // com merge, fecha
        let (mut m2, sel2) = polyline(&[(1.0, 0.0), (1.0, 1.0)]);
        spin(&mut m2, &sel2, Vec3::ZERO, Vec3::Y, TAU, 8, true).unwrap();
        let t2 = Topology::build(&m2);
        assert_eq!(t2.boundary_edge_count(), 16, "tubo aberto nas duas pontas");
        assert_eq!(m2.vert_count(), 16);
        assert_eq!(t2.euler_characteristic(&m2), 0);
    }

    #[test]
    fn screw_makes_a_helix_ramp() {
        let (mut m, sel) = polyline(&[(1.0, 0.0), (1.5, 0.0)]);
        let out = screw(&mut m, &sel, Vec3::ZERO, Vec3::Y, TAU * 3.0, 36, 0.5).unwrap();
        assert_eq!(out.created_faces.len(), 36);
        let top = out.selection.verts.iter().map(|&v| m.co(v).y).fold(f32::MIN, f32::max);
        assert!((top - 1.5).abs() < 1e-4, "3 voltas * 0.5 = 1.5, obteve {top}");
        let t = Topology::build(&m);
        assert_eq!(t.non_manifold_edge_count(), 0);
        assert_consistent_orientation(&m);
        // nunca fecha o anel
        let (mut m2, sel2) = polyline(&[(1.0, 0.0), (1.5, 0.0)]);
        let o2 = screw(&mut m2, &sel2, Vec3::ZERO, Vec3::Y, TAU, 12, 0.5).unwrap();
        assert_eq!(o2.created_verts.len(), 24);
    }

    #[test]
    fn cycle_profile_makes_closed_torus() {
        let mut m = EditMesh::new("sq");
        let pts = [(2.0, -0.5), (3.0, -0.5), (3.0, 0.5), (2.0, 0.5)];
        let vs: Vec<u32> = pts.iter().map(|&(r, y)| m.add_vert(Vec3::new(r, y, 0.0))).collect();
        for i in 0..4 {
            m.edge_attrs.insert(edge_key(vs[i], vs[(i + 1) % 4]), EdgeAttr::default());
        }
        let sel = Selection::from_edges(m.edge_attrs.keys().copied());
        spin(&mut m, &sel, Vec3::ZERO, Vec3::Y, TAU, 12, true).unwrap();
        assert_closed(&m, Some(0));
        assert_consistent_orientation(&m);
        assert!(signed_volume(&m) > 0.0, "normais para fora");
    }

    #[test]
    fn vertices_only_make_wire_rings() {
        let mut m = EditMesh::new("v");
        let v = m.add_vert(Vec3::new(1.0, 0.0, 0.0));
        let axis_v = m.add_vert(Vec3::new(0.0, 1.0, 0.0));
        let out = spin(&mut m, &Selection::from_verts([v, axis_v]), Vec3::ZERO, Vec3::Y, TAU, 6, true).unwrap();
        assert!(out.created_faces.is_empty());
        assert_eq!(out.created_verts.len(), 5, "o vértice no eixo não é duplicado");
        let t = Topology::build(&m);
        assert_eq!(t.edges.len(), 6, "anel de 6 arestas soltas");
        assert!(t.edges.iter().all(|e| e.faces.is_empty()));
        m.validate().unwrap();
    }

    #[test]
    fn face_selection_spins_its_outline() {
        let mut m = EditMesh::new("quad");
        let pts = [(2.0, 0.0), (3.0, 0.0), (3.0, 1.0), (2.0, 1.0)];
        let vs: Vec<u32> = pts.iter().map(|&(r, y)| m.add_vert(Vec3::new(r, y, 0.0))).collect();
        m.add_face(&vs).unwrap();
        let sel = Selection::from_faces(&m, [0u32]);
        let n0 = m.face_count();
        let out = spin(&mut m, &sel, Vec3::ZERO, Vec3::Y, TAU, 8, true).unwrap();
        assert_eq!(out.created_faces.len(), 32);
        assert_eq!(m.face_count(), n0 + 32);
        m.validate().unwrap();
    }

    #[test]
    fn edge_attrs_travel_to_copies() {
        let (mut m, sel) = polyline(&[(1.0, 0.0), (1.0, 1.0)]);
        m.set_seam(0, 1, true);
        let out = spin(&mut m, &sel, Vec3::ZERO, Vec3::Y, TAU / 2.0, 4, true).unwrap();
        let last = out.selection.edges.iter().next().copied().unwrap();
        assert!(m.edge_attr(last.0, last.1).seam);
        assert!(m.edge_attr(0, 1).seam);
        // uv dentro de [0,1]
        for f in &m.faces {
            for c in &f.corners {
                assert!(c.uv.x >= -1e-6 && c.uv.x <= 1.0 + 1e-6 && c.uv.y >= -1e-6 && c.uv.y <= 1.0 + 1e-6);
            }
        }
    }

    #[test]
    fn invalid_inputs_leave_mesh_untouched() {
        let (mut m, sel) = polyline(&[(1.0, 0.0), (1.0, 1.0)]);
        let h = m.content_hash();
        let ok = |m: &mut EditMesh, s: &Selection, c: Vec3, a: Vec3, ang: f32, st: u32| spin(m, s, c, a, ang, st, true);
        assert!(matches!(ok(&mut m, &sel, Vec3::ZERO, Vec3::ZERO, 1.0, 4), Err(ModelError::InvalidParameter(_))));
        assert!(matches!(ok(&mut m, &sel, Vec3::NAN, Vec3::Y, 1.0, 4), Err(ModelError::InvalidParameter(_))));
        assert!(matches!(ok(&mut m, &sel, Vec3::ZERO, Vec3::Y, 0.0, 4), Err(ModelError::InvalidParameter(_))));
        assert!(matches!(ok(&mut m, &sel, Vec3::ZERO, Vec3::Y, f32::NAN, 4), Err(ModelError::InvalidParameter(_))));
        assert!(matches!(ok(&mut m, &sel, Vec3::ZERO, Vec3::Y, 1000.0, 4), Err(ModelError::InvalidParameter(_))));
        assert!(matches!(ok(&mut m, &sel, Vec3::ZERO, Vec3::Y, 1.0, 0), Err(ModelError::InvalidParameter(_))));
        assert!(matches!(ok(&mut m, &Selection::default(), Vec3::ZERO, Vec3::Y, 1.0, 4), Err(ModelError::EmptySelection(_))));
        assert!(matches!(
            ok(&mut m, &Selection::from_verts([9]), Vec3::ZERO, Vec3::Y, 1.0, 4),
            Err(ModelError::VertexOutOfRange(9, 2))
        ));
        let mut bad = sel.clone();
        bad.edges.insert((0, 5));
        assert!(matches!(ok(&mut m, &bad, Vec3::ZERO, Vec3::Y, 1.0, 4), Err(ModelError::VertexOutOfRange(5, 2))));
        let mut m3 = m.clone();
        m3.add_vert(Vec3::ONE);
        m3.add_vert(Vec3::ONE * 2.0);
        let mut missing = Selection::default();
        missing.edges.insert((0, 3));
        let h3 = m3.content_hash();
        assert!(matches!(ok(&mut m3, &missing, Vec3::ZERO, Vec3::Y, 1.0, 4), Err(ModelError::MissingEdge(0, 3))));
        assert_eq!(h3, m3.content_hash());
        let mut f = sel.clone();
        f.faces.insert(4);
        assert!(matches!(ok(&mut m, &f, Vec3::ZERO, Vec3::Y, 1.0, 4), Err(ModelError::FaceOutOfRange(4, 0))));
        assert!(matches!(
            screw(&mut m, &sel, Vec3::ZERO, Vec3::Y, 1.0, 4, f32::INFINITY),
            Err(ModelError::InvalidParameter(_))
        ));
        assert_eq!(h, m.content_hash());
    }

    #[test]
    fn spin_is_deterministic_and_fast_with_dense_profile() {
        let run = || {
            let pts: Vec<(f32, f32)> = (0..200)
                .map(|i| {
                    let t = i as f32 / 199.0;
                    (0.2 + (t * 3.0).sin().abs() * 0.5, t * 2.0)
                })
                .collect();
            let (mut m, sel) = polyline(&pts);
            let t0 = std::time::Instant::now();
            spin(&mut m, &sel, Vec3::ZERO, Vec3::Y, TAU, 256, true).unwrap();
            assert!(t0.elapsed().as_secs() < 30);
            m
        };
        let a = run();
        assert_eq!(a.face_count(), 199 * 256);
        a.validate().unwrap();
        assert_eq!(a.content_hash(), run().content_hash());
    }
}
