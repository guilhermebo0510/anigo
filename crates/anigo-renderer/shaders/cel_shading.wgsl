// ANIGO Stylized Anime Cel-Shading NPR Shader (WGSL) - Sprint 02 Standard
// Features:
// - Hardware 1D/2D Texture Sampling for Toon Ramp with subpixel linear filtering
// - Analytical Multi-Band fallback (1-step hard anime, 2-step Ghibli soft, continuous)
// - Mathematical Hue-Shifting in shadows (RGB <-> HSV color wheel rotation)
// - Chromaticity-preserving luminance scaling (zero blowout to solid white)
// - Anisotropic Specular Highlight with anime jitter banding ("angel ring")
// - Stylized Fresnel Rim Light with vertex mask and back-light incidence
// - Stylized Ambient Occlusion from vertex attribute (R channel)
// - Shadow shift bias from vertex attribute (G channel)

struct CameraUniform {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    model: mat4x4<f32>,          // P2-14 model matrix (was identity)
    normal_mat: mat4x4<f32>,     // P2-14 normal matrix (inverse transpose of model, padded to mat4)
};

struct LightUniform {
    direction: vec4<f32>,       // xyz: normalized light dir, w: intensity
    color: vec4<f32>,           // rgb: light color, w: ambient_intensity
    shadow_color: vec4<f32>,    // rgb: cool/warm hue-shifted shadow tint, w: saturation
    ambient_sky: vec4<f32>,     // P2-04 sky hemisphere color (rgb) + unused w
    ambient_ground: vec4<f32>,  // P2-04 ground hemisphere color
};

struct MaterialUniform {
    base_color: vec4<f32>,
    shade_color: vec4<f32>,
    specular_color: vec4<f32>,
    rim_color: vec4<f32>,       // P0-09: separate rim tint (was light.shadow_color)
    params: vec4<f32>,          // x: shadow_threshold, y: shadow_smoothness, z: spec_intensity, w: spec_power
    params2: vec4<f32>,         // x: rim_intensity, y: rim_spread, z: hue_shift_rad, w: toon_steps
    params3: vec4<f32>,         // x: spec_softness, y: spec_offset, z: spec_size, w: ao_intensity
    // ── Fase 2 (#18) — material anime VRoid/MToon ─────────────────────────
    emission_color: vec4<f32>,  // MToon subEmission (cor, HDR via intensity)
    params4: vec4<f32>,         // x: emission_intensity, y: second_shade_shift, z: second_shade_softness, w: matcap_intensity
    params5: vec4<f32>,         // x: main_tex_enabled, y: shade_tex_enabled, z: second_shade_enabled, w: emission_enabled
    params6: vec4<f32>,         // x: matcap_enabled, y: matcap_mode (0 normal / 1 additive), z: shade_toony, w: reserved
    // ── Fase 2 (#17) — sombra facial SDF ──────────────────────────────────
    params7: vec4<f32>,         // x: face_shadow_offset, y: face_shadow_smoothness, z: face_sdf_enabled, w: reserved
    // ── Fase 2 (#43) — olho anime (parallax + highlights) ─────────────────
    params8: vec4<f32>,         // x: eye_depth_scale, y: eye_highlight_intensity, z: eye_enabled, w: reserved
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(0) @binding(1)
var<uniform> light: LightUniform;

@group(0) @binding(2)
var<uniform> material: MaterialUniform;

@group(0) @binding(3)
var toon_ramp_tex: texture_2d<f32>;

@group(0) @binding(4)
var toon_ramp_sampler: sampler;

// Fase 2 (#18): slots de textura do material anime (VRoid/MToon). Sem textura,
// o slot é desativado pelo flag em material.params5 e o passe segue 100%
// procedural (o renderer ancora um neutro 1x1 nesses bindings).
@group(0) @binding(6)
var main_tex: texture_2d<f32>;

@group(0) @binding(7)
var main_sampler: sampler;

@group(0) @binding(8)
var shade_tex: texture_2d<f32>;

@group(0) @binding(9)
var shade_sampler: sampler;

@group(0) @binding(10)
var second_shade_tex: texture_2d<f32>;

@group(0) @binding(11)
var second_shade_sampler: sampler;

@group(0) @binding(12)
var emission_tex: texture_2d<f32>;

@group(0) @binding(13)
var emission_sampler: sampler;

@group(0) @binding(14)
var sphere_add_tex: texture_2d<f32>;

@group(0) @binding(15)
var sphere_add_sampler: sampler;

// Fase 2 (#17): mapa SDF da sombra facial (canal R; ver face_sdf.wgsl).
// Sem textura, o slot é desativado por material.params7.z e o renderer
// ancora o neutro 1x1 branco (fator de sombra 0).
@group(0) @binding(16)
var face_sdf_tex: texture_2d<f32>;

@group(0) @binding(17)
var face_sdf_sampler: sampler;

struct BonePalette {
    matrices: array<mat4x4<f32>, 24>,
};

@group(0) @binding(5)
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
    @location(3) color: vec4<f32>, // r: AO, g: shadow shift, b: outline mask, a: specular/rim mask
    @location(4) joints: vec4<u32>,
    @location(5) weights: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) world_position: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) anime_attr: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    // P1-04: deformação por osso antes do model matrix. A matriz é rígida, então
    // serve para posição (w = 1) e para direção (w = 0).
    let skin = skin_palette(in.joints, in.weights);
    let skinned_position = (skin * vec4<f32>(in.position, 1.0)).xyz;
    let skinned_normal = (skin * vec4<f32>(in.normal, 0.0)).xyz;
    let world_pos = camera.model * vec4<f32>(skinned_position, 1.0);
    out.clip_position = camera.view_proj * world_pos;
    out.world_position = world_pos.xyz;
    out.world_normal = normalize((camera.normal_mat * vec4<f32>(skinned_normal, 0.0)).xyz);
    out.uv = in.uv;
    out.anime_attr = in.color;
    return out;
}

