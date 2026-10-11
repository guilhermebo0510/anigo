//! Matemática de transformação do Edit Mode: translação/rotação/escala com pivô e
//! orientação, **Proportional Editing**, snapping e mapa de simetria.
//!
//! A UI (gizmos, mouse) vive em `anigo-studio`; aqui está só o que é testável:
//! dado o delta/ângulo/escala, mover os vértices certos, com pesos de falloff
//! proporcional (`O` no Blender) e os helpers de snap (`Shift` do gizmo).
//!
//! Funções puras auxiliares (`snap_to_grid`, `proportional_weights`,
//! `mirror_vertex_map`, …) devolvem valores simples e tratam parâmetros inválidos de
//! forma defensiva e documentada (sem `Result`), para uso direto no laço de
//! interação.

use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, EdgeKey, EditMesh};
use crate::ops::delete::unique_verts;
use crate::ops::merge::PointGrid;
use crate::ops::smooth::XorShift64;
use crate::ops::OpOutput;
use crate::select::Selection;
use crate::topology::Topology;
use glam::{Mat3, Quat, Vec3};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

// ------------------------------------------------------------------------------ pivô

/// Ponto fixo da rotação/escala (Blender: *Transform Pivot Point*).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pivot {
    /// Média das posições dos vértices.
    MedianPoint,
    /// Centro da caixa envolvente (AABB) dos vértices.
    BoundingBoxCenter,
    /// Posição de um vértice específico (o "ativo"; não precisa estar na lista).
    ActiveElement(u32),
    /// Cursor 3D.
    Cursor(Vec3),
}

/// Resolve o pivô para uma posição. Erros: lista vazia/inválida, vértice ativo fora
/// do intervalo, cursor não finito.
pub fn pivot_point(mesh: &EditMesh, verts: &[u32], pivot: Pivot) -> ModelResult<Vec3> {
    let sel = unique_verts(mesh, verts, "vértices")?;
    match pivot {
        Pivot::MedianPoint => {
            Ok(sel.iter().map(|&v| mesh.co(v)).sum::<Vec3>() / sel.len() as f32)
        }
        Pivot::BoundingBoxCenter => {
            let first = mesh.co(sel[0]);
            let (lo, hi) = sel
                .iter()
                .fold((first, first), |(lo, hi), &v| (lo.min(mesh.co(v)), hi.max(mesh.co(v))));
            Ok((lo + hi) * 0.5)
        }
        Pivot::ActiveElement(v) => {
            if v as usize >= mesh.verts.len() {
                return Err(ModelError::VertexOutOfRange(v, mesh.verts.len()));
            }
            Ok(mesh.co(v))
        }
        Pivot::Cursor(c) => {
            if !c.is_finite() {
                return Err(ModelError::InvalidParameter("cursor não finito".into()));
            }
            Ok(c)
        }
    }
}

fn result(sel: &[u32]) -> OpOutput {
    OpOutput {
        selection: Selection::from_verts(sel.iter().copied()),
        ..Default::default()
    }
}

/// Aplica `f` às normais customizadas dos corners cujo vértice está em `set`.
fn map_corner_normals(mesh: &mut EditMesh, set: &[bool], f: impl Fn(Vec3) -> Vec3) {
    for face in &mut mesh.faces {
        for c in &mut face.corners {
            if set[c.vert as usize] {
                if let Some(n) = c.normal.as_mut() {
                    *n = f(*n).try_normalize().unwrap_or(*n);
                }
            }
        }
    }
}

fn membership(mesh: &EditMesh, sel: &[u32]) -> Vec<bool> {
    let mut set = vec![false; mesh.verts.len()];
    for &v in sel {
        set[v as usize] = true;
    }
    set
}

/// Translada os vértices por `delta`. Seleção devolvida: `verts`. Erros: seleção
/// vazia/inválida, `delta` não finito.
pub fn translate_verts(mesh: &mut EditMesh, verts: &[u32], delta: Vec3) -> ModelResult<OpOutput> {
    let sel = unique_verts(mesh, verts, "vértices")?;
    if !delta.is_finite() {
        return Err(ModelError::InvalidParameter("delta não finito".into()));
    }
    for &v in &sel {
        mesh.verts[v as usize].co += delta;
    }
    Ok(result(&sel))
}

/// Rotaciona os vértices `angle` radianos (regra da mão direita) em torno do eixo
/// `axis` (normalizado internamente) passando pelo pivô. Normais customizadas dos
/// corners dos vértices movidos giram junto. Seleção devolvida: `verts`. Erros:
/// seleção/pivô inválidos, eixo nulo ou não finito, ângulo não finito.
pub fn rotate_verts(
    mesh: &mut EditMesh,
    verts: &[u32],
    pivot: Pivot,
    axis: Vec3,
    angle: f32,
) -> ModelResult<OpOutput> {
    let sel = unique_verts(mesh, verts, "vértices")?;
    let axis = axis
        .try_normalize()
        .ok_or_else(|| ModelError::InvalidParameter("eixo de rotação nulo ou não finito".into()))?;
    if !angle.is_finite() {
        return Err(ModelError::InvalidParameter("ângulo não finito".into()));
    }
    let p0 = pivot_point(mesh, &sel, pivot)?;
    let q = Quat::from_axis_angle(axis, angle);
    let set = membership(mesh, &sel);
    for &v in &sel {
        let p = mesh.co(v);
        mesh.verts[v as usize].co = p0 + q * (p - p0);
    }
    map_corner_normals(mesh, &set, |n| q * n);
    Ok(result(&sel))
}

/// Escala os vértices em torno do pivô, por `scale` (por eixo) **na orientação**
/// dada: `orientation` é uma matriz cujas colunas são os eixos locais (use
/// `Mat3::IDENTITY` para eixos globais). `p' = pivô + O · diag(scale) · Oᵀ · (p − pivô)`.
/// Normais customizadas dos corners transformam pela inversa transposta (e são
/// renormalizadas). Seleção devolvida: `verts`.
///
/// Erros: seleção/pivô inválidos, `scale` não finito, orientação não finita ou
/// singular (|det| < 1e-9).
pub fn scale_verts(
    mesh: &mut EditMesh,
    verts: &[u32],
    pivot: Pivot,
    scale: Vec3,
    orientation: Mat3,
) -> ModelResult<OpOutput> {
    let sel = unique_verts(mesh, verts, "vértices")?;
    if !scale.is_finite() {
        return Err(ModelError::InvalidParameter("escala não finita".into()));
    }
    if !orientation.to_cols_array().iter().all(|x| x.is_finite())
        || orientation.determinant().abs() < 1e-9
    {
        return Err(ModelError::InvalidParameter(
            "orientação não finita ou singular".into(),
        ));
    }
    let p0 = pivot_point(mesh, &sel, pivot)?;
    let m = orientation * Mat3::from_diagonal(scale) * orientation.inverse();
    let set = membership(mesh, &sel);
    for &v in &sel {
        let p = mesh.co(v);
        mesh.verts[v as usize].co = p0 + m * (p - p0);
    }
    if m.determinant().abs() > 1e-12 {
        let nm = m.inverse().transpose();
        map_corner_normals(mesh, &set, |n| nm * n);
    }
    Ok(result(&sel))
}

