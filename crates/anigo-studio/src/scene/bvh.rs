//! Raycast da cena: BVH de triângulos por ativo (espaço local, construído UMA vez e compartilhado por
//! todas as instâncias) + varredura de caixas mundiais por instância ("TLAS" simples).
//!
//! Selecionar entre centenas de objetos custa: 1 teste raio×caixa por instância (µs) + travessia da BVH
//! do ativo só nos poucos candidatos que o raio realmente atravessa, em ordem de distância.

use anigo_core::ids::NodeId;
use anigo_core::Mesh;
use glam::{Mat4, Vec3};
use std::sync::Arc;

/// Caixa alinhada aos eixos.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub const EMPTY: Aabb = Aabb { min: Vec3::splat(f32::INFINITY), max: Vec3::splat(f32::NEG_INFINITY) };

    pub fn from_points(points: impl IntoIterator<Item = Vec3>) -> Aabb {
        let mut b = Aabb::EMPTY;
        for p in points {
            b.grow(p);
        }
        b
    }

    pub fn grow(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    pub fn union(&self, o: &Aabb) -> Aabb {
        Aabb { min: self.min.min(o.min), max: self.max.max(o.max) }
    }

    pub fn is_valid(&self) -> bool {
        self.min.x <= self.max.x && self.min.y <= self.max.y && self.min.z <= self.max.z
    }

    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }

    pub fn corners(&self) -> [Vec3; 8] {
        let (a, b) = (self.min, self.max);
        [
            Vec3::new(a.x, a.y, a.z),
            Vec3::new(b.x, a.y, a.z),
            Vec3::new(b.x, a.y, b.z),
            Vec3::new(a.x, a.y, b.z),
            Vec3::new(a.x, b.y, a.z),
            Vec3::new(b.x, b.y, a.z),
            Vec3::new(b.x, b.y, b.z),
            Vec3::new(a.x, b.y, b.z),
        ]
    }

    /// Caixa mundial de uma caixa local transformada por `m`.
    pub fn transformed(&self, m: &Mat4) -> Aabb {
        Aabb::from_points(self.corners().into_iter().map(|c| m.transform_point3(c)))
    }

    /// Distância de entrada do raio (slab test); `None` se não cruza antes de `max_t`.
    pub fn ray_entry(&self, o: Vec3, inv_d: Vec3, max_t: f32) -> Option<f32> {
        let t1 = (self.min - o) * inv_d;
        let t2 = (self.max - o) * inv_d;
        let tmin = t1.min(t2);
        let tmax = t1.max(t2);
        let near = tmin.x.max(tmin.y).max(tmin.z).max(0.0);
        let far = tmax.x.min(tmax.y).min(tmax.z).min(max_t);
        (near <= far).then_some(near)
    }
}

fn inv_dir(d: Vec3) -> Vec3 {
    let f = |v: f32| if v.abs() < 1e-12 { 1e12f32.copysign(v) } else { 1.0 / v };
    Vec3::new(f(d.x), f(d.y), f(d.z))
}

/// Interseção raio×triângulo (Möller–Trumbore, dupla face). `t` na unidade de `d`.
pub fn ray_triangle(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let e1 = b - a;
    let e2 = c - a;
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-14 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 1e-7).then_some(t)
}

#[derive(Debug, Clone, Copy)]
struct BvhNode {
    bounds: Aabb,
    /// Folha: primeiro triângulo em `order`; interno: índice do filho esquerdo (o direito é `left + 1`).
    first: u32,
    /// Folha: nº de triângulos (> 0); interno: 0.
    count: u32,
}

/// Resultado de um acerto local.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalHit {
    pub t: f32,
    pub tri: u32,
    pub normal: Vec3,
}

/// BVH de triângulos de UMA malha, no espaço local.
#[derive(Debug)]
pub struct MeshBvh {
    nodes: Vec<BvhNode>,
    pos: Vec<Vec3>,
    tris: Vec<[u32; 3]>,
    order: Vec<u32>,
    pub bounds: Aabb,
}

