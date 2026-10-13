//! Ponte entre [`EditMesh`] (n-gons editáveis) e `anigo_core::Mesh`
//! (triângulos indexados no layout NPR de 72 bytes consumido pelo renderer).

use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, Corner, EVert, EditMesh, Face};
use crate::normals::{compute_corner_normals, NormalParams};
use anigo_core::{Mesh, Vertex};
use glam::{Vec2, Vec3};
use std::collections::HashMap;

/// Triangula um polígono (índices locais 0..n). Triângulo → trivial; quad →
/// diagonal que preserva a orientação; n-gon → ear clipping no plano de Newell.
pub fn triangulate_polygon(pts: &[Vec3]) -> Vec<[usize; 3]> {
    let n = pts.len();
    if n < 3 {
        return Vec::new();
    }
    if n == 3 {
        return vec![[0, 1, 2]];
    }
    let normal = crate::mesh::newell_normal(pts);
    if normal == Vec3::ZERO {
        // degenerado: leque simples, ainda produz índices válidos
        return (1..n - 1).map(|i| [0, i, i + 1]).collect();
    }
    if n == 4 {
        let ok = |a: usize, b: usize, c: usize| (pts[b] - pts[a]).cross(pts[c] - pts[a]).dot(normal) > 0.0;
        // escolhe a diagonal mais curta entre as que mantêm ambos os triângulos válidos
        let d02 = pts[0].distance_squared(pts[2]);
        let d13 = pts[1].distance_squared(pts[3]);
        let first_02 = d02 <= d13;
        let tri_02 = ok(0, 1, 2) && ok(0, 2, 3);
        let tri_13 = ok(1, 2, 3) && ok(1, 3, 0);
        return match (first_02, tri_02, tri_13) {
            (_, true, false) | (true, true, true) => vec![[0, 1, 2], [0, 2, 3]],
            (_, false, true) | (false, true, true) => vec![[1, 2, 3], [1, 3, 0]],
            _ => vec![[0, 1, 2], [0, 2, 3]],
        };
    }
    // ear clipping
    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity(n - 2);
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n {
        guard += 1;
        let m = idx.len();
        let mut clipped = false;
        for k in 0..m {
            let (ia, ib, ic) = (idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]);
            let (a, b, c) = (pts[ia], pts[ib], pts[ic]);
            if (b - a).cross(c - a).dot(normal) <= 1e-12 {
                continue; // reflexo ou colinear
            }
            let inside = idx.iter().any(|&j| {
                j != ia && j != ib && j != ic && point_in_triangle(pts[j], a, b, c, normal)
            });
            if inside {
                continue;
            }
            out.push([ia, ib, ic]);
            idx.remove(k);
            clipped = true;
            break;
        }
        if !clipped {
            // polígono patológico: cai para leque para nunca travar
            let first = idx[0];
            for w in idx[1..].windows(2) {
                out.push([first, w[0], w[1]]);
            }
            return out;
        }
    }
    if idx.len() == 3 {
        out.push([idx[0], idx[1], idx[2]]);
    }
    out
}

fn point_in_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3, n: Vec3) -> bool {
    let s0 = (b - a).cross(p - a).dot(n);
    let s1 = (c - b).cross(p - b).dot(n);
    let s2 = (a - c).cross(p - c).dot(n);
    s0 >= 0.0 && s1 >= 0.0 && s2 >= 0.0
}

#[derive(Debug, Clone)]
pub struct ToCoreOptions {
    pub normals: NormalParams,
}

impl Default for ToCoreOptions {
    fn default() -> Self {
        Self {
            normals: NormalParams::default(),
        }
    }
}

/// `EditMesh` → `anigo_core::Mesh`. Vértices de render são divididos por
/// (vértice, UV, normal) — exatamente o que o renderer precisa para costuras
/// de UV e arestas duras.
pub fn to_core_mesh(mesh: &EditMesh, opts: &ToCoreOptions) -> ModelResult<Mesh> {
    mesh.validate()?;
    let cn = compute_corner_normals(mesh, &opts.normals);
    let mut out_verts: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut lookup: HashMap<(u32, [u32; 2], [u32; 3]), u32> = HashMap::new();

    for (fi, face) in mesh.faces.iter().enumerate() {
        if face.hidden {
            continue;
        }
        let pts = mesh.face_positions(fi as u32);
        let mut corner_ids = Vec::with_capacity(face.corners.len());
        for (ci, c) in face.corners.iter().enumerate() {
            let n = cn[fi][ci];
            let key = (
                c.vert,
                [c.uv.x.to_bits(), c.uv.y.to_bits()],
                [n.x.to_bits(), n.y.to_bits(), n.z.to_bits()],
            );
            let id = *lookup.entry(key).or_insert_with(|| {
                let v = &mesh.verts[c.vert as usize];
                out_verts.push(Vertex::with_skinning(
                    v.co.to_array(),
                    n.to_array(),
                    c.uv.to_array(),
                    v.color,
                    v.joints,
                    v.weights,
                ));
                (out_verts.len() - 1) as u32
            });
            corner_ids.push(id);
        }
        for [a, b, c] in triangulate_polygon(&pts) {
            indices.extend_from_slice(&[corner_ids[a], corner_ids[b], corner_ids[c]]);
        }
    }
    Ok(Mesh::new(mesh.name.clone(), out_verts, indices))
}

