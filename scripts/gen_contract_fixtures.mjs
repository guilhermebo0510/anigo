#!/usr/bin/env node
/**
 * ANIGO — canonical contract fixture generator (deterministic).
 *
 * Writes `contracts/fixtures/*.json`: tiny, fully specified payloads that both
 * sides of the Rust ⇄ TypeScript boundary must decode to the *same* values.
 *
 * The generator only knows the *documented* layouts (see
 * `crates/anigo-core/src/snapshot.rs` module docs and `contracts/README.md`);
 * it never imports application code, so the fixture is an independent statement
 * of the contract rather than a snapshot of one implementation.
 *
 * Usage:
 *   node scripts/gen_contract_fixtures.mjs            # write fixtures
 *   node scripts/gen_contract_fixtures.mjs --check    # verify they are current
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const outDir = path.join(root, "contracts", "fixtures");

const VERTEX_STRIDE_BYTES = 72;
const MORPH_DELTA_STRIDE_BYTES = 32;

/** Packs vertices into the frozen 72-byte layout (pos f32x3 | normal f32x3 | uv
 *  f32x2 | color f32x4 | joints u16x4 | weights f32x4). */
export function packVertices(vertices) {
  const buffer = Buffer.alloc(vertices.length * VERTEX_STRIDE_BYTES);
  vertices.forEach((vertex, i) => {
    const base = i * VERTEX_STRIDE_BYTES;
    vertex.position.forEach((value, j) => buffer.writeFloatLE(value, base + j * 4));
    vertex.normal.forEach((value, j) => buffer.writeFloatLE(value, base + 12 + j * 4));
    vertex.uv.forEach((value, j) => buffer.writeFloatLE(value, base + 24 + j * 4));
    vertex.color.forEach((value, j) => buffer.writeFloatLE(value, base + 32 + j * 4));
    (vertex.joints ?? [0, 0, 0, 0]).forEach((value, j) =>
      buffer.writeUInt16LE(value, base + 48 + j * 2)
    );
    (vertex.weights ?? [1, 0, 0, 0]).forEach((value, j) =>
      buffer.writeFloatLE(value, base + 56 + j * 4)
    );
  });
  return buffer;
}

/** Packs indices as little-endian u32. */
export function packIndices(indices) {
  const buffer = Buffer.alloc(indices.length * 4);
  indices.forEach((value, i) => buffer.writeUInt32LE(value >>> 0, i * 4));
  return buffer;
}

/** Packs sparse morph deltas into the frozen 32-byte layout. */
export function packDeltas(deltas) {
  const buffer = Buffer.alloc(deltas.length * MORPH_DELTA_STRIDE_BYTES);
  deltas.forEach((delta, i) => {
    const base = i * MORPH_DELTA_STRIDE_BYTES;
    buffer.writeUInt32LE(delta.vertexIndex >>> 0, base);
    delta.position.forEach((value, j) => buffer.writeFloatLE(value, base + 4 + j * 4));
    delta.normal.forEach((value, j) => buffer.writeFloatLE(value, base + 16 + j * 4));
    buffer.writeFloatLE(0, base + 28);
  });
  return buffer;
}

