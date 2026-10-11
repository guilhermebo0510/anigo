//! Costuras de UV (seams) e ilhas.
//!
//! Uma **ilha** é um componente de faces ligado por arestas "contínuas": arestas
//! manifold (exatamente 2 faces), que não são costura e — no modo sensível a UV
//! ([`split_islands`]) — cujos UVs coincidem nas duas faces. Arestas de fronteira
//! e não-manifold (3+ faces) sempre separam ilhas (agem como costuras implícitas).
//!
//! Equivalentes no Blender: Mark/Clear Seam, Seams from Islands, Shortest Path
//! Pick (`seam_path`), Mark Seams by angle (`auto_seams_by_angle`).

use crate::error::{ModelError, ModelResult};
use crate::mesh::{EdgeKey, EditMesh};
use crate::topology::Topology;
use glam::Vec2;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeSet, BinaryHeap, HashMap, HashSet};

/// Tolerância absoluta (espaço UV) para considerar dois UVs "o mesmo ponto".
pub const UV_EPS: f32 = 1e-5;

/// Ilha de UV: faces conectadas sem atravessar costuras (nem descontinuidades de UV).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Island {
    /// Índices de face, em ordem crescente.
    pub faces: Vec<u32>,
    /// Caixa envolvente dos UVs dos corners da ilha: `(min, max)`.
    pub bounds_uv: (Vec2, Vec2),
}

// ------------------------------------------------------------------ utilidades internas

/// União-busca determinística (a menor raiz sempre vence).
pub(crate) struct Dsu {
    parent: Vec<u32>,
}

impl Dsu {
    pub(crate) fn new(n: usize) -> Self {
        Self {
            parent: (0..n as u32).collect(),
        }
    }

    pub(crate) fn find(&mut self, mut x: u32) -> u32 {
        while self.parent[x as usize] != x {
            let p = self.parent[x as usize];
            self.parent[x as usize] = self.parent[p as usize];
            x = self.parent[x as usize];
        }
        x
    }

    pub(crate) fn union(&mut self, a: u32, b: u32) -> bool {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return false;
        }
        if ra < rb {
            self.parent[rb as usize] = ra;
        } else {
            self.parent[ra as usize] = rb;
        }
        true
    }
}

/// Resolve uma seleção opcional de faces: `None` = todas. Valida índices,
/// ordena e remove duplicatas. Erro se a seleção resultante for vazia.
pub(crate) fn resolve_faces(mesh: &EditMesh, faces: Option<&[u32]>) -> ModelResult<Vec<u32>> {
    match faces {
        None => {
            if mesh.faces.is_empty() {
                return Err(ModelError::EmptySelection("a malha não possui faces"));
            }
            Ok((0..mesh.faces.len() as u32).collect())
        }
        Some(list) => {
            if list.is_empty() {
                return Err(ModelError::EmptySelection("nenhuma face selecionada"));
            }
            for &f in list {
                if f as usize >= mesh.faces.len() {
                    return Err(ModelError::FaceOutOfRange(f, mesh.faces.len()));
                }
            }
            let mut v = list.to_vec();
            v.sort_unstable();
            v.dedup();
            Ok(v)
        }
    }
}

pub(crate) fn membership(mesh: &EditMesh, faces: &[u32]) -> Vec<bool> {
    let mut m = vec![false; mesh.faces.len()];
    for &f in faces {
        m[f as usize] = true;
    }
    m
}

/// Caixa dos UVs dos corners das faces dadas (`ZERO` se vazio).
pub(crate) fn faces_uv_bounds(mesh: &EditMesh, faces: &[u32]) -> (Vec2, Vec2) {
    let mut lo = Vec2::splat(f32::INFINITY);
    let mut hi = Vec2::splat(f32::NEG_INFINITY);
    for &f in faces {
        for c in &mesh.faces[f as usize].corners {
            lo = lo.min(c.uv);
            hi = hi.max(c.uv);
        }
    }
    if lo.x > hi.x {
        (Vec2::ZERO, Vec2::ZERO)
    } else {
        (lo, hi)
    }
}

/// Posição (índice do corner de partida) da aresta `e` dentro da face `f`.
fn edge_pos_in_face(topo: &Topology, f: u32, e: u32) -> Option<usize> {
    topo.face_edges[f as usize].iter().position(|&x| x == e)
}

/// UVs `(uv_a, uv_b)` dos extremos da chave ordenada `(a, b)` da aresta `e`, vistos pela face `f`.
fn face_edge_uvs(mesh: &EditMesh, topo: &Topology, f: u32, e: u32) -> Option<(Vec2, Vec2)> {
    let (a, b) = topo.edges[e as usize].verts;
    let i = edge_pos_in_face(topo, f, e)?;
    let face = &mesh.faces[f as usize];
    let n = face.corners.len();
    let (c0, c1) = (face.corners[i], face.corners[(i + 1) % n]);
    if c0.vert == a && c1.vert == b {
        Some((c0.uv, c1.uv))
    } else if c0.vert == b && c1.vert == a {
        Some((c1.uv, c0.uv))
    } else {
        None
    }
}

