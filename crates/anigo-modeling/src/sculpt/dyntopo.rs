//! Topologia dinâmica (Dyntopo): subdivide arestas longas e colapsa arestas
//! curtas dentro de uma região, mantendo a malha manifold/fechada.
//!
//! Funcionamento de [`dyntopo_step`]:
//! 1. Por iteração, uma passada de **split** (arestas > `detail × split_ratio`)
//!    e uma de **collapse** (arestas < `detail × collapse_ratio`).
//! 2. Só faces adjacentes a arestas candidatas são trianguladas (n-gons viram
//!    triângulos, atributos por corner preservados); o resto da malha fica intacto.
//! 3. Cada passada trabalha sobre um conjunto independente de arestas (faces /
//!    vértices já tocados na passada são adiados para a próxima iteração).
//! 4. Ao final, vértices órfãos criados pelos colapsos são compactados e o
//!    `Remap` é devolvido — **índices de vértice mudam**; reconstrua o BVH e o
//!    `SculptContext` depois de chamar.
//!
//! Limitações: o split usa o ponto médio da aresta (não projeta na superfície
//! suave); o colapso não toca fronteiras (por padrão), arestas com
//! sharp/seam/vinco, vértices mascarados/ocultos nem arestas não-manifold;
//! UVs de corners colapsados mantêm o valor de cada corner (ilhas de UV podem
//! esticar levemente). O custo por passada é O(faces) (reconstrução da
//! `Topology`), adequado a dabs, não a quadros de 60 Hz em malhas > 200 k faces.

use crate::convert::triangulate_polygon;
use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, Corner, EVert, EditMesh, Remap};
use crate::topology::Topology;
use glam::Vec3;
use std::collections::HashMap;

/// Parâmetros do Dyntopo.
#[derive(Debug, Clone, PartialEq)]
pub struct DyntopoParams {
    /// Arestas maiores que `detail × split_ratio` são divididas.
    pub split_ratio: f32,
    /// Arestas menores que `detail × collapse_ratio` são colapsadas. Precisa
    /// ser < `split_ratio / 2` quando as duas operações estão ligadas.
    pub collapse_ratio: f32,
    pub subdivide: bool,
    pub collapse: bool,
    /// Máximo de iterações (split+collapse) por chamada.
    pub max_iterations: u32,
    /// Orçamento de operações (splits + colapsos) por chamada.
    pub max_edge_ops: usize,
    /// Não colapsa arestas que tocam fronteira aberta.
    pub preserve_boundary: bool,
    /// Ignora arestas com extremo de máscara ≥ 0.5.
    pub respect_mask: bool,
}

impl Default for DyntopoParams {
    fn default() -> Self {
        Self {
            split_ratio: 1.0,
            collapse_ratio: 0.4,
            subdivide: true,
            collapse: true,
            max_iterations: 4,
            max_edge_ops: 50_000,
            preserve_boundary: true,
            respect_mask: true,
        }
    }
}

impl DyntopoParams {
    pub fn validate(&self) -> ModelResult<()> {
        let bad = |what: &str, v: f32| Err(ModelError::InvalidParameter(format!("{what} inválido: {v}")));
        if !(self.split_ratio > 0.0) || !self.split_ratio.is_finite() {
            return bad("split_ratio", self.split_ratio);
        }
        if !(self.collapse_ratio >= 0.0) || !self.collapse_ratio.is_finite() {
            return bad("collapse_ratio", self.collapse_ratio);
        }
        if self.subdivide && self.collapse && self.collapse_ratio >= 0.5 * self.split_ratio {
            return Err(ModelError::InvalidParameter(format!(
                "collapse_ratio ({}) deve ser < metade de split_ratio ({}) para evitar oscilação",
                self.collapse_ratio, self.split_ratio
            )));
        }
        if self.max_iterations == 0 {
            return Err(ModelError::InvalidParameter("max_iterations deve ser ≥ 1".into()));
        }
        Ok(())
    }
}

/// O que um passo de Dyntopo fez.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DyntopoReport {
    pub splits: usize,
    pub collapses: usize,
    /// Faces n-gon trianguladas para permitir as operações.
    pub triangulated_faces: usize,
    pub iterations: u32,
    /// Parou por limite de iterações/orçamento com trabalho ainda pendente.
    pub hit_limit: bool,
    pub verts_before: usize,
    pub verts_after: usize,
    pub faces_before: usize,
    pub faces_after: usize,
    /// Região atualizada (inclui vértices novos; índices já remapeados), ordenada.
    pub region: Vec<u32>,
    /// Presente quando vértices foram compactados (colapsos).
    pub remap: Option<Remap>,
}

impl DyntopoReport {
    /// A malha mudou de topologia.
    pub fn changed(&self) -> bool {
        self.splits > 0 || self.collapses > 0 || self.triangulated_faces > 0
    }
}

