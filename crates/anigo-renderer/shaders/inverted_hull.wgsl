// ANIGO Stylized Anime Inverted Hull Outline Shader (WGSL)
// Extrudes vertices along normals with camera distance compensation

struct CameraUniform {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
};

struct OutlineUniform {
    color: vec4<f32>,
    params: vec4<f32>, // x: line_width, y: distance_scaling, z: unused, w: unused
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(0) @binding(1)
var<uniform> outline: OutlineUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>, // B channel modulates outline thickness
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let world_pos = vec4<f32>(in.position, 1.0);
    var clip_pos = camera.view_proj * world_pos;

    // Extrusion along normal in clip space, scaled by distance (clip_pos.w) to maintain constant screen thickness
    let normal_vec4 = camera.view_proj * vec4<f32>(in.normal, 0.0);
    let normal_clip = normalize(normal_vec4.xy);

    let thickness = outline.params.x * in.color.b;
    clip_pos.x += normal_clip.x * thickness * clip_pos.w;
    clip_pos.y += normal_clip.y * thickness * clip_pos.w;

    out.clip_position = clip_pos;
    return out;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return outline.color;
}
