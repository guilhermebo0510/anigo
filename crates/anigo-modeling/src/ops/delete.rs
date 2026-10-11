//! Delete (Blender: `X` / `Delete` em Edit Mode) e helpers de validação compartilhados
//! pelos operadores deste diretório (`merge`, `dissolve`, `fill`, `misc`…).
//!
//! # Modelo de arestas soltas
//!
//! A adjacência é derivada das faces; arestas *soltas* (wire) só existem como
//! entradas em `EditMesh::edge_attrs` (mesmo que com atributos padrão). Por isso
//! todo operador que faz uma aresta de face desaparecer precisa também podar o
//! atributo correspondente (senão a aresta "ressuscitaria" como wire) e, ao
//! contrário, uma aresta que deve *sobrar* como wire (ex.: "Only Faces") ganha
//! uma entrada padrão explícita. As funções `pub(crate)` abaixo encapsulam isso.
//!
//! Todas as funções públicas devolvem `OpOutput` com `remap = Some(..)` (índices
//! de vértices e faces compactados) e seleção vazia.

use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, EdgeAttr, EdgeKey, EditMesh, Remap};
use crate::ops::OpOutput;
use crate::select::Selection;
use crate::topology::Topology;
use std::collections::HashSet;

// ------------------------------------------------------------------ helpers comuns

/// Valida índices de vértice; ordena e remove duplicatas. `what` nomeia a
/// seleção na mensagem de `EmptySelection`.
pub(crate) fn unique_verts(
    mesh: &EditMesh,
    verts: &[u32],
    what: &'static str,
) -> ModelResult<Vec<u32>> {
    if verts.is_empty() {
        return Err(ModelError::EmptySelection(what));
    }
    let mut out = verts.to_vec();
    out.sort_unstable();
    out.dedup();
    if let Some(&v) = out.last() {
        if v as usize >= mesh.verts.len() {
            return Err(ModelError::VertexOutOfRange(v, mesh.verts.len()));
        }
    }
    Ok(out)
}

/// Valida índices de face; ordena e remove duplicatas.
pub(crate) fn unique_faces(
    mesh: &EditMesh,
    faces: &[u32],
    what: &'static str,
) -> ModelResult<Vec<u32>> {
    if faces.is_empty() {
        return Err(ModelError::EmptySelection(what));
    }
    let mut out = faces.to_vec();
    out.sort_unstable();
    out.dedup();
    if let Some(&f) = out.last() {
        if f as usize >= mesh.faces.len() {
            return Err(ModelError::FaceOutOfRange(f, mesh.faces.len()));
        }
    }
    Ok(out)
}

/// Valida arestas (devem existir na topologia, inclusive wire); chaves
/// canônicas, ordenadas e sem duplicatas.
pub(crate) fn unique_edges(
    topo: &Topology,
    edges: &[EdgeKey],
    what: &'static str,
) -> ModelResult<Vec<EdgeKey>> {
    if edges.is_empty() {
        return Err(ModelError::EmptySelection(what));
    }
    let mut out: Vec<EdgeKey> = edges.iter().map(|&(a, b)| edge_key(a, b)).collect();
    out.sort_unstable();
    out.dedup();
    for &(a, b) in &out {
        if topo.edge(a, b).is_none() {
            return Err(ModelError::MissingEdge(a, b));
        }
    }
    Ok(out)
}

/// Conjunto de arestas que pertencem a alguma face.
pub(crate) fn face_edge_set(mesh: &EditMesh) -> HashSet<EdgeKey> {
    let mut set = HashSet::with_capacity(mesh.faces.len() * 2);
    for f in &mesh.faces {
        for (a, b) in f.edges() {
            set.insert(edge_key(a, b));
        }
    }
    set
}

/// Remove os atributos de arestas que *eram* de face em `old_face_edges` e
/// deixaram de existir. Arestas wire pré-existentes (que não estavam em `old`)
/// são preservadas. Chamar ANTES de compactar vértices.
pub(crate) fn prune_vanished_edge_attrs(mesh: &mut EditMesh, old_face_edges: &HashSet<EdgeKey>) {
    let now = face_edge_set(mesh);
    mesh.edge_attrs
        .retain(|k, _| !old_face_edges.contains(k) || now.contains(k));
}