/// Executa um passo de Dyntopo na região `region_verts`.
///
/// `detail_edge_len` é o comprimento de aresta alvo (unidades do mundo). Veja
/// o módulo para o algoritmo e as limitações. Valida tudo antes de mutar.
pub fn dyntopo_step(
    mesh: &mut EditMesh,
    region_verts: &[u32],
    detail_edge_len: f32,
    params: &DyntopoParams,
) -> ModelResult<DyntopoReport> {
    params.validate()?;
    if !(detail_edge_len > 0.0) || !detail_edge_len.is_finite() {
        return Err(ModelError::InvalidParameter(format!(
            "comprimento de detalhe inválido: {detail_edge_len}"
        )));
    }
    mesh.validate()?;
    if region_verts.is_empty() {
        return Err(ModelError::EmptySelection("região de dyntopo vazia"));
    }
    let n0 = mesh.verts.len();
    for &v in region_verts {
        if v as usize >= n0 {
            return Err(ModelError::VertexOutOfRange(v, n0));
        }
    }

    let max_len = detail_edge_len * params.split_ratio;
    let min_len = detail_edge_len * params.collapse_ratio;
    let used_before = mesh.used_verts();
    let mut in_region = vec![false; n0];
    for &v in region_verts {
        in_region[v as usize] = true;
    }
    let mut report = DyntopoReport {
        verts_before: n0,
        faces_before: mesh.faces.len(),
        ..Default::default()
    };
    let mut ops_left = params.max_edge_ops;
    let mut pending = false;
    for it in 0..params.max_iterations {
        let mut changed = false;
        if params.subdivide && ops_left > 0 {
            let s = split_pass(mesh, &mut in_region, max_len, params, &mut ops_left, &mut report);
            report.splits += s;
            changed |= s > 0;
        }
        if params.collapse && ops_left > 0 {
            let limit = params.subdivide.then_some(max_len);
            let c = collapse_pass(mesh, &mut in_region, min_len, limit, params, &mut ops_left, &mut report);
            report.collapses += c;
            changed |= c > 0;
        }
        report.iterations = it + 1;
        pending = changed;
        if !changed {
            break;
        }
        if ops_left == 0 {
            break;
        }
    }
    report.hit_limit = pending;

    // compacta vértices órfãos criados pelos colapsos (nunca os que já eram soltos)
    let mut remap = None;
    if report.collapses > 0 {
        let used_after = mesh.used_verts();
        let mut keep: Vec<bool> = (0..mesh.verts.len())
            .map(|v| used_after[v] || v >= used_before.len() || !used_before[v])
            .collect();
        for (a, b) in mesh.edge_attrs.keys() {
            keep[*a as usize] = true;
            keep[*b as usize] = true;
        }
        let r = mesh.compact_verts(&keep);
        let mut new_region: Vec<u32> = (0..in_region.len())
            .filter(|&v| in_region[v])
            .filter_map(|v| r.vert.get(v).copied().flatten())
            .collect();
        new_region.sort_unstable();
        new_region.dedup();
        report.region = new_region;
        remap = Some(r);
    } else {
        report.region = (0..in_region.len() as u32).filter(|&v| in_region[v as usize]).collect();
    }
    report.remap = remap;
    report.verts_after = mesh.verts.len();
    report.faces_after = mesh.faces.len();
    Ok(report)
}

// ------------------------------------------------------------------ utilidades

fn edge_len(mesh: &EditMesh, a: u32, b: u32) -> f32 {
    mesh.co(a).distance(mesh.co(b))
}

/// Triangula as faces indicadas (a primeira triângulo ocupa o lugar da face;
/// os demais são anexados). Devolve quantas faces foram trianguladas.
fn triangulate_faces(mesh: &mut EditMesh, faces: &[u32]) -> usize {
    let mut count = 0;
    for &f in faces {
        let face = mesh.faces[f as usize].clone();
        if face.corners.len() <= 3 {
            continue;
        }
        let pts: Vec<Vec3> = face.corners.iter().map(|c| mesh.verts[c.vert as usize].co).collect();
        let tris = triangulate_polygon(&pts);
        if tris.is_empty() {
            continue;
        }
        let mk = |t: &[usize; 3]| {
            let mut nf = face.clone();
            nf.corners = vec![face.corners[t[0]], face.corners[t[1]], face.corners[t[2]]];
            nf
        };
        mesh.faces[f as usize] = mk(&tris[0]);
        for t in &tris[1..] {
            mesh.faces.push(mk(t));
        }
        count += 1;
    }
    count
}

fn vert_ok(mesh: &EditMesh, params: &DyntopoParams, v: u32) -> bool {
    let vert = &mesh.verts[v as usize];
    !vert.hidden && !(params.respect_mask && vert.mask >= 0.5)
}

fn faces_ok(mesh: &EditMesh, faces: &[u32]) -> bool {
    faces.iter().all(|&f| !mesh.faces[f as usize].hidden)
}

fn is_open_boundary_vert(topo: &Topology, v: u32) -> bool {
    topo.vert_edges[v as usize]
        .iter()
        .any(|&e| topo.edges[e as usize].faces.len() == 1)
}

// ----------------------------------------------------------------------- split

fn long_edges(
    mesh: &EditMesh,
    topo: &Topology,
    in_region: &[bool],
    max_len: f32,
    params: &DyntopoParams,
) -> Vec<(f32, u32)> {
    let mut out = Vec::new();
    for (ei, e) in topo.edges.iter().enumerate() {
        let (a, b) = e.verts;
        if e.faces.is_empty() || e.faces.len() > 2 {
            continue;
        }
        if e.faces.len() == 2 && e.faces[0] == e.faces[1] {
            continue;
        }
        if !(in_region[a as usize] || in_region[b as usize]) {
            continue;
        }
        if !vert_ok(mesh, params, a) || !vert_ok(mesh, params, b) || !faces_ok(mesh, &e.faces) {
            continue;
        }
        let len = edge_len(mesh, a, b);
        if len > max_len {
            out.push((len, ei as u32));
        }
    }
    out
}