const LEAF_MAX: usize = 4;

impl MeshBvh {
    pub fn build(mesh: &Mesh) -> MeshBvh {
        let pos: Vec<Vec3> = mesh.vertices.iter().map(|v| Vec3::from_array(v.position)).collect();
        let tris: Vec<[u32; 3]> = mesh
            .indices
            .chunks_exact(3)
            .map(|c| [c[0], c[1], c[2]])
            .filter(|t| t.iter().all(|i| (*i as usize) < pos.len()))
            .collect();
        let mut order: Vec<u32> = (0..tris.len() as u32).collect();
        let bounds = Aabb::from_points(pos.iter().copied());
        let mut bvh = MeshBvh { nodes: Vec::new(), pos, tris, order: Vec::new(), bounds };
        if bvh.tris.is_empty() {
            bvh.order = order;
            return bvh;
        }
        let centroids: Vec<Vec3> = bvh.tris.iter().map(|t| (bvh.pos[t[0] as usize] + bvh.pos[t[1] as usize] + bvh.pos[t[2] as usize]) / 3.0).collect();
        let tri_bounds: Vec<Aabb> = bvh
            .tris
            .iter()
            .map(|t| Aabb::from_points([bvh.pos[t[0] as usize], bvh.pos[t[1] as usize], bvh.pos[t[2] as usize]]))
            .collect();
        bvh.nodes.push(BvhNode { bounds: Aabb::EMPTY, first: 0, count: 0 });
        let n = order.len();
        bvh.build_range(0, &mut order, 0, n, &centroids, &tri_bounds);
        bvh.order = order;
        bvh
    }

    fn build_range(&mut self, node: usize, order: &mut [u32], lo: usize, hi: usize, centroids: &[Vec3], tri_bounds: &[Aabb]) {
        let mut bounds = Aabb::EMPTY;
        let mut cb = Aabb::EMPTY;
        for &i in &order[lo..hi] {
            bounds = bounds.union(&tri_bounds[i as usize]);
            cb.grow(centroids[i as usize]);
        }
        if hi - lo <= LEAF_MAX {
            self.nodes[node] = BvhNode { bounds, first: lo as u32, count: (hi - lo) as u32 };
            return;
        }
        let ext = cb.size();
        let axis = if ext.x >= ext.y && ext.x >= ext.z {
            0
        } else if ext.y >= ext.z {
            1
        } else {
            2
        };
        if ext[axis] < 1e-9 {
            // centroides coincidentes: parte no meio para garantir progresso
            self.nodes[node] = BvhNode { bounds, first: lo as u32, count: (hi - lo) as u32 };
            return;
        }
        let mid = (lo + hi) / 2;
        order[lo..hi].select_nth_unstable_by(mid - lo, |a, b| centroids[*a as usize][axis].total_cmp(&centroids[*b as usize][axis]));
        let left = self.nodes.len();
        self.nodes.push(BvhNode { bounds: Aabb::EMPTY, first: 0, count: 0 });
        self.nodes.push(BvhNode { bounds: Aabb::EMPTY, first: 0, count: 0 });
        self.nodes[node] = BvhNode { bounds, first: left as u32, count: 0 };
        self.build_range(left, order, lo, mid, centroids, tri_bounds);
        self.build_range(left + 1, order, mid, hi, centroids, tri_bounds);
    }