/// Mescla atributos de duas arestas que passaram a ser a mesma (OR de
/// sharp/seam, máximo de crease/bevel).
pub(crate) fn merge_edge_attr(a: EdgeAttr, b: EdgeAttr) -> EdgeAttr {
    EdgeAttr {
        sharp: a.sharp || b.sharp,
        seam: a.seam || b.seam,
        crease: a.crease.max(b.crease),
        bevel_weight: a.bevel_weight.max(b.bevel_weight),
    }
}

/// Remove vértices candidatos que ficaram sem face e sem aresta wire; devolve o
/// remap (faces: identidade). `candidates` em qualquer ordem.
pub(crate) fn remove_unused_candidates(mesh: &mut EditMesh, candidates: &[u32]) -> Remap {
    let used = mesh.used_verts();
    let mut keep = vec![true; mesh.verts.len()];
    let mut in_attr = vec![false; mesh.verts.len()];
    for &(a, b) in mesh.edge_attrs.keys() {
        in_attr[a as usize] = true;
        in_attr[b as usize] = true;
    }
    for &v in candidates {
        let i = v as usize;
        if i < keep.len() && !used[i] && !in_attr[i] {
            keep[i] = false;
        }
    }
    mesh.compact_verts(&keep)
}

// ------------------------------------------------------------------ núcleo de deleção

/// Descrição de uma deleção para [`apply_delete`].
pub(crate) struct DeletePlan<'a> {
    /// Faces a remover explicitamente.
    pub kill_faces: &'a [u32],
    /// Vértices a remover (levam consigo faces e arestas incidentes).
    pub kill_verts: &'a [u32],
    /// Arestas a remover (levam consigo as faces que as usam).
    pub drop_edges: &'a [EdgeKey],
    /// Arestas de faces removidas que ficam sem face sobram como wire (estilo
    /// "Only Faces" / "Vertices" / "Edges" do Blender) em vez de sumirem.
    pub promote_wire: bool,
    /// Vértices que devem ser removidos se terminarem sem face nem aresta.
    pub cleanup_verts: &'a [u32],
}

/// Aplica a deleção. Os índices devem ter sido validados pelo chamador.
pub(crate) fn apply_delete(mesh: &mut EditMesh, topo: &Topology, plan: &DeletePlan) -> Remap {
    let nv = mesh.verts.len();
    let nf = mesh.faces.len();
    let mut kill_v = vec![false; nv];
    for &v in plan.kill_verts {
        kill_v[v as usize] = true;
    }
    let drop: HashSet<EdgeKey> = plan.drop_edges.iter().copied().collect();
    let mut kill_f = vec![false; nf];
    for &f in plan.kill_faces {
        kill_f[f as usize] = true;
    }
    for (fi, face) in mesh.faces.iter().enumerate() {
        if kill_f[fi] {
            continue;
        }
        if face.verts().any(|v| kill_v[v as usize])
            || face.edges().any(|(a, b)| drop.contains(&edge_key(a, b)))
        {
            kill_f[fi] = true;
        }
    }

    // Destino dos atributos de aresta.
    let mut promote: Vec<EdgeKey> = Vec::new();
    let mut remove: Vec<EdgeKey> = Vec::new();
    for e in &topo.edges {
        let (a, b) = e.verts;
        if drop.contains(&e.verts) {
            remove.push(e.verts);
            continue;
        }
        if e.faces.is_empty() {
            continue; // wire pré-existente: fica (a menos que toque vértice removido)
        }
        if e.faces.iter().all(|&f| kill_f[f as usize]) {
            if kill_v[a as usize] || kill_v[b as usize] {
                remove.push(e.verts);
            } else if plan.promote_wire {
                promote.push(e.verts);
            } else {
                remove.push(e.verts);
            }
        }
    }
    for k in remove {
        mesh.edge_attrs.remove(&k);
    }
    for k in promote {
        mesh.edge_attrs.entry(k).or_default();
    }

    // Faces sobreviventes e mapa de faces.
    let mut face_map: Vec<Option<u32>> = vec![None; nf];
    let old = std::mem::take(&mut mesh.faces);
    for (fi, face) in old.into_iter().enumerate() {
        if !kill_f[fi] {
            face_map[fi] = Some(mesh.faces.len() as u32);
            mesh.faces.push(face);
        }
    }

    // Vértices.
    let used = mesh.used_verts();
    let mut in_attr = vec![false; nv];
    for &(a, b) in mesh.edge_attrs.keys() {
        in_attr[a as usize] = true;
        in_attr[b as usize] = true;
    }
    let mut keep: Vec<bool> = kill_v.iter().map(|k| !*k).collect();
    for &v in plan.cleanup_verts {
        let i = v as usize;
        if !used[i] && !in_attr[i] {
            keep[i] = false;
        }
    }
    let mut remap = mesh.compact_verts(&keep);
    remap.face = face_map;
    remap
}

