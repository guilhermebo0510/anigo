#version 300 es
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec3 a_normal;
layout(location = 3) in vec4 a_color;
layout(location = 4) in uvec4 a_joints;
layout(location = 5) in vec4 a_weights;

uniform mat4 u_view_proj;
uniform float u_outline_width;
uniform float u_aspect;
uniform float u_outline_depth_bias;

void main() {
  if (a_color.b <= 0.001) {
    gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
    return;
  }
  vec4 clip = u_view_proj * vec4(a_pos, 1.0);
  vec4 norm = u_view_proj * vec4(a_normal, 0.0);
  float len = length(norm.xy);
  vec2 norm_clip = mix(vec2(0.0), norm.xy / len, step(1e-5, len));
  float aspectSafe = max(u_aspect, 0.001);
  clip.x += (norm_clip.x / aspectSafe) * u_outline_width * a_color.b * clip.w;
  clip.y += norm_clip.y * u_outline_width * a_color.b * clip.w;
  clip.z += u_outline_depth_bias * clip.w;
  gl_Position = clip;
}
