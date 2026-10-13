//! Espelho CPU do sólido de uma malha em escultura: mantém o `Vec<SolidVertex>` que está na GPU e
//! atualiza só os cantos dos vértices que se moveram (posição, normal suave e sombreado da máscara).
//! Evita reconstruir a malha inteira (corner normals + topologia + arestas) a cada quadro do traço.

use super::mesh_data::SolidVertex;
use anigo_modeling::{convert::triangulate_polygon, EditMesh};
use glam::Vec3;

/// Cor de um vértice mascarado (máscara 1 = protegido), como o cinza escuro do Blender.
const MASK_COLOR: [f32; 4] = [0.07, 0.07, 0.09, 1.0];
/// Quanto a máscara 1.0 escurece a cor base (0 = nada, 1 = cor da máscara).
const MASK_STRENGTH: f32 = 0.88;

/// Cor sombreada pela máscara.
pub fn shade_by_mask(base: [f32; 4], mask: f32) -> [f32; 4] {
    let t = (mask.clamp(0.0, 1.0)) * MASK_STRENGTH;
    [
        base[0] + (MASK_COLOR[0] - base[0]) * t,
        base[1] + (MASK_COLOR[1] - base[1]) * t,
        base[2] + (MASK_COLOR[2] - base[2]) * t,
        base[3],
    ]
}

#[derive(Debug, Clone)]
pub struct SculptMirror {
    /// Vértices do sólido (não indexado, 3 por triângulo) — idêntico ao buffer da GPU.
    pub verts: Vec<SolidVertex>,
    /// Índice do vértice da malha de cada vértice do sólido.
    corner_vert: Vec<u32>,
    /// CSR: vértice da malha → índices de vértices do sólido.
    v2c_start: Vec<u32>,
    v2c: Vec<u32>,
    base_color: [f32; 4],
    /// Intervalo `[lo, hi)` de vértices do sólido alterados desde o último `take_dirty`.
    dirty: Option<(usize, usize)>,
}

impl SculptMirror {
    /// Constrói o espelho. `normals` são as normais suaves por vértice da malha (do `SculptContext`).
    pub fn build(mesh: &EditMesh, normals: &[Vec3], base_color: [f32; 4]) -> Self {
        let mut verts: Vec<SolidVertex> = Vec::with_capacity(mesh.faces.len() * 6);
        let mut corner_vert: Vec<u32> = Vec::with_capacity(mesh.faces.len() * 6);
        for (fi, face) in mesh.faces.iter().enumerate() {
            if face.hidden {
                continue;
            }
            let pts = mesh.face_positions(fi as u32);
            for [a, b, c] in triangulate_polygon(&pts) {
                for i in [a, b, c] {
                    let v = face.corners[i].vert;
                    let vert = &mesh.verts[v as usize];
                    verts.push(SolidVertex {
                        pos: vert.co.to_array(),
                        normal: normals.get(v as usize).copied().unwrap_or(Vec3::Y).to_array(),
                        color: shade_by_mask(base_color, vert.mask),
                    });
                    corner_vert.push(v);
                }
            }
        }
        // CSR vértice -> cantos
        let n = mesh.verts.len();
        let mut count = vec![0u32; n + 1];
        for &v in &corner_vert {
            count[v as usize + 1] += 1;
        }
        for i in 0..n {
            count[i + 1] += count[i];
        }
        let mut fill = count.clone();
        let mut v2c = vec![0u32; corner_vert.len()];
        for (ci, &v) in corner_vert.iter().enumerate() {
            let slot = &mut fill[v as usize];
            v2c[*slot as usize] = ci as u32;
            *slot += 1;
        }
        let dirty = (!verts.is_empty()).then_some((0, verts.len()));
        Self {
            verts,
            corner_vert,
            v2c_start: count,
            v2c,
            base_color,
            dirty,
        }
    }

    pub fn len(&self) -> usize {
        self.verts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.verts.is_empty()
    }

    /// Vértice da malha que originou o vértice `i` do sólido.
    pub fn mesh_vert(&self, i: usize) -> Option<u32> {
        self.corner_vert.get(i).copied()
    }