fn split_pass(
    mesh: &mut EditMesh,
    in_region: &mut Vec<bool>,
    max_len: f32,
    params: &DyntopoParams,
    ops_left: &mut usize,
    report: &mut DyntopoReport,
) -> usize {
    let mut topo = Topology::build(mesh);
    let mut cands = long_edges(mesh, &topo, in_region, max_len, params);
    if cands.is_empty() {
        return 0;
    }
    // triangula só as faces adjacentes às candidatas
    let mut to_tri: Vec<u32> = cands
        .iter()
        .flat_map(|&(_, e)| topo.edges[e as usize].faces.iter().copied())
        .filter(|&f| mesh.faces[f as usize].len() != 3)
        .collect();
    to_tri.sort_unstable();
    to_tri.dedup();
    if !to_tri.is_empty() {
        report.triangulated_faces += triangulate_faces(mesh, &to_tri);
        topo = Topology::build(mesh);
        cands = long_edges(mesh, &topo, in_region, max_len, params);
    }
    // mais longas primeiro; desempate pela chave da aresta (determinismo)
    cands.sort_by(|x, y| {
        y.0.total_cmp(&x.0)
            .then(topo.edges[x.1 as usize].verts.cmp(&topo.edges[y.1 as usize].verts))
    });
    let mut done = vec![false; mesh.faces.len()];
    let mut splits = 0;
    for (_, ei) in cands {
        if *ops_left == 0 {
            break;
        }
        let e = &topo.edges[ei as usize];
        let (a, b) = e.verts;
        let faces = e.faces.clone();
        if faces
            .iter()
            .any(|&f| done[f as usize] || mesh.faces[f as usize].len() != 3)
        {
            continue;
        }
        // localiza a aresta em cada face antes de mutar
        let mut slots = Vec::with_capacity(2);
        for &f in &faces {
            let c = &mesh.faces[f as usize].corners;
            let found = (0..3).find(|&i| {
                let (x, y) = (c[i].vert, c[(i + 1) % 3].vert);
                (x == a && y == b) || (x == b && y == a)
            });
            match found {
                Some(i) => slots.push((f, i)),
                None => break,
            }
        }
        if slots.len() != faces.len() {
            continue;
        }
        let mid = EVert::lerp(&mesh.verts[a as usize], &mesh.verts[b as usize], 0.5);
        let m = mesh.push_vert(mid);
        in_region.push(true);
        for (f, i) in slots {
            let face = mesh.faces[f as usize].clone();
            let (c0, c1, c2) = (face.corners[i], face.corners[(i + 1) % 3], face.corners[(i + 2) % 3]);
            let mid_corner = Corner {
                vert: m,
                uv: (c0.uv + c1.uv) * 0.5,
                normal: match (c0.normal, c1.normal) {
                    (Some(x), Some(y)) => (x + y).try_normalize().or(Some(x)),
                    _ => None,
                },
            };
            let mut fa = face.clone();
            fa.corners = vec![c0, mid_corner, c2];
            let mut fb = face;
            fb.corners = vec![mid_corner, c1, c2];
            mesh.faces[f as usize] = fa;
            mesh.faces.push(fb);
            done[f as usize] = true;
        }
        if let Some(attr) = mesh.edge_attrs.remove(&edge_key(a, b)) {
            mesh.edge_attrs.insert(edge_key(a, m), attr);
            mesh.edge_attrs.insert(edge_key(m, b), attr);
        }
        splits += 1;
        *ops_left -= 1;
    }
    splits
}

// -------------------------------------------------------------------- collapse

fn short_edges(
    mesh: &EditMesh,
    topo: &Topology,
    in_region: &[bool],
    min_len: f32,
    params: &DyntopoParams,
) -> Vec<(f32, u32)> {
    let mut out = Vec::new();
    for (ei, e) in topo.edges.iter().enumerate() {
        let (a, b) = e.verts;
        if e.faces.is_empty() || e.faces.len() > 2 {
            continue;
        }
        if e.faces.len() == 2 && e.faces[0] == e.faces[1] {
            continue;
        }
        if !(in_region[a as usize] && in_region[b as usize]) {
            continue;
        }
        if !vert_ok(mesh, params, a) || !vert_ok(mesh, params, b) || !faces_ok(mesh, &e.faces) {
            continue;
        }
        if !mesh.edge_attr(a, b).is_default() {
            continue; // preserva arestas de feature (sharp/seam/vinco/bevel)
        }
        let len = edge_len(mesh, a, b);
        if len < min_len {
            out.push((len, ei as u32));
        }
    }
    out
}

fn sorted_intersection(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                out.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out
}

/// Terceiro vértice do triângulo `f` além de `a` e `b`.
fn third_vertex(mesh: &EditMesh, f: u32, a: u32, b: u32) -> Option<u32> {
    mesh.faces[f as usize].verts().find(|&v| v != a && v != b)
}

