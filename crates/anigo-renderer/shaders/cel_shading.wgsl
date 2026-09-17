// ANIGO Stylized Anime Cel-Shading NPR Shader (WGSL)
// Supports Toon Ramp, Hue-Shifting Shadow Tint, and Anime Vertex Attributes

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
    params: vec4<f32>,          // x: shadow_threshold, y: shadow_smoothness, z: unused, w: unused
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(0) @binding(1)
var<uniform> light: LightUniform;

@group(0) @binding(2)
var<uniform> material: MaterialUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>, // r: AO, g: shadow shift, b: outline, a: specular mask
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

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let N = normalize(in.world_normal);
    let L = normalize(light.direction.xyz);
    let V = normalize(camera.camera_pos.xyz - in.world_position);

    // 1. Calculate base N dot L with half-lambert remapping
    let n_dot_l = dot(N, L);
    let half_lambert = n_dot_l * 0.5 + 0.5;

    // 2. Modulate shadow threshold using vertex attribute (G channel = shadow shift)
    let shadow_shift = (in.anime_attr.g - 0.5) * 0.3;
    let threshold = material.params.x + shadow_shift;
    let smoothness = max(material.params.y, 0.001);

    // 3. Toon step calculation with smooth transition to prevent pixel aliasing
    let toon_factor = smoothstep(threshold - smoothness, threshold + smoothness, half_lambert);

    // 4. Stylized Ambient Occlusion from vertex color (R channel)
    let ao = in.anime_attr.r;

    // 5. Blending Lit vs Shadow colors with Hue-Shifted shadow tint
    let lit_color = material.base_color.rgb * light.color.rgb;
    let shadow_tint = material.shade_color.rgb * light.shadow_color.rgb;
    let ambient_light = material.shade_color.rgb * light.color.w * ao;

    let final_rgb = mix(shadow_tint + ambient_light, lit_color, toon_factor) * ao;

    return vec4<f32>(final_rgb, material.base_color.a);
}
