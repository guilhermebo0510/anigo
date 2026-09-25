// ANIGO — Anime Cinematic Depth of Field (Fase 2 #53)
//
// Passe de pós-processamento: fullscreen triangle DEPOIS do passe NPR
// (outline + cel). Le a cor + profundidade da cena, reconstrói a distância
// de vista, calcula o Circle of Confusion e desfoca com a forma da abertura
// (bokeh circular ou hexagonal clássico de anime).
//
// Fórmula do CoC (idêntica a anigo-core::math::circle_of_confusion e
// src/services/camera_cinematic.ts — os três implementam o mesmo golden do
// contrato, seção `cinematography`):
//   CoC = |(D − F_dist) / D| × F² / (N × (F_dist − F))
//   D = distância do fragmento, F_dist = distância de foco, F = focal (m),
//   N = número f.
//
// Paridade: o MESMO shader roda no viewport (webgpu_renderer.ts) e no
// headless (headless.rs) com os MESMOS bytes de uniform — desligado
// (dof.enabled = false) o passe nem existe: imagem idêntica ao frame
// congelado.

struct DofUniform {
    // x: focus_distance (m) — plano de foco milimetricamente nítido.
    // y: f_number (abertura) — menor = mais bokeh.
    // z: bokeh_shape (0 = círculo, 1 = hexágono).
    // w: focal length (m).
    params: vec4<f32>,
    // x: largura (px), y: altura (px), z: z_near (m), w: z_far (m).
    resolution: vec4<f32>,
    // x: raio máximo do bokeh (px) — teto de custo.
    // y: altura do sensor (m) — converte CoC (m no sensor) em px.
    limits: vec4<f32>,
};

@group(0) @binding(0) var<uniform> dof: DofUniform;
@group(0) @binding(1) var scene_color: texture_2d<f32>;
// Profundidade real do frame: depth24plus só liga como texture_depth_2d (texture_2d<f32> falha na validação do wgpu/browser).
@group(0) @binding(2) var scene_depth: texture_depth_2d;
@group(0) @binding(3) var dof_sampler: sampler;

// Disco de amostragem determinístico (16 pontos): 4 anel interno (r=0.5) +
// 12 anel externo (r=0.93, a cada 30°). O centro é amostrado à parte com
// peso 1 — é isso que impede halo/serrilha no contorno do personagem em
// foco (aceite 1): a região nítida sempre inclui o pixel original.
const DOF_DISC: array<vec2<f32>, 16> =
    array<vec2<f32>, 16>(
        vec2<f32>(0.353553, 0.353553),
        vec2<f32>(-0.353553, 0.353553),
        vec2<f32>(-0.353553, -0.353553),
        vec2<f32>(0.353553, -0.353553),
        vec2<f32>(0.930000, 0.000000),
        vec2<f32>(0.805404, 0.465000),
        vec2<f32>(0.465000, 0.805404),
        vec2<f32>(0.000000, 0.930000),
        vec2<f32>(-0.465000, 0.805404),
        vec2<f32>(-0.805404, 0.465000),
        vec2<f32>(-0.930000, 0.000000),
        vec2<f32>(-0.805404, -0.465000),
        vec2<f32>(-0.465000, -0.805404),
        vec2<f32>(-0.000000, -0.930000),
        vec2<f32>(0.465000, -0.805404),
        vec2<f32>(0.805404, -0.465000),
    );

// Fullscreen triangle sem buffer de vértices.
@vertex
fn vs_dof(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(vi) * 2 - 1);
    let y = f32(i32(vi) * 2 - 3);
    return vec4<f32>(x, y, 0.0, 1.0);
}

// CoC — o mesmo golden do contrato (cinematography.dof).
fn circle_of_confusion(frag_dist: f32, focus_dist: f32, focal_m: f32, f_number: f32) -> f32 {
    let d = max(frag_dist, 0.0001);
    let fd = max(focus_dist, 0.0001);
    let f = max(focal_m, 0.0001);
    let n = max(f_number, 0.05);
    return (abs(fd - d) / d) * (f * f) / (n * max(fd - f, 0.0001));
}

// Profundidade de hardware (zero_to_one, RH) → distância de vista (m):
//   z = near·far / (far − d·(far − near))
fn linearize_depth(d: f32) -> f32 {
    let near = dof.resolution.z;
    let far = dof.resolution.w;
    return (near * far) / (far - d * (far - near));
}

// Hexágono regular (raio circunscrito 1, vértices em 0/60/…°): o ponto está
// dentro se a projeção em cada direção de aresta (30/90/…°) ≤ apótema
// (cos 30° = 0.8660254). Ponto no eixo do vértice r=0.93 passa; no eixo da
// aresta r=0.93 cai → cantos vivos, arestas chapadas (bokeh hex clássico).
fn hexagon_mask(p: vec2<f32>) -> f32 {
    var max_proj: f32 = -1e9;
    for (var k = 0u; k < 6u; k++) {
        let ang = (30.0 + f32(k) * 60.0) * 0.017453292519943295; // 1° em rad
        max_proj = max(max_proj, dot(p, vec2<f32>(cos(ang), sin(ang))));
    }
    return 1.0 - step(0.8660254, max_proj);
}

fn aperture_mask(p: vec2<f32>, shape: f32) -> f32 {
    if (shape < 0.5) {
        return 1.0; // bokeh circular: todos os pontos do disco
    }
    return hexagon_mask(p);
}

@fragment
fn fs_dof(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = pos.xy / dof.resolution.xy;
    let frag_dist = linearize_depth(textureSampleLevel(scene_depth, dof_sampler, uv, 0));

    // Raio do bokeh em px: CoC (m no sensor) → fração da altura do sensor →
    // px da imagem, limitado por `limits.x` (teto de custo).
    let coc = circle_of_confusion(frag_dist, dof.params.x, dof.params.w, dof.params.y);
    let radius_px = min(coc / dof.limits.y * dof.resolution.y, dof.limits.x);

    // Em foco (ou quase): amostra única — nitidez milimétrica garantida.
    if (radius_px < 0.5) {
        return textureSample(scene_color, dof_sampler, uv);
    }

    var acc = textureSample(scene_color, dof_sampler, uv).rgba;
    var wsum = 1.0;
    let r = radius_px / dof.resolution.y; // fração de tela vertical
    for (var i = 0u; i < 16u; i++) {
        let p = DOF_DISC[i];
        let w = aperture_mask(p, dof.params.z);
        if (w > 0.5) {
            acc += textureSample(scene_color, dof_sampler, uv + vec2<f32>(p.x * r, p.y * r)).rgba * w;
            wsum += w;
        }
    }
    return acc / wsum;
}