/// `true` se os UVs dos extremos da aresta coincidem nas duas faces vizinhas.
/// Arestas que não têm exatamente 2 faces são consideradas contínuas (não há o que comparar).
fn edge_uv_continuous(mesh: &EditMesh, topo: &Topology, e: u32) -> bool {
    let info = &topo.edges[e as usize];
    if info.faces.len() != 2 {
        return true;
    }
    let (Some((a0, b0)), Some((a1, b1))) = (
        face_edge_uvs(mesh, topo, info.faces[0], e),
        face_edge_uvs(mesh, topo, info.faces[1], e),
    ) else {
        return true;
    };
    (a0 - a1).abs().max_element() <= UV_EPS && (b0 - b1).abs().max_element() <= UV_EPS
}

fn is_seam(mesh: &EditMesh, key: EdgeKey) -> bool {
    mesh.edge_attrs.get(&key).is_some_and(|a| a.seam)
}

/// Aresta que une duas faces distintas ambas no conjunto, sem costura (e, se
/// `uv_aware`, com UV contínuo).
fn edge_joins(
    mesh: &EditMesh,
    topo: &Topology,
    e: u32,
    in_set: &[bool],
    uv_aware: bool,
) -> Option<(u32, u32)> {
    let info = &topo.edges[e as usize];
    if info.faces.len() != 2 {
        return None;
    }
    let (f, g) = (info.faces[0], info.faces[1]);
    if f == g || !in_set[f as usize] || !in_set[g as usize] || is_seam(mesh, info.verts) {
        return None;
    }
    if uv_aware && !edge_uv_continuous(mesh, topo, e) {
        return None;
    }
    Some((f, g))
}

/// Agrupa as faces de `faces` (ordenadas) em ilhas. Ordem determinística:
/// ilhas ordenadas pela menor face; faces ordenadas dentro da ilha.
pub(crate) fn build_islands(
    mesh: &EditMesh,
    topo: &Topology,
    faces: &[u32],
    uv_aware: bool,
) -> Vec<Vec<u32>> {
    let in_set = membership(mesh, faces);
    let mut dsu = Dsu::new(mesh.faces.len());
    for e in 0..topo.edges.len() as u32 {
        if let Some((f, g)) = edge_joins(mesh, topo, e, &in_set, uv_aware) {
            dsu.union(f, g);
        }
    }
    let mut slot: HashMap<u32, usize> = HashMap::new();
    let mut out: Vec<Vec<u32>> = Vec::new();
    for &f in faces {
        let r = dsu.find(f);
        let idx = *slot.entry(r).or_insert_with(|| {
            out.push(Vec::new());
            out.len() - 1
        });
        out[idx].push(f);
    }
    out
}

fn to_islands(mesh: &EditMesh, groups: Vec<Vec<u32>>) -> Vec<Island> {
    groups
        .into_iter()
        .map(|faces| {
            let bounds_uv = faces_uv_bounds(mesh, &faces);
            Island { faces, bounds_uv }
        })
        .collect()
}

// ------------------------------------------------------------------ wedges (corners unificados)

/// Corners de um mesmo vértice ligados por arestas "contínuas" formam um
/// *wedge* (uma variável de UV). Vértices em costuras têm vários wedges.
pub(crate) struct Wedges {
    pub offset: Vec<usize>,
    /// Wedge de cada corner (indexado por `offset[f] + corner`); `u32::MAX` fora do conjunto.
    pub var: Vec<u32>,
    pub count: usize,
}

impl Wedges {
    #[inline]
    pub(crate) fn corner_var(&self, f: u32, c: usize) -> u32 {
        self.var[self.offset[f as usize] + c]
    }
}

pub(crate) fn compute_wedges(
    mesh: &EditMesh,
    topo: &Topology,
    in_set: &[bool],
    uv_aware: bool,
) -> Wedges {
    let mut offset = Vec::with_capacity(mesh.faces.len() + 1);
    let mut total = 0usize;
    for f in &mesh.faces {
        offset.push(total);
        total += f.corners.len();
    }
    offset.push(total);
    let mut dsu = Dsu::new(total);
    for e in 0..topo.edges.len() as u32 {
        let Some((f, g)) = edge_joins(mesh, topo, e, in_set, uv_aware) else {
            continue;
        };
        let (Some(pf), Some(pg)) = (edge_pos_in_face(topo, f, e), edge_pos_in_face(topo, g, e))
        else {
            continue;
        };
        let (a, b) = topo.edges[e as usize].verts;
        let (cf, cg) = (&mesh.faces[f as usize].corners, &mesh.faces[g as usize].corners);
        for x in [a, b] {
            let find = |cs: &Vec<crate::mesh::Corner>, p: usize| {
                [p, (p + 1) % cs.len()].into_iter().find(|&c| cs[c].vert == x)
            };
            if let (Some(i), Some(j)) = (find(cf, pf), find(cg, pg)) {
                dsu.union((offset[f as usize] + i) as u32, (offset[g as usize] + j) as u32);
            }
        }
    }
    let mut id_of_root: Vec<u32> = vec![u32::MAX; total];
    let mut var = vec![u32::MAX; total];
    let mut count = 0u32;
    for (f, face) in mesh.faces.iter().enumerate() {
        if !in_set[f] {
            continue;
        }
        for c in 0..face.corners.len() {
            let r = dsu.find((offset[f] + c) as u32) as usize;
            if id_of_root[r] == u32::MAX {
                id_of_root[r] = count;
                count += 1;
            }
            var[offset[f] + c] = id_of_root[r];
        }
    }
    Wedges {
        offset,
        var,
        count: count as usize,
    }
}

