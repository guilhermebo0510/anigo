/**
 * ANIGO render contract v1 (P0 "Consolidar o renderer").
 *
 * The repository must contain *one* definition of every production shader, buffer,
 * uniform, camera convention and pass. This module freezes that definition:
 *
 *  - `shaders`       — the canonical WGSL files (`crates/anigo-renderer/shaders/`,
 *                      loaded by Rust with `include_str!` and by the viewport with
 *                      `?raw`) plus the clearly-labelled WebGL2 fallback GLSL;
 *                      every entry carries the FNV-1a-64 of its bytes, so any
 *                      drift between the two languages fails a test.
 *  - `vertex_layout` — the 72-byte NPR vertex, attribute by attribute.
 *  - `uniforms`      — byte layout of every uniform block (offsets/sizes).
 *  - `bind_groups`   — bindings declared by the WGSL (`@group(0) @binding(n)`),
 *                      which both renderers must implement.
 *  - `passes`        — the pass graph and its pipeline state (order, cull, blend,
 *                      depth, MSAA) shared by headless and viewport.
 *  - `camera`        — projection/clip conventions and the reference frame whose
 *                      uniform bytes both sides must reproduce.
 *  - `toon_ramp`     — the 256x4 ramp rows used by the cel shader.
 *
 * The reference frame numbers were produced by an independent implementation
 * (right-handed look-at + 0..1-depth perspective, column-major) and are frozen:
 * `crates/anigo-renderer/src/render_contract.rs` (glam) and
 * `src/services/camera_math.ts` + `src/services/render_uniforms.ts` must both
 * reproduce them within 1e-5.
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, "..");

function fnv1a64Bytes(bytes) {
  let hash = 0xcbf29ce484222325n;
  for (const byte of bytes) {
    hash ^= BigInt(byte);
    hash = (hash * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return hash.toString(16).padStart(16, "0");
}

function fnv1a64(input) {
  return fnv1a64Bytes(new TextEncoder().encode(input));
}

/**
 * Materialises the toon ramp from the frozen row spec. This is the *reference*
 * implementation: the production generators (`src/services/toon_ramp.ts` and
 * `crates/anigo-renderer/src/render_contract.rs`) must hash their bytes to
 * `toon_ramp.bytes_fnv1a64`.
 */
export function toonRampBytes(spec) {
  const data = new Uint8Array(spec.width * spec.height * 4);
  for (let y = 0; y < spec.height; y++) {
    const row = spec.rows[y];
    for (let x = 0; x < spec.width; x++) {
      const u = x / (spec.width - 1);
      let factor = row.kind === "identity" ? u : row.base ?? 0;
      if (row.kind !== "identity") {
        for (const step of row.steps) {
          if (u >= step.threshold) factor = step.value;
        }
      }
      const value = Math.min(255, Math.max(0, Math.round(factor * 255)));
      const index = (y * spec.width + x) * 4;
      data[index] = value;
      data[index + 1] = value;
      data[index + 2] = value;
      data[index + 3] = 255;
    }
  }
  return data;
}

function shader(name, relativePath, language, role, entryPoints) {
  const raw = fs.readFileSync(path.join(repoRoot, relativePath), "utf8");
  const source = raw.replace(/\r\n/g, "\n");
  return {
    name,
    path: relativePath,
    language,
    role,
    entry_points: entryPoints,
    lines: source.split("\n").length - (source.endsWith("\n") ? 1 : 0),
    fnv1a64: fnv1a64(source),
  };
}