fn output(remap: Remap) -> OpOutput {
    OpOutput {
        selection: Selection::default(),
        created_verts: Vec::new(),
        created_faces: Vec::new(),
        remap: Some(remap),
    }
}

// --------------------------------------------------------------------- API pública

/// Delete → *Vertices*: remove os vértices e todas as arestas/faces que os
/// usam. As demais arestas das faces removidas sobram como wire; vértices que
/// ficam isolados permanecem (use [`delete_loose`] para limpá-los).
///
/// Erros: seleção vazia ou índice fora do intervalo (malha intacta).
pub fn delete_verts(mesh: &mut EditMesh, verts: &[u32]) -> ModelResult<OpOutput> {
    let verts = unique_verts(mesh, verts, "vértices")?;
    let topo = Topology::build(mesh);
    let remap = apply_delete(
        mesh,
        &topo,
        &DeletePlan {
            kill_faces: &[],
            kill_verts: &verts,
            drop_edges: &[],
            promote_wire: true,
            cleanup_verts: &[],
        },
    );
    Ok(output(remap))
}

/// Delete → *Edges*: remove as arestas (inclusive wire) e as faces que as usam.
/// Vértices que ficam sem nenhuma aresta/face e eram extremos das arestas
/// removidas também são apagados; as outras arestas das faces removidas sobram
/// como wire.
///
/// Erros: seleção vazia ou aresta inexistente (`MissingEdge`).
pub fn delete_edges(mesh: &mut EditMesh, edges: &[EdgeKey]) -> ModelResult<OpOutput> {
    let topo = Topology::build(mesh);
    let edges = unique_edges(&topo, edges, "arestas")?;
    let endpoints: Vec<u32> = edges.iter().flat_map(|&(a, b)| [a, b]).collect();
    let remap = apply_delete(
        mesh,
        &topo,
        &DeletePlan {
            kill_faces: &[],
            kill_verts: &[],
            drop_edges: &edges,
            promote_wire: true,
            cleanup_verts: &endpoints,
        },
    );
    Ok(output(remap))
}

/// Delete → *Faces*: remove as faces e as arestas/vértices que só elas usavam
/// (mantém o que é compartilhado com faces restantes ou wire pré-existente).
pub fn delete_faces(mesh: &mut EditMesh, faces: &[u32]) -> ModelResult<OpOutput> {
    let faces = unique_faces(mesh, faces, "faces")?;
    let topo = Topology::build(mesh);
    let cleanup: Vec<u32> = faces
        .iter()
        .flat_map(|&f| mesh.faces[f as usize].verts().collect::<Vec<_>>())
        .collect();
    let remap = apply_delete(
        mesh,
        &topo,
        &DeletePlan {
            kill_faces: &faces,
            kill_verts: &[],
            drop_edges: &[],
            promote_wire: false,
            cleanup_verts: &cleanup,
        },
    );
    Ok(output(remap))
}

/// Delete → *Only Faces*: remove só as faces; vértices e arestas permanecem
/// (as arestas que ficam sem face viram wire). Nenhum vértice é compactado,
/// logo `remap.vert` é a identidade.
pub fn delete_faces_keep_verts(mesh: &mut EditMesh, faces: &[u32]) -> ModelResult<OpOutput> {
    let faces = unique_faces(mesh, faces, "faces")?;
    let topo = Topology::build(mesh);
    let remap = apply_delete(
        mesh,
        &topo,
        &DeletePlan {
            kill_faces: &faces,
            kill_verts: &[],
            drop_edges: &[],
            promote_wire: true,
            cleanup_verts: &[],
        },
    );
    Ok(output(remap))
}

