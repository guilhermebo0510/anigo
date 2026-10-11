//! Picking e seleção por caixa em edit mode (CPU).
//!
//! Visibilidade (oclusão) é decidida por raycast do olho ao elemento contra a
//! malha — só para os poucos candidatos perto do cursor, então força bruta é
//! suficiente; trocar por BVH (`anigo_modeling::bvh`) quando integrado.

use super::camera::{project_with, OrbitCamera};
use anigo_modeling::{edge_key, EditMesh, EdgeKey, SelectMode, Selection, Topology};
use glam::{Mat4, Vec2, Vec3};
use std::collections::BTreeSet;

/// Interseção raio × triângulo (Möller–Trumbore, dupla face). Devolve `t`.
pub fn ray_triangle(origin: Vec3, dir: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let e1 = b - a;
    let e2 = c - a;
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 1e-6).then_some(t)
}

/// Face mais próxima atingida pelo raio (força bruta, triangula n-gons).
pub fn raycast_mesh(mesh: &EditMesh, origin: Vec3, dir: Vec3) -> Option<(u32, f32)> {
    let mut best: Option<(u32, f32)> = None;
    for (fi, face) in mesh.faces.iter().enumerate() {
        if face.hidden {
            continue;
        }
        let pts = mesh.face_positions(fi as u32);
        for [i, j, k] in anigo_modeling::convert::triangulate_polygon(&pts) {
            if let Some(t) = ray_triangle(origin, dir, pts[i], pts[j], pts[k]) {
                if best.is_none_or(|(_, bt)| t < bt) {
                    best = Some((fi as u32, t));
                }
            }
        }
    }
    best
}

/// `true` se `point` não é ocultado por outra superfície vista de `eye`.
fn point_visible(mesh: &EditMesh, eye: Vec3, point: Vec3, ortho_dir: Option<Vec3>) -> bool {
    let (origin, dir, dist) = match ortho_dir {
        Some(d) => (point - d * 1000.0, d, 1000.0),
        None => {
            let v = point - eye;
            let len = v.length();
            if len < 1e-6 {
                return true;
            }
            (eye, v / len, len)
        }
    };
    // tolerância: o ponto pertence à superfície, então só conta oclusão bem antes dele
    match raycast_mesh(mesh, origin, dir) {
        Some((_, t)) => t >= dist - 1e-3 * dist.max(1.0),
        None => true,
    }
}

pub struct PickContext<'a> {
    pub mesh: &'a EditMesh,
    pub camera: &'a OrbitCamera,
    pub size: Vec2,
    /// Raio-X: ignora oclusão.
    pub xray: bool,
}

impl PickContext<'_> {
    fn vp(&self) -> Mat4 {
        self.camera.view_proj(self.size.x / self.size.y.max(1.0))
    }

    fn visible(&self, p: Vec3) -> bool {
        if self.xray {
            return true;
        }
        let ortho = self.camera.orthographic.then(|| self.camera.forward());
        point_visible(self.mesh, self.camera.eye(), p, ortho)
    }
}

/// Vértice mais próximo do cursor dentro de `radius_px`.
pub fn pick_vertex(ctx: &PickContext, mouse: Vec2, radius_px: f32) -> Option<u32> {
    let vp = ctx.vp();
    let used = ctx.mesh.used_verts();
    let mut cands: Vec<(f32, u32)> = ctx
        .mesh
        .verts
        .iter()
        .enumerate()
        .filter(|(i, v)| used[*i] && !v.hidden)
        .filter_map(|(i, v)| {
            let (p, _) = project_with(vp, v.co, ctx.size)?;
            let d = p.distance(mouse);
            (d <= radius_px).then_some((d, i as u32))
        })
        .collect();
    cands.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    cands.into_iter().find(|(_, v)| ctx.visible(ctx.mesh.co(*v))).map(|(_, v)| v)
}

fn dist_point_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 < 1e-12 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Aresta mais próxima do cursor dentro de `radius_px`.
pub fn pick_edge(ctx: &PickContext, topo: &Topology, mouse: Vec2, radius_px: f32) -> Option<EdgeKey> {
    let vp = ctx.vp();
    let mut cands: Vec<(f32, EdgeKey)> = topo
        .edges
        .iter()
        .filter_map(|e| {
            let (a, b) = e.verts;
            let (pa, _) = project_with(vp, ctx.mesh.co(a), ctx.size)?;
            let (pb, _) = project_with(vp, ctx.mesh.co(b), ctx.size)?;
            let d = dist_point_segment(mouse, pa, pb);
            (d <= radius_px).then_some((d, e.verts))
        })
        .collect();
    cands.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    cands
        .into_iter()
        .find(|(_, (a, b))| ctx.visible((ctx.mesh.co(*a) + ctx.mesh.co(*b)) * 0.5))
        .map(|(_, k)| k)
}