export function renderContractFixture() {
  const contract = {
    $comment:
      "Render contract v1: one canonical WGSL set (crates/anigo-renderer/shaders), " +
      "shared vertex/uniform layouts, bind groups, pass graph, camera conventions and " +
      "toon ramp. Verified by crates/anigo-renderer/src/render_contract.rs and " +
      "tests/contracts/render_contract.test.ts (same shader hashes, same uniform floats).",
    version: 1,
    shaders: [
    shader("cel_shading", "crates/anigo-renderer/shaders/cel_shading.wgsl", "wgsl", "production", { vertex: "vs_main", fragment: "fs_main" }),
    shader("inverted_hull", "crates/anigo-renderer/shaders/inverted_hull.wgsl", "wgsl", "production", { vertex: "vs_main", fragment: "fs_main" }),
    shader("morph_sparse_compute", "crates/anigo-renderer/shaders/morph_sparse_compute.wgsl", "wgsl", "production", { compute: "cs_accumulate_morphs", compute_reset: "cs_reset_vertices" }),
    shader("face_sdf", "crates/anigo-renderer/shaders/face_sdf.wgsl", "wgsl", "library", { sample: "sample_face_shadow" }),
    shader("webgl2_fallback/cel_vertex", "crates/anigo-renderer/shaders/webgl2_fallback/cel_vertex.glsl", "glsl", "fallback_webgl2", { vertex: "main" }),
    shader("webgl2_fallback/cel_fragment", "crates/anigo-renderer/shaders/webgl2_fallback/cel_fragment.glsl", "glsl", "fallback_webgl2", { fragment: "main" }),
    shader("webgl2_fallback/outline_vertex", "crates/anigo-renderer/shaders/webgl2_fallback/outline_vertex.glsl", "glsl", "fallback_webgl2", { vertex: "main" }),
    shader("webgl2_fallback/outline_fragment", "crates/anigo-renderer/shaders/webgl2_fallback/outline_fragment.glsl", "glsl", "fallback_webgl2", { fragment: "main" }),
    ],
    vertex_layout: {
      stride: 72,
      step_mode: "vertex",
      attributes: [
        { shader_location: 0, offset: 0, format: "float32x3", name: "position" },
        { shader_location: 1, offset: 12, format: "float32x3", name: "normal" },
        { shader_location: 2, offset: 24, format: "float32x2", name: "uv" },
        { shader_location: 3, offset: 32, format: "float32x4", name: "anime_attr" },
        { shader_location: 4, offset: 48, format: "uint16x4", name: "joints" },
        { shader_location: 5, offset: 56, format: "float32x4", name: "weights" },
      ],
    },
    uniforms: {
      camera: {
        struct: "CameraUniform",
        address_space: "uniform",
        size: 208,
        fields: [
          { name: "view_proj", kind: "mat4x4", offset: 0, size: 64 },
          { name: "camera_pos", kind: "vec4", offset: 64, size: 16 },
          { name: "model", kind: "mat4x4", offset: 80, size: 64 },
          { name: "normal_mat", kind: "mat4x4", offset: 144, size: 64 },
        ],
      },
      light: {
        struct: "LightUniform",
        address_space: "uniform",
        size: 80,
        fields: [
          { name: "direction", kind: "vec4", offset: 0, size: 16, meaning: "xyz dir, w intensity" },
          { name: "color", kind: "vec4", offset: 16, size: 16, meaning: "rgb color, w ambient_intensity" },
          { name: "shadow_color", kind: "vec4", offset: 32, size: 16, meaning: "rgb tint, w saturation" },
          { name: "ambient_sky", kind: "vec4", offset: 48, size: 16 },
          { name: "ambient_ground", kind: "vec4", offset: 64, size: 16 },
        ],
      },
      material: {
        struct: "MaterialUniform",
        address_space: "uniform",
        size: 176,
        fields: [
          { name: "base_color", kind: "vec4", offset: 0, size: 16 },
          { name: "shade_color", kind: "vec4", offset: 16, size: 16 },
          { name: "specular_color", kind: "vec4", offset: 32, size: 16 },
          { name: "rim_color", kind: "vec4", offset: 48, size: 16 },
          { name: "params", kind: "vec4", offset: 64, size: 16, meaning: "shadow_threshold, shadow_smoothness, spec_intensity, spec_power" },
          { name: "params2", kind: "vec4", offset: 80, size: 16, meaning: "rim_intensity, rim_spread, hue_shift_rad, toon_steps" },
          { name: "params3", kind: "vec4", offset: 96, size: 16, meaning: "specular_softness, specular_offset, specular_size, ao_intensity" },
          // Fase 2 (#18): material anime VRoid/MToon
          { name: "emission_color", kind: "vec4", offset: 112, size: 16, meaning: "MToon subEmission (rgb sRGB, w alpha)" },
          { name: "params4", kind: "vec4", offset: 128, size: 16, meaning: "emission_intensity, second_shade_shift, second_shade_softness, matcap_intensity" },
          { name: "params5", kind: "vec4", offset: 144, size: 16, meaning: "main_tex_enabled, shade_tex_enabled, second_shade_enabled, emission_enabled" },
          { name: "params6", kind: "vec4", offset: 160, size: 16, meaning: "matcap_enabled, matcap_mode (0 normal/1 additive), shade_toony, reserved" },
        ],
      },
      outline: {
        struct: "OutlineUniform",
        address_space: "uniform",
        size: 48,
        fields: [
          { name: "color", kind: "vec4", offset: 0, size: 16 },
          { name: "params", kind: "vec4", offset: 16, size: 16, meaning: "width, aspect, depth_bias, opacity" },
          { name: "params2", kind: "vec4", offset: 32, size: 16, meaning: "smoothness, width_tex_enabled (Fase 2 #18)" },
        ],
      },
      sparse_morph_header: {
        struct: "SparseMorphHeader",
        address_space: "uniform",
        size: 16,
        fields: [
          { name: "active_channel_count", kind: "u32", offset: 0, size: 4 },
          { name: "total_vertex_count", kind: "u32", offset: 4, size: 4 },
          { name: "total_delta_count", kind: "u32", offset: 8, size: 4 },
          { name: "_pad", kind: "u32", offset: 12, size: 4 },
        ],
      },
      morph_channel: {
        struct: "MorphChannel",
        address_space: "storage_read",
        size: 16,
        fields: [
          { name: "weight", kind: "f32", offset: 0, size: 4 },
          { name: "start_offset", kind: "u32", offset: 4, size: 4 },
          { name: "delta_count", kind: "u32", offset: 8, size: 4 },
          { name: "_pad", kind: "u32", offset: 12, size: 4 },
        ],
      },
      sparse_morph_delta: {
        struct: "SparseMorphDelta",
        address_space: "storage_read",
        size: 32,
        fields: [
          { name: "vertex_index", kind: "u32", offset: 0, size: 4 },
          { name: "delta_px", kind: "f32", offset: 4, size: 4 },
          { name: "delta_py", kind: "f32", offset: 8, size: 4 },
          { name: "delta_pz", kind: "f32", offset: 12, size: 4 },
          { name: "delta_nx", kind: "f32", offset: 16, size: 4 },
          { name: "delta_ny", kind: "f32", offset: 20, size: 4 },
          { name: "delta_nz", kind: "f32", offset: 24, size: 4 },
          { name: "_pad", kind: "f32", offset: 28, size: 4 },
        ],
      },
      // P1-04: paleta de skinning (24 ossos × mat4 = 1536 B). Vai como `uniform`
      // porque o WebGPU não permite storage buffer no estágio de vértice — e é
      // no vértice que a pele é aplicada.
      bones: {
        struct: "BonePalette",
        address_space: "uniform",
        size: 1536,
        note: "array<mat4x4<f32>, 24>: stride 64 B por osso, sem padding entre elementos",
        fields: [
          { name: "matrices", kind: "array_mat4_f32_24", offset: 0, size: 1536, meaning: "world × inverse bind por osso, na ordem do esqueleto" },
        ],
      },
      vertex_raw: {
        struct: "VertexRaw",
        address_space: "vertex_and_storage_read",
        size: 72,
        note: "same 72 bytes in the vertex buffer and in array<VertexRaw>; the WGSL struct declares scalar members (no vec3), so align == 4 and the 72-byte array stride stays legal",
        fields: [
          { name: "pos", kind: "vec3_f32", offset: 0, size: 12 },
          { name: "norm", kind: "vec3_f32", offset: 12, size: 12 },
          { name: "uv", kind: "vec2_f32", offset: 24, size: 8 },
          { name: "color", kind: "vec4_f32", offset: 32, size: 16 },
          { name: "joints", kind: "vec4_u32", offset: 48, size: 16 },
          { name: "weights", kind: "vec4_f32", offset: 56, size: 16 },
        ],
      },
    },
    bind_groups: [
      {
        name: "cel",
        group: 0,
        entries: [
          { binding: 0, kind: "uniform", stages: ["vertex", "fragment"], declaration: "var<uniform> camera: CameraUniform" },
          { binding: 1, kind: "uniform", stages: ["fragment"], declaration: "var<uniform> light: LightUniform" },
          { binding: 2, kind: "uniform", stages: ["fragment"], declaration: "var<uniform> material: MaterialUniform" },
          { binding: 3, kind: "texture_2d<f32>", stages: ["fragment"], declaration: "var toon_ramp_tex: texture_2d<f32>" },
          { binding: 4, kind: "sampler", stages: ["fragment"], declaration: "var toon_ramp_sampler: sampler" },
          { binding: 5, kind: "uniform", stages: ["vertex"], declaration: "var<uniform> bones: BonePalette" },
          // Fase 2 (#18): slots de textura do material anime (VRoid/MToon).
          // Sem textura, o slot é desativado via material.params5 e o renderer
          // ancora um neutro 1x1 (branco) nesses bindings.
          { binding: 6, kind: "texture_2d<f32>", stages: ["fragment"], declaration: "var main_tex: texture_2d<f32>" },
          { binding: 7, kind: "sampler", stages: ["fragment"], declaration: "var main_sampler: sampler" },
          { binding: 8, kind: "texture_2d<f32>", stages: ["fragment"], declaration: "var shade_tex: texture_2d<f32>" },
          { binding: 9, kind: "sampler", stages: ["fragment"], declaration: "var shade_sampler: sampler" },
          { binding: 10, kind: "texture_2d<f32>", stages: ["fragment"], declaration: "var second_shade_tex: texture_2d<f32>" },
          { binding: 11, kind: "sampler", stages: ["fragment"], declaration: "var second_shade_sampler: sampler" },
          { binding: 12, kind: "texture_2d<f32>", stages: ["fragment"], declaration: "var emission_tex: texture_2d<f32>" },
          { binding: 13, kind: "sampler", stages: ["fragment"], declaration: "var emission_sampler: sampler" },
          { binding: 14, kind: "texture_2d<f32>", stages: ["fragment"], declaration: "var sphere_add_tex: texture_2d<f32>" },
          { binding: 15, kind: "sampler", stages: ["fragment"], declaration: "var sphere_add_sampler: sampler" },
        ],
      },
      {
        name: "outline",
        group: 0,
        entries: [
          { binding: 0, kind: "uniform", stages: ["vertex"], declaration: "var<uniform> camera: CameraUniform" },
          { binding: 1, kind: "uniform", stages: ["vertex", "fragment"], declaration: "var<uniform> outline: OutlineUniform" },
          { binding: 2, kind: "uniform", stages: ["vertex"], declaration: "var<uniform> bones: BonePalette" },
          // Fase 2 (#18): mapa de espessura do contorno (MToon outlineWidth)
          { binding: 3, kind: "texture_2d<f32>", stages: ["vertex"], declaration: "var outline_width_tex: texture_2d<f32>" },
          { binding: 4, kind: "sampler", stages: ["vertex"], declaration: "var outline_width_sampler: sampler" },
        ],
      },
      {
        name: "sparse_morph",
        group: 0,
        entries: [
          { binding: 0, kind: "uniform", stages: ["compute"], declaration: "var<uniform> header: SparseMorphHeader" },
          { binding: 1, kind: "storage_read", stages: ["compute"], declaration: "var<storage, read> base_vertices: array<VertexRaw>" },
          { binding: 2, kind: "storage_read", stages: ["compute"], declaration: "var<storage, read> morph_deltas: array<SparseMorphDelta>" },
          { binding: 3, kind: "storage_read", stages: ["compute"], declaration: "var<storage, read> active_channels: array<MorphChannel>" },
          { binding: 4, kind: "storage_read_write", stages: ["compute"], declaration: "var<storage, read_write> out_vertices: array<VertexRaw>" },
        ],
      },
    ],
    passes: [
      {
        order: 0,
        name: "outline",
        kind: "render",
        shader: "inverted_hull",
        vertex_entry: "vs_main",
        fragment_entry: "fs_main",
        bind_group: "outline",
        cull_mode: "front",
        depth_write: false,
        depth_compare: "less-equal",
        depth_bias: { constant: 1, slope_scale: 1.0, clamp: 0.0 },
        blend: "src_alpha_one_minus_src_alpha",
        write_mask: "all",
      },
      {
        order: 1,
        name: "cel",
        kind: "render",
        shader: "cel_shading",
        vertex_entry: "vs_main",
        fragment_entry: "fs_main",
        bind_group: "cel",
        cull_mode: "back",
        depth_write: true,
        depth_compare: "less-equal",
        depth_bias: { constant: 0, slope_scale: 0.0, clamp: 0.0 },
        blend: "none",
        write_mask: "all",
      },
      {
        order: -1,
        name: "sparse_morph",
        kind: "compute",
        shader: "morph_sparse_compute",
        compute_entry: "cs_accumulate_morphs",
        bind_group: "sparse_morph",
        workgroup_size: 64,
        only_when: "gpu_morph_active",
      },
    ],
    // ---------------------------------------------------------------------
    // P1-02: códigos estáveis de diagnóstico. Rust e TypeScript *precisam*
    // concordar: a UI, a telemetria e os testes falam a mesma língua, e um
    // código novo só existe depois de declarado aqui.
    // ---------------------------------------------------------------------
    diagnostics: {
      note:
        "Códigos estáveis compartilhados pelos dois lados. Severidade define se o " +
        "renderer está degradado (error) ou apenas avisado (warning).",
      codes: [
        { code: "webgpu_unavailable", severity: "warning" },
        { code: "backend_unavailable", severity: "error" },
        { code: "gpu_device_error", severity: "error" },
        { code: "pipeline_init_failed", severity: "error" },
        { code: "compute_init_failed", severity: "error" },
        { code: "channel_upload_failed", severity: "error" },
        { code: "context_configure_failed", severity: "warning" },
        { code: "frame_skipped", severity: "warning" },
        { code: "snapshot_invalid", severity: "error" },
        { code: "snapshot_stale", severity: "warning" },
        { code: "geometry_unavailable", severity: "warning" },
        { code: "mesh_invalid", severity: "error" },
        { code: "model_load_failed", severity: "error" },
        { code: "geometry_rejected", severity: "error" },
        { code: "channel_missing", severity: "warning" },
        { code: "unexpected_error", severity: "error" },
        { code: "contract_drift", severity: "error" },
        { code: "device_unavailable", severity: "error" },
        { code: "shader_compile_failed", severity: "error" },
        { code: "buffer_creation_failed", severity: "error" },
        { code: "readback_failed", severity: "error" },
      ],
    },
    // ---------------------------------------------------------------------
    // P1-03: checagens obrigatórias de malha **antes** de criar buffers de GPU.
    // GLB e geometria canônica passam pelas mesmas regras nos dois lados.
    // ---------------------------------------------------------------------
    mesh_validation: {
      note: "Códigos compartilhados: uma malha reprovada aqui não vira buffer de GPU.",
      stride_bytes: 72,
      codes: [
        { code: "EMPTY_MESH", meaning: "sem vértices ou sem índices" },
        { code: "INDEX_COUNT_MISMATCH", meaning: "índices não formam triângulos (múltiplo de 3)" },
        { code: "INDEX_OUT_OF_RANGE", meaning: "índice aponta para fora da lista de vértices" },
        { code: "NON_FINITE_VALUE", meaning: "posição/normal/uv/peso com NaN ou infinito" },
        { code: "BAD_VERTEX_STRIDE", meaning: "tamanho do buffer não é múltiplo do stride do vértice" },
      ],
    },
    // ---------------------------------------------------------------------
    // P1-04: skinning de verdade (LBS). O núcleo entrega a paleta; o shader só
    // aplica. O bloco de código de skinning é **idêntico** em cel_shading.wgsl e
    // inverted_hull.wgsl (delimitado por marcadores): uma definição, dois usos.
    // ---------------------------------------------------------------------
    skinning: {
      algorithm: "linear_blend_skinning",
      joint_count: 24,
      max_influences: 4,
      matrices_per_joint: 16,
      palette_uniform: "bones",
      palette_bytes: 1536,
      joints_location: 4,
      weights_location: 5,
      weights_normalization: "soma dos pesos normalizada no shader antes de combinar as matrizes",
      unskinned_fallback: "peso total < 1e-5 devolve a matriz identidade (vértice sem influência não colapsa na origem)",
      index_clamp: "índice de osso limitado a joint_count-1 antes de indexar a paleta",
      block_markers: ["// ANIGO-SKINNING-BEGIN", "// ANIGO-SKINNING-END"],
    },
    targets: {
      offscreen_color_format: "rgba8unorm",
      viewport_color_format_policy: "surface_preferred",
      depth_format: "depth24plus",
      msaa_samples: 4,
      color_space: "linear_working_no_srgb_view",
      resolve_to_swapchain: true,
      clear: { source: "scene.background_color" },
    },
    camera: {
      projection: "perspective_rh",
      clip_depth: "zero_to_one",
      matrix_layout: "column_major",
      up_axis: "y",
      fov_y_degrees_default: 45.0,
      z_near_default: 0.05,
      z_far_default: 100.0,
      model_from: "scene.nodes[0].transform",
      uniform: "camera",
    },
    toon_ramp: {
      width: 256,
      height: 4,
      format: "rgba8unorm",
      mag_filter: "linear",
      min_filter: "linear",
      address_mode: "clamp_to_edge",
      rows: [
        { name: "continuous", kind: "identity" },
        { name: "one_step", kind: "steps", steps: [{ threshold: 0.5, value: 1.0 }], base: 0.0 },
        { name: "two_step", kind: "steps", steps: [{ threshold: 0.35, value: 0.5 }, { threshold: 0.65, value: 1.0 }], base: 0.0 },
        { name: "three_step", kind: "steps", steps: [{ threshold: 0.25, value: 0.35 }, { threshold: 0.5, value: 0.7 }, { threshold: 0.75, value: 1.0 }], base: 0.0 },
      ],
    },
    reference_frame: {
      description:
        "Frozen frame: the uniform blocks both renderers must produce byte for byte " +
        "(numbers from an independent implementation, tolerance 1e-5).",
      camera: {
        eye: [0.4, 1.7, 3.2],
        target: [0.0, 1.05, 0.0],
        up: [0.0, 1.0, 0.0],
        fov_y_degrees: 50.0,
        fov_y_radians: 0.87266463,
        aspect: 1.777777791,
        z_near: 0.05,
        z_far: 100.0,
      },
      model: {
        translation: [0.25, 0.5, -0.75],
        scale: [1.2, 0.9, 1.1],
        rotation_xyzw: [0.0, 0.258819, 0.0, 0.965926],
      },
      light: {
        direction: [0.36, 0.8, 0.48],
        intensity: 1.35,
        color: [1.0, 0.96, 0.9],
        ambient_intensity: 0.42,
        shadow_color: [0.95, 0.97, 1.0],
        shadow_saturation: 0.85,
        ambient_sky: [0.52, 0.6, 0.78],
        ambient_ground: [0.25, 0.2, 0.18],
      },
      material: {
        base_color: [0.98, 0.92, 0.85, 1.0],
        shade_color: [0.82, 0.73, 0.78, 1.0],
        specular_color: [1.0, 1.0, 1.0, 1.0],
        rim_color: [0.576, 0.773, 0.992, 1.0],
        shadow_threshold: 0.58,
        shadow_smoothness: 0.03,
        spec_intensity: 0.55,
        spec_power: 24.0,
        rim_intensity: 0.7,
        rim_spread: 0.45,
        hue_shift_degrees: -22.0,
        toon_steps: 2.0,
        specular_softness: 0.06,
        specular_offset: 0.01,
        specular_size: 0.4,
        ao_intensity: 0.8,
        // Fase 2 (#18): material anime VRoid/MToon — no frame congelado todos
        // os slots de textura estão desativados, então os 16 floats novos do
        // MaterialUniform são zero (exceto shade_toony = 1), e o visual do
        // frame continua idêntico ao layout de 28 floats.
        emission_color: [0.0, 0.0, 0.0, 0.0],
        emission_intensity: 0.0,
        second_shade_shift: 0.0,
        second_shade_softness: 0.05,
        matcap_intensity: 0.0,
        main_texture_enabled: false,
        shade_texture_enabled: false,
        second_shade_texture_enabled: false,
        emission_texture_enabled: false,
        matcap_enabled: false,
        matcap_mode: 0,
        shade_toony: true,
        outline_color: [0.25, 0.15, 0.2, 1.0],
        outline_width: 0.004,
        outline_depth_bias: 0.02,
        outline_opacity: 0.9,
        outline_smoothness: 0.01,
      },
      expected: {
        view_proj: [
          1.196970105, -0.052555762, -0.121650361, -0.121589534, 0.0, 2.102230549, -0.197681829, -0.19758299,
          -0.149621263, -0.420446098, -0.973202884, -0.972716272, 0.0, -2.207341909, 3.448943377, 3.497219086,
        ],
        model_matrix: [
          1.039230585, 0.0, -0.599999845, 0.0, 0.0, 0.899999976, 0.0, 0.0,
          0.549999833, 0.0, 0.952628016, 0.0, 0.25, 0.5, -0.75, 1.0,
        ],
        normal_matrix: [
          0.721687913, -0.0, -0.416666538, 0.0, -0.0, 1.111111164, -0.0, 0.0,
          0.454545319, -0.0, 0.787295878, 0.0, 0.0, 0.0, 0.0, 1.0,
        ],
        camera_uniform: [
          1.196970105, -0.052555762, -0.121650361, -0.121589534, 0.0, 2.102230549, -0.197681829, -0.19758299,
          -0.149621263, -0.420446098, -0.973202884, -0.972716272, 0.0, -2.207341909, 3.448943377, 3.497219086,
          0.400000006, 1.700000048, 3.200000048, 1.0, 1.039230585, 0.0, -0.599999845, 0.0,
          0.0, 0.899999976, 0.0, 0.0, 0.549999833, 0.0, 0.952628016, 0.0,
          0.25, 0.5, -0.75, 1.0, 0.721687913, -0.0, -0.416666538, 0.0,
          -0.0, 1.111111164, -0.0, 0.0, 0.454545319, -0.0, 0.787295878, 0.0,
          0.0, 0.0, 0.0, 1.0,
        ],
        light_uniform: [
          0.360000014, 0.800000012, 0.479999989, 1.350000024, 1.0, 0.959999979, 0.899999976, 0.419999987,
          0.949999988, 0.970000029, 1.0, 0.850000024, 0.519999981, 0.600000024, 0.779999971, 1.0,
          0.25, 0.200000003, 0.180000007, 1.0,
        ],
        material_uniform: [
          0.980000019, 0.920000017, 0.850000024, 1.0, 0.819999993, 0.730000019, 0.779999971, 1.0,
          1.0, 1.0, 1.0, 1.0, 0.575999975, 0.773000002, 0.991999984, 1.0,
          0.579999983, 0.029999999, 0.550000012, 24.0, 0.699999988, 0.449999988, -0.383972436, 2.0,
          0.059999999, 0.01, 0.400000006, 0.800000012,
          // Fase 2 (#18): emission_color, params4 (emission, second_shade x2,
          // matcap), params5 (4 slots de textura), params6 (matcap, mode,
          // shade_toony, reserved) — frame congelado com tudo desativado.
          0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.050000001, 0.0, 0.0, 0.0, 0.0, 0.0,
          0.0, 0.0, 1.0, 0.0,
        ],
        outline_uniform: [
          0.25, 0.150000006, 0.200000003, 1.0, 0.004, 1.777777791, 0.02, 0.899999976,
          0.01, 0.0, 0.0, 0.0,
        ],
      },
    },
  };
  const rampBytes = toonRampBytes(contract.toon_ramp);
  contract.toon_ramp.bytes_len = rampBytes.length;
  contract.toon_ramp.bytes_fnv1a64 = fnv1a64Bytes(rampBytes);
  contract.toon_ramp.sample_u8 = Array.from({ length: 16 }, (_, i) => rampBytes[i * 256]);
  return contract;
}

export default renderContractFixture;
