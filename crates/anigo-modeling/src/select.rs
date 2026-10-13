//! Seleção de elementos (vértices/arestas/faces) com sincronização entre modos.

use crate::mesh::{edge_key, EdgeKey, EditMesh};
use crate::topology::Topology;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectMode {
    Vertex,
    Edge,
    Face,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    pub verts: BTreeSet<u32>,
    pub edges: BTreeSet<EdgeKey>,
    pub faces: BTreeSet<u32>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.verts.is_empty() && self.edges.is_empty() && self.faces.is_empty()
    }

    pub fn clear(&mut self) {
        *self = Selection::default();
    }

    /// Seleção a partir de faces: vértices e arestas derivados.
    pub fn from_faces(mesh: &EditMesh, faces: impl IntoIterator<Item = u32>) -> Self {
        let mut s = Selection::default();
        for f in faces {
            let Some(face) = mesh.faces.get(f as usize) else {
                continue;
            };
            s.faces.insert(f);
            for (a, b) in face.edges() {
                s.verts.insert(a);
                s.edges.insert(edge_key(a, b));
            }
        }
        s
    }

    pub fn from_verts(verts: impl IntoIterator<Item = u32>) -> Self {
        Self {
            verts: verts.into_iter().collect(),
            ..Default::default()
        }
    }

    /// Seleção a partir de arestas: vértices derivados.
    pub fn from_edges(edges: impl IntoIterator<Item = EdgeKey>) -> Self {
        let mut s = Selection::default();
        for (a, b) in edges {
            s.edges.insert(edge_key(a, b));
            s.verts.insert(a);
            s.verts.insert(b);
        }
        s
    }

    pub fn select_all(mesh: &EditMesh) -> Self {
        let mut s = Selection::from_faces(mesh, 0..mesh.faces.len() as u32);
        let topo = Topology::build(mesh);
        for e in &topo.edges {
            s.edges.insert(e.verts);
            s.verts.insert(e.verts.0);
            s.verts.insert(e.verts.1);
        }
        for (i, used) in mesh.used_verts().iter().enumerate() {
            if *used {
                s.verts.insert(i as u32);
            }
        }
        s
    }

    pub fn invert(&self, mesh: &EditMesh, mode: SelectMode) -> Selection {
        let topo = Topology::build(mesh);
        match mode {
            SelectMode::Vertex => Selection::from_verts(
                (0..mesh.vert_count() as u32).filter(|v| !self.verts.contains(v)),
            ),
            SelectMode::Edge => Selection::from_edges(
                topo.edges
                    .iter()
                    .map(|e| e.verts)
                    .filter(|k| !self.edges.contains(k)),
            ),
            SelectMode::Face => Selection::from_faces(
                mesh,
                (0..mesh.face_count() as u32).filter(|f| !self.faces.contains(f)),
            ),
        }
    }

    /// Faces cujos vértices estão **todos** selecionados.
    pub fn faces_fully_selected(&self, mesh: &EditMesh) -> Vec<u32> {
        mesh.faces
            .iter()
            .enumerate()
            .filter(|(_, f)| f.verts().all(|v| self.verts.contains(&v)))
            .map(|(i, _)| i as u32)
            .collect()
    }

    /// Arestas com ambos os vértices selecionados e existentes na topologia.
    pub fn edges_fully_selected(&self, topo: &Topology) -> Vec<EdgeKey> {
        topo.edges
            .iter()
            .map(|e| e.verts)
            .filter(|(a, b)| self.verts.contains(a) && self.verts.contains(b))
            .collect()
    }

    /// Reduz a seleção ao modo dado e propaga (flush) para os demais níveis.
    pub fn flush(&self, mesh: &EditMesh, mode: SelectMode) -> Selection {
        let topo = Topology::build(mesh);
        match mode {
            SelectMode::Vertex => {
                let mut s = Selection::from_verts(self.verts.iter().copied());
                s.edges = self.edges_fully_selected(&topo).into_iter().collect();
                s.faces = self.faces_fully_selected(mesh).into_iter().collect();
                s
            }
            SelectMode::Edge => {
                let mut s = Selection::from_edges(self.edges.iter().copied());
                s.faces = mesh
                    .faces
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| f.edges().all(|(a, b)| s.edges.contains(&edge_key(a, b))))
                    .map(|(i, _)| i as u32)
                    .collect();
                s
            }
            SelectMode::Face => Selection::from_faces(mesh, self.faces.iter().copied()),
        }
    }
}

/// Expande a seleção de faces em um anel de vizinhança (grow).
pub fn grow_faces(mesh: &EditMesh, sel: &Selection) -> Selection {
    let topo = Topology::build(mesh);
    let mut faces = sel.faces.clone();
    for &f in &sel.faces {
        // vizinhança por vértice (como o "More" do Blender)
        if let Some(face) = mesh.faces.get(f as usize) {
            for v in face.verts() {
                faces.extend(topo.vert_faces[v as usize].iter().copied());
            }
        }
    }
    Selection::from_faces(mesh, faces)
}

/// Contrai a seleção de faces (shrink): remove faces com algum vizinho
/// (por vértice) fora da seleção.
pub fn shrink_faces(mesh: &EditMesh, sel: &Selection) -> Selection {
    let topo = Topology::build(mesh);
    let keep = sel.faces.iter().copied().filter(|&f| {
        mesh.faces[f as usize]
            .verts()
            .all(|v| topo.vert_faces[v as usize].iter().all(|g| sel.faces.contains(g)))
    });
    Selection::from_faces(mesh, keep)
}

/// Seleciona tudo que está ligado (por arestas) às faces/vértices atuais.
pub fn select_linked(mesh: &EditMesh, sel: &Selection) -> Selection {
    let topo = Topology::build(mesh);
    let (comp, _) = topo.face_components(mesh);
    let mut wanted: BTreeSet<u32> = sel.faces.iter().map(|&f| comp[f as usize]).collect();
    for &v in &sel.verts {
        for &f in &topo.vert_faces[v as usize] {
            wanted.insert(comp[f as usize]);
        }
    }
    Selection::from_faces(
        mesh,
        (0..mesh.face_count() as u32).filter(|&f| wanted.contains(&comp[f as usize])),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    #[test]
    fn grow_then_shrink_roundtrips_on_a_grid_patch() {
        let m = primitives::grid(5, 5, 5.0, 5.0);
        let center = Selection::from_faces(&m, [12u32]);
        let grown = grow_faces(&m, &center);
        assert_eq!(grown.faces.len(), 9);
        let back = shrink_faces(&m, &grown);
        assert_eq!(back.faces, center.faces);
    }

    #[test]
    fn linked_selects_whole_component() {
        let mut m = primitives::cube(1.0);
        m.append(&primitives::cube(1.0));
        let s = select_linked(&m, &Selection::from_faces(&m, [0u32]));
        assert_eq!(s.faces.len(), 6);
    }

    #[test]
    fn invert_faces() {
        let m = primitives::cube(1.0);
        let s = Selection::from_faces(&m, [0u32, 1]);
        assert_eq!(s.invert(&m, SelectMode::Face).faces.len(), 4);
    }

    #[test]
    fn flush_vertex_mode_derives_faces() {
        let m = primitives::cube(1.0);
        let all = Selection::from_verts(0..8);
        let f = all.flush(&m, SelectMode::Vertex);
        assert_eq!(f.faces.len(), 6);
        assert_eq!(f.edges.len(), 12);
    }
}
