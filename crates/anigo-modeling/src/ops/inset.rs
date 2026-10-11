//! Inset (região e individual) — equivalente ao `I` do Blender.
//!
//! O inset cria um anel de quads entre a fronteira original e uma fronteira
//! deslocada para dentro; as faces internas permanecem selecionadas (para que
//! o usuário extrude em seguida e forme um "botão").
//!
//! # Deslocamento por vértice
//! Cada vértice da fronteira se move ao longo da bissetriz dos vetores
//! "para dentro" das arestas vizinhas. Com `use_even_offset` o deslocamento é
//! escalado por `1 / sin(θ/2)` (θ = ângulo interno) para que a espessura do anel
//! seja uniforme, com limite [`MAX_EVEN_FACTOR`] para ângulos muito agudos.

use super::extrude::{
    face_pts_uvs, prune_orphan_edge_attrs, uv_at_shifted, validated_faces, wire_edge_keys,
};
use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, EdgeAttr, EdgeKey, EditMesh, Face};
use crate::ops::OpOutput;
use crate::select::Selection;
use crate::topology::Topology;
use glam::{Vec2, Vec3};
use std::collections::{BTreeMap, HashMap};

/// Limite do fator `1/sin(θ/2)` do offset uniforme (ângulos agudos demais seriam explosivos).
pub const MAX_EVEN_FACTOR: f32 = 8.0;

fn check_params(thickness: f32, depth: f32) -> ModelResult<()> {
    if !thickness.is_finite() || thickness < 0.0 {
        return Err(ModelError::InvalidParameter(format!(
            "espessura do inset deve ser finita e >= 0 (recebeu {thickness})"
        )));
    }
    if !depth.is_finite() {
        return Err(ModelError::InvalidParameter(format!(
            "profundidade do inset não finita: {depth}"
        )));
    }
    if thickness == 0.0 && depth == 0.0 {
        return Err(ModelError::InvalidParameter(
            "inset com espessura e profundidade zero não faz nada".into(),
        ));
    }
    Ok(())
}

/// Combina vetores unitários "para dentro" de arestas num deslocamento de
/// comprimento `t` (ou `t / sin(θ/2)` limitado, se `even`).
fn combine_inward(ins: &[Vec3], t: f32, even: bool) -> Vec3 {
    if ins.is_empty() {
        return Vec3::ZERO;
    }
    let sum: Vec3 = ins.iter().copied().sum();
    let len = sum.length();
    if len < 1e-6 {
        // arestas opostas (faixa fina): sem bissetriz definida
        return ins[0] * t;
    }
    let dir = sum / len;
    let mag = if even {
        t * (ins.len() as f32 / len).min(MAX_EVEN_FACTOR)
    } else {
        t
    };
    dir * mag
}

