// ANIGO WebGPU Sparse Morph Target Compute Shader (WGSL)
// Processes active blendshape channels on indexed sparse deltas directly on GPU VRAM.

struct SparseMorphHeader {
    active_channel_count: u32,
    total_vertex_count: u32,
    total_delta_count: u32,
    _pad: u32,
};

struct MorphChannel {
    weight: f32,
    start_offset: u32,
    delta_count: u32,
    _pad: u32,
};

struct SparseMorphDelta {
    vertex_index: u32,
    delta_px: f32,
    delta_py: f32,
    delta_pz: f32,
    delta_nx: f32,
    delta_ny: f32,
    delta_nz: f32,
    _pad: f32,
};

// 72-byte packed NPR vertex structure layout
struct VertexRaw {
    pos_x: f32,
    pos_y: f32,
    pos_z: f32,
    norm_x: f32,
    norm_y: f32,
    norm_z: f32,
    uv_u: f32,
    uv_v: f32,
    col_r: f32,
    col_g: f32,
    col_b: f32,
    col_a: f32,
    joints_0_1: u32,
    joints_2_3: u32,
    weight_0: f32,
    weight_1: f32,
    weight_2: f32,
    weight_3: f32,
};

@group(0) @binding(0) var<uniform> header: SparseMorphHeader;
@group(0) @binding(1) var<storage, read> base_vertices: array<VertexRaw>;
@group(0) @binding(2) var<storage, read> morph_deltas: array<SparseMorphDelta>;
@group(0) @binding(3) var<storage, read> active_channels: array<MorphChannel>;
@group(0) @binding(4) var<storage, read_write> out_vertices: array<VertexRaw>;

@compute @workgroup_size(64)
fn cs_accumulate_morphs(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let vert_idx = global_id.x;
    if (vert_idx >= header.total_vertex_count) {
        return;
    }

    var base_v = base_vertices[vert_idx];
    var p = vec3<f32>(base_v.pos_x, base_v.pos_y, base_v.pos_z);
    var n = vec3<f32>(base_v.norm_x, base_v.norm_y, base_v.norm_z);

    for (var c: u32 = 0u; c < header.active_channel_count; c = c + 1u) {
        let ch = active_channels[c];
        if (abs(ch.weight) > 1e-6 && ch.delta_count > 0u) {
            let start = ch.start_offset;
            let count = ch.delta_count;

            var low: u32 = 0u;
            var high: u32 = count;
            var found_idx: u32 = 0xFFFFFFFFu;

            while (low < high) {
                let mid = low + (high - low) / 2u;
                let delta_vert = morph_deltas[start + mid].vertex_index;
                if (delta_vert == vert_idx) {
                    found_idx = start + mid;
                    break;
                } else if (delta_vert < vert_idx) {
                    low = mid + 1u;
                } else {
                    high = mid;
                }
            }

            if (found_idx != 0xFFFFFFFFu) {
                let delta = morph_deltas[found_idx];
                p = p + ch.weight * vec3<f32>(delta.delta_px, delta.delta_py, delta.delta_pz);
                n = n + ch.weight * vec3<f32>(delta.delta_nx, delta.delta_ny, delta.delta_nz);
            }
        }
    }

    let n_sq = dot(n, n);
    if (n_sq > 1e-12) {
        n = normalize(n);
    }

    base_v.pos_x = p.x;
    base_v.pos_y = p.y;
    base_v.pos_z = p.z;
    base_v.norm_x = n.x;
    base_v.norm_y = n.y;
    base_v.norm_z = n.z;

    out_vertices[vert_idx] = base_v;
}

@compute @workgroup_size(64)
fn cs_reset_vertices(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let vert_idx = global_id.x;
    if (vert_idx >= header.total_vertex_count) {
        return;
    }
    out_vertices[vert_idx] = base_vertices[vert_idx];
}
