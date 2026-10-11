//! Seleção de loops/rings e Loop Cut & Slide.
//!
//! * [`edge_loop`] — Alt+clique: segue por vértices de valência 4 (e por
//!   bordas), parando em polos (valência ≠ 4) e na fronteira.
//! * [`edge_ring`] / [`face_loop`] — arestas paralelas atravessando quads.
//! * [`boundary_loop`] — cadeia de arestas de fronteira.
//! * [`loop_cut`] — `Ctrl+R`: divide todos os quads do ring em `cuts + 1` fatias.

use super::extrude::{lerp_corner, prune_orphan_edge_attrs, validated_verts, wire_edge_keys};
use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, EVert, EdgeAttr, EdgeKey, EditMesh, Face};
use crate::ops::OpOutput;
use crate::select::Selection;
use crate::topology::Topology;
use std::collections::{HashMap, HashSet};

/// Caminho de arestas ordenado (edge loop ou boundary loop).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EdgePath {
    /// Vértices na ordem do caminho. Se `closed`, o último liga ao primeiro e
    /// `verts.len() == edges.len()`; senão `verts.len() == edges.len() + 1`.
    pub verts: Vec<u32>,
    /// `edges[i]` liga `verts[i]` a `verts[i + 1]` (módulo `len` se fechado).
    pub edges: Vec<EdgeKey>,
    pub closed: bool,
}

impl EdgePath {
    /// Monta um caminho a partir de vértices ordenados (útil para o bridge).
    pub fn from_verts(verts: Vec<u32>, closed: bool) -> EdgePath {
        let n = verts.len();
        let count = if closed { n } else { n.saturating_sub(1) };
        let edges = (0..count)
            .map(|i| edge_key(verts[i], verts[(i + 1) % n]))
            .collect();
        EdgePath {
            verts,
            edges,
            closed,
        }
    }

    /// Seleção equivalente (arestas e vértices).
    pub fn to_selection(&self) -> Selection {
        Selection::from_edges(self.edges.iter().copied())
    }
}

/// Edge ring: arestas paralelas e as faces (quads) entre elas.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EdgeRing {
    /// Arestas na ordem do ring.
    pub edges: Vec<EdgeKey>,
    /// `faces[i]` fica entre `edges[i]` e `edges[i + 1]` (a última, entre a
    /// última e a primeira, quando `closed`).
    pub faces: Vec<u32>,
    pub closed: bool,
}

fn check_edge(topo: &Topology, e: EdgeKey) -> ModelResult<()> {
    if topo.edge(e.0, e.1).is_none() {
        return Err(ModelError::MissingEdge(e.0, e.1));
    }
    Ok(())
}

