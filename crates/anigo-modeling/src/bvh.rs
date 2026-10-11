//! BVH de triângulos para picking e escultura: raycast, ponto mais próximo,
//! consultas por esfera, refit incremental — mais uma grade hash de vértices
//! ([`VertexGrid`]) e utilidades de picking de câmera.
//!
//! * [`TriBvh`] triangula os n-gons da [`EditMesh`] (via
//!   [`crate::convert::triangulate_polygon`]) e guarda uma **cópia das posições
//!   dos triângulos**, então as consultas não precisam da malha. Depois de mover
//!   vértices (escultura) use [`TriBvh::refit`] ou [`TriBvh::refit_verts`]
//!   (incremental, só os triângulos tocados) — a triangulação é congelada na
//!   construção; mudar a topologia exige reconstruir.
//! * A construção usa SAH com 16 bins no eixo mais longo dos centroides; folhas
//!   têm no máximo [`LEAF_SIZE`] triângulos. A árvore é armazenada em pré-ordem
//!   (filho esquerdo = `i + 1`), o que torna o refit uma varredura reversa.
//! * Todas as consultas são determinísticas.

use crate::convert::triangulate_polygon;
use crate::error::{ModelError, ModelResult};
use crate::mesh::EditMesh;
use glam::{Mat4, Vec2, Vec3};
use std::collections::{BTreeSet, HashMap};

/// Máximo de triângulos por folha.
pub const LEAF_SIZE: usize = 4;
const SAH_BINS: usize = 16;
const MAX_STACK: usize = 128;

// ----------------------------------------------------------------------- AABB

/// Caixa alinhada aos eixos.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    /// Caixa vazia (união neutra).
    pub const EMPTY: Aabb = Aabb {
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
    };

    pub fn from_points(pts: &[Vec3]) -> Aabb {
        let mut b = Aabb::EMPTY;
        for &p in pts {
            b.grow(p);
        }
        b
    }

    #[inline]
    pub fn grow(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    #[inline]
    pub fn union(&self, o: &Aabb) -> Aabb {
        Aabb {
            min: self.min.min(o.min),
            max: self.max.max(o.max),
        }
    }

    #[inline]
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    #[inline]
    pub fn extent(&self) -> Vec3 {
        self.max - self.min
    }

    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x
    }

    /// Área de superfície (custo SAH).
    pub fn surface_area(&self) -> f32 {
        if self.is_empty() {
            return 0.0;
        }
        let e = self.extent();
        2.0 * (e.x * e.y + e.y * e.z + e.z * e.x)
    }

    /// Distância² do ponto à caixa (0 se dentro).
    #[inline]
    pub fn dist_sq(&self, p: Vec3) -> f32 {
        let d = (self.min - p).max(p - self.max).max(Vec3::ZERO);
        d.length_squared()
    }

    #[inline]
    pub fn contains(&self, p: Vec3) -> bool {
        p.cmpge(self.min).all() && p.cmple(self.max).all()
    }

    /// Teste de interseção raio × caixa (slab). Devolve o `t` de entrada
    /// (≥ 0) se o raio atinge a caixa antes de `max_t`.
    #[inline]
    pub fn ray_entry(&self, origin: Vec3, inv_dir: Vec3, max_t: f32) -> Option<f32> {
        let mut enter = 0.0f32;
        let mut exit = max_t;
        for a in 0..3 {
            let (o, inv) = (origin[a], inv_dir[a]);
            if inv.is_infinite() {
                // raio paralelo à laje: dentro dela ignora o eixo, fora não acerta
                if o < self.min[a] || o > self.max[a] {
                    return None;
                }
                continue;
            }
            let t1 = (self.min[a] - o) * inv;
            let t2 = (self.max[a] - o) * inv;
            enter = enter.max(t1.min(t2));
            exit = exit.min(t1.max(t2));
        }
        // folga de arredondamento: o teste é conservador (poda, não decide)
        exit += exit.abs() * 4e-7;
        if enter <= exit {
            Some(enter)
        } else {
            None
        }
    }
}

// -------------------------------------------------------------- geometria tri

/// Ponto mais próximo de `p` no triângulo `abc` (Ericson, *Real-Time Collision
/// Detection* §5.1.5) e suas coordenadas baricêntricas `[a, b, c]`.
/// Triângulo degenerado → devolve `a`.
pub fn closest_point_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> (Vec3, [f32; 3]) {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, [1.0, 0.0, 0.0]);
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return (b, [0.0, 1.0, 0.0]);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return (a + ab * v, [1.0 - v, v, 0.0]);
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return (c, [0.0, 0.0, 1.0]);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return (a + ac * w, [1.0 - w, 0.0, w]);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (b + (c - b) * w, [0.0, 1.0 - w, w]);
    }
    let sum = va + vb + vc;
    if sum == 0.0 || !sum.is_finite() {
        return (a, [1.0, 0.0, 0.0]);
    }
    let denom = 1.0 / sum;
    let v = vb * denom;
    let w = vc * denom;
    (a + ab * v + ac * w, [1.0 - v - w, v, w])
}

/// Möller–Trumbore. Devolve `(t, u, v)` com `bary = [1-u-v, u, v]`.
/// `cull_back`: ignora triângulos cuja normal (anti-horária) aponta na mesma
/// direção do raio (face de trás).
#[inline]
fn ray_triangle(
    origin: Vec3,
    dir: Vec3,
    a: Vec3,
    b: Vec3,
    c: Vec3,
    cull_back: bool,
) -> Option<(f32, f32, f32)> {
    const EPS: f32 = 1e-12;
    let e1 = b - a;
    let e2 = c - a;
    let pvec = dir.cross(e2);
    let det = e1.dot(pvec);
    if det.abs() < EPS || (cull_back && det < 0.0) {
        return None;
    }
    let inv = 1.0 / det;
    let tvec = origin - a;
    // Tolerância nas baricêntricas: raios que passam exatamente numa aresta
    // compartilhada não podem "vazar" entre dois triângulos.
    const BARY_EPS: f32 = 1e-6;
    let u = tvec.dot(pvec) * inv;
    if u < -BARY_EPS || u > 1.0 + BARY_EPS {
        return None;
    }
    let qvec = tvec.cross(e1);
    let v = dir.dot(qvec) * inv;
    if v < -BARY_EPS || u + v > 1.0 + BARY_EPS {
        return None;
    }
    let t = e2.dot(qvec) * inv;
    Some((t, u, v))
}

#[inline]
fn tri_normal(p: &[Vec3; 3]) -> Vec3 {
    (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero()
}

// ---------------------------------------------------------------------- tipos

/// Resultado de raycast.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    /// Face (n-gon) da malha original que contém o triângulo.
    pub face: u32,
    /// Índice do triângulo na lista estável do BVH ([`TriBvh::triangle`]).
    pub tri_index: u32,
    /// Parâmetro ao longo de `dir` (distância se `dir` é unitário).
    pub t: f32,
    pub point: Vec3,
    /// Coordenadas baricêntricas nos 3 vértices do triângulo.
    pub bary: [f32; 3],
    /// Normal geométrica (unitária) do triângulo.
    pub normal: Vec3,
}

/// Resultado de ponto mais próximo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClosestHit {
    pub face: u32,
    pub tri_index: u32,
    pub point: Vec3,
    pub distance: f32,
    pub bary: [f32; 3],
    /// Normal geométrica (unitária) do triângulo.
    pub normal: Vec3,
}

/// Triângulo do BVH: face de origem e índices de vértice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriRef {
    pub face: u32,
    pub verts: [u32; 3],
}

#[derive(Debug, Clone, Copy)]
struct Node {
    bb: Aabb,
    /// Folha: primeiro slot; interno: índice do filho direito.
    a: u32,
    /// Folha: nº de triângulos (> 0); interno: 0.
    b: u32,
}

impl Node {
    #[inline]
    fn is_leaf(&self) -> bool {
        self.b > 0
    }
}

// ------------------------------------------------------------------------ BVH

/// BVH binária de triângulos. Ver documentação do módulo.
#[derive(Debug, Clone)]
pub struct TriBvh {
    tris: Vec<TriRef>,
    pos: Vec<[Vec3; 3]>,
    /// slot da folha → índice de triângulo.
    order: Vec<u32>,
    /// triângulo → slot.
    tri_slot: Vec<u32>,
    nodes: Vec<Node>,
    parent: Vec<u32>,
    slot_leaf: Vec<u32>,
    /// CSR vértice → triângulos.
    vt_start: Vec<u32>,
    vt_list: Vec<u32>,
    vert_count: usize,
    face_count: usize,
    mark: Vec<bool>,
}

