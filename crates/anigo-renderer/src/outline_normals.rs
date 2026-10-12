//! Normais de extrusão do contorno (inverted hull) — o SEGUNDO stream de
//! vértices do pipeline `outline` (ver `vertex_layout.outline_stream` no
//! contrato). Calculadas no lado Rust: soldagem por posição + média ponderada
//! por ângulo (`anigo_core::npr::smooth_outline_normals`), com cache por
//! geometria (a suavização é O(V) com hash; a cena quase nunca muda de forma).

use std::collections::HashMap;
use std::sync::Arc;

use anigo_core::mesh::Mesh;
use anigo_core::npr;

/// Quantas geometrias suavizadas o cache guarda antes de ser esvaziado.
const CACHE_CAPACITY: usize = 32;

/// Cache de normais suavizadas por geometria (posições + índices).
#[derive(Default)]
pub struct OutlineNormalCache {
    entries: HashMap<u64, Arc<Vec<[f32; 3]>>>,
}

/// FNV-1a 64 de posições e índices (a suavização só depende deles e das
/// normais originais; estas entram no hash porque servem de salvaguarda).
fn geometry_key(mesh: &Mesh) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for vertex in &mesh.vertices {
        feed(bytemuck::bytes_of(&vertex.position));
        feed(bytemuck::bytes_of(&vertex.normal));
    }
    feed(bytemuck::cast_slice(&mesh.indices));
    hash
}

impl OutlineNormalCache {
    /// Normais SUAVIZADAS (blend = 1) da malha, do cache quando já calculadas.
    pub fn smooth(&mut self, mesh: &Mesh) -> Arc<Vec<[f32; 3]>> {
        let key = geometry_key(mesh);
        if let Some(found) = self.entries.get(&key) {
            return Arc::clone(found);
        }
        let positions: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.position).collect();
        let normals: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.normal).collect();
        let smooth = Arc::new(npr::smooth_outline_normals(&positions, &normals, &mesh.indices));
        if self.entries.len() >= CACHE_CAPACITY {
            self.entries.clear();
        }
        self.entries.insert(key, Arc::clone(&smooth));
        smooth
    }

    /// Normais de extrusão finais de um nó: `blend = 0` → as da própria malha
    /// (byte a byte o stream que o shader lia antes); `> 0` → mistura com as
    /// suavizadas.
    pub fn outline_normals(&mut self, mesh: &Mesh, blend: f32) -> Vec<[f32; 3]> {
        if blend <= 0.0 {
            return mesh.vertices.iter().map(|v| v.normal).collect();
        }
        let smooth = self.smooth(mesh);
        mesh.vertices
            .iter()
            .zip(smooth.iter())
            .map(|(v, s)| npr::blend_outline_normal(v.normal, *s, blend))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_zero_returns_mesh_normals_and_does_not_touch_the_cache() {
        let mesh = Mesh::create_cube(1.0);
        let mut cache = OutlineNormalCache::default();
        let normals = cache.outline_normals(&mesh, 0.0);
        assert_eq!(normals.len(), mesh.vertices.len());
        for (n, v) in normals.iter().zip(&mesh.vertices) {
            assert_eq!(*n, v.normal);
        }
        assert!(cache.is_empty());
    }

    #[test]
    fn smoothed_cube_normals_point_along_the_diagonals_and_are_cached() {
        let mesh = Mesh::create_cube(1.0);
        let mut cache = OutlineNormalCache::default();
        let normals = cache.outline_normals(&mesh, 1.0);
        let inv = 1.0 / 3.0f32.sqrt();
        for (n, v) in normals.iter().zip(&mesh.vertices) {
            for a in 0..3 {
                let expected = v.position[a].signum() * inv;
                assert!((n[a] - expected).abs() < 1e-5, "{n:?} vs {:?}", v.position);
            }
        }
        assert_eq!(cache.len(), 1);
        let _ = cache.outline_normals(&mesh, 0.5);
        assert_eq!(cache.len(), 1, "mesma geometria reaproveita o cache");
    }
}