    /// Atualiza posição/normal/cor dos cantos dos vértices `moved` (índices da malha).
    pub fn update(&mut self, mesh: &EditMesh, normals: &[Vec3], moved: &[u32]) {
        let (mut lo, mut hi) = self.dirty.unwrap_or((usize::MAX, 0));
        for &v in moved {
            let vi = v as usize;
            if vi + 1 >= self.v2c_start.len() || vi >= mesh.verts.len() {
                continue;
            }
            let vert = &mesh.verts[vi];
            let pos = vert.co.to_array();
            let normal = normals.get(vi).copied().unwrap_or(Vec3::Y).to_array();
            let color = shade_by_mask(self.base_color, vert.mask);
            let (a, b) = (self.v2c_start[vi] as usize, self.v2c_start[vi + 1] as usize);
            for &ci in &self.v2c[a..b] {
                let ci = ci as usize;
                self.verts[ci] = SolidVertex { pos, normal, color };
                lo = lo.min(ci);
                hi = hi.max(ci + 1);
            }
        }
        self.dirty = (hi > lo).then_some((lo, hi));
    }

    pub fn mark_all_dirty(&mut self) {
        self.dirty = (!self.verts.is_empty()).then_some((0, self.verts.len()));
    }

    /// Intervalo sujo desde a última chamada (e o limpa).
    pub fn take_dirty(&mut self) -> Option<(usize, usize)> {
        self.dirty.take()
    }

    pub fn has_dirty(&self) -> bool {
        self.dirty.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_modeling::primitives;

    fn normals_of(mesh: &EditMesh) -> Vec<Vec3> {
        mesh.verts.iter().map(|v| v.co.normalize_or_zero()).collect()
    }

    #[test]
    fn build_matches_triangle_count_and_mask_shading() {
        let mut m = primitives::quad_sphere(1.0, 4);
        m.verts[0].mask = 1.0;
        let n = normals_of(&m);
        let mir = SculptMirror::build(&m, &n, [0.7, 0.7, 0.7, 1.0]);
        assert_eq!(mir.len(), m.triangle_count() * 3);
        let masked: Vec<_> = (0..mir.len()).filter(|&i| mir.mesh_vert(i) == Some(0)).collect();
        assert!(!masked.is_empty());
        for i in masked {
            assert!(mir.verts[i].color[0] < 0.2, "vértice mascarado deve escurecer");
        }
        let free = (0..mir.len()).find(|&i| mir.mesh_vert(i) == Some(5)).unwrap();
        assert!((mir.verts[free].color[0] - 0.7).abs() < 1e-6);
    }

    #[test]
    fn update_moves_only_requested_vertices_and_reports_dirty_range() {
        let mut m = primitives::quad_sphere(1.0, 4);
        let n = normals_of(&m);
        let mut mir = SculptMirror::build(&m, &n, [0.7, 0.7, 0.7, 1.0]);
        assert_eq!(mir.take_dirty(), Some((0, mir.len())));
        assert_eq!(mir.take_dirty(), None);
        let before = mir.verts.clone();
        m.verts[3].co += Vec3::new(0.0, 0.0, 0.5);
        mir.update(&m, &n, &[3]);
        let (lo, hi) = mir.take_dirty().expect("sujo");
        assert!(lo < hi && hi <= mir.len());
        for i in 0..mir.len() {
            if mir.mesh_vert(i) == Some(3) {
                assert_eq!(mir.verts[i].pos, m.verts[3].co.to_array());
            } else {
                assert_eq!(mir.verts[i], before[i]);
            }
        }
    }

    #[test]
    fn hidden_faces_are_skipped() {
        let mut m = primitives::cube(2.0);
        m.faces[0].hidden = true;
        let n = normals_of(&m);
        let mir = SculptMirror::build(&m, &n, [0.7; 4]);
        assert_eq!(mir.len(), 10 * 3);
    }

    #[test]
    fn shade_by_mask_interpolates() {
        let base = [0.8, 0.8, 0.8, 1.0];
        assert_eq!(shade_by_mask(base, 0.0), base);
        let full = shade_by_mask(base, 1.0);
        assert!(full[0] < 0.2);
        let half = shade_by_mask(base, 0.5);
        assert!(half[0] < base[0] && half[0] > full[0]);
    }
}
