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
import { renderContractFixture } from "./render_contract_fixture.mjs";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const outDir = path.join(root, "contracts", "fixtures");

const VERTEX_STRIDE_BYTES = 72;
const MORPH_DELTA_STRIDE_BYTES = 32;
// P1-04: paleta de skinning (24 ossos × mat4) — mesmo tamanho do bloco `bones`
// do render contract e do `SkinPayload` do core.
const BONE_COUNT = 24;
const MATRIX_FLOATS = 16;

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

/**
 * P1-04: paleta neutra (`boneCount` matrizes identidade, 16 floats cada).
 * Mesmo layout do bloco `bones` do render contract e do `SkinPayload` do core.
 */
export function identityPalette(boneCount) {
  const palette = [];
  for (let bone = 0; bone < boneCount; bone++) {
    palette.push(
      1, 0, 0, 0,
      0, 1, 0, 0,
      0, 0, 1, 0,
      0, 0, 0, 1
    );
  }
  return palette;
}

/** FNV-1a-64 de bytes crus, em hex de 16 dígitos (mesmo algoritmo do Rust/TS). */
export function fnv1a64Bytes(bytes) {
  let hash = 0xcbf29ce484222325n;
  for (const byte of bytes) {
    hash ^= BigInt(byte);
    hash = (hash * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return hash.toString(16).padStart(16, "0");
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

/** Dados crus do fixture (o manifesto de exportação reusa a mesma malha). */
let fixtureData;

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

  fixtureData = { vertices, indices, deltas, channels };

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
      skin: {
        bone_count: BONE_COUNT,
        // Identidade por osso: é o que o núcleo entrega enquanto as proporções
        // são assadas na malha base (skinning neutro, Σ wᵢ·(I·p) = p).
        palette: identityPalette(BONE_COUNT),
        bind_pose: "canonical_rest",
        proportions_baked: true,
        palette_is_identity: true,
      },
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
          // Issue #12: a árvore de transformações também faz parte do snapshot
          // (`W(node) = W(pai) × T(local)`), então o fixture declara a raiz do
          // personagem com a matriz mundial identidade.
          parent_id: null,
          children: [],
          kind: "character_root",
          world_matrix: [
            1, 0, 0, 0,
            0, 1, 0, 0,
            0, 0, 1, 0,
            0, 0, 0, 1,
          ],
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
        // Issue #14: reconfiguração do render graph via snapshot — ordem dos
        // passes, passes desligados e o pré-passe de profundidade (o plano é
        // derivado do contrato + estes três campos, nos dois renderers).
        graph_order: [],
        graph_disabled: [],
        depth_prepass: false,
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

/**
 * Stable-id fixture: the documented derivation `ast_<fnv1a64(normalize_uri)>`.
 *
 * Independent of both implementations — the Rust `AssetId::for_uri` and the TS
 * `assetIdForUri` must both reproduce these values.
 */
function normalizeUri(uri) {
  let normalized = uri.trim().replace(/\\/g, "/");
  const query = normalized.indexOf("?");
  if (query !== -1) normalized = normalized.slice(0, query);
  const fragment = normalized.indexOf("#");
  if (fragment !== -1) normalized = normalized.slice(0, fragment);
  while (normalized.startsWith("./")) normalized = normalized.slice(2);
  return normalized.toLowerCase();
}

function fnv1a64(input) {
  let hash = 0xcbf29ce484222325n;
  for (const byte of new TextEncoder().encode(input)) {
    hash ^= BigInt(byte);
    hash = (hash * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return hash;
}

export function assetIdForUri(uri) {
  return `ast_${fnv1a64(normalizeUri(uri)).toString(16).padStart(16, "0")}`;
}

/**
 * Manifesto de exportação (P0 "Consolidar o renderer" / §8 "viewport e
 * exportação passarem teste de paridade").
 *
 * Derivado do fixture do snapshot — o manifesto **é** a identidade verificável
 * do artefato exportado, então os checksums da geometria base saem exatamente
 * dos bytes que o snapshot entrega ao viewport (`vertex_buffer_base64` /
 * `index_buffer_base64`). Quem confere do lado Rust: `crates/anigo-core/src/
 * export.rs::tests::fixture_manifest_agrees_with_the_snapshot_fixture`.
 *
 * A parte deformada segue a regra canônica de acumulação (a mesma do WGSL e do
 * `apply_cpu` do núcleo): `p += w · Δp` e `n = normalize(n + Σ w · Δn)`, com
 * canais de |w| ≤ 1e-6 ignorados. O teste do TypeScript reproduz esses dois
 * checksums passando pelo caminho real do viewport (`applyDeltasCpu`).
 */
export function applyFixtureMorphs(vertices, deltas, morphWeights, channels) {
  // Aritmética em **float32**, exatamente como o buffer do viewport: as posições,
  // normais e deltas vêm de bytes f32 (`Buffer.writeFloatLE`/`Math.fround`), e
  // cada acumulação é arredondada ao ser gravada de volta no buffer. Sem isso o
  // checksum do fixture não bateria com o `applyDeltasCpu` do viewport.
  const f32 = Math.fround;
  const out = vertices.map((vertex) => ({
    ...vertex,
    position: vertex.position.map(f32),
    normal: vertex.normal.map(f32),
  }));
  for (const channel of channels) {
    const entry = morphWeights.find((weight) => weight.slider_id === channel.slider_id);
    // O peso chega como número de JSON (f64) — o mesmo que o viewport usa.
    const weight = entry ? entry.weight : 0;
    if (Math.abs(weight) <= 1e-6) continue;
    for (let index = 0; index < channel.delta_count; index++) {
      const delta = deltas[channel.start_offset + index];
      if (!delta) continue;
      const vertex = out[delta.vertexIndex];
      if (!vertex) continue;
      for (let axis = 0; axis < 3; axis++) {
        const dpos = f32(delta.position[axis]);
        const dnorm = f32(delta.normal[axis]);
        vertex.position[axis] = f32(vertex.position[axis] + f32(weight * dpos));
        vertex.normal[axis] = f32(vertex.normal[axis] + f32(weight * dnorm));
      }
    }
  }
  for (const vertex of out) {
    const squared =
      vertex.normal[0] * vertex.normal[0] +
      vertex.normal[1] * vertex.normal[1] +
      vertex.normal[2] * vertex.normal[2];
    if (squared > 1e-12) {
      const inverse = 1 / Math.sqrt(squared);
      vertex.normal = vertex.normal.map((value) => f32(value * inverse));
    }
  }
  return out;
}

const exportManifestFixture = (() => {
  const { vertices, indices, deltas, channels } = fixtureData;
  const staticPayload = fixture.static_payload;
  const dynamic = fixture.dynamic;
  const deformed = applyFixtureMorphs(vertices, deltas, dynamic.morph_weights, channels);
  const vertexBytes = Buffer.from(staticPayload.vertex_buffer_base64, "base64");
  const indexBytes = Buffer.from(staticPayload.index_buffer_base64, "base64");
  const deformedBytes = packVertices(deformed);
  const deformedPositions = Buffer.alloc(deformed.length * 12);
  deformed.forEach((vertex, i) => {
    vertex.position.forEach((value, axis) => deformedPositions.writeFloatLE(value, i * 12 + axis * 4));
  });

  return {
    $comment:
      "Export manifest v1 fixture — a identidade verificável do artefato exportado. " +
      "Decodificado por src/contracts/export_manifest.v1.ts (viewport) e por " +
      "crates/anigo-core/src/export.rs (Rust); os checksums da base são os bytes do " +
      "core_snapshot_v1.json, então o teste de paridade do frontend compara o que o " +
      "viewport desenha com o que foi exportado usando dados reais.",
    format_version: 1,
    generator: "anigo-core/export",
    project: {
      project_id: dynamic.project_id,
      project_name: dynamic.project_name,
      character_id: dynamic.character_id,
      base_gender: dynamic.base_gender,
      static_revision: staticPayload.static_revision,
      dynamic_revision: dynamic.dynamic_revision,
      snapshot_version: fixture.snapshot_version,
    },
    geometry: {
      vertex_count: vertices.length,
      index_count: indices.length,
      triangle_count: indices.length / 3,
      vertex_stride_bytes: VERTEX_STRIDE_BYTES,
      topology_hash: staticPayload.topology_hash,
      base_vertex_checksum: fnv1a64Bytes(vertexBytes),
      base_index_checksum: fnv1a64Bytes(indexBytes),
      catalog_fingerprint: staticPayload.catalog_fingerprint,
      mesh_uri: staticPayload.mesh_uri,
      mesh_asset_id: staticPayload.mesh_asset_id,
    },
    deformation: {
      authority: "core",
      morph_total_deltas: staticPayload.morph_total_deltas,
      active_channels: dynamic.morph_weights.length,
      morph_weights: dynamic.morph_weights,
      deformed_vertex_checksum: fnv1a64Bytes(deformedBytes),
      deformed_position_checksum: fnv1a64Bytes(deformedPositions),
    },
    skin: {
      bone_count: BONE_COUNT,
      bind_pose: staticPayload.skin.bind_pose,
      palette_is_identity: staticPayload.skin.palette_is_identity,
      palette_checksum: fnv1a64Bytes(Buffer.from(new Float32Array(identityPalette(BONE_COUNT)).buffer)),
      skinned_vertices: vertices.length,
      unskinned_vertices: 0,
    },
  };
})();

const assetIdsFixture = (() => {
  const uris = [
    "anigo://base/anigo_base_male.glb",
    "anigo://base/anigo_base_female.glb",
    "anigo://preset/cube",
    "anigo://preset/uv_sphere",
    // Normalization cases: trailing spaces, uppercase, query and fragment.
    "  anigo://base/anigo_base_male.glb  ",
    "ANIGO://BASE/ANIGO_BASE_MALE.GLB",
    "anigo://preset/cube?v=2",
    "anigo://preset/cube#lod1",
    "./assets/models/character.glb",
    "anigo://preset/cube?version=2#lod1",
    "anigo://imports/assets/character_face.glb",
  ];
  return {
    $comment:
      "Stable asset ids for the frozen derivation `ast_<fnv1a64(normalize_uri(uri))>`. " +
      "Validated by crates/anigo-core/src/ids.rs::tests (Rust) and " +
      "src/services/project_persistence.ts::assetIdForUri (TypeScript).",
    algorithm: "fnv1a64(normalize_uri(uri)) hexadecimal, 16 digits, prefixed with 'ast_'",
    normalization: "trim, backslashes to slashes, drop ?query, drop #fragment, strip leading ./ , lowercase",
    ids: uris.map((uri) => ({ uri, asset_id: assetIdForUri(uri) })),
  };
})();

/**
 * Command-log fixture (P0 undo/redo item 3).
 *
 * The log stores the commands that were **accepted** by the history, in order,
 * so replaying it rebuilds the session. Both sides validate the same payload:
 * `crates/anigo-core/src/command.rs` (`CommandLog::from_json` + byte-parity test)
 * and `src/services/command_history.ts` (`parseCommandLog`).
 */
const commandLogFixture = (() => {
  const entries = [
    {
      sequence: 1,
      revision: 1,
      description: "Morph head_width",
      scope: "deformation",
      command: { kind: "set_morph_value", target: "mrf_head_width", value: 1.25 },
    },
    {
      sequence: 2,
      revision: 2,
      description: "Somatotype",
      scope: "deformation",
      command: { kind: "set_somatotype", endomorph: 0.4, mesomorph: 0.35, ectomorph: 0.25 },
    },
    {
      sequence: 3,
      revision: 3,
      description: "Material outline",
      scope: "shading",
      command: {
        kind: "set_material_params",
        material_id: "mat_default_anime",
        patch: { outline_width: 2.0, toon_steps: 2.0 },
      },
    },
    {
      sequence: 4,
      revision: 4,
      description: "Proporções",
      scope: "base_geometry",
      command: {
        kind: "set_proportions",
        head_scale: 1.2,
        shoulder_width: 1.15,
        height_overall: 1.05,
      },
    },
    {
      sequence: 5,
      revision: 5,
      description: "Visibilidade",
      scope: "presentation",
      command: { kind: "set_node_visibility", node_id: "nod_character_base", visible: false },
    },
  ];
  return {
    $comment:
      "Command log v1: accepted commands in application order. Decoded by " +
      "crates/anigo-core/src/command.rs (`CommandLog::from_json`) and " +
      "src/services/command_history.ts (`parseCommandLog`).",
    version: 1,
    entries,
    // Invariants the decoders must observe.
    expected_entry_count: entries.length,
    expected_undo_depth: entries.length,
    expected_kinds: entries.map((entry) => entry.command.kind),
    expected_scopes: entries.map((entry) => entry.scope),
    expected_sequences: entries.map((entry) => entry.sequence),
  };
})();

const renderContractSerialized = `${compactNumberArrays(JSON.stringify(renderContractFixture(), null, 2))}\n`;
const renderContractTarget = path.join(outDir, "render_contract_v1.json");
// The viewport cannot import JSON under Node's ESM rules (the test runner runs
// the same modules), so the same document is also emitted as a typed TS module.
// Both files come from `scripts/render_contract_fixture.mjs` (single source) and
// `tests/contracts/render_contract.test.ts` asserts they are identical.
const renderContractCompact = compactNumberArrays(JSON.stringify(renderContractFixture(), null, 2));
const renderContractTsSerialized =
  "/**\n" +
  " * ANIGO — dados do contrato do renderer v1 (GERADO).\n" +
  " *\n" +
  " * Fonte: `scripts/render_contract_fixture.mjs` → `contracts/fixtures/render_contract_v1.json`\n" +
  " * (lido pelo Rust) e este módulo (lido pelo viewport). Não edite à mão:\n" +
  " * `npm run fixtures:gen` regenera os dois.\n" +
  " */\n\n" +
  "export const RENDER_CONTRACT_DATA = " +
  renderContractCompact +
  " as const;\n";
const renderContractTsTarget = path.join(
  path.resolve(outDir, "..", ".."),
  "src/contracts/render_contract_data.v1.ts"
);

/**
 * Keeps the generated JSON readable: arrays that contain only numbers are packed
 * onto wrapped lines instead of one value per line (the uniform reference frame
 * has hundreds of floats).
 */
function compactNumberArrays(json) {
  return json.replace(/\[\n\s*((?:-?[\d.]+(?:e-?\d+)?,?\s*)+)\n\s*\]/g, (match, body) => {
    const values = body.split(",").map((value) => value.trim()).filter((value) => value.length > 0);
    if (values.length === 0 || !values.every((value) => /^-?[\d.]+(?:e-?\d+)?$/.test(value))) return match;
    const lines = [];
    let current = "   ";
    for (const value of values) {
      const candidate = current === "   " ? current + value : `${current}, ${value}`;
      if (candidate.length > 100) {
        lines.push(`${current},`);
        current = `   ${value}`;
      } else {
        current = candidate;
      }
    }
    lines.push(current);
    return `[\n${lines.join("\n")}\n  ]`;
  });
}

const serialized = `${JSON.stringify(fixture, null, 2)}\n`;
const target = path.join(outDir, "core_snapshot_v1.json");
const exportManifestSerialized = `${JSON.stringify(exportManifestFixture, null, 2)}\n`;
const exportManifestTarget = path.join(outDir, "export_manifest_v1.json");
const assetIdsSerialized = `${JSON.stringify(assetIdsFixture, null, 2)}\n`;
const assetIdsTarget = path.join(outDir, "asset_ids_v1.json");
const commandLogSerialized = `${JSON.stringify(commandLogFixture, null, 2)}\n`;
const commandLogTarget = path.join(outDir, "command_log_v1.json");
const renderContractFile = path.join(outDir, "render_contract_v1.json");

const outputs = [
  { target, serialized },
  { target: assetIdsTarget, serialized: assetIdsSerialized },
  { target: commandLogTarget, serialized: commandLogSerialized },
  { target: exportManifestTarget, serialized: exportManifestSerialized },
  { target: renderContractFile, serialized: renderContractSerialized },
  { target: renderContractTsTarget, serialized: renderContractTsSerialized },
];

if (process.argv.includes("--check")) {
  let stale = 0;
  for (const output of outputs) {
    const current = fs.existsSync(output.target) ? fs.readFileSync(output.target, "utf8") : "";
    if (current !== output.serialized) {
      console.error(
        `[gen_contract_fixtures] ${path.relative(root, output.target)} is stale — run without --check`
      );
      stale++;
    } else {
      console.log(`[gen_contract_fixtures] ${path.relative(root, output.target)} is up to date`);
    }
  }
  process.exit(stale === 0 ? 0 : 1);
}

fs.mkdirSync(outDir, { recursive: true });
for (const output of outputs) {
  fs.writeFileSync(output.target, output.serialized);
  console.log(`[gen_contract_fixtures] wrote ${path.relative(root, output.target)}`);
}
