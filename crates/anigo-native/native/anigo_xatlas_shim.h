/*
 * Fronteira C estável entre o ANIGO (Rust) e o xatlas (C++).
 *
 * Por que existe: o crate `xatlas-rs` 0.1.3 do crates.io tem os nomes de símbolo
 * MSVC x64 gravados em `#[link_name]` (só linka no Windows/MSVC) e o `-v2` exige
 * libclang (bindgen). Esta camada `extern "C"` evita as duas coisas: nenhum nome
 * C++ mangled, nenhuma geração de bindings, só `cc`.
 *
 * Contrato de memória:
 *  - Entradas são ponteiros de leitura, válidos só durante a chamada (o xatlas copia).
 *  - Saídas são alocadas pelo shim (malloc) e devem ser liberadas por
 *    anigo_xatlas_free_result(). O lado Rust copia tudo para Vec<> e libera de imediato.
 *  - Nenhuma exceção C++ atravessa esta fronteira (try/catch -> código de erro).
 */
#ifndef ANIGO_XATLAS_SHIM_H
#define ANIGO_XATLAS_SHIM_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct AnigoXatlasParams {
    /* ChartOptions */
    float max_chart_area;
    float max_boundary_length;
    float normal_deviation_weight;
    float roundness_weight;
    float straightness_weight;
    float normal_seam_weight;
    float texture_seam_weight;
    float max_cost;
    uint32_t max_iterations;
    uint8_t use_input_mesh_uvs;
    uint8_t fix_winding;
    /* PackOptions */
    uint32_t max_chart_size;
    uint32_t padding;
    float texels_per_unit;
    uint32_t resolution;
    uint8_t bilinear;
    uint8_t block_align;
    uint8_t brute_force;
    uint8_t rotate_charts;
    uint8_t rotate_charts_to_axis;
} AnigoXatlasParams;

typedef struct AnigoXatlasResult {
    uint32_t width;
    uint32_t height;
    uint32_t atlas_count;
    uint32_t chart_count;
    float texels_per_unit;
    float utilization; /* do primeiro sub-atlas */
    uint32_t vertex_count;
    uint32_t index_count;
    float *uv;             /* 2 * vertex_count, em texels (não normalizado) */
    int32_t *atlas_index;  /* vertex_count; -1 = vértice fora do atlas */
    int32_t *chart_index;  /* vertex_count; -1 = vértice fora de qualquer chart */
    uint32_t *xref;        /* vertex_count; índice do vértice de entrada */
    uint32_t *indices;     /* index_count; mesmo layout de corners da entrada */
} AnigoXatlasResult;

/* Códigos de retorno de anigo_xatlas_generate. */
enum {
    ANIGO_XATLAS_OK = 0,
    ANIGO_XATLAS_ERR_ADD_MESH = 1,
    ANIGO_XATLAS_ERR_INDEX_OUT_OF_RANGE = 2,
    ANIGO_XATLAS_ERR_INVALID_FACE = 3,
    ANIGO_XATLAS_ERR_NO_OUTPUT = 4,
    ANIGO_XATLAS_ERR_EXCEPTION = 5,
    ANIGO_XATLAS_ERR_ALLOC = 6
};

/*
 * positions:     3 * vertex_count floats.
 * indices:       3 * tri_count índices u32 (apenas TRIÂNGULOS: o modo polígono do xatlas
 *                vendorizado está quebrado — ver docs/native/CPP_FFI.md).
 * tri_ignore:    NULL ou tri_count bytes (!=0 = não atlasar; UV fica (0,0), atlas_index -1).
 * tri_material:  NULL ou tri_count u32 (charts nunca misturam materiais).
 * A saída `indices` tem o mesmo layout da entrada (3 por triângulo, mesma ordem).
 * Retorna ANIGO_XATLAS_OK e preenche *out, ou um código de erro (out zerado).
 */
int anigo_xatlas_generate(const float *positions, uint32_t vertex_count,
                          const uint32_t *indices, uint32_t tri_count,
                          const uint8_t *tri_ignore, const uint32_t *tri_material,
                          const AnigoXatlasParams *params, AnigoXatlasResult *out);

void anigo_xatlas_free_result(AnigoXatlasResult *out);

/* tamanhos das structs vistos pelo C++, para o Rust conferir o ABI nos testes. */
uint32_t anigo_xatlas_sizeof_params(void);
uint32_t anigo_xatlas_sizeof_result(void);

#ifdef __cplusplus
}
#endif

#endif
