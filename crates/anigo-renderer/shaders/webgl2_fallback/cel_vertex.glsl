#version 300 es
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;
layout(location = 3) in vec4 a_color;
layout(location = 4) in uvec4 a_joints;
layout(location = 5) in vec4 a_weights;

uniform mat4 u_view_proj;

uniform mat4 u_bones[24];

mat4 anigo_skin_palette(uvec4 joints, vec4 weights) {
  float total = weights.x + weights.y + weights.z + weights.w;
  if (total < 1e-5) {
    return mat4(1.0);
  }
  mat4 blend = mat4(0.0);
  for (int slot = 0; slot < 4; ++slot) {
    float weight = weights[slot];
    if (weight <= 1e-6) {
      continue;
    }
    int bone = min(int(joints[slot]), 23);
    blend += u_bones[bone] * weight;
  }
  return blend / total;
}

out vec3 v_normal;
out vec3 v_pos;
out vec4 v_color;
out vec2 v_uv;

void main() {
  // P1-04: mesma paleta e mesma conta do WGSL (LBS + pesos normalizados).
  mat4 skin = anigo_skin_palette(a_joints, a_weights);
  v_pos = (skin * vec4(a_pos, 1.0)).xyz;
  v_normal = (skin * vec4(a_normal, 0.0)).xyz;
  v_color = a_color;
  v_uv = a_uv;
  gl_Position = u_view_proj * vec4(v_pos, 1.0);
}
