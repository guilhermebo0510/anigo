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
};

struct LightUniform {
    direction: vec4<f32>,       // xyz: normalized light dir, w: intensity
    color: vec4<f32>,           // rgb: light color, w: ambient_intensity
    shadow_color: vec4<f32>,    // rgb: cool/warm hue-shifted shadow tint
};

struct MaterialUniform {
    base_color: vec4<f32>,
    shade_color: vec4<f32>,
    specular_color: vec4<f32>,
    params: vec4<f32>,          // x: shadow_threshold, y: shadow_smoothness, z: spec_intensity, w: spec_power
    params2: vec4<f32>,         // x: rim_intensity, y: rim_spread, z: hue_shift_rad, w: toon_steps
    params3: vec4<f32>,         // x: spec_softness, y: spec_offset, z: unused, w: unused
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
    let world_pos = vec4<f32>(in.position, 1.0);
    out.clip_position = camera.view_proj * world_pos;
    out.world_position = in.position;
    out.world_normal = normalize(in.normal);
    out.uv = in.uv;
    out.anime_attr = in.color;
    return out;
}

// ─────────────────────────────────────────────────────────────
// Color Space Conversions & Mathematical Hue Shifting
// ─────────────────────────────────────────────────────────────

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

fn apply_hue_shift(rgb: vec3<f32>, shift_radians: f32, sat_mult: f32) -> vec3<f32> {
    var hsv = rgb_to_hsv(rgb);
    hsv.x = fract(fract(hsv.x + shift_radians / 6.28318530718) + 1.0);
    hsv.y = clamp(hsv.y * sat_mult, 0.0, 1.0);
    return hsv_to_rgb(hsv);
}

// ─────────────────────────────────────────────────────────────
// Fragment Shader
// ─────────────────────────────────────────────────────────────

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let N = normalize(in.world_normal);
    let L = normalize(light.direction.xyz);
    let V = normalize(camera.camera_pos.xyz - in.world_position);

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

    // Blend texture ramp and analytical calculation based on smoothness:
    // Preserves exact baseline when smoothness <= 0.015, and dynamically softens when smoothness increases
    let toon_factor = mix(ramp_sample.r, analytical_toon, clamp((smoothness - 0.015) * 15.0, 0.0, 1.0));

    // 4. Stylized Ambient Occlusion (R channel)
    let ao = in.anime_attr.r;

    // 5. Mathematical Hue-Shifting in Shadows & Saturation
    let hue_shift_rad = material.params2.z;
    let raw_shadow_color = material.shade_color.rgb * light.shadow_color.rgb;
    let shadow_sat = max(light.shadow_color.w, 0.0);
    let hue_shifted_shadow = apply_hue_shift(raw_shadow_color, hue_shift_rad, shadow_sat);

    // 6. Base Lit and Shadow Blending (Chromaticity-Preserving)
    let intensity = light.direction.w;
    var lit_color = material.base_color.rgb * light.color.rgb * intensity;
    let max_lit = max(max(lit_color.r, lit_color.g), lit_color.b);
    if (max_lit > 1.0) {
        lit_color = lit_color / max_lit;
    }

    let ambient_term = clamp(0.2 + light.color.w * 0.8, 0.05, 1.5);
    let shadow_color = hue_shifted_shadow * ambient_term;
    let base_cel = mix(shadow_color, lit_color, toon_factor);

    // 7. Anisotropic Specular with Stylized Anime Jitter ("Angel Ring")
    let H = normalize(L + V);
    let n_dot_h = max(dot(N, H), 0.0);
    let spec_power = max(material.params.w, 1.0);
    let spec_intensity = material.params.z;
    let spec_softness = material.params3.x;
    let spec_offset = material.params3.y;
    let spec_rgb = material.specular_color.rgb;

    let up_vec = vec3<f32>(0.0, 1.0, 0.0);
    let tangent = normalize(cross(N, select(up_vec, vec3<f32>(1.0, 0.0, 0.0), abs(N.y) > 0.99)));
    let t_dot_h = dot(tangent, H);
    let aniso_factor = sqrt(max(1.0 - t_dot_h * t_dot_h, 0.0));

    let jitter_pos = in.world_position.y * 35.0 + in.uv.x * 20.0 + spec_offset * 10.0;
    let jitter = sin(jitter_pos) * 0.08;
    let spec_base = max(mix(n_dot_h, aniso_factor * n_dot_h, 0.35), 0.0);
    let spec_term = pow(spec_base, spec_power);
    let spec_cutoff = clamp(0.65 - (spec_intensity * 0.12), 0.30, 0.65);
    
    let spec_soft_clamped = max(spec_softness, 0.001);
    let spec_step = smoothstep(spec_cutoff + jitter - spec_soft_clamped, spec_cutoff + jitter + spec_soft_clamped, spec_term) * spec_intensity * in.anime_attr.a * toon_factor;

    // 8. Stylized Fresnel Rim Lighting
    let rim_intensity = material.params2.x;
    let rim_spread = clamp(material.params2.y, 0.05, 0.95);
    let rim_dot = 1.0 - max(dot(V, N), 0.0);
    let rim_fresnel = smoothstep(1.0 - rim_spread, 1.0, rim_dot);
    let rim_backlight = max(dot(L, -V) * 0.6 + 0.4, 0.0);
    let rim_term = rim_fresnel * rim_backlight * rim_intensity * in.anime_attr.a;

    // 9. Final Color Composition
    let lit_highlighted = mix(base_cel, spec_rgb, clamp(spec_step, 0.0, 1.0));
    let with_rim = lit_highlighted + (light.shadow_color.rgb * rim_term);
    let final_rgb = clamp(with_rim, vec3<f32>(0.0), vec3<f32>(1.0)) * ao;

    return vec4<f32>(final_rgb, material.base_color.a);
}