/// Face sob o cursor (raycast).
pub fn pick_face(ctx: &PickContext, mouse: Vec2) -> Option<u32> {
    let (o, d) = ctx.camera.ray_from_pixel(mouse, ctx.size);
    raycast_mesh(ctx.mesh, o, d).map(|(f, _)| f)
}

/// Seleção por caixa (retângulo em pixels). Vértices dentro da caixa (e visíveis,
/// se não houver raio-X) → derivam arestas/faces conforme o modo.
pub fn box_select(ctx: &PickContext, topo: &Topology, min: Vec2, max: Vec2, mode: SelectMode) -> Selection {
    let vp = ctx.vp();
    let used = ctx.mesh.used_verts();
    let inside = |p: Vec2| p.x >= min.x && p.x <= max.x && p.y >= min.y && p.y <= max.y;
    let verts: BTreeSet<u32> = ctx
        .mesh
        .verts
        .iter()
        .enumerate()
        .filter(|(i, v)| used[*i] && !v.hidden)
        .filter(|(_, v)| project_with(vp, v.co, ctx.size).is_some_and(|(p, _)| inside(p)))
        .filter(|(_, v)| ctx.visible(v.co))
        .map(|(i, _)| i as u32)
        .collect();
    let base = Selection::from_verts(verts.iter().copied());
    match mode {
        SelectMode::Vertex => base.flush(ctx.mesh, SelectMode::Vertex),
        SelectMode::Edge => {
            let edges = base.edges_fully_selected(topo);
            Selection::from_edges(edges).flush(ctx.mesh, SelectMode::Edge)
        }
        SelectMode::Face => {
            let faces = base.faces_fully_selected(ctx.mesh);
            Selection::from_faces(ctx.mesh, faces)
        }
    }
}