/// Delete → *Only Edges & Faces*: remove as faces e as arestas indicadas (e as
/// faces que usam essas arestas), mas **mantém todos os vértices** (podem ficar
/// soltos). Arestas que ficam sem face e não foram listadas também somem.
///
/// Pelo menos uma das listas deve ser não vazia.
pub fn delete_only_edges_and_faces(
    mesh: &mut EditMesh,
    edges: &[EdgeKey],
    faces: &[u32],
) -> ModelResult<OpOutput> {
    if edges.is_empty() && faces.is_empty() {
        return Err(ModelError::EmptySelection("arestas ou faces"));
    }
    let topo = Topology::build(mesh);
    let edges = if edges.is_empty() {
        Vec::new()
    } else {
        unique_edges(&topo, edges, "arestas")?
    };
    let faces = if faces.is_empty() {
        Vec::new()
    } else {
        unique_faces(mesh, faces, "faces")?
    };
    let remap = apply_delete(
        mesh,
        &topo,
        &DeletePlan {
            kill_faces: &faces,
            kill_verts: &[],
            drop_edges: &edges,
            promote_wire: false,
            cleanup_verts: &[],
        },
    );
    Ok(output(remap))
}

/// Select → *Delete Loose* com o padrão do Blender: vértices e arestas soltos
/// (faces não). Equivale a `delete_loose_with(mesh, true, true, false)`.
pub fn delete_loose(mesh: &mut EditMesh) -> ModelResult<OpOutput> {
    delete_loose_with(mesh, true, true, false)
}