struct Builder<'a> {
    bounds: &'a [Aabb],
    centroids: &'a [Vec3],
    order: Vec<u32>,
    nodes: Vec<Node>,
}

impl Builder<'_> {
    fn range_bounds(&self, lo: usize, hi: usize) -> (Aabb, Aabb) {
        let mut bb = Aabb::EMPTY;
        let mut cb = Aabb::EMPTY;
        for &t in &self.order[lo..hi] {
            bb = bb.union(&self.bounds[t as usize]);
            cb.grow(self.centroids[t as usize]);
        }
        (bb, cb)
    }

    fn build(&mut self, lo: usize, hi: usize, depth: usize) -> u32 {
        let idx = self.nodes.len() as u32;
        let (bb, cb) = self.range_bounds(lo, hi);
        let count = hi - lo;
        self.nodes.push(Node {
            bb,
            a: lo as u32,
            b: count as u32,
        });
        if count <= LEAF_SIZE {
            return idx;
        }
        let ext = cb.extent();
        let axis = if ext.x >= ext.y && ext.x >= ext.z {
            0
        } else if ext.y >= ext.z {
            1
        } else {
            2
        };
        let mid = self.partition(lo, hi, axis, &cb, &bb, depth);
        let _left = self.build(lo, mid, depth + 1);
        let right = self.build(mid, hi, depth + 1);
        self.nodes[idx as usize].a = right;
        self.nodes[idx as usize].b = 0;
        idx
    }

    /// Particiona `order[lo..hi]`; devolve o ponto de corte (lo < mid < hi).
    fn partition(&mut self, lo: usize, hi: usize, axis: usize, cb: &Aabb, bb: &Aabb, depth: usize) -> usize {
        let count = hi - lo;
        let cmin = cb.min[axis];
        let cext = cb.extent()[axis];
        if cext > 1e-20 && depth < 48 {
            // SAH com bins
            let scale = SAH_BINS as f32 / cext;
            let bin_of = |c: f32| (((c - cmin) * scale) as usize).min(SAH_BINS - 1);
            let mut bin_bb = [Aabb::EMPTY; SAH_BINS];
            let mut bin_n = [0usize; SAH_BINS];
            for &t in &self.order[lo..hi] {
                let b = bin_of(self.centroids[t as usize][axis]);
                bin_n[b] += 1;
                bin_bb[b] = bin_bb[b].union(&self.bounds[t as usize]);
            }
            // custo acumulado da esquerda e da direita
            let mut left_cost = [0.0f32; SAH_BINS - 1];
            let mut acc = Aabb::EMPTY;
            let mut n = 0usize;
            for i in 0..SAH_BINS - 1 {
                acc = acc.union(&bin_bb[i]);
                n += bin_n[i];
                left_cost[i] = acc.surface_area() * n as f32;
            }
            let mut best = (f32::INFINITY, 0usize);
            let mut acc = Aabb::EMPTY;
            let mut n = 0usize;
            for i in (1..SAH_BINS).rev() {
                acc = acc.union(&bin_bb[i]);
                n += bin_n[i];
                let cost = left_cost[i - 1] + acc.surface_area() * n as f32;
                if cost < best.0 {
                    best = (cost, i - 1);
                }
            }
            let split_bin = best.1;
            // partição estável por bin (determinística)
            let slice = &mut self.order[lo..hi];
            let cents = self.centroids;
            let mut left: Vec<u32> = Vec::with_capacity(count);
            let mut right: Vec<u32> = Vec::with_capacity(count);
            for &t in slice.iter() {
                if bin_of(cents[t as usize][axis]) <= split_bin {
                    left.push(t);
                } else {
                    right.push(t);
                }
            }
            if !left.is_empty() && !right.is_empty() {
                let mid = lo + left.len();
                slice[..left.len()].copy_from_slice(&left);
                slice[left.len()..].copy_from_slice(&right);
                let _ = bb;
                return mid;
            }
        }
        // fallback: mediana por índice no eixo (ou ordem atual se centroides iguais)
        let cents = self.centroids;
        self.order[lo..hi].sort_by(|&x, &y| {
            cents[x as usize][axis]
                .total_cmp(&cents[y as usize][axis])
                .then(x.cmp(&y))
        });
        lo + count / 2
    }
}

impl TriBvh {
    /// Constrói o BVH de todas as faces da malha (n-gons são triangulados).
    /// Malha sem faces gera um BVH vazio (consultas devolvem `None`/vazio).
    pub fn build(mesh: &EditMesh) -> TriBvh {
        let mut tris: Vec<TriRef> = Vec::with_capacity(mesh.triangle_count());
        let mut pos: Vec<[Vec3; 3]> = Vec::with_capacity(mesh.triangle_count());
        for (fi, face) in mesh.faces.iter().enumerate() {
            let pts: Vec<Vec3> = face.corners.iter().map(|c| mesh.verts[c.vert as usize].co).collect();
            for t in triangulate_polygon(&pts) {
                let v = [face.corners[t[0]].vert, face.corners[t[1]].vert, face.corners[t[2]].vert];
                tris.push(TriRef {
                    face: fi as u32,
                    verts: v,
                });
                pos.push([pts[t[0]], pts[t[1]], pts[t[2]]]);
            }
        }
        let n = tris.len();
        let bounds: Vec<Aabb> = pos.iter().map(|p| Aabb::from_points(p)).collect();
        let centroids: Vec<Vec3> = bounds.iter().map(|b| b.center()).collect();
        let mut builder = Builder {
            bounds: &bounds,
            centroids: &centroids,
            order: (0..n as u32).collect(),
            nodes: Vec::with_capacity(n / 2 + 1),
        };
        if n > 0 {
            builder.build(0, n, 0);
        }
        let Builder { order, nodes, .. } = builder;

        // pais, slots, folha de cada slot
        let mut parent = vec![u32::MAX; nodes.len()];
        let mut slot_leaf = vec![0u32; n];
        for (i, nd) in nodes.iter().enumerate() {
            if nd.is_leaf() {
                for s in nd.a..nd.a + nd.b {
                    slot_leaf[s as usize] = i as u32;
                }
            } else {
                parent[i + 1] = i as u32;
                parent[nd.a as usize] = i as u32;
            }
        }
        let mut tri_slot = vec![0u32; n];
        for (s, &t) in order.iter().enumerate() {
            tri_slot[t as usize] = s as u32;
        }
        // CSR vértice → triângulos
        let vc = mesh.verts.len();
        let mut counts = vec![0u32; vc + 1];
        for t in &tris {
            for &v in &t.verts {
                counts[v as usize + 1] += 1;
            }
        }
        for i in 0..vc {
            counts[i + 1] += counts[i];
        }
        let mut cursor = counts.clone();
        let mut vt_list = vec![0u32; counts[vc] as usize];
        for (ti, t) in tris.iter().enumerate() {
            for &v in &t.verts {
                vt_list[cursor[v as usize] as usize] = ti as u32;
                cursor[v as usize] += 1;
            }
        }
        let mark = vec![false; nodes.len()];
        TriBvh {
            tris,
            pos,
            order,
            tri_slot,
            nodes,
            parent,
            slot_leaf,
            vt_start: counts,
            vt_list,
            vert_count: vc,
            face_count: mesh.faces.len(),
            mark,
        }
    }

    // -------------------------------------------------------------- consultas

