//! Adjacência derivada de uma [`EditMesh`]. Reconstruída sob demanda (O(n));
//! nunca é fonte de verdade.

use crate::mesh::{edge_key, EdgeKey, EditMesh};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct EdgeInfo {
    pub verts: EdgeKey,
    /// Faces que usam a aresta (com multiplicidade; >2 = não-manifold).
    pub faces: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct Topology {
    pub edges: Vec<EdgeInfo>,
    pub edge_index: HashMap<EdgeKey, u32>,
    /// Faces incidentes por vértice.
    pub vert_faces: Vec<Vec<u32>>,
    /// Arestas incidentes por vértice.
    pub vert_edges: Vec<Vec<u32>>,
    /// Para cada face, índice de aresta de cada corner i→i+1.
    pub face_edges: Vec<Vec<u32>>,
}

impl Topology {
    pub fn build(mesh: &EditMesh) -> Self {
        let mut edges: Vec<EdgeInfo> = Vec::new();
        let mut edge_index: HashMap<EdgeKey, u32> = HashMap::new();
        let mut vert_faces = vec![Vec::new(); mesh.verts.len()];
        let mut vert_edges: Vec<Vec<u32>> = vec![Vec::new(); mesh.verts.len()];
        let mut face_edges = Vec::with_capacity(mesh.faces.len());

        let mut get_edge = |a: u32, b: u32, edges: &mut Vec<EdgeInfo>, vert_edges: &mut Vec<Vec<u32>>| {
            let key = edge_key(a, b);
            *edge_index.entry(key).or_insert_with(|| {
                let idx = edges.len() as u32;
                edges.push(EdgeInfo {
                    verts: key,
                    faces: Vec::new(),
                });
                vert_edges[key.0 as usize].push(idx);
                vert_edges[key.1 as usize].push(idx);
                idx
            })
        };

        for (fi, face) in mesh.faces.iter().enumerate() {
            let mut fe = Vec::with_capacity(face.corners.len());
            for (a, b) in face.edges() {
                let e = get_edge(a, b, &mut edges, &mut vert_edges);
                edges[e as usize].faces.push(fi as u32);
                fe.push(e);
            }
            for c in &face.corners {
                vert_faces[c.vert as usize].push(fi as u32);
            }
            face_edges.push(fe);
        }
        // Arestas soltas: atributos de aresta cujo par não pertence a nenhuma face.
        for &(a, b) in mesh.edge_attrs.keys() {
            get_edge(a, b, &mut edges, &mut vert_edges);
        }
        Self {
            edges,
            edge_index,
            vert_faces,
            vert_edges,
            face_edges,
        }
    }

    pub fn edge(&self, a: u32, b: u32) -> Option<&EdgeInfo> {
        self.edge_index
            .get(&edge_key(a, b))
            .map(|&i| &self.edges[i as usize])
    }

    pub fn edge_faces(&self, a: u32, b: u32) -> &[u32] {
        self.edge(a, b).map(|e| e.faces.as_slice()).unwrap_or(&[])
    }

    pub fn is_boundary_edge(&self, a: u32, b: u32) -> bool {
        self.edge(a, b).is_some_and(|e| e.faces.len() == 1)
    }

    pub fn is_wire_edge(&self, a: u32, b: u32) -> bool {
        self.edge(a, b).is_some_and(|e| e.faces.is_empty())
    }

    pub fn is_manifold_edge(&self, a: u32, b: u32) -> bool {
        self.edge(a, b).is_some_and(|e| e.faces.len() == 2)
    }

    /// Vértices vizinhos (ligados por aresta), ordenados e sem repetição.
    pub fn vert_neighbors(&self, v: u32) -> Vec<u32> {
        let mut out: Vec<u32> = self.vert_edges[v as usize]
            .iter()
            .map(|&e| {
                let (a, b) = self.edges[e as usize].verts;
                if a == v {
                    b
                } else {
                    a
                }
            })
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    pub fn valence(&self, v: u32) -> usize {
        self.vert_edges[v as usize].len()
    }

    pub fn is_boundary_vert(&self, v: u32) -> bool {
        self.vert_edges[v as usize].iter().any(|&e| {
            let n = self.edges[e as usize].faces.len();
            n != 2
        })
    }

    /// Faces vizinhas (compartilham aresta), ordenadas e sem repetição.
    pub fn face_neighbors(&self, f: u32) -> Vec<u32> {
        let mut out = Vec::new();
        for &e in &self.face_edges[f as usize] {
            for &g in &self.edges[e as usize].faces {
                if g != f {
                    out.push(g);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Número de arestas de fronteira (1 face) — 0 para sólidos fechados.
    pub fn boundary_edge_count(&self) -> usize {
        self.edges.iter().filter(|e| e.faces.len() == 1).count()
    }

    pub fn non_manifold_edge_count(&self) -> usize {
        self.edges.iter().filter(|e| e.faces.len() > 2).count()
    }

    /// Malha fechada e 2-manifold (cada aresta tem exatamente 2 faces).
    pub fn is_closed_manifold(&self) -> bool {
        !self.edges.is_empty() && self.edges.iter().all(|e| e.faces.len() == 2)
    }

    /// Característica de Euler V − E + F (considera só vértices usados).
    pub fn euler_characteristic(&self, mesh: &EditMesh) -> i64 {
        let used = mesh.used_verts().iter().filter(|u| **u).count() as i64;
        used - self.edges.len() as i64 + mesh.faces.len() as i64
    }

    /// Componentes conectados por face (via arestas compartilhadas). Retorna
    /// o id de componente de cada face.
    pub fn face_components(&self, mesh: &EditMesh) -> (Vec<u32>, u32) {
        let mut comp = vec![u32::MAX; mesh.faces.len()];
        let mut next = 0u32;
        for start in 0..mesh.faces.len() {
            if comp[start] != u32::MAX {
                continue;
            }
            let mut stack = vec![start as u32];
            comp[start] = next;
            while let Some(f) = stack.pop() {
                for g in self.face_neighbors(f) {
                    if comp[g as usize] == u32::MAX {
                        comp[g as usize] = next;
                        stack.push(g);
                    }
                }
            }
            next += 1;
        }
        (comp, next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    #[test]
    fn cube_is_closed_manifold_with_euler_2() {
        let m = primitives::cube(1.0);
        let t = Topology::build(&m);
        assert!(t.is_closed_manifold());
        assert_eq!(t.edges.len(), 12);
        assert_eq!(t.euler_characteristic(&m), 2);
        assert_eq!(t.boundary_edge_count(), 0);
    }

    #[test]
    fn plane_has_boundary_and_valence() {
        let m = primitives::grid(2, 2, 1.0, 1.0);
        let t = Topology::build(&m);
        assert!(!t.is_closed_manifold());
        assert_eq!(t.boundary_edge_count(), 8);
        // vértice central tem valência 4 e não é de fronteira
        let center = (0..m.vert_count() as u32)
            .find(|&v| m.co(v).length() < 1e-6)
            .unwrap();
        assert_eq!(t.valence(center), 4);
        assert!(!t.is_boundary_vert(center));
    }

    #[test]
    fn components_are_counted() {
        let mut m = primitives::cube(1.0);
        let other = primitives::cube(1.0);
        m.append(&other);
        let t = Topology::build(&m);
        assert_eq!(t.face_components(&m).1, 2);
    }
}