const fixture = (() => {
  // 1-triangle mesh — enough to exercise every field of the vertex layout.
  const vertices = [
    {
      position: [0, 0, 0],
      normal: [0, 1, 0],
      uv: [0, 0],
      color: [1, 0.5, 1, 1],
      joints: [0, 1, 2, 3],
      weights: [1, 0, 0, 0],
    },
    {
      position: [1, 0, 0],
      normal: [0, 1, 0],
      uv: [1, 0],
      color: [0.9, 0.4, 0.8, 1],
      joints: [4, 5, 0, 0],
      weights: [0.5, 0.5, 0, 0],
    },
    {
      position: [0, 1, 0],
      normal: [0, 0, 1],
      uv: [0, 1],
      color: [0.8, 0.3, 0.7, 1],
      joints: [6, 0, 0, 0],
      weights: [1, 0, 0, 0],
    },
  ];
  const indices = [0, 1, 2];
  const deltas = [
    { vertexIndex: 1, position: [0.01, 0.02, 0.03], normal: [0, 0.5, 0] },
    { vertexIndex: 2, position: [-0.01, 0, 0.02], normal: [0.1, 0, 0] },
    { vertexIndex: 2, position: [0.005, 0.01, -0.02], normal: [0, 0.2, 0.1] },
  ];
  const channels = [
    {
      target: "mrf_head_width",
      slider_id: "head_width",
      start_offset: 0,
      delta_count: 2,
    },
    {
      target: "mrf_jaw_v_line_taper",
      slider_id: "jaw_v_line_taper",
      start_offset: 2,
      delta_count: 1,
    },
  ];

  return {
    $comment:
      "Canonical CoreSnapshot v1 fixture — decoded by crates/anigo-core/src/snapshot.rs::tests and by src/contracts/core_snapshot.v1.ts tests.",
    snapshot_version: 1,
    static_payload: {
      format_version: 1,
      static_revision: 3,
      base_gender: "Male",
      mesh_asset_id: "ast_fixture_mesh",
      mesh_uri: "anigo://fixture/triangle",
      vertex_count: vertices.length,
      index_count: indices.length,
      vertex_stride_bytes: VERTEX_STRIDE_BYTES,
      vertex_buffer_base64: packVertices(vertices).toString("base64"),
      index_buffer_base64: packIndices(indices).toString("base64"),
      topology_hash: "00000000deadbeef",
      morph_delta_stride_bytes: MORPH_DELTA_STRIDE_BYTES,
      morph_channels: channels,
      morph_deltas_base64: packDeltas(deltas).toString("base64"),
      morph_total_deltas: deltas.length,
      catalog_fingerprint: "0123456789abcdef",
    },
    dynamic: {
      dynamic_revision: 7,
      static_revision: 3,
      project_id: "prj_fixture",
      project_name: "Fixture Project",
      character_id: "chr_canonical",
      base_gender: "Male",
      gender_dimorphism: 0.5,
      somatotype: { endomorph: 0.4, mesomorph: 0.3, ectomorph: 0.3 },
      morph_weights: [
        {
          target: "mrf_head_width",
          slider_id: "head_width",
          value: 1.2,
          weight: 0.2,
        },
        {
          target: "mrf_jaw_v_line_taper",
          slider_id: "jaw_v_line_taper",
          value: 0.6,
          weight: 0.1,
        },
      ],
      camera: {
        eye: [0, 1.5, 3.5],
        target: [0, 1, 0],
        up: [0, 1, 0],
        fov_y_radians: 0.7853982,
        z_near: 0.05,
        z_far: 100,
        aspect: 1.7777778,
      },
      lights: [
        {
          light_id: "lgt_key",
          direction: [0.577, 0.577, 0.577],
          color: [1, 0.98, 0.95],
          intensity: 1,
          shadow_color: [1, 1, 1],
          ambient_intensity: 0.35,
          shadow_saturation: 1,
          ambient_sky: [0.52, 0.6, 0.78],
          ambient_ground: [0.25, 0.2, 0.18],
        },
      ],
      materials: [
        {
          material_id: "mat_default_anime",
          name: "DefaultAnimeMaterial",
          base_color: [0.98, 0.92, 0.85, 1],
          shade_color: [0.82, 0.73, 0.78, 1],
          outline_color: [0.25, 0.15, 0.2, 1],
          outline_width: 0.0035,
          outline_opacity: 1,
          outline_smoothness: 0,
          outline_depth_bias: 0,
          shadow_threshold: 0.5,
          shadow_smoothness: 0.02,
          specular_color: [1, 1, 1, 1],
          spec_intensity: 0.4,
          spec_power: 32,
          specular_softness: 0.05,
          specular_offset: 0,
          specular_size: 0.45,
          rim_color: [0.576, 0.773, 0.992, 1],
          rim_intensity: 0.8,
          rim_spread: 0.4,
          hue_shift: -15,
          toon_steps: 1,
          ao_intensity: 0.85,
        },
      ],
      nodes: [
        {
          node_id: "nod_character_base",
          name: "Anime Mannequin",
          visible: true,
          material_id: "mat_default_anime",
          translation: [0, 0, 0],
          rotation: [0, 0, 0, 1],
          scale: [1, 1, 1],
        },
      ],
      render: {
        settings_version: 1,
        msaa_samples: 4,
        background_color: [0.08, 0.09, 0.13, 1],
        color: {
          input_texture_space: "srgb",
          working_space: "linear_srgb",
          display_space: "srgb",
        },
        tonemap: "none",
      },
      deformation_authority: "reference_ts",
      deformation_coverage: {
        total_sliders: 157,
        sliders_with_geometry: 157,
        morph_targets: 157,
        bakes_proportions: false,
        bakes_gender: false,
        somatotype_via_macro_sliders: false,
        proportion_policy_version: 1,
        somatotype_policy_version: 1,
      },
    },
    // Values the decoders must produce (independent of the packing code).
    expected_first_position: [0, 0, 0],
    expected_second_position: [1, 0, 0],
    expected_first_delta_vertex_index: 1,
    expected_first_delta_position: [0.01, 0.02, 0.03],
    expected_channel_count: 2,
    expected_total_deltas: 3,
    expected_vertex_count: 3,
    expected_index_count: 3,
  };
})();

const serialized = `${JSON.stringify(fixture, null, 2)}\n`;
const target = path.join(outDir, "core_snapshot_v1.json");

if (process.argv.includes("--check")) {
  const current = fs.existsSync(target) ? fs.readFileSync(target, "utf8") : "";
  if (current !== serialized) {
    console.error(`[gen_contract_fixtures] ${path.relative(root, target)} is stale — run without --check`);
    process.exit(1);
  }
  console.log(`[gen_contract_fixtures] ${path.relative(root, target)} is up to date`);
  process.exit(0);
}

fs.mkdirSync(outDir, { recursive: true });
fs.writeFileSync(target, serialized);
console.log(`[gen_contract_fixtures] wrote ${path.relative(root, target)}`);