/// Inset de região (`I` com várias faces selecionadas).
///
/// * `thickness`: largura do anel (>= 0), em unidades da malha.
/// * `depth`: deslocamento das faces internas ao longo da normal (pode ser negativo).
/// * `use_outset`: o anel nasce **para fora** da região (a região mantém sua
///   forma e os vértices originais da fronteira se afastam).
/// * `use_boundary`: também insere as arestas da região que são fronteira da
///   malha (sem isso, esses vértices ficam presos e deslizam pela borda).
/// * `use_even_offset`: espessura uniforme nos cantos (escala `1/sin(θ/2)`).
///
/// Arestas internas da região não geram anel. A região mantém os índices de
/// face; os quads do anel são anexados ao fim. Atributos de aresta da fronteira
/// são copiados para a fronteira interna. UVs: a fronteira interna usa um
/// mapeamento afim local (o inset "encolhe" a textura); no outset o anel
/// herda a UV da vizinhança.
///
/// Seleção devolvida: as faces internas (a região). `created_faces`: o anel.
pub fn inset_faces_region(
    mesh: &mut EditMesh,
    faces: &[u32],
    thickness: f32,
    depth: f32,
    use_outset: bool,
    use_boundary: bool,
    use_even_offset: bool,
) -> ModelResult<OpOutput> {
    check_params(thickness, depth)?;
    let region = validated_faces(mesh, faces, "inset de região precisa de ao menos uma face")?;
    for &f in &region {
        if mesh.face_normal(f) == Vec3::ZERO {
            return Err(ModelError::Numeric(format!(
                "face {f} degenerada (normal nula); inset indefinido"
            )));
        }
    }
    let topo = Topology::build(mesh);
    let wires = wire_edge_keys(mesh);
    let pos: Vec<Vec3> = mesh.verts.iter().map(|v| v.co).collect();
    let mut in_region = vec![false; mesh.faces.len()];
    for &f in &region {
        in_region[f as usize] = true;
    }
    let mut region_count: HashMap<u32, u32> = HashMap::new();
    for &f in &region {
        for &e in &topo.face_edges[f as usize] {
            *region_count.entry(e).or_insert(0) += 1;
        }
    }
    let normals: HashMap<u32, Vec3> = region.iter().map(|&f| (f, mesh.face_normal(f))).collect();

    // arestas de anel (f, i, a, b), vetores "para dentro" por vértice e arestas presas
    let mut ring: Vec<(u32, usize, u32, u32)> = Vec::new();
    let mut inward: BTreeMap<u32, Vec<Vec3>> = BTreeMap::new();
    let mut pinned_dirs: HashMap<u32, Vec<u32>> = HashMap::new();
    for &f in &region {
        let face = &mesh.faces[f as usize];
        for (i, (a, b)) in face.edges().enumerate() {
            let e = topo.face_edges[f as usize][i];
            if region_count.get(&e).copied().unwrap_or(0) != 1 {
                continue;
            }
            let on_mesh_border = topo.edges[e as usize].faces.len() == 1;
            if on_mesh_border && !use_boundary {
                pinned_dirs.entry(a).or_default().push(b);
                pinned_dirs.entry(b).or_default().push(a);
                continue;
            }
            ring.push((f, i, a, b));
            let dir = pos[b as usize] - pos[a as usize];
            let inw = normals[&f].cross(dir).normalize_or_zero();
            inward.entry(a).or_default().push(inw);
            inward.entry(b).or_default().push(inw);
        }
    }
    if ring.is_empty() {
        return Err(ModelError::UnsupportedTopology(
            "a região não tem fronteira inserível (casca fechada ou apenas bordas da malha com use_boundary=false)"
                .into(),
        ));
    }

    // deslocamento no plano por vértice da fronteira
    let mut shift: HashMap<u32, Vec3> = HashMap::new();
    for (&v, ins) in &inward {
        let base = combine_inward(ins, thickness, use_even_offset);
        let mut s = base;
        if ins.len() == 1 {
            if let Some(others) = pinned_dirs.get(&v) {
                if others.len() == 1 {
                    // desliza ao longo da borda presa até alcançar a espessura
                    let mut u = (pos[others[0] as usize] - pos[v as usize]).normalize_or_zero();
                    let mut d = u.dot(ins[0]);
                    if d < 0.0 {
                        u = -u;
                        d = -d;
                    }
                    if d > 0.1 {
                        s = u * (thickness / d);
                    }
                }
            }
        }
        shift.insert(v, if use_outset { -s } else { s });
    }

    // normal por vértice da região para a profundidade
    let mut vnormal: HashMap<u32, Vec3> = HashMap::new();
    for &f in &region {
        for v in mesh.faces[f as usize].verts() {
            *vnormal.entry(v).or_insert(Vec3::ZERO) += normals[&f];
        }
    }

    // ---- mutação
    let mut dup: HashMap<u32, u32> = HashMap::new();
    let mut created_verts = Vec::new();
    for &v in inward.keys() {
        let nv = mesh.push_vert(mesh.verts[v as usize].clone());
        dup.insert(v, nv);
        created_verts.push(nv);
    }
    let depth_vec = |v: u32| -> Vec3 {
        vnormal
            .get(&v)
            .map(|n| n.normalize_or_zero() * depth)
            .unwrap_or(Vec3::ZERO)
    };
    for (&v, &nv) in &dup {
        let s = shift[&v];
        if use_outset {
            // a região mantém a forma; os originais (vizinhança) se afastam
            mesh.verts[v as usize].co = pos[v as usize] + s;
            mesh.verts[nv as usize].co = pos[v as usize] + depth_vec(v);
        } else {
            mesh.verts[nv as usize].co = pos[v as usize] + s + depth_vec(v);
        }
    }
    // vértices internos da região (sem duplicata) só recebem profundidade
    if depth != 0.0 {
        for &v in vnormal.keys() {
            if dup.contains_key(&v) {
                continue;
            }
            let all_region = topo.vert_faces[v as usize]
                .iter()
                .all(|&g| in_region[g as usize]);
            if all_region {
                mesh.verts[v as usize].co = pos[v as usize] + depth_vec(v);
            }
        }
    }

    // reescreve as faces da região e guarda UV deslocada por (face, corner)
    let mut new_uv: HashMap<(u32, usize), Vec2> = HashMap::new();
    let snapshot: HashMap<u32, Face> = region
        .iter()
        .map(|&f| (f, mesh.faces[f as usize].clone()))
        .collect();
    let mut attr_moves: Vec<(EdgeKey, EdgeAttr)> = Vec::new();
    for &f in &region {
        let old = &snapshot[&f];
        let (pts, uvs) = face_pts_uvs(&pos, old);
        let n = old.corners.len();
        for i in 0..n {
            let v = old.corners[i].vert;
            let uv = match (dup.get(&v), use_outset) {
                (Some(_), false) => uv_at_shifted(&pts, &uvs, i, pts[i] + shift[&v]),
                _ => uvs[i],
            };
            new_uv.insert((f, i), uv);
            if let Some(&nv) = dup.get(&v) {
                let c = &mut mesh.faces[f as usize].corners[i];
                c.vert = nv;
                c.uv = uv;
            }
        }
        for (a, b) in old.edges() {
            let (na, nb) = (
                dup.get(&a).copied().unwrap_or(a),
                dup.get(&b).copied().unwrap_or(b),
            );
            if (na, nb) != (a, b) {
                let attr = mesh.edge_attr(a, b);
                if !attr.is_default() {
                    attr_moves.push((edge_key(na, nb), attr));
                }
            }
        }
    }
    let mut created_faces = Vec::new();
    for &(f, i, a, b) in &ring {
        let old = &snapshot[&f];
        let n = old.corners.len();
        let j = (i + 1) % n;
        let (na, nb) = (dup[&a], dup[&b]);
        let (ua, ub) = (old.corners[i].uv, old.corners[j].uv);
        let mut quad = old.with_verts_from(&[a, b, nb, na]);
        if use_outset {
            // anel fora da região: a UV externa segue a vizinhança, se existir
            let (pts, uvs) = face_pts_uvs(&pos, old);
            let (oa, ob) = match topo
                .edge_faces(a, b)
                .iter()
                .copied()
                .find(|&g| g != f && !in_region[g as usize])
            {
                Some(g) => {
                    let gf = &mesh.faces[g as usize];
                    (
                        super::extrude::corner_uv(gf, a).unwrap_or(ua),
                        super::extrude::corner_uv(gf, b).unwrap_or(ub),
                    )
                }
                None => (
                    uv_at_shifted(&pts, &uvs, i, pos[a as usize] + shift[&a]),
                    uv_at_shifted(&pts, &uvs, j, pos[b as usize] + shift[&b]),
                ),
            };
            quad.corners[0].uv = oa;
            quad.corners[1].uv = ob;
            quad.corners[2].uv = ub;
            quad.corners[3].uv = ua;
        } else {
            quad.corners[0].uv = ua;
            quad.corners[1].uv = ub;
            quad.corners[2].uv = new_uv[&(f, j)];
            quad.corners[3].uv = new_uv[&(f, i)];
        }
        mesh.faces.push(quad);
        created_faces.push((mesh.faces.len() - 1) as u32);
    }
    for (k, attr) in attr_moves {
        mesh.edge_attrs.insert(k, attr);
    }
    prune_orphan_edge_attrs(mesh, &wires);
    Ok(OpOutput {
        selection: Selection::from_faces(mesh, region.iter().copied()),
        created_verts,
        created_faces,
        remap: None,
    })
}

