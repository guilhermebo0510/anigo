/**
 * ANIGO — dados do contrato do renderer v1 (GERADO).
 *
 * Fonte: `scripts/render_contract_fixture.mjs` → `contracts/fixtures/render_contract_v1.json`
 * (lido pelo Rust) e este módulo (lido pelo viewport). Não edite à mão:
 * `npm run fixtures:gen` regenera os dois.
 */

export const RENDER_CONTRACT_DATA = {
  "$comment": "Render contract v1: one canonical WGSL set (crates/anigo-renderer/shaders), shared vertex/uniform layouts, bind groups, pass graph, camera conventions and toon ramp. Verified by crates/anigo-renderer/src/render_contract.rs and tests/contracts/render_contract.test.ts (same shader hashes, same uniform floats).",
  "version": 1,
  "shaders": [
    {
      "name": "cel_shading",
      "path": "crates/anigo-renderer/shaders/cel_shading.wgsl",
      "language": "wgsl",
      "role": "production",
      "entry_points": {
        "vertex": "vs_main",
        "fragment": "fs_main"
      },
      "lines": 525,
      "fnv1a64": "8a86bc11e3b88d97"
    },
    {
      "name": "inverted_hull",
      "path": "crates/anigo-renderer/shaders/inverted_hull.wgsl",
      "language": "wgsl",
      "role": "production",
      "entry_points": {
        "vertex": "vs_main",
        "fragment": "fs_main"
      },
      "lines": 136,
      "fnv1a64": "d0c5f653e26cc4f7"
    },
    {
      "name": "morph_sparse_compute",
      "path": "crates/anigo-renderer/shaders/morph_sparse_compute.wgsl",
      "language": "wgsl",
      "role": "production",
      "entry_points": {
        "compute": "cs_accumulate_morphs",
        "compute_reset": "cs_reset_vertices"
      },
      "lines": 121,
      "fnv1a64": "777d38f9081bf595"
    },
    {
      "name": "face_sdf",
      "path": "crates/anigo-renderer/shaders/face_sdf.wgsl",
      "language": "wgsl",
      "role": "library",
      "entry_points": {
        "theta": "face_sdf_theta",
        "threshold": "face_sdf_threshold",
        "factor": "face_sdf_factor"
      },
      "lines": 41,
      "fnv1a64": "bda7e76d52a49c35"
    },
    {
      "name": "anime_eye",
      "path": "crates/anigo-renderer/shaders/anime_eye.wgsl",
      "language": "wgsl",
      "role": "library",
      "entry_points": {
        "parallax": "eye_parallax_uv",
        "highlight_mask": "eye_highlight_mask",
        "highlight_rgb": "eye_highlight_rgb"
      },
      "lines": 51,
      "fnv1a64": "9c2baec2b6f6fcfa"
    },
    {
      "name": "postprocess_dof",
      "path": "crates/anigo-renderer/shaders/postprocess_dof.wgsl",
      "language": "wgsl",
      "role": "production",
      "entry_points": {
        "vertex": "vs_dof",
        "fragment": "fs_dof"
      },
      "lines": 134,
      "fnv1a64": "2caa9896a0efd4ce"
    },
    {
      "name": "webgl2_fallback/cel_vertex",
      "path": "crates/anigo-renderer/shaders/webgl2_fallback/cel_vertex.glsl",
      "language": "glsl",
      "role": "fallback_webgl2",
      "entry_points": {
        "vertex": "main"
      },
      "lines": 43,
      "fnv1a64": "0befca84c6990db3"
    },
    {
      "name": "webgl2_fallback/cel_fragment",
      "path": "crates/anigo-renderer/shaders/webgl2_fallback/cel_fragment.glsl",
      "language": "glsl",
      "role": "fallback_webgl2",
      "entry_points": {
        "fragment": "main"
      },
      "lines": 171,
      "fnv1a64": "2ad671261efabdfa"
    },
    {
      "name": "webgl2_fallback/outline_vertex",
      "path": "crates/anigo-renderer/shaders/webgl2_fallback/outline_vertex.glsl",
      "language": "glsl",
      "role": "fallback_webgl2",
      "entry_points": {
        "vertex": "main"
      },
      "lines": 47,
      "fnv1a64": "0bf2171b4bb5f3ea"
    },
    {
      "name": "webgl2_fallback/outline_fragment",
      "path": "crates/anigo-renderer/shaders/webgl2_fallback/outline_fragment.glsl",
      "language": "glsl",
      "role": "fallback_webgl2",
      "entry_points": {
        "fragment": "main"
      },
      "lines": 15,
      "fnv1a64": "bd0901e958cd0081"
    }
  ],
  "vertex_layout": {
    "stride": 72,
    "step_mode": "vertex",
    "attributes": [
      {
        "shader_location": 0,
        "offset": 0,
        "format": "float32x3",
        "name": "position"
      },
      {
        "shader_location": 1,
        "offset": 12,
        "format": "float32x3",
        "name": "normal"
      },
      {
        "shader_location": 2,
        "offset": 24,
        "format": "float32x2",
        "name": "uv"
      },
      {
        "shader_location": 3,
        "offset": 32,
        "format": "float32x4",
        "name": "anime_attr"
      },
      {
        "shader_location": 4,
        "offset": 48,
        "format": "uint16x4",
        "name": "joints"
      },
      {
        "shader_location": 5,
        "offset": 56,
        "format": "float32x4",
        "name": "weights"
      }
    ]
  },
  "uniforms": {
    "camera": {
      "struct": "CameraUniform",
      "address_space": "uniform",
      "size": 208,
      "fields": [
        {
          "name": "view_proj",
          "kind": "mat4x4",
          "offset": 0,
          "size": 64
        },
        {
          "name": "camera_pos",
          "kind": "vec4",
          "offset": 64,
          "size": 16
        },
        {
          "name": "model",
          "kind": "mat4x4",
          "offset": 80,
          "size": 64
        },
        {
          "name": "normal_mat",
          "kind": "mat4x4",
          "offset": 144,
          "size": 64
        }
      ]
    },
    "light": {
      "struct": "LightUniform",
      "address_space": "uniform",
      "size": 80,
      "fields": [
        {
          "name": "direction",
          "kind": "vec4",
          "offset": 0,
          "size": 16,
          "meaning": "xyz dir, w intensity"
        },
        {
          "name": "color",
          "kind": "vec4",
          "offset": 16,
          "size": 16,
          "meaning": "rgb color, w ambient_intensity"
        },
        {
          "name": "shadow_color",
          "kind": "vec4",
          "offset": 32,
          "size": 16,
          "meaning": "rgb tint, w saturation"
        },
        {
          "name": "ambient_sky",
          "kind": "vec4",
          "offset": 48,
          "size": 16
        },
        {
          "name": "ambient_ground",
          "kind": "vec4",
          "offset": 64,
          "size": 16
        }
      ]
    },
    "material": {
      "struct": "MaterialUniform",
      "address_space": "uniform",
      "size": 208,
      "fields": [
        {
          "name": "base_color",
          "kind": "vec4",
          "offset": 0,
          "size": 16
        },
        {
          "name": "shade_color",
          "kind": "vec4",
          "offset": 16,
          "size": 16
        },
        {
          "name": "specular_color",
          "kind": "vec4",
          "offset": 32,
          "size": 16
        },
        {
          "name": "rim_color",
          "kind": "vec4",
          "offset": 48,
          "size": 16
        },
        {
          "name": "params",
          "kind": "vec4",
          "offset": 64,
          "size": 16,
          "meaning": "shadow_threshold, shadow_smoothness, spec_intensity, spec_power"
        },
        {
          "name": "params2",
          "kind": "vec4",
          "offset": 80,
          "size": 16,
          "meaning": "rim_intensity, rim_spread, hue_shift_rad, toon_steps"
        },
        {
          "name": "params3",
          "kind": "vec4",
          "offset": 96,
          "size": 16,
          "meaning": "specular_softness, specular_offset, specular_size, ao_intensity"
        },
        {
          "name": "emission_color",
          "kind": "vec4",
          "offset": 112,
          "size": 16,
          "meaning": "MToon subEmission (rgb sRGB, w alpha)"
        },
        {
          "name": "params4",
          "kind": "vec4",
          "offset": 128,
          "size": 16,
          "meaning": "emission_intensity, second_shade_shift, second_shade_softness, matcap_intensity"
        },
        {
          "name": "params5",
          "kind": "vec4",
          "offset": 144,
          "size": 16,
          "meaning": "main_tex_enabled, shade_tex_enabled, second_shade_enabled, emission_enabled"
        },
        {
          "name": "params6",
          "kind": "vec4",
          "offset": 160,
          "size": 16,
          "meaning": "matcap_enabled, matcap_mode (0 normal/1 additive), shade_toony, reserved"
        },
        {
          "name": "params7",
          "kind": "vec4",
          "offset": 176,
          "size": 16,
          "meaning": "face_shadow_offset, face_shadow_smoothness, face_sdf_enabled, reserved"
        },
        {
          "name": "params8",
          "kind": "vec4",
          "offset": 192,
          "size": 16,
          "meaning": "eye_depth_scale, eye_highlight_intensity, eye_enabled, reserved"
        }
      ]
    },
    "outline": {
      "struct": "OutlineUniform",
      "address_space": "uniform",
      "size": 48,
      "fields": [
        {
          "name": "color",
          "kind": "vec4",
          "offset": 0,
          "size": 16
        },
        {
          "name": "params",
          "kind": "vec4",
          "offset": 16,
          "size": 16,
          "meaning": "width, aspect, depth_bias, opacity"
        },
        {
          "name": "params2",
          "kind": "vec4",
          "offset": 32,
          "size": 16,
          "meaning": "smoothness, width_tex_enabled (Fase 2 #18)"
        }
      ]
    },
    "sparse_morph_header": {
      "struct": "SparseMorphHeader",
      "address_space": "uniform",
      "size": 16,
      "fields": [
        {
          "name": "active_channel_count",
          "kind": "u32",
          "offset": 0,
          "size": 4
        },
        {
          "name": "total_vertex_count",
          "kind": "u32",
          "offset": 4,
          "size": 4
        },
        {
          "name": "total_delta_count",
          "kind": "u32",
          "offset": 8,
          "size": 4
        },
        {
          "name": "_pad",
          "kind": "u32",
          "offset": 12,
          "size": 4
        }
      ]
    },
    "morph_channel": {
      "struct": "MorphChannel",
      "address_space": "storage_read",
      "size": 16,
      "fields": [
        {
          "name": "weight",
          "kind": "f32",
          "offset": 0,
          "size": 4
        },
        {
          "name": "start_offset",
          "kind": "u32",
          "offset": 4,
          "size": 4
        },
        {
          "name": "delta_count",
          "kind": "u32",
          "offset": 8,
          "size": 4
        },
        {
          "name": "_pad",
          "kind": "u32",
          "offset": 12,
          "size": 4
        }
      ]
    },
    "sparse_morph_delta": {
      "struct": "SparseMorphDelta",
      "address_space": "storage_read",
      "size": 32,
      "fields": [
        {
          "name": "vertex_index",
          "kind": "u32",
          "offset": 0,
          "size": 4
        },
        {
          "name": "delta_px",
          "kind": "f32",
          "offset": 4,
          "size": 4
        },
        {
          "name": "delta_py",
          "kind": "f32",
          "offset": 8,
          "size": 4
        },
        {
          "name": "delta_pz",
          "kind": "f32",
          "offset": 12,
          "size": 4
        },
        {
          "name": "delta_nx",
          "kind": "f32",
          "offset": 16,
          "size": 4
        },
        {
          "name": "delta_ny",
          "kind": "f32",
          "offset": 20,
          "size": 4
        },
        {
          "name": "delta_nz",
          "kind": "f32",
          "offset": 24,
          "size": 4
        },
        {
          "name": "_pad",
          "kind": "f32",
          "offset": 28,
          "size": 4
        }
      ]
    },
    "bones": {
      "struct": "BonePalette",
      "address_space": "uniform",
      "size": 1536,
      "note": "array<mat4x4<f32>, 24>: stride 64 B por osso, sem padding entre elementos",
      "fields": [
        {
          "name": "matrices",
          "kind": "array_mat4_f32_24",
          "offset": 0,
          "size": 1536,
          "meaning": "world × inverse bind por osso, na ordem do esqueleto"
        }
      ]
    },
    "dof": {
      "struct": "DofUniform",
      "address_space": "uniform",
      "size": 48,
      "note": "params = focus_distance(m), f_number, bokeh_shape (0 círculo/1 hex), focal_m; resolution = w, h, z_near, z_far (px/m); limits = max_radius_px, sensor_height_m",
      "fields": [
        {
          "name": "params",
          "kind": "vec4",
          "offset": 0,
          "size": 16,
          "meaning": "focus_distance (m), f_number, bokeh_shape, focal_m (m)"
        },
        {
          "name": "resolution",
          "kind": "vec4",
          "offset": 16,
          "size": 16,
          "meaning": "width_px, height_px, z_near (m), z_far (m)"
        },
        {
          "name": "limits",
          "kind": "vec4",
          "offset": 32,
          "size": 16,
          "meaning": "max_radius_px, sensor_height_m (0.024), reserved, reserved"
        }
      ]
    },
    "vertex_raw": {
      "struct": "VertexRaw",
      "address_space": "vertex_and_storage_read",
      "size": 72,
      "note": "same 72 bytes in the vertex buffer and in array<VertexRaw>; the WGSL struct declares scalar members (no vec3), so align == 4 and the 72-byte array stride stays legal",
      "fields": [
        {
          "name": "pos",
          "kind": "vec3_f32",
          "offset": 0,
          "size": 12
        },
        {
          "name": "norm",
          "kind": "vec3_f32",
          "offset": 12,
          "size": 12
        },
        {
          "name": "uv",
          "kind": "vec2_f32",
          "offset": 24,
          "size": 8
        },
        {
          "name": "color",
          "kind": "vec4_f32",
          "offset": 32,
          "size": 16
        },
        {
          "name": "joints",
          "kind": "vec4_u32",
          "offset": 48,
          "size": 16
        },
        {
          "name": "weights",
          "kind": "vec4_f32",
          "offset": 56,
          "size": 16
        }
      ]
    }
  },
  "bind_groups": [
    {
      "name": "cel",
      "group": 0,
      "entries": [
        {
          "binding": 0,
          "kind": "uniform",
          "stages": [
            "vertex",
            "fragment"
          ],
          "declaration": "var<uniform> camera: CameraUniform"
        },
        {
          "binding": 1,
          "kind": "uniform",
          "stages": [
            "fragment"
          ],
          "declaration": "var<uniform> light: LightUniform"
        },
        {
          "binding": 2,
          "kind": "uniform",
          "stages": [
            "fragment"
          ],
          "declaration": "var<uniform> material: MaterialUniform"
        },
        {
          "binding": 3,
          "kind": "texture_2d<f32>",
          "stages": [
            "fragment"
          ],
          "declaration": "var toon_ramp_tex: texture_2d<f32>"
        },
        {
          "binding": 4,
          "kind": "sampler",
          "stages": [
            "fragment"
          ],
          "declaration": "var toon_ramp_sampler: sampler"
        },
        {
          "binding": 5,
          "kind": "uniform",
          "stages": [
            "vertex"
          ],
          "declaration": "var<uniform> bones: BonePalette"
        },
        {
          "binding": 6,
          "kind": "texture_2d<f32>",
          "stages": [
            "fragment"
          ],
          "declaration": "var main_tex: texture_2d<f32>"
        },
        {
          "binding": 7,
          "kind": "sampler",
          "stages": [
            "fragment"
          ],
          "declaration": "var main_sampler: sampler"
        },
        {
          "binding": 8,
          "kind": "texture_2d<f32>",
          "stages": [
            "fragment"
          ],
          "declaration": "var shade_tex: texture_2d<f32>"
        },
        {
          "binding": 9,
          "kind": "sampler",
          "stages": [
            "fragment"
          ],
          "declaration": "var shade_sampler: sampler"
        },
        {
          "binding": 10,
          "kind": "texture_2d<f32>",
          "stages": [
            "fragment"
          ],
          "declaration": "var second_shade_tex: texture_2d<f32>"
        },
        {
          "binding": 11,
          "kind": "sampler",
          "stages": [
            "fragment"
          ],
          "declaration": "var second_shade_sampler: sampler"
        },
        {
          "binding": 12,
          "kind": "texture_2d<f32>",
          "stages": [
            "fragment"
          ],
          "declaration": "var emission_tex: texture_2d<f32>"
        },
        {
          "binding": 13,
          "kind": "sampler",
          "stages": [
            "fragment"
          ],
          "declaration": "var emission_sampler: sampler"
        },
        {
          "binding": 14,
          "kind": "texture_2d<f32>",
          "stages": [
            "fragment"
          ],
          "declaration": "var sphere_add_tex: texture_2d<f32>"
        },
        {
          "binding": 15,
          "kind": "sampler",
          "stages": [
            "fragment"
          ],
          "declaration": "var sphere_add_sampler: sampler"
        },
        {
          "binding": 16,
          "kind": "texture_2d<f32>",
          "stages": [
            "fragment"
          ],
          "declaration": "var face_sdf_tex: texture_2d<f32>"
        },
        {
          "binding": 17,
          "kind": "sampler",
          "stages": [
            "fragment"
          ],
          "declaration": "var face_sdf_sampler: sampler"
        }
      ]
    },
    {
      "name": "outline",
      "group": 0,
      "entries": [
        {
          "binding": 0,
          "kind": "uniform",
          "stages": [
            "vertex"
          ],
          "declaration": "var<uniform> camera: CameraUniform"
        },
        {
          "binding": 1,
          "kind": "uniform",
          "stages": [
            "vertex",
            "fragment"
          ],
          "declaration": "var<uniform> outline: OutlineUniform"
        },
        {
          "binding": 2,
          "kind": "uniform",
          "stages": [
            "vertex"
          ],
          "declaration": "var<uniform> bones: BonePalette"
        },
        {
          "binding": 3,
          "kind": "texture_2d<f32>",
          "stages": [
            "vertex"
          ],
          "declaration": "var outline_width_tex: texture_2d<f32>"
        },
        {
          "binding": 4,
          "kind": "sampler",
          "stages": [
            "vertex"
          ],
          "declaration": "var outline_width_sampler: sampler"
        }
      ]
    },
    {
      "name": "sparse_morph",
      "group": 0,
      "entries": [
        {
          "binding": 0,
          "kind": "uniform",
          "stages": [
            "compute"
          ],
          "declaration": "var<uniform> header: SparseMorphHeader"
        },
        {
          "binding": 1,
          "kind": "storage_read",
          "stages": [
            "compute"
          ],
          "declaration": "var<storage, read> base_vertices: array<VertexRaw>"
        },
        {
          "binding": 2,
          "kind": "storage_read",
          "stages": [
            "compute"
          ],
          "declaration": "var<storage, read> morph_deltas: array<SparseMorphDelta>"
        },
        {
          "binding": 3,
          "kind": "storage_read",
          "stages": [
            "compute"
          ],
          "declaration": "var<storage, read> active_channels: array<MorphChannel>"
        },
        {
          "binding": 4,
          "kind": "storage_read_write",
          "stages": [
            "compute"
          ],
          "declaration": "var<storage, read_write> out_vertices: array<VertexRaw>"
        }
      ]
    },
    {
      "name": "dof",
      "group": 0,
      "entries": [
        {
          "binding": 0,
          "kind": "uniform",
          "stages": [
            "fragment"
          ],
          "declaration": "var<uniform> dof: DofUniform"
        },
        {
          "binding": 1,
          "kind": "texture_2d<f32>",
          "stages": [
            "fragment"
          ],
          "declaration": "var scene_color: texture_2d<f32>"
        },
        {
          "binding": 2,
          "kind": "texture_2d<f32>",
          "stages": [
            "fragment"
          ],
          "declaration": "var scene_depth: texture_2d<f32>"
        },
        {
          "binding": 3,
          "kind": "sampler",
          "stages": [
            "fragment"
          ],
          "declaration": "var dof_sampler: sampler"
        }
      ]
    }
  ],
  "passes": [
    {
      "order": 0,
      "name": "outline",
      "kind": "render",
      "shader": "inverted_hull",
      "vertex_entry": "vs_main",
      "fragment_entry": "fs_main",
      "bind_group": "outline",
      "cull_mode": "front",
      "depth_write": false,
      "depth_compare": "less-equal",
      "depth_bias": {
        "constant": 1,
        "slope_scale": 1,
        "clamp": 0
      },
      "blend": "src_alpha_one_minus_src_alpha",
      "write_mask": "all"
    },
    {
      "order": 1,
      "name": "cel",
      "kind": "render",
      "shader": "cel_shading",
      "vertex_entry": "vs_main",
      "fragment_entry": "fs_main",
      "bind_group": "cel",
      "cull_mode": "back",
      "depth_write": true,
      "depth_compare": "less-equal",
      "depth_bias": {
        "constant": 0,
        "slope_scale": 0,
        "clamp": 0
      },
      "blend": "none",
      "write_mask": "all"
    },
    {
      "order": 2,
      "name": "dof_post",
      "kind": "render",
      "shader": "postprocess_dof",
      "vertex_entry": "vs_dof",
      "fragment_entry": "fs_dof",
      "bind_group": "dof",
      "cull_mode": "back",
      "depth_write": false,
      "depth_compare": "always",
      "blend": "none",
      "write_mask": "all",
      "only_when": "dof_enabled",
      "fullscreen_triangle": true
    },
    {
      "order": -1,
      "name": "sparse_morph",
      "kind": "compute",
      "shader": "morph_sparse_compute",
      "compute_entry": "cs_accumulate_morphs",
      "bind_group": "sparse_morph",
      "workgroup_size": 64,
      "only_when": "gpu_morph_active"
    }
  ],
  "diagnostics": {
    "note": "Códigos estáveis compartilhados pelos dois lados. Severidade define se o renderer está degradado (error) ou apenas avisado (warning).",
    "codes": [
      {
        "code": "webgpu_unavailable",
        "severity": "warning"
      },
      {
        "code": "backend_unavailable",
        "severity": "error"
      },
      {
        "code": "gpu_device_error",
        "severity": "error"
      },
      {
        "code": "pipeline_init_failed",
        "severity": "error"
      },
      {
        "code": "compute_init_failed",
        "severity": "error"
      },
      {
        "code": "channel_upload_failed",
        "severity": "error"
      },
      {
        "code": "context_configure_failed",
        "severity": "warning"
      },
      {
        "code": "frame_skipped",
        "severity": "warning"
      },
      {
        "code": "snapshot_invalid",
        "severity": "error"
      },
      {
        "code": "snapshot_stale",
        "severity": "warning"
      },
      {
        "code": "geometry_unavailable",
        "severity": "warning"
      },
      {
        "code": "mesh_invalid",
        "severity": "error"
      },
      {
        "code": "model_load_failed",
        "severity": "error"
      },
      {
        "code": "geometry_rejected",
        "severity": "error"
      },
      {
        "code": "channel_missing",
        "severity": "warning"
      },
      {
        "code": "unexpected_error",
        "severity": "error"
      },
      {
        "code": "contract_drift",
        "severity": "error"
      },
      {
        "code": "device_unavailable",
        "severity": "error"
      },
      {
        "code": "shader_compile_failed",
        "severity": "error"
      },
      {
        "code": "buffer_creation_failed",
        "severity": "error"
      },
      {
        "code": "readback_failed",
        "severity": "error"
      },
      {
        "code": "dof_unavailable",
        "severity": "warning"
      }
    ]
  },
  "mesh_validation": {
    "note": "Códigos compartilhados: uma malha reprovada aqui não vira buffer de GPU.",
    "stride_bytes": 72,
    "codes": [
      {
        "code": "EMPTY_MESH",
        "meaning": "sem vértices ou sem índices"
      },
      {
        "code": "INDEX_COUNT_MISMATCH",
        "meaning": "índices não formam triângulos (múltiplo de 3)"
      },
      {
        "code": "INDEX_OUT_OF_RANGE",
        "meaning": "índice aponta para fora da lista de vértices"
      },
      {
        "code": "NON_FINITE_VALUE",
        "meaning": "posição/normal/uv/peso com NaN ou infinito"
      },
      {
        "code": "BAD_VERTEX_STRIDE",
        "meaning": "tamanho do buffer não é múltiplo do stride do vértice"
      }
    ]
  },
  "skinning": {
    "algorithm": "linear_blend_skinning",
    "joint_count": 24,
    "max_influences": 4,
    "matrices_per_joint": 16,
    "palette_uniform": "bones",
    "palette_bytes": 1536,
    "joints_location": 4,
    "weights_location": 5,
    "weights_normalization": "soma dos pesos normalizada no shader antes de combinar as matrizes",
    "unskinned_fallback": "peso total < 1e-5 devolve a matriz identidade (vértice sem influência não colapsa na origem)",
    "index_clamp": "índice de osso limitado a joint_count-1 antes de indexar a paleta",
    "block_markers": [
      "// ANIGO-SKINNING-BEGIN",
      "// ANIGO-SKINNING-END"
    ]
  },
  "face_sdf": {
    "note": "Sombra facial por SDF com projeção angular: theta = atan2(dot(L, eixoX_local), dot(L, eixoZ_local)); threshold = 0.5 + (1 - (cos(theta)*0.5+0.5)) * 0.25 + face_shadow_offset; fator = 1 - smoothstep(threshold ∓ smoothness, sdf). Canal R do SDF: 0 = centro da sombra, 1 = fora.",
    "sdf_channel": "r",
    "sdf_semantics": "0 = centro da região de sombra (nasal/olhos/queixo), 1 = fora da região",
    "neutral_when_disabled": "neutro 1x1 branco (R=1) → fator 0 → imagem inalterada",
    "darkening": 0.72,
    "block_markers": [
      "// ANIGO-FACE-SDF-BEGIN",
      "// ANIGO-FACE-SDF-END"
    ],
    "shared_by": [
      "face_sdf",
      "cel_shading"
    ],
    "entry_functions": [
      "face_sdf_theta",
      "face_sdf_threshold",
      "face_sdf_factor"
    ],
    "golden": [
      {
        "azimuth_degrees": 0,
        "theta": 0,
        "light_front": 1,
        "threshold": 0.5,
        "factor_sdf_half": 0.5
      },
      {
        "azimuth_degrees": 45,
        "theta": 0.78539819,
        "light_front": 0.8535534,
        "threshold": 0.53661167,
        "factor_sdf_half": 0.9510253
      },
      {
        "azimuth_degrees": 90,
        "theta": 1.5707964,
        "light_front": 0.5,
        "threshold": 0.625,
        "factor_sdf_half": 1
      },
      {
        "azimuth_degrees": 135,
        "theta": 2.3561945,
        "light_front": 0.14644662,
        "threshold": 0.71338832,
        "factor_sdf_half": 1
      }
    ],
    "golden_note": "sdf = 0.5, face_shadow_smoothness = 0.05, face_shadow_offset = 0, modelo identidade"
  },
  "anime_eye": {
    "note": "Íris com parallax mapping (profundidade convexa sem cavidade geométrica) e highlights desenhados à mão desacoplados da iluminação. Slot main recebe a textura de olho (Fase 2 #26) amostrada com o UV parallaxado; com eye off ou depth_scale 0, eye_uv = in.uv e o highlight some — frame congelado intacto.",
    "parallax_formula": "UV_iris = UV + V_tangent.xy * depth_scale (clamp 0..1)",
    "v_tangent_basis": "T = normalize(cross(N, up)), B = cross(N, T); V_tangent = (dot(V,T), dot(V,B))",
    "highlight": {
      "main_center": [
   0.38, 0.62
  ],
      "main_falloff": [
   0.075, 0.125
  ],
      "main_ellipse_y_scale": 0.72,
      "second_center": [
   0.68, 0.34
  ],
      "second_falloff": [
   0.028, 0.055
  ],
      "second_intensity": 0.85,
      "decoupled_from_lighting": true
    },
    "block_markers": [
      "// ANIGO-ANIME-EYE-BEGIN",
      "// ANIGO-ANIME-EYE-END"
    ],
    "shared_by": [
      "anime_eye",
      "cel_shading"
    ],
    "entry_functions": [
      "eye_parallax_uv",
      "eye_highlight_mask",
      "eye_highlight_rgb"
    ],
    "golden": [
      {
        "name": "parallax_central",
        "uv": [
   0.5, 0.5
  ],
        "v_tangent": [
   0.2, -0.1
  ],
        "depth_scale": 0.15,
        "expected_uv": [
   0.53, 0.485
  ]
      },
      {
        "name": "parallax_clamp",
        "uv": [
   0.02, 0.98
  ],
        "v_tangent": [
   0.5, 0.5
  ],
        "depth_scale": 0.2,
        "expected_uv": [
   0.12, 1
  ]
      },
      {
        "name": "mask_principal",
        "uv": [
   0.38, 0.62
  ],
        "expected_mask": 1
      },
      {
        "name": "mask_secundario",
        "uv": [
   0.68, 0.34
  ],
        "expected_mask": 0.85
      },
      {
        "name": "mask_entre_brilhos",
        "uv": [
   0.5, 0.5
  ],
        "expected_mask": 0
      }
    ],
    "golden_note": "formulas em ponto flutuante duplo, congeladas em f32",
    "gaze": {
      "eye_offsets_head_local": {
        "left": [
   0.035, -0.01, 0.09
  ],
        "right": [
   -0.035, -0.01, 0.09
  ]
      },
      "max_yaw_degrees": 45,
      "max_pitch_degrees": 35,
      "saccade_amplitude_degrees_default": 2.5,
      "saccade_amplitude_range": [
   2, 5
  ],
      "damping_default": 6,
      "note": "yaw = atan2(v.x, v.z), pitch = atan2(v.y, hypot(v.x, v.z)) no espaço local da cabeça, v = alvo − olho; clamps físicos impedem rotação além do cômodo. Micro-sacadas: soma de 3 senoides incomensuráveis × amplitude (suave, determinística em (t, seed)).",
      "golden": [
        {
          "name": "frontal",
          "target": [
   0, -0.01, 0.4
  ],
          "yaw": -0.11242713,
          "pitch": 0,
          "note": "convergência natural do olho esquerdo para o centro (−6.44°)"
        },
        {
          "name": "azimut_30",
          "target": [
   0.5, 0, 0.8660254
  ],
          "yaw": 0.53983635,
          "pitch": 0.01105322,
          "note": "alvo a 30° de azimut"
        },
        {
          "name": "elevacao_20",
          "target": [
   0, 0.1819852, 0.5
  ],
          "yaw": -0.08515939,
          "pitch": 0.43653914,
          "note": "alvo a 20° de elevação (visto do olho: 25°)"
        },
        {
          "name": "clamp_esquerda_90",
          "target": [
   10, 0, 0.1
  ],
          "yaw": 0.78539819,
          "pitch": 0.00100351,
          "note": "yaw clampado no limite físico de 45°"
        },
        {
          "name": "clamp_acima",
          "target": [
   0.035, 5, 1
  ],
          "yaw": 0,
          "pitch": 0.61086524,
          "note": "pitch clampado no limite físico de 35°"
        }
      ],
      "saccades": [
        {
          "t": 0,
          "yaw": 0.01888627,
          "pitch": 0.0121564
        },
        {
          "t": 1,
          "yaw": 0.01128491,
          "pitch": 0.00615212
        },
        {
          "t": 2,
          "yaw": 0.00091829,
          "pitch": -0.01229176
        },
        {
          "t": 3,
          "yaw": -0.01041111,
          "pitch": -0.00766383
        }
      ],
      "saccades_note": "seed 1.23, amplitude 2.5° (radianos); |sacada| ≤ amplitude sempre",
      "damping": {
        "from": [
   0, 0
  ],
        "to": [
   0.1, -0.05
  ],
        "damping_per_second": 6,
        "frames_60fps": 30,
        "expected": [
   0.09502129, -0.04751065
  ],
        "note": "suavização exponencial 1 − e^(−damping×dt) por frame"
      }
    }
  },
  "cinematography": {
    "note": "Presets de lente (24/35/50/85/135 mm em sensor full-frame 24 mm de altura), DoF por Circle of Confusion (passo de pós com bokeh circular/hexagonal) e tracking de alvo com amortecimento exponencial. Desligado (dof.enabled=false) o passo de pós não existe — frame congelado intacto.",
    "lens": {
      "sensor_height_mm": 24,
      "fov_formula": "fov_y = 2 * atan(sensor_height_mm / 2 / focal_mm)",
      "presets": [
        {
          "id": "24",
          "focal_mm": 24,
          "name": "Ação Panorâmica",
          "fov_y_degrees": 53.130102
        },
        {
          "id": "35",
          "focal_mm": 35,
          "name": "Corpo Inteiro",
          "fov_y_degrees": 37.849289
        },
        {
          "id": "50",
          "focal_mm": 50,
          "name": "Visão Humana Neutra",
          "fov_y_degrees": 26.991467
        },
        {
          "id": "85",
          "focal_mm": 85,
          "name": "Retrato Anime",
          "fov_y_degrees": 16.071421
        },
        {
          "id": "135",
          "focal_mm": 135,
          "name": "Close-up Dramático",
          "fov_y_degrees": 10.159216
        }
      ],
      "reframe": {
        "formula": "r' = r * tan(fov_from/2) / tan(fov_to/2)",
        "golden": {
          "from_mm": 50,
          "to_mm": 85,
          "radius": 3,
          "expected_radius": 5.1,
          "note": "trocar 50→85 mm afasta o orbitador para o sujeito manter o tamanho em tela (aceite 2)"
        }
      }
    },
    "dof": {
      "coc_formula": "CoC = |(D - F_dist) / D| * F^2 / (N * (F_dist - F))",
      "coc_variables": "D = distância do fragmento (m), F_dist = distância de foco (m), F = focal (m), N = número f",
      "coc_to_pixels": "radius_px = min(CoC / sensor_height_m * image_height_px, max_radius_px)",
      "bokeh_shapes": {
        "0": "circle",
        "1": "hexagon"
      },
      "samples": {
        "disc_points": 16,
        "center_always_included": true,
        "note": "amostra central com peso 1 impede halo/serrilha no contorno em foco (aceite 1)"
      },
      "defaults": {
        "focus_distance_m": 2,
        "f_number": 2,
        "focal_mm": 50,
        "max_radius_px": 16,
        "bokeh_shape": 0
      },
      "golden": [
        {
          "name": "em_foco",
          "frag_dist": 2,
          "focus_dist": 2,
          "focal_m": 0.05,
          "f_number": 2,
          "expected_coc": 0,
          "note": "plano de foco → CoC zero (rosto nítido)"
        },
        {
          "name": "50mm_f2_frente",
          "frag_dist": 1,
          "focus_dist": 2,
          "focal_m": 0.05,
          "f_number": 2,
          "expected_coc": 0.00064103
        },
        {
          "name": "50mm_f2_atras",
          "frag_dist": 3,
          "focus_dist": 2,
          "focal_m": 0.05,
          "f_number": 2,
          "expected_coc": 0.00021368,
          "note": "atras do plano o CoC e menor (fisica do fino-lente)"
        },
        {
          "name": "85mm_f2_atras",
          "frag_dist": 3,
          "focus_dist": 1.5,
          "focal_m": 0.085,
          "f_number": 2,
          "expected_coc": 0.0012765
        },
        {
          "name": "135mm_f18_closeup",
          "frag_dist": 0.5,
          "focus_dist": 1,
          "focal_m": 0.135,
          "f_number": 1.8,
          "expected_coc": 0.0117052,
          "note": "close-up dramatico: bokeh enorme"
        }
      ],
      "bokeh_px_golden": {
        "image_height_px": 1080,
        "sensor_height_m": 0.024,
        "values": [
          {
            "name": "50mm_f2_frente",
            "coc": 0.00064103,
            "expected_radius_px": 28.8462,
            "note": "acima do teto → clamp"
          },
          {
            "name": "50mm_f2_frente_clampado",
            "max_radius_px": 16,
            "expected_radius_px": 16
          },
          {
            "name": "50mm_f4_atras",
            "coc": 0.00010684,
            "expected_radius_px": 4.8078
          }
        ]
      }
    },
    "tracking": {
      "modes": [
        "off",
        "head",
        "hips",
        "poi"
      ],
      "target_positions": {
        "head": [
   0, 1.49, 0
  ],
        "hips": [
   0, 0.85, 0
  ],
        "note": "pontos canônicos do esqueleto humanoide (cabeça e centro de massa); poi = ponto arbitrário do usuário"
      },
      "damping": {
        "formula": "target += (desejado - target) * (1 - e^(-damping * dt))",
        "default_damping_per_second": 6,
        "golden": {
          "from": [
   0, 0, 0
  ],
          "to": [
   1, 0.5, 0
  ],
          "damping_per_second": 6,
          "frames_60fps": 30,
          "expected": [
   0.95021296, 0.47510648, 0
  ],
          "note": "amortecimento exponencial por componente; nunca ultrapassa o alvo"
        }
      },
      "note": "o tracking move target e eye juntos (mesmo delta amortecido) — o enquadramento é mantido enquanto o personagem se move (aceite 3); roda no viewport (CPU) a cada frame"
    }
  },
  "targets": {
    "offscreen_color_format": "rgba8unorm",
    "viewport_color_format_policy": "surface_preferred",
    "depth_format": "depth24plus",
    "msaa_samples": 4,
    "color_space": "linear_working_no_srgb_view",
    "resolve_to_swapchain": true,
    "clear": {
      "source": "scene.background_color"
    }
  },
  "camera": {
    "projection": "perspective_rh",
    "clip_depth": "zero_to_one",
    "matrix_layout": "column_major",
    "up_axis": "y",
    "fov_y_degrees_default": 45,
    "z_near_default": 0.05,
    "z_far_default": 100,
    "model_from": "scene.nodes[0].transform",
    "uniform": "camera"
  },
  "toon_ramp": {
    "width": 256,
    "height": 4,
    "format": "rgba8unorm",
    "mag_filter": "linear",
    "min_filter": "linear",
    "address_mode": "clamp_to_edge",
    "rows": [
      {
        "name": "continuous",
        "kind": "identity"
      },
      {
        "name": "one_step",
        "kind": "steps",
        "steps": [
          {
            "threshold": 0.5,
            "value": 1
          }
        ],
        "base": 0
      },
      {
        "name": "two_step",
        "kind": "steps",
        "steps": [
          {
            "threshold": 0.35,
            "value": 0.5
          },
          {
            "threshold": 0.65,
            "value": 1
          }
        ],
        "base": 0
      },
      {
        "name": "three_step",
        "kind": "steps",
        "steps": [
          {
            "threshold": 0.25,
            "value": 0.35
          },
          {
            "threshold": 0.5,
            "value": 0.7
          },
          {
            "threshold": 0.75,
            "value": 1
          }
        ],
        "base": 0
      }
    ],
    "bytes_len": 4096,
    "bytes_fnv1a64": "334eb404f19d1a4d",
    "sample_u8": [
   0, 64, 128, 192, 0, 0, 255, 255, 0, 0, 128, 255, 0, 89, 179, 255
  ]
  },
  "reference_frame": {
    "description": "Frozen frame: the uniform blocks both renderers must produce byte for byte (numbers from an independent implementation, tolerance 1e-5).",
    "camera": {
      "eye": [
   0.4, 1.7, 3.2
  ],
      "target": [
   0, 1.05, 0
  ],
      "up": [
   0, 1, 0
  ],
      "fov_y_degrees": 50,
      "fov_y_radians": 0.87266463,
      "aspect": 1.777777791,
      "z_near": 0.05,
      "z_far": 100
    },
    "model": {
      "translation": [
   0.25, 0.5, -0.75
  ],
      "scale": [
   1.2, 0.9, 1.1
  ],
      "rotation_xyzw": [
   0, 0.258819, 0, 0.965926
  ]
    },
    "light": {
      "direction": [
   0.36, 0.8, 0.48
  ],
      "intensity": 1.35,
      "color": [
   1, 0.96, 0.9
  ],
      "ambient_intensity": 0.42,
      "shadow_color": [
   0.95, 0.97, 1
  ],
      "shadow_saturation": 0.85,
      "ambient_sky": [
   0.52, 0.6, 0.78
  ],
      "ambient_ground": [
   0.25, 0.2, 0.18
  ]
    },
    "material": {
      "base_color": [
   0.98, 0.92, 0.85, 1
  ],
      "shade_color": [
   0.82, 0.73, 0.78, 1
  ],
      "specular_color": [
   1, 1, 1, 1
  ],
      "rim_color": [
   0.576, 0.773, 0.992, 1
  ],
      "shadow_threshold": 0.58,
      "shadow_smoothness": 0.03,
      "spec_intensity": 0.55,
      "spec_power": 24,
      "rim_intensity": 0.7,
      "rim_spread": 0.45,
      "hue_shift_degrees": -22,
      "toon_steps": 2,
      "specular_softness": 0.06,
      "specular_offset": 0.01,
      "specular_size": 0.4,
      "ao_intensity": 0.8,
      "emission_color": [
   0, 0, 0, 0
  ],
      "emission_intensity": 0,
      "second_shade_shift": 0,
      "second_shade_softness": 0.05,
      "matcap_intensity": 0,
      "main_texture_enabled": false,
      "shade_texture_enabled": false,
      "second_shade_texture_enabled": false,
      "emission_texture_enabled": false,
      "matcap_enabled": false,
      "matcap_mode": 0,
      "shade_toony": true,
      "face_shadow_offset": 0,
      "face_shadow_smoothness": 0.05,
      "face_sdf_enabled": false,
      "eye_depth_scale": 0,
      "eye_highlight_intensity": 0,
      "eye_enabled": false,
      "gaze_tracking_enabled": false,
      "gaze_saccade_amplitude": 2.5,
      "gaze_damping": 6,
      "outline_color": [
   0.25, 0.15, 0.2, 1
  ],
      "outline_width": 0.004,
      "outline_depth_bias": 0.02,
      "outline_opacity": 0.9,
      "outline_smoothness": 0.01
    },
    "expected": {
      "view_proj": [
   1.196970105, -0.052555762, -0.121650361, -0.121589534, 0, 2.102230549, -0.197681829, -0.19758299,
   -0.149621263, -0.420446098, -0.973202884, -0.972716272, 0, -2.207341909, 3.448943377, 3.497219086
  ],
      "model_matrix": [
   1.039230585, 0, -0.599999845, 0, 0, 0.899999976, 0, 0, 0.549999833, 0, 0.952628016, 0, 0.25, 0.5,
   -0.75, 1
  ],
      "normal_matrix": [
   0.721687913, 0, -0.416666538, 0, 0, 1.111111164, 0, 0, 0.454545319, 0, 0.787295878, 0, 0, 0, 0, 1
  ],
      "camera_uniform": [
   1.196970105, -0.052555762, -0.121650361, -0.121589534, 0, 2.102230549, -0.197681829, -0.19758299,
   -0.149621263, -0.420446098, -0.973202884, -0.972716272, 0, -2.207341909, 3.448943377, 3.497219086,
   0.400000006, 1.700000048, 3.200000048, 1, 1.039230585, 0, -0.599999845, 0, 0, 0.899999976, 0, 0,
   0.549999833, 0, 0.952628016, 0, 0.25, 0.5, -0.75, 1, 0.721687913, 0, -0.416666538, 0, 0,
   1.111111164, 0, 0, 0.454545319, 0, 0.787295878, 0, 0, 0, 0, 1
  ],
      "light_uniform": [
   0.360000014, 0.800000012, 0.479999989, 1.350000024, 1, 0.959999979, 0.899999976, 0.419999987,
   0.949999988, 0.970000029, 1, 0.850000024, 0.519999981, 0.600000024, 0.779999971, 1, 0.25,
   0.200000003, 0.180000007, 1
  ],
      "material_uniform": [
   0.980000019, 0.920000017, 0.850000024, 1, 0.819999993, 0.730000019, 0.779999971, 1, 1, 1, 1, 1,
   0.575999975, 0.773000002, 0.991999984, 1, 0.579999983, 0.029999999, 0.550000012, 24, 0.699999988,
   0.449999988, -0.383972436, 2, 0.059999999, 0.01, 0.400000006, 0.800000012, 0, 0, 0, 0, 0, 0,
   0.050000001, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0.050000001, 0, 0, 0, 0, 0, 0
  ],
      "outline_uniform": [
   0.25, 0.150000006, 0.200000003, 1, 0.004, 1.777777791, 0.02, 0.899999976, 0.01, 0, 0, 0
  ]
    }
  }
} as const;