/// Próximo vértice do loop ao chegar em `cur` vindo de `prev`.
fn next_loop_vert(topo: &Topology, prev: u32, cur: u32) -> Option<u32> {
    let e = topo.edge(prev, cur)?;
    let incident = &topo.vert_edges[cur as usize];
    match e.faces.len() {
        1 => {
            // segue a borda
            let cand: Vec<u32> = incident
                .iter()
                .copied()
                .filter(|&ei| {
                    let info = &topo.edges[ei as usize];
                    info.faces.len() == 1 && edge_key(prev, cur) != info.verts
                })
                .collect();
            if cand.len() == 1 {
                let (a, b) = topo.edges[cand[0] as usize].verts;
                Some(if a == cur { b } else { a })
            } else {
                None
            }
        }
        2 => {
            if incident.len() != 4
                || incident
                    .iter()
                    .any(|&ei| topo.edges[ei as usize].faces.len() != 2)
            {
                return None;
            }
            let cand: Vec<u32> = incident
                .iter()
                .copied()
                .filter(|&ei| {
                    let info = &topo.edges[ei as usize];
                    info.verts != edge_key(prev, cur)
                        && info.faces.iter().all(|f| !e.faces.contains(f))
                })
                .collect();
            if cand.len() == 1 {
                let (a, b) = topo.edges[cand[0] as usize].verts;
                Some(if a == cur { b } else { a })
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Segue o loop a partir de `prev -> cur`. Devolve os vértices seguintes e se
/// fechou de volta em `stop`.
fn walk_loop(topo: &Topology, stop: u32, mut prev: u32, mut cur: u32, limit: usize) -> (Vec<u32>, bool) {
    let mut out = vec![cur];
    for _ in 0..limit {
        if cur == stop {
            out.pop();
            return (out, true);
        }
        match next_loop_vert(topo, prev, cur) {
            Some(nx) => {
                prev = cur;
                cur = nx;
                out.push(cur);
            }
            None => return (out, false),
        }
    }
    (out, false)
}

/// Edge loop a partir de `start_edge` (equivalente a Alt+clique numa aresta).
///
/// Regras: ao passar por um vértice de valência 4 (interior, todas as arestas
/// manifold) segue pela aresta oposta; em bordas segue a cadeia de fronteira;
/// em qualquer outro vértice (polos de valência 3/5+, junções, vértices de
/// borda com aresta interna) o loop termina ali. Arestas soltas ou
/// não-manifold devolvem um caminho de uma única aresta.
///
/// `topo` deve ter sido construída a partir de `mesh` (parâmetro mantido para
/// reaproveitar a topologia entre chamadas). Falha com `MissingEdge` se a
/// aresta não existe.
pub fn edge_loop(mesh: &EditMesh, topo: &Topology, start_edge: EdgeKey) -> ModelResult<EdgePath> {
    let _ = mesh;
    let (a, b) = edge_key(start_edge.0, start_edge.1);
    check_edge(topo, (a, b))?;
    let limit = topo.edges.len() + 2;
    let (fwd, closed) = walk_loop(topo, a, a, b, limit);
    if closed {
        // fwd = b, ..., (último antes de voltar a a); path = a, b, ...
        let mut verts = vec![a];
        verts.extend(fwd);
        return Ok(EdgePath::from_verts(verts, true));
    }
    let (back, _) = walk_loop(topo, b, b, a, limit);
    // back = a, x1, x2...; caminho = rev(back[1..]) + a + fwd
    let mut verts: Vec<u32> = back.iter().skip(1).rev().copied().collect();
    verts.push(a);
    verts.extend(fwd);
    Ok(EdgePath::from_verts(verts, false))
}

/// Boundary loop: cadeia de arestas de fronteira conectada a `start_edge`.
/// Para em junções (mais de uma continuação). Falha se a aresta não é de fronteira.
pub fn boundary_loop(mesh: &EditMesh, topo: &Topology, start_edge: EdgeKey) -> ModelResult<EdgePath> {
    let _ = mesh;
    let (a, b) = edge_key(start_edge.0, start_edge.1);
    check_edge(topo, (a, b))?;
    if !topo.is_boundary_edge(a, b) {
        return Err(ModelError::InvalidParameter(format!(
            "aresta ({a}, {b}) não é de fronteira"
        )));
    }
    edge_loop(mesh, topo, (a, b))
}

/// Quad com 4 vértices distintos? Devolve os corners como vértices.
fn quad_verts(mesh: &EditMesh, f: u32) -> Option<[u32; 4]> {
    let face = mesh.faces.get(f as usize)?;
    if face.corners.len() != 4 {
        return None;
    }
    let v = [
        face.corners[0].vert,
        face.corners[1].vert,
        face.corners[2].vert,
        face.corners[3].vert,
    ];
    for i in 0..4 {
        for j in i + 1..4 {
            if v[i] == v[j] {
                return None;
            }
        }
    }
    Some(v)
}

/// Aresta oposta de `key` dentro do quad `f`.
fn opposite_in_quad(mesh: &EditMesh, f: u32, key: EdgeKey) -> Option<EdgeKey> {
    let c = quad_verts(mesh, f)?;
    for k in 0..4 {
        if edge_key(c[k], c[(k + 1) % 4]) == key {
            return Some(edge_key(c[(k + 2) % 4], c[(k + 3) % 4]));
        }
    }
    None
}

struct RingWalk {
    edges: Vec<EdgeKey>,
    faces: Vec<u32>,
    closed: bool,
}

fn walk_ring(
    mesh: &EditMesh,
    topo: &Topology,
    start: EdgeKey,
    first_face: u32,
    seen_edges: &mut HashSet<EdgeKey>,
    seen_faces: &mut HashSet<u32>,
) -> RingWalk {
    let mut w = RingWalk {
        edges: Vec::new(),
        faces: Vec::new(),
        closed: false,
    };
    let (mut e, mut f) = (start, first_face);
    loop {
        if seen_faces.contains(&f) {
            break;
        }
        let Some(opp) = opposite_in_quad(mesh, f, e) else {
            break;
        };
        if opp == start {
            w.faces.push(f);
            w.closed = true;
            break;
        }
        if seen_edges.contains(&opp) {
            break;
        }
        seen_faces.insert(f);
        seen_edges.insert(opp);
        w.faces.push(f);
        w.edges.push(opp);
        let faces = topo.edge_faces(opp.0, opp.1);
        if faces.len() != 2 {
            break;
        }
        let g = if faces[0] == f { faces[1] } else { faces[0] };
        e = opp;
        f = g;
    }
    w
}

/// Edge ring a partir de `start_edge`: atravessa quads pela aresta oposta, nos
/// dois sentidos, até fechar ou encontrar fronteira/n-gon/triângulo.
///
/// O ring sempre contém `start_edge`. Falha com `MissingEdge` se ela não existe.
pub fn edge_ring(mesh: &EditMesh, topo: &Topology, start_edge: EdgeKey) -> ModelResult<EdgeRing> {
    let start = edge_key(start_edge.0, start_edge.1);
    check_edge(topo, start)?;
    let faces = topo.edge_faces(start.0, start.1);
    let mut seen_edges = HashSet::new();
    seen_edges.insert(start);
    let mut seen_faces = HashSet::new();
    let fwd = match faces.first() {
        Some(&f) => walk_ring(mesh, topo, start, f, &mut seen_edges, &mut seen_faces),
        None => RingWalk {
            edges: vec![],
            faces: vec![],
            closed: false,
        },
    };
    if fwd.closed {
        let mut edges = vec![start];
        edges.extend(fwd.edges);
        return Ok(EdgeRing {
            edges,
            faces: fwd.faces,
            closed: true,
        });
    }
    let back = match (faces.get(1), faces.len()) {
        (Some(&f), 2) => walk_ring(mesh, topo, start, f, &mut seen_edges, &mut seen_faces),
        _ => RingWalk {
            edges: vec![],
            faces: vec![],
            closed: false,
        },
    };
    let mut edges: Vec<EdgeKey> = back.edges.iter().rev().copied().collect();
    edges.push(start);
    edges.extend(fwd.edges);
    let mut out_faces: Vec<u32> = back.faces.iter().rev().copied().collect();
    out_faces.extend(fwd.faces);
    Ok(EdgeRing {
        edges,
        faces: out_faces,
        closed: false,
    })
}

/// Face loop: as faces (quads) atravessadas pelo ring de `start_edge`, em ordem.
pub fn face_loop(mesh: &EditMesh, topo: &Topology, start_edge: EdgeKey) -> ModelResult<Vec<u32>> {
    Ok(edge_ring(mesh, topo, start_edge)?.faces)
}

// ------------------------------------------------------------------ loop cut

/// Resultado do [`loop_cut`].
#[derive(Debug, Clone)]
pub struct LoopCutOutput {
    /// Seleção = novas arestas do(s) loop(s); vértices e faces criados.
    pub op: OpOutput,
    /// Quantos quads do ring foram cortados.
    pub faces_cut: usize,
    /// O ring era fechado?
    pub closed: bool,
}

impl From<LoopCutOutput> for OpOutput {
    fn from(v: LoopCutOutput) -> OpOutput {
        v.op
    }
}

struct Split {
    x: u32,
    inner: Vec<u32>,
    ts: Vec<f32>,
}

/// Pontos (vértice, fração) ao longo de `a -> b` incluindo extremos.
fn points_along(splits: &HashMap<EdgeKey, Split>, a: u32, b: u32) -> Vec<(u32, f32)> {
    let mut out = vec![(a, 0.0)];
    if let Some(s) = splits.get(&edge_key(a, b)) {
        if s.x == a {
            out.extend(s.inner.iter().copied().zip(s.ts.iter().copied()));
        } else {
            out.extend(s.inner.iter().rev().copied().zip(s.ts.iter().rev().map(|t| 1.0 - t)));
        }
    }
    out.push((b, 1.0));
    out
}

fn combine_rung_attrs(a: EdgeAttr, b: EdgeAttr) -> EdgeAttr {
    EdgeAttr {
        sharp: a.sharp && b.sharp,
        seam: a.seam && b.seam,
        crease: (a.crease + b.crease) * 0.5,
        bevel_weight: (a.bevel_weight + b.bevel_weight) * 0.5,
    }
}

/// Loop Cut & Slide (`Ctrl+R`).
///
/// * `ring_edge`: aresta que define o ring (a ferramenta corta *perpendicular*
///   a ela: o novo loop cruza essa aresta).
/// * `cuts`: número de cortes (1 = um loop no meio).
/// * `slide_factor`: em `[-1, 1]`; desloca os cortes entre as arestas vizinhas.
///   `0` = espaçamento uniforme. Positivo move na direção do **maior índice**
///   de vértice de `ring_edge` (o sentido é propagado de forma consistente
///   por todo o ring). Fatores extremos são limitados a `[0.001, 0.999]`.
///
/// Cada quad do ring vira `cuts + 1` quads. O ring para ao encontrar fronteira
/// ou uma face que não é quad (n-gon/triângulo); essas faces de fim de ring não
/// são divididas, mas recebem os novos vértices na aresta compartilhada
/// (virando n-gons, como no Blender), o que mantém a malha estanque.
///
/// UVs e normais por corner são interpolados; sharp/seam/crease/bevel_weight
/// das arestas cortadas são copiados para os segmentos, e as arestas novas do
/// loop herdam o atributo das duas arestas "degrau" vizinhas quando concordam.
///
/// Devolve [`LoopCutOutput`] com a seleção do novo edge loop (todos os cortes).
pub fn loop_cut(
    mesh: &mut EditMesh,
    ring_edge: EdgeKey,
    cuts: u32,
    slide_factor: f32,
) -> ModelResult<LoopCutOutput> {
    if cuts == 0 || cuts > 1024 {
        return Err(ModelError::InvalidParameter(format!(
            "número de cortes deve estar em 1..=1024 (recebeu {cuts})"
        )));
    }
    if !slide_factor.is_finite() || slide_factor.abs() > 1.0 {
        return Err(ModelError::InvalidParameter(format!(
            "slide deve estar em [-1, 1] (recebeu {slide_factor})"
        )));
    }
    let start = edge_key(ring_edge.0, ring_edge.1);
    for v in [start.0, start.1] {
        if v as usize >= mesh.verts.len() {
            return Err(ModelError::VertexOutOfRange(v, mesh.verts.len()));
        }
    }
    let topo = Topology::build(mesh);
    check_edge(&topo, start)?;
    let n_start_faces = topo.edge_faces(start.0, start.1).len();
    if n_start_faces == 0 || n_start_faces > 2 {
        return Err(ModelError::UnsupportedTopology(format!(
            "aresta ({}, {}) tem {n_start_faces} faces; o ring exige uma aresta de superfície (1 ou 2 faces)",
            start.0, start.1
        )));
    }
    let ring = edge_ring(mesh, &topo, start)?;
    if ring.faces.is_empty() {
        return Err(ModelError::UnsupportedTopology(
            "nenhuma face quad adjacente à aresta: Loop Cut só atravessa quads".into(),
        ));
    }
    // ---- orientação consistente das arestas do ring
    let n = ring.edges.len();
    let pos0 = ring.edges.iter().position(|&e| e == start).unwrap_or(0);
    let mut orient: Vec<(u32, u32)> = vec![(0, 0); n];
    orient[pos0] = start;
    // face entre edges[i] e edges[i+1] é ring.faces[i]
    let pair = |mesh: &EditMesh, f: u32, from: EdgeKey, from_or: (u32, u32)| -> Option<(u32, u32)> {
        let c = quad_verts(mesh, f)?;
        for k in 0..4 {
            if edge_key(c[k], c[(k + 1) % 4]) == from {
                let forward = (c[k], c[(k + 1) % 4]) == from_or;
                return Some(if forward {
                    (c[(k + 3) % 4], c[(k + 2) % 4])
                } else {
                    (c[(k + 2) % 4], c[(k + 3) % 4])
                });
            }
        }
        None
    };
    for i in pos0..n.saturating_sub(1) {
        orient[i + 1] = pair(mesh, ring.faces[i], ring.edges[i], orient[i])
            .ok_or_else(|| ModelError::UnsupportedTopology("ring inconsistente".into()))?;
    }
    for i in (0..pos0).rev() {
        orient[i] = pair(mesh, ring.faces[i], ring.edges[i + 1], orient[i + 1])
            .ok_or_else(|| ModelError::UnsupportedTopology("ring inconsistente".into()))?;
    }
    if ring.closed {
        let back = pair(mesh, ring.faces[n - 1], ring.edges[n - 1], orient[n - 1])
            .ok_or_else(|| ModelError::UnsupportedTopology("ring inconsistente".into()))?;
        if back != orient[0] {
            return Err(ModelError::UnsupportedTopology(
                "ring fechado com orientação invertida (faixa de Möbius)".into(),
            ));
        }
    }
    for (e, o) in ring.edges.iter().zip(&orient) {
        if edge_key(o.0, o.1) != *e {
            return Err(ModelError::UnsupportedTopology("ring inconsistente".into()));
        }
    }
    // o sentido positivo do slide: do menor para o maior índice da aresta inicial
    let flip = orient[pos0] != (start.0, start.1);
    let slide = if flip { -slide_factor } else { slide_factor };

    // ---- novos vértices
    let cuts_f = cuts as f32;
    let ts: Vec<f32> = (1..=cuts)
        .map(|k| (k as f32 / (cuts_f + 1.0) + slide / (cuts_f + 1.0)).clamp(0.001, 0.999))
        .collect();
    let mut splits: HashMap<EdgeKey, Split> = HashMap::new();
    let mut created_verts = Vec::new();
    for (i, &e) in ring.edges.iter().enumerate() {
        let (x, y) = orient[i];
        let mut inner = Vec::with_capacity(cuts as usize);
        for &t in &ts {
            let v = EVert::lerp(&mesh.verts[x as usize], &mesh.verts[y as usize], t);
            inner.push(mesh.push_vert(v));
        }
        created_verts.extend(inner.iter().copied());
        splits.insert(
            e,
            Split {
                x,
                inner,
                ts: ts.clone(),
            },
        );
    }
    let wires = wire_edge_keys(mesh);

    // ---- faces do ring
    let ring_faces: HashSet<u32> = ring.faces.iter().copied().collect();
    let mut created_faces = Vec::new();
    let mut new_loop_edges: Vec<EdgeKey> = Vec::new();
    let mut attr_new: Vec<(EdgeKey, EdgeAttr)> = Vec::new();
    for (i, &f) in ring.faces.iter().enumerate() {
        let e_a = ring.edges[i];
        let e_b = ring.edges[(i + 1) % n];
        let old = mesh.faces[f as usize].clone();
        let Some(c) = quad_verts(mesh, f) else {
            continue;
        };
        // k: posição do primeiro corner da aresta e_a no quad
        let mut k_found = None;
        for k in 0..4 {
            if edge_key(c[k], c[(k + 1) % 4]) == e_a {
                k_found = Some(k);
                break;
            }
        }
        let Some(k) = k_found else {
            continue;
        };
        let (p0, p1, p2, p3) = (c[k], c[(k + 1) % 4], c[(k + 2) % 4], c[(k + 3) % 4]);
        debug_assert_eq!(edge_key(p2, p3), e_b);
        let (cr0, cr1, cr2, cr3) = (
            old.corners[k],
            old.corners[(k + 1) % 4],
            old.corners[(k + 2) % 4],
            old.corners[(k + 3) % 4],
        );
        let a_pts = points_along(&splits, p0, p1);
        let b_pts = points_along(&splits, p3, p2);
        for j in 0..=cuts as usize {
            let (a0, a1) = (a_pts[j], a_pts[j + 1]);
            let (b0, b1) = (b_pts[j], b_pts[j + 1]);
            let mut face: Face = old.with_verts_from(&[a0.0, a1.0, b1.0, b0.0]);
            face.corners[0] = lerp_corner(&cr0, &cr1, a0.1, a0.0);
            face.corners[1] = lerp_corner(&cr0, &cr1, a1.1, a1.0);
            face.corners[2] = lerp_corner(&cr3, &cr2, b1.1, b1.0);
            face.corners[3] = lerp_corner(&cr3, &cr2, b0.1, b0.0);
            if j == 0 {
                mesh.faces[f as usize] = face;
            } else {
                mesh.faces.push(face);
                created_faces.push((mesh.faces.len() - 1) as u32);
            }
            if j >= 1 {
                // aresta do loop entre a0 e b0 (cruza o quad)
                new_loop_edges.push(edge_key(a0.0, b0.0));
                let attr = combine_rung_attrs(mesh.edge_attr(p1, p2), mesh.edge_attr(p3, p0));
                if !attr.is_default() {
                    attr_new.push((edge_key(a0.0, b0.0), attr));
                }
            }
        }
    }

    // ---- demais faces que tocam arestas cortadas (fins de ring): inserem os pontos
    let mut touching: Vec<u32> = Vec::new();
    for e in &ring.edges {
        for &g in topo.edge_faces(e.0, e.1) {
            if !ring_faces.contains(&g) {
                touching.push(g);
            }
        }
    }
    touching.sort_unstable();
    touching.dedup();
    for g in touching {
        let old = mesh.faces[g as usize].clone();
        let m = old.corners.len();
        let mut corners = Vec::with_capacity(m + cuts as usize);
        for i in 0..m {
            let (ca, cb) = (old.corners[i], old.corners[(i + 1) % m]);
            corners.push(ca);
            if splits.contains_key(&edge_key(ca.vert, cb.vert)) {
                let pts = points_along(&splits, ca.vert, cb.vert);
                for &(v, t) in &pts[1..pts.len() - 1] {
                    corners.push(lerp_corner(&ca, &cb, t, v));
                }
            }
        }
        mesh.faces[g as usize].corners = corners;
    }

    // ---- atributos de aresta: segmentos herdam; arestas cortadas antigas somem
    for (&e, s) in &splits {
        let attr = mesh.edge_attr(e.0, e.1);
        if attr.is_default() {
            continue;
        }
        let other = if s.x == e.0 { e.1 } else { e.0 };
        let mut chain = vec![s.x];
        chain.extend(s.inner.iter().copied());
        chain.push(other);
        for w in chain.windows(2) {
            attr_new.push((edge_key(w[0], w[1]), attr));
        }
    }
    for (k, a) in attr_new {
        mesh.edge_attrs.insert(k, a);
    }
    prune_orphan_edge_attrs(mesh, &wires);

    new_loop_edges.sort_unstable();
    new_loop_edges.dedup();
    let selection = Selection::from_edges(new_loop_edges);
    Ok(LoopCutOutput {
        op: OpOutput {
            selection,
            created_verts,
            created_faces,
            remap: None,
        },
        faces_cut: ring.faces.len(),
        closed: ring.closed,
    })
}

/// Valida um conjunto de vértices usados como início de seleção de loops
/// (helper de conveniência para a UI): devolve os índices ordenados.
pub fn validated_loop_verts(mesh: &EditMesh, verts: &[u32]) -> ModelResult<Vec<u32>> {
    validated_verts(mesh, verts, "seleção de vértices vazia")
}

// -------------------------------------------------------------------- testes

#[cfg(test)]
mod tests {
    use super::super::extrude::test_util::*;
    use super::*;
    use crate::primitives;
    use glam::{Vec2, Vec3};

    fn some_edge_with<F: Fn(Vec3, Vec3) -> bool>(m: &EditMesh, f: F) -> EdgeKey {
        let t = Topology::build(m);
        t.edges
            .iter()
            .map(|e| e.verts)
            .find(|&(a, b)| f(m.co(a), m.co(b)))
            .unwrap()
    }

    #[test]
    fn edge_loop_closes_on_quad_sphere_and_stops_at_poles() {
        let m = primitives::quad_sphere(1.0, 4);
        let t = Topology::build(&m);
        // aresta do equador (y ~ 0 nos dois vértices)
        let e = some_edge_with(&m, |a, b| a.y.abs() < 1e-5 && b.y.abs() < 1e-5);
        let lp = edge_loop(&m, &t, e).unwrap();
        assert!(lp.closed, "equador da quad sphere é um loop fechado");
        assert_eq!(lp.verts.len(), 16);
        assert_eq!(lp.edges.len(), 16);
        for &v in &lp.verts {
            assert!(m.co(v).y.abs() < 1e-5);
        }
        // uv sphere: loop vertical termina nos polos (valência != 4)
        let u = primitives::uv_sphere(1.0, 6, 8);
        let tu = Topology::build(&u);
        let e = some_edge_with(&u, |a, b| (a.y - b.y).abs() > 0.1 && a.y.abs() < 0.9 && b.y.abs() < 0.9);
        let lp = edge_loop(&u, &tu, e).unwrap();
        assert!(!lp.closed);
        assert_eq!(lp.edges.len(), lp.verts.len() - 1);
        // meridiano: do polo norte ao sul passando por 5 anéis + polos = 7 vértices, ou termina nos anéis
        assert!(lp.verts.len() >= 5);
    }

    #[test]
    fn edge_loop_on_cube_goes_around() {
        let m = primitives::cube(1.0);
        let t = Topology::build(&m);
        // no cubo, valência 3: o loop não atravessa vértices
        let e = t.edges[0].verts;
        let lp = edge_loop(&m, &t, e).unwrap();
        assert_eq!(lp.edges.len(), 1);
        assert!(!lp.closed);
        let sub = primitives::cube_subdivided(1.0, 2);
        let ts = Topology::build(&sub);
        let e = some_edge_with(&sub, |a, b| (a.y - 0.0).abs() < 1e-5 && (b.y).abs() < 1e-5);
        let lp = edge_loop(&sub, &ts, e).unwrap();
        assert!(lp.closed);
        assert_eq!(lp.verts.len(), 8);
    }

    #[test]
    fn boundary_loop_follows_open_rim() {
        let m = primitives::cylinder(1.0, 2.0, 8, 2, false);
        let t = Topology::build(&m);
        let e = t.edges.iter().find(|e| e.faces.len() == 1).unwrap().verts;
        let lp = boundary_loop(&m, &t, e).unwrap();
        assert!(lp.closed);
        assert_eq!(lp.verts.len(), 8);
        let y0 = m.co(lp.verts[0]).y;
        assert!(lp.verts.iter().all(|&v| (m.co(v).y - y0).abs() < 1e-6));
        // aresta interna: erro claro
        let interior = t.edges.iter().find(|e| e.faces.len() == 2).unwrap().verts;
        assert!(matches!(
            boundary_loop(&m, &t, interior),
            Err(ModelError::InvalidParameter(_))
        ));
        // aresta que não existe
        assert!(matches!(
            boundary_loop(&m, &t, (0, 999)),
            Err(ModelError::MissingEdge(0, 999))
        ));
    }

    #[test]
    fn edge_ring_and_face_loop_on_grid_and_cylinder() {
        let m = primitives::grid(4, 3, 4.0, 3.0);
        let t = Topology::build(&m);
        // aresta horizontal (varia em x) no meio: o ring atravessa as 3 fileiras verticalmente
        let e = some_edge_with(&m, |a, b| (a.z - b.z).abs() < 1e-6 && (a.x - b.x).abs() > 0.5 && a.z.abs() < 0.6);
        let ring = edge_ring(&m, &t, e).unwrap();
        assert_eq!(ring.edges.len(), 4);
        assert_eq!(ring.faces.len(), 3);
        assert!(!ring.closed);
        let fl = face_loop(&m, &t, e).unwrap();
        assert_eq!(fl, ring.faces);

        let c = primitives::cylinder(1.0, 2.0, 8, 1, true);
        let tc = Topology::build(&c);
        // aresta vertical: ring fechado de 8 arestas
        let e = some_edge_with(&c, |a, b| (a.y - b.y).abs() > 1.0);
        let ring = edge_ring(&c, &tc, e).unwrap();
        assert!(ring.closed);
        assert_eq!(ring.edges.len(), 8);
        assert_eq!(ring.faces.len(), 8);
        // aresta horizontal da lateral: o ring para nas tampas (n-gons)
        let e = some_edge_with(&c, |a, b| (a.y - b.y).abs() < 1e-6 && a.y > 0.0);
        let ring = edge_ring(&c, &tc, e).unwrap();
        assert!(!ring.closed);
        assert_eq!(ring.edges.len(), 2);
        assert_eq!(ring.faces.len(), 1);
    }

    #[test]
    fn loop_cut_on_quad_sphere_preserves_euler() {
        let mut m = primitives::quad_sphere(1.0, 4);
        let before_f = m.face_count();
        let before_v = m.vert_count();
        let e = some_edge_with(&m, |a, b| a.y > 0.0 && b.y > 0.0 && (a.y - b.y).abs() > 0.05);
        let out = loop_cut(&mut m, e, 1, 0.0).unwrap();
        assert!(out.closed);
        assert_eq!(out.faces_cut, 16);
        assert_eq!(m.face_count(), before_f + 16);
        assert_eq!(m.vert_count(), before_v + 16);
        assert_eq!(out.op.created_faces.len(), 16);
        assert_eq!(out.op.created_verts.len(), 16);
        assert_eq!(out.op.selection.edges.len(), 16);
        assert_closed(&m, Some(2));
        assert_outward_star(&m);
        // vários cortes
        let mut m2 = primitives::quad_sphere(1.0, 4);
        let out2 = loop_cut(&mut m2, e, 3, 0.0).unwrap();
        assert_eq!(m2.face_count(), before_f + 16 * 3);
        assert_eq!(out2.op.selection.edges.len(), 48);
        assert_closed(&m2, Some(2));
    }

    #[test]
    fn loop_cut_on_cylinder_side_and_caps() {
        // ring vertical -> corta horizontalmente em volta
        let mut m = primitives::cylinder(1.0, 2.0, 8, 1, true);
        let e = some_edge_with(&m, |a, b| (a.y - b.y).abs() > 1.0);
        let out = loop_cut(&mut m, e, 1, 0.0).unwrap();
        assert_eq!(out.faces_cut, 8);
        assert_eq!(m.face_count(), 10 + 8);
        assert_closed(&m, Some(2));
        for &v in &out.op.created_verts {
            assert!(m.co(v).y.abs() < 1e-5, "o corte fica no meio");
        }
        // ring horizontal -> atravessa 1 quad e termina nas tampas (n-gons ganham vértices)
        let mut m = primitives::cylinder(1.0, 2.0, 8, 1, true);
        let e = some_edge_with(&m, |a, b| (a.y - b.y).abs() < 1e-6 && a.y > 0.0);
        let out = loop_cut(&mut m, e, 1, 0.0).unwrap();
        assert_eq!(out.faces_cut, 1);
        assert!(!out.closed);
        assert_closed(&m, Some(2));
        let max_len = m.faces.iter().map(|f| f.len()).max().unwrap();
        assert_eq!(max_len, 9, "tampas viram 9-gons");
    }

    #[test]
    fn loop_cut_slide_positions_and_direction() {
        let mut m = primitives::grid(2, 1, 2.0, 1.0);
        let e = some_edge_with(&m, |a, b| (a.x - b.x).abs() > 0.5 && (a.z + 0.5).abs() < 1e-6 && (b.z + 0.5).abs() < 1e-6 && a.x.min(b.x) < -0.5);
        // a aresta cobre x de -1 a 0; o corte no meio cai em x = -0.5
        let mut m0 = m.clone();
        loop_cut(&mut m0, e, 1, 0.0).unwrap();
        let mid: Vec<f32> = (m.vert_count()..m0.vert_count()).map(|v| m0.co(v as u32).x).collect();
        assert!(mid.iter().any(|x| (x + 0.5).abs() < 1e-5));
        // slide +0.5 -> t = 0.75 a partir do menor índice da aresta
        let out = loop_cut(&mut m, e, 1, 0.5).unwrap();
        let (lo, hi) = (m.co(e.0).x, m.co(e.1).x);
        let expected = lo + (hi - lo) * 0.75;
        let xs: Vec<f32> = out.op.created_verts.iter().map(|&v| m.co(v).x).collect();
        assert!(xs.iter().any(|x| (x - expected).abs() < 1e-5), "{xs:?} vs {expected}");
        // extremos não geram vértices degenerados
        let mut m2 = primitives::grid(2, 1, 2.0, 1.0);
        loop_cut(&mut m2, e, 1, 1.0).unwrap();
        for f in 0..m2.face_count() as u32 {
            assert!(m2.face_area(f) > 1e-6, "face {f} degenerada");
        }
    }

    #[test]
    fn loop_cut_interpolates_uv_and_attrs() {
        let mut m = primitives::grid(2, 1, 2.0, 1.0);
        let e = some_edge_with(&m, |a, b| (a.x - b.x).abs() > 0.5 && (a.z + 0.5).abs() < 1e-6 && (b.z + 0.5).abs() < 1e-6 && a.x.min(b.x) < -0.5);
        m.set_seam(e.0, e.1, true);
        m.set_crease(e.0, e.1, 0.8);
        // rungs com sharp nos dois lados
        let t = Topology::build(&m);
        let face = t.edge_faces(e.0, e.1)[0];
        let verts: Vec<u32> = m.faces[face as usize].verts().collect();
        let rungs: Vec<EdgeKey> = (0..4)
            .map(|i| edge_key(verts[i], verts[(i + 1) % 4]))
            .filter(|&k| k != edge_key(e.0, e.1) && (m.co(k.0).x - m.co(k.1).x).abs() < 1e-6)
            .collect();
        assert_eq!(rungs.len(), 2);
        for r in &rungs {
            m.set_sharp(r.0, r.1, true);
        }
        let out = loop_cut(&mut m, e, 1, 0.0).unwrap();
        let mid = out.op.created_verts[0];
        // UV da face esquerda: u 0..0.5 -> o novo corner está em 0.25
        let mut found = false;
        for f in &m.faces {
            for c in &f.corners {
                if c.vert == mid {
                    assert!(c.uv.is_finite());
                    found = true;
                    assert!((c.uv.x - 0.25).abs() < 1e-5 || (c.uv.x - 0.25).abs() < 1e-5);
                }
            }
        }
        assert!(found);
        // seam e crease nos dois segmentos
        let nl = out.op.created_verts[0];
        assert!(m.edge_attr(e.0, nl).seam && m.edge_attr(nl, e.1).seam);
        assert!((m.edge_attr(e.0, nl).crease - 0.8).abs() < 1e-6);
        assert!(!m.edge_attrs.contains_key(&edge_key(e.0, e.1)), "aresta antiga não existe mais");
        // aresta nova do loop é sharp (ambos os degraus eram)
        let loop_edge = *out.op.selection.edges.iter().next().unwrap();
        assert!(m.edge_attr(loop_edge.0, loop_edge.1).sharp);
        m.validate().unwrap();
        let _ = Vec2::ZERO;
    }

    #[test]
    fn loop_cut_invalid_inputs_leave_mesh_untouched() {
        let mut m = primitives::quad_sphere(1.0, 2);
        let h = m.content_hash();
        let e = Topology::build(&m).edges[0].verts;
        assert!(matches!(loop_cut(&mut m, e, 0, 0.0), Err(ModelError::InvalidParameter(_))));
        assert!(matches!(loop_cut(&mut m, e, 1, 1.5), Err(ModelError::InvalidParameter(_))));
        assert!(matches!(loop_cut(&mut m, e, 1, f32::NAN), Err(ModelError::InvalidParameter(_))));
        assert!(matches!(loop_cut(&mut m, (0, 9999), 1, 0.0), Err(ModelError::VertexOutOfRange(9999, _))));
        let missing = (0..m.vert_count() as u32)
            .find(|&v| Topology::build(&m).edge(0, v).is_none() && v != 0)
            .unwrap();
        assert!(matches!(loop_cut(&mut m, (0, missing), 1, 0.0), Err(ModelError::MissingEdge(_, _))));
        assert_eq!(h, m.content_hash());
        // aresta solta
        let mut w = primitives::cube(1.0);
        crate::ops::extrude::extrude_verts(&mut w, &[0]).unwrap();
        let hw = w.content_hash();
        let wire = *w.edge_attrs.keys().next().unwrap();
        assert!(matches!(loop_cut(&mut w, wire, 1, 0.0), Err(ModelError::UnsupportedTopology(_))));
        // triângulos: ring vazio
        let mut u = primitives::uv_sphere(1.0, 3, 4);
        let tri_edge = Topology::build(&u)
            .edges
            .iter()
            .find(|e| e.faces.iter().all(|&f| u.faces[f as usize].len() == 3))
            .map(|e| e.verts);
        if let Some(te) = tri_edge {
            let hu = u.content_hash();
            assert!(matches!(loop_cut(&mut u, te, 1, 0.0), Err(ModelError::UnsupportedTopology(_))));
            assert_eq!(hu, u.content_hash());
        }
        assert_eq!(hw, w.content_hash());
    }

    #[test]
    fn loop_cut_handles_open_mesh_and_torus() {
        let mut m = primitives::torus(2.0, 0.5, 12, 8);
        let t = Topology::build(&m);
        let e = t.edges[0].verts;
        let f0 = m.face_count();
        let out = loop_cut(&mut m, e, 2, 0.0).unwrap();
        assert!(out.closed);
        assert_eq!(m.face_count(), f0 + out.faces_cut * 2);
        assert_closed(&m, Some(0));

        let mut g = primitives::grid(4, 4, 4.0, 4.0);
        let e = some_edge_with(&g, |a, b| (a.z - b.z).abs() > 0.5 && a.x.abs() < 0.1 && b.x.abs() < 0.1);
        let out = loop_cut(&mut g, e, 1, 0.0).unwrap();
        assert_eq!(out.faces_cut, 4);
        let tg = Topology::build(&g);
        assert_eq!(tg.non_manifold_edge_count(), 0);
        g.validate().unwrap();
    }

    #[test]
    fn loop_cut_is_deterministic_and_fast_on_large_sphere() {
        let build = || {
            let mut m = primitives::quad_sphere(1.0, 64);
            let e = some_edge_with(&m, |a, b| a.y > 0.3 && b.y > 0.3 && (a.y - b.y).abs() > 0.001);
            let t0 = std::time::Instant::now();
            let out = loop_cut(&mut m, e, 1, 0.25).unwrap();
            assert!(out.faces_cut > 100);
            assert!(t0.elapsed().as_secs() < 30);
            m
        };
        let a = build();
        let b = build();
        assert_eq!(a.content_hash(), b.content_hash());
        assert_closed(&a, Some(2));
    }
}
