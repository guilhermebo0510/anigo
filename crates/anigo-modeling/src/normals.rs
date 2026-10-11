//! Normais por corner (split normals), o coração do visual cel-shaded.
//!
//! Em NPR anime, a forma da sombra no rosto e no cabelo é controlada pela
//! normal, não pela geometria (Guilty Gear Xrd editava normais "em toda
//! feature"). Por isso o ANIGO trata normais como dado de primeira classe:
//!
//! * `Face::smooth` + `EdgeAttr::sharp` + limite de ângulo definem os grupos suaves;
//! * `Corner::normal` (customizada) sempre vence o cálculo automático;
//! * ferramentas de edição/transferência de normais vivem neste módulo.

use crate::mesh::{edge_key, EditMesh};
use crate::topology::Topology;
use glam::Vec3;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NormalWeighting {
    /// Ângulo interno no vértice (padrão do Blender "Face Angle").
    Angle,
    /// Área da face.
    Area,
    /// Ângulo × área.
    AngleArea,
    /// Sem peso (média simples das normais de face).
    Uniform,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalParams {
    pub weighting: NormalWeighting,
    /// Arestas com ângulo diedral maior que isto viram duras (radianos).
    /// `None` = só respeita `sharp` e `Face::smooth`.
    pub auto_smooth_angle: Option<f32>,
    /// Respeitar `Corner::normal` customizadas.
    pub use_custom: bool,
}

impl Default for NormalParams {
    fn default() -> Self {
        Self {
            weighting: NormalWeighting::Angle,
            auto_smooth_angle: Some(std::f32::consts::PI * 30.0 / 180.0),
            use_custom: true,
        }
    }
}

/// Normais por corner: `result[face][corner]`.
pub type CornerNormals = Vec<Vec<Vec3>>;

fn corner_angle(pts: &[Vec3], i: usize) -> f32 {
    let n = pts.len();
    let prev = (pts[(i + n - 1) % n] - pts[i]).normalize_or_zero();
    let next = (pts[(i + 1) % n] - pts[i]).normalize_or_zero();
    prev.dot(next).clamp(-1.0, 1.0).acos()
}

struct Dsu(Vec<usize>);
impl Dsu {
    fn new(n: usize) -> Self {
        Dsu((0..n).collect())
    }
    fn find(&mut self, mut x: usize) -> usize {
        while self.0[x] != x {
            self.0[x] = self.0[self.0[x]];
            x = self.0[x];
        }
        x
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.0[ra] = rb;
        }
    }
}

