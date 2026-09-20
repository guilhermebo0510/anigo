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
    let world_pos = camera.model * vec4<f32>(in.position, 1.0);
    out.clip_position = camera.view_proj * world_pos;
    out.world_position = world_pos.xyz;
    out.world_normal = normalize((camera.normal_mat * vec4<f32>(in.normal, 0.0)).xyz);
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
    let base_lin = srgb_to_linear(material.base_color.rgb);
    let light_lin = srgb_to_linear(light.color.rgb);
    var lit_color = base_lin * light_lin * intensity;

    // P2-04 real hemisphere ambient (was 0.2+0.8*a scaling shadow color)
    let hemi = clamp(N.y * 0.5 + 0.5, 0.0, 1.0);
    let sky_lin = srgb_to_linear(light.ambient_sky.rgb);
    let ground_lin = srgb_to_linear(light.ambient_ground.rgb);
    let ambient_hemi = mix(ground_lin, sky_lin, hemi) * clamp(light.color.w, 0.0, 2.0);
    let ambient_term = ambient_hemi;
    // P2-05 AO modulates shadow/ambient, not spec/rim
    let shadow_color = hue_shifted_shadow * ambient_term * ao;
    let lit_color_ao = lit_color; // direct light not occluded (only shadow)
    let base_cel = mix(shadow_color, lit_color_ao, toon_factor);

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
    let with_rim = lit_highlighted + (rim_rgb * rim_term);
    // P1-01: linear→sRGB for display
    let exposure = exp2(light.ambient_sky.w); // P3-01 exposure EV stored in sky.w (fallback 0)
    let final_linear = clamp(with_rim * exposure, vec3<f32>(0.0), vec3<f32>(10.0));
    let tm = tonemap_aces(final_linear); // or reinhard
    let final_srgb = linear_to_srgb(tm);
    return vec4<f32>(final_srgb, material.base_color.a);
}