/// Característica de Euler da ilha **cortada** (V' − E' + F usando wedges como vértices).
/// Um disco topológico tem χ' = 1.
pub(crate) fn island_euler(mesh: &EditMesh, w: &Wedges, faces: &[u32]) -> i64 {
    let mut vars: Vec<u32> = Vec::new();
    let mut edges: HashSet<(u32, u32)> = HashSet::new();
    for &f in faces {
        let n = mesh.faces[f as usize].corners.len();
        for i in 0..n {
            let a = w.corner_var(f, i);
            let b = w.corner_var(f, (i + 1) % n);
            vars.push(a);
            edges.insert((a.min(b), a.max(b)));
        }
    }
    vars.sort_unstable();
    vars.dedup();
    vars.len() as i64 - edges.len() as i64 + faces.len() as i64
}

// ------------------------------------------------------------------ API pública: ilhas

/// Divide a malha em ilhas de UV: componentes de faces conectadas **sem atravessar
/// costuras** (`EdgeAttr::seam`) **nem descontinuidades de UV** entre faces vizinhas.
///
/// Arestas de fronteira ou não-manifold também separam ilhas. A ordem é determinística
/// (ilhas pela menor face). Em malha sem faces devolve vetor vazio.
pub fn split_islands(mesh: &EditMesh) -> Vec<Island> {
    let topo = Topology::build(mesh);
    let faces: Vec<u32> = (0..mesh.faces.len() as u32).collect();
    to_islands(mesh, build_islands(mesh, &topo, &faces, true))
}

/// Como [`split_islands`], restrito a um subconjunto de faces: arestas para fora
/// do subconjunto contam como fronteira. Erro se `faces` tem índice inválido ou está vazio.
pub fn split_islands_in(mesh: &EditMesh, faces: &[u32]) -> ModelResult<Vec<Island>> {
    let faces = resolve_faces(mesh, Some(faces))?;
    let topo = Topology::build(mesh);
    Ok(to_islands(mesh, build_islands(mesh, &topo, &faces, true)))
}

/// Ilhas definidas **apenas pelas costuras** (ignora os UVs atuais) — é a definição
/// usada pelo unwrap, já que os UVs ainda não são confiáveis. `faces = None` usa todas.
pub fn split_islands_by_seams(mesh: &EditMesh, faces: Option<&[u32]>) -> ModelResult<Vec<Island>> {
    let faces = resolve_faces(mesh, faces)?;
    let topo = Topology::build(mesh);
    Ok(to_islands(mesh, build_islands(mesh, &topo, &faces, false)))
}

// ------------------------------------------------------------------ marcar costuras

/// Marca (`seam = true`) ou limpa (`false`) a costura nas `edges` dadas (Mark/Clear Seam).
///
/// Valida **todas** as arestas antes de mutar: se alguma não existe na malha devolve
/// `MissingEdge` e a malha fica intacta. Devolve quantas arestas mudaram de estado
/// (duplicatas contam uma vez; repetir a operação devolve 0).
pub fn mark_seams(mesh: &mut EditMesh, edges: &[EdgeKey], seam: bool) -> ModelResult<usize> {
    let topo = Topology::build(mesh);
    for &(a, b) in edges {
        if topo.edge(a, b).is_none() {
            return Err(ModelError::MissingEdge(a, b));
        }
    }
    let unique: BTreeSet<EdgeKey> = edges.iter().map(|&(a, b)| crate::mesh::edge_key(a, b)).collect();
    let mut changed = 0;
    for (a, b) in unique {
        if mesh.edge_attr(a, b).seam != seam {
            mesh.set_seam(a, b, seam);
            changed += 1;
        }
    }
    Ok(changed)
}

/// Remove todas as costuras da malha (Clear Seam em tudo). Devolve quantas havia.
pub fn clear_seams(mesh: &mut EditMesh) -> usize {
    let keys: Vec<EdgeKey> = mesh
        .edge_attrs
        .iter()
        .filter(|(_, a)| a.seam)
        .map(|(k, _)| *k)
        .collect();
    for &(a, b) in &keys {
        mesh.set_seam(a, b, false);
    }
    keys.len()
}

/// Seams from Islands: marca costura em toda aresta manifold cujos UVs divergem entre
/// as duas faces (os UVs permanecem como estão). Devolve quantas costuras novas.
/// Arestas de fronteira não são marcadas (já separam ilhas por si só).
pub fn seams_from_uv_islands(mesh: &mut EditMesh) -> usize {
    let topo = Topology::build(mesh);
    let mut marks: Vec<EdgeKey> = Vec::new();
    for (e, info) in topo.edges.iter().enumerate() {
        if info.faces.len() == 2
            && !is_seam(mesh, info.verts)
            && !edge_uv_continuous(mesh, &topo, e as u32)
        {
            marks.push(info.verts);
        }
    }
    for &(a, b) in &marks {
        mesh.set_seam(a, b, true);
    }
    marks.len()
}