/// Aplica um clique de seleção ao conjunto atual (`extend` = Shift: alterna).
pub fn apply_click(
    mesh: &EditMesh,
    current: &Selection,
    mode: SelectMode,
    hit: ClickHit,
    extend: bool,
) -> Selection {
    let mut sel = if extend { current.clone() } else { Selection::default() };
    match (mode, hit) {
        (SelectMode::Vertex, ClickHit::Vertex(v)) => {
            if extend && sel.verts.contains(&v) {
                sel.verts.remove(&v);
            } else {
                sel.verts.insert(v);
            }
            sel.flush(mesh, SelectMode::Vertex)
        }
        (SelectMode::Edge, ClickHit::Edge((a, b))) => {
            let k = edge_key(a, b);
            if extend && sel.edges.contains(&k) {
                sel.edges.remove(&k);
            } else {
                sel.edges.insert(k);
            }
            sel.flush(mesh, SelectMode::Edge)
        }
        (SelectMode::Face, ClickHit::Face(f)) => {
            if extend && sel.faces.contains(&f) {
                sel.faces.remove(&f);
            } else {
                sel.faces.insert(f);
            }
            sel.flush(mesh, SelectMode::Face)
        }
        (_, ClickHit::None) => {
            if extend {
                sel
            } else {
                Selection::default()
            }
        }
        _ => sel,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClickHit {
    None,
    Vertex(u32),
    Edge(EdgeKey),
    Face(u32),
}

/// Resolve o que está sob o cursor para o modo de seleção atual.
pub fn pick(ctx: &PickContext, topo: &Topology, mode: SelectMode, mouse: Vec2) -> ClickHit {
    match mode {
        SelectMode::Vertex => pick_vertex(ctx, mouse, 12.0).map_or(ClickHit::None, ClickHit::Vertex),
        SelectMode::Edge => pick_edge(ctx, topo, mouse, 10.0).map_or(ClickHit::None, ClickHit::Edge),
        SelectMode::Face => pick_face(ctx, mouse).map_or(ClickHit::None, ClickHit::Face),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_modeling::primitives;

    const SIZE: Vec2 = Vec2::new(800.0, 600.0);

    fn cam_front() -> OrbitCamera {
        let mut c = OrbitCamera {
            target: Vec3::ZERO,
            distance: 5.0,
            ..Default::default()
        };
        c.yaw = 0.0;
        c.pitch = 0.0;
        c
    }

    #[test]
    fn ray_triangle_hits_and_misses() {
        let t = ray_triangle(
            Vec3::new(0.2, 0.2, 5.0),
            Vec3::NEG_Z,
            Vec3::ZERO,
            Vec3::X,
            Vec3::Y,
        );
        assert!((t.unwrap() - 5.0).abs() < 1e-5);
        assert!(ray_triangle(Vec3::new(2.0, 2.0, 5.0), Vec3::NEG_Z, Vec3::ZERO, Vec3::X, Vec3::Y).is_none());
    }

    #[test]
    fn click_on_front_face_center_picks_front_face() {
        let m = primitives::cube(2.0);
        let cam = cam_front();
        let ctx = PickContext { mesh: &m, camera: &cam, size: SIZE, xray: false };
        let f = pick_face(&ctx, SIZE * 0.5).unwrap();
        assert!(m.face_normal(f).dot(Vec3::Z) > 0.99);
    }

    #[test]
    fn vertex_pick_ignores_occluded_back_vertices() {
        let m = primitives::cube(2.0);
        let cam = cam_front();
        let ctx = PickContext { mesh: &m, camera: &cam, size: SIZE, xray: false };
        // Pelo centro da tela, os vértices frontais e traseiros se projetam em cantos
        // coincidentes em ortho; em perspectiva os traseiros ficam mais perto do centro.
        let (front_corner, _) = cam.project(Vec3::new(1.0, 1.0, 1.0), SIZE).unwrap();
        let v = pick_vertex(&ctx, front_corner, 8.0).unwrap();
        assert!(m.co(v).z > 0.0, "deve escolher o vértice frontal, não o oculto");
        let (back_corner, _) = cam.project(Vec3::new(1.0, 1.0, -1.0), SIZE).unwrap();
        // com raio-X o vértice traseiro fica selecionável
        let xray = PickContext { xray: true, ..PickContext { mesh: &m, camera: &cam, size: SIZE, xray: true } };
        let vb = pick_vertex(&xray, back_corner, 4.0).unwrap();
        assert!(m.co(vb).z < 0.0);
    }

    #[test]
    fn edge_pick_finds_edge_near_cursor() {
        let m = primitives::cube(2.0);
        let topo = Topology::build(&m);
        let cam = cam_front();
        let ctx = PickContext { mesh: &m, camera: &cam, size: SIZE, xray: false };
        let (a, _) = cam.project(Vec3::new(-1.0, 1.0, 1.0), SIZE).unwrap();
        let (b, _) = cam.project(Vec3::new(1.0, 1.0, 1.0), SIZE).unwrap();
        let mid = (a + b) * 0.5 + Vec2::new(0.0, 2.0);
        let e = pick_edge(&ctx, &topo, mid, 6.0).unwrap();
        let (pa, pb) = (m.co(e.0), m.co(e.1));
        assert!(pa.y > 0.9 && pb.y > 0.9 && pa.z > 0.9 && pb.z > 0.9);
    }

    #[test]
    fn box_select_front_face_in_face_mode() {
        let m = primitives::cube(2.0);
        let topo = Topology::build(&m);
        let cam = cam_front();
        let ctx = PickContext { mesh: &m, camera: &cam, size: SIZE, xray: false };
        let (p0, _) = cam.project(Vec3::new(-1.0, -1.0, 1.0), SIZE).unwrap();
        let (p1, _) = cam.project(Vec3::new(1.0, 1.0, 1.0), SIZE).unwrap();
        let min = p0.min(p1) - Vec2::splat(3.0);
        let max = p0.max(p1) + Vec2::splat(3.0);
        let sel = box_select(&ctx, &topo, min, max, SelectMode::Face);
        // só a face frontal tem os 4 vértices visíveis dentro da caixa
        assert_eq!(sel.faces.len(), 1);
        let f = *sel.faces.iter().next().unwrap();
        assert!(m.face_normal(f).dot(Vec3::Z) > 0.99);
    }

    #[test]
    fn click_toggle_and_clear_semantics() {
        let m = primitives::cube(2.0);
        let s0 = Selection::default();
        let s1 = apply_click(&m, &s0, SelectMode::Face, ClickHit::Face(0), false);
        assert_eq!(s1.faces.len(), 1);
        let s2 = apply_click(&m, &s1, SelectMode::Face, ClickHit::Face(1), true);
        assert_eq!(s2.faces.len(), 2);
        let s3 = apply_click(&m, &s2, SelectMode::Face, ClickHit::Face(1), true);
        assert_eq!(s3.faces.len(), 1);
        let cleared = apply_click(&m, &s3, SelectMode::Face, ClickHit::None, false);
        assert!(cleared.is_empty());
        let kept = apply_click(&m, &s3, SelectMode::Face, ClickHit::None, true);
        assert_eq!(kept, s3);
    }
}
