//! ANIGO — validação de malha antes de criar buffers (P1 — Robustez, item 3).
//!
//! O headless cria VBO/IBO a cada quadro; se a malha vier corrompida (índice fora
//! do intervalo, NaN, topologia quebrada), o wgpu só reclamaria depois — com
//! erro não capturado ou artefato visual. Aqui a reprovação acontece **antes**,
//! com o mesmo vocabulário do lado TypeScript (`mesh_validation.codes` no render
//! contract).
//!
//! O invariante de layout também é checado: `size_of::<Vertex>()` precisa ser o
//! stride declarado no contrato (72 B), porque o headless faz `cast_slice` do
//! vetor de vértices direto para o buffer.

use anigo_core::mesh::{Mesh, Vertex};

use crate::render_contract;

/// Motivo da reprovação (`code` é um dos códigos do contrato).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshProblem {
    pub code: &'static str,
    pub detail: String,
}

impl MeshProblem {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    /// Mensagem pronta para diagnóstico/telemetria.
    pub fn message(&self) -> String {
        format!("[MESH {}] {}", self.code, self.detail)
    }
}

/// Códigos de validação declarados no contrato.
pub fn codes() -> Vec<&'static str> {
    let codes = match render_contract::contract()["mesh_validation"]["codes"].as_array() {
        Some(codes) => codes,
        None => return Vec::new(),
    };
    codes
        .iter()
        .filter_map(|entry| entry["code"].as_str())
        .collect()
}

/// Stride do vértice canônico declarado no contrato (72 B).
pub fn stride_bytes() -> u64 {
    render_contract::contract()["mesh_validation"]["stride_bytes"]
        .as_u64()
        .unwrap_or(72)
}

/// Valida uma malha antes de qualquer criação de buffer.
pub fn validate_mesh(mesh: &Mesh) -> Result<(), MeshProblem> {
    let vertex_count = mesh.vertices.len();
    let index_count = mesh.indices.len();

    if vertex_count == 0 || index_count == 0 {
        return Err(MeshProblem::new(
            "EMPTY_MESH",
            format!("malha vazia ({vertex_count} vértices, {index_count} índices)"),
        ));
    }

    let stride = stride_bytes();
    if std::mem::size_of::<Vertex>() as u64 != stride {
        return Err(MeshProblem::new(
            "BAD_VERTEX_STRIDE",
            format!(
                "Vertex tem {} B mas o contrato exige {} B por vértice",
                std::mem::size_of::<Vertex>(),
                stride
            ),
        ));
    }

    if index_count % 3 != 0 {
        return Err(MeshProblem::new(
            "INDEX_COUNT_MISMATCH",
            format!("{index_count} índices não formam triângulos"),
        ));
    }

    for (position, index) in mesh.indices.iter().enumerate() {
        if *index as usize >= vertex_count {
            return Err(MeshProblem::new(
                "INDEX_OUT_OF_RANGE",
                format!("índice[{position}] = {index} aponta para fora de {vertex_count} vértices"),
            ));
        }
    }

    for (position, vertex) in mesh.vertices.iter().enumerate() {
        if !is_finite_vertex(vertex) {
            return Err(MeshProblem::new(
                "NON_FINITE_VALUE",
                format!("vértice[{position}] tem NaN/Infinito em posição, normal, uv ou pesos"),
            ));
        }
    }

    Ok(())
}

fn is_finite_vertex(vertex: &Vertex) -> bool {
    vertex.position.iter().all(|value| value.is_finite())
        && vertex.normal.iter().all(|value| value.is_finite())
        && vertex.uv.iter().all(|value| value.is_finite())
        && vertex.weights.iter().all(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle() -> Mesh {
        let mut mesh = Mesh::new();
        mesh.vertices.push(Vertex::new([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0]));
        mesh.vertices.push(Vertex::new([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0]));
        mesh.vertices.push(Vertex::new([0.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0]));
        mesh.indices.extend_from_slice(&[0, 1, 2]);
        mesh
    }

    #[test]
    fn codes_come_from_the_contract() {
        let codes = codes();
        assert!(codes.contains(&"EMPTY_MESH"));
        assert!(codes.contains(&"INDEX_OUT_OF_RANGE"));
        assert!(codes.contains(&"NON_FINITE_VALUE"));
        assert!(codes.contains(&"BAD_VERTEX_STRIDE"));
        assert!(codes.contains(&"INDEX_COUNT_MISMATCH"));
    }

    #[test]
    fn vertex_stride_matches_the_contract() {
        // O headless faz `cast_slice(&mesh.vertices)` direto para o VBO: se o
        // layout do `Vertex` divergir do contrato, o buffer sai torto.
        assert_eq!(std::mem::size_of::<Vertex>() as u64, stride_bytes());
    }

    #[test]
    fn valid_triangle_passes() {
        assert!(validate_mesh(&triangle()).is_ok());
    }

    #[test]
    fn empty_mesh_is_rejected() {
        let problem = validate_mesh(&Mesh::new()).expect_err("malha vazia reprovada");
        assert_eq!(problem.code, "EMPTY_MESH");
    }

    #[test]
    fn out_of_range_index_is_rejected() {
        let mut mesh = triangle();
        mesh.indices[2] = 99;
        let problem = validate_mesh(&mesh).expect_err("índice fora do intervalo");
        assert_eq!(problem.code, "INDEX_OUT_OF_RANGE");
        assert!(problem.detail.contains("99"));
    }

    #[test]
    fn broken_topology_is_rejected() {
        let mut mesh = triangle();
        mesh.indices.pop();
        let problem = validate_mesh(&mesh).expect_err("topologia quebrada");
        assert_eq!(problem.code, "INDEX_COUNT_MISMATCH");
    }

    #[test]
    fn non_finite_position_is_rejected() {
        let mut mesh = triangle();
        mesh.vertices[1].position[0] = f32::NAN;
        let problem = validate_mesh(&mesh).expect_err("NaN reprovado");
        assert_eq!(problem.code, "NON_FINITE_VALUE");
    }

    #[test]
    fn message_carries_the_code() {
        let problem = MeshProblem::new("EMPTY_MESH", "vazio");
        assert!(problem.message().contains("EMPTY_MESH"));
    }
}
