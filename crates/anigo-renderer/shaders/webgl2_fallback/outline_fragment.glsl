#version 300 es
precision highp float;
uniform vec4 u_outline_color;
uniform float u_outline_opacity;
uniform float u_outline_smoothness;
out vec4 fragColor;

void main() {
  float smooth = clamp(u_outline_smoothness, 0.0, 1.0);
  vec4 col = vec4(u_outline_color.rgb, u_outline_color.a * u_outline_opacity);
  if (smooth > 0.001) {
    col.a = col.a * mix(1.0, 0.85, clamp(smooth * 8.0, 0.0, 1.0));
  }
  fragColor = col;
}