/// Marca costura em toda aresta com ângulo diedral maior que `angle` (radianos, 0..=π),
/// como "Seams by Angle". Só arestas manifold com normais definidas são avaliadas.
/// Devolve quantas costuras novas; `angle` inválido → `InvalidParameter` (malha intacta).
pub fn auto_seams_by_angle(mesh: &mut EditMesh, angle: f32) -> ModelResult<usize> {
    if !angle.is_finite() || !(0.0..=std::f32::consts::PI).contains(&angle) {
        return Err(ModelError::InvalidParameter(format!(
            "ângulo de costura deve estar em [0, π], recebeu {angle}"
        )));
    }
    let topo = Topology::build(mesh);
    let normals: Vec<_> = (0..mesh.faces.len() as u32).map(|f| mesh.face_normal(f)).collect();
    let mut marks = Vec::new();
    for info in &topo.edges {
        if info.faces.len() != 2 || is_seam(mesh, info.verts) {
            continue;
        }
        let (n0, n1) = (normals[info.faces[0] as usize], normals[info.faces[1] as usize]);
        if n0 == glam::Vec3::ZERO || n1 == glam::Vec3::ZERO {
            continue;
        }
        if n0.dot(n1).clamp(-1.0, 1.0).acos() > angle {
            marks.push(info.verts);
        }
    }
    for &(a, b) in &marks {
        mesh.set_seam(a, b, true);
    }
    Ok(marks.len())
}

/// Converte arestas duras (`sharp`) em costuras. Devolve quantas costuras novas.
pub fn seams_from_sharp(mesh: &mut EditMesh) -> usize {
    let keys: Vec<EdgeKey> = mesh
        .edge_attrs
        .iter()
        .filter(|(_, a)| a.sharp && !a.seam)
        .map(|(k, _)| *k)
        .collect();
    for &(a, b) in &keys {
        mesh.set_seam(a, b, true);
    }
    keys.len()
}

// ------------------------------------------------------------------ Dijkstra / caminho mínimo

