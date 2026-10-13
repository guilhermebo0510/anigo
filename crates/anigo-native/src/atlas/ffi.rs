//! Camada `extern "C"` do xatlas — o **único** lugar com `unsafe` desta crate.
//!
//! Invariantes que tornam [`generate`] seguro de chamar:
//! 1. Antes do `unsafe`, todos os buffers são conferidos: `indices.len()` é múltiplo de 3,
//!    cada índice `< positions.len()`, posições finitas, `tri_ignore`/`tri_material`
//!    (se houver) têm uma entrada por triângulo.
//!    O C++ (xatlas) lê exatamente esses tamanhos e nada além.
//! 2. O C++ copia as entradas (`AddMesh` copia) e não guarda ponteiros após retornar.
//! 3. As saídas são `malloc`adas pelo shim; o `Guard` as libera em `Drop`, também em
//!    caminho de erro/pânico. Antes de montar slices, confere ponteiros não nulos e
//!    contagens coerentes (`index_count` == entrada).
//! 4. Nenhuma exceção C++ cruza a fronteira (o shim captura `...` e devolve código).
//! 5. As structs `#[repr(C)]` espelham `native/anigo_xatlas_shim.h`; o teste
//!    `abi_sizes_match_cpp` compara `size_of` com o `sizeof` do C++.
#![allow(unsafe_code)]

use std::os::raw::c_int;
use std::slice;

#[repr(C)]
struct RawParams {
    max_chart_area: f32,
    max_boundary_length: f32,
    normal_deviation_weight: f32,
    roundness_weight: f32,
    straightness_weight: f32,
    normal_seam_weight: f32,
    texture_seam_weight: f32,
    max_cost: f32,
    max_iterations: u32,
    use_input_mesh_uvs: u8,
    fix_winding: u8,
    max_chart_size: u32,
    padding: u32,
    texels_per_unit: f32,
    resolution: u32,
    bilinear: u8,
    block_align: u8,
    brute_force: u8,
    rotate_charts: u8,
    rotate_charts_to_axis: u8,
}

#[repr(C)]
struct RawResult {
    width: u32,
    height: u32,
    atlas_count: u32,
    chart_count: u32,
    texels_per_unit: f32,
    utilization: f32,
    vertex_count: u32,
    index_count: u32,
    uv: *mut f32,
    atlas_index: *mut i32,
    chart_index: *mut i32,
    xref: *mut u32,
    indices: *mut u32,
}

extern "C" {
    fn anigo_xatlas_generate(
        positions: *const f32,
        vertex_count: u32,
        indices: *const u32,
        tri_count: u32,
        tri_ignore: *const u8,
        tri_material: *const u32,
        params: *const RawParams,
        out: *mut RawResult,
    ) -> c_int;
    fn anigo_xatlas_free_result(out: *mut RawResult);
    #[cfg_attr(not(test), allow(dead_code))]
    fn anigo_xatlas_sizeof_params() -> u32;
    #[cfg_attr(not(test), allow(dead_code))]
    fn anigo_xatlas_sizeof_result() -> u32;
}

/// Libera o resultado do C++ ao sair de escopo.
struct Guard(RawResult);

impl Drop for Guard {
    fn drop(&mut self) {
        // SAFETY: `self.0` foi preenchido (ou zerado) por `anigo_xatlas_generate`;
        // `free_result` aceita ambos e zera a struct (liberação idempotente).
        unsafe { anigo_xatlas_free_result(&mut self.0) }
    }
}

/// Parâmetros em tipos seguros (espelha `AnigoXatlasParams`).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Params {
    pub max_chart_area: f32,
    pub max_boundary_length: f32,
    pub normal_deviation_weight: f32,
    pub roundness_weight: f32,
    pub straightness_weight: f32,
    pub normal_seam_weight: f32,
    pub texture_seam_weight: f32,
    pub max_cost: f32,
    pub max_iterations: u32,
    pub use_input_mesh_uvs: bool,
    pub fix_winding: bool,
    pub max_chart_size: u32,
    pub padding: u32,
    pub texels_per_unit: f32,
    pub resolution: u32,
    pub bilinear: bool,
    pub block_align: bool,
    pub brute_force: bool,
    pub rotate_charts: bool,
    pub rotate_charts_to_axis: bool,
}

impl Params {
    fn to_raw(&self) -> RawParams {
        RawParams {
            max_chart_area: self.max_chart_area,
            max_boundary_length: self.max_boundary_length,
            normal_deviation_weight: self.normal_deviation_weight,
            roundness_weight: self.roundness_weight,
            straightness_weight: self.straightness_weight,
            normal_seam_weight: self.normal_seam_weight,
            texture_seam_weight: self.texture_seam_weight,
            max_cost: self.max_cost,
            max_iterations: self.max_iterations,
            use_input_mesh_uvs: self.use_input_mesh_uvs as u8,
            fix_winding: self.fix_winding as u8,
            max_chart_size: self.max_chart_size,
            padding: self.padding,
            texels_per_unit: self.texels_per_unit,
            resolution: self.resolution,
            bilinear: self.bilinear as u8,
            block_align: self.block_align as u8,
            brute_force: self.brute_force as u8,
            rotate_charts: self.rotate_charts as u8,
            rotate_charts_to_axis: self.rotate_charts_to_axis as u8,
        }
    }
}