    pub fn tri_count(&self) -> usize {
        self.tris.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    /// Acerto mais próximo (espaço local). `d` não precisa ser unitário: `t` está na unidade de `d`.
    pub fn raycast(&self, o: Vec3, d: Vec3, max_t: f32) -> Option<LocalHit> {
        if self.nodes.is_empty() {
            return None;
        }
        let inv = inv_dir(d);
        let mut best: Option<LocalHit> = None;
        let mut limit = max_t;
        let mut stack: [u32; 64] = [0; 64];
        let mut sp = 1usize;
        stack[0] = 0;
        while sp > 0 {
            sp -= 1;
            let node = &self.nodes[stack[sp] as usize];
            if node.bounds.ray_entry(o, inv, limit).is_none() {
                continue;
            }
            if node.count > 0 {
                for k in 0..node.count {
                    let tri = self.order[(node.first + k) as usize];
                    let [a, b, c] = self.tris[tri as usize];
                    let (pa, pb, pc) = (self.pos[a as usize], self.pos[b as usize], self.pos[c as usize]);
                    if let Some(t) = ray_triangle(o, d, pa, pb, pc) {
                        if t < limit {
                            limit = t;
                            let mut n = (pb - pa).cross(pc - pa).normalize_or_zero();
                            if n.dot(d) > 0.0 {
                                n = -n; // normal virada para o raio
                            }
                            best = Some(LocalHit { t, tri, normal: n });
                        }
                    }
                }
            } else if sp + 2 <= stack.len() {
                let l = node.first;
                // visita primeiro o filho cuja caixa é entrada antes
                let tl = self.nodes[l as usize].bounds.ray_entry(o, inv, limit);
                let tr = self.nodes[l as usize + 1].bounds.ray_entry(o, inv, limit);
                match (tl, tr) {
                    (Some(a), Some(b)) => {
                        let (near, far) = if a <= b { (l, l + 1) } else { (l + 1, l) };
                        stack[sp] = far;
                        stack[sp + 1] = near;
                        sp += 2;
                    }
                    (Some(_), None) => {
                        stack[sp] = l;
                        sp += 1;
                    }
                    (None, Some(_)) => {
                        stack[sp] = l + 1;
                        sp += 1;
                    }
                    (None, None) => {}
                }
            }
        }
        best
    }

    /// Vértice mais próximo de um ponto local (para snap a vértice — Fase 2; usado em testes e ferramentas).
    pub fn nearest_vertex(&self, p: Vec3) -> Option<(Vec3, f32)> {
        self.pos.iter().map(|v| (*v, v.distance(p))).min_by(|a, b| a.1.total_cmp(&b.1))
    }
}

/// Instância na cena: ativo + matriz mundial.
#[derive(Clone)]
pub struct Instance {
    pub id: NodeId,
    pub world: Mat4,
    pub inv: Mat4,
    pub bvh: Arc<MeshBvh>,
    pub world_bounds: Aabb,
}

impl Instance {
    pub fn new(id: NodeId, world: Mat4, bvh: Arc<MeshBvh>) -> Option<Instance> {
        if !bvh.bounds.is_valid() || !world.is_finite() || world.determinant().abs() < 1e-12 {
            return None;
        }
        let world_bounds = bvh.bounds.transformed(&world);
        Some(Instance { id, world, inv: world.inverse(), bvh, world_bounds })
    }
}

/// Acerto na cena.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneHit {
    pub id: NodeId,
    pub t: f32,
    pub point: Vec3,
    /// Normal de mundo (unitária, virada para o raio).
    pub normal: Vec3,
}

/// Conjunto de instâncias consultável por raio.
#[derive(Default, Clone)]
pub struct SceneBvh {
    pub instances: Vec<Instance>,
}

impl SceneBvh {
    pub fn new(instances: Vec<Instance>) -> Self {
        Self { instances }
    }

