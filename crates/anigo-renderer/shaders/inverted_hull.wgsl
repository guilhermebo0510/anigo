// ANIGO Stylized Anime Inverted Hull Outline Shader (WGSL)
// Extrudes vertices along normals with camera distance and aspect ratio compensation

struct CameraUniform {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    model: mat4x4<f32>,          // P2-14 model
    normal_mat: mat4x4<f32>,     // P2-14
};

struct OutlineUniform {
    color: vec4<f32>,
    params: vec4<f32>,  // x: line_width, y: aspect_ratio, z: depth_bias, w: opacity
    params2: vec4<f32>, // x: smoothness, y: width_tex_enabled (Fase 2 #18), z: reserved
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(0) @binding(1)
var<uniform> outline: OutlineUniform;

// Fase 2 (#18): mapa de espessura do contorno (MToon outlineWidth). O canal R
// modula a largura uniforme por UV; sem textura o slot é desativado por
// params2.y e o renderer ancora um neutro branco 1x1 (modulação 1.0).
@group(0) @binding(3)
var outline_width_tex: texture_2d<f32>;

@group(0) @binding(4)
var outline_width_sampler: sampler;

struct BonePalette {
    matrices: array<mat4x4<f32>, 24>,
};

@group(0) @binding(2)
var<uniform> bones: BonePalette;

// ANIGO-SKINNING-BEGIN — bloco compartilhado (byte a byte igual entre shaders; conferido por scripts/check_wgsl.mjs)
const PALETTE_JOINT_COUNT: u32 = 24u;

// Linear Blend Skinning: soma ponderada das matrizes dos ossos com peso > 0.
// Os pesos são normalizados **aqui** (o buffer de vértice carrega o peso cru do
// GLB), então peso total nulo (vértice sem skin) devolve a identidade em vez de
// colapsar o vértice na origem.
fn skin_palette(joints: vec4<u32>, weights: vec4<f32>) -> mat4x4<f32> {
    var blend = mat4x4<f32>();
    var total = 0.0;
    for (var slot = 0u; slot < 4u; slot = slot + 1u) {
        let weight = weights[slot];
        if (weight <= 1e-6) {
            continue;
        }
        let bone = min(joints[slot], PALETTE_JOINT_COUNT - 1u);
        blend = blend + bones.matrices[bone] * weight;
        total = total + weight;
    }
    if (total < 1e-5) {
        return mat4x4<f32>(
            vec4<f32>(1.0, 0.0, 0.0, 0.0),
            vec4<f32>(0.0, 1.0, 0.0, 0.0),
            vec4<f32>(0.0, 0.0, 1.0, 0.0),
            vec4<f32>(0.0, 0.0, 0.0, 1.0),
        );
    }
    let inv_total = 1.0 / total;
    return blend * inv_total;
}
// ANIGO-SKINNING-END

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>, // B channel modulates outline thickness (0 = no outline)
    @location(4) joints: vec4<u32>,
    @location(5) weights: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    if (in.color.b <= 0.001) {
        out.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }

    // P1-04: o contorno segue a mesma pele do cel shading (mesma paleta, mesma
    // função) — sem isto o hull ficaria na pose de repouso.
    let skin = skin_palette(in.joints, in.weights);
    let skinned_position = (skin * vec4<f32>(in.position, 1.0)).xyz;
    let skinned_normal = (skin * vec4<f32>(in.normal, 0.0)).xyz;
    let world_pos = camera.model * vec4<f32>(skinned_position, 1.0);
    var clip_pos = camera.view_proj * world_pos;

    // Extrusion along normal in clip space — P2-01 depth bias calibrated, scaled by distance (clip_pos.w) to maintain constant screen thickness
    let world_n = normalize((camera.normal_mat * vec4<f32>(skinned_normal, 0.0)).xyz);
    let normal_vec4 = camera.view_proj * vec4<f32>(world_n, 0.0); // P2-14 world normal
    let len = length(normal_vec4.xy);
    let normal_clip = select(vec2<f32>(0.0, 0.0), normal_vec4.xy / len, len > 1e-5);

    // Fase 2 (#18): modulação por mapa de espessura (R do slot outlineWidth)
    var width_mod: f32;
    if (outline.params2.y > 0.5) {
        width_mod = textureSample(outline_width_tex, outline_width_sampler, in.uv).r;
    } else {
        width_mod = 1.0;
    }
    let thickness = outline.params.x * in.color.b * width_mod;
    let aspect = max(outline.params.y, 0.001);
    let depth_bias = outline.params.z;

    clip_pos.x += (normal_clip.x / aspect) * thickness * clip_pos.w;
    clip_pos.y += normal_clip.y * thickness * clip_pos.w;
    clip_pos.z += depth_bias * clip_pos.w;

    out.clip_position = clip_pos;
    return out;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    let opacity = outline.params.w;
    let smoothness = max(outline.params2.x, 0.0);
    var out_col = vec4<f32>(outline.color.rgb, outline.color.a * opacity);
    if (smoothness > 0.001) {
        let aa = clamp(smoothness * 8.0, 0.0, 1.0);
        out_col.a = out_col.a * mix(1.0, 0.85, aa);
    }
    return out_col;
}

