#version 300 es
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;
layout(location = 3) in vec4 a_color;
layout(location = 4) in uvec4 a_joints;
layout(location = 5) in vec4 a_weights;

uniform mat4 u_view_proj;
out vec3 v_normal;
out vec3 v_pos;
out vec4 v_color;
out vec2 v_uv;

void main() {
  v_pos = a_pos;
  v_normal = a_normal;
  v_color = a_color;
  v_uv = a_uv;
  gl_Position = u_view_proj * vec4(a_pos, 1.0);
}