/// Inset individual: cada face é inserida sobre si mesma (sempre com offset
/// uniforme), gerando um anel de quads por face e uma face interna.
///
/// * `thickness`: largura do anel (>= 0); `depth`: deslocamento da face
///   interna ao longo da normal da própria face.
///
/// Os vértices originais permanecem (as faces vizinhas não mudam); cada face
/// ganha vértices internos exclusivos. Seleção devolvida: as faces internas.
pub fn inset_faces_individual(
    mesh: &mut EditMesh,
    faces: &[u32],
    thickness: f32,
    depth: f32,
) -> ModelResult<OpOutput> {
    check_params(thickness, depth)?;
    let region = validated_faces(mesh, faces, "inset individual precisa de ao menos uma face")?;
    for &f in &region {
        if mesh.face_normal(f) == Vec3::ZERO {
            return Err(ModelError::Numeric(format!(
                "face {f} degenerada (normal nula); inset indefinido"
            )));
        }
    }
    let wires = wire_edge_keys(mesh);
    let pos: Vec<Vec3> = mesh.verts.iter().map(|v| v.co).collect();
    let mut created_verts = Vec::new();
    let mut created_faces = Vec::new();
    let mut attr_moves: Vec<(EdgeKey, EdgeAttr)> = Vec::new();
    for &f in &region {
        let old = mesh.faces[f as usize].clone();
        let n_f = mesh.face_normal(f);
        let (pts, uvs) = face_pts_uvs(&pos, &old);
        let n = old.corners.len();
        let mut inner: Vec<u32> = Vec::with_capacity(n);
        let mut inner_uv: Vec<Vec2> = Vec::with_capacity(n);
        for i in 0..n {
            let (p_prev, p, p_next) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
            let in_prev = n_f.cross(p - p_prev).normalize_or_zero();
            let in_next = n_f.cross(p_next - p).normalize_or_zero();
            let s = combine_inward(&[in_prev, in_next], thickness, true);
            let nv = mesh.push_vert(mesh.verts[old.corners[i].vert as usize].clone());
            mesh.verts[nv as usize].co = p + s + n_f * depth;
            created_verts.push(nv);
            inner.push(nv);
            inner_uv.push(uv_at_shifted(&pts, &uvs, i, p + s));
        }
        for i in 0..n {
            let c = &mut mesh.faces[f as usize].corners[i];
            c.vert = inner[i];
            c.uv = inner_uv[i];
        }
        for i in 0..n {
            let j = (i + 1) % n;
            let (a, b) = (old.corners[i].vert, old.corners[j].vert);
            let attr = mesh.edge_attr(a, b);
            if !attr.is_default() {
                attr_moves.push((edge_key(inner[i], inner[j]), attr));
            }
            let mut quad = old.with_verts_from(&[a, b, inner[j], inner[i]]);
            quad.corners[0].uv = uvs[i];
            quad.corners[1].uv = uvs[j];
            quad.corners[2].uv = inner_uv[j];
            quad.corners[3].uv = inner_uv[i];
            mesh.faces.push(quad);
            created_faces.push((mesh.faces.len() - 1) as u32);
        }
    }
    for (k, attr) in attr_moves {
        mesh.edge_attrs.insert(k, attr);
    }
    prune_orphan_edge_attrs(mesh, &wires);
    Ok(OpOutput {
        selection: Selection::from_faces(mesh, region.iter().copied()),
        created_verts,
        created_faces,
        remap: None,
    })
}