#[derive(Debug, Clone)]
pub struct FromCoreOptions {
    /// Distância de soldagem de posições (0 = só coincidentes exatas).
    pub weld_epsilon: f32,
    /// Guarda as normais do vértice de origem como normais customizadas
    /// (preserva a intenção de sombreamento de malhas autorais/VRM).
    pub keep_normals: bool,
}

impl Default for FromCoreOptions {
    fn default() -> Self {
        Self {
            weld_epsilon: 1e-5,
            keep_normals: true,
        }
    }
}

/// `anigo_core::Mesh` → `EditMesh`: solda vértices por posição, preserva UV
/// por corner, cor/skin do primeiro vértice do grupo e (opcionalmente) normais.
pub fn from_core_mesh(mesh: &Mesh, opts: &FromCoreOptions) -> ModelResult<EditMesh> {
    if mesh.indices.len() % 3 != 0 {
        return Err(ModelError::InvalidParameter(format!(
            "número de índices ({}) não é múltiplo de 3",
            mesh.indices.len()
        )));
    }
    let inv = if opts.weld_epsilon > 0.0 { 1.0 / opts.weld_epsilon } else { 1.0e7 };
    let quant = |p: [f32; 3]| -> [i64; 3] {
        [
            (p[0] * inv).round() as i64,
            (p[1] * inv).round() as i64,
            (p[2] * inv).round() as i64,
        ]
    };
    let mut out = EditMesh::new(mesh.name.clone());
    let mut map: HashMap<[i64; 3], u32> = HashMap::new();
    let mut remap: Vec<u32> = Vec::with_capacity(mesh.vertices.len());
    for v in &mesh.vertices {
        let id = *map.entry(quant(v.position)).or_insert_with(|| {
            let mut ev = EVert::new(Vec3::from_array(v.position));
            ev.color = v.color;
            ev.joints = v.joints;
            ev.weights = v.weights;
            out.push_vert(ev)
        });
        remap.push(id);
    }
    for tri in mesh.indices.chunks_exact(3) {
        for &i in tri {
            if i as usize >= mesh.vertices.len() {
                return Err(ModelError::VertexOutOfRange(i, mesh.vertices.len()));
            }
        }
        let ids = [remap[tri[0] as usize], remap[tri[1] as usize], remap[tri[2] as usize]];
        if ids[0] == ids[1] || ids[1] == ids[2] || ids[0] == ids[2] {
            continue; // triângulo degenerado após soldagem
        }
        let corners = tri
            .iter()
            .zip(ids)
            .map(|(&i, vert)| {
                let src = &mesh.vertices[i as usize];
                Corner {
                    vert,
                    uv: Vec2::from_array(src.uv),
                    normal: opts.keep_normals.then(|| Vec3::from_array(src.normal)),
                }
            })
            .collect();
        out.push_face(Face {
            corners,
            material: 0,
            smooth: true,
            face_set: 0,
            hidden: false,
        })?;
    }
    Ok(out)
}

