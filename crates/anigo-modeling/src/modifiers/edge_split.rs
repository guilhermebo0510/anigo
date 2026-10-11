//! Modificador **Edge Split**: duplica vértices ao longo de arestas duras
//! (marcadas `sharp`) e/ou de arestas cujo ângulo diedral excede um limite,
//! separando as faces (equivalente ao Edge Split Modifier do Blender).
//!
//! Útil para exportar para engines que exigem uma normal por vértice (cada
//! quina dura vira um par de vértices) e para cel shading com facetas nítidas.
//! O UV e as normais customizadas por corner são preservados: só o índice de
//! vértice dos cantos muda.

use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, EditMesh};
use crate::topology::Topology;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Parâmetros do Edge Split para a pilha de modificadores.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EdgeSplitParams {
    /// Limite do ângulo diedral em **radianos** (`0..=π`); `None` desliga o corte por ângulo.
    pub angle: Option<f32>,
    /// Corta as arestas marcadas como `sharp`.
    pub use_sharp: bool,
}

impl Default for EdgeSplitParams {
    fn default() -> Self {
        Self {
            angle: Some(30.0f32.to_radians()),
            use_sharp: true,
        }
    }
}

/// Separa as faces ao longo de arestas duras e devolve a malha nova.
///
/// * `angle`: arestas com exatamente 2 faces cujo ângulo entre as normais das
///   faces seja **maior** que `angle` (radianos, `0..=π`) são cortadas. `None`
///   ignora o ângulo.
/// * `use_sharp`: corta arestas com `EdgeAttr::sharp` (inclusive não-manifold,
///   onde todas as faces ficam independentes).
/// * Resultado: para cada vértice, as faces ao redor são agrupadas por
///   conectividade através das arestas *não* cortadas; cada grupo ganha o seu
///   vértice (o primeiro grupo mantém o índice original, os demais são anexados no
///   fim). Arestas de fronteira e soltas não são afetadas. Atributos de aresta
///   (`sharp`, `seam`, `crease`, `bevel_weight`) são copiados para as arestas
///   resultantes em ambos os lados do corte.
/// * Sem critério ativo (`angle = None` e `use_sharp = false`) devolve um clone.
///
/// Erros: `angle` não finito ou fora de `0..=π`; malha inválida.
pub fn edge_split(mesh: &EditMesh, angle: Option<f32>, use_sharp: bool) -> ModelResult<EditMesh> {
    if let Some(a) = angle {
        if !a.is_finite() || !(0.0..=std::f32::consts::PI).contains(&a) {
            return Err(ModelError::InvalidParameter(format!(
                "angle deve estar em 0..=π radianos, recebeu {a}"
            )));
        }
    }
    mesh.validate()?;
    if (angle.is_none() && !use_sharp) || mesh.faces.is_empty() {
        return Ok(mesh.clone());
    }
    let topo = Topology::build(mesh);
    let face_normals: Vec<_> = (0..mesh.faces.len() as u32).map(|f| mesh.face_normal(f)).collect();
    let cos_limit = angle.map(|a| a.cos());

    let split: Vec<bool> = topo
        .edges
        .iter()
        .map(|e| {
            let sharp = use_sharp && mesh.edge_attr(e.verts.0, e.verts.1).sharp;
            if sharp && e.faces.len() >= 2 {
                return true;
            }
            if e.faces.len() == 2 {
                if let Some(cl) = cos_limit {
                    let d = face_normals[e.faces[0] as usize].dot(face_normals[e.faces[1] as usize]);
                    return d < cl - 1e-6;
                }
            }
            false
        })
        .collect();

    // novo índice de vértice por (face, corner)
    let mut new_vert: Vec<Vec<u32>> = mesh.faces.iter().map(|f| f.corners.iter().map(|c| c.vert).collect()).collect();
    let mut extra: Vec<u32> = Vec::new(); // vértice original de cada duplicata
    let n = mesh.verts.len();
    for v in 0..n as u32 {
        let incident = &topo.vert_faces[v as usize];
        if incident.len() < 2 {
            continue;
        }
        // usos do vértice (face, corner), na ordem das faces incidentes
        let mut uses: Vec<(u32, usize)> = Vec::new();
        for &f in incident {
            if uses.iter().any(|u| u.0 == f) {
                continue;
            }
            if let Some(ci) = mesh.faces[f as usize].corners.iter().position(|c| c.vert == v) {
                uses.push((f, ci));
            }
        }
        if uses.len() < 2 {
            continue;
        }
        let mut parent: Vec<usize> = (0..uses.len()).collect();
        fn find(parent: &mut [usize], mut x: usize) -> usize {
            while parent[x] != x {
                parent[x] = parent[parent[x]];
                x = parent[x];
            }
            x
        }
        for &e in &topo.vert_edges[v as usize] {
            if split[e as usize] {
                continue;
            }
            let faces = &topo.edges[e as usize].faces;
            let mut first: Option<usize> = None;
            for &f in faces {
                if let Some(ui) = uses.iter().position(|u| u.0 == f) {
                    match first {
                        None => first = Some(ui),
                        Some(fu) => {
                            let (ra, rb) = (find(&mut parent, fu), find(&mut parent, ui));
                            if ra != rb {
                                let (lo, hi) = (ra.min(rb), ra.max(rb));
                                parent[hi] = lo;
                            }
                        }
                    }
                }
            }
        }
        // grupos em ordem do menor uso; o primeiro mantém o índice original
        let mut group_vert: BTreeMap<usize, u32> = BTreeMap::new();
        for (ui, &(f, ci)) in uses.iter().enumerate() {
            let root = find(&mut parent, ui);
            let vid = match group_vert.get(&root) {
                Some(&id) => id,
                None => {
                    let id = if group_vert.is_empty() {
                        v
                    } else {
                        extra.push(v);
                        (n + extra.len() - 1) as u32
                    };
                    group_vert.insert(root, id);
                    id
                }
            };
            new_vert[f as usize][ci] = vid;
        }
    }

    let mut out = EditMesh::new(mesh.name.clone());
    out.verts = mesh.verts.clone();
    for &orig in &extra {
        out.verts.push(mesh.verts[orig as usize].clone());
    }
    out.faces = mesh.faces.clone();
    for (f, face) in out.faces.iter_mut().enumerate() {
        for (c, corner) in face.corners.iter_mut().enumerate() {
            corner.vert = new_vert[f][c];
        }
    }
    // atributos de aresta: copiados para cada instância da aresta; arestas soltas ficam como estão
    for (f, face) in mesh.faces.iter().enumerate() {
        let k = face.corners.len();
        for i in 0..k {
            let (a, b) = (face.corners[i].vert, face.corners[(i + 1) % k].vert);
            let attr = mesh.edge_attr(a, b);
            if !attr.is_default() {
                out.edge_attrs
                    .insert(edge_key(new_vert[f][i], new_vert[f][(i + 1) % k]), attr);
            }
        }
    }
    for e in &topo.edges {
        if e.faces.is_empty() {
            let attr = mesh.edge_attr(e.verts.0, e.verts.1);
            if !attr.is_default() {
                out.edge_attrs.insert(e.verts, attr);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::{self, assert_outward};
    use glam::Vec3;

    #[test]
    fn cube_with_small_angle_becomes_six_separate_faces() {
        let c = primitives::cube(2.0);
        let m = edge_split(&c, Some(30.0f32.to_radians()), false).unwrap();
        m.validate().unwrap();
        assert_eq!(m.vert_count(), 24);
        assert_eq!(m.face_count(), 6);
        let t = Topology::build(&m);
        assert_eq!(t.face_components(&m).1, 6);
        assert_eq!(t.boundary_edge_count(), 24);
        // posições preservadas: cada vértice duplicado coincide com um original
        for v in &m.verts {
            assert!(c.verts.iter().any(|o| o.co == v.co));
        }
        assert_outward(&m);
    }

    #[test]
    fn angle_above_dihedral_keeps_mesh_identical() {
        let c = primitives::cube(2.0);
        // 90° < 100°: nenhuma aresta passa do limite
        let m = edge_split(&c, Some(100.0f32.to_radians()), false).unwrap();
        assert_eq!(m, c);
        // None + false = clone
        assert_eq!(edge_split(&c, None, false).unwrap(), c);
        // plano: ângulo 0 não corta nada (diedral exatamente 0)
        let g = primitives::grid(4, 4, 1.0, 1.0);
        assert_eq!(edge_split(&g, Some(0.0), true).unwrap().content_hash(), g.content_hash());
        // π (180°) nunca corta
        let s = primitives::quad_sphere(1.0, 3);
        assert_eq!(edge_split(&s, Some(std::f32::consts::PI), true).unwrap(), s);
    }

    #[test]
    fn sharp_equator_splits_sphere_in_two_closed_shells_boundaries() {
        let mut s = primitives::quad_sphere(1.0, 4);
        let topo = Topology::build(&s);
        let mut marked = 0;
        for e in &topo.edges {
            if s.co(e.verts.0).y.abs() < 1e-6 && s.co(e.verts.1).y.abs() < 1e-6 {
                s.set_sharp(e.verts.0, e.verts.1, true);
                s.set_seam(e.verts.0, e.verts.1, true);
                marked += 1;
            }
        }
        assert_eq!(marked, 16);
        let m = edge_split(&s, None, true).unwrap();
        m.validate().unwrap();
        assert_eq!(m.vert_count(), s.vert_count() + 16);
        let t = Topology::build(&m);
        assert_eq!(t.face_components(&m).1, 2);
        assert_eq!(t.boundary_edge_count(), 32);
        assert_eq!(t.non_manifold_edge_count(), 0);
        // os atributos aparecem nas duas costuras
        assert_eq!(m.edge_attrs.values().filter(|a| a.sharp && a.seam).count(), 32);
        // use_sharp = false ignora a marcação
        let ignored = edge_split(&s, None, false).unwrap();
        assert_eq!(ignored, s);
        // idempotente: arestas cortadas viraram fronteira
        let again = edge_split(&m, None, true).unwrap();
        assert_eq!(again.vert_count(), m.vert_count());
    }

    #[test]
    fn uv_and_custom_normals_are_preserved() {
        let mut c = primitives::cube(2.0);
        for f in &mut c.faces {
            for (i, k) in f.corners.iter_mut().enumerate() {
                k.normal = Some(Vec3::new(0.0, 1.0, i as f32).normalize());
            }
        }
        let m = edge_split(&c, Some(0.1), false).unwrap();
        for (a, b) in c.faces.iter().zip(&m.faces) {
            for (ca, cb) in a.corners.iter().zip(&b.corners) {
                assert_eq!(ca.uv, cb.uv);
                assert_eq!(ca.normal, cb.normal);
                assert_eq!(c.co(ca.vert), m.co(cb.vert));
            }
            assert_eq!(a.material, b.material);
        }
    }

    #[test]
    fn vertex_attributes_are_cloned_to_duplicates() {
        let mut c = primitives::cube(2.0);
        for (i, v) in c.verts.iter_mut().enumerate() {
            v.mask = i as f32 / 10.0;
            v.joints = [i as u16, 0, 0, 0];
        }
        let m = edge_split(&c, Some(0.1), false).unwrap();
        for v in &m.verts {
            let o = c.verts.iter().find(|o| o.co == v.co).unwrap();
            assert_eq!(o.mask, v.mask);
            assert_eq!(o.joints, v.joints);
        }
    }

    #[test]
    fn boundary_and_wire_edges_are_untouched() {
        let mut g = primitives::grid(2, 2, 2.0, 2.0);
        // fronteira marcada como sharp não deve criar vértices novos
        let topo = Topology::build(&g);
        for e in &topo.edges {
            if e.faces.len() == 1 {
                g.set_sharp(e.verts.0, e.verts.1, true);
            }
        }
        let m = edge_split(&g, Some(0.01), true).unwrap();
        assert_eq!(m.vert_count(), g.vert_count());
        assert_eq!(m.edge_attrs.len(), g.edge_attrs.len());
        // aresta solta com atributo sobrevive
        let mut w = primitives::cube(1.0);
        let a = w.add_vert(Vec3::splat(5.0));
        let b = w.add_vert(Vec3::splat(6.0));
        w.set_seam(a, b, true);
        let r = edge_split(&w, Some(0.1), true).unwrap();
        assert!(r.edge_attr(a, b).seam);
    }

    #[test]
    fn non_manifold_edge_only_splits_when_sharp() {
        let mut m = EditMesh::new("nm");
        for p in [Vec3::ZERO, Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z] {
            m.add_vert(p);
        }
        m.add_face(&[0, 1, 2]).unwrap();
        m.add_face(&[1, 0, 3]).unwrap();
        m.add_face(&[0, 1, 4]).unwrap();
        // sem sharp e sem ângulo: nada muda (aresta com 3 faces não é cortada pelo ângulo)
        assert_eq!(edge_split(&m, Some(0.1), false).unwrap().vert_count(), 5);
        m.set_sharp(0, 1, true);
        let s = edge_split(&m, None, true).unwrap();
        // 3 faces independentes na aresta (0,1): 2 vértices extras para cada ponta
        assert_eq!(s.vert_count(), 5 + 4);
        assert_eq!(Topology::build(&s).non_manifold_edge_count(), 0);
        s.validate().unwrap();
    }

    #[test]
    fn invalid_angle_is_rejected_and_input_intact() {
        let c = primitives::cube(1.0);
        for bad in [-0.1, 4.0, f32::NAN, f32::INFINITY] {
            assert!(matches!(edge_split(&c, Some(bad), true), Err(ModelError::InvalidParameter(_))));
        }
        let mut broken = c.clone();
        broken.faces[1].corners[1].vert = 100;
        assert!(edge_split(&broken, Some(0.1), true).is_err());
        assert_eq!(c, primitives::cube(1.0));
        let empty = EditMesh::new("e");
        assert_eq!(edge_split(&empty, Some(0.1), true).unwrap(), empty);
    }

    #[test]
    fn character_shapes_and_determinism() {
        // cápsula com limite de 40°: polos suaves ficam unidos, só quinas fortes se separam
        let cap = primitives::capsule(0.5, 1.0, 12, 4);
        let a = edge_split(&cap, Some(40.0f32.to_radians()), false).unwrap();
        let b = edge_split(&cap, Some(40.0f32.to_radians()), false).unwrap();
        assert_eq!(a.content_hash(), b.content_hash());
        a.validate().unwrap();
        // cilindro com tampas: tampa vira ilha separada, laterais seguem conectadas
        let cyl = primitives::cylinder(1.0, 2.0, 16, 2, true);
        let m = edge_split(&cyl, Some(60.0f32.to_radians()), false).unwrap();
        let t = Topology::build(&m);
        assert_eq!(t.face_components(&m).1, 3);
        assert_eq!(m.vert_count(), cyl.vert_count() + 32);
    }

    #[test]
    fn large_mesh_is_fast() {
        let s = primitives::quad_sphere(1.0, 64);
        let t = std::time::Instant::now();
        let m = edge_split(&s, Some(0.0001), true).unwrap();
        m.validate().unwrap();
        assert!(m.vert_count() > s.vert_count());
        assert!(t.elapsed().as_secs_f32() < 20.0);
    }

    #[test]
    fn serde_roundtrip() {
        let p = EdgeSplitParams::default();
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<EdgeSplitParams>(&json).unwrap(), p);
    }
}