    /// Acerto mais próximo, ignorando instâncias em que `skip` devolve `true`.
    pub fn raycast(&self, o: Vec3, d: Vec3, max_t: f32, skip: impl Fn(&NodeId) -> bool) -> Option<SceneHit> {
        let d = d.normalize_or_zero();
        if d == Vec3::ZERO {
            return None;
        }
        let inv = inv_dir(d);
        // candidatos: caixa mundial atravessada, por distância de entrada
        let mut cands: Vec<(f32, usize)> = self
            .instances
            .iter()
            .enumerate()
            .filter(|(_, i)| !skip(&i.id))
            .filter_map(|(k, i)| i.world_bounds.ray_entry(o, inv, max_t).map(|t| (t, k)))
            .collect();
        cands.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut best: Option<SceneHit> = None;
        let mut limit = max_t;
        for (entry, k) in cands {
            if entry > limit {
                break;
            }
            let inst = &self.instances[k];
            // raio no espaço local: `t` é preservado porque a direção local NÃO é normalizada
            let lo = inst.inv.transform_point3(o);
            let ld = inst.inv.transform_vector3(d);
            if let Some(h) = inst.bvh.raycast(lo, ld, limit) {
                if h.t < limit {
                    limit = h.t;
                    let n = inst.inv.transpose().transform_vector3(h.normal).normalize_or_zero();
                    let n = if n.dot(d) > 0.0 { -n } else { n };
                    best = Some(SceneHit { id: inst.id.clone(), t: h.t, point: o + d * h.t, normal: n });
                }
            }
        }
        best
    }