/// Une triângulos coplanares adjacentes em quads (inverso do "triangulate").
/// Candidatos são ordenados por "quadratura" (cantos próximos de 90°), então
/// pares que formam quads bons vencem pares que só são convexos.
pub fn quadify(mesh: &mut EditMesh, max_angle_rad: f32) {
    let topo = crate::topology::Topology::build(mesh);
    let cos_limit = max_angle_rad.cos();
    let mut candidates: Vec<(f32, usize, usize, Vec<Corner>)> = Vec::new();
    for e in &topo.edges {
        if e.faces.len() != 2 {
            continue;
        }
        let (f0, f1) = (e.faces[0] as usize, e.faces[1] as usize);
        if mesh.faces[f0].len() != 3 || mesh.faces[f1].len() != 3 {
            continue;
        }
        let attr = mesh.edge_attr(e.verts.0, e.verts.1);
        if attr.sharp || attr.seam {
            continue;
        }
        if mesh.face_normal(f0 as u32).dot(mesh.face_normal(f1 as u32)) < cos_limit {
            continue;
        }
        let (a, b) = e.verts;
        let c0 = &mesh.faces[f0];
        let c1 = &mesh.faces[f1];
        let Some(i0) = (0..3).find(|&i| edge_key(c0.corners[i].vert, c0.corners[(i + 1) % 3].vert) == (a, b)) else {
            continue;
        };
        let (x, y) = (c0.corners[i0], c0.corners[(i0 + 1) % 3]);
        let apex0 = c0.corners[(i0 + 2) % 3];
        let Some(apex1) = c1.corners.iter().copied().find(|c| c.vert != a && c.vert != b) else {
            continue;
        };
        // f0 = (x, y, apex0) anti-horário; apex1 está do outro lado da aresta
        // compartilhada, então o quad anti-horário é (x, apex1, y, apex0).
        let quad = vec![x, apex1, y, apex0];
        let pts: Vec<Vec3> = quad.iter().map(|c| mesh.co(c.vert)).collect();
        let n = crate::mesh::newell_normal(&pts);
        let mut worst = 0.0f32;
        let mut convex = true;
        for i in 0..4 {
            let (p, c, q) = (pts[(i + 3) % 4], pts[i], pts[(i + 1) % 4]);
            if (c - p).cross(q - c).dot(n) <= 1e-10 {
                convex = false;
                break;
            }
            let ang = (p - c).normalize_or_zero().dot((q - c).normalize_or_zero()).clamp(-1.0, 1.0).acos();
            worst = worst.max((ang - std::f32::consts::FRAC_PI_2).abs());
        }
        if convex {
            candidates.push((worst, f0, f1, quad));
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut used = vec![false; mesh.faces.len()];
    let mut merged: Vec<Face> = Vec::new();
    for (_, f0, f1, quad) in candidates {
        if used[f0] || used[f1] {
            continue;
        }
        used[f0] = true;
        used[f1] = true;
        let mut nf = mesh.faces[f0].clone();
        nf.corners = quad;
        merged.push(nf);
    }
    let mut faces: Vec<Face> = Vec::with_capacity(mesh.faces.len());
    for (i, f) in std::mem::take(&mut mesh.faces).into_iter().enumerate() {
        if !used[i] {
            faces.push(f);
        }
    }
    faces.extend(merged);
    mesh.faces = faces;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    #[test]
    fn triangulate_concave_l_shape_covers_area_exactly() {
        // polígono em L (côncavo) no plano XZ, enrolamento anti-horário visto de +Y
        let pts = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 2.0),
            Vec3::new(1.0, 0.0, 2.0),
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::new(2.0, 0.0, 1.0),
            Vec3::new(2.0, 0.0, 0.0),
        ];
        let tris = triangulate_polygon(&pts);
        assert_eq!(tris.len(), 4);
        let area: f32 = tris
            .iter()
            .map(|t| (pts[t[1]] - pts[t[0]]).cross(pts[t[2]] - pts[t[0]]).length() * 0.5)
            .sum();
        assert!((area - 3.0).abs() < 1e-5, "área {area}");
        let n = crate::mesh::newell_normal(&pts);
        for t in &tris {
            assert!((pts[t[1]] - pts[t[0]]).cross(pts[t[2]] - pts[t[0]]).dot(n) > 0.0);
        }
    }

    #[test]
    fn to_core_mesh_cube_has_expected_counts_and_unit_normals() {
        let m = primitives::cube(2.0);
        let core = to_core_mesh(&m, &ToCoreOptions::default()).unwrap();
        // chapado: 6 faces × 4 cantos (normais distintas) = 24 vértices, 12 triângulos
        assert_eq!(core.vertices.len(), 24);
        assert_eq!(core.indices.len(), 36);
        for v in &core.vertices {
            let n = Vec3::from_array(v.normal);
            assert!((n.length() - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn smooth_sphere_shares_vertices() {
        let mut m = primitives::quad_sphere(1.0, 4);
        for f in &mut m.faces {
            for c in &mut f.corners {
                c.uv = Vec2::ZERO; // sem costuras de UV, vértices devem ser compartilhados
            }
        }
        let core = to_core_mesh(&m, &ToCoreOptions::default()).unwrap();
        assert_eq!(core.vertices.len(), m.vert_count());
    }

    #[test]
    fn core_roundtrip_preserves_geometry() {
        let core = anigo_core::Mesh::create_cube(2.0);
        let em = from_core_mesh(&core, &FromCoreOptions::default()).unwrap();
        assert_eq!(em.vert_count(), 8, "cubo do core tem 24 vértices que soldam em 8");
        assert_eq!(em.face_count(), 12);
        let back = to_core_mesh(&em, &ToCoreOptions::default()).unwrap();
        assert_eq!(back.indices.len(), core.indices.len());
        // normais customizadas preservadas → mesmas normais chapadas do cubo original
        for v in &back.vertices {
            let n = Vec3::from_array(v.normal);
            assert!((n.length() - 1.0).abs() < 1e-5);
            assert!(n.abs().max_element() > 0.999);
        }
    }

    #[test]
    fn quadify_restores_quads_from_triangulated_grid() {
        let g = primitives::grid(3, 3, 3.0, 3.0);
        let core = to_core_mesh(&g, &ToCoreOptions::default()).unwrap();
        let mut em = from_core_mesh(&core, &FromCoreOptions { keep_normals: false, ..Default::default() }).unwrap();
        assert_eq!(em.face_count(), 18);
        quadify(&mut em, 0.1);
        assert_eq!(em.face_count(), 9);
        assert!(em.faces.iter().all(|f| f.len() == 4));
        em.validate().unwrap();
    }

    #[test]
    fn odd_index_count_is_rejected() {
        let mut core = anigo_core::Mesh::create_cube(1.0);
        core.indices.pop();
        assert!(from_core_mesh(&core, &FromCoreOptions::default()).is_err());
    }
}
