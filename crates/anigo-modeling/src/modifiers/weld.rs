//! Modificador **Weld** (solda por distância), equivalente a "Merge by Distance"
//! / Weld Modifier do Blender, implementado de forma independente de
//! `ops::merge`.
//!
//! Vértices a menos de `distance` uns dos outros formam um cluster (transitivo,
//! via union-find); cada cluster colapsa no vértice de **menor índice**, que
//! mantém a própria posição e atributos (nada é deslocado nem promediado, então
//! a solda é estável e determinística).

use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, Corner, EdgeAttr, EditMesh, Face};
use crate::topology::Topology;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Parâmetros do Weld para uso na pilha de modificadores.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WeldParams {
    /// Distância máxima de solda (`>= 0`; 0 solda apenas vértices coincidentes).
    pub distance: f32,
    /// Só solda vértices "soltos": os que tocam alguma aresta que não tem
    /// exatamente 2 faces (fronteira, aresta solta, não-manifold) ou que não
    /// pertencem a nenhuma face. Costura bordas sem tocar no interior da malha.
    pub only_loose: bool,
}

impl Default for WeldParams {
    fn default() -> Self {
        Self {
            distance: 1e-4,
            only_loose: false,
        }
    }
}

struct Dsu(Vec<u32>);

impl Dsu {
    fn new(n: usize) -> Self {
        Dsu((0..n as u32).collect())
    }
    fn find(&mut self, mut x: u32) -> u32 {
        while self.0[x as usize] != x {
            let p = self.0[x as usize];
            self.0[x as usize] = self.0[p as usize];
            x = self.0[x as usize];
        }
        x
    }
    /// Une mantendo o menor índice como raiz.
    fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra < rb {
            self.0[rb as usize] = ra;
        } else if rb < ra {
            self.0[ra as usize] = rb;
        }
    }
}

fn validate_distance(distance: f32) -> ModelResult<()> {
    if !distance.is_finite() || distance < 0.0 {
        return Err(ModelError::InvalidParameter(format!(
            "distância de solda deve ser finita e >= 0, recebeu {distance}"
        )));
    }
    Ok(())
}

/// Calcula, para cada vértice, o índice (da malha original) do representante do
/// seu cluster de solda (o próprio vértice se não for soldado).
///
/// Mesmos parâmetros e erros de [`weld`]; útil para a UI mostrar quantos
/// vértices serão soldados antes de aplicar.
pub fn weld_targets(mesh: &EditMesh, distance: f32, only_loose: bool) -> ModelResult<Vec<u32>> {
    validate_distance(distance)?;
    mesh.validate()?;
    let n = mesh.verts.len();
    let candidate: Vec<bool> = if only_loose {
        let topo = Topology::build(mesh);
        (0..n)
            .map(|v| topo.vert_faces[v].is_empty() || topo.vert_edges[v].iter().any(|&e| topo.edges[e as usize].faces.len() != 2))
            .collect()
    } else {
        vec![true; n]
    };

    let cell = distance.max(1e-6) as f64;
    let key = |p: glam::Vec3| -> (i64, i64, i64) {
        (
            (p.x as f64 / cell).floor() as i64,
            (p.y as f64 / cell).floor() as i64,
            (p.z as f64 / cell).floor() as i64,
        )
    };
    let d2 = (distance as f64) * (distance as f64);
    let mut grid: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
    let mut dsu = Dsu::new(n);
    for v in 0..n as u32 {
        if !candidate[v as usize] {
            continue;
        }
        let p = mesh.verts[v as usize].co;
        let (cx, cy, cz) = key(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(list) = grid.get(&(cx + dx, cy + dy, cz + dz)) {
                        for &u in list {
                            let q = mesh.verts[u as usize].co;
                            if (p.as_dvec3() - q.as_dvec3()).length_squared() <= d2 {
                                dsu.union(u, v);
                            }
                        }
                    }
                }
            }
        }
        grid.entry((cx, cy, cz)).or_default().push(v);
    }
    Ok((0..n as u32).map(|v| dsu.find(v)).collect())
}