// ─────────────────────────────────────────────────────────────
// Color Space Conversions & Mathematical Hue Shifting
// ─────────────────────────────────────────────────────────────

// P1-01: sRGB ↔ linear (IEC 61966)
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let a = vec3<f32>(0.055);
    let lo = c / 12.92;
    let hi = pow((c + a) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}
fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let a = vec3<f32>(0.055);
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0/2.4)) - a;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn rgb_to_hsv(c: vec3<f32>) -> vec3<f32> {
    let K = vec4<f32>(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    let p = select(vec4<f32>(c.bg, K.wz), vec4<f32>(c.gb, K.xy), c.b < c.g);
    let q = select(vec4<f32>(p.xyw, c.r), vec4<f32>(c.r, p.yzx), p.x < c.r);
    let d = q.x - min(q.w, q.y);
    let e = 1.0e-10;
    return vec3<f32>(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

fn hsv_to_rgb(c: vec3<f32>) -> vec3<f32> {
    let K = vec4<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    let p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
    return c.z * mix(K.xxx, clamp(p - K.xxx, vec3<f32>(0.0), vec3<f32>(1.0)), c.y);
}

// P1-02: OKLab perceptually uniform hue rotation (Bottosson 2020) — fallback to HSV for low chroma
fn linear_srgb_to_oklab(c: vec3<f32>) -> vec3<f32> {
    let l = 0.4122214708*c.r + 0.5363325363*c.g + 0.0514459929*c.b;
    let m = 0.2119034982*c.r + 0.6806995451*c.g + 0.1073969566*c.b;
    let s = 0.0883024619*c.r + 0.2817188376*c.g + 0.6299787005*c.b;
    let l_ = pow(max(l, 0.0), 1.0/3.0);
    let m_ = pow(max(m, 0.0), 1.0/3.0);
    let s_ = pow(max(s, 0.0), 1.0/3.0);
    return vec3<f32>(
        0.2104542553*l_ + 0.7936177850*m_ - 0.0040720468*s_,
        1.9779984951*l_ - 2.4285922050*m_ + 0.4505937099*s_,
        0.0259040371*l_ + 0.7827717662*m_ - 0.8086757660*s_
    );
}
fn oklab_to_linear_srgb(c: vec3<f32>) -> vec3<f32> {
    let l_ = c.x + 0.3963377774*c.y + 0.2158037573*c.z;
    let m_ = c.x - 0.1055613458*c.y - 0.0638541728*c.z;
    let s_ = c.x - 0.0894841775*c.y - 1.2914855480*c.z;
    let l = l_*l_*l_;
    let m = m_*m_*m_;
    let s = s_*s_*s_;
    return vec3<f32>(
        4.0767416621*l - 3.3077115913*m + 0.2309699292*s,
        -1.2684380046*l + 2.6097574011*m - 0.3413193965*s,
        -0.0041960863*l - 0.7034186147*m + 1.7076147010*s
    );
}
fn apply_hue_shift(rgb_linear: vec3<f32>, shift_radians: f32, sat_mult: f32) -> vec3<f32> {
    let shift = clamp(shift_radians, -3.14159265, 3.14159265);
    var lab = linear_srgb_to_oklab(rgb_linear);
    let C = length(lab.yz);
    if (C < 0.0001) {
        return rgb_linear * mix(1.0, sat_mult, 0.5);
    }
    let hue = atan2(lab.z, lab.y);
    let new_hue = hue + shift;
    let C2 = clamp(C * sat_mult, 0.0, 0.4);
    lab.y = C2 * cos(new_hue);
    lab.z = C2 * sin(new_hue);
    return oklab_to_linear_srgb(lab);
}

// ─────────────────────────────────────────────────────────────

// P3-01 HDR tonemap (ACES simplified + Reinhard) + exposure/white balance
fn tonemap_aces(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51; let b = 0.03; let c = 2.43; let d = 0.59; let e = 0.14;
    return clamp((x*(a*x+b))/(x*(c*x+d)+e), vec3<f32>(0.0), vec3<f32>(1.0));
}
fn tonemap_reinhard(x: vec3<f32>) -> vec3<f32> {
    return x / (1.0 + x);
}

// Fase 2 (#17): sombra facial SDF — a mesma definição canônica que está em
// face_sdf.wgsl (marcadores conferidos por check:wgsl).
// ANIGO-FACE-SDF-BEGIN — bloco compartilhado (byte a byte igual em
// face_sdf.wgsl e cel_shading.wgsl; conferido por scripts/check_wgsl.mjs)
fn face_sdf_theta(local_x: vec3<f32>, local_z: vec3<f32>, light_dir: vec3<f32>) -> f32 {
    let lx = dot(light_dir, local_x);
    let lz = dot(light_dir, local_z);
    return atan2(lx, lz);
}
fn face_sdf_threshold(theta: f32, offset: f32) -> f32 {
    // light_front: 1 = luz frontal (theta ≈ 0), 0 = luz traseira (|theta| ≈ π).
    // A banda de sombra cresce até 0.25 de threshold quando a luz vai para o
    // lado/costas; offset é o controle do usuário (face_shadow_offset).
    let light_front = cos(theta) * 0.5 + 0.5;
    return 0.5 + (1.0 - light_front) * 0.25 + offset;
}
fn face_sdf_factor(sdf: f32, threshold: f32, softness: f32) -> f32 {
    // 1 = totalmente na sombra, 0 = fora da região de sombra.
    let s = max(softness, 0.001);
    return 1.0 - smoothstep(threshold - s, threshold + s, sdf);
}
// ANIGO-FACE-SDF-END

// Fase 2 (#43): olho anime — a mesma definição canônica que está em
// anime_eye.wgsl (marcadores conferidos por check:wgsl).
// ANIGO-ANIME-EYE-BEGIN — bloco compartilhado (byte a byte igual em
// anime_eye.wgsl e cel_shading.wgsl; conferido por scripts/check_wgsl.mjs)
fn eye_parallax_uv(uv: vec2<f32>, view_tangent_xy: vec2<f32>, depth_scale: f32) -> vec2<f32> {
    // V_tangent.xy: direção de visão projetada na base tangente (T, B) da
    // superfície. depth_scale é a "recalada" da íris (0 = plano, >0 = fundo).
    return clamp(uv + view_tangent_xy * depth_scale, vec2<f32>(0.0), vec2<f32>(1.0));
}
fn eye_highlight_mask(uv: vec2<f32>) -> f32 {
    // Dois brilhos desenhados à mão (convenção clássica de olho anime):
    // principal = elipse grande no canto superior-esquerco do olho,
    // secundário = ponto menor no canto inferior-direito (85% da intensidade).
    let d_main = length((uv - vec2<f32>(0.38, 0.62)) * vec2<f32>(1.0, 0.72));
    let main = 1.0 - smoothstep(0.075, 0.125, d_main);
    let d_second = length(uv - vec2<f32>(0.68, 0.34));
    let second = (1.0 - smoothstep(0.028, 0.055, d_second)) * 0.85;
    return max(main, second);
}
fn eye_highlight_rgb(mask: f32, intensity: f32) -> vec3<f32> {
    // Branco linear × mask × intensidade — somado após a iluminação (nunca
    // multiplicado por luz/sombra): visível mesmo em sombra total.
    return vec3<f32>(mask * intensity);
}
// ANIGO-ANIME-EYE-END

// Fragment Shader
// ─────────────────────────────────────────────────────────────

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let N = normalize(in.world_normal);
    let L = normalize(light.direction.xyz);
    let V = normalize(camera.camera_pos.xyz - in.world_position);

    // Fase 2 (#43): UV da íris com parallax — deslocado pela direção de visão
    // na base tangente (UV + V_tangent.xy × depth_scale). Com eye off ou
    // depth_scale 0 o resultado é exatamente in.uv (frame congelado intacto).
    var eye_uv = in.uv;
    if (material.params8.z > 0.5) {
        let eye_up = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(N.y) > 0.99);
        let eye_t = normalize(cross(N, eye_up));
        let eye_b = cross(N, eye_t);
        let v_tangent = vec2<f32>(dot(V, eye_t), dot(V, eye_b));
        eye_uv = eye_parallax_uv(in.uv, v_tangent, material.params8.x);
    }

    // 1. Half-Lambert Remapping (0..1)
    let n_dot_l = dot(N, L);
    let half_lambert = n_dot_l * 0.5 + 0.5;

    // 2. Vertex Attribute Shadow Bias (G channel = shadow shift) & Smoothness
    let shadow_shift = (in.anime_attr.g - 0.5) * 0.3;
    let threshold = material.params.x + shadow_shift;
    let smoothness = max(material.params.y, 0.001);

    // 3. Toon Ramp 1D/2D Texture Sampling with Analytical Fallback
    let toon_steps = material.params2.w;
    let ramp_v = select(
        select(0.125, 0.375, toon_steps >= 0.5),
        select(0.625, 0.875, toon_steps >= 2.5),
        toon_steps >= 1.5
    );
    let u_coord = clamp((half_lambert - threshold) + 0.5, 0.002, 0.998);
    let ramp_sample = textureSample(toon_ramp_tex, toon_ramp_sampler, vec2<f32>(u_coord, ramp_v));

    var analytical_toon: f32;
    if (toon_steps < 0.5) {
        // Continuous / Smooth anime gradient
        analytical_toon = smoothstep(threshold - 0.35 - smoothness, threshold + 0.35 + smoothness, half_lambert);
    } else if (toon_steps < 1.5) {
        // 1 Degrau (Hard Anime Cel)
        analytical_toon = smoothstep(threshold - smoothness, threshold + smoothness, half_lambert);
    } else if (toon_steps < 2.5) {
        // 2 Degraus (Ghibli Soft)
        let s1 = smoothstep(threshold - 0.14 - smoothness, threshold - 0.14 + smoothness, half_lambert);
        let s2 = smoothstep(threshold + 0.14 - smoothness, threshold + 0.14 + smoothness, half_lambert);
        analytical_toon = s1 * 0.45 + s2 * 0.55;
    } else {
        // 3 Degraus (High-Key Multi-band)
        let s1 = smoothstep(threshold - 0.20 - smoothness, threshold - 0.20 + smoothness, half_lambert);
        let s2 = smoothstep(threshold - smoothness, threshold + smoothness, half_lambert);
        let s3 = smoothstep(threshold + 0.20 - smoothness, threshold + 0.20 + smoothness, half_lambert);
        analytical_toon = (s1 + s2 + s3) / 3.0;
    }

    // P1-03: single analytical model (smoothness controls penumbra only, not model switch)
    let toon_factor = analytical_toon; // ramp_sample kept for validation but not mixed

    // 4. Stylized Ambient Occlusion (R channel) — P2-05 applied to diffuse/ambient only
    let ao_raw = clamp(in.anime_attr.r, 0.0, 1.0);
    let ao = mix(1.0, ao_raw, 0.85); // P2-05 aoIntensity 0..1 lerp (keeps 15% base to avoid black)

    // 5. Mathematical Hue-Shifting in Shadows & Saturation — P1-01/02 linear OKLab
    let hue_shift_rad = clamp(material.params2.z, -3.14159265, 3.14159265);
    let shade_lin = srgb_to_linear(material.shade_color.rgb);
    let shadow_tint_lin = srgb_to_linear(light.shadow_color.rgb);
    let raw_shadow_color = shade_lin * shadow_tint_lin;
    let shadow_sat = clamp(max(light.shadow_color.w, 0.0), 0.0, 2.0);
    let hue_shifted_shadow = apply_hue_shift(raw_shadow_color, hue_shift_rad, shadow_sat);

    // 6. Base Lit and Shadow Blending + P2-04 Hemisphere Ambient
    let intensity = clamp(light.direction.w, 0.0, 3.0);
    // Fase 2 (#18): albedo — MToon mainTex (slot ativado por params5.x) ou cor base.
    // A textura modula a cor base (VRoid: mainTex × baseColorFactor).
    // Fase 2 (#43): quando o olho anime está ativo, o slot main amostra com o
    // UV parallaxado (a textura de olho desliza com a câmera — íris afundada).
    var base_lin: vec3<f32>;
    if (material.params5.x > 0.5) {
        base_lin = srgb_to_linear(textureSample(main_tex, main_sampler, eye_uv).rgb) * srgb_to_linear(material.base_color.rgb);
    } else {
        base_lin = srgb_to_linear(material.base_color.rgb);
    }
    let light_lin = srgb_to_linear(light.color.rgb);
    var lit_color = base_lin * light_lin * intensity;

    // P2-04 real hemisphere ambient (was 0.2+0.8*a scaling shadow color)
    let hemi = clamp(N.y * 0.5 + 0.5, 0.0, 1.0);
    let sky_lin = srgb_to_linear(light.ambient_sky.rgb);
    let ground_lin = srgb_to_linear(light.ambient_ground.rgb);
    let ambient_hemi = mix(ground_lin, sky_lin, hemi) * clamp(light.color.w, 0.0, 2.0);
    let ambient_term = ambient_hemi;
    // P2-05 AO modulates shadow/ambient, not spec/rim
    var shadow_color = hue_shifted_shadow * ambient_term * ao;

    // Fase 2 (#18): MToon shade map — quando habilitado (params5.y) substitui a
    // sombra computada (VRoid: shadeTex define a cor da primeira banda).
    // shade_toony (params6.z) quantiza a amostra em `toon_steps` bandas, a
    // convenção VRoid de "shade toony".
    if (material.params5.y > 0.5) {
        var shade_sample = textureSample(shade_tex, shade_sampler, in.uv).rgb;
        if (material.params6.z > 0.5) {
            let shade_steps = max(toon_steps, 1.0);
            shade_sample = floor(shade_sample * shade_steps + 0.5) / shade_steps;
        }
        shadow_color = srgb_to_linear(shade_sample) * ambient_term * ao;
    }

    let lit_color_ao = lit_color; // direct light not occluded (only shadow)
    var base_cel = mix(shadow_color, lit_color_ao, toon_factor);

    // Fase 2 (#18): segunda banda de sombra (MToon second shade) — faixa profunda
    // abaixo de (threshold - second_shade_shift). A textura do slot é multiplicada
    // pela sombra base escurecida (0.45) — com o neutro branco ancorado o resultado
    // é exatamente shade_lin × 0.45, a convenção de segunda sombra do VRoid.
    if (material.params5.z > 0.5) {
        let second_shift = max(material.params4.y, 0.0);
        let second_threshold = max(threshold - second_shift, 0.001);
        let second_soft = max(material.params4.z, 0.001);
        let second_blend = smoothstep(second_threshold - second_soft, second_threshold + second_soft, half_lambert);
        let deep_color = srgb_to_linear(textureSample(second_shade_tex, second_shade_sampler, in.uv).rgb) * shade_lin * 0.45;
        let deep_band = deep_color * ambient_term * ao;
        base_cel = mix(deep_band, base_cel, second_blend);
    }

    // Fase 2 (#17): sombra facial SDF — azimut da luz projetado no espaço
    // local da cabeça (eixo Z local = frente do rosto). A região do SDF
    // (nasal/olhos/queixo) escurece 28% — sem textura real ancorada o fator
    // fica 0 e o resultado é idêntico ao frame congelado.
    if (material.params7.z > 0.5) {
        let face_sdf = textureSample(face_sdf_tex, face_sdf_sampler, in.uv).r;
        let face_theta = face_sdf_theta(camera.model[0].xyz, camera.model[2].xyz, L);
        let face_thr = face_sdf_threshold(face_theta, material.params7.x);
        let face_factor = face_sdf_factor(face_sdf, face_thr, material.params7.y);
        base_cel = mix(base_cel, base_cel * 0.72, face_factor);
    }

    // 7. Anisotropic Specular with Stylized Anime Jitter ("Angel Ring")
    let H = normalize(L + V);
    let n_dot_h = max(dot(N, H), 0.0);
    let spec_power = max(material.params.w, 1.0);
    let spec_intensity = material.params.z;
    let spec_softness = material.params3.x;
    let spec_offset = material.params3.y;
    let spec_size = clamp(material.params3.z, 0.20, 0.80); // P2-07 separate spec_size (was 0.65-0.12*intensity coupled)
    let spec_rgb = srgb_to_linear(material.specular_color.rgb);

    let up_vec = vec3<f32>(0.0, 1.0, 0.0);
    let tangent = normalize(cross(N, select(up_vec, vec3<f32>(1.0, 0.0, 0.0), abs(N.y) > 0.99)));
    let t_dot_h = dot(tangent, H);
    let aniso_factor = sqrt(max(1.0 - t_dot_h * t_dot_h, 0.0));

    // P2-06 jitter now seeded by anisotropic mask (was magic constants y*35+uv.x*20)
    let jitter_pos = in.world_position.y * 32.0 + in.uv.x * 18.0 + spec_offset * 9.0;
    let jitter = sin(jitter_pos) * clamp(spec_softness * 0.35, 0.0, 0.08);
    let spec_base = max(mix(n_dot_h, aniso_factor * n_dot_h, 0.35), 0.0);
    let spec_term = pow(spec_base, spec_power);
    let spec_cutoff = spec_size; // P2-07 decoupled (was 0.65-0.12*intensity)
    
    let spec_soft_clamped = max(spec_softness, 0.001);
    let spec_step = smoothstep(spec_cutoff + jitter - spec_soft_clamped, spec_cutoff + jitter + spec_soft_clamped, spec_term) * spec_intensity * in.anime_attr.a * toon_factor;

    // 8. Stylized Fresnel Rim Lighting — P0-09: use material.rim_color
    let rim_intensity = material.params2.x;
    let rim_spread = clamp(material.params2.y, 0.05, 0.95);
    let rim_dot = 1.0 - max(dot(V, N), 0.0);
    let rim_fresnel = smoothstep(1.0 - rim_spread, 1.0, rim_dot);
    let rim_backlight = max(dot(L, -V) * 0.6 + 0.4, 0.0);
    let rim_term = rim_fresnel * rim_backlight * rim_intensity * in.anime_attr.a;

    // 9. Final Color Composition — P2-05 AO already in base_cel, not here
    let lit_highlighted = mix(base_cel, spec_rgb, clamp(spec_step, 0.0, 1.0));
    let rim_rgb = srgb_to_linear(material.rim_color.rgb);
    var with_rim = lit_highlighted + (rim_rgb * rim_term);

    // Fase 2 (#18): matcap (MToon sphereAdd) — normal na base da câmera derivada
    // de V e up (o uniform só carrega view_proj, então reconstruímos a base):
    // uv = N_view.xy × 0.5 + 0.5. Modo normal multiplica, additive soma.
    if (material.params6.x > 0.5) {
        var cam_right = cross(vec3<f32>(0.0, 1.0, 0.0), V);
        let cam_right_len = length(cam_right);
        cam_right = cam_right_len > 1e-4 ? normalize(cam_right) : vec3<f32>(1.0, 0.0, 0.0);
        let cam_up = cross(V, cam_right);
        let n_view = vec3<f32>(dot(N, cam_right), dot(N, cam_up), dot(N, V));
        let matcap_uv = clamp(n_view.xy * 0.5 + 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
        let matcap = srgb_to_linear(textureSample(sphere_add_tex, sphere_add_sampler, matcap_uv).rgb) * material.params4.w;
        with_rim = mix(with_rim * matcap, with_rim + matcap, material.params6.y);
    }

    // Fase 2 (#18): sub-emission (MToon) — desacoplada da iluminação, somada em
    // linear antes do tonemap (brilho persistente mesmo em sombra).
    if (material.params5.w > 0.5) {
        let emission_map = textureSample(emission_tex, emission_sampler, in.uv).rgb;
        with_rim = with_rim + srgb_to_linear(material.emission_color.rgb) * emission_map * material.params4.x;
    }

    // Fase 2 (#43): highlights do olho anime — camada desenhada à mão somada
    // DEPOIS de toda a iluminação (nunca multiplicada por luz/sombra): o
    // branco dos olhos permanece radiante mesmo em penumbra total.
    if (material.params8.z > 0.5) {
        let eye_highlight = eye_highlight_mask(eye_uv) * material.params8.y;
        with_rim = with_rim + eye_highlight_rgb(eye_highlight, 1.0);
    }

    // P1-01: linear→sRGB for display
    let exposure = exp2(light.ambient_sky.w); // P3-01 exposure EV stored in sky.w (fallback 0)
    let final_linear = clamp(with_rim * exposure, vec3<f32>(0.0), vec3<f32>(10.0));
    let tm = tonemap_aces(final_linear); // or reinhard
    let final_srgb = linear_to_srgb(tm);
    return vec4<f32>(final_srgb, material.base_color.a);
}