/// Calcula normais por corner respeitando faces chapadas, arestas duras,
/// limite de ângulo e normais customizadas.
pub fn compute_corner_normals(mesh: &EditMesh, params: &NormalParams) -> CornerNormals {
    let topo = Topology::build(mesh);
    let face_normals: Vec<Vec3> = (0..mesh.face_count() as u32).map(|f| mesh.face_normal(f)).collect();
    let face_areas: Vec<f32> = (0..mesh.face_count() as u32).map(|f| mesh.face_area(f)).collect();
    let positions: Vec<Vec<Vec3>> = (0..mesh.face_count() as u32).map(|f| mesh.face_positions(f)).collect();

    // Posição de cada (face, corner) dentro do vértice, para a DSU local.
    let mut out: CornerNormals = mesh
        .faces
        .iter()
        .enumerate()
        .map(|(fi, f)| vec![face_normals[fi]; f.corners.len()])
        .collect();

    let cos_limit = params.auto_smooth_angle.map(|a| a.cos());

    for v in 0..mesh.vert_count() {
        let incident = &topo.vert_faces[v];
        if incident.is_empty() {
            continue;
        }
        // (face, corner_index) para cada uso do vértice
        let mut uses: Vec<(u32, usize)> = Vec::new();
        for &f in incident {
            for (ci, c) in mesh.faces[f as usize].corners.iter().enumerate() {
                if c.vert as usize == v {
                    uses.push((f, ci));
                }
            }
        }
        let mut dsu = Dsu::new(uses.len());
        // liga usos cujas faces compartilham uma aresta que contém v e é "suave"
        let mut by_edge: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        for (ui, &(f, ci)) in uses.iter().enumerate() {
            let face = &mesh.faces[f as usize];
            let n = face.corners.len();
            let prev = face.corners[(ci + n - 1) % n].vert;
            let next = face.corners[(ci + 1) % n].vert;
            by_edge.entry(edge_key(v as u32, prev)).or_default().push(ui);
            by_edge.entry(edge_key(v as u32, next)).or_default().push(ui);
        }
        for ((a, b), users) in &by_edge {
            if users.len() != 2 {
                continue; // fronteira ou não-manifold: não suaviza através
            }
            let (u0, u1) = (users[0], users[1]);
            let (f0, f1) = (uses[u0].0, uses[u1].0);
            if f0 == f1 {
                continue;
            }
            if !mesh.faces[f0 as usize].smooth || !mesh.faces[f1 as usize].smooth {
                continue;
            }
            if mesh.edge_attr(*a, *b).sharp {
                continue;
            }
            if let Some(cl) = cos_limit {
                if face_normals[f0 as usize].dot(face_normals[f1 as usize]) < cl {
                    continue;
                }
            }
            dsu.union(u0, u1);
        }
        // acumula por grupo
        let mut acc: HashMap<usize, Vec3> = HashMap::new();
        for (ui, &(f, ci)) in uses.iter().enumerate() {
            let w = match params.weighting {
                NormalWeighting::Angle => corner_angle(&positions[f as usize], ci),
                NormalWeighting::Area => face_areas[f as usize],
                NormalWeighting::AngleArea => corner_angle(&positions[f as usize], ci) * face_areas[f as usize],
                NormalWeighting::Uniform => 1.0,
            };
            let root = dsu.find(ui);
            *acc.entry(root).or_insert(Vec3::ZERO) += face_normals[f as usize] * w;
        }
        for (ui, &(f, ci)) in uses.iter().enumerate() {
            if !mesh.faces[f as usize].smooth {
                continue;
            }
            let root = dsu.find(ui);
            if let Some(n) = acc.get(&root).and_then(|n| n.try_normalize()) {
                out[f as usize][ci] = n;
            }
        }
    }

    if params.use_custom {
        for (fi, f) in mesh.faces.iter().enumerate() {
            for (ci, c) in f.corners.iter().enumerate() {
                if let Some(n) = c.normal {
                    if let Some(n) = n.try_normalize() {
                        out[fi][ci] = n;
                    }
                }
            }
        }
    }
    out
}

/// Normais por vértice (média das normais de corner) — para sculpt/preview.
pub fn compute_vertex_normals(mesh: &EditMesh) -> Vec<Vec3> {
    let cn = compute_corner_normals(
        mesh,
        &NormalParams {
            auto_smooth_angle: None,
            use_custom: false,
            ..Default::default()
        },
    );
    let mut acc = vec![Vec3::ZERO; mesh.vert_count()];
    for (fi, f) in mesh.faces.iter().enumerate() {
        for (ci, c) in f.corners.iter().enumerate() {
            acc[c.vert as usize] += cn[fi][ci];
        }
    }
    acc.into_iter().map(|n| n.try_normalize().unwrap_or(Vec3::Y)).collect()
}

/// Congela as normais calculadas como customizadas (equivale a "Apply" do
/// auto-smooth — permite editar/exportar exatamente o que se vê).
pub fn bake_custom_normals(mesh: &mut EditMesh, params: &NormalParams) {
    let cn = compute_corner_normals(mesh, params);
    for (fi, f) in mesh.faces.iter_mut().enumerate() {
        for (ci, c) in f.corners.iter_mut().enumerate() {
            c.normal = Some(cn[fi][ci]);
        }
    }
}