// -------------------------------------------------------------------- testes

#[cfg(test)]
mod tests {
    use super::super::extrude::test_util::*;
    use super::super::extrude::*;
    use super::*;
    use crate::primitives;

    fn face_with_normal(m: &EditMesh, n: Vec3) -> u32 {
        (0..m.face_count() as u32)
            .max_by(|&a, &b| {
                m.face_normal(a)
                    .dot(n)
                    .partial_cmp(&m.face_normal(b).dot(n))
                    .unwrap()
            })
            .unwrap()
    }

    #[test]
    fn cube_face_inset_geometry_and_topology() {
        let mut m = primitives::cube(2.0);
        let f = face_with_normal(&m, Vec3::Y);
        let area0 = m.face_area(f);
        let out = inset_faces_region(&mut m, &[f], 0.25, 0.0, false, true, true).unwrap();
        assert_eq!(m.vert_count(), 12);
        assert_eq!(m.face_count(), 10);
        assert_eq!(out.created_faces.len(), 4);
        assert_closed(&m, Some(2));
        assert_outward_star(&m);
        assert!((area0 - 4.0).abs() < 1e-5);
        assert!(
            (m.face_area(f) - 2.25).abs() < 1e-4,
            "área interna {}",
            m.face_area(f)
        );
        assert!((signed_volume(&m) - 8.0).abs() < 1e-4);
        assert!(out.selection.faces.contains(&f) && out.selection.faces.len() == 1);
        // todos os vértices internos permanecem no plano do topo
        for v in m.faces[f as usize].verts() {
            assert!((m.co(v).y - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn inset_then_extrude_makes_a_button() {
        let mut m = primitives::cube(2.0);
        let f = face_with_normal(&m, Vec3::Y);
        inset_faces_region(&mut m, &[f], 0.4, 0.0, false, true, true).unwrap();
        extrude_faces_region_along_normal(&mut m, &[f], 0.3).unwrap();
        assert_closed(&m, Some(2));
        assert_outward_star(&m);
        // cubo 8 + botão (1.2 x 1.2 x 0.3)
        assert!((signed_volume(&m) - (8.0 + 1.44 * 0.3)).abs() < 1e-3);
        assert_eq!(m.face_count(), 6 + 4 + 4);
    }

    #[test]
    fn inset_depth_moves_along_normal() {
        let mut m = primitives::cube(2.0);
        let f = face_with_normal(&m, Vec3::Y);
        inset_faces_region(&mut m, &[f], 0.2, -0.3, false, true, true).unwrap();
        for v in m.faces[f as usize].verts() {
            assert!((m.co(v).y - 0.7).abs() < 1e-5);
        }
        assert_closed(&m, Some(2));
        // thickness 0 com depth != 0 vale (só afunda)
        let mut m2 = primitives::cube(2.0);
        let f2 = face_with_normal(&m2, Vec3::Y);
        inset_faces_region(&mut m2, &[f2], 0.0, 0.5, false, true, true).unwrap();
        assert_closed(&m2, Some(2));
    }

    #[test]
    fn even_offset_gives_uniform_thickness_on_a_triangle() {
        // triângulo equilátero no plano XZ
        let build = || {
            let mut m = EditMesh::new("tri");
            let a = m.add_vert(Vec3::new(0.0, 0.0, 0.0));
            let b = m.add_vert(Vec3::new(2.0, 0.0, 0.0));
            let c = m.add_vert(Vec3::new(1.0, 0.0, -(3.0f32).sqrt()));
            m.add_face(&[a, b, c]).unwrap();
            // normal +Y?
            if m.face_normal(0).y < 0.0 {
                m.flip_face(0).unwrap();
            }
            m
        };
        let dist_to_edge = |m: &EditMesh, f: u32| -> f32 {
            // distância do vértice interno 0 ao lado original a-b (z=0)
            let v = m.faces[f as usize].corners[0].vert;
            let p = m.co(v);
            // distância à aresta original mais próxima entre as três
            let tri = [
                (Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)),
                (
                    Vec3::new(2.0, 0.0, 0.0),
                    Vec3::new(1.0, 0.0, -(3.0f32).sqrt()),
                ),
                (
                    Vec3::new(1.0, 0.0, -(3.0f32).sqrt()),
                    Vec3::new(0.0, 0.0, 0.0),
                ),
            ];
            tri.iter()
                .map(|&(a, b)| {
                    let d = (b - a).normalize();
                    let w = p - a;
                    (w - d * w.dot(d)).length()
                })
                .fold(f32::MAX, f32::min)
        };
        let mut even = build();
        inset_faces_region(&mut even, &[0], 0.2, 0.0, false, true, true).unwrap();
        // com offset uniforme, a distância às arestas é exatamente 0.2
        assert!(
            (dist_to_edge(&even, 0) - 0.2).abs() < 1e-4,
            "{}",
            dist_to_edge(&even, 0)
        );
        let mut non_even = build();
        inset_faces_region(&mut non_even, &[0], 0.2, 0.0, false, true, false).unwrap();
        // sem offset uniforme, o vértice anda 0.2 na bissetriz: distância = 0.2*sin(30deg) = 0.1
        assert!(
            (dist_to_edge(&non_even, 0) - 0.1).abs() < 1e-4,
            "{}",
            dist_to_edge(&non_even, 0)
        );
    }

    #[test]
    fn outset_keeps_region_shape_and_grows_ring_outside() {
        let mut m = primitives::cube_subdivided(2.0, 3);
        let f = (0..m.face_count() as u32)
            .find(|&f| {
                m.face_normal(f).y > 0.9
                    && m.face_center(f).length() < 1.0 + 1e-3
                    && m.face_center(f).x.abs() < 1e-3
                    && m.face_center(f).z.abs() < 1e-3
            })
            .unwrap();
        let center_before = m.face_center(f);
        let area_before = m.face_area(f);
        let out = inset_faces_region(&mut m, &[f], 0.1, 0.0, true, true, true).unwrap();
        assert_closed(&m, Some(2));
        assert!((m.face_center(f) - center_before).length() < 1e-5);
        assert!(
            (m.face_area(f) - area_before).abs() < 1e-5,
            "região mantém forma"
        );
        // o anel está fora: cada quad do anel tem área > 0
        for &q in &out.created_faces {
            assert!(m.face_area(q) > 1e-4);
        }
        assert_eq!(out.created_faces.len(), 4);
    }

    #[test]
    fn region_inset_of_2x2_patch_has_no_ring_on_shared_edges() {
        let mut m = primitives::cube_subdivided(2.0, 4);
        let seed = face_with_normal(&m, Vec3::Y);
        let topo = Topology::build(&m);
        let mut faces = vec![seed];
        faces.extend(
            topo.face_neighbors(seed)
                .into_iter()
                .filter(|&g| m.face_normal(g).y > 0.9)
                .take(1),
        );
        let out = inset_faces_region(&mut m, &faces, 0.05, 0.0, false, true, true).unwrap();
        assert_eq!(out.created_faces.len(), 6); // perímetro de 2 quads
        assert_closed(&m, Some(2));
    }

    #[test]
    fn use_boundary_false_pins_mesh_border() {
        let mut m = primitives::grid(3, 3, 3.0, 3.0);
        // canto: face 0 toca a borda em dois lados
        let corner = (0..m.face_count() as u32)
            .find(|&f| m.face_center(f).x < -0.9 && m.face_center(f).z < -0.9)
            .unwrap();
        let with = {
            let mut g = m.clone();
            inset_faces_region(&mut g, &[corner], 0.2, 0.0, false, true, true).unwrap()
        };
        assert_eq!(with.created_faces.len(), 4);
        let out = inset_faces_region(&mut m, &[corner], 0.2, 0.0, false, false, true).unwrap();
        assert_eq!(out.created_faces.len(), 2, "só as 2 arestas internas");
        m.validate().unwrap();
        let t = Topology::build(&m);
        assert_eq!(t.non_manifold_edge_count(), 0);
        // a borda da malha ganha 2 vértices (os que deslizaram): 12 + 2 arestas
        assert_eq!(t.boundary_edge_count(), 14);
        // o vértice do canto externo não se moveu
        let corner_v = (0..m.vert_count() as u32)
            .find(|&v| (m.co(v) - Vec3::new(-1.5, 0.0, -1.5)).length() < 1e-5);
        assert!(corner_v.is_some());
        // todas as faces ainda apontam para +Y
        for f in 0..m.face_count() as u32 {
            assert!(m.face_normal(f).y > 0.99, "face {f} virou");
        }
    }

    #[test]
    fn individual_inset_each_face_gets_a_ring() {
        let mut m = primitives::cube(2.0);
        let all: Vec<u32> = (0..6).collect();
        let out = inset_faces_individual(&mut m, &all, 0.3, 0.0).unwrap();
        assert_eq!(out.created_faces.len(), 24);
        assert_eq!(out.created_verts.len(), 24);
        assert_closed(&m, Some(2));
    }

    #[test]
    fn inset_propagates_uv_and_attrs() {
        let mut m = primitives::grid(1, 1, 1.0, 1.0);
        m.faces[0].material = 5;
        let verts: Vec<u32> = m.faces[0].verts().collect();
        m.set_seam(verts[0], verts[1], true);
        let out = inset_faces_region(&mut m, &[0], 0.1, 0.0, false, true, true).unwrap();
        // UV da face interna: 0.1 .. 0.9
        for c in &m.faces[0].corners {
            assert!(
                c.uv.x > 0.09 && c.uv.x < 0.91 && c.uv.y > 0.09 && c.uv.y < 0.91,
                "{:?}",
                c.uv
            );
            assert!((c.uv.x - 0.1).abs() < 1e-4 || (c.uv.x - 0.9).abs() < 1e-4);
        }
        // o anel mantém material e UV externa original
        for &q in &out.created_faces {
            assert_eq!(m.faces[q as usize].material, 5);
        }
        // seam copiada para a aresta interna
        let cv: Vec<u32> = m.faces[0].verts().collect();
        assert!(m.edge_attr(cv[0], cv[1]).seam);
        assert!(m.edge_attr(verts[0], verts[1]).seam);
    }

    #[test]
    fn inset_on_quad_sphere_region_keeps_manifold_and_euler() {
        let mut m = primitives::quad_sphere(1.0, 8);
        let seed = 10u32;
        let sel = crate::select::grow_faces(&m, &Selection::from_faces(&m, [seed]));
        let faces: Vec<u32> = sel.faces.iter().copied().collect();
        let before_area: f32 = faces.iter().map(|&f| m.face_area(f)).sum();
        inset_faces_region(&mut m, &faces, 0.03, 0.0, false, true, true).unwrap();
        assert_closed(&m, Some(2));
        let after_area: f32 = faces.iter().map(|&f| m.face_area(f)).sum();
        assert!(after_area < before_area);
        assert_outward_star(&m);
    }

    #[test]
    fn invalid_params_and_selection_leave_mesh_intact() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        assert!(matches!(
            inset_faces_region(&mut m, &[0], -0.1, 0.0, false, true, true),
            Err(ModelError::InvalidParameter(_))
        ));
        assert!(matches!(
            inset_faces_region(&mut m, &[0], f32::NAN, 0.0, false, true, true),
            Err(ModelError::InvalidParameter(_))
        ));
        assert!(matches!(
            inset_faces_region(&mut m, &[0], 0.1, f32::INFINITY, false, true, true),
            Err(ModelError::InvalidParameter(_))
        ));
        assert!(matches!(
            inset_faces_region(&mut m, &[0], 0.0, 0.0, false, true, true),
            Err(ModelError::InvalidParameter(_))
        ));
        assert!(matches!(
            inset_faces_region(&mut m, &[], 0.1, 0.0, false, true, true),
            Err(ModelError::EmptySelection(_))
        ));
        assert!(matches!(
            inset_faces_region(&mut m, &[99], 0.1, 0.0, false, true, true),
            Err(ModelError::FaceOutOfRange(99, 6))
        ));
        // casca fechada inteira não tem fronteira
        let all: Vec<u32> = (0..6).collect();
        assert!(matches!(
            inset_faces_region(&mut m, &all, 0.1, 0.0, false, true, true),
            Err(ModelError::UnsupportedTopology(_))
        ));
        assert!(matches!(
            inset_faces_individual(&mut m, &[0], -1.0, 0.0),
            Err(ModelError::InvalidParameter(_))
        ));
        assert!(matches!(
            inset_faces_individual(&mut m, &[40], 0.1, 0.0),
            Err(ModelError::FaceOutOfRange(40, 6))
        ));
        assert_eq!(h, m.content_hash());
        // face degenerada
        let mut d = EditMesh::new("deg");
        for p in [Vec3::ZERO, Vec3::X, Vec3::X * 2.0] {
            d.add_vert(p);
        }
        d.add_face(&[0, 1, 2]).unwrap();
        let hd = d.content_hash();
        assert!(matches!(
            inset_faces_region(&mut d, &[0], 0.1, 0.0, false, true, true),
            Err(ModelError::Numeric(_))
        ));
        assert_eq!(hd, d.content_hash());
    }

    #[test]
    fn inset_is_deterministic_and_fast_on_large_mesh() {
        let run = || {
            let mut m = primitives::quad_sphere(1.0, 64);
            let faces: Vec<u32> = (0..m.face_count() as u32).filter(|f| f % 3 == 0).collect();
            let t0 = std::time::Instant::now();
            inset_faces_individual(&mut m, &faces, 0.005, 0.0).unwrap();
            assert!(t0.elapsed().as_secs() < 30);
            m.content_hash()
        };
        assert_eq!(run(), run());
        let mut m = primitives::quad_sphere(1.0, 64);
        let faces: Vec<u32> = (0..m.face_count() as u32)
            .filter(|f| m.face_center(*f).y > 0.5)
            .collect();
        let t0 = std::time::Instant::now();
        inset_faces_region(&mut m, &faces, 0.003, 0.0, false, true, true).unwrap();
        assert!(t0.elapsed().as_secs() < 30);
        assert_closed(&m, Some(2));
    }
}