#[allow(clippy::too_many_arguments)]
fn collapse_pass(
    mesh: &mut EditMesh,
    in_region: &mut [bool],
    min_len: f32,
    max_len: Option<f32>,
    params: &DyntopoParams,
    ops_left: &mut usize,
    report: &mut DyntopoReport,
) -> usize {
    let mut topo = Topology::build(mesh);
    let mut cands = short_edges(mesh, &topo, in_region, min_len, params);
    if cands.is_empty() {
        return 0;
    }
    let mut to_tri: Vec<u32> = cands
        .iter()
        .flat_map(|&(_, e)| topo.edges[e as usize].faces.iter().copied())
        .filter(|&f| mesh.faces[f as usize].len() != 3)
        .collect();
    to_tri.sort_unstable();
    to_tri.dedup();
    if !to_tri.is_empty() {
        report.triangulated_faces += triangulate_faces(mesh, &to_tri);
        topo = Topology::build(mesh);
        cands = short_edges(mesh, &topo, in_region, min_len, params);
    }
    cands.sort_by(|x, y| {
        x.0.total_cmp(&y.0)
            .then(topo.edges[x.1 as usize].verts.cmp(&topo.edges[y.1 as usize].verts))
    });

    let mut live_faces = mesh.faces.len();
    let mut dirty = vec![false; mesh.verts.len()];
    let mut dead: Vec<u32> = Vec::new();
    let mut collapsed: HashMap<u32, u32> = HashMap::new();
    let mut count = 0usize;

    for (_, ei) in cands {
        if *ops_left == 0 {
            break;
        }
        let e = &topo.edges[ei as usize];
        let (a, b) = e.verts;
        if dirty[a as usize] || dirty[b as usize] {
            continue;
        }
        let faces = e.faces.clone();
        if faces.iter().any(|&f| mesh.faces[f as usize].len() != 3) {
            continue;
        }
        if live_faces <= 4 + faces.len() {
            continue; // nunca reduz a malha a menos que um tetraedro
        }
        let (ba, bb) = (is_open_boundary_vert(&topo, a), is_open_boundary_vert(&topo, b));
        if params.preserve_boundary && (ba || bb) {
            continue;
        }
        if (ba || bb) && faces.len() == 2 {
            continue; // aresta interna ligando bordas: colapsar fecha um "pescoço"
        }
        // condição de link: vizinhos comuns == vértices opostos das faces da aresta
        let na = topo.vert_neighbors(a);
        let nb = topo.vert_neighbors(b);
        let common = sorted_intersection(&na, &nb);
        let mut opp: Vec<u32> = faces.iter().filter_map(|&f| third_vertex(mesh, f, a, b)).collect();
        opp.sort_unstable();
        opp.dedup();
        if common != opp {
            continue;
        }
        // quem fica e onde
        let (keep, remove) = if bb && !ba { (b, a) } else { (a, b) };
        let merged = if ba || bb {
            mesh.verts[keep as usize].clone()
        } else {
            EVert::lerp(&mesh.verts[a as usize], &mesh.verts[b as usize], 0.5)
        };
        let new_pos = merged.co;

        // geometria das faces que sobrevivem: sem inversão/degeneração e sem criar arestas longas
        let mut ok = true;
        'faces: for &endpoint in &[a, b] {
            for &g in &topo.vert_faces[endpoint as usize] {
                if faces.contains(&g) {
                    continue;
                }
                let pts: Vec<(u32, Vec3)> = mesh.faces[g as usize]
                    .verts()
                    .map(|v| (v, mesh.verts[v as usize].co))
                    .collect();
                if pts.len() != 3 {
                    ok = false;
                    break 'faces;
                }
                let old = (pts[1].1 - pts[0].1).cross(pts[2].1 - pts[0].1);
                let moved: Vec<Vec3> = pts
                    .iter()
                    .map(|&(v, p)| if v == a || v == b { new_pos } else { p })
                    .collect();
                let new = (moved[1] - moved[0]).cross(moved[2] - moved[0]);
                if new.length_squared() < 1e-20 {
                    ok = false;
                    break 'faces;
                }
                if old.length_squared() > 1e-20 && new.dot(old) <= 0.5 * new.length() * old.length() {
                    ok = false;
                    break 'faces;
                }
                // não cria fatias (triângulos finos): qualidade = 2·área / maior aresta²
                let quality = |p: &[Vec3]| {
                    let e = (p[0].distance_squared(p[1]))
                        .max(p[1].distance_squared(p[2]))
                        .max(p[2].distance_squared(p[0]));
                    if e <= 0.0 {
                        0.0
                    } else {
                        (p[1] - p[0]).cross(p[2] - p[0]).length() / e
                    }
                };
                let old_pts: Vec<Vec3> = pts.iter().map(|x| x.1).collect();
                let q_new = quality(&moved);
                if q_new < 0.15 && q_new < quality(&old_pts) {
                    ok = false;
                    break 'faces;
                }
                if let Some(limit) = max_len {
                    for &(v, p) in &pts {
                        if v != a && v != b && p.distance(new_pos) > limit {
                            ok = false;
                            break 'faces;
                        }
                    }
                }
            }
        }
        if !ok {
            continue;
        }

        // aplica
        for &g in &topo.vert_faces[remove as usize] {
            if faces.contains(&g) {
                continue;
            }
            for c in &mut mesh.faces[g as usize].corners {
                if c.vert == remove {
                    c.vert = keep;
                }
            }
        }
        mesh.verts[keep as usize] = merged;
        dead.extend_from_slice(&faces);
        live_faces -= faces.len();
        collapsed.insert(remove, keep);
        in_region[remove as usize] = false;
        for &v in na.iter().chain(nb.iter()).chain([a, b].iter()) {
            dirty[v as usize] = true;
        }
        count += 1;
        *ops_left -= 1;
    }

    if count > 0 {
        let attrs = std::mem::take(&mut mesh.edge_attrs);
        for ((x, y), attr) in attrs {
            let x2 = collapsed.get(&x).copied().unwrap_or(x);
            let y2 = collapsed.get(&y).copied().unwrap_or(y);
            if x2 != y2 {
                mesh.edge_attrs.entry(edge_key(x2, y2)).or_insert(attr);
            }
        }
        dead.sort_unstable();
        dead.dedup();
        // índices de face em `dead` são os atuais (nenhuma remoção aconteceu antes)
        let _ = mesh.delete_faces(&dead);
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    fn volume(m: &EditMesh) -> f32 {
        let mut v = 0.0;
        for f in 0..m.face_count() as u32 {
            let pts = m.face_positions(f);
            for t in triangulate_polygon(&pts) {
                v += pts[t[0]].dot(pts[t[1]].cross(pts[t[2]])) / 6.0;
            }
        }
        v
    }

    /// Área da superfície triangulada (a área de Newell de um quad não planar
    /// difere da soma dos dois triângulos).
    fn area(m: &EditMesh) -> f32 {
        let mut a = 0.0;
        for f in 0..m.face_count() as u32 {
            let pts = m.face_positions(f);
            for t in triangulate_polygon(&pts) {
                a += (pts[t[1]] - pts[t[0]]).cross(pts[t[2]] - pts[t[0]]).length() * 0.5;
            }
        }
        a
    }

    fn all_verts(m: &EditMesh) -> Vec<u32> {
        (0..m.vert_count() as u32).collect()
    }

    fn step_all(m: &mut EditMesh, detail: f32, p: &DyntopoParams) -> ModelResult<DyntopoReport> {
        let region = all_verts(m);
        dyntopo_step(m, &region, detail, p)
    }

    fn avg_edge(m: &EditMesh) -> f32 {
        let t = Topology::build(m);
        t.edges.iter().map(|e| edge_len(m, e.verts.0, e.verts.1)).sum::<f32>() / t.edges.len() as f32
    }

    fn assert_closed_sphere(m: &EditMesh) {
        m.validate().unwrap();
        let t = Topology::build(m);
        assert!(t.is_closed_manifold(), "deve continuar fechada e manifold");
        assert_eq!(t.euler_characteristic(m), 2);
        assert_eq!(t.non_manifold_edge_count(), 0);
        // normais para fora (a esfera está centrada na origem); ignora fatias
        // quase degeneradas, cuja normal é numericamente instável
        for f in 0..m.face_count() as u32 {
            let pts = m.face_positions(f);
            let max_edge = (0..pts.len())
                .map(|i| pts[i].distance(pts[(i + 1) % pts.len()]))
                .fold(0.0f32, f32::max);
            if m.face_area(f) < 0.02 * max_edge * max_edge {
                continue;
            }
            assert!(m.face_normal(f).dot(m.face_center(f)) > 0.0, "face {f} invertida");
        }
    }

    #[test]
    fn subdivide_grows_counts_keeps_euler_and_geometry() {
        let mut m = primitives::quad_sphere(1.0, 4);
        let (f0, v0) = (m.face_count(), m.vert_count());
        let (a0, vol0) = (area(&m), volume(&m));
        let target = avg_edge(&m) * 0.5;
        let params = DyntopoParams {
            collapse: false,
            max_iterations: 6,
            ..Default::default()
        };
        let r = step_all(&mut m, target, &params).unwrap();
        assert!(r.splits > 0 && r.collapses == 0 && r.triangulated_faces > 0);
        assert!(m.face_count() > f0 * 2 && m.vert_count() > v0);
        assert_eq!(r.verts_after, m.vert_count());
        assert_eq!(r.faces_after, m.face_count());
        assert_closed_sphere(&m);
        // splits no ponto médio não mudam a superfície
        assert!((area(&m) - a0).abs() < 1e-3);
        assert!((volume(&m) - vol0).abs() < 1e-3);
        // arestas ficam (quase) todas abaixo do limite
        let t = Topology::build(&m);
        let long = t.edges.iter().filter(|e| edge_len(&m, e.verts.0, e.verts.1) > target * 1.0001).count();
        assert!(long < t.edges.len() / 20, "{long} de {} ainda longas", t.edges.len());
        assert!(r.region.len() >= m.vert_count() - 1);
        assert!(r.remap.is_none());
    }

    #[test]
    fn subdivision_converges_and_second_run_is_noop() {
        let mut m = primitives::quad_sphere(1.0, 3);
        let params = DyntopoParams {
            collapse: false,
            max_iterations: 20,
            ..Default::default()
        };
        let detail = 0.25;
        let r = step_all(&mut m, detail, &params).unwrap();
        assert!(!r.hit_limit, "deveria convergir em 20 iterações: {r:?}");
        let r2 = step_all(&mut m, detail, &params).unwrap();
        assert!(!r2.changed(), "{r2:?}");
        assert_eq!(r2.iterations, 1);
        assert_closed_sphere(&m);
    }

    #[test]
    fn collapse_reduces_counts_keeps_closed_manifold() {
        let mut m = primitives::quad_sphere(1.0, 8);
        let (f0, v0) = (m.face_count(), m.vert_count());
        let vol0 = volume(&m);
        let target = avg_edge(&m) * 2.2; // min_len = 0.88 * média
        let params = DyntopoParams {
            subdivide: false,
            max_iterations: 8,
            collapse_ratio: 0.4,
            ..Default::default()
        };
        let r = step_all(&mut m, target, &params).unwrap();
        assert!(r.collapses > 0 && r.splits == 0);
        assert!(m.face_count() < f0 * 2, "tri: {} vs {}", m.face_count(), f0);
        assert!(m.vert_count() < v0);
        assert_closed_sphere(&m);
        // forma preservada aproximadamente
        let vol = volume(&m);
        assert!(vol > vol0 * 0.7 && vol <= vol0 * 1.0001, "{vol} vs {vol0}");
        let remap = r.remap.expect("colapsos compactam vértices");
        assert_eq!(remap.vert.len(), v0 + 0);
        assert_eq!(r.verts_after, m.vert_count());
        // região remapeada válida
        assert!(r.region.iter().all(|&v| (v as usize) < m.vert_count()));
    }

    #[test]
    fn split_then_collapse_with_consistent_ratios_stays_valid() {
        let mut m = primitives::quad_sphere(1.0, 5);
        let mut rng = crate::bvh::test_util::Rng(4);
        for step in 0..6 {
            let detail = [0.15, 0.4, 0.2, 0.5, 0.12, 0.3][step];
            let region = all_verts(&m);
            let r = dyntopo_step(&mut m, &region, detail, &DyntopoParams::default()).unwrap();
            assert_closed_sphere(&m);
            assert!(r.verts_after == m.vert_count());
            let _ = rng.next_u32();
        }
    }

    #[test]
    fn region_limits_where_topology_changes() {
        let mut m = primitives::quad_sphere(1.0, 6);
        let orig = m.clone();
        let region: Vec<u32> = (0..m.vert_count() as u32).filter(|&v| m.co(v).y > 0.8).collect();
        assert!(!region.is_empty());
        let params = DyntopoParams {
            collapse: false,
            max_iterations: 2,
            ..Default::default()
        };
        let detail = avg_edge(&m) * 0.4;
        let r = dyntopo_step(&mut m, &region, detail, &params).unwrap();
        assert!(r.splits > 0);
        assert_closed_sphere(&m);
        // vértices originais nunca se movem em splits
        for v in 0..orig.vert_count() {
            assert_eq!(m.verts[v].co, orig.verts[v].co);
        }
        // novos vértices ficam perto da região (y alto)
        for v in orig.vert_count()..m.vert_count() {
            assert!(m.verts[v].co.y > 0.5, "vértice novo longe da região: {:?}", m.verts[v].co);
        }
        // uma região total gera bem mais splits
        let mut full = orig.clone();
        let rf = step_all(&mut full, detail, &params).unwrap();
        assert!(rf.splits > r.splits * 2);
    }

    #[test]
    fn open_mesh_boundary_is_preserved_on_collapse() {
        let mut m = primitives::grid(12, 12, 2.0, 2.0);
        let boundary_before: Vec<Vec3> = {
            let t = Topology::build(&m);
            (0..m.vert_count() as u32)
                .filter(|&v| is_open_boundary_vert(&t, v))
                .map(|v| m.co(v))
                .collect()
        };
        let params = DyntopoParams {
            subdivide: false,
            max_iterations: 6,
            ..Default::default()
        };
        let r = step_all(&mut m, 0.6, &params).unwrap();
        assert!(r.collapses > 0);
        m.validate().unwrap();
        let t = Topology::build(&m);
        assert_eq!(t.euler_characteristic(&m), 1, "continua um disco");
        assert_eq!(t.non_manifold_edge_count(), 0);
        let boundary_after: Vec<Vec3> = (0..m.vert_count() as u32)
            .filter(|&v| is_open_boundary_vert(&t, v))
            .map(|v| m.co(v))
            .collect();
        assert_eq!(boundary_before.len(), boundary_after.len());
        for p in &boundary_before {
            assert!(boundary_after.contains(p), "fronteira mudou em {p:?}");
        }
        // planar: nenhum vértice saiu do plano
        assert!(m.verts.iter().all(|v| v.co.y.abs() < 1e-6));
    }

    #[test]
    fn open_mesh_boundary_edges_can_be_split() {
        let mut m = primitives::grid(2, 2, 2.0, 2.0);
        let params = DyntopoParams {
            collapse: false,
            max_iterations: 3,
            ..Default::default()
        };
        let r = step_all(&mut m, 0.4, &params).unwrap();
        assert!(r.splits > 0);
        let t = Topology::build(&m);
        assert_eq!(t.euler_characteristic(&m), 1);
        assert_eq!(t.non_manifold_edge_count(), 0);
        assert!((area(&m) - 4.0).abs() < 1e-4);
    }

    #[test]
    fn collapse_on_boundary_is_possible_when_not_preserved() {
        let mut m = primitives::grid(10, 10, 2.0, 2.0);
        let params = DyntopoParams {
            subdivide: false,
            preserve_boundary: false,
            max_iterations: 6,
            ..Default::default()
        };
        let r = step_all(&mut m, 0.7, &params).unwrap();
        assert!(r.collapses > 0);
        m.validate().unwrap();
        let t = Topology::build(&m);
        assert_eq!(t.euler_characteristic(&m), 1);
        assert_eq!(t.non_manifold_edge_count(), 0);
        // nenhuma face invertida
        for f in 0..m.face_count() as u32 {
            assert!(m.face_normal(f).y > 0.0, "face {f} invertida");
        }
    }

    #[test]
    fn attributes_are_interpolated_on_split() {
        // dois triângulos formando um quad; aresta diagonal longa compartilhada
        let mut m = EditMesh::new("t");
        let a = m.add_vert(Vec3::new(0.0, 0.0, 0.0));
        let b = m.add_vert(Vec3::new(4.0, 0.0, 0.0));
        let c = m.add_vert(Vec3::new(1.0, 0.0, 1.0));
        let d = m.add_vert(Vec3::new(3.0, 0.0, -1.0));
        m.verts[a as usize].color = [0.0, 0.0, 0.0, 1.0];
        m.verts[b as usize].color = [1.0, 1.0, 1.0, 1.0];
        m.verts[a as usize].mask = 0.0;
        m.verts[b as usize].mask = 0.4;
        let f0 = m.add_face(&[a, c, b]).unwrap();
        let f1 = m.add_face(&[a, b, d]).unwrap();
        for (f, uvs) in [
            (f0, [glam::Vec2::new(0.0, 0.0), glam::Vec2::new(0.0, 1.0), glam::Vec2::new(1.0, 0.0)]),
            (f1, [glam::Vec2::new(0.0, 0.0), glam::Vec2::new(1.0, 0.0), glam::Vec2::new(1.0, -1.0)]),
        ] {
            for (cn, uv) in m.faces[f as usize].corners.iter_mut().zip(uvs) {
                cn.uv = uv;
            }
        }
        m.faces[f0 as usize].material = 3;
        m.faces[f1 as usize].face_set = 9;
        m.faces[f1 as usize].corners[0].normal = Some(Vec3::Y);
        m.faces[f1 as usize].corners[1].normal = Some(Vec3::Y);
        m.set_seam(a, b, true);
        m.set_crease(a, b, 0.5);
        let params = DyntopoParams {
            collapse: false,
            max_iterations: 1,
            split_ratio: 1.0,
            ..Default::default()
        };
        // só a diagonal a-b (4.0) passa de 3.5 (as demais arestas medem ≤ 3.17)
        let r = dyntopo_step(&mut m, &[a, b], 3.5, &params).unwrap();
        assert_eq!(r.splits, 1);
        assert_eq!(m.vert_count(), 5);
        assert_eq!(m.face_count(), 4);
        let mid = 4u32;
        assert_eq!(m.co(mid), Vec3::new(2.0, 0.0, 0.0));
        assert_eq!(m.verts[mid as usize].color, [0.5, 0.5, 0.5, 1.0]);
        assert!((m.verts[mid as usize].mask - 0.2).abs() < 1e-6);
        // UV do corner novo = média dos extremos em cada face
        for f in &m.faces {
            for cn in &f.corners {
                if cn.vert == mid {
                    // as faces originais tinham a-b com uv (0,0)-(1,0)
                    assert_eq!(cn.uv, glam::Vec2::new(0.5, 0.0));
                }
            }
        }
        // atributos de face herdados pelas duas metades
        assert_eq!(m.faces.iter().filter(|f| f.material == 3).count(), 2);
        assert_eq!(m.faces.iter().filter(|f| f.face_set == 9).count(), 2);
        // normal customizada interpolada onde ambos os extremos tinham
        let with_normal: Vec<&Corner> = m
            .faces
            .iter()
            .flat_map(|f| f.corners.iter())
            .filter(|c| c.vert == mid && c.normal.is_some())
            .collect();
        assert_eq!(with_normal.len(), 2);
        assert!(with_normal.iter().all(|c| (c.normal.unwrap() - Vec3::Y).length() < 1e-6));
        // costura/vinco propagados às duas metades, aresta antiga removida
        assert!(m.edge_attr(a, mid).seam && m.edge_attr(mid, b).seam);
        assert_eq!(m.edge_attr(a, mid).crease, 0.5);
        assert!(!m.edge_attr(a, b).seam);
        let t = Topology::build(&m);
        assert_eq!(t.non_manifold_edge_count(), 0);
        assert_eq!(t.euler_characteristic(&m), 1);
        m.validate().unwrap();
    }

    #[test]
    fn collapse_preserves_feature_edges_and_masked_or_hidden_vertices() {
        let build = || {
            let mut m = primitives::quad_sphere(1.0, 6);
            m.name = "s".into();
            m
        };
        let params = DyntopoParams {
            subdivide: false,
            max_iterations: 6,
            ..Default::default()
        };
        // marca todas as arestas como costura: nada colapsa
        let mut m = build();
        let t = Topology::build(&m);
        for e in &t.edges {
            m.set_seam(e.verts.0, e.verts.1, true);
        }
        let h = m.content_hash();
        let r = step_all(&mut m, 5.0, &params).unwrap();
        assert_eq!(r.collapses, 0);
        assert_eq!(m.content_hash(), h);
        // máscara protege
        let mut m = build();
        for v in m.verts.iter_mut() {
            v.mask = 1.0;
        }
        let r = step_all(&mut m, 5.0, &params).unwrap();
        assert!(!r.changed());
        // sem respeitar a máscara colapsa
        let free = DyntopoParams {
            respect_mask: false,
            ..params.clone()
        };
        let r = step_all(&mut m, 5.0, &free).unwrap();
        assert!(r.collapses > 0);
        // ocultos protegem
        let mut m = build();
        for v in m.verts.iter_mut() {
            v.hidden = true;
        }
        assert!(!step_all(&mut m, 5.0, &params).unwrap().changed());
        let mut m = build();
        for f in m.faces.iter_mut() {
            f.hidden = true;
        }
        assert!(!step_all(&mut m, 5.0, &params).unwrap().changed());
    }

    #[test]
    fn collapse_remaps_edge_attributes() {
        let mut m = primitives::quad_sphere(1.0, 6);
        let t = Topology::build(&m);
        // marca sharp só em um "equador" (arestas com y ≈ 0)
        let mut marked = 0;
        for e in &t.edges {
            if m.co(e.verts.0).y.abs() < 1e-5 && m.co(e.verts.1).y.abs() < 1e-5 {
                m.set_sharp(e.verts.0, e.verts.1, true);
                marked += 1;
            }
        }
        assert!(marked > 8);
        let params = DyntopoParams {
            subdivide: false,
            max_iterations: 6,
            ..Default::default()
        };
        step_all(&mut m, 3.0, &params).unwrap();
        m.validate().unwrap();
        // atributos continuam apontando para arestas existentes
        let t = Topology::build(&m);
        for (a, b) in m.edge_attrs.keys() {
            assert!(t.edge(*a, *b).is_some(), "atributo de aresta órfão ({a},{b})");
        }
        assert_eq!(m.edge_attrs.len(), marked, "arestas de feature não colapsam");
    }

    #[test]
    fn loose_vertices_survive_compaction() {
        let mut m = primitives::quad_sphere(1.0, 6);
        let loose = m.add_vert(Vec3::new(9.0, 9.0, 9.0));
        let params = DyntopoParams {
            subdivide: false,
            max_iterations: 4,
            ..Default::default()
        };
        let r = step_all(&mut m, 3.0, &params).unwrap();
        assert!(r.collapses > 0);
        let new_idx = r.remap.as_ref().unwrap().vert[loose as usize].expect("vértice solto mantido");
        assert_eq!(m.co(new_idx), Vec3::new(9.0, 9.0, 9.0));
    }

    #[test]
    fn non_manifold_edges_are_skipped() {
        // três triângulos compartilhando uma aresta longa
        let mut m = EditMesh::new("nm");
        let a = m.add_vert(Vec3::ZERO);
        let b = m.add_vert(Vec3::new(4.0, 0.0, 0.0));
        let c = m.add_vert(Vec3::new(2.0, 1.0, 0.0));
        let d = m.add_vert(Vec3::new(2.0, 0.0, 1.0));
        let e = m.add_vert(Vec3::new(2.0, -1.0, 0.0));
        m.add_face(&[a, b, c]).unwrap();
        m.add_face(&[a, b, d]).unwrap();
        m.add_face(&[a, b, e]).unwrap();
        let before = m.clone();
        let params = DyntopoParams {
            collapse: false,
            ..Default::default()
        };
        let r = dyntopo_step(&mut m, &[a, b], 3.0, &params).unwrap();
        // a aresta a-b (4.0) é não-manifold: só as arestas a-c etc. (curtas) existem
        assert_eq!(r.splits, 0);
        assert_eq!(m, before);
    }

    #[test]
    fn invalid_input_leaves_mesh_intact() {
        let mut m = primitives::quad_sphere(1.0, 3);
        let before = m.clone();
        let all = all_verts(&m);
        let p = DyntopoParams::default();
        assert!(matches!(dyntopo_step(&mut m, &all, 0.0, &p), Err(ModelError::InvalidParameter(_))));
        assert!(dyntopo_step(&mut m, &all, f32::NAN, &p).is_err());
        assert!(dyntopo_step(&mut m, &all, -1.0, &p).is_err());
        assert!(matches!(dyntopo_step(&mut m, &[], 0.1, &p), Err(ModelError::EmptySelection(_))));
        assert!(matches!(
            dyntopo_step(&mut m, &[9999], 0.1, &p),
            Err(ModelError::VertexOutOfRange(9999, _))
        ));
        for bad in [
            DyntopoParams {
                split_ratio: 0.0,
                ..Default::default()
            },
            DyntopoParams {
                collapse_ratio: -1.0,
                ..Default::default()
            },
            DyntopoParams {
                collapse_ratio: 0.6,
                ..Default::default()
            },
            DyntopoParams {
                max_iterations: 0,
                ..Default::default()
            },
            DyntopoParams {
                split_ratio: f32::NAN,
                ..Default::default()
            },
        ] {
            assert!(dyntopo_step(&mut m, &all, 0.1, &bad).is_err());
        }
        assert_eq!(m, before);
        // malha inválida
        let mut broken = before.clone();
        broken.verts[0].co = Vec3::NAN;
        assert!(dyntopo_step(&mut broken, &all, 0.1, &p).is_err());
    }

    #[test]
    fn op_budget_and_iteration_limit_are_reported() {
        let mut m = primitives::quad_sphere(1.0, 4);
        let params = DyntopoParams {
            collapse: false,
            max_edge_ops: 10,
            max_iterations: 10,
            ..Default::default()
        };
        let r = step_all(&mut m, 0.05, &params).unwrap();
        assert_eq!(r.splits, 10);
        assert!(r.hit_limit);
        assert_closed_sphere(&m);

        let mut m = primitives::quad_sphere(1.0, 4);
        let params = DyntopoParams {
            collapse: false,
            max_iterations: 1,
            ..Default::default()
        };
        let r = step_all(&mut m, 0.05, &params).unwrap();
        assert!(r.hit_limit && r.iterations == 1);
        assert_closed_sphere(&m);
    }

    #[test]
    fn dyntopo_is_deterministic() {
        let run = || {
            let mut m = primitives::quad_sphere(1.0, 5);
            let region: Vec<u32> = (0..m.vert_count() as u32).filter(|&v| m.co(v).x > 0.0).collect();
            dyntopo_step(&mut m, &region, 0.15, &DyntopoParams::default()).unwrap();
            step_all(&mut m, 0.3, &DyntopoParams::default()).unwrap();
            m.content_hash()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn works_on_meshes_with_ngons_and_poles() {
        for mut m in [primitives::uv_sphere(1.0, 8, 12), primitives::capsule(0.5, 1.0, 12, 3)] {
            let v0 = m.vert_count();
            let params = DyntopoParams {
                max_iterations: 5,
                ..Default::default()
            };
            let r = step_all(&mut m, 0.25, &params).unwrap();
            assert!(r.splits > 0 || r.collapses > 0, "{}", m.name);
            let t = Topology::build(&m);
            assert!(t.is_closed_manifold(), "{} fechada", m.name);
            assert_eq!(t.euler_characteristic(&m), 2);
            assert!(m.vert_count() != v0);
            m.validate().unwrap();
        }
        // cilindro com tampas n-gon (triangulação de n-gon)
        let mut m = primitives::cylinder(0.5, 1.0, 10, 2, true);
        let params = DyntopoParams {
            collapse: false,
            ..Default::default()
        };
        let r = step_all(&mut m, 0.3, &params).unwrap();
        assert!(r.triangulated_faces > 0);
        let t = Topology::build(&m);
        assert!(t.is_closed_manifold());
        assert_eq!(t.euler_characteristic(&m), 2);
    }

    #[test]
    fn property_random_regions_and_details_keep_manifold() {
        let mut rng = crate::bvh::test_util::Rng(31);
        let mut m = primitives::quad_sphere(1.0, 5);
        for _ in 0..12 {
            let c = rng.unit_vec();
            let r = rng.range(0.3, 1.2);
            let region: Vec<u32> = (0..m.vert_count() as u32).filter(|&v| m.co(v).distance(c) < r).collect();
            if region.is_empty() {
                continue;
            }
            let detail = rng.range(0.08, 0.5);
            let rep = dyntopo_step(&mut m, &region, detail, &DyntopoParams::default()).unwrap();
            assert!(rep.region.iter().all(|&v| (v as usize) < m.vert_count()));
            assert_closed_sphere(&m);
        }
    }

    #[test]
    fn perf_dyntopo_step_on_large_mesh() {
        let mut m = primitives::quad_sphere(1.0, 40);
        let region: Vec<u32> = (0..m.vert_count() as u32).filter(|&v| m.co(v).y > 0.9).collect();
        let t0 = std::time::Instant::now();
        let detail = avg_edge(&m) * 0.5;
        let r = dyntopo_step(&mut m, &region, detail, &DyntopoParams::default()).unwrap();
        println!(
            "[dyntopo perf] {} -> {} faces, {} splits, {} colapsos em {:?}",
            r.faces_before,
            r.faces_after,
            r.splits,
            r.collapses,
            t0.elapsed()
        );
        assert!(r.splits > 0);
        assert_closed_sphere(&m);
    }
}