/// *Delete Loose*: apaga geometria solta.
/// * `use_verts`: vértices sem nenhuma aresta (inclui os que ficam isolados
///   após remover as arestas soltas);
/// * `use_edges`: arestas wire (sem face);
/// * `use_faces`: faces isoladas (todas as arestas pertencem só a ela).
///
/// Nada solto → no-op bem-sucedido (remap identidade).
pub fn delete_loose_with(
    mesh: &mut EditMesh,
    use_verts: bool,
    use_edges: bool,
    use_faces: bool,
) -> ModelResult<OpOutput> {
    let topo = Topology::build(mesh);
    let mut kill_verts: Vec<u32> = Vec::new();
    let mut drop_edges: Vec<EdgeKey> = Vec::new();
    let mut kill_faces: Vec<u32> = Vec::new();
    let mut cleanup: Vec<u32> = Vec::new();
    if use_edges {
        for e in &topo.edges {
            if e.faces.is_empty() {
                drop_edges.push(e.verts);
                cleanup.push(e.verts.0);
                cleanup.push(e.verts.1);
            }
        }
    }
    if use_faces {
        for (fi, edges) in topo.face_edges.iter().enumerate() {
            if edges
                .iter()
                .all(|&e| topo.edges[e as usize].faces.len() == 1)
            {
                kill_faces.push(fi as u32);
                cleanup.extend(mesh.faces[fi].verts());
            }
        }
    }
    if use_verts {
        for v in 0..mesh.verts.len() {
            if topo.vert_edges[v].is_empty() {
                kill_verts.push(v as u32);
            }
        }
    } else {
        cleanup.clear();
        // Sem `use_verts` as pontas das arestas/faces removidas permanecem.
    }
    let remap = apply_delete(
        mesh,
        &topo,
        &DeletePlan {
            kill_faces: &kill_faces,
            kill_verts: &kill_verts,
            drop_edges: &drop_edges,
            promote_wire: false,
            cleanup_verts: &cleanup,
        },
    );
    Ok(output(remap))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;
    use glam::Vec3;

    fn counts(m: &EditMesh) -> (usize, usize, usize) {
        let t = Topology::build(m);
        (m.vert_count(), t.edges.len(), m.face_count())
    }

    #[test]
    fn delete_verts_cube_corner_removes_three_faces() {
        let mut m = primitives::cube(1.0);
        let out = delete_verts(&mut m, &[0]).unwrap();
        m.validate().unwrap();
        assert_eq!(m.face_count(), 3);
        assert_eq!(m.vert_count(), 7);
        let r = out.remap.unwrap();
        assert_eq!(r.vert[0], None);
        assert_eq!(r.vert.iter().filter(|v| v.is_some()).count(), 7);
        assert_eq!(r.face.iter().filter(|f| f.is_none()).count(), 3);
    }

    #[test]
    fn delete_verts_deterministic_and_order_independent() {
        let mut a = primitives::quad_sphere(1.0, 3);
        let mut b = a.clone();
        delete_verts(&mut a, &[5, 9, 5, 11]).unwrap();
        delete_verts(&mut b, &[11, 9, 5]).unwrap();
        assert_eq!(a.content_hash(), b.content_hash());
        a.validate().unwrap();
    }

    #[test]
    fn delete_verts_errors_leave_mesh_intact() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        assert!(matches!(
            delete_verts(&mut m, &[]),
            Err(ModelError::EmptySelection(_))
        ));
        assert!(matches!(
            delete_verts(&mut m, &[0, 99]),
            Err(ModelError::VertexOutOfRange(99, 8))
        ));
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn delete_verts_keeps_surrounding_ring_as_wire_for_lone_quad() {
        let mut m = primitives::plane(2.0);
        delete_verts(&mut m, &[0]).unwrap();
        // 3 vértices restantes, 0 faces, 1 aresta wire entre os extremos não adjacentes a v0.
        assert_eq!(m.face_count(), 0);
        assert_eq!(m.vert_count(), 3);
        let t = Topology::build(&m);
        assert_eq!(t.edges.len(), 2);
        assert!(t.edges.iter().all(|e| e.faces.is_empty()));
    }

    #[test]
    fn delete_edges_interior_edge_of_grid_makes_hole_of_two_quads() {
        let mut m = primitives::grid(3, 3, 3.0, 3.0);
        let t = Topology::build(&m);
        let e = t
            .edges
            .iter()
            .find(|e| e.faces.len() == 2)
            .map(|e| e.verts)
            .unwrap();
        let before = m.face_count();
        delete_edges(&mut m, &[e]).unwrap();
        assert_eq!(m.face_count(), before - 2);
        m.validate().unwrap();
        // a aresta deletada não pode ter ressuscitado como wire
        let t2 = Topology::build(&m);
        // a aresta deletada não ressuscita (nem como wire)
        assert!(t2.edge(e.0, e.1).is_none());
    }

    #[test]
    fn delete_edges_errors() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        assert!(matches!(
            delete_edges(&mut m, &[]),
            Err(ModelError::EmptySelection(_))
        ));
        assert!(matches!(
            delete_edges(&mut m, &[(0, 99)]),
            Err(ModelError::MissingEdge(0, 99))
        ));
        // 0 e 6 são diagonais do cubo: não é aresta
        assert!(delete_edges(&mut m, &[(0, 6)]).is_err());
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn delete_edges_removes_wire_edge_and_isolated_endpoints() {
        let mut m = EditMesh::new("w");
        let a = m.add_vert(Vec3::ZERO);
        let b = m.add_vert(Vec3::X);
        let c = m.add_vert(Vec3::Y);
        m.edge_attrs.insert(edge_key(a, b), EdgeAttr::default());
        m.edge_attrs.insert(edge_key(b, c), EdgeAttr::default());
        let out = delete_edges(&mut m, &[(a, b)]).unwrap();
        // a ficou isolado e foi removido; b e c ainda ligados
        assert_eq!(m.vert_count(), 2);
        assert_eq!(out.remap.unwrap().vert[a as usize], None);
        assert_eq!(m.edge_attrs.len(), 1);
    }

    #[test]
    fn delete_faces_removes_exclusive_verts_and_edges() {
        let mut m = primitives::cube(1.0);
        // remove uma face: os 4 vértices continuam (usados por outras faces)
        delete_faces(&mut m, &[0]).unwrap();
        assert_eq!(m.face_count(), 5);
        assert_eq!(m.vert_count(), 8);
        let t = Topology::build(&m);
        assert_eq!(t.boundary_edge_count(), 4);
        assert_eq!(t.euler_characteristic(&m), 1);

        // plano isolado: some tudo
        let mut p = primitives::plane(1.0);
        delete_faces(&mut p, &[0]).unwrap();
        assert!(p.is_empty());
        assert!(p.edge_attrs.is_empty());
    }

    #[test]
    fn delete_faces_prunes_edge_attrs_of_vanished_edges() {
        let mut m = primitives::plane(1.0);
        m.set_sharp(0, 1, true);
        m.set_seam(2, 3, true);
        delete_faces(&mut m, &[0]).unwrap();
        assert!(m.edge_attrs.is_empty());
    }

    #[test]
    fn delete_faces_keep_verts_leaves_wire_edges_and_all_verts() {
        let mut m = primitives::plane(1.0);
        let out = delete_faces_keep_verts(&mut m, &[0]).unwrap();
        assert_eq!(m.face_count(), 0);
        assert_eq!(m.vert_count(), 4);
        let t = Topology::build(&m);
        assert_eq!(t.edges.len(), 4);
        assert!(t.edges.iter().all(|e| e.faces.is_empty()));
        let r = out.remap.unwrap();
        assert!(r.vert.iter().enumerate().all(|(i, v)| *v == Some(i as u32)));
        // delete_loose remove tudo
        delete_loose(&mut m).unwrap();
        assert!(m.is_empty());
    }

    #[test]
    fn delete_only_edges_and_faces_keeps_verts() {
        let mut m = primitives::cube(1.0);
        let out = delete_only_edges_and_faces(&mut m, &[], &[0, 1]).unwrap();
        assert_eq!(m.vert_count(), 8);
        assert_eq!(m.face_count(), 4);
        out.remap.unwrap();
        // por aresta: aresta do cubo mata 2 faces
        let mut c = primitives::cube(1.0);
        let t = Topology::build(&c);
        let e = t.edges[0].verts;
        delete_only_edges_and_faces(&mut c, &[e], &[]).unwrap();
        assert_eq!(c.face_count(), 4);
        assert_eq!(c.vert_count(), 8);
        assert!(matches!(
            delete_only_edges_and_faces(&mut c, &[], &[]),
            Err(ModelError::EmptySelection(_))
        ));
        assert!(delete_only_edges_and_faces(&mut c, &[(0, 99)], &[]).is_err());
        assert!(delete_only_edges_and_faces(&mut c, &[], &[77]).is_err());
    }

    #[test]
    fn delete_loose_removes_isolated_verts_and_wire() {
        let mut m = primitives::cube(1.0);
        let loose = m.add_vert(Vec3::splat(5.0));
        let w0 = m.add_vert(Vec3::splat(6.0));
        let w1 = m.add_vert(Vec3::splat(7.0));
        m.edge_attrs.insert(edge_key(w0, w1), EdgeAttr::default());
        let _ = loose;
        let out = delete_loose(&mut m).unwrap();
        assert_eq!(m.vert_count(), 8);
        assert_eq!(m.face_count(), 6);
        assert!(m.edge_attrs.is_empty());
        let r = out.remap.unwrap();
        assert_eq!(r.vert[8], None);
        // idempotente
        let h = m.content_hash();
        delete_loose(&mut m).unwrap();
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn delete_loose_faces_option() {
        let mut m = primitives::cube(1.0);
        m.append(&primitives::plane(1.0));
        // padrão não toca faces
        delete_loose(&mut m).unwrap();
        assert_eq!(m.face_count(), 7);
        delete_loose_with(&mut m, true, true, true).unwrap();
        assert_eq!(m.face_count(), 6);
        assert_eq!(m.vert_count(), 8);
        // use_verts = false mantém pontas das arestas soltas
        let mut w = EditMesh::new("w");
        let a = w.add_vert(Vec3::ZERO);
        let b = w.add_vert(Vec3::X);
        w.edge_attrs.insert(edge_key(a, b), EdgeAttr::default());
        delete_loose_with(&mut w, false, true, false).unwrap();
        assert_eq!(w.vert_count(), 2);
        assert!(w.edge_attrs.is_empty());
    }

    #[test]
    fn delete_preserves_attributes_of_survivors() {
        let mut m = primitives::cube(1.0);
        m.set_sharp(4, 5, true);
        m.verts[5].mask = 0.7;
        m.faces[5].material = 3;
        let out = delete_faces(&mut m, &[0]).unwrap();
        let r = out.remap.unwrap();
        let v5 = r.vert[5].unwrap();
        assert!((m.verts[v5 as usize].mask - 0.7).abs() < 1e-6);
        let f5 = r.face[5].unwrap();
        assert_eq!(m.faces[f5 as usize].material, 3);
        assert!(m.edge_attrs.values().any(|e| e.sharp));
    }

    #[test]
    fn delete_on_large_mesh_is_fast_and_valid() {
        let mut m = primitives::quad_sphere(1.0, 64);
        assert!(m.face_count() > 24_000);
        let verts: Vec<u32> = (0..m.vert_count() as u32).step_by(7).collect();
        let t0 = std::time::Instant::now();
        delete_verts(&mut m, &verts).unwrap();
        m.validate().unwrap();
        assert!(t0.elapsed().as_secs() < 20);
        let counts = counts(&m);
        assert!(counts.2 > 0);
    }
}