// ------------------------------------------------------------ proportional editing

/// Curvas de falloff do Proportional Editing (idênticas às do Blender). Com
/// `t = distância / raio` e `d = 1 − t`:
/// * `Smooth`: `3d² − 2d³`;
/// * `Sphere`: `√(2d − d²)`;
/// * `Root`: `√d`;
/// * `InverseSquare`: `d·(2 − d)`;
/// * `Sharp`: `d²`;
/// * `Linear`: `d`;
/// * `Constant`: `1` dentro do raio;
/// * `Random`: `d` multiplicado por um aleatório em `[0, 1)` determinístico por vértice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProportionalFalloff {
    Smooth,
    Sphere,
    Root,
    InverseSquare,
    Sharp,
    Linear,
    Constant,
    Random,
}

/// Semente fixa do falloff `Random` (resultado estável entre chamadas e execuções).
pub const PROPORTIONAL_RANDOM_SEED: u64 = 0x50_52_4F_50;

impl ProportionalFalloff {
    /// Peso para uma razão `distância/raio` em `[0, 1)`; `rand` só é usado por `Random`.
    /// Razões ≥ 1 (ou negativas/NaN) dão 0.
    pub fn eval(self, ratio: f32, rand: f32) -> f32 {
        if !ratio.is_finite() || !(0.0..1.0).contains(&ratio) {
            return 0.0;
        }
        let d = 1.0 - ratio;
        let w = match self {
            ProportionalFalloff::Smooth => 3.0 * d * d - 2.0 * d * d * d,
            ProportionalFalloff::Sphere => (2.0 * d - d * d).max(0.0).sqrt(),
            ProportionalFalloff::Root => d.sqrt(),
            ProportionalFalloff::InverseSquare => d * (2.0 - d),
            ProportionalFalloff::Sharp => d * d,
            ProportionalFalloff::Linear => d,
            ProportionalFalloff::Constant => 1.0,
            ProportionalFalloff::Random => d * rand,
        };
        w.clamp(0.0, 1.0)
    }
}

#[derive(PartialEq)]
struct HeapItem(f32, u32);
impl Eq for HeapItem {}
impl Ord for HeapItem {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap é max-heap: inverte para menor distância primeiro; desempate por índice
        other.0.total_cmp(&self.0).then(other.1.cmp(&self.1))
    }
}
impl PartialOrd for HeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Distâncias geodésicas (ao longo das arestas, Dijkstra multi-fonte) das fontes até
/// no máximo `radius`. Não alcançados = `f32::INFINITY`.
fn geodesic_distances(mesh: &EditMesh, sources: &[u32], radius: f32) -> Vec<f32> {
    let topo = Topology::build(mesh);
    let mut dist = vec![f32::INFINITY; mesh.verts.len()];
    let mut heap = BinaryHeap::new();
    for &s in sources {
        dist[s as usize] = 0.0;
        heap.push(HeapItem(0.0, s));
    }
    while let Some(HeapItem(d, v)) = heap.pop() {
        if d > dist[v as usize] {
            continue;
        }
        for &e in &topo.vert_edges[v as usize] {
            let (a, b) = topo.edges[e as usize].verts;
            let o = if a == v { b } else { a };
            let nd = d + (mesh.co(v) - mesh.co(o)).length();
            if nd < radius && nd < dist[o as usize] {
                dist[o as usize] = nd;
                heap.push(HeapItem(nd, o));
            }
        }
    }
    dist
}

/// Pesos do Proportional Editing: um valor `0..=1` por vértice da malha.
///
/// * vértices em `selected` têm peso 1;
/// * os demais, `falloff.eval(d / radius)` onde `d` é
///   * `connected_only = false`: distância euclidiana ao vértice selecionado mais
///     próximo (grade espacial, O(n));
///   * `connected_only = true`: distância **geodésica** ao longo das arestas (Dijkstra) —
///     ilhas desconexas da seleção recebem 0, não importa quão próximas;
/// * vértices a `radius` ou mais, e (no modo conectado) inalcançáveis, têm peso 0.
///
/// Defensivo: `radius` não finito ou ≤ 0 equivale a raio zero (só a seleção pesa 1);
/// índices fora do intervalo em `selected` são ignorados. `Random` usa
/// [`PROPORTIONAL_RANDOM_SEED`] e o índice do vértice, então é determinístico.
pub fn proportional_weights(
    mesh: &EditMesh,
    selected: &[u32],
    radius: f32,
    falloff: ProportionalFalloff,
    connected_only: bool,
) -> Vec<f32> {
    let nv = mesh.verts.len();
    let mut w = vec![0.0f32; nv];
    let mut sel: Vec<u32> = selected.iter().copied().filter(|&v| (v as usize) < nv).collect();
    sel.sort_unstable();
    sel.dedup();
    for &v in &sel {
        w[v as usize] = 1.0;
    }
    if sel.is_empty() || !radius.is_finite() || radius <= 0.0 {
        return w;
    }
    let dist: Vec<f32> = if connected_only {
        geodesic_distances(mesh, &sel, radius)
    } else {
        let mut grid = PointGrid::new(radius);
        for &s in &sel {
            grid.insert(s, mesh.co(s));
        }
        let mut dist = vec![f32::INFINITY; nv];
        for v in 0..nv {
            let p = mesh.verts[v].co;
            let mut best = f32::INFINITY;
            grid.for_each_near(p, radius, |s| {
                let d = (mesh.co(s) - p).length();
                if d < best {
                    best = d;
                }
            });
            dist[v] = best;
        }
        dist
    };
    for v in 0..nv {
        if w[v] == 1.0 {
            continue;
        }
        let d = dist[v];
        if !d.is_finite() {
            continue;
        }
        let rand = if falloff == ProportionalFalloff::Random {
            XorShift64::for_index(PROPORTIONAL_RANDOM_SEED, v as u64).next_f32()
        } else {
            0.0
        };
        w[v] = falloff.eval(d / radius, rand);
    }
    w
}