#[derive(PartialEq)]
struct HeapNode {
    cost: f64,
    v: u32,
}
impl Eq for HeapNode {}
impl Ord for HeapNode {
    fn cmp(&self, o: &Self) -> Ordering {
        o.cost.total_cmp(&self.cost).then_with(|| o.v.cmp(&self.v))
    }
}
impl PartialOrd for HeapNode {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// Entrada da árvore de caminhos mínimos: distância, vértice pai, aresta usada.
pub(crate) type DijkstraTree = HashMap<u32, (f64, u32, u32)>;

/// Dijkstra determinístico a partir de `src` pelas arestas para as quais `weight`
/// devolve `Some(custo)`. Com `target`, para ao fixá-lo.
pub(crate) fn dijkstra(
    topo: &Topology,
    src: u32,
    target: Option<u32>,
    weight: impl Fn(u32) -> Option<f64>,
) -> DijkstraTree {
    let mut tree: DijkstraTree = HashMap::new();
    let mut done: HashSet<u32> = HashSet::new();
    let mut heap = BinaryHeap::new();
    tree.insert(src, (0.0, u32::MAX, u32::MAX));
    heap.push(HeapNode { cost: 0.0, v: src });
    while let Some(HeapNode { cost, v }) = heap.pop() {
        if !done.insert(v) {
            continue;
        }
        if Some(v) == target {
            break;
        }
        for &e in &topo.vert_edges[v as usize] {
            let Some(w) = weight(e) else { continue };
            let (a, b) = topo.edges[e as usize].verts;
            let o = if a == v { b } else { a };
            if done.contains(&o) {
                continue;
            }
            let nc = cost + w;
            let better = match tree.get(&o) {
                Some(&(c, _, _)) => nc < c,
                None => true,
            };
            if better {
                tree.insert(o, (nc, v, e));
                heap.push(HeapNode { cost: nc, v: o });
            }
        }
    }
    tree
}

fn edge_len(mesh: &EditMesh, topo: &Topology, e: u32) -> f64 {
    let (a, b) = topo.edges[e as usize].verts;
    (mesh.co(a).as_dvec3() - mesh.co(b).as_dvec3()).length()
}

/// Caminho mais curto (comprimento 3D) entre dois vértices ao longo das arestas — o
/// "Shortest Path Pick" do Blender. Devolve as arestas em ordem de `from` até `to`
/// (chaves ordenadas); `from == to` devolve vazio. Desempates determinísticos.
///
/// Erros: vértice fora do intervalo (`VertexOutOfRange`) ou sem caminho entre eles
/// (`UnsupportedTopology`, componentes distintos). Não altera a malha — combine com [`mark_seams`].
pub fn seam_path(mesh: &EditMesh, from_vert: u32, to_vert: u32) -> ModelResult<Vec<EdgeKey>> {
    let n = mesh.verts.len();
    for v in [from_vert, to_vert] {
        if v as usize >= n {
            return Err(ModelError::VertexOutOfRange(v, n));
        }
    }
    if from_vert == to_vert {
        return Ok(Vec::new());
    }
    let topo = Topology::build(mesh);
    let tree = dijkstra(&topo, from_vert, Some(to_vert), |e| Some(edge_len(mesh, &topo, e)));
    if !tree.contains_key(&to_vert) {
        return Err(ModelError::UnsupportedTopology(format!(
            "não há caminho de arestas entre os vértices {from_vert} e {to_vert}"
        )));
    }
    let mut path = Vec::new();
    let mut v = to_vert;
    while v != from_vert {
        let (_, p, e) = tree[&v];
        path.push(topo.edges[e as usize].verts);
        v = p;
    }
    path.reverse();
    Ok(path)
}

/// Caminho mínimo encadeado por vários vértices (`verts[0] → verts[1] → ...`), sem
/// arestas repetidas. Exige pelo menos 2 vértices.
pub fn seam_path_through(mesh: &EditMesh, verts: &[u32]) -> ModelResult<Vec<EdgeKey>> {
    if verts.len() < 2 {
        return Err(ModelError::InvalidParameter(
            "seam_path_through exige pelo menos 2 vértices".into(),
        ));
    }
    let mut out: Vec<EdgeKey> = Vec::new();
    let mut seen: HashSet<EdgeKey> = HashSet::new();
    for w in verts.windows(2) {
        for e in seam_path(mesh, w[0], w[1])? {
            if seen.insert(e) {
                out.push(e);
            }
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------ corte em disco

/// Calcula um conjunto de arestas internas da ilha que, cortadas, a transformam num
/// disco topológico (algoritmo árvore/co-árvore com poda de galhos soltos).
///
/// 1. Árvore geradora de caminhos mínimos `T` nos vértices (arestas de fronteira da
///    ilha custam 0, então fronteiras são "contraídas" e os cortes ligam laços pelo
///    caminho mais curto).
/// 2. Árvore geradora `D` das faces (Kruskal), preferindo arestas fora de `T`.
/// 3. Corte = arestas internas fora de `D`; poda iterativa de pontas soltas
///    (grau 1 fora da fronteira). Em ilha fechada protege o caminho raiz→mais distante,
///    de modo que sobre ao menos um corte (esfera vira meridiano).
fn disk_cut(mesh: &EditMesh, topo: &Topology, faces: &[u32], in_island: &[bool]) -> Vec<EdgeKey> {
    let mut iedges: Vec<u32> = faces
        .iter()
        .flat_map(|&f| topo.face_edges[f as usize].iter().copied())
        .collect();
    iedges.sort_unstable();
    iedges.dedup();
    let edge_set: HashSet<u32> = iedges.iter().copied().collect();
    let interior = |e: u32| -> bool {
        let info = &topo.edges[e as usize];
        info.faces.len() == 2
            && info.faces[0] != info.faces[1]
            && in_island[info.faces[0] as usize]
            && in_island[info.faces[1] as usize]
            && !is_seam(mesh, info.verts)
    };
    let boundary_edges: Vec<u32> = iedges.iter().copied().filter(|&e| !interior(e)).collect();
    let mut island_verts: Vec<u32> = iedges
        .iter()
        .flat_map(|&e| {
            let (a, b) = topo.edges[e as usize].verts;
            [a, b]
        })
        .collect();
    island_verts.sort_unstable();
    island_verts.dedup();

    // Raiz: menor vértice de fronteira; sem fronteira, o vértice mais baixo (y, x, z).
    let root = if let Some(v) = boundary_edges
        .iter()
        .flat_map(|&e| {
            let (a, b) = topo.edges[e as usize].verts;
            [a, b]
        })
        .min()
    {
        v
    } else {
        island_verts
            .iter()
            .copied()
            .min_by(|&a, &b| {
                let (pa, pb) = (mesh.co(a), mesh.co(b));
                pa.y.total_cmp(&pb.y)
                    .then(pa.x.total_cmp(&pb.x))
                    .then(pa.z.total_cmp(&pb.z))
                    .then(a.cmp(&b))
            })
            .unwrap_or(0)
    };

    let tree = dijkstra(topo, root, None, |e| {
        if !edge_set.contains(&e) {
            None
        } else if interior(e) {
            Some(edge_len(mesh, topo, e))
        } else {
            Some(0.0)
        }
    });
    let t_edges: HashSet<u32> = tree
        .iter()
        .filter(|(&v, _)| v != root)
        .map(|(_, &(_, _, e))| e)
        .collect();

    // Caminho protegido (somente ilha fechada).
    let mut protected: HashSet<u32> = HashSet::new();
    if boundary_edges.is_empty() {
        let far = island_verts
            .iter()
            .copied()
            .filter(|v| tree.contains_key(v))
            .max_by(|&a, &b| tree[&a].0.total_cmp(&tree[&b].0).then(b.cmp(&a)));
        if let Some(mut v) = far {
            while v != root {
                let (_, p, e) = tree[&v];
                protected.insert(e);
                v = p;
            }
        }
    }

    // Árvore das faces (Kruskal): arestas fora de T primeiro.
    let interior_edges: Vec<u32> = iedges.iter().copied().filter(|&e| interior(e)).collect();
    let mut dsu = Dsu::new(faces.len());
    let local = |f: u32| faces.binary_search(&f).unwrap_or(0) as u32;
    let mut in_d: HashSet<u32> = HashSet::new();
    for pass in 0..2 {
        for &e in &interior_edges {
            if t_edges.contains(&e) != (pass == 1) {
                continue;
            }
            let info = &topo.edges[e as usize];
            if dsu.union(local(info.faces[0]), local(info.faces[1])) {
                in_d.insert(e);
            }
        }
    }

    // Corte inicial e poda de pontas soltas.
    let mut cut: BTreeSet<u32> = interior_edges.iter().copied().filter(|e| !in_d.contains(e)).collect();
    let mut deg: HashMap<u32, u32> = HashMap::new();
    let mut inc: HashMap<u32, Vec<u32>> = HashMap::new();
    for &e in boundary_edges.iter().chain(cut.iter()) {
        let (a, b) = topo.edges[e as usize].verts;
        for v in [a, b] {
            *deg.entry(v).or_insert(0) += 1;
            inc.entry(v).or_default().push(e);
        }
    }
    let mut queue: Vec<u32> = {
        let mut q: Vec<u32> = deg.iter().filter(|(_, &d)| d == 1).map(|(&v, _)| v).collect();
        q.sort_unstable();
        q
    };
    while let Some(v) = queue.pop() {
        if deg.get(&v).copied() != Some(1) {
            continue;
        }
        let Some(&e) = inc[&v].iter().find(|e| cut.contains(e)) else {
            continue;
        };
        if protected.contains(&e) {
            continue;
        }
        cut.remove(&e);
        let (a, b) = topo.edges[e as usize].verts;
        for x in [a, b] {
            if let Some(d) = deg.get_mut(&x) {
                *d -= 1;
            }
        }
        let other = if a == v { b } else { a };
        if deg.get(&other).copied() == Some(1) {
            queue.push(other);
        }
    }
    cut.into_iter().map(|e| topo.edges[e as usize].verts).collect()
}

// ------------------------------------------------------------------ costuras para personagem

/// Parâmetros de [`auto_seams_for_character_with`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CharacterSeamParams {
    /// Limiar diedral (radianos) acima do qual a aresta vira costura (padrão 70°).
    pub angle: f32,
    /// Corta ilhas que ainda não são disco (esferas, tubos, toros) para permitir o unwrap.
    pub open_closed_islands: bool,
}

impl Default for CharacterSeamParams {
    fn default() -> Self {
        Self {
            angle: 70.0_f32.to_radians(),
            open_closed_islands: true,
        }
    }
}

/// Resumo de [`auto_seams_for_character`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AutoSeamReport {
    /// Costuras novas por ângulo diedral.
    pub angle_seams: usize,
    /// Costuras novas para abrir ilhas fechadas/tubulares/com genus.
    pub cut_seams: usize,
    /// Ilhas resultantes.
    pub islands: usize,
    /// Ilhas que continuam sem ser disco (topologia patológica; o unwrap ainda roda mas com distorção).
    pub non_disk_islands: usize,
}

/// Costuras automáticas para personagem com os parâmetros padrão. Ver [`auto_seams_for_character_with`].
pub fn auto_seams_for_character(mesh: &mut EditMesh) -> ModelResult<AutoSeamReport> {
    auto_seams_for_character_with(mesh, &CharacterSeamParams::default())
}

/// Heurística de costuras para personagens: (1) marca arestas com ângulo diedral acima
/// de `params.angle` (separa dobras fortes: nariz, orelha, bordas de roupa); (2) para
/// cada ilha resultante que não seja disco, aplica um corte mínimo (caminho mais curto
/// entre laços de fronteira, meridiano em esferas, geradores em toros) que a abre.
/// Não é um "auto-seam artístico": posicione costuras ocultas à mão quando importar.
pub fn auto_seams_for_character_with(
    mesh: &mut EditMesh,
    params: &CharacterSeamParams,
) -> ModelResult<AutoSeamReport> {
    if mesh.faces.is_empty() {
        return Err(ModelError::EmptySelection("a malha não possui faces"));
    }
    let mut report = AutoSeamReport {
        angle_seams: auto_seams_by_angle(mesh, params.angle)?,
        ..Default::default()
    };
    let topo = Topology::build(mesh);
    let all: Vec<u32> = (0..mesh.faces.len() as u32).collect();
    let in_all = vec![true; mesh.faces.len()];
    let islands = build_islands(mesh, &topo, &all, false);
    let wedges = compute_wedges(mesh, &topo, &in_all, false);
    let mut cuts: Vec<EdgeKey> = Vec::new();
    let mut unresolved = Vec::new();
    for isl in &islands {
        if island_euler(mesh, &wedges, isl) == 1 || !params.open_closed_islands {
            continue;
        }
        let in_island = membership(mesh, isl);
        cuts.extend(disk_cut(mesh, &topo, isl, &in_island));
        unresolved.push(isl.clone());
    }
    for &(a, b) in &cuts {
        mesh.set_seam(a, b, true);
    }
    report.cut_seams = cuts.len();
    // Reavalia após o corte.
    let topo2 = Topology::build(mesh);
    let islands2 = build_islands(mesh, &topo2, &all, false);
    let wedges2 = compute_wedges(mesh, &topo2, &in_all, false);
    report.islands = islands2.len();
    report.non_disk_islands = islands2
        .iter()
        .filter(|isl| island_euler(mesh, &wedges2, isl) != 1)
        .count();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;
    use glam::Vec3;

    fn zero_uvs(m: &mut EditMesh) {
        for f in &mut m.faces {
            for c in &mut f.corners {
                c.uv = Vec2::ZERO;
            }
        }
    }

    /// Corta as 7 arestas de uma "cruz" (complemento de uma árvore geradora do dual).
    fn cross_cut(m: &EditMesh) -> Vec<EdgeKey> {
        let topo = Topology::build(m);
        let mut dsu = Dsu::new(m.faces.len());
        let mut keep = HashSet::new();
        for (i, e) in topo.edges.iter().enumerate() {
            if e.faces.len() == 2 && dsu.union(e.faces[0], e.faces[1]) {
                keep.insert(i);
            }
        }
        topo.edges
            .iter()
            .enumerate()
            .filter(|(i, _)| !keep.contains(i))
            .map(|(_, e)| e.verts)
            .collect()
    }

    fn disk_count(m: &EditMesh) -> (usize, usize) {
        let topo = Topology::build(m);
        let all: Vec<u32> = (0..m.faces.len() as u32).collect();
        let w = compute_wedges(m, &topo, &vec![true; m.faces.len()], false);
        let islands = build_islands(m, &topo, &all, false);
        let disks = islands.iter().filter(|i| island_euler(m, &w, i) == 1).count();
        (islands.len(), disks)
    }

    #[test]
    fn mark_and_clear_seams_report_changes() {
        let mut m = primitives::cube(1.0);
        let topo = Topology::build(&m);
        let real: Vec<EdgeKey> = topo.edges.iter().take(3).map(|e| e.verts).collect();
        assert_eq!(mark_seams(&mut m, &real, true).unwrap(), 3);
        assert_eq!(mark_seams(&mut m, &real, true).unwrap(), 0);
        assert!(m.edge_attr(real[0].0, real[0].1).seam);
        // duplicatas na entrada contam uma vez
        assert_eq!(mark_seams(&mut m, &[real[0], (real[0].1, real[0].0)], false).unwrap(), 1);
        assert_eq!(clear_seams(&mut m), 2);
        assert!(m.edge_attrs.is_empty());
    }

    #[test]
    fn mark_seams_rejects_missing_edge_and_leaves_mesh_intact() {
        let mut m = primitives::cube(1.0);
        let topo = Topology::build(&m);
        let good = topo.edges[0].verts;
        let before = m.clone();
        // vértices 0 e 6 podem não ser adjacentes; usa índice inexistente para garantir
        let err = mark_seams(&mut m, &[good, (0, 99)], true).unwrap_err();
        assert!(matches!(err, ModelError::MissingEdge(0, 99)));
        assert_eq!(m, before);
    }

    #[test]
    fn default_cube_has_six_uv_islands_and_seams_from_islands_marks_all_edges() {
        let mut m = primitives::cube(1.0);
        assert_eq!(split_islands(&m).len(), 6);
        assert_eq!(seams_from_uv_islands(&mut m), 12);
        assert_eq!(seams_from_uv_islands(&mut m), 0);
        // por costuras: 6 ilhas; sem costuras e UVs zerados: 1 ilha
        assert_eq!(split_islands_by_seams(&m, None).unwrap().len(), 6);
        clear_seams(&mut m);
        zero_uvs(&mut m);
        let isl = split_islands(&m);
        assert_eq!(isl.len(), 1);
        assert_eq!(isl[0].faces, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn cube_with_seven_cross_seams_is_one_disk_island() {
        let mut m = primitives::cube(1.0);
        zero_uvs(&mut m);
        let cut = cross_cut(&m);
        assert_eq!(cut.len(), 7);
        assert_eq!(mark_seams(&mut m, &cut, true).unwrap(), 7);
        let isl = split_islands(&m);
        assert_eq!(isl.len(), 1);
        assert_eq!(isl[0].faces.len(), 6);
        assert_eq!(disk_count(&m), (1, 1));
        // duas fendas disjuntas: a ilha continua uma só, mas vira anel (não é disco)
        let mut m2 = primitives::cube(1.0);
        zero_uvs(&mut m2);
        let topo = Topology::build(&m2);
        let e0 = topo.edges[0].verts;
        let e1 = topo
            .edges
            .iter()
            .map(|e| e.verts)
            .find(|e| e.0 != e0.0 && e.0 != e0.1 && e.1 != e0.0 && e.1 != e0.1)
            .unwrap();
        mark_seams(&mut m2, &[e0, e1], true).unwrap();
        assert_eq!(disk_count(&m2), (1, 0));
    }

    #[test]
    fn split_islands_in_restricts_to_subset_and_validates() {
        let mut m = primitives::cube(1.0);
        zero_uvs(&mut m);
        let isl = split_islands_in(&m, &[0, 2]).unwrap();
        assert_eq!(isl.len(), 1);
        assert_eq!(split_islands_in(&m, &[0, 1]).unwrap().len(), 2);
        assert!(matches!(split_islands_in(&m, &[]), Err(ModelError::EmptySelection(_))));
        assert!(matches!(
            split_islands_in(&m, &[0, 77]),
            Err(ModelError::FaceOutOfRange(77, 6))
        ));
        assert!(split_islands(&EditMesh::new("vazia")).is_empty());
    }

    #[test]
    fn island_bounds_cover_uvs() {
        let m = primitives::grid(2, 2, 1.0, 1.0);
        let isl = split_islands(&m);
        assert_eq!(isl.len(), 1);
        let (lo, hi) = isl[0].bounds_uv;
        assert!((lo - Vec2::ZERO).length() < 1e-6 && (hi - Vec2::ONE).length() < 1e-6);
    }

    #[test]
    fn non_manifold_edge_separates_islands() {
        // três quads compartilhando uma aresta (livro de 3 páginas)
        let mut m = EditMesh::new("livro");
        for p in [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(0.0, 1.0, 1.0),
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(-1.0, 1.0, 0.0),
        ] {
            m.add_vert(p);
        }
        m.add_face(&[0, 2, 3, 1]).unwrap();
        m.add_face(&[0, 4, 5, 1]).unwrap();
        m.add_face(&[0, 6, 7, 1]).unwrap();
        assert_eq!(split_islands(&m).len(), 3);
    }

    #[test]
    fn auto_seams_by_angle_on_cube_and_validation() {
        let mut m = primitives::cube(1.0);
        let before = m.clone();
        assert!(auto_seams_by_angle(&mut m, -1.0).is_err());
        assert!(auto_seams_by_angle(&mut m, f32::NAN).is_err());
        assert!(auto_seams_by_angle(&mut m, 4.0).is_err());
        assert_eq!(m, before);
        assert_eq!(auto_seams_by_angle(&mut m, 100.0_f32.to_radians()).unwrap(), 0);
        assert_eq!(auto_seams_by_angle(&mut m, 45.0_f32.to_radians()).unwrap(), 12);
        assert_eq!(auto_seams_by_angle(&mut m, 45.0_f32.to_radians()).unwrap(), 0);
    }

    #[test]
    fn seams_from_sharp_converts_edges() {
        let mut m = primitives::cube(1.0);
        m.set_sharp(0, 1, true);
        m.set_sharp(1, 2, true);
        let n = seams_from_sharp(&mut m);
        assert_eq!(n, 2);
        assert!(m.edge_attr(0, 1).seam && m.edge_attr(1, 2).seam);
        assert_eq!(seams_from_sharp(&mut m), 0);
    }

    #[test]
    fn seam_path_straight_diagonal_and_errors() {
        let m = primitives::grid(4, 4, 4.0, 4.0);
        // vértices 0 (canto) e 4 (outro canto da mesma linha): caminho reto de 4 arestas
        let p = seam_path(&m, 0, 4).unwrap();
        assert_eq!(p.len(), 4);
        // contíguo
        let mut cur = 0u32;
        for &(a, b) in &p {
            assert!(a == cur || b == cur);
            cur = if a == cur { b } else { a };
        }
        assert_eq!(cur, 4);
        assert!(seam_path(&m, 3, 3).unwrap().is_empty());
        assert!(matches!(seam_path(&m, 0, 999), Err(ModelError::VertexOutOfRange(999, _))));
        let mut two = primitives::cube(1.0);
        two.append(&primitives::cube(1.0));
        assert!(matches!(seam_path(&two, 0, 8), Err(ModelError::UnsupportedTopology(_))));
        // encadeado
        let chain = seam_path_through(&m, &[0, 4, 24]).unwrap();
        assert_eq!(chain.len(), 8);
        assert!(seam_path_through(&m, &[0]).is_err());
    }

    #[test]
    fn seam_path_prefers_geometrically_short_route() {
        // dois caminhos entre A e B: reto longo-curto vs. desvio; o curto vence
        let m = primitives::grid(2, 1, 2.0, 1.0);
        // grid 2x1: vértices (ix, iz) -> iz*3+ix. Do (0,0)=0 ao (2,1)=5: Manhattan = 3.
        let p = seam_path(&m, 0, 5).unwrap();
        let len: f32 = p.iter().map(|&(a, b)| (m.co(a) - m.co(b)).length()).sum();
        assert!((len - 3.0).abs() < 1e-5, "len {len}");
        // com uma diagonal curta o caminho passa a usá-la
        let mut d = EditMesh::new("diag");
        for p in [Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0), Vec3::new(10.0, 1.0, 0.0), Vec3::new(0.0, 1.0, 0.0)] {
            d.add_vert(p);
        }
        d.add_face(&[0, 1, 2]).unwrap();
        d.add_face(&[0, 2, 3]).unwrap();
        assert_eq!(seam_path(&d, 0, 2).unwrap(), vec![(0, 2)]);
    }

    #[test]
    fn auto_seams_character_opens_sphere_tube_and_torus_into_disks() {
        let mut meshes = vec![
            primitives::quad_sphere(1.0, 6),
            primitives::uv_sphere(1.0, 10, 12),
            primitives::torus(1.0, 0.3, 14, 8),
            primitives::cylinder(0.5, 2.0, 10, 3, false), // tubo aberto: 2 laços
            primitives::capsule(0.4, 1.0, 10, 3),
        ];
        for m in &mut meshes {
            zero_uvs(m);
            let name = m.name.clone();
            let rep = auto_seams_for_character(m).unwrap();
            let (n, d) = disk_count(m);
            assert_eq!(n, d, "{name}: todas as ilhas devem ser discos ({rep:?})");
            assert_eq!(rep.non_disk_islands, 0, "{name}");
            assert!(rep.cut_seams > 0, "{name} precisa de corte");
            m.validate().unwrap();
        }
    }

    #[test]
    fn auto_seams_character_keeps_existing_disk_untouched_and_is_idempotent() {
        let mut g = primitives::grid(4, 4, 1.0, 1.0);
        let rep = auto_seams_for_character(&mut g).unwrap();
        assert_eq!(rep.cut_seams + rep.angle_seams, 0);
        assert_eq!(rep.islands, 1);
        let mut s = primitives::quad_sphere(1.0, 5);
        auto_seams_for_character(&mut s).unwrap();
        let snapshot = s.clone();
        let rep2 = auto_seams_for_character(&mut s).unwrap();
        assert_eq!(rep2.angle_seams + rep2.cut_seams, 0);
        assert_eq!(s, snapshot);
        assert!(auto_seams_for_character(&mut EditMesh::new("v")).is_err());
    }

    #[test]
    fn auto_seams_character_is_deterministic() {
        let run = || {
            let mut m = primitives::capsule(0.4, 1.0, 12, 4);
            auto_seams_for_character(&mut m).unwrap();
            m.edge_attrs.keys().copied().collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn large_mesh_seams_sanity() {
        let mut m = primitives::quad_sphere(1.0, 64);
        zero_uvs(&mut m);
        assert!(m.face_count() > 24_000);
        let t = std::time::Instant::now();
        let rep = auto_seams_for_character(&mut m).unwrap();
        assert_eq!(rep.non_disk_islands, 0);
        assert!(t.elapsed().as_secs() < 60, "demorou {:?}", t.elapsed());
        assert_eq!(split_islands(&m).len(), 1);
    }
}