/// Solda por distância e devolve a malha resultante (a entrada não é alterada).
///
/// * `distance`: raio de solda (`>= 0`). Clusters são transitivos (A~B e B~C
///   soldam A, B e C) e colapsam no vértice de menor índice, sem mover nada.
/// * `only_loose`: restringe a solda a vértices de fronteira/soltos/não-manifold
///   (ver [`WeldParams::only_loose`]).
/// * Faces: cantos que colapsam no mesmo vértice consecutivamente são fundidos;
///   faces com menos de 3 cantos distintos são removidas. UV e normais por corner
///   são preservados nos cantos que sobram.
/// * Atributos de aresta: arestas que se fundem combinam `sharp`/`seam` por OU e
///   `crease`/`bevel_weight` pelo máximo, então nenhuma marcação é perdida.
/// * Vértices que não participam de nenhum cluster permanecem (mesmo sem faces).
///
/// Erros: `distance` negativa ou não finita; malha inválida.
pub fn weld(mesh: &EditMesh, distance: f32, only_loose: bool) -> ModelResult<EditMesh> {
    let target = weld_targets(mesh, distance, only_loose)?;
    let n = mesh.verts.len();
    let mut new_id = vec![u32::MAX; n];
    let mut verts = Vec::with_capacity(n);
    for v in 0..n {
        if target[v] as usize == v {
            new_id[v] = verts.len() as u32;
            verts.push(mesh.verts[v].clone());
        }
    }
    let map = |v: u32| new_id[target[v as usize] as usize];

    let mut faces: Vec<Face> = Vec::with_capacity(mesh.faces.len());
    for f in &mesh.faces {
        let mut corners: Vec<Corner> = Vec::with_capacity(f.corners.len());
        for c in &f.corners {
            let nv = map(c.vert);
            if corners.last().is_some_and(|l| l.vert == nv) {
                continue;
            }
            corners.push(Corner { vert: nv, ..*c });
        }
        while corners.len() > 1 && corners[0].vert == corners[corners.len() - 1].vert {
            corners.pop();
        }
        if corners.len() < 3 {
            continue;
        }
        let mut nf = f.clone();
        nf.corners = corners;
        faces.push(nf);
    }

    let mut edge_attrs: BTreeMap<(u32, u32), EdgeAttr> = BTreeMap::new();
    for (&(a, b), attr) in &mesh.edge_attrs {
        let (na, nb) = (map(a), map(b));
        if na == nb {
            continue;
        }
        let key = edge_key(na, nb);
        let merged = match edge_attrs.get(&key) {
            Some(old) => EdgeAttr {
                sharp: old.sharp || attr.sharp,
                seam: old.seam || attr.seam,
                crease: old.crease.max(attr.crease),
                bevel_weight: old.bevel_weight.max(attr.bevel_weight),
            },
            None => *attr,
        };
        edge_attrs.insert(key, merged);
    }
    Ok(EditMesh {
        name: mesh.name.clone(),
        verts,
        faces,
        edge_attrs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::{self, assert_outward};
    use glam::Vec3;

    /// Divide a grade `nx × nz` em duas metades independentes (vértices duplicados na costura).
    fn split_grid() -> EditMesh {
        let g = primitives::grid(4, 2, 4.0, 2.0);
        let left: Vec<u32> = (0..g.face_count() as u32).filter(|&f| g.face_center(f).x < 0.0).collect();
        let right: Vec<u32> = (0..g.face_count() as u32).filter(|&f| g.face_center(f).x > 0.0).collect();
        let build = |sel: &[u32]| {
            let mut m = g.clone();
            let del: Vec<u32> = (0..g.face_count() as u32).filter(|f| !sel.contains(f)).collect();
            m.delete_faces(&del).unwrap();
            m.remove_unused_verts();
            m
        };
        let mut a = build(&left);
        let b = build(&right);
        a.append(&b);
        a
    }

    #[test]
    fn welds_split_grid_back_into_manifold_sheet() {
        let split = split_grid();
        assert_eq!(split.vert_count(), 18); // duas metades de 3x3 vértices
        let m = weld(&split, 1e-4, false).unwrap();
        m.validate().unwrap();
        assert_eq!(m.vert_count(), 15);
        assert_eq!(m.face_count(), 8);
        let t = Topology::build(&m);
        assert_eq!(t.boundary_edge_count(), 12);
        assert_eq!(t.non_manifold_edge_count(), 0);
        assert_eq!(t.euler_characteristic(&m), 1);
        // only_loose também resolve (as costuras são fronteira)
        let m2 = weld(&split, 1e-4, true).unwrap();
        assert_eq!(m2.vert_count(), 15);
        // distância insuficiente não solda
        let mut apart = split.clone();
        for v in apart.verts.iter_mut().skip(9) {
            v.co.x += 0.01;
        }
        assert_eq!(weld(&apart, 0.001, false).unwrap().vert_count(), 18);
        assert_eq!(weld(&apart, 0.02, false).unwrap().vert_count(), 15);
    }

    #[test]
    fn only_loose_leaves_interior_vertices_alone() {
        let g = primitives::grid(2, 2, 0.001, 0.001);
        let all = weld(&g, 0.01, false).unwrap();
        assert_eq!(all.vert_count(), 1);
        assert_eq!(all.face_count(), 0);
        let loose = weld(&g, 0.01, true).unwrap();
        // 8 vértices de borda viram 1; o centro permanece
        assert_eq!(loose.vert_count(), 2);
        assert_eq!(loose.face_count(), 0);
        // malha fechada: nada a soldar com only_loose
        let c = primitives::cube(0.001);
        assert_eq!(weld(&c, 1.0, true).unwrap(), c);
        assert_eq!(weld(&c, 1.0, false).unwrap().vert_count(), 1);
    }

    #[test]
    fn duplicate_cube_welds_exactly_with_zero_distance() {
        let c = primitives::cube(2.0);
        let mut two = c.clone();
        two.append(&c);
        let m = weld(&two, 0.0, false).unwrap();
        assert_eq!(m.vert_count(), 8);
        assert_eq!(m.face_count(), 12, "faces duplicadas permanecem");
        assert_eq!(weld(&two, 0.0, true).unwrap().vert_count(), 16, "cubos fechados: nada solto");
        // distância zero não une vértices diferentes
        assert_eq!(weld(&c, 0.0, false).unwrap(), c);
    }

    #[test]
    fn clusters_are_transitive_and_collapse_to_lowest_index() {
        let mut m = EditMesh::new("linha");
        for x in [0.0, 0.6, 1.2, 5.0] {
            m.add_vert(Vec3::new(x, 0.0, 0.0));
        }
        m.add_vert(Vec3::new(0.0, 3.0, 0.0));
        m.add_face(&[0, 3, 4]).unwrap();
        let t = weld_targets(&m, 0.7, false).unwrap();
        assert_eq!(t, vec![0, 0, 0, 3, 4]);
        let w = weld(&m, 0.7, false).unwrap();
        assert_eq!(w.vert_count(), 3);
        assert_eq!(w.verts[0].co, Vec3::ZERO, "representante não se move");
        assert_eq!(w.face_count(), 1);
    }

    #[test]
    fn degenerate_faces_are_dropped_and_corner_data_survives() {
        let mut g = primitives::grid(1, 1, 1.0, 1.0);
        g.faces[0].corners[2].normal = Some(Vec3::Y);
        // dois cantos adjacentes coincidem => quad vira triângulo
        g.verts[1].co = g.verts[0].co + Vec3::new(0.0, 0.0, 1e-5);
        let m = weld(&g, 1e-3, false).unwrap();
        assert_eq!(m.faces[0].len(), 3);
        assert!(m.faces[0].corners.iter().any(|c| c.normal == Some(Vec3::Y)));
        assert!(m.faces[0].corners.iter().all(|c| c.uv.is_finite()));
        // triângulo que colapsa inteiro desaparece
        let mut t = EditMesh::new("t");
        for p in [Vec3::ZERO, Vec3::X * 1e-5, Vec3::Y * 1e-5] {
            t.add_vert(p);
        }
        t.add_face(&[0, 1, 2]).unwrap();
        let w = weld(&t, 1e-3, false).unwrap();
        assert_eq!(w.face_count(), 0);
        w.validate().unwrap();
    }

    #[test]
    fn edge_attributes_are_combined_not_lost() {
        let split = split_grid();
        let mut m = split.clone();
        // costura marcada de um lado como seam e do outro como sharp+crease
        let seam: Vec<(u32, u32)> = {
            let t = Topology::build(&m);
            t.edges
                .iter()
                .filter(|e| m.co(e.verts.0).x.abs() < 1e-6 && m.co(e.verts.1).x.abs() < 1e-6)
                .map(|e| e.verts)
                .collect()
        };
        assert_eq!(seam.len(), 4);
        for (i, &(a, b)) in seam.iter().enumerate() {
            if i % 2 == 0 {
                m.set_seam(a, b, true);
            } else {
                m.set_sharp(a, b, true);
                m.set_crease(a, b, 0.4);
            }
        }
        let w = weld(&m, 1e-4, false).unwrap();
        let on_seam: Vec<_> = w
            .edge_attrs
            .iter()
            .filter(|((a, b), _)| w.co(*a).x.abs() < 1e-6 && w.co(*b).x.abs() < 1e-6)
            .collect();
        assert_eq!(on_seam.len(), 2, "arestas duplicadas se fundem: {on_seam:?}");
        assert!(on_seam.iter().all(|(_, a)| a.seam || a.sharp));
        // sem laços: nenhuma aresta degenerada
        assert!(w.edge_attrs.keys().all(|(a, b)| a != b));
        // uma aresta marcada some quando as pontas colapsam
        let mut tiny = primitives::cube(0.001);
        tiny.set_sharp(0, 1, true);
        let r = weld(&tiny, 1.0, false).unwrap();
        assert!(r.edge_attrs.is_empty());
        r.validate().unwrap();
    }

    #[test]
    fn invalid_input_is_rejected_and_unchanged() {
        let c = primitives::cube(1.0);
        let before = c.clone();
        for bad in [-1.0, f32::NAN, f32::INFINITY] {
            assert!(matches!(weld(&c, bad, false), Err(ModelError::InvalidParameter(_))));
        }
        let mut broken = c.clone();
        broken.faces[0].corners[0].vert = 99;
        assert!(weld(&broken, 0.1, false).is_err());
        assert_eq!(c, before);
    }

    #[test]
    fn wire_edges_and_isolated_vertices_are_handled() {
        let mut m = EditMesh::new("w");
        for p in [Vec3::ZERO, Vec3::X, Vec3::X * 1.00001, Vec3::new(3.0, 0.0, 0.0), Vec3::splat(9.0)] {
            m.add_vert(p);
        }
        m.set_sharp(0, 1, true);
        m.set_seam(2, 3, true);
        let w = weld(&m, 1e-3, true).unwrap();
        assert_eq!(w.vert_count(), 4);
        // as duas arestas soltas agora compartilham o vértice soldado
        assert_eq!(w.edge_attrs.len(), 2);
        let keys: Vec<_> = w.edge_attrs.keys().collect();
        assert!(keys.iter().any(|k| k.1 == 1) && keys.iter().any(|k| k.0 == 1));
    }

    #[test]
    fn welded_sphere_halves_form_closed_sphere() {
        let s = primitives::quad_sphere(1.0, 6);
        let mut a = s.clone();
        let mut b = s.clone();
        a.transform_positions(|p| p);
        b.transform_positions(|p| p);
        // quebra a esfera em duas malhas: remove metade das faces de cada
        let n = s.face_count() as u32;
        let keep_a: Vec<u32> = (0..n).filter(|&f| s.face_center(f).y >= 0.0).collect();
        let del_a: Vec<u32> = (0..n).filter(|f| !keep_a.contains(f)).collect();
        a.delete_faces(&del_a).unwrap();
        let del_b: Vec<u32> = (0..n).filter(|f| keep_a.contains(f)).collect();
        b.delete_faces(&del_b).unwrap();
        a.remove_unused_verts();
        b.remove_unused_verts();
        a.append(&b);
        let w = weld(&a, 1e-4, true).unwrap();
        let t = Topology::build(&w);
        assert!(t.is_closed_manifold());
        assert_eq!(t.euler_characteristic(&w), 2);
        assert_outward(&w);
    }

    #[test]
    fn large_mesh_is_fast_and_deterministic() {
        let mut g = primitives::grid(150, 150, 10.0, 10.0);
        let copy = g.clone();
        g.append(&copy);
        let t = std::time::Instant::now();
        let a = weld(&g, 1e-4, false).unwrap();
        let b = weld(&g, 1e-4, false).unwrap();
        assert_eq!(a.content_hash(), b.content_hash());
        assert_eq!(a.vert_count(), 151 * 151);
        assert!(t.elapsed().as_secs_f32() < 20.0);
    }

    #[test]
    fn serde_roundtrip() {
        let p = WeldParams {
            distance: 0.5,
            only_loose: true,
        };
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<WeldParams>(&json).unwrap(), p);
    }
}
