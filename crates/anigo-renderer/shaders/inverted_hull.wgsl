// ANIGO Stylized Anime Inverted Hull Outline Shader (WGSL)
// Extrudes vertices along normals with camera distance and aspect ratio compensation
//
// Motor cel (Fase 1) — contorno de alta qualidade, tudo opcional e com padrão
// que reproduz o shader anterior byte a byte:
//  - normal SUAVIZADA de contorno (@location(6), segundo stream só deste
//    passe): sem frestas em arestas duras. O lado Rust a calcula (soldagem por
//    posição + média por ângulo, anigo-core::npr::smooth_outline_normals); sem
//    suavização o stream carrega a própria normal da malha.
//  - largura por distância/FOV (params2.z = falloff, params2.w = distância de
//    referência efetiva já com o FOV): afina ao longe, engrossa perto.
//  - taper (params3.x): peso de traço por orientação da superfície.
// A matemática é espelhada em anigo-core::npr (outline_width_scale,
// outline_taper_scale) com testes golden.

struct CameraUniform {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    model: mat4x4<f32>,          // P2-14 model
    normal_mat: mat4x4<f32>,     // P2-14
};

struct OutlineUniform {
    color: vec4<f32>,
    params: vec4<f32>,  // x: line_width, y: aspect_ratio, z: depth_bias, w: opacity
    params2: vec4<f32>, // x: smoothness, y: width_tex_enabled (Fase 2 #18), z: distance_falloff (0 = espessura constante em px), w: ref_distance_effective (m, com FOV)
    params3: vec4<f32>, // x: taper (0..1), y: min_width_scale (piso do falloff), z/w: reserved
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

// ── Matemática do contorno (pura; espelhada em anigo-core::npr com testes que
// executam ESTE texto numa GPU e comparam com o Rust) ────────────────────────

// Escala da largura por distância: k = ref_eff / profundidade de vista (clip.w);
// escala = k^falloff entre [piso, 4]. falloff <= 0 → 1.0 exato (espessura
// constante em pixels, o comportamento anterior).
fn outline_width_scale(view_depth: f32, ref_eff: f32, falloff: f32, min_scale: f32) -> f32 {
    if (falloff > 0.0) {
        let k = ref_eff / max(view_depth, 0.001);
        return clamp(pow(k, falloff), min_scale, 4.0);
    }
    return 1.0;
}

// Taper por orientação: facing = |N·V| (0 = silhueta rasante, 1 = de frente);
// 1 - taper*0.75*facing. taper = 0 → 1.0 exato.
fn outline_taper_scale(n_dot_v: f32, taper: f32) -> f32 {
    return 1.0 - taper * 0.75 * min(abs(n_dot_v), 1.0);
}

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>, // B channel modulates outline thickness (0 = no outline)
    @location(4) joints: vec4<u32>,
    @location(5) weights: vec4<f32>,
    // Segundo stream (só o contorno): normal de extrusão (suavizada ou da malha).
    @location(6) outline_normal: vec3<f32>,
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
    let skinned_normal = (skin * vec4<f32>(in.outline_normal, 0.0)).xyz;
    let world_pos = camera.model * vec4<f32>(skinned_position, 1.0);
    var clip_pos = camera.view_proj * world_pos;

    // Extrusion along normal in clip space — P2-01 depth bias calibrated, scaled by distance (clip_pos.w) to maintain constant screen thickness
    let world_n = normalize((camera.normal_mat * vec4<f32>(skinned_normal, 0.0)).xyz);
    let normal_vec4 = camera.view_proj * vec4<f32>(world_n, 0.0); // P2-14 world normal
    let len = length(normal_vec4.xy);
    let normal_clip = select(vec2<f32>(0.0, 0.0), normal_vec4.xy / len, len > 1e-5);

    // Largura por distância/FOV e taper por orientação (funções puras acima,
    // espelhadas em anigo-core::npr). Ambos 1.0 exato nos padrões.
    let width_scale = outline_width_scale(clip_pos.w, outline.params2.w, outline.params2.z, outline.params3.y);
    let to_camera = normalize(camera.camera_pos.xyz - world_pos.xyz);
    let taper_scale = outline_taper_scale(dot(world_n, to_camera), outline.params3.x);

    let thickness = outline.params.x * in.color.b * width_scale * taper_scale;
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

