#version 300 es
precision highp float;
in vec3 v_normal;
in vec3 v_pos;
in vec4 v_color;
in vec2 v_uv;

uniform vec3 u_light_dir;
uniform float u_light_intensity;
uniform vec3 u_light_color;
uniform vec3 u_shadow_color;
uniform float u_ambient_intensity;
uniform float u_shadow_saturation;
uniform vec4 u_base_color;
uniform vec4 u_shade_color;
uniform float u_shadow_threshold;
uniform float u_shadow_smoothness;
uniform float u_hue_shift;
uniform float u_toon_steps;
uniform vec3 u_camera_pos;
uniform float u_spec_intensity;
uniform float u_spec_size;
uniform float u_ao_intensity;
uniform float u_spec_power;
uniform float u_spec_softness;
uniform float u_spec_offset;
uniform vec4 u_spec_color;
uniform float u_rim_intensity;
uniform float u_rim_spread;
uniform vec3 u_rim_color;

out vec4 fragColor;

vec3 rgb2hsv(vec3 c) {
  vec4 K = vec4(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
  vec4 p = mix(vec4(c.bg, K.wz), vec4(c.gb, K.xy), step(c.b, c.g));
  vec4 q = mix(vec4(p.xyw, c.r), vec4(c.r, p.yzx), step(p.x, c.r));
  float d = q.x - min(q.w, q.y);
  float e = 1.0e-10;
  return vec3(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

vec3 hsv2rgb(vec3 c) {
  vec4 K = vec4(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
  vec3 p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
  return c.z * mix(K.xxx, clamp(p - K.xxx, 0.0, 1.0), c.y);
}
// P1-01 sRGB ↔ linear
vec3 srgbToLinear(vec3 c) {
  bvec3 cutoff = lessThanEqual(c, vec3(0.04045));
  vec3 lo = c / 12.92;
  vec3 hi = pow((c + vec3(0.055)) / 1.055, vec3(2.4));
  return mix(hi, lo, vec3(cutoff));
}
vec3 linearToSrgb(vec3 c) {
  bvec3 cutoff = lessThanEqual(c, vec3(0.0031308));
  vec3 lo = c * 12.92;
  vec3 hi = 1.055 * pow(c, vec3(1.0/2.4)) - 0.055;
  return mix(hi, lo, vec3(cutoff));
}
// P1-02 OKLab hue rotation (fallback to linear * saturation for low chroma)
vec3 linearToOklab(vec3 c) {
  float l = 0.4122214708*c.r + 0.5363325363*c.g + 0.0514459929*c.b;
  float m = 0.2119034982*c.r + 0.6806995451*c.g + 0.1073969566*c.b;
  float s = 0.0883024619*c.r + 0.2817188376*c.g + 0.6299787005*c.b;
  float l_ = pow(max(l,0.0), 1.0/3.0);
  float m_ = pow(max(m,0.0), 1.0/3.0);
  float s_ = pow(max(s,0.0), 1.0/3.0);
  return vec3(
    0.2104542553*l_ + 0.7936177850*m_ - 0.0040720468*s_,
    1.9779984951*l_ - 2.4285922050*m_ + 0.4505937099*s_,
    0.0259040371*l_ + 0.7827717662*m_ - 0.8086757660*s_
  );
}
vec3 oklabToLinear(vec3 c) {
  float l_ = c.x + 0.3963377774*c.y + 0.2158037573*c.z;
  float m_ = c.x - 0.1055613458*c.y - 0.0638541728*c.z;
  float s_ = c.x - 0.0894841775*c.y - 1.2914855480*c.z;
  float l = l_*l_*l_;
  float m = m_*m_*m_;
  float s = s_*s_*s_;
  return vec3(
    4.0767416621*l - 3.3077115913*m + 0.2309699292*s,
    -1.2684380046*l + 2.6097574011*m - 0.3413193965*s,
    -0.0041960863*l - 0.7034186147*m + 1.7076147010*s
  );
}

void main() {
  vec3 N = normalize(v_normal);
  vec3 L = normalize(u_light_dir);
  vec3 V = normalize(u_camera_pos - v_pos);
  float n_dot_l = dot(N, L);
  float half_lambert = n_dot_l * 0.5 + 0.5;
  float shift = (v_color.g - 0.5) * 0.3;
  float threshold = u_shadow_threshold + shift;
  float smoothness = max(u_shadow_smoothness, 0.001);

  float u_coord = clamp((half_lambert - threshold) + 0.5, 0.0, 1.0);
  float toon = u_coord;
  if (u_toon_steps < 0.5) {
    toon = smoothstep(threshold - 0.35 - smoothness, threshold + 0.35 + smoothness, half_lambert);
  } else if (u_toon_steps >= 0.5 && u_toon_steps < 1.5) {
    toon = smoothstep(threshold - smoothness, threshold + smoothness, half_lambert);
  } else if (u_toon_steps >= 1.5 && u_toon_steps < 2.5) {
    float s1 = smoothstep(threshold - 0.14 - smoothness, threshold - 0.14 + smoothness, half_lambert);
    float s2 = smoothstep(threshold + 0.14 - smoothness, threshold + 0.14 + smoothness, half_lambert);
    toon = s1 * 0.45 + s2 * 0.55;
  } else {
    float s1 = smoothstep(threshold - 0.20 - smoothness, threshold - 0.20 + smoothness, half_lambert);
    float s2 = smoothstep(threshold - smoothness, threshold + smoothness, half_lambert);
    float s3 = smoothstep(threshold + 0.20 - smoothness, threshold + 0.20 + smoothness, half_lambert);
    toon = (s1 + s2 + s3) / 3.0;
  }

  // P0-03 + P1-01 linear: base/light in linear
  vec3 baseLin = srgbToLinear(u_base_color.rgb);
  vec3 lightLin = srgbToLinear(u_light_color);
  vec3 lit = baseLin * lightLin * clamp(u_light_intensity, 0.0, 3.0);

  // P1-01 linear + P1-02 OKLab hue (clamp ±180)
  float hueShiftRad = radians(clamp(u_hue_shift, -180.0, 180.0));
  vec3 shadeLin = srgbToLinear(u_shade_color.rgb);
  vec3 shadowTintLin = srgbToLinear(u_shadow_color);
  vec3 raw_shadow_lin = shadeLin * shadowTintLin;
  // OKLab hue rotation
  vec3 lab = linearToOklab(raw_shadow_lin);
  float C = length(lab.yz);
  vec3 hueShiftedLin;
  if (C < 0.0001) {
    hueShiftedLin = raw_shadow_lin * mix(1.0, clamp(u_shadow_saturation,0.0,2.0), 0.5);
  } else {
    float hue = atan(lab.z, lab.y);
    float newHue = hue + hueShiftRad;
    float C2 = clamp(C * clamp(u_shadow_saturation,0.0,2.0), 0.0, 0.4);
    lab.y = C2 * cos(newHue);
    lab.z = C2 * sin(newHue);
    hueShiftedLin = oklabToLinear(lab);
  }
  float hemi = clamp(N.y * 0.5 + 0.5, 0.0, 1.0);
  vec3 sky = vec3(0.52, 0.60, 0.78);
  vec3 ground = vec3(0.25, 0.20, 0.18);
  vec3 ambient = srgbToLinear(mix(ground, sky, hemi)) * clamp(u_ambient_intensity, 0.0, 2.0);
  float ao = mix(1.0, clamp(v_color.r, 0.0, 1.0), clamp(u_ao_intensity, 0.0, 1.0));
  vec3 shadow = hueShiftedLin * ambient * ao;

  vec3 base_cel = mix(shadow, lit, toon);

  vec3 H = normalize(L + V);
  float n_dot_h = max(dot(N, H), 0.0);
  vec3 up_vec = vec3(0.0, 1.0, 0.0);
  vec3 tangent = normalize(cross(N, mix(up_vec, vec3(1.0, 0.0, 0.0), step(0.99, abs(N.y)))));
  float t_dot_h = dot(tangent, H);
  float aniso = sqrt(max(1.0 - t_dot_h * t_dot_h, 0.0));
  // P0-04: jitter unified to world_pos.y*35 + uv.x*20 (was v_pos.x diverging)
  float jitter_pos = v_pos.y * 35.0 + v_uv.x * 20.0 + u_spec_offset * 10.0;
  float jitter = sin(jitter_pos) * 0.08;
  float spec_base = max(mix(n_dot_h, aniso * n_dot_h, 0.35), 0.0);
  float spec_term = pow(spec_base, max(u_spec_power, 1.0));
  float spec_cutoff = clamp(u_spec_size, 0.20, 0.80);
  float spec_soft_clamped = max(u_spec_softness, 0.001);
  float spec_step = smoothstep(spec_cutoff + jitter - spec_soft_clamped, spec_cutoff + jitter + spec_soft_clamped, spec_term) * u_spec_intensity * v_color.a * toon;

  float rim_dot = 1.0 - max(dot(V, N), 0.0);
  float rim_fresnel = smoothstep(1.0 - u_rim_spread, 1.0, rim_dot);
  float rim_backlight = max(dot(L, -V) * 0.6 + 0.4, 0.0);
  float rim_term = rim_fresnel * rim_backlight * u_rim_intensity * v_color.a;

  vec3 specLin = srgbToLinear(u_spec_color.rgb);
  vec3 rimLin = srgbToLinear(u_rim_color);
  vec3 lit_highlighted = mix(base_cel, specLin, clamp(spec_step, 0.0, 1.0));
  vec3 with_rim = lit_highlighted + (rimLin * rim_term);
  // P1-01 linear -> srgb for display (pipeline Rgba8Unorm non-sRGB)
  vec3 colLin = clamp(with_rim, 0.0, 1.0) * clamp(v_color.r, 0.0, 1.0);
  vec3 col = linearToSrgb(colLin);
  fragColor = vec4(col, u_base_color.a);
}