fn check_weights(mesh: &EditMesh, weights: &[f32]) -> ModelResult<()> {
    if weights.len() != mesh.verts.len() {
        return Err(ModelError::InvalidParameter(format!(
            "weights tem {} entradas; a malha tem {} vértices",
            weights.len(),
            mesh.verts.len()
        )));
    }
    if weights.iter().any(|w| !w.is_finite() || !(0.0..=1.0).contains(w)) {
        return Err(ModelError::InvalidParameter(
            "weights deve conter valores finitos em [0, 1]".into(),
        ));
    }
    Ok(())
}

/// Aplica uma transformação arbitrária com pesos: `p' = lerp(p, f(p), w)` para todo
/// vértice com `w > 0`. É a base de rotação/escala proporcional (passe um closure
/// que roda/escala em torno do pivô). Erros: tamanho de `weights` diferente do número
/// de vértices ou valores fora de `[0, 1]`/não finitos.
pub fn apply_transform_proportional(
    mesh: &mut EditMesh,
    weights: &[f32],
    f: impl Fn(Vec3) -> Vec3,
) -> ModelResult<OpOutput> {
    check_weights(mesh, weights)?;
    let mut moved = Vec::new();
    for (v, &w) in weights.iter().enumerate() {
        if w > 0.0 {
            let p = mesh.verts[v].co;
            let target = f(p);
            if !target.is_finite() {
                return Err(ModelError::Numeric(
                    "a transformação produziu coordenadas não finitas".into(),
                ));
            }
            moved.push((v, p.lerp(target, w)));
        }
    }
    for (v, p) in moved {
        mesh.verts[v].co = p;
    }
    Ok(OpOutput::default())
}

/// Translação proporcional: cada vértice se move `delta · peso`. `selected` é forçado
/// a peso 1 (mesmo que `weights` diga outra coisa) e é a seleção devolvida. Erros:
/// como [`apply_transform_proportional`], `selected` vazio/inválido, `delta` não finito.
pub fn apply_translate_proportional(
    mesh: &mut EditMesh,
    selected: &[u32],
    delta: Vec3,
    weights: &[f32],
) -> ModelResult<OpOutput> {
    let sel = unique_verts(mesh, selected, "vértices")?;
    if !delta.is_finite() {
        return Err(ModelError::InvalidParameter("delta não finito".into()));
    }
    check_weights(mesh, weights)?;
    let mut w = weights.to_vec();
    for &v in &sel {
        w[v as usize] = 1.0;
    }
    apply_transform_proportional(mesh, &w, |p| p + delta)?;
    Ok(result(&sel))
}

// ------------------------------------------------------------------------ snapping

/// Arredonda um escalar ao múltiplo mais próximo de `step`. `step` ≤ 0 ou não finito
/// devolve `value` sem alteração.
pub fn snap_value(value: f32, step: f32) -> f32 {
    if !step.is_finite() || step <= 0.0 || !value.is_finite() {
        return value;
    }
    (value / step).round() * step
}

/// Snap de uma posição à grade de passo `step` (cada eixo). `step` inválido devolve
/// `p` sem alteração.
pub fn snap_to_grid(p: Vec3, step: f32) -> Vec3 {
    Vec3::new(snap_value(p.x, step), snap_value(p.y, step), snap_value(p.z, step))
}

/// Vértice mais próximo de `point` dentro de `max_distance` (busca linear O(n);
/// para consultas repetidas use [`VertexSnapIndex`]). `exclude` lista vértices que
/// não podem ser alvo (ex.: os que estão sendo arrastados). Empate: menor índice.
pub fn snap_to_nearest_vertex(
    mesh: &EditMesh,
    point: Vec3,
    max_distance: f32,
    exclude: &[u32],
) -> Option<u32> {
    if !point.is_finite() || !max_distance.is_finite() || max_distance < 0.0 {
        return None;
    }
    let mut best: Option<(f32, u32)> = None;
    for (i, v) in mesh.verts.iter().enumerate() {
        let i = i as u32;
        if exclude.contains(&i) {
            continue;
        }
        let d = v.co.distance(point);
        if d <= max_distance && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, i));
        }
    }
    best.map(|(_, i)| i)
}

/// Índice espacial de vértices para snapping interativo (consultas em O(1)
/// amortizado). Guarda uma cópia das posições: reconstrua após editar a malha.
#[derive(Debug, Clone)]
pub struct VertexSnapIndex {
    grid: PointGrid,
    pos: Vec<Vec3>,
}

impl VertexSnapIndex {
    /// `cell` ≈ raio de snap típico (células menores = menos candidatos).
    pub fn build(mesh: &EditMesh, cell: f32) -> Self {
        let mut grid = PointGrid::new(cell);
        let pos: Vec<Vec3> = mesh.verts.iter().map(|v| v.co).collect();
        for (i, p) in pos.iter().enumerate() {
            grid.insert(i as u32, *p);
        }
        Self { grid, pos }
    }

    /// Mesma semântica de [`snap_to_nearest_vertex`].
    pub fn nearest(&self, point: Vec3, max_distance: f32, exclude: &[u32]) -> Option<u32> {
        if !point.is_finite() || !max_distance.is_finite() || max_distance < 0.0 {
            return None;
        }
        let mut best: Option<(f32, u32)> = None;
        self.grid.for_each_near(point, max_distance, |i| {
            if exclude.contains(&i) {
                return;
            }
            let d = self.pos[i as usize].distance(point);
            if d <= max_distance
                && best.is_none_or(|(bd, bi)| d < bd || (d == bd && i < bi))
            {
                best = Some((d, i));
            }
        });
        best.map(|(_, i)| i)
    }
}

