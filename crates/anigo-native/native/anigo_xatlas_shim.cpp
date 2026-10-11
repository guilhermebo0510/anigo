// Implementação da fronteira C do xatlas. Ver anigo_xatlas_shim.h para o contrato.
#include "anigo_xatlas_shim.h"

#include "../vendor/xatlas/xatlas.h"

#include <cstdlib>
#include <cstring>
#include <new>
#include <vector>

namespace {

template <typename T>
T *dup_array(const T *src, size_t count) {
    if (count == 0) return nullptr;
    T *dst = static_cast<T *>(std::malloc(sizeof(T) * count));
    if (dst) std::memcpy(dst, src, sizeof(T) * count);
    return dst;
}

}  // namespace

extern "C" int anigo_xatlas_generate(const float *positions, uint32_t vertex_count,
                                     const uint32_t *indices, uint32_t tri_count,
                                     const uint8_t *tri_ignore, const uint32_t *tri_material,
                                     const AnigoXatlasParams *params, AnigoXatlasResult *out) {
    if (!out) return ANIGO_XATLAS_ERR_ADD_MESH;
    std::memset(out, 0, sizeof(*out));
    if (!positions || !indices || !params || vertex_count == 0 || tri_count == 0 ||
        tri_count > 0x55555555u) {
        return ANIGO_XATLAS_ERR_ADD_MESH;
    }
    const uint32_t index_count = tri_count * 3;

    xatlas::Atlas *atlas = nullptr;
    int rc = ANIGO_XATLAS_OK;
    try {
        atlas = xatlas::Create();
        if (!atlas) return ANIGO_XATLAS_ERR_ALLOC;

        // xatlas::MeshDecl::faceIgnoreData é `const bool *`; converte de bytes.
        std::vector<char> ignore_storage;
        const bool *ignore_ptr = nullptr;
        if (tri_ignore) {
            ignore_storage.resize(tri_count);
            for (uint32_t i = 0; i < tri_count; i++) ignore_storage[i] = tri_ignore[i] ? 1 : 0;
            ignore_ptr = reinterpret_cast<const bool *>(ignore_storage.data());
        }

        xatlas::MeshDecl decl;
        decl.vertexCount = vertex_count;
        decl.vertexPositionData = positions;
        decl.vertexPositionStride = sizeof(float) * 3;
        decl.indexCount = index_count;
        decl.indexData = indices;
        decl.indexFormat = xatlas::IndexFormat::UInt32;
        decl.faceCount = tri_count;
        decl.faceIgnoreData = ignore_ptr;
        decl.faceMaterialData = tri_material;

        xatlas::AddMeshError err = xatlas::AddMesh(atlas, decl);
        if (err != xatlas::AddMeshError::Success) {
            switch (err) {
                case xatlas::AddMeshError::IndexOutOfRange:
                    rc = ANIGO_XATLAS_ERR_INDEX_OUT_OF_RANGE;
                    break;
                case xatlas::AddMeshError::InvalidFaceVertexCount:
                case xatlas::AddMeshError::InvalidIndexCount:
                    rc = ANIGO_XATLAS_ERR_INVALID_FACE;
                    break;
                default:
                    rc = ANIGO_XATLAS_ERR_ADD_MESH;
                    break;
            }
            xatlas::Destroy(atlas);
            return rc;
        }
        xatlas::AddMeshJoin(atlas);

        xatlas::ChartOptions chart;
        chart.maxChartArea = params->max_chart_area;
        chart.maxBoundaryLength = params->max_boundary_length;
        chart.normalDeviationWeight = params->normal_deviation_weight;
        chart.roundnessWeight = params->roundness_weight;
        chart.straightnessWeight = params->straightness_weight;
        chart.normalSeamWeight = params->normal_seam_weight;
        chart.textureSeamWeight = params->texture_seam_weight;
        chart.maxCost = params->max_cost;
        chart.maxIterations = params->max_iterations;
        chart.useInputMeshUvs = params->use_input_mesh_uvs != 0;
        chart.fixWinding = params->fix_winding != 0;

        xatlas::PackOptions pack;
        pack.maxChartSize = params->max_chart_size;
        pack.padding = params->padding;
        pack.texelsPerUnit = params->texels_per_unit;
        pack.resolution = params->resolution;
        pack.bilinear = params->bilinear != 0;
        pack.blockAlign = params->block_align != 0;
        pack.bruteForce = params->brute_force != 0;
        pack.createImage = false;
        pack.rotateCharts = params->rotate_charts != 0;
        pack.rotateChartsToAxis = params->rotate_charts_to_axis != 0;

        xatlas::Generate(atlas, chart, pack);

        if (atlas->meshCount != 1 || !atlas->meshes) {
            xatlas::Destroy(atlas);
            return ANIGO_XATLAS_ERR_NO_OUTPUT;
        }
        const xatlas::Mesh &m = atlas->meshes[0];
        if (m.vertexCount == 0 || m.indexCount != index_count) {
            xatlas::Destroy(atlas);
            return ANIGO_XATLAS_ERR_NO_OUTPUT;
        }

        out->width = atlas->width;
        out->height = atlas->height;
        out->atlas_count = atlas->atlasCount;
        out->chart_count = atlas->chartCount;
        out->texels_per_unit = atlas->texelsPerUnit;
        out->utilization = (atlas->utilization && atlas->atlasCount > 0) ? atlas->utilization[0] : 0.0f;
        out->vertex_count = m.vertexCount;
        out->index_count = m.indexCount;
        out->uv = static_cast<float *>(std::malloc(sizeof(float) * 2 * (size_t)m.vertexCount));
        out->atlas_index = static_cast<int32_t *>(std::malloc(sizeof(int32_t) * (size_t)m.vertexCount));
        out->chart_index = static_cast<int32_t *>(std::malloc(sizeof(int32_t) * (size_t)m.vertexCount));
        out->xref = static_cast<uint32_t *>(std::malloc(sizeof(uint32_t) * (size_t)m.vertexCount));
        out->indices = dup_array<uint32_t>(m.indexArray, m.indexCount);
        if (!out->uv || !out->atlas_index || !out->chart_index || !out->xref || !out->indices) {
            anigo_xatlas_free_result(out);
            xatlas::Destroy(atlas);
            return ANIGO_XATLAS_ERR_ALLOC;
        }
        for (uint32_t i = 0; i < m.vertexCount; i++) {
            out->uv[2 * i] = m.vertexArray[i].uv[0];
            out->uv[2 * i + 1] = m.vertexArray[i].uv[1];
            out->atlas_index[i] = m.vertexArray[i].atlasIndex;
            out->chart_index[i] = m.vertexArray[i].chartIndex;
            out->xref[i] = m.vertexArray[i].xref;
        }
        xatlas::Destroy(atlas);
        return ANIGO_XATLAS_OK;
    } catch (...) {
        if (atlas) xatlas::Destroy(atlas);
        anigo_xatlas_free_result(out);
        return ANIGO_XATLAS_ERR_EXCEPTION;
    }
}

extern "C" void anigo_xatlas_free_result(AnigoXatlasResult *out) {
    if (!out) return;
    std::free(out->uv);
    std::free(out->atlas_index);
    std::free(out->chart_index);
    std::free(out->xref);
    std::free(out->indices);
    std::memset(out, 0, sizeof(*out));
}

extern "C" uint32_t anigo_xatlas_sizeof_params(void) { return (uint32_t)sizeof(AnigoXatlasParams); }
extern "C" uint32_t anigo_xatlas_sizeof_result(void) { return (uint32_t)sizeof(AnigoXatlasResult); }
