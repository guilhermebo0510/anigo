//! Importa a pele esculpida de um GLB (o primeiro primitivo) para a ordem canônica, quando possível.
//!
//! Um GLB do próprio `anigo-charbase` traz os vértices canônicos primeiro e as duplicatas de
//! costura de UV depois; soldar por posição devolve a ordem do contrato. Um GLB reexportado por
//! outra ferramenta pode ter reordenado tudo: nesse caso `validate_topology_tris` acusa a diferença
//! e `rederive` usa vizinho mais próximo.

use anigo_core::Mesh;
use glam::Vec3;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct ImportedSkin {
    pub positions: Vec<Vec3>,
    pub tris: Vec<[u32; 3]>,
    /// Vértices do arquivo antes da soldagem por posição.
    pub file_vertices: usize,
}

/// Lê o primeiro primitivo do GLB e solda vértices coincidentes (costuras de UV/normais).
pub fn import_glb_skin(bytes: &[u8]) -> Result<ImportedSkin, String> {
    let mesh = Mesh::from_glb_bytes(bytes).map_err(|e| e.to_string())?;
    if mesh.vertices.is_empty() || mesh.indices.len() < 3 || mesh.indices.len() % 3 != 0 {
        return Err("o GLB não tem triângulos".into());
    }
    let q = |p: [f32; 3]| -> (i64, i64, i64) { ((p[0] * 1e6).round() as i64, (p[1] * 1e6).round() as i64, (p[2] * 1e6).round() as i64) };
    let mut first: BTreeMap<(i64, i64, i64), u32> = BTreeMap::new();
    let mut remap: Vec<u32> = Vec::with_capacity(mesh.vertices.len());
    let mut positions: Vec<Vec3> = Vec::new();
    for v in &mesh.vertices {
        if !v.position.iter().all(|x| x.is_finite()) {
            return Err("o GLB tem posições não finitas".into());
        }
        let id = *first.entry(q(v.position)).or_insert_with(|| {
            positions.push(Vec3::from_array(v.position));
            (positions.len() - 1) as u32
        });
        remap.push(id);
    }
    let mut tris = Vec::with_capacity(mesh.indices.len() / 3);
    for t in mesh.indices.chunks(3) {
        let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
        if a >= remap.len() || b >= remap.len() || c >= remap.len() {
            return Err("índice de triângulo fora da faixa".into());
        }
        tris.push([remap[a], remap[b], remap[c]]);
    }
    Ok(ImportedSkin {
        positions,
        tris,
        file_vertices: mesh.vertices.len(),
    })
}