/// Ponto da aresta `(a, b)` mais próximo de `p` e o parâmetro `t ∈ [0, 1]`.
pub fn closest_point_on_segment(a: Vec3, b: Vec3, p: Vec3) -> (Vec3, f32) {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 < 1e-30 {
        return (a, 0.0);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (a + ab * t, t)
}

/// Aresta mais próxima de `point` (dentro de `max_distance`) com o ponto projetado
/// nela — snapping a aresta. Busca linear nas arestas da topologia.
pub fn snap_to_nearest_edge(
    mesh: &EditMesh,
    point: Vec3,
    max_distance: f32,
) -> Option<(EdgeKey, Vec3)> {
    if !point.is_finite() || !max_distance.is_finite() || max_distance < 0.0 {
        return None;
    }
    let topo = Topology::build(mesh);
    let mut best: Option<(f32, EdgeKey, Vec3)> = None;
    for e in &topo.edges {
        let (q, _) = closest_point_on_segment(mesh.co(e.verts.0), mesh.co(e.verts.1), point);
        let d = q.distance(point);
        if d <= max_distance && best.is_none_or(|(bd, _, _)| d < bd) {
            best = Some((d, e.verts, q));
        }
    }
    best.map(|(_, k, q)| (k, q))
}

// ----------------------------------------------------------------------- simetria

/// Mapa vértice → vértice espelho ao longo de `axis` (0 = X, 1 = Y, 2 = Z) pelo plano
/// que passa pela origem. Cada posição refletida é buscada em até `epsilon`:
///
/// * vértices no plano (|coordenada| ≤ `epsilon`) mapeiam para si mesmos;
/// * `None` quando não há par simétrico, ou o pareamento não é recíproco (o espelho
///   do espelho não volta ao original), garantindo uma involução nos `Some`.
///
/// Parâmetros inválidos (`axis > 2`, `epsilon` negativo/não finito) devolvem tudo `None`.
/// É a base da simetria do Edit Mode (Mirror X): `translate_verts_symmetric` a usa.
pub fn mirror_vertex_map(mesh: &EditMesh, axis: usize, epsilon: f32) -> Vec<Option<u32>> {
    let nv = mesh.verts.len();
    if axis > 2 || !epsilon.is_finite() || epsilon < 0.0 {
        return vec![None; nv];
    }
    let flip = |p: Vec3| -> Vec3 {
        let mut q = p;
        q[axis] = -q[axis];
        q
    };
    let mut grid = PointGrid::new(epsilon.max(1e-6));
    for (i, v) in mesh.verts.iter().enumerate() {
        grid.insert(i as u32, v.co);
    }
    let mut cand: Vec<Option<u32>> = vec![None; nv];
    for v in 0..nv {
        let p = mesh.verts[v].co;
        if p[axis].abs() <= epsilon {
            cand[v] = Some(v as u32);
            continue;
        }
        let target = flip(p);
        let mut best: Option<(f32, u32)> = None;
        grid.for_each_near(target, epsilon, |u| {
            let d = mesh.co(u).distance(target);
            if d <= epsilon && best.is_none_or(|(bd, bi)| d < bd || (d == bd && u < bi)) {
                best = Some((d, u));
            }
        });
        cand[v] = best.map(|(_, u)| u);
    }
    (0..nv)
        .map(|v| {
            let u = cand[v]?;
            (cand[u as usize] == Some(v as u32)).then_some(u)
        })
        .collect()
}

/// Translação com simetria: move `verts` por `delta` e os espelhos (segundo `map`, de
/// [`mirror_vertex_map`] para o mesmo `axis`) por `delta` refletido. Vértices no plano
/// de simetria (que são o próprio espelho) recebem só a componente permitida: a do
/// eixo espelhado é anulada. Devolve a seleção com os vértices e seus espelhos.
///
/// Erros: seleção vazia/inválida, `axis > 2`, `delta` não finito, `map` com tamanho
/// diferente do número de vértices.
pub fn translate_verts_symmetric(
    mesh: &mut EditMesh,
    verts: &[u32],
    delta: Vec3,
    axis: usize,
    map: &[Option<u32>],
) -> ModelResult<OpOutput> {
    let sel = unique_verts(mesh, verts, "vértices")?;
    if axis > 2 {
        return Err(ModelError::InvalidParameter(format!("axis {axis} fora de 0..=2")));
    }
    if !delta.is_finite() {
        return Err(ModelError::InvalidParameter("delta não finito".into()));
    }
    if map.len() != mesh.verts.len() {
        return Err(ModelError::InvalidParameter(
            "mapa de simetria com tamanho diferente do número de vértices".into(),
        ));
    }
    let mut mirrored_delta = delta;
    mirrored_delta[axis] = -mirrored_delta[axis];
    let mut moved: std::collections::BTreeMap<u32, Vec3> = std::collections::BTreeMap::new();
    for &v in &sel {
        match map[v as usize] {
            Some(m) if m == v => {
                let mut d = delta;
                d[axis] = 0.0;
                moved.insert(v, d);
            }
            Some(m) => {
                moved.entry(v).or_insert(delta);
                moved.entry(m).or_insert(mirrored_delta);
            }
            None => {
                moved.entry(v).or_insert(delta);
            }
        }
    }
    let mut out = result(&sel);
    for (v, d) in moved {
        mesh.verts[v as usize].co += d;
        out.selection.verts.insert(v);
    }
    Ok(out)
}

/// Chave de aresta canônica (re-export de conveniência para chamadores que montam
/// seleções de snap).
pub fn edge_of(a: u32, b: u32) -> EdgeKey {
    edge_key(a, b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    fn all(m: &EditMesh) -> Vec<u32> {
        (0..m.vert_count() as u32).collect()
    }

    fn area(m: &EditMesh) -> f32 {
        (0..m.face_count() as u32).map(|f| m.face_area(f)).sum()
    }

    #[test]
    fn pivots_resolve_to_expected_points() {
        let mut m = EditMesh::new("p");
        for p in [Vec3::ZERO, Vec3::new(4.0, 0.0, 0.0), Vec3::new(0.0, 2.0, 0.0), Vec3::new(1.0, 1.0, 0.0)] {
            m.add_vert(p);
        }
        let sel = [0, 1, 2, 3];
        assert_eq!(pivot_point(&m, &sel, Pivot::MedianPoint).unwrap(), Vec3::new(1.25, 0.75, 0.0));
        assert_eq!(pivot_point(&m, &sel, Pivot::BoundingBoxCenter).unwrap(), Vec3::new(2.0, 1.0, 0.0));
        assert_eq!(pivot_point(&m, &sel, Pivot::ActiveElement(2)).unwrap(), Vec3::new(0.0, 2.0, 0.0));
        assert_eq!(pivot_point(&m, &sel, Pivot::Cursor(Vec3::splat(7.0))).unwrap(), Vec3::splat(7.0));
        assert!(pivot_point(&m, &sel, Pivot::ActiveElement(9)).is_err());
        assert!(pivot_point(&m, &sel, Pivot::Cursor(Vec3::NAN)).is_err());
        assert!(pivot_point(&m, &[], Pivot::MedianPoint).is_err());
        assert!(pivot_point(&m, &[7], Pivot::MedianPoint).is_err());
    }

    #[test]
    fn translate_moves_only_selected() {
        let orig = primitives::cube(1.0);
        let mut m = orig.clone();
        let out = translate_verts(&mut m, &[0, 1, 1], Vec3::new(1.0, 2.0, 3.0)).unwrap();
        assert_eq!(out.selection.verts.len(), 2);
        assert_eq!(m.co(0), orig.co(0) + Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(m.co(2), orig.co(2));
        assert!(translate_verts(&mut m, &[], Vec3::X).is_err());
        assert!(translate_verts(&mut m, &[0], Vec3::NAN).is_err());
        assert!(translate_verts(&mut m, &[88], Vec3::X).is_err());
    }

    #[test]
    fn rotate_about_median_keeps_shape_and_rotates_normals() {
        let mut m = primitives::cube(2.0);
        for f in &mut m.faces {
            for c in &mut f.corners {
                c.normal = Some(Vec3::Y);
            }
        }
        let ids = all(&m);
        let centroid = m.centroid();
        let vol_area = area(&m);
        rotate_verts(&mut m, &ids, Pivot::MedianPoint, Vec3::Z, std::f32::consts::FRAC_PI_2).unwrap();
        assert!((m.centroid() - centroid).length() < 1e-5);
        assert!((area(&m) - vol_area).abs() < 1e-4);
        // cubo girado 90° em Z coincide consigo mesmo: bounds iguais
        let (lo, hi) = m.bounds().unwrap();
        assert!((hi - Vec3::ONE).length() < 1e-5 && (lo + Vec3::ONE).length() < 1e-5);
        // +Y girado 90° em torno de Z => -X
        assert!((m.faces[0].corners[0].normal.unwrap() - (-Vec3::X)).length() < 1e-5);
        // pivô no cursor: um ponto em (1,0,0) gira para (0,1,0) em torno da origem
        let mut p = EditMesh::new("p");
        p.add_vert(Vec3::X);
        rotate_verts(&mut p, &[0], Pivot::Cursor(Vec3::ZERO), Vec3::Z, std::f32::consts::FRAC_PI_2).unwrap();
        assert!((p.co(0) - Vec3::Y).length() < 1e-6);
        // ângulo 0 não altera
        let mut z = primitives::cube(1.0);
        let h = z.content_hash();
        let zi = all(&z);
        rotate_verts(&mut z, &zi, Pivot::MedianPoint, Vec3::Y, 0.0).unwrap();
        assert!(z.verts.iter().zip(primitives::cube(1.0).verts.iter()).all(|(a, b)| (a.co - b.co).length() < 1e-6));
        let _ = h;
    }

    #[test]
    fn rotate_errors() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        assert!(rotate_verts(&mut m, &[0], Pivot::MedianPoint, Vec3::ZERO, 1.0).is_err());
        assert!(rotate_verts(&mut m, &[0], Pivot::MedianPoint, Vec3::NAN, 1.0).is_err());
        assert!(rotate_verts(&mut m, &[0], Pivot::MedianPoint, Vec3::Y, f32::NAN).is_err());
        assert!(rotate_verts(&mut m, &[0], Pivot::ActiveElement(100), Vec3::Y, 1.0).is_err());
        assert!(rotate_verts(&mut m, &[], Pivot::MedianPoint, Vec3::Y, 1.0).is_err());
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn scale_global_local_and_pivots() {
        let mut m = primitives::cube(2.0);
        let ids = all(&m);
        scale_verts(&mut m, &ids, Pivot::MedianPoint, Vec3::new(2.0, 1.0, 0.5), Mat3::IDENTITY).unwrap();
        let (lo, hi) = m.bounds().unwrap();
        assert!((hi - Vec3::new(2.0, 1.0, 0.5)).length() < 1e-5);
        assert!((lo + Vec3::new(2.0, 1.0, 0.5)).length() < 1e-5);
        // escala local: orientação girada 45° em Z; escalar X local por 2 estica na diagonal
        let rot = Mat3::from_rotation_z(std::f32::consts::FRAC_PI_4);
        let mut p = EditMesh::new("p");
        p.add_vert(Vec3::new(1.0, 1.0, 0.0));
        p.add_vert(Vec3::new(-1.0, -1.0, 0.0));
        scale_verts(&mut p, &[0, 1], Pivot::MedianPoint, Vec3::new(2.0, 1.0, 1.0), rot).unwrap();
        assert!((p.co(0) - Vec3::new(2.0, 2.0, 0.0)).length() < 1e-5);
        // escala em torno do vértice ativo mantém-no fixo
        let mut c = primitives::cube(2.0);
        let anchor = c.co(0);
        let ids = all(&c);
        scale_verts(&mut c, &ids, Pivot::ActiveElement(0), Vec3::splat(3.0), Mat3::IDENTITY).unwrap();
        assert!((c.co(0) - anchor).length() < 1e-6);
        // escala 0 achata sem NaN
        let mut f = primitives::cube(2.0);
        let ids = all(&f);
        scale_verts(&mut f, &ids, Pivot::MedianPoint, Vec3::new(1.0, 0.0, 1.0), Mat3::IDENTITY).unwrap();
        assert!(f.verts.iter().all(|v| v.co.is_finite() && v.co.y.abs() < 1e-6));
        f.validate().unwrap();
    }

    #[test]
    fn scale_updates_custom_normals_with_inverse_transpose() {
        let mut m = primitives::plane(2.0);
        for c in &mut m.faces[0].corners {
            c.normal = Some(Vec3::new(1.0, 1.0, 0.0).normalize());
        }
        let ids = all(&m);
        scale_verts(&mut m, &ids, Pivot::MedianPoint, Vec3::new(2.0, 1.0, 1.0), Mat3::IDENTITY).unwrap();
        let n = m.faces[0].corners[0].normal.unwrap();
        // inversa transposta de diag(2,1,1): x/2, y => (0.5, 1) normalizado
        let expected = Vec3::new(0.5, 1.0, 0.0).normalize();
        assert!((n - expected).length() < 1e-5);
    }

    #[test]
    fn scale_errors() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        assert!(scale_verts(&mut m, &[0], Pivot::MedianPoint, Vec3::NAN, Mat3::IDENTITY).is_err());
        assert!(scale_verts(&mut m, &[0], Pivot::MedianPoint, Vec3::ONE, Mat3::ZERO).is_err());
        assert!(scale_verts(&mut m, &[0], Pivot::MedianPoint, Vec3::ONE, Mat3::from_diagonal(Vec3::NAN)).is_err());
        assert!(scale_verts(&mut m, &[], Pivot::MedianPoint, Vec3::ONE, Mat3::IDENTITY).is_err());
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn falloff_curves_have_blender_endpoints_and_are_monotonic() {
        use ProportionalFalloff::*;
        for f in [Smooth, Sphere, Root, InverseSquare, Sharp, Linear, Constant] {
            assert!((f.eval(0.0, 0.0) - 1.0).abs() < 1e-6, "{f:?}");
            assert_eq!(f.eval(1.0, 0.0), 0.0, "{f:?}");
            assert_eq!(f.eval(1.5, 0.0), 0.0);
            assert_eq!(f.eval(f32::NAN, 0.0), 0.0);
            assert_eq!(f.eval(-0.1, 0.0), 0.0);
            let mut prev = f32::MAX;
            for i in 0..100 {
                let w = f.eval(i as f32 / 100.0, 0.0);
                assert!((0.0..=1.0).contains(&w));
                assert!(w <= prev + 1e-6, "{f:?} não monótona em {i}");
                prev = w;
            }
        }
        // valores conhecidos em t = 0.5 (d = 0.5)
        assert!((Smooth.eval(0.5, 0.0) - 0.5).abs() < 1e-6);
        assert!((Linear.eval(0.5, 0.0) - 0.5).abs() < 1e-6);
        assert!((Sharp.eval(0.5, 0.0) - 0.25).abs() < 1e-6);
        assert!((Root.eval(0.5, 0.0) - 0.5f32.sqrt()).abs() < 1e-6);
        assert!((Sphere.eval(0.5, 0.0) - (0.75f32).sqrt()).abs() < 1e-6);
        assert!((InverseSquare.eval(0.5, 0.0) - 0.75).abs() < 1e-6);
        assert_eq!(Constant.eval(0.99, 0.0), 1.0);
        assert!((Random.eval(0.5, 0.4) - 0.2).abs() < 1e-6);
    }

    #[test]
    fn euclidean_weights_decay_with_distance() {
        let m = primitives::grid(20, 1, 20.0, 1.0);
        let origin = (0..m.vert_count() as u32).min_by(|&a, &b| m.co(a).x.total_cmp(&m.co(b).x).then(m.co(a).z.total_cmp(&m.co(b).z))).unwrap();
        for f in [ProportionalFalloff::Smooth, ProportionalFalloff::Linear, ProportionalFalloff::Sphere, ProportionalFalloff::Root, ProportionalFalloff::Sharp, ProportionalFalloff::InverseSquare] {
            let w = proportional_weights(&m, &[origin], 5.0, f, false);
            assert_eq!(w.len(), m.vert_count());
            assert_eq!(w[origin as usize], 1.0);
            assert!(w.iter().all(|x| (0.0..=1.0).contains(x)));
            // peso depende só da distância: ordenando por distância os pesos são não-crescentes
            let mut pairs: Vec<(f32, f32)> = (0..m.vert_count()).map(|v| (m.co(v as u32).distance(m.co(origin)), w[v])).collect();
            pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
            for p in pairs.windows(2) {
                assert!(p[1].1 <= p[0].1 + 1e-6, "{f:?}");
            }
            // fora do raio = 0
            for (d, wt) in pairs {
                if d >= 5.0 {
                    assert_eq!(wt, 0.0);
                }
            }
        }
        // constante: 1 dentro do raio
        let w = proportional_weights(&m, &[origin], 5.0, ProportionalFalloff::Constant, false);
        assert!(w.iter().enumerate().all(|(v, &x)| (m.co(v as u32).distance(m.co(origin)) < 5.0) == (x == 1.0)));
    }

    #[test]
    fn connected_only_does_not_leak_between_islands() {
        // dois cubos a 1.2 de distância (superfícies separadas por 0.2): raio 1.0 euclidiano
        // alcança o vizinho, o modo conectado não
        let mut m = primitives::cube(1.0);
        let mut other = primitives::cube(1.0);
        other.transform_positions(|p| p + Vec3::new(1.2, 0.0, 0.0));
        let n0 = m.vert_count() as u32;
        m.append(&other);
        let sel = [0u32];
        let euclid = proportional_weights(&m, &sel, 1.5, ProportionalFalloff::Linear, false);
        let conn = proportional_weights(&m, &sel, 1.5, ProportionalFalloff::Linear, true);
        let leak_e = (n0..m.vert_count() as u32).any(|v| euclid[v as usize] > 0.0);
        let leak_c = (n0..m.vert_count() as u32).any(|v| conn[v as usize] > 0.0);
        assert!(leak_e, "euclidiano deveria atingir o cubo vizinho");
        assert!(!leak_c, "conectado não pode vazar entre ilhas");
        // dentro da mesma ilha o geodésico é >= euclidiano, logo pesos conectados <= euclidianos
        for v in 0..n0 as usize {
            assert!(conn[v] <= euclid[v] + 1e-5);
        }
        assert!(conn[1] > 0.0);
    }

    #[test]
    fn geodesic_distance_follows_the_surface() {
        // tira dobrada em U: pontos opostos do U estão perto no espaço, longe na superfície
        let mut m = EditMesh::new("u");
        let n = 20;
        for i in 0..=n {
            let t = i as f32 / n as f32 * std::f32::consts::PI;
            for z in [0.0, 1.0] {
                m.add_vert(Vec3::new(t.cos() * 0.5, t.sin() * 0.5 + 0.0, z));
            }
        }
        for i in 0..n {
            let a = (i * 2) as u32;
            m.add_face(&[a, a + 2, a + 3, a + 1]).unwrap();
        }
        let first = 0u32;
        let last = (n * 2) as u32;
        let euclid = proportional_weights(&m, &[first], 1.5, ProportionalFalloff::Linear, false);
        let conn = proportional_weights(&m, &[first], 1.5, ProportionalFalloff::Linear, true);
        assert!(euclid[last as usize] > 0.0);
        // arco = π·0.5 ≈ 1.57 > 1.5: inalcançável pelo geodésico
        assert_eq!(conn[last as usize], 0.0);
    }

    #[test]
    fn weights_edge_cases() {
        let m = primitives::cube(1.0);
        // raio inválido: só a seleção
        for r in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let w = proportional_weights(&m, &[0, 99], r, ProportionalFalloff::Smooth, false);
            assert_eq!(w.iter().filter(|&&x| x == 1.0).count(), 1);
            assert_eq!(w.iter().filter(|&&x| x > 0.0 && x < 1.0).count(), 0);
        }
        // seleção vazia: tudo 0
        let w = proportional_weights(&m, &[], 5.0, ProportionalFalloff::Smooth, true);
        assert!(w.iter().all(|&x| x == 0.0));
        // malha vazia
        assert!(proportional_weights(&EditMesh::new("e"), &[0], 1.0, ProportionalFalloff::Linear, true).is_empty());
    }

    #[test]
    fn random_falloff_is_deterministic_and_bounded_by_linear() {
        let m = primitives::grid(10, 10, 10.0, 10.0);
        let a = proportional_weights(&m, &[0], 6.0, ProportionalFalloff::Random, false);
        let b = proportional_weights(&m, &[0], 6.0, ProportionalFalloff::Random, false);
        assert_eq!(a, b);
        let lin = proportional_weights(&m, &[0], 6.0, ProportionalFalloff::Linear, false);
        assert!(a.iter().zip(lin.iter()).all(|(r, l)| *r <= *l + 1e-6));
        assert!(a.iter().zip(lin.iter()).any(|(r, l)| *r < *l * 0.9));
    }

    #[test]
    fn apply_translate_proportional_blends() {
        let mut m = primitives::grid(10, 1, 10.0, 1.0);
        let center = (0..m.vert_count() as u32).min_by(|&a, &b| m.co(a).length().total_cmp(&m.co(b).length())).unwrap();
        let orig = m.clone();
        let w = proportional_weights(&m, &[center], 3.0, ProportionalFalloff::Smooth, false);
        let out = apply_translate_proportional(&mut m, &[center], Vec3::Y, &w).unwrap();
        assert_eq!(out.selection.verts.len(), 1);
        assert!((m.co(center).y - 1.0).abs() < 1e-6);
        for v in 0..m.vert_count() {
            let dy = m.verts[v].co.y - orig.verts[v].co.y;
            assert!((dy - w[v]).abs() < 1e-6);
            if orig.verts[v].co.distance(orig.co(center)) >= 3.0 {
                assert_eq!(dy, 0.0);
            }
        }
        // seleção sempre peso 1 mesmo com weights zerados
        let mut z = orig.clone();
        let zeros = vec![0.0; z.vert_count()];
        apply_translate_proportional(&mut z, &[center], Vec3::Y, &zeros).unwrap();
        assert_eq!(z.co(center).y, 1.0);
        assert_eq!(z.co(0).y, 0.0);
    }

    #[test]
    fn apply_transform_proportional_rotates_with_falloff() {
        let mut m = primitives::grid(6, 6, 6.0, 6.0);
        let w = proportional_weights(&m, &[0], 4.0, ProportionalFalloff::Linear, false);
        let q = Quat::from_rotation_y(0.5);
        let orig = m.clone();
        apply_transform_proportional(&mut m, &w, |p| q * p).unwrap();
        for v in 0..m.vert_count() {
            let expected = orig.verts[v].co.lerp(q * orig.verts[v].co, w[v]);
            assert!((m.verts[v].co - expected).length() < 1e-6);
        }
    }

    #[test]
    fn proportional_apply_errors() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        let ok = vec![0.5; m.vert_count()];
        assert!(apply_translate_proportional(&mut m, &[0], Vec3::X, &ok[..3]).is_err());
        let mut bad = ok.clone();
        bad[2] = 2.0;
        assert!(apply_translate_proportional(&mut m, &[0], Vec3::X, &bad).is_err());
        bad[2] = f32::NAN;
        assert!(apply_translate_proportional(&mut m, &[0], Vec3::X, &bad).is_err());
        assert!(apply_translate_proportional(&mut m, &[], Vec3::X, &ok).is_err());
        assert!(apply_translate_proportional(&mut m, &[0], Vec3::NAN, &ok).is_err());
        assert!(apply_transform_proportional(&mut m, &ok, |_| Vec3::NAN).is_err());
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn snapping_helpers() {
        assert_eq!(snap_to_grid(Vec3::new(0.26, -0.74, 1.49), 0.5), Vec3::new(0.5, -0.5, 1.5));
        assert_eq!(snap_to_grid(Vec3::new(0.3, 0.3, 0.3), 0.0), Vec3::new(0.3, 0.3, 0.3));
        assert_eq!(snap_to_grid(Vec3::new(0.3, 0.3, 0.3), -1.0), Vec3::new(0.3, 0.3, 0.3));
        assert_eq!(snap_to_grid(Vec3::new(0.3, 0.3, 0.3), f32::NAN), Vec3::new(0.3, 0.3, 0.3));
        assert_eq!(snap_value(7.0, 5.0), 5.0);
        assert_eq!(snap_value(f32::NAN, 5.0).is_nan(), true);
        let m = primitives::cube(2.0);
        let near = snap_to_nearest_vertex(&m, Vec3::new(1.05, 1.0, 0.95), 0.2, &[]).unwrap();
        assert!((m.co(near) - Vec3::ONE).length() < 1e-5);
        assert!(snap_to_nearest_vertex(&m, Vec3::new(5.0, 5.0, 5.0), 0.2, &[]).is_none());
        // exclusão
        let second = snap_to_nearest_vertex(&m, m.co(near), 3.0, &[near]).unwrap();
        assert_ne!(second, near);
        assert!(snap_to_nearest_vertex(&m, Vec3::NAN, 1.0, &[]).is_none());
        assert!(snap_to_nearest_vertex(&m, Vec3::ZERO, -1.0, &[]).is_none());
        // índice espacial dá o mesmo resultado
        let idx = VertexSnapIndex::build(&m, 0.5);
        for p in [Vec3::new(1.05, 1.0, 0.95), Vec3::new(-0.9, 1.1, -1.0), Vec3::ZERO, Vec3::new(5.0, 5.0, 5.0)] {
            for r in [0.05, 0.3, 1.5, 3.0] {
                assert_eq!(idx.nearest(p, r, &[]), snap_to_nearest_vertex(&m, p, r, &[]), "{p:?} {r}");
            }
        }
        assert_eq!(idx.nearest(m.co(near), 3.0, &[near]), Some(second));
        assert!(idx.nearest(Vec3::NAN, 1.0, &[]).is_none());
    }

    #[test]
    fn edge_snapping_projects_onto_edges() {
        let m = primitives::cube(2.0);
        let (key, q) = snap_to_nearest_edge(&m, Vec3::new(1.2, 1.0, 0.1), 0.5).unwrap();
        assert!((q - Vec3::new(1.0, 1.0, 0.1)).length() < 1e-5);
        assert!(Topology::build(&m).edge(key.0, key.1).is_some());
        assert!(snap_to_nearest_edge(&m, Vec3::new(5.0, 5.0, 5.0), 0.5).is_none());
        assert!(snap_to_nearest_edge(&m, Vec3::NAN, 0.5).is_none());
        let (c, t) = closest_point_on_segment(Vec3::ZERO, Vec3::X, Vec3::new(2.0, 1.0, 0.0));
        assert_eq!((c, t), (Vec3::X, 1.0));
        assert_eq!(closest_point_on_segment(Vec3::X, Vec3::X, Vec3::ZERO), (Vec3::X, 0.0));
        assert_eq!(edge_of(3, 1), (1, 3));
    }

    #[test]
    fn mirror_map_is_an_involution_on_symmetric_meshes() {
        for m in [primitives::quad_sphere(1.0, 6), primitives::uv_sphere(1.0, 8, 12), primitives::cube_subdivided(2.0, 4), primitives::capsule(0.5, 1.0, 8, 3)] {
            for axis in 0..3 {
                let map = mirror_vertex_map(&m, axis, 1e-4);
                assert_eq!(map.len(), m.vert_count());
                for (v, mirror) in map.iter().enumerate() {
                    // toda esfera/cubo centrado é simétrico nos 3 eixos; cápsula e UV sphere em Y também
                    if let Some(u) = *mirror {
                        assert_eq!(map[u as usize], Some(v as u32), "{} eixo {axis}", m.name);
                        let mut p = m.co(v as u32);
                        p[axis] = -p[axis];
                        assert!((p - m.co(u)).length() <= 1e-4 + 1e-6);
                    }
                }
                if m.name == "QuadSphere" || m.name == "Cube" || m.name == "Cube" {
                    assert!(map.iter().all(|x| x.is_some()), "{} eixo {axis}", m.name);
                }
            }
        }
    }

    #[test]
    fn mirror_map_plane_vertices_map_to_themselves_and_asymmetry_gives_none() {
        let mut m = primitives::grid(4, 4, 4.0, 4.0);
        let map = mirror_vertex_map(&m, 0, 1e-4);
        for (v, mm) in map.iter().enumerate() {
            if m.co(v as u32).x.abs() < 1e-6 {
                assert_eq!(*mm, Some(v as u32));
            }
        }
        // desloca um vértice: ele e seu espelho original perdem o par
        let moved = (0..m.vert_count() as u32).find(|&v| m.co(v).x > 0.5).unwrap();
        let old_mirror = map[moved as usize].unwrap();
        m.verts[moved as usize].co.x += 0.3;
        let map2 = mirror_vertex_map(&m, 0, 1e-4);
        assert_eq!(map2[moved as usize], None);
        assert_eq!(map2[old_mirror as usize], None);
        // parâmetros inválidos
        for (a, e) in [(3, 1e-4), (0, -1.0), (0, f32::NAN)] {
            assert!(mirror_vertex_map(&m, a, e).iter().all(|x| x.is_none()));
        }
        assert!(mirror_vertex_map(&EditMesh::new("e"), 0, 1e-4).is_empty());
    }

    #[test]
    fn mirror_map_tolerance_controls_pairing() {
        let mut m = primitives::grid(2, 2, 2.0, 2.0);
        let moved = (0..m.vert_count() as u32).find(|&v| m.co(v).x > 0.5).unwrap();
        m.verts[moved as usize].co.x += 0.01;
        assert_eq!(mirror_vertex_map(&m, 0, 1e-4)[moved as usize], None);
        assert!(mirror_vertex_map(&m, 0, 0.05)[moved as usize].is_some());
    }

    #[test]
    fn symmetric_translate_moves_mirror_counterparts() {
        let mut m = primitives::quad_sphere(1.0, 4);
        let map = mirror_vertex_map(&m, 0, 1e-4);
        let v = (0..m.vert_count() as u32).find(|&v| m.co(v).x > 0.3 && map[v as usize].is_some()).unwrap();
        let mv = map[v as usize].unwrap();
        let (p0, q0) = (m.co(v), m.co(mv));
        let out = translate_verts_symmetric(&mut m, &[v], Vec3::new(0.1, 0.2, 0.3), 0, &map).unwrap();
        assert!((m.co(v) - (p0 + Vec3::new(0.1, 0.2, 0.3))).length() < 1e-6);
        assert!((m.co(mv) - (q0 + Vec3::new(-0.1, 0.2, 0.3))).length() < 1e-6);
        assert!(out.selection.verts.contains(&mv));
        // continua simétrica
        let map2 = mirror_vertex_map(&m, 0, 1e-4);
        assert!(map2[v as usize].is_some());
        // vértice no plano: só a componente permitida
        let on_plane = (0..m.vert_count() as u32).find(|&v| m.co(v).x.abs() < 1e-6).unwrap();
        let before = m.co(on_plane);
        translate_verts_symmetric(&mut m, &[on_plane], Vec3::new(0.5, 0.0, 0.1), 0, &map).unwrap();
        assert!((m.co(on_plane) - (before + Vec3::new(0.0, 0.0, 0.1))).length() < 1e-6);
        // erros
        assert!(translate_verts_symmetric(&mut m, &[], Vec3::X, 0, &map).is_err());
        assert!(translate_verts_symmetric(&mut m, &[v], Vec3::X, 5, &map).is_err());
        assert!(translate_verts_symmetric(&mut m, &[v], Vec3::NAN, 0, &map).is_err());
        assert!(translate_verts_symmetric(&mut m, &[v], Vec3::X, 0, &map[..3]).is_err());
    }

    #[test]
    fn proportional_weights_on_large_mesh_are_fast() {
        let m = primitives::quad_sphere(1.0, 64);
        let t0 = std::time::Instant::now();
        for connected in [false, true] {
            let w = proportional_weights(&m, &[0, 100, 5000], 0.3, ProportionalFalloff::Smooth, connected);
            assert!(w.iter().any(|&x| x > 0.0 && x < 1.0));
        }
        let map = mirror_vertex_map(&m, 0, 1e-4);
        assert!(map.iter().filter(|x| x.is_some()).count() > m.vert_count() * 9 / 10);
        assert!(t0.elapsed().as_secs() < 20, "{:?}", t0.elapsed());
    }
}
