//! Cache das métricas de UV usadas pelo editor (estiramento, densidade, ilhas, invertidas, sobrepostas,
//! relatório). Recalculado só quando geometria ou UVs mudam (chave por hash de conteúdo).

use anigo_modeling::uv::metrics::{self, UvReport};
use anigo_modeling::uv::seams::split_islands;
use anigo_modeling::{EditMesh, Topology};

/// Chave de validade: objeto, hash da geometria/costuras, hash dos UVs e resolução da textura.
pub type CacheKey = (u32, u64, u64, u32);

pub fn cache_key(obj_id: u32, mesh: &EditMesh, tex_size: u32) -> CacheKey {
    (obj_id, mesh.content_hash(), metrics::uv_hash(mesh), tex_size)
}

#[derive(Debug, Clone)]
pub struct UvCache {
    pub key: CacheKey,
    pub report: Result<UvReport, String>,
    /// `face_stretch` por face (NaN = índice inválido, inf = UV colapsado).
    pub stretch: Vec<f32>,
    /// Densidade de texels (px por unidade 3D) por face; 0 se a face 3D é degenerada.
    pub density: Vec<f32>,
    pub mean_density: f32,
    /// Índice da ilha de cada face.
    pub island: Vec<u32>,
    pub flipped: Vec<bool>,
    /// Sobreposta a outra ilha ou a si mesma.
    pub overlap: Vec<bool>,
    /// Arestas de fronteira de ilha como `(face, índice do canto)` (segmento canto i -> i+1).
    pub borders: Vec<(u32, u16)>,
}

impl UvCache {
    pub fn build(key: CacheKey, mesh: &EditMesh) -> Self {
        let n = mesh.faces.len();
        let tex = key.3.max(1);
        let report = metrics::uv_report(mesh, tex).map_err(|e| e.to_string());
        let stretch: Vec<f32> = (0..n as u32).map(|f| metrics::face_stretch(mesh, f)).collect();
        let density: Vec<f32> = (0..n as u32).map(|f| metrics::face_texel_density(mesh, f, tex).unwrap_or(0.0)).collect();
        let mean_density = report
            .as_ref()
            .ok()
            .and_then(|r| r.texel_density.map(|d| d.mean))
            .unwrap_or_else(|| {
                let v: Vec<f32> = density.iter().copied().filter(|d| *d > 0.0).collect();
                if v.is_empty() {
                    1.0
                } else {
                    v.iter().sum::<f32>() / v.len() as f32
                }
            });
        let islands = split_islands(mesh);
        let mut island = vec![0u32; n];
        for (i, isl) in islands.iter().enumerate() {
            for &f in &isl.faces {
                if let Some(slot) = island.get_mut(f as usize) {
                    *slot = i as u32;
                }
            }
        }
        let mut flipped = vec![false; n];
        for f in metrics::flipped_faces(mesh) {
            flipped[f as usize] = true;
        }
        let mut overlap = vec![false; n];
        for f in metrics::overlapping_faces(mesh).into_iter().chain(metrics::self_overlapping_faces(mesh)) {
            overlap[f as usize] = true;
        }
        let topo = Topology::build(mesh);
        let mut borders = Vec::new();
        for (fi, face) in mesh.faces.iter().enumerate() {
            let k = face.corners.len();
            for i in 0..k {
                let (a, b) = (face.corners[i].vert, face.corners[(i + 1) % k].vert);
                let others = topo.edge_faces(a, b);
                let is_border = others.len() < 2 || others.iter().any(|&o| o as usize != fi && island[o as usize] != island[fi]);
                if is_border {
                    borders.push((fi as u32, i as u16));
                }
            }
        }
        Self { key, report, stretch, density, mean_density, island, flipped, overlap, borders }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_modeling::primitives;

    #[test]
    fn cache_matches_engine_metrics_and_invalidates_on_uv_change() {
        let mut m = primitives::quad_sphere(1.0, 3);
        let key = cache_key(1, &m, 1024);
        let c = UvCache::build(key, &m);
        assert_eq!(c.stretch.len(), m.faces.len());
        assert_eq!(c.island.len(), m.faces.len());
        let rep = c.report.as_ref().expect("relatório");
        assert_eq!(rep.face_count, m.faces.len());
        // alterar um UV muda a chave mesmo sem mudar a geometria
        m.faces[0].corners[0].uv.x += 0.3;
        assert_ne!(cache_key(1, &m, 1024), key);
        assert_ne!(cache_key(1, &m, 2048), key);
        assert_ne!(cache_key(2, &m, 1024), key);
    }

    #[test]
    fn borders_follow_island_boundaries() {
        // plano 2x1 com UVs contínuos: uma ilha, fronteira = 6 arestas externas
        let mut m = primitives::grid(2, 1, 2.0, 1.0);
        for f in &mut m.faces {
            for c in &mut f.corners {
                let p = m.verts[c.vert as usize].co;
                c.uv = glam::Vec2::new(p.x * 0.5 + 0.5, p.z + 0.5);
            }
        }
        let c = UvCache::build(cache_key(1, &m, 1024), &m);
        assert_eq!(c.borders.len(), 6);
        assert!(c.island.iter().all(|&i| i == 0));
        // separa a face 1 em UV: a aresta interna passa a ser fronteira nas duas faces
        for corner in &mut m.faces[1].corners {
            corner.uv += glam::Vec2::new(3.0, 0.0);
        }
        let c2 = UvCache::build(cache_key(1, &m, 1024), &m);
        assert_eq!(c2.borders.len(), 8);
        assert_ne!(c2.island[0], c2.island[1]);
    }
}