/// Saída copiada para memória Rust.
#[derive(Debug, Clone)]
pub(super) struct Output {
    pub width: u32,
    pub height: u32,
    pub atlas_count: u32,
    pub chart_count: u32,
    pub texels_per_unit: f32,
    pub utilization: f32,
    /// UV em texels (não normalizado), um por vértice de saída.
    pub uv: Vec<[f32; 2]>,
    pub atlas_index: Vec<i32>,
    pub chart_index: Vec<i32>,
    /// Índices por corner (mesmo layout da entrada) → vértice de saída.
    pub indices: Vec<u32>,
}

/// Executa o xatlas sobre **triângulos**. Valida todas as entradas (invariante 1)
/// antes do `unsafe`. `indices` tem 3 entradas por triângulo; `tri_ignore` e
/// `tri_material` (se houver) têm uma entrada por triângulo.
pub(super) fn generate(
    positions: &[[f32; 3]],
    indices: &[u32],
    tri_ignore: Option<&[u8]>,
    tri_material: Option<&[u32]>,
    params: &Params,
) -> Result<Output, String> {
    if positions.is_empty() || indices.is_empty() {
        return Err("entrada vazia".into());
    }
    if indices.len() % 3 != 0 {
        return Err("número de índices não é múltiplo de 3".into());
    }
    if positions.len() > u32::MAX as usize / 2 || indices.len() > u32::MAX as usize / 2 {
        return Err("malha grande demais para o xatlas (> 2^31 elementos)".into());
    }
    let tri_count = indices.len() / 3;
    if indices.iter().any(|&i| i as usize >= positions.len()) {
        return Err("índice fora de faixa".into());
    }
    if positions.iter().flatten().any(|c| !c.is_finite()) {
        return Err("posição não finita".into());
    }
    if let Some(fi) = tri_ignore {
        if fi.len() != tri_count {
            return Err("tri_ignore com tamanho errado".into());
        }
    }
    if let Some(fm) = tri_material {
        if fm.len() != tri_count {
            return Err("tri_material com tamanho errado".into());
        }
    }

    let raw_params = params.to_raw();
    let mut guard = Guard(RawResult {
        width: 0,
        height: 0,
        atlas_count: 0,
        chart_count: 0,
        texels_per_unit: 0.0,
        utilization: 0.0,
        vertex_count: 0,
        index_count: 0,
        uv: std::ptr::null_mut(),
        atlas_index: std::ptr::null_mut(),
        chart_index: std::ptr::null_mut(),
        xref: std::ptr::null_mut(),
        indices: std::ptr::null_mut(),
    });

    // SAFETY: invariantes 1–4 do módulo. Os ponteiros vêm de slices vivos durante a
    // chamada, com os tamanhos conferidos acima; `[f32; 3]` tem layout contíguo
    // (3 × f32, sem padding), então `positions.as_ptr()` é um `*const f32` de
    // `3 * positions.len()` elementos.
    let rc = unsafe {
        anigo_xatlas_generate(
            positions.as_ptr().cast::<f32>(),
            positions.len() as u32,
            indices.as_ptr(),
            tri_count as u32,
            tri_ignore.map_or(std::ptr::null(), |s| s.as_ptr()),
            tri_material.map_or(std::ptr::null(), |s| s.as_ptr()),
            &raw_params,
            &mut guard.0,
        )
    };
    if rc != 0 {
        let why = match rc {
            1 => "AddMesh falhou",
            2 => "índice fora de faixa (xatlas)",
            3 => "face inválida (xatlas)",
            4 => "xatlas não produziu malha de saída",
            5 => "exceção C++ capturada",
            6 => "sem memória",
            _ => "código de erro desconhecido",
        };
        return Err(format!("{why} (código {rc})"));
    }

    let r = &guard.0;
    let vc = r.vertex_count as usize;
    let ic = r.index_count as usize;
    if vc == 0
        || ic != indices.len()
        || r.uv.is_null()
        || r.atlas_index.is_null()
        || r.chart_index.is_null()
        || r.indices.is_null()
    {
        return Err("resultado do xatlas inconsistente".into());
    }

    // SAFETY: ponteiros não nulos, alocados pelo shim com exatamente `vc`/`2*vc`/`ic`
    // elementos (ver anigo_xatlas_shim.cpp); permanecem válidos até o `Guard` cair,
    // e copiamos para Vec antes disso. `f32` e `[f32; 2]` têm alinhamento 4.
    let (uv_flat, atlas_index, chart_index, out_indices) = unsafe {
        (
            slice::from_raw_parts(r.uv, vc * 2),
            slice::from_raw_parts(r.atlas_index, vc),
            slice::from_raw_parts(r.chart_index, vc),
            slice::from_raw_parts(r.indices, ic),
        )
    };
    // Defesa em profundidade: o xatlas só devolve índices < vertex_count.
    if out_indices.iter().any(|&i| i as usize >= vc) {
        return Err("índice de saída fora de faixa".into());
    }
    Ok(Output {
        width: r.width,
        height: r.height,
        atlas_count: r.atlas_count,
        chart_count: r.chart_count,
        texels_per_unit: r.texels_per_unit,
        utilization: r.utilization,
        uv: uv_flat.chunks_exact(2).map(|c| [c[0], c[1]]).collect(),
        atlas_index: atlas_index.to_vec(),
        chart_index: chart_index.to_vec(),
        indices: out_indices.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_sizes_match_cpp() {
        // SAFETY: funções puras sem argumentos.
        let (p, r) = unsafe { (anigo_xatlas_sizeof_params(), anigo_xatlas_sizeof_result()) };
        assert_eq!(p as usize, std::mem::size_of::<RawParams>());
        assert_eq!(r as usize, std::mem::size_of::<RawResult>());
    }
}