    /// Caixa mundial de um nó.
    pub fn bounds_of(&self, id: &NodeId) -> Option<Aabb> {
        self.instances.iter().find(|i| &i.id == id).map(|i| i.world_bounds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::Mesh;
    use std::time::Instant;

    fn cube_bvh() -> Arc<MeshBvh> {
        Arc::new(MeshBvh::build(&Mesh::create_cube(1.0)))
    }

    fn sphere_mesh(rings: u32, sectors: u32) -> Mesh {
        let em = anigo_modeling::primitives::uv_sphere(0.5, rings, sectors);
        anigo_modeling::convert::to_core_mesh(&em, &Default::default()).unwrap()
    }

    #[test]
    fn ray_hits_the_cube_face_with_the_right_normal() {
        let bvh = cube_bvh();
        let h = bvh.raycast(Vec3::new(0.1, 0.2, 5.0), Vec3::NEG_Z, 100.0).unwrap();
        assert!((h.t - 4.5).abs() < 1e-4, "{}", h.t);
        assert!((h.normal - Vec3::Z).length() < 1e-4);
        assert!(bvh.raycast(Vec3::new(2.0, 0.0, 5.0), Vec3::NEG_Z, 100.0).is_none());
        assert!(bvh.raycast(Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z, 3.0).is_none(), "max_t respeitado");
    }

    #[test]
    fn bvh_matches_brute_force_on_a_dense_mesh() {
        let mesh = sphere_mesh(24, 36);
        let bvh = MeshBvh::build(&mesh);
        let tris: Vec<[Vec3; 3]> = mesh
            .indices
            .chunks_exact(3)
            .map(|c| [c[0], c[1], c[2]].map(|i| Vec3::from_array(mesh.vertices[i as usize].position)))
            .collect();
        let mut rng = 12345u64;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            (rng >> 11) as f32 / (1u64 << 53) as f32
        };
        let mut hits = 0;
        for _ in 0..300 {
            let o = Vec3::new(next() * 4.0 - 2.0, next() * 4.0 - 2.0, 3.0);
            let target = Vec3::new(next() - 0.5, next() - 0.5, next() - 0.5);
            let d = (target - o).normalize();
            let brute = tris.iter().filter_map(|t| ray_triangle(o, d, t[0], t[1], t[2])).fold(None, |a: Option<f32>, t| Some(a.map_or(t, |x| x.min(t))));
            let fast = bvh.raycast(o, d, 1e9).map(|h| h.t);
            match (brute, fast) {
                (Some(a), Some(b)) => {
                    hits += 1;
                    assert!((a - b).abs() < 1e-4, "{a} vs {b}");
                }
                (None, None) => {}
                other => panic!("divergiu: {other:?}"),
            }
        }
        assert!(hits > 40, "o teste precisa acertar a esfera: {hits}");
    }

    #[test]
    fn scene_raycast_respects_instance_transforms_and_picks_the_nearest() {
        let bvh = cube_bvh();
        let at = |x: f32, z: f32, s: f32| Mat4::from_scale_rotation_translation(Vec3::splat(s), glam::Quat::from_rotation_y(0.4), Vec3::new(x, 0.5, z));
        let scene = SceneBvh::new(vec![
            Instance::new(NodeId::from_slug("far"), at(0.0, -4.0, 1.0), bvh.clone()).unwrap(),
            Instance::new(NodeId::from_slug("near"), at(0.0, 0.0, 2.0), bvh.clone()).unwrap(),
        ]);
        let h = scene.raycast(Vec3::new(0.0, 0.5, 10.0), Vec3::NEG_Z, 100.0, |_| false).unwrap();
        assert_eq!(h.id, NodeId::from_slug("near"));
        assert!(h.t > 7.0 && h.t < 9.5, "{}", h.t);
        assert!((h.point - (Vec3::new(0.0, 0.5, 10.0) + Vec3::NEG_Z * h.t)).length() < 1e-4);
        assert!(h.normal.dot(Vec3::Z) > 0.5, "normal virada para o raio: {:?}", h.normal);
        // ignorar o da frente revela o de trás
        let h = scene.raycast(Vec3::new(0.0, 0.5, 10.0), Vec3::NEG_Z, 100.0, |id| id == &NodeId::from_slug("near")).unwrap();
        assert_eq!(h.id, NodeId::from_slug("far"));
        // não-uniforme: t continua em unidades de mundo
        let flat = Mat4::from_scale(Vec3::new(4.0, 0.2, 4.0));
        let s = SceneBvh::new(vec![Instance::new(NodeId::from_slug("floor"), flat, bvh).unwrap()]);
        let h = s.raycast(Vec3::new(0.0, 5.0, 0.0), Vec3::NEG_Y, 100.0, |_| false).unwrap();
        assert!((h.t - 4.9).abs() < 1e-4, "{}", h.t);
    }

    #[test]
    fn picking_hundreds_of_objects_is_well_under_5_ms() {
        let bvh = Arc::new(MeshBvh::build(&sphere_mesh(16, 24)));
        let mut instances = Vec::new();
        for i in 0..600u32 {
            let (gx, gz) = ((i % 30) as f32 * 1.5 - 22.0, (i / 30) as f32 * 1.5 - 15.0);
            let m = Mat4::from_scale_rotation_translation(Vec3::splat(0.6 + (i % 5) as f32 * 0.1), glam::Quat::from_rotation_y(i as f32), Vec3::new(gx, 0.5, gz));
            instances.push(Instance::new(NodeId::from_slug(&format!("n{i}")), m, bvh.clone()).unwrap());
        }
        let scene = SceneBvh::new(instances);
        let start = Instant::now();
        let mut hits = 0;
        const N: u32 = 400;
        for k in 0..N {
            let o = Vec3::new(-25.0 + k as f32 * 0.12, 6.0, -18.0 + (k % 17) as f32 * 2.0);
            let d = (Vec3::new(o.x * 0.3, 0.3, -2.0) - o).normalize();
            if scene.raycast(o, d, 1e9, |_| false).is_some() {
                hits += 1;
            }
        }
        let avg = start.elapsed().as_secs_f64() * 1000.0 / N as f64;
        assert!(hits > 20, "ao menos alguns raios acertam: {hits}");
        assert!(avg < 5.0, "média de {avg:.3} ms por seleção (orçamento: 5 ms, mesmo em debug)");
    }

    #[test]
    fn empty_and_degenerate_meshes_do_not_panic() {
        let empty = MeshBvh::build(&Mesh::new("x", vec![], vec![]));
        assert!(empty.raycast(Vec3::ZERO, Vec3::X, 10.0).is_none());
        assert!(Instance::new(NodeId::from_slug("a"), Mat4::ZERO, cube_bvh()).is_none());
        assert!(SceneBvh::default().raycast(Vec3::ZERO, Vec3::ZERO, 10.0, |_| false).is_none());
    }
}
