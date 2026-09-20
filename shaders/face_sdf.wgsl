
// P3-03 Face SDF shadow map (Genshin style) — samples baked SDF texture with light angle
// Requires: face_sdf_tex: texture_2d<f32>, face_sdf_sampler, uniform face_light_angle
struct FaceSdfUniform {
    light_angle: f32, // radians, 0 = front
    sdf_threshold: f32,
    sdf_softness: f32,
    _pad: f32,
};
@group(0) @binding(0) var face_sdf_tex: texture_2d<f32>;
@group(0) @binding(1) var face_sdf_sampler: sampler;
@group(0) @binding(2) var<uniform> face_sdf: FaceSdfUniform;

fn sample_face_shadow(uv: vec2<f32>, n_dot_l: f32) -> f32 {
    let sdf = textureSample(face_sdf_tex, face_sdf_sampler, uv).r;
    let angle_factor = cos(face_sdf.light_angle);
    let threshold = face_sdf.sdf_threshold + angle_factor * 0.1;
    return smoothstep(threshold - face_sdf.sdf_softness, threshold + face_sdf.sdf_softness, sdf);
}