/// Remove normais customizadas (volta ao cálculo automático).
pub fn clear_custom_normals(mesh: &mut EditMesh) {
    for f in &mut mesh.faces {
        for c in &mut f.corners {
            c.normal = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    #[test]
    fn flat_cube_keeps_face_normals() {
        let m = primitives::cube(2.0);
        let cn = compute_corner_normals(&m, &NormalParams::default());
        for (fi, f) in cn.iter().enumerate() {
            let fnorm = m.face_normal(fi as u32);
            assert!(f.iter().all(|n| (*n - fnorm).length() < 1e-5));
        }
    }

    #[test]
    fn smooth_cube_without_angle_limit_points_outward_diagonally() {
        let mut m = primitives::cube(2.0);
        for f in &mut m.faces {
            f.smooth = true;
        }
        let params = NormalParams {
            auto_smooth_angle: None,
            ..Default::default()
        };
        let cn = compute_corner_normals(&m, &params);
        let expect = Vec3::splat(1.0).normalize();
        for (fi, f) in m.faces.iter().enumerate() {
            for (ci, c) in f.corners.iter().enumerate() {
                let want = (m.co(c.vert)).normalize();
                assert!((cn[fi][ci] - want).length() < 1e-4, "{:?} vs {:?}", cn[fi][ci], expect);
            }
        }
    }

    #[test]
    fn auto_smooth_angle_keeps_cube_hard_but_smooths_sphere() {
        let mut cube = primitives::cube(2.0);
        for f in &mut cube.faces {
            f.smooth = true;
        }
        let cn = compute_corner_normals(&cube, &NormalParams::default()); // 30°
        for (fi, _) in cube.faces.iter().enumerate() {
            let fnorm = cube.face_normal(fi as u32);
            assert!(cn[fi].iter().all(|n| (*n - fnorm).length() < 1e-4));
        }
        let sphere = primitives::quad_sphere(1.0, 6);
        let cn = compute_corner_normals(&sphere, &NormalParams::default());
        for (fi, f) in sphere.faces.iter().enumerate() {
            for (ci, c) in f.corners.iter().enumerate() {
                assert!(cn[fi][ci].dot(sphere.co(c.vert).normalize()) > 0.98);
            }
        }
    }

    #[test]
    fn sharp_edge_splits_normals() {
        let mut m = primitives::grid(2, 1, 2.0, 1.0);
        // dobra o plano ao longo da aresta central para formar um "V" suave
        let mid: Vec<u32> = (0..m.vert_count() as u32).filter(|&v| m.co(v).x.abs() < 1e-6).collect();
        for &v in &mid {
            m.verts[v as usize].co.y = 0.0;
        }
        for v in 0..m.vert_count() {
            if m.verts[v].co.x.abs() > 0.5 {
                m.verts[v].co.y = 0.3;
            }
        }
        let params = NormalParams {
            auto_smooth_angle: None,
            ..Default::default()
        };
        let smooth = compute_corner_normals(&m, &params);
        m.set_sharp(mid[0], mid[1], true);
        let sharp = compute_corner_normals(&m, &params);
        // o vértice da quina deve ter normais diferentes entre as duas faces quando duro
        let differs = |cn: &CornerNormals| {
            let v = mid[0];
            let ns: Vec<Vec3> = m
                .faces
                .iter()
                .enumerate()
                .flat_map(|(fi, f)| {
                    f.corners
                        .iter()
                        .enumerate()
                        .filter(move |(_, c)| c.vert == v)
                        .map(move |(ci, _)| cn[fi][ci])
                })
                .collect();
            (ns[0] - ns[1]).length() > 1e-3
        };
        assert!(!differs(&smooth));
        assert!(differs(&sharp));
    }

    #[test]
    fn custom_normals_override_and_bake_roundtrip() {
        let mut m = primitives::cube(2.0);
        m.faces[0].corners[0].normal = Some(Vec3::Y * 3.0); // não normalizada de propósito
        let cn = compute_corner_normals(&m, &NormalParams::default());
        assert!((cn[0][0] - Vec3::Y).length() < 1e-6);
        bake_custom_normals(&mut m, &NormalParams::default());
        assert!(m.faces.iter().all(|f| f.corners.iter().all(|c| c.normal.is_some())));
        clear_custom_normals(&mut m);
        assert!(m.faces.iter().all(|f| f.corners.iter().all(|c| c.normal.is_none())));
    }

    #[test]
    fn vertex_normals_are_unit_length() {
        let m = primitives::uv_sphere(1.0, 8, 12);
        for n in compute_vertex_normals(&m) {
            assert!((n.length() - 1.0).abs() < 1e-4);
        }
    }
}