    pub fn tri_count(&self) -> usize {
        self.tris.len()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    /// Vértices da malha com que o BVH foi construído.
    pub fn vert_count(&self) -> usize {
        self.vert_count
    }

    /// Faces da malha com que o BVH foi construído.
    pub fn face_count(&self) -> usize {
        self.face_count
    }

    /// Posições atuais (cache) do triângulo `i`. Pânico não ocorre: índice
    /// inválido devolve triângulo nulo.
    pub fn triangle(&self, i: u32) -> [Vec3; 3] {
        self.pos.get(i as usize).copied().unwrap_or([Vec3::ZERO; 3])
    }

    pub fn tri_ref(&self, i: u32) -> Option<TriRef> {
        self.tris.get(i as usize).copied()
    }

    /// Caixa do mundo (raiz). `None` se vazio.
    pub fn bounds(&self) -> Option<Aabb> {
        self.nodes.first().map(|n| n.bb)
    }

    /// Profundidade máxima da árvore (diagnóstico/testes).
    pub fn depth(&self) -> usize {
        fn go(nodes: &[Node], i: usize) -> usize {
            let n = &nodes[i];
            if n.is_leaf() {
                1
            } else {
                1 + go(nodes, i + 1).max(go(nodes, n.a as usize))
            }
        }
        if self.nodes.is_empty() {
            0
        } else {
            go(&self.nodes, 0)
        }
    }

    /// Raycast de face dupla: primeiro triângulo atingido em `0 <= t <= max_t`.
    /// `dir` não precisa ser unitário (t é paramétrico em `dir`).
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Option<RayHit> {
        self.raycast_ex(origin, dir, max_t, false)
    }

    /// Raycast com opção de descartar faces de trás (`cull_backfaces`): só
    /// atinge triângulos cuja normal aponta contra o raio.
    pub fn raycast_ex(&self, origin: Vec3, dir: Vec3, max_t: f32, cull_backfaces: bool) -> Option<RayHit> {
        if self.nodes.is_empty() || !dir.is_finite() || !origin.is_finite() || dir == Vec3::ZERO {
            return None;
        }
        let inv = Vec3::new(1.0 / dir.x, 1.0 / dir.y, 1.0 / dir.z);
        let mut best_t = max_t;
        let mut best: Option<(u32, f32, f32)> = None;
        let mut stack = [0u32; MAX_STACK];
        let mut sp = 0usize;
        if self.nodes[0].bb.ray_entry(origin, inv, best_t).is_none() {
            return None;
        }
        stack[sp] = 0;
        sp += 1;
        while sp > 0 {
            sp -= 1;
            let ni = stack[sp] as usize;
            let node = &self.nodes[ni];
            if node.bb.ray_entry(origin, inv, best_t).is_none() {
                continue;
            }
            if node.is_leaf() {
                for s in node.a..node.a + node.b {
                    let ti = self.order[s as usize];
                    let p = &self.pos[ti as usize];
                    if let Some((t, u, v)) = ray_triangle(origin, dir, p[0], p[1], p[2], cull_backfaces) {
                        let better = t >= 0.0
                            && (t < best_t
                                || (t == best_t && best.is_some_and(|(bt, _, _)| ti < bt)));
                        if better && t <= max_t {
                            best_t = t;
                            best = Some((ti, u, v));
                        }
                    }
                }
            } else {
                let l = ni + 1;
                let r = node.a as usize;
                let tl = self.nodes[l].bb.ray_entry(origin, inv, best_t);
                let tr = self.nodes[r].bb.ray_entry(origin, inv, best_t);
                match (tl, tr) {
                    (Some(a), Some(b)) => {
                        if sp + 2 <= MAX_STACK {
                            // empilha o mais distante primeiro
                            if a <= b {
                                stack[sp] = r as u32;
                                stack[sp + 1] = l as u32;
                            } else {
                                stack[sp] = l as u32;
                                stack[sp + 1] = r as u32;
                            }
                            sp += 2;
                        }
                    }
                    (Some(_), None) => {
                        if sp < MAX_STACK {
                            stack[sp] = l as u32;
                            sp += 1;
                        }
                    }
                    (None, Some(_)) => {
                        if sp < MAX_STACK {
                            stack[sp] = r as u32;
                            sp += 1;
                        }
                    }
                    (None, None) => {}
                }
            }
        }
        best.map(|(ti, u, v)| self.make_ray_hit(ti, origin, dir, best_t, u, v))
    }

    fn make_ray_hit(&self, ti: u32, origin: Vec3, dir: Vec3, t: f32, u: f32, v: f32) -> RayHit {
        let p = &self.pos[ti as usize];
        RayHit {
            face: self.tris[ti as usize].face,
            tri_index: ti,
            t,
            point: origin + dir * t,
            bary: [1.0 - u - v, u, v],
            normal: tri_normal(p),
        }
    }

    /// Todas as interseções do raio com a malha, ordenadas por `t` crescente
    /// (face dupla). Útil para seleção através de objetos (X-ray) e testes de
    /// paridade.
    pub fn raycast_all(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Vec<RayHit> {
        let mut out = Vec::new();
        if self.nodes.is_empty() || !dir.is_finite() || !origin.is_finite() || dir == Vec3::ZERO {
            return out;
        }
        let inv = Vec3::new(1.0 / dir.x, 1.0 / dir.y, 1.0 / dir.z);
        let mut stack = vec![0u32];
        while let Some(ni) = stack.pop() {
            let node = &self.nodes[ni as usize];
            if node.bb.ray_entry(origin, inv, max_t).is_none() {
                continue;
            }
            if node.is_leaf() {
                for s in node.a..node.a + node.b {
                    let ti = self.order[s as usize];
                    let p = &self.pos[ti as usize];
                    if let Some((t, u, v)) = ray_triangle(origin, dir, p[0], p[1], p[2], false) {
                        if t >= 0.0 && t <= max_t {
                            out.push(self.make_ray_hit(ti, origin, dir, t, u, v));
                        }
                    }
                }
            } else {
                stack.push(ni + 1);
                stack.push(node.a);
            }
        }
        out.sort_by(|a, b| a.t.total_cmp(&b.t).then(a.tri_index.cmp(&b.tri_index)));
        out
    }

    /// Ponto da superfície mais próximo de `p`. `None` se o BVH é vazio.
    pub fn closest_point(&self, p: Vec3) -> Option<ClosestHit> {
        self.closest_point_within(p, f32::INFINITY)
    }

    /// Como [`Self::closest_point`], mas ignora superfícies além de `max_dist`.
    pub fn closest_point_within(&self, p: Vec3, max_dist: f32) -> Option<ClosestHit> {
        if self.nodes.is_empty() || !p.is_finite() {
            return None;
        }
        let mut best_d2 = if max_dist.is_finite() {
            max_dist * max_dist
        } else {
            f32::INFINITY
        };
        let mut best: Option<(u32, Vec3, [f32; 3])> = None;
        let mut stack = [0u32; MAX_STACK];
        let mut sp = 1usize;
        stack[0] = 0;
        while sp > 0 {
            sp -= 1;
            let ni = stack[sp] as usize;
            let node = &self.nodes[ni];
            if node.bb.dist_sq(p) > best_d2 {
                continue;
            }
            if node.is_leaf() {
                for s in node.a..node.a + node.b {
                    let ti = self.order[s as usize];
                    let t = &self.pos[ti as usize];
                    let (q, bary) = closest_point_on_triangle(p, t[0], t[1], t[2]);
                    let d2 = q.distance_squared(p);
                    if d2 < best_d2 || (d2 == best_d2 && best.is_some_and(|(bt, _, _)| ti < bt)) {
                        best_d2 = d2;
                        best = Some((ti, q, bary));
                    }
                }
            } else {
                let l = ni + 1;
                let r = node.a as usize;
                let dl = self.nodes[l].bb.dist_sq(p);
                let dr = self.nodes[r].bb.dist_sq(p);
                // visita o mais próximo primeiro (empilha o mais distante antes)
                let (far, near, far_d, near_d) = if dl <= dr { (r, l, dr, dl) } else { (l, r, dl, dr) };
                if sp + 2 <= MAX_STACK {
                    if far_d <= best_d2 {
                        stack[sp] = far as u32;
                        sp += 1;
                    }
                    if near_d <= best_d2 {
                        stack[sp] = near as u32;
                        sp += 1;
                    }
                }
            }
        }
        best.map(|(ti, q, bary)| ClosestHit {
            face: self.tris[ti as usize].face,
            tri_index: ti,
            point: q,
            distance: q.distance(p),
            bary,
            normal: tri_normal(&self.pos[ti as usize]),
        })
    }

    /// Triângulos que intersectam a esfera (`dist(centro, triângulo) <= radius`),
    /// ordenados por índice.
    pub fn query_sphere_tris(&self, center: Vec3, radius: f32) -> Vec<u32> {
        let mut out = Vec::new();
        if self.nodes.is_empty() || !(radius >= 0.0) || !center.is_finite() {
            return out;
        }
        let r2 = radius * radius;
        let mut stack = vec![0u32];
        while let Some(ni) = stack.pop() {
            let node = &self.nodes[ni as usize];
            if node.bb.dist_sq(center) > r2 {
                continue;
            }
            if node.is_leaf() {
                for s in node.a..node.a + node.b {
                    let ti = self.order[s as usize];
                    let t = &self.pos[ti as usize];
                    let (q, _) = closest_point_on_triangle(center, t[0], t[1], t[2]);
                    if q.distance_squared(center) <= r2 {
                        out.push(ti);
                    }
                }
            } else {
                stack.push(ni + 1);
                stack.push(node.a);
            }
        }
        out.sort_unstable();
        out
    }

    /// Vértices (de faces) com `|co - center| <= radius`, ordenados e sem
    /// repetição. Vértices soltos (sem face) não são vistos pelo BVH.
    pub fn query_sphere(&self, center: Vec3, radius: f32) -> Vec<u32> {
        let mut out = Vec::new();
        if self.nodes.is_empty() || !(radius >= 0.0) || !center.is_finite() {
            return out;
        }
        let r2 = radius * radius;
        let mut stack = vec![0u32];
        while let Some(ni) = stack.pop() {
            let node = &self.nodes[ni as usize];
            if node.bb.dist_sq(center) > r2 {
                continue;
            }
            if node.is_leaf() {
                for s in node.a..node.a + node.b {
                    let ti = self.order[s as usize];
                    let t = &self.pos[ti as usize];
                    let tr = &self.tris[ti as usize];
                    for k in 0..3 {
                        if t[k].distance_squared(center) <= r2 {
                            out.push(tr.verts[k]);
                        }
                    }
                }
            } else {
                stack.push(ni + 1);
                stack.push(node.a);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Triângulos cujo AABB intersecta a caixa dada, ordenados.
    pub fn query_aabb(&self, bb: &Aabb) -> Vec<u32> {
        let mut out = Vec::new();
        if self.nodes.is_empty() {
            return out;
        }
        let overlap = |a: &Aabb, b: &Aabb| a.min.cmple(b.max).all() && b.min.cmple(a.max).all();
        let mut stack = vec![0u32];
        while let Some(ni) = stack.pop() {
            let node = &self.nodes[ni as usize];
            if !overlap(&node.bb, bb) {
                continue;
            }
            if node.is_leaf() {
                for s in node.a..node.a + node.b {
                    let ti = self.order[s as usize];
                    if overlap(&Aabb::from_points(&self.pos[ti as usize]), bb) {
                        out.push(ti);
                    }
                }
            } else {
                stack.push(ni + 1);
                stack.push(node.a);
            }
        }
        out.sort_unstable();
        out
    }

    // ----------------------------------------------------------------- refit

    fn check_synced(&self, mesh: &EditMesh) -> ModelResult<()> {
        if mesh.verts.len() != self.vert_count || mesh.faces.len() != self.face_count {
            return Err(ModelError::InvalidParameter(format!(
                "BVH desatualizado: construído com {} vértices/{} faces, malha tem {}/{}; reconstrua",
                self.vert_count,
                self.face_count,
                mesh.verts.len(),
                mesh.faces.len()
            )));
        }
        Ok(())
    }

    fn recompute_node(&mut self, i: usize) {
        let node = self.nodes[i];
        let bb = if node.is_leaf() {
            let mut bb = Aabb::EMPTY;
            for s in node.a..node.a + node.b {
                let ti = self.order[s as usize];
                for &p in &self.pos[ti as usize] {
                    bb.grow(p);
                }
            }
            bb
        } else {
            self.nodes[i + 1].bb.union(&self.nodes[node.a as usize].bb)
        };
        self.nodes[i].bb = bb;
    }

    /// Atualiza posições e caixas depois que vértices foram movidos, sem
    /// reconstruir a árvore. A topologia (faces/vértices) não pode ter mudado.
    /// O resultado das consultas é equivalente a um `build` novo (a qualidade
    /// da árvore pode degradar após deformações grandes).
    pub fn refit(&mut self, mesh: &EditMesh) -> ModelResult<()> {
        self.check_synced(mesh)?;
        for (ti, tr) in self.tris.iter().enumerate() {
            for k in 0..3 {
                self.pos[ti][k] = mesh.verts[tr.verts[k] as usize].co;
            }
        }
        for i in (0..self.nodes.len()).rev() {
            self.recompute_node(i);
        }
        Ok(())
    }

    /// Refit incremental: só os triângulos que usam `verts` e seus ancestrais.
    /// Custo proporcional ao número de triângulos tocados (não ao total).
    pub fn refit_verts(&mut self, mesh: &EditMesh, verts: &[u32]) -> ModelResult<()> {
        self.check_synced(mesh)?;
        for &v in verts {
            if v as usize >= self.vert_count {
                return Err(ModelError::VertexOutOfRange(v, self.vert_count));
            }
        }
        let mut dirty: Vec<u32> = Vec::new();
        for &v in verts {
            let (s, e) = (self.vt_start[v as usize] as usize, self.vt_start[v as usize + 1] as usize);
            for k in s..e {
                let ti = self.vt_list[k] as usize;
                let tr = self.tris[ti];
                for j in 0..3 {
                    self.pos[ti][j] = mesh.verts[tr.verts[j] as usize].co;
                }
                let mut n = self.slot_leaf[self.tri_slot[ti] as usize];
                while n != u32::MAX && !self.mark[n as usize] {
                    self.mark[n as usize] = true;
                    dirty.push(n);
                    n = self.parent[n as usize];
                }
            }
        }
        dirty.sort_unstable_by(|a, b| b.cmp(a)); // filhos (índice maior) antes dos pais
        for &n in &dirty {
            self.recompute_node(n as usize);
            self.mark[n as usize] = false;
        }
        Ok(())
    }

    /// Verifica o invariante estrutural: cada nó contém seus filhos/triângulos.
    pub fn bounds_are_consistent(&self) -> bool {
        const EPS: f32 = 1e-4;
        let contains = |outer: &Aabb, inner: &Aabb| {
            inner.min.cmpge(outer.min - Vec3::splat(EPS)).all() && inner.max.cmple(outer.max + Vec3::splat(EPS)).all()
        };
        for (i, n) in self.nodes.iter().enumerate() {
            if n.is_leaf() {
                for s in n.a..n.a + n.b {
                    let ti = self.order[s as usize] as usize;
                    if !contains(&n.bb, &Aabb::from_points(&self.pos[ti])) {
                        return false;
                    }
                }
            } else if !contains(&n.bb, &self.nodes[i + 1].bb) || !contains(&n.bb, &self.nodes[n.a as usize].bb) {
                return false;
            }
        }
        true
    }
}

// ----------------------------------------------------------------- VertexGrid

type CellKey = (i32, i32, i32);

/// Grade hash uniforme de vértices para consultas rápidas por esfera, raio e
/// vizinho mais próximo. Guarda cópia das posições; após mover vértices chame
/// [`VertexGrid::update`].
#[derive(Debug, Clone)]
pub struct VertexGrid {
    cell: f32,
    inv: f32,
    cells: HashMap<CellKey, Vec<u32>>,
    vert_cell: Vec<CellKey>,
    pos: Vec<Vec3>,
}

impl VertexGrid {
    /// Constrói a grade com células de lado `cell_size` (> 0 e finito).
    pub fn build(mesh: &EditMesh, cell_size: f32) -> ModelResult<VertexGrid> {
        if !(cell_size > 0.0) || !cell_size.is_finite() {
            return Err(ModelError::InvalidParameter(format!(
                "tamanho de célula inválido: {cell_size}"
            )));
        }
        let mut g = VertexGrid {
            cell: cell_size,
            inv: 1.0 / cell_size,
            cells: HashMap::new(),
            vert_cell: Vec::with_capacity(mesh.verts.len()),
            pos: Vec::with_capacity(mesh.verts.len()),
        };
        for (i, v) in mesh.verts.iter().enumerate() {
            let key = g.key(v.co);
            g.cells.entry(key).or_default().push(i as u32);
            g.vert_cell.push(key);
            g.pos.push(v.co);
        }
        Ok(g)
    }

    /// Constrói escolhendo o tamanho de célula pela densidade de vértices
    /// (≈ 4 vértices por célula assumindo superfície 2D).
    pub fn build_auto(mesh: &EditMesh) -> ModelResult<VertexGrid> {
        let diag = mesh.bounds().map(|(a, b)| (b - a).length()).unwrap_or(1.0);
        let n = mesh.verts.len().max(1) as f32;
        let cell = (diag / n.sqrt() * 2.0).max(diag * 1e-4).max(1e-6);
        Self::build(mesh, cell)
    }

    #[inline]
    fn key(&self, p: Vec3) -> CellKey {
        (
            (p.x * self.inv).floor() as i32,
            (p.y * self.inv).floor() as i32,
            (p.z * self.inv).floor() as i32,
        )
    }

    pub fn cell_size(&self) -> f32 {
        self.cell
    }

    pub fn len(&self) -> usize {
        self.pos.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pos.is_empty()
    }

    /// Reposiciona na grade os vértices indicados (leitura da malha atual).
    pub fn update(&mut self, mesh: &EditMesh, verts: &[u32]) -> ModelResult<()> {
        if mesh.verts.len() != self.pos.len() {
            return Err(ModelError::InvalidParameter(
                "grade desatualizada: número de vértices mudou; reconstrua".into(),
            ));
        }
        for &v in verts {
            let i = v as usize;
            if i >= self.pos.len() {
                return Err(ModelError::VertexOutOfRange(v, self.pos.len()));
            }
            let co = mesh.verts[i].co;
            let new_key = self.key(co);
            let old_key = self.vert_cell[i];
            if new_key != old_key {
                let mut remove_cell = false;
                if let Some(list) = self.cells.get_mut(&old_key) {
                    if let Some(p) = list.iter().position(|&x| x == v) {
                        list.swap_remove(p);
                    }
                    remove_cell = list.is_empty();
                }
                if remove_cell {
                    self.cells.remove(&old_key);
                }
                self.cells.entry(new_key).or_default().push(v);
                self.vert_cell[i] = new_key;
            }
            self.pos[i] = co;
        }
        Ok(())
    }

    /// Vértices com `|co - center| <= radius`, ordenados.
    pub fn query_sphere(&self, center: Vec3, radius: f32) -> Vec<u32> {
        let mut out = Vec::new();
        if !(radius >= 0.0) || !center.is_finite() || self.pos.is_empty() {
            return out;
        }
        let lo = self.key(center - Vec3::splat(radius));
        let hi = self.key(center + Vec3::splat(radius));
        let span = (hi.0 as i64 - lo.0 as i64 + 1) * (hi.1 as i64 - lo.1 as i64 + 1) * (hi.2 as i64 - lo.2 as i64 + 1);
        let r2 = radius * radius;
        if span > self.cells.len() as i64 * 2 {
            // consulta maior que a grade: varre as células ocupadas
            for list in self.cells.values() {
                for &v in list {
                    if self.pos[v as usize].distance_squared(center) <= r2 {
                        out.push(v);
                    }
                }
            }
        } else {
            for x in lo.0..=hi.0 {
                for y in lo.1..=hi.1 {
                    for z in lo.2..=hi.2 {
                        if let Some(list) = self.cells.get(&(x, y, z)) {
                            for &v in list {
                                if self.pos[v as usize].distance_squared(center) <= r2 {
                                    out.push(v);
                                }
                            }
                        }
                    }
                }
            }
        }
        out.sort_unstable();
        out
    }

    /// Vértice mais próximo de `p` dentro de `max_dist` (desempate: menor índice).
    pub fn nearest(&self, p: Vec3, max_dist: f32) -> Option<(u32, f32)> {
        let cand = self.query_sphere(p, max_dist);
        let mut best: Option<(u32, f32)> = None;
        for v in cand {
            let d2 = self.pos[v as usize].distance_squared(p);
            if best.is_none_or(|(_, bd)| d2 < bd) {
                best = Some((v, d2));
            }
        }
        best.map(|(v, d2)| (v, d2.sqrt()))
    }

    /// Vértices a no máximo `radius` do segmento do raio (`0 <= t <= max_t`),
    /// com a distância ao longo do raio, ordenados por `t`. Serve para picking
    /// de vértices (seleção por clique/pincel) sem BVH.
    pub fn query_ray(&self, origin: Vec3, dir: Vec3, max_t: f32, radius: f32) -> Vec<(u32, f32)> {
        let mut out: Vec<(u32, f32)> = Vec::new();
        let dl = dir.length();
        if self.pos.is_empty() || !(radius >= 0.0) || dl == 0.0 || !dl.is_finite() || !origin.is_finite() {
            return out;
        }
        let d = dir / dl;
        let max_len = max_t * dl;
        let r2 = radius * radius;
        let test = |v: u32, out: &mut Vec<(u32, f32)>| {
            let w = self.pos[v as usize] - origin;
            let t = w.dot(d).clamp(0.0, max_len);
            let closest = origin + d * t;
            if self.pos[v as usize].distance_squared(closest) <= r2 {
                out.push((v, t / dl));
            }
        };
        // limita o raio à caixa ocupada pela grade
        let mut gmin = Vec3::splat(f32::INFINITY);
        let mut gmax = Vec3::splat(f32::NEG_INFINITY);
        for p in &self.pos {
            gmin = gmin.min(*p);
            gmax = gmax.max(*p);
        }
        let bb = Aabb {
            min: gmin - Vec3::splat(radius),
            max: gmax + Vec3::splat(radius),
        };
        let inv = Vec3::new(1.0 / d.x, 1.0 / d.y, 1.0 / d.z);
        let Some(t0) = bb.ray_entry(origin, inv, max_len) else {
            return out;
        };
        // saída do raio da caixa
        let t1v = (bb.min - origin) * inv;
        let t2v = (bb.max - origin) * inv;
        let t_exit = t1v.max(t2v).min_element().min(max_len);
        let steps = ((t_exit - t0) / self.cell).ceil().max(0.0) as usize + 1;
        let reach = (radius * self.inv).ceil() as i32 + 1;
        let mut visited: BTreeSet<CellKey> = BTreeSet::new();
        let budget = self.cells.len();
        let mut brute = false;
        for s in 0..=steps {
            let t = (t0 + s as f32 * self.cell).min(t_exit);
            let c = self.key(origin + d * t);
            for x in -reach..=reach {
                for y in -reach..=reach {
                    for z in -reach..=reach {
                        visited.insert((c.0 + x, c.1 + y, c.2 + z));
                    }
                }
            }
            if visited.len() > budget.saturating_mul(4) + 64 {
                brute = true;
                break;
            }
        }
        if brute {
            for v in 0..self.pos.len() as u32 {
                test(v, &mut out);
            }
        } else {
            for key in visited {
                if let Some(list) = self.cells.get(&key) {
                    for &v in list {
                        test(v, &mut out);
                    }
                }
            }
        }
        out.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        out
    }
}

// -------------------------------------------------------------------- picking

/// Raio de picking (origem, direção unitária) para um ponto NDC (`x, y` em
/// `[-1, 1]`, y para cima), dado o inverso de `projeção * visão`.
///
/// Assume profundidade de clip em `[0, 1]` com 0 = plano próximo (convenção
/// wgpu/`glam::Mat4::perspective_rh`). Para câmera com Z invertido use
/// [`pick_ray_depth`]. Câmera ortográfica funciona (origem no plano próximo).
/// Matriz degenerada → direção nula.
pub fn pick_ray(camera_inv_view_proj: Mat4, ndc: Vec2) -> (Vec3, Vec3) {
    pick_ray_depth(camera_inv_view_proj, ndc, 0.0, 1.0)
}

/// Como [`pick_ray`], com as profundidades de clip dos planos próximo/distante.
pub fn pick_ray_depth(camera_inv_view_proj: Mat4, ndc: Vec2, near_z: f32, far_z: f32) -> (Vec3, Vec3) {
    let near = camera_inv_view_proj.project_point3(Vec3::new(ndc.x, ndc.y, near_z));
    let far = camera_inv_view_proj.project_point3(Vec3::new(ndc.x, ndc.y, far_z));
    if !near.is_finite() || !far.is_finite() {
        return (Vec3::ZERO, Vec3::ZERO);
    }
    (near, (far - near).normalize_or_zero())
}

/// Pixel (origem no canto superior esquerdo) → NDC (y para cima).
/// Viewport degenerado → `Vec2::ZERO`.
pub fn pixel_to_ndc(px: Vec2, viewport: Vec2) -> Vec2 {
    if viewport.x <= 0.0 || viewport.y <= 0.0 {
        return Vec2::ZERO;
    }
    Vec2::new(px.x / viewport.x * 2.0 - 1.0, 1.0 - px.y / viewport.y * 2.0)
}

/// Raio do pincel em pixels → raio no mundo a `distance` da câmera
/// (perspectiva, `fov_y` em radianos, `viewport_h` em pixels).
pub fn screen_radius_to_world(distance: f32, fov_y: f32, viewport_h: f32, px: f32) -> f32 {
    if viewport_h <= 0.0 {
        return 0.0;
    }
    px * 2.0 * distance * (fov_y * 0.5).tan() / viewport_h
}

/// Inverso de [`screen_radius_to_world`].
pub fn world_radius_to_screen(distance: f32, fov_y: f32, viewport_h: f32, radius: f32) -> f32 {
    let denom = 2.0 * distance * (fov_y * 0.5).tan();
    if denom.abs() < 1e-12 {
        return 0.0;
    }
    radius * viewport_h / denom
}

// ---------------------------------------------------------------------- testes

#[cfg(test)]
pub(crate) mod test_util {
    /// Gerador LCG determinístico para testes property-style.
    pub struct Rng(pub u64);
    impl Rng {
        pub fn next_u32(&mut self) -> u32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 33) as u32
        }
        pub fn f32(&mut self) -> f32 {
            self.next_u32() as f32 / (1u64 << 31) as f32
        }
        pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
            lo + (hi - lo) * self.f32()
        }
        pub fn unit_vec(&mut self) -> glam::Vec3 {
            loop {
                let v = glam::Vec3::new(self.range(-1.0, 1.0), self.range(-1.0, 1.0), self.range(-1.0, 1.0));
                let l = v.length();
                if l > 0.1 && l <= 1.0 {
                    return v / l;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::Rng;
    use super::*;
    use crate::primitives;

    fn brute_closest(bvh: &TriBvh, p: Vec3) -> f32 {
        (0..bvh.tri_count() as u32)
            .map(|i| {
                let t = bvh.triangle(i);
                closest_point_on_triangle(p, t[0], t[1], t[2]).0.distance(p)
            })
            .fold(f32::INFINITY, f32::min)
    }

    fn brute_ray(bvh: &TriBvh, o: Vec3, d: Vec3, max_t: f32, cull: bool) -> Option<f32> {
        let mut best: Option<f32> = None;
        for i in 0..bvh.tri_count() as u32 {
            let t = bvh.triangle(i);
            if let Some((tt, _, _)) = ray_triangle(o, d, t[0], t[1], t[2], cull) {
                if tt >= 0.0 && tt <= max_t && best.is_none_or(|b| tt < b) {
                    best = Some(tt);
                }
            }
        }
        best
    }

    #[test]
    fn sphere_raycast_from_center_hits_at_radius() {
        let m = primitives::quad_sphere(1.0, 12);
        let bvh = TriBvh::build(&m);
        for d in [Vec3::X, Vec3::Y, Vec3::Z, -Vec3::X, Vec3::new(1.0, 1.0, 1.0).normalize()] {
            let hit = bvh.raycast(Vec3::ZERO, d, 10.0).expect("deve acertar");
            assert!((hit.t - 1.0).abs() < 0.01, "t={}", hit.t);
            assert!((hit.point.length() - 1.0).abs() < 0.01);
            assert!(hit.normal.dot(d) > 0.95, "normal para fora");
            assert!((hit.bary.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn raycast_from_outside_front_and_backface_culling() {
        let m = primitives::quad_sphere(1.0, 8);
        let bvh = TriBvh::build(&m);
        let o = Vec3::new(0.0, 0.0, 5.0);
        let d = -Vec3::Z;
        let front = bvh.raycast_ex(o, d, 100.0, true).unwrap();
        assert!((front.t - 4.0).abs() < 0.03);
        // de dentro para fora com culling: só faces viradas contra o raio → nada
        assert!(bvh.raycast_ex(Vec3::ZERO, Vec3::X, 100.0, true).is_none());
        // sem culling acerta
        assert!(bvh.raycast(Vec3::ZERO, Vec3::X, 100.0).is_some());
        // max_t curto não alcança
        assert!(bvh.raycast(o, d, 3.0).is_none());
        // raio que passa ao lado
        assert!(bvh.raycast(Vec3::new(3.0, 0.0, 5.0), d, 100.0).is_none());
        // direção nula / não finita
        assert!(bvh.raycast(o, Vec3::ZERO, 10.0).is_none());
        assert!(bvh.raycast(o, Vec3::new(f32::NAN, 0.0, 0.0), 10.0).is_none());
    }

    #[test]
    fn raycast_all_returns_entry_and_exit_sorted() {
        let m = primitives::quad_sphere(1.0, 8);
        let bvh = TriBvh::build(&m);
        let hits = bvh.raycast_all(Vec3::new(0.1, 0.07, 5.0), -Vec3::Z, 100.0);
        assert!(hits.len() >= 2, "entrada e saída");
        assert!(hits.windows(2).all(|w| w[0].t <= w[1].t));
        assert!((hits[0].t - 4.0).abs() < 0.05);
        assert!((hits.last().unwrap().t - 6.0).abs() < 0.05);
    }

    #[test]
    fn closest_point_outside_and_inside_sphere() {
        let m = primitives::quad_sphere(1.0, 12);
        let bvh = TriBvh::build(&m);
        let p = Vec3::new(0.0, 3.0, 0.0);
        let c = bvh.closest_point(p).unwrap();
        assert!((c.distance - 2.0).abs() < 0.01, "d={}", c.distance);
        assert!((c.point - Vec3::Y).length() < 0.02);
        assert!(c.normal.dot(Vec3::Y) > 0.95);
        // dentro
        let c = bvh.closest_point(Vec3::new(0.2, 0.0, 0.0)).unwrap();
        assert!((c.distance - 0.8).abs() < 0.02);
        // limite de distância
        assert!(bvh.closest_point_within(p, 1.0).is_none());
        assert!(bvh.closest_point_within(p, 2.5).is_some());
        // entrada inválida
        assert!(bvh.closest_point(Vec3::NAN).is_none());
    }

    #[test]
    fn empty_mesh_gives_empty_bvh() {
        let bvh = TriBvh::build(&EditMesh::new("vazio"));
        assert!(bvh.is_empty());
        assert!(bvh.bounds().is_none());
        assert!(bvh.raycast(Vec3::ZERO, Vec3::X, 1.0).is_none());
        assert!(bvh.closest_point(Vec3::ZERO).is_none());
        assert!(bvh.query_sphere(Vec3::ZERO, 1.0).is_empty());
        assert!(bvh.query_sphere_tris(Vec3::ZERO, 1.0).is_empty());
        assert_eq!(bvh.depth(), 0);
        assert!(bvh.bounds_are_consistent());
    }

    #[test]
    fn ngons_are_triangulated_and_map_back_to_faces() {
        let m = primitives::cylinder(1.0, 2.0, 8, 1, true); // tampas são octógonos
        let bvh = TriBvh::build(&m);
        assert_eq!(bvh.tri_count(), m.triangle_count());
        let hit = bvh.raycast(Vec3::new(0.1, 5.0, 0.2), -Vec3::Y, 100.0).unwrap();
        assert!((hit.point.y - 1.0).abs() < 1e-4);
        assert_eq!(m.faces[hit.face as usize].len(), 8);
        assert!(hit.normal.dot(Vec3::Y) > 0.99);
    }

    #[test]
    fn leaves_respect_size_and_bounds_are_consistent() {
        let m = primitives::quad_sphere(1.0, 16);
        let bvh = TriBvh::build(&m);
        assert!(bvh.bounds_are_consistent());
        assert!(bvh.depth() < 40, "profundidade {}", bvh.depth());
        for n in &bvh.nodes {
            if n.is_leaf() {
                assert!(n.b as usize <= LEAF_SIZE);
            }
        }
        // todo triângulo aparece exatamente uma vez
        let mut seen = vec![0u8; bvh.tri_count()];
        for &t in &bvh.order {
            seen[t as usize] += 1;
        }
        assert!(seen.iter().all(|&c| c == 1));
    }

    #[test]
    fn degenerate_identical_centroids_still_build() {
        // 20 triângulos idênticos: SAH não separa; cai na mediana por índice.
        let mut m = EditMesh::new("dup");
        for _ in 0..20 {
            let a = m.add_vert(Vec3::ZERO);
            let b = m.add_vert(Vec3::X);
            let c = m.add_vert(Vec3::Y);
            m.add_face(&[a, b, c]).unwrap();
        }
        let bvh = TriBvh::build(&m);
        assert!(bvh.bounds_are_consistent());
        let hit = bvh.raycast(Vec3::new(0.2, 0.2, 1.0), -Vec3::Z, 5.0).unwrap();
        assert!((hit.t - 1.0).abs() < 1e-5);
        assert_eq!(bvh.raycast_all(Vec3::new(0.2, 0.2, 1.0), -Vec3::Z, 5.0).len(), 20);
    }

    #[test]
    fn property_raycast_matches_bruteforce() {
        let m = primitives::uv_sphere(1.0, 14, 20);
        let bvh = TriBvh::build(&m);
        let mut rng = Rng(7);
        for i in 0..300 {
            let o = rng.unit_vec() * rng.range(0.0, 3.0);
            let d = rng.unit_vec();
            let cull = i % 3 == 0;
            let max_t = if i % 5 == 0 { 1.5 } else { 100.0 };
            let a = bvh.raycast_ex(o, d, max_t, cull).map(|h| h.t);
            let b = brute_ray(&bvh, o, d, max_t, cull);
            match (a, b) {
                (None, None) => {}
                (Some(x), Some(y)) => assert!((x - y).abs() < 1e-4, "{x} vs {y}"),
                other => panic!("divergência {other:?} (o={o:?}, d={d:?})"),
            }
        }
    }

    #[test]
    fn property_closest_point_matches_bruteforce() {
        let m = primitives::capsule(0.5, 1.0, 12, 4);
        let bvh = TriBvh::build(&m);
        let mut rng = Rng(11);
        for _ in 0..300 {
            let p = rng.unit_vec() * rng.range(0.0, 3.0);
            let a = bvh.closest_point(p).unwrap().distance;
            let b = brute_closest(&bvh, p);
            assert!((a - b).abs() < 1e-4, "{a} vs {b} em {p:?}");
        }
    }

    #[test]
    fn property_sphere_queries_match_bruteforce() {
        let m = primitives::quad_sphere(1.0, 10);
        let bvh = TriBvh::build(&m);
        let mut rng = Rng(23);
        for _ in 0..200 {
            let c = rng.unit_vec() * rng.range(0.0, 1.3);
            let r = rng.range(0.05, 0.9);
            // vértices
            let got = bvh.query_sphere(c, r);
            let used = m.used_verts();
            let want: Vec<u32> = (0..m.vert_count() as u32)
                .filter(|&v| used[v as usize] && m.co(v).distance_squared(c) <= r * r)
                .collect();
            assert_eq!(got, want, "c={c:?} r={r}");
            // triângulos
            let got_t = bvh.query_sphere_tris(c, r);
            let want_t: Vec<u32> = (0..bvh.tri_count() as u32)
                .filter(|&i| {
                    let t = bvh.triangle(i);
                    closest_point_on_triangle(c, t[0], t[1], t[2]).0.distance_squared(c) <= r * r
                })
                .collect();
            assert_eq!(got_t, want_t);
            // todo triângulo com vértice na esfera aparece em query_sphere_tris
            for &v in &got {
                assert!(got_t.iter().any(|&ti| bvh.tri_ref(ti).unwrap().verts.contains(&v)));
            }
        }
        assert!(bvh.query_sphere(Vec3::ZERO, -1.0).is_empty());
    }

    #[test]
    fn query_aabb_matches_bruteforce() {
        let m = primitives::quad_sphere(1.0, 8);
        let bvh = TriBvh::build(&m);
        let bb = Aabb {
            min: Vec3::new(0.2, -0.3, -0.5),
            max: Vec3::new(0.9, 0.4, 0.5),
        };
        let got = bvh.query_aabb(&bb);
        let want: Vec<u32> = (0..bvh.tri_count() as u32)
            .filter(|&i| {
                let t = Aabb::from_points(&bvh.triangle(i));
                t.min.cmple(bb.max).all() && bb.min.cmple(t.max).all()
            })
            .collect();
        assert_eq!(got, want);
        assert!(!got.is_empty());
    }

    fn deform(m: &mut EditMesh, rng: &mut Rng, verts: &[u32]) {
        for &v in verts {
            m.verts[v as usize].co += rng.unit_vec() * 0.15;
        }
    }

    #[test]
    fn refit_is_equivalent_to_rebuild() {
        // malha triangulada: a re-triangulação de quads não planares mudaria a
        // superfície num rebuild, o que não é o que se quer comparar aqui.
        let q = primitives::quad_sphere(1.0, 10);
        let mut m = EditMesh::new("tri");
        m.verts = q.verts.clone();
        for f in &q.faces {
            let v: Vec<u32> = f.verts().collect();
            m.add_face(&[v[0], v[1], v[2]]).unwrap();
            m.add_face(&[v[0], v[2], v[3]]).unwrap();
        }
        let mut bvh = TriBvh::build(&m);
        let mut rng = Rng(5);
        let all: Vec<u32> = (0..m.vert_count() as u32).collect();
        deform(&mut m, &mut rng, &all);
        bvh.refit(&m).unwrap();
        let fresh = TriBvh::build(&m);
        assert!(bvh.bounds_are_consistent());
        for _ in 0..200 {
            let o = rng.unit_vec() * 3.0;
            let d = (-o + rng.unit_vec() * 0.5).normalize();
            let a = bvh.raycast(o, d, 100.0).map(|h| h.t);
            let b = fresh.raycast(o, d, 100.0).map(|h| h.t);
            match (a, b) {
                (None, None) => {}
                (Some(x), Some(y)) => assert!((x - y).abs() < 1e-4),
                other => panic!("{other:?}"),
            }
            let p = rng.unit_vec() * rng.range(0.0, 2.0);
            let da = bvh.closest_point(p).unwrap().distance;
            let db = fresh.closest_point(p).unwrap().distance;
            assert!((da - db).abs() < 1e-5);
        }
    }

    #[test]
    fn partial_refit_matches_full_refit() {
        let mut m = primitives::quad_sphere(1.0, 10);
        let mut part = TriBvh::build(&m);
        let mut full = part.clone();
        let mut rng = Rng(99);
        let moved: Vec<u32> = (0..m.vert_count() as u32).filter(|v| v % 7 == 0).collect();
        deform(&mut m, &mut rng, &moved);
        part.refit_verts(&m, &moved).unwrap();
        full.refit(&m).unwrap();
        assert!(part.bounds_are_consistent());
        for (a, b) in part.nodes.iter().zip(&full.nodes) {
            assert!((a.bb.min - b.bb.min).length() < 1e-6 && (a.bb.max - b.bb.max).length() < 1e-6);
        }
        assert!(part.mark.iter().all(|m| !m), "marcas limpas");
        for i in 0..part.tri_count() as u32 {
            assert_eq!(part.triangle(i), full.triangle(i));
        }
    }

    #[test]
    fn refit_rejects_stale_or_invalid_input() {
        let mut m = primitives::cube(1.0);
        let mut bvh = TriBvh::build(&m);
        assert!(matches!(
            bvh.refit_verts(&m, &[99]),
            Err(ModelError::VertexOutOfRange(99, 8))
        ));
        m.add_vert(Vec3::ZERO);
        assert!(matches!(bvh.refit(&m), Err(ModelError::InvalidParameter(_))));
        assert!(matches!(bvh.refit_verts(&m, &[0]), Err(ModelError::InvalidParameter(_))));
    }

    #[test]
    fn vertex_grid_sphere_query_matches_bruteforce() {
        let m = primitives::quad_sphere(1.0, 10);
        let grid = VertexGrid::build_auto(&m).unwrap();
        assert_eq!(grid.len(), m.vert_count());
        let mut rng = Rng(3);
        for _ in 0..200 {
            let c = rng.unit_vec() * rng.range(0.0, 1.4);
            let r = rng.range(0.0, 2.5);
            let got = grid.query_sphere(c, r);
            let want: Vec<u32> = (0..m.vert_count() as u32)
                .filter(|&v| m.co(v).distance_squared(c) <= r * r)
                .collect();
            assert_eq!(got, want);
        }
        assert!(VertexGrid::build(&m, 0.0).is_err());
        assert!(VertexGrid::build(&m, f32::NAN).is_err());
    }

    #[test]
    fn vertex_grid_update_tracks_moved_vertices() {
        let mut m = primitives::quad_sphere(1.0, 6);
        let mut grid = VertexGrid::build(&m, 0.25).unwrap();
        let mut rng = Rng(17);
        let moved: Vec<u32> = (0..m.vert_count() as u32).filter(|v| v % 3 == 0).collect();
        deform(&mut m, &mut rng, &moved);
        grid.update(&m, &moved).unwrap();
        let fresh = VertexGrid::build(&m, 0.25).unwrap();
        for _ in 0..50 {
            let c = rng.unit_vec();
            assert_eq!(grid.query_sphere(c, 0.6), fresh.query_sphere(c, 0.6));
        }
        assert!(matches!(grid.update(&m, &[1000]), Err(ModelError::VertexOutOfRange(..))));
        m.add_vert(Vec3::ZERO);
        assert!(grid.update(&m, &[0]).is_err());
    }

    #[test]
    fn vertex_grid_nearest_and_ray_query() {
        let m = primitives::quad_sphere(1.0, 8);
        let grid = VertexGrid::build_auto(&m).unwrap();
        let (v, d) = grid.nearest(Vec3::new(0.0, 1.2, 0.0), 1.0).unwrap();
        assert!((m.co(v) - Vec3::Y).length() < 0.2 && (d - (m.co(v) - Vec3::new(0.0, 1.2, 0.0)).length()).abs() < 1e-6);
        assert!(grid.nearest(Vec3::new(0.0, 5.0, 0.0), 1.0).is_none());

        let mut rng = Rng(41);
        for _ in 0..100 {
            let o = rng.unit_vec() * 3.0;
            let d = (-o + rng.unit_vec() * 0.3).normalize();
            let r = rng.range(0.02, 0.3);
            let got: Vec<u32> = {
                let mut g: Vec<u32> = grid.query_ray(o, d, 10.0, r).into_iter().map(|(v, _)| v).collect();
                g.sort_unstable();
                g
            };
            let want: Vec<u32> = (0..m.vert_count() as u32)
                .filter(|&v| {
                    let w = m.co(v) - o;
                    let t = w.dot(d).clamp(0.0, 10.0);
                    m.co(v).distance_squared(o + d * t) <= r * r
                })
                .collect();
            assert_eq!(got, want);
        }
        // ordenado por t
        let hits = grid.query_ray(Vec3::new(0.0, 0.0, 4.0), -Vec3::Z, 10.0, 0.3);
        assert!(hits.windows(2).all(|w| w[0].1 <= w[1].1));
        assert!(grid.query_ray(Vec3::ZERO, Vec3::ZERO, 1.0, 1.0).is_empty());
    }

    #[test]
    fn pick_ray_roundtrip_with_perspective_camera() {
        let eye = Vec3::new(0.0, 0.0, 5.0);
        let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh(60f32.to_radians(), 16.0 / 9.0, 0.1, 100.0);
        let vp = proj * view;
        let inv = vp.inverse();
        // centro da tela → olha para -Z
        let (o, d) = pick_ray(inv, Vec2::ZERO);
        assert!((d - (-Vec3::Z)).length() < 1e-4, "d={d:?}");
        assert!(o.z < 5.0 && o.z > 4.0);
        // um ponto no mundo projetado volta como raio que o atravessa
        let world = Vec3::new(0.7, -0.4, 0.3);
        let clip = vp * world.extend(1.0);
        let ndc = Vec2::new(clip.x / clip.w, clip.y / clip.w);
        let (o, d) = pick_ray(inv, ndc);
        let t = (world - o).dot(d);
        assert!((o + d * t - world).length() < 1e-3);
        // raio contra esfera
        let m = primitives::quad_sphere(1.0, 12);
        let bvh = TriBvh::build(&m);
        let (o, d) = pick_ray(inv, Vec2::new(0.0, 0.0));
        let hit = bvh.raycast(o, d, 100.0).unwrap();
        assert!((hit.point.z - 1.0).abs() < 0.02);
        // matriz degenerada
        let (_, dz) = pick_ray(Mat4::ZERO, Vec2::ZERO);
        assert_eq!(dz, Vec3::ZERO);
    }

    #[test]
    fn pick_ray_orthographic_has_constant_direction() {
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let proj = Mat4::orthographic_rh(-2.0, 2.0, -2.0, 2.0, 0.1, 100.0);
        let inv = (proj * view).inverse();
        let (o1, d1) = pick_ray(inv, Vec2::new(-1.0, 1.0));
        let (o2, d2) = pick_ray(inv, Vec2::new(0.5, -0.5));
        assert!((d1 - d2).length() < 1e-5);
        assert!((o1.x + 2.0).abs() < 1e-4 && (o1.y - 2.0).abs() < 1e-4);
        assert!((o2.x - 1.0).abs() < 1e-4 && (o2.y + 1.0).abs() < 1e-4);
    }

    #[test]
    fn pixel_ndc_and_radius_conversions() {
        let vp = Vec2::new(800.0, 600.0);
        assert_eq!(pixel_to_ndc(Vec2::new(0.0, 0.0), vp), Vec2::new(-1.0, 1.0));
        assert_eq!(pixel_to_ndc(Vec2::new(800.0, 600.0), vp), Vec2::new(1.0, -1.0));
        assert_eq!(pixel_to_ndc(Vec2::new(400.0, 300.0), vp), Vec2::ZERO);
        assert_eq!(pixel_to_ndc(Vec2::ONE, Vec2::ZERO), Vec2::ZERO);

        let fov = 90f32.to_radians();
        // a 1 de distância com fov 90, a altura visível é 2 → 600 px = 2 un
        let r = screen_radius_to_world(1.0, fov, 600.0, 300.0);
        assert!((r - 1.0).abs() < 1e-5);
        let back = world_radius_to_screen(1.0, fov, 600.0, r);
        assert!((back - 300.0).abs() < 1e-3);
        assert_eq!(screen_radius_to_world(1.0, fov, 0.0, 10.0), 0.0);
        assert_eq!(world_radius_to_screen(0.0, fov, 600.0, 1.0), 0.0);
    }

    #[test]
    fn closest_point_on_triangle_regions() {
        let (a, b, c) = (Vec3::ZERO, Vec3::X, Vec3::Y);
        // face
        let (q, w) = closest_point_on_triangle(Vec3::new(0.2, 0.2, 1.0), a, b, c);
        assert!((q - Vec3::new(0.2, 0.2, 0.0)).length() < 1e-6 && (w.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        // vértices
        assert_eq!(closest_point_on_triangle(Vec3::new(-1.0, -1.0, 0.0), a, b, c).0, a);
        assert_eq!(closest_point_on_triangle(Vec3::new(2.0, -1.0, 0.0), a, b, c).0, b);
        assert_eq!(closest_point_on_triangle(Vec3::new(-1.0, 2.0, 0.0), a, b, c).0, c);
        // arestas
        assert!((closest_point_on_triangle(Vec3::new(0.5, -1.0, 0.0), a, b, c).0 - Vec3::new(0.5, 0.0, 0.0)).length() < 1e-6);
        assert!((closest_point_on_triangle(Vec3::new(1.0, 1.0, 0.0), a, b, c).0 - Vec3::new(0.5, 0.5, 0.0)).length() < 1e-6);
        // degenerado
        let (q, _) = closest_point_on_triangle(Vec3::new(1.0, 2.0, 3.0), a, a, a);
        assert_eq!(q, a);
    }

    #[test]
    fn rays_along_axes_through_vertices_do_not_leak() {
        // regressão: raio paralelo a eixo passando exatamente por vértices/arestas
        let m = primitives::quad_sphere(1.0, 8);
        let bvh = TriBvh::build(&m);
        for i in -4..=4 {
            for j in -4..=4 {
                let o = Vec3::new(i as f32 * 0.25 * 0.6, j as f32 * 0.25 * 0.6, 5.0);
                if o.truncate().length() < 0.9 {
                    assert!(bvh.raycast(o, -Vec3::Z, 100.0).is_some(), "vazou em {o:?}");
                }
            }
        }
    }

    #[test]
    fn large_mesh_build_and_queries_are_fast_enough() {
        let m = primitives::quad_sphere(1.0, 64);
        let t0 = std::time::Instant::now();
        let bvh = TriBvh::build(&m);
        let build = t0.elapsed();
        let t1 = std::time::Instant::now();
        let mut rng = Rng(77);
        let mut hits = 0;
        for _ in 0..2000 {
            let o = rng.unit_vec() * 3.0;
            if bvh.raycast(o, -o.normalize(), 100.0).is_some() {
                hits += 1;
            }
        }
        let queries = t1.elapsed();
        assert_eq!(hits, 2000, "todos os raios apontam para o centro");
        println!("[bvh perf] {} tris: build {:?}, 2000 raycasts {:?}", bvh.tri_count(), build, queries);
        assert!(build.as_secs() < 30 && queries.as_secs() < 30);
    }
}
