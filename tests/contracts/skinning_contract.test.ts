/**
 * ANIGO — contrato de skinning (P1 — Robustez, item 4).
 *
 * O que este teste protege: a paleta de ossos existe **uma vez só** (no render
 * contract), o jacaré de duas numerações de osso (malha × esqueleto) não volta,
 * e o viewport sobe a paleta do núcleo em vez de inventar matrizes.
 *
 * Três camadas:
 *   1. contrato congelado (`render_contract_v1.json`) — números e bindings;
 *   2. shaders em disco — bloco marcado idêntico nos dois WGSL, LBS conferido
 *      pelo `scripts/check_wgsl.mjs` (o mesmo código que roda no CI);
 *   3. consumidores — snapshot do núcleo → `ViewportGeometry` → uniform buffer.
 */

import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import {
  RENDER_CONTRACT,
  bonePaletteBytes,
  bonePaletteFloats,
  renderPassOrder,
  skinning,
  skinningBinding,
  uniformSize,
} from "../../src/contracts/render_contract.v1.ts";
import {
  decodeCoreSnapshot,
  type CoreSnapshotWire,
} from "../../src/contracts/core_snapshot.v1.ts";
import {
  identitySkinPalette,
  validateSkinPalette,
  viewportGeometryFromDecoded,
} from "../../src/services/viewport_mesh.ts";
import { checkContract } from "../../scripts/check_wgsl.mjs";

const readRepoFile = (relative: string): string =>
  readFileSync(fileURLToPath(new URL(`../../${relative}`, import.meta.url)), "utf8").replace(/\r\n/g, "\n");

const CEL_WGSL = "crates/anigo-renderer/shaders/cel_shading.wgsl";
const OUTLINE_WGSL = "crates/anigo-renderer/shaders/inverted_hull.wgsl";
const CEL_GLSL = "crates/anigo-renderer/shaders/webgl2_fallback/cel_vertex.glsl";
const OUTLINE_GLSL = "crates/anigo-renderer/shaders/webgl2_fallback/outline_vertex.glsl";

/** Trecho entre os marcadores do bloco de skinning. */
function markedBlock(source: string, markers: string[]): string {
  const [open, close] = markers;
  const start = source.indexOf(open);
  const end = source.indexOf(close);
  assert.ok(start >= 0, `marcador '${open}' ausente`);
  assert.ok(end > start, `marcador '${close}' ausente ou fora de ordem`);
  return source.slice(start, end + close.length);
}

test("contrato: a paleta de ossos tem 24 matrizes e vem do núcleo", () => {
  const spec = skinning();
  assert.equal(spec.algorithm, "linear_blend_skinning");
  assert.equal(spec.joint_count, 24);
  assert.equal(spec.matrices_per_joint, 16);
  assert.equal(spec.max_influences, 4);
  assert.equal(spec.palette_uniform, "bones");
  assert.equal(spec.palette_bytes, 1536);
  assert.equal(bonePaletteBytes(), 1536);
  assert.equal(bonePaletteFloats(), 384);
  assert.equal(uniformSize("bones"), skinning().palette_bytes);
  assert.equal(RENDER_CONTRACT.uniforms.bones.struct, "BonePalette");
  // a paleta acompanha o layout de vértice (16 dos 72 B são skin)
  assert.equal(spec.joints_location, 4);
  assert.equal(spec.weights_location, 5);
  for (const location of [spec.joints_location, spec.weights_location]) {
    const attribute = RENDER_CONTRACT.vertex_layout.attributes.find(
      (candidate) => candidate.shader_location === location
    );
    assert.ok(attribute, `atributo ${location} precisa existir`);
  }
  assert.deepEqual(spec.block_markers, ["// ANIGO-SKINNING-BEGIN", "// ANIGO-SKINNING-END"]);
  assert.match(spec.unskinned_fallback, /identidade/);
});

test("contrato: cel e outline declaram a paleta como uniform de vértice", () => {
  assert.equal(skinningBinding("cel"), 5);
  assert.equal(skinningBinding("outline"), 2);
  for (const [group, binding] of [
    ["cel", 5],
    ["outline", 2],
  ] as const) {
    const entry = RENDER_CONTRACT.bind_groups
      .find((candidate) => candidate.name === group)!
      .entries.find((candidate) => candidate.binding === binding);
    assert.ok(entry, `${group} sem o binding ${binding}`);
    assert.equal(entry.kind, "uniform");
    assert.deepEqual(entry.stages, ["vertex"]);
    assert.match(entry.declaration, /bones: BonePalette/);
  }
  // Issue #14: a ordem vem do render graph; cel e outline compartilham a paleta.
  assert.deepEqual(renderPassOrder(), ["depth_prepass", "cel", "outline"]);
});

test("shaders WGSL: bloco de skinning idêntico e LBS sem repetição de conta", () => {
  const cel = readRepoFile(CEL_WGSL);
  const outline = readRepoFile(OUTLINE_WGSL);
  const block = markedBlock(cel, skinning().block_markers);
  assert.equal(
    markedBlock(outline, skinning().block_markers),
    block,
    "o bloco de skinning precisa ser byte a byte o mesmo nos dois shaders"
  );

  // a paleta é declarada com o binding do contrato em cada shader
  assert.ok(cel.includes(`@group(0) @binding(${skinningBinding("cel")})\nvar<uniform> bones: BonePalette;`));
  assert.ok(outline.includes(`@group(0) @binding(${skinningBinding("outline")})\nvar<uniform> bones: BonePalette;`));
  assert.ok(cel.includes("struct BonePalette {\n    matrices: array<mat4x4<f32>, 24>,"));
  assert.ok(outline.includes("struct BonePalette {\n    matrices: array<mat4x4<f32>, 24>,"));

  // LBS: pesos normalizados, índice limitado, identidade quando não há peso
  assert.match(block, /return blend \* inv_total;/);
  assert.match(block, /min\(joints\[slot\], PALETTE_JOINT_COUNT - 1u\)/);
  assert.match(block, /if \(total < 1e-5\) \{\n\s+return mat4x4<f32>\(/);
  assert.match(block, /vec4<f32>\(1\.0, 0\.0, 0\.0, 0\.0\)/);
  assert.equal(block.match(/blend \* inv_total/g)?.length, 1, "a normalização aparece uma vez só");

  // posição (w = 1) e direção (w = 0) passam pela mesma matriz, nos dois passes
  for (const source of [cel, outline]) {
    assert.match(source, /skin_palette\(in\.joints, in\.weights\)/);
    assert.match(source, /\(skin \* vec4<f32>\(in\.position, 1\.0\)\)\.xyz/);
    assert.match(source, /\(skin \* vec4<f32>\(in\.normal, 0\.0\)\)\.xyz/);
  }
  // um único struct de entrada de vértice por shader (sem cópia divergente)
  assert.equal(cel.match(/struct VertexInput/g)?.length, 1);
  assert.equal(outline.match(/struct VertexInput/g)?.length, 1);
});

test("checker de WGSL: os shaders de produção conferem com o contrato", () => {
  // O mesmo código que o CI roda (`npm run check:wgsl`) — aqui dentro, para que
  // uma edição de shader quebre o `npm test` mesmo sem CI.
  const contract = JSON.parse(readRepoFile("contracts/fixtures/render_contract_v1.json"));
  const problems = checkContract(contract, (relative: string) => readRepoFile(relative));
  assert.deepEqual(problems, [], `problemas de shader:\n${problems.join("\n")}`);
});

test("fallback WebGL2: a paleta é declarada e aplicada (não zerada)", () => {
  for (const source of [readRepoFile(CEL_GLSL), readRepoFile(OUTLINE_GLSL)]) {
    assert.ok(source.includes(`uniform mat4 u_bones[${skinning().joint_count}];`));
    assert.ok(source.includes("u_bones[bone] * weight"));
    assert.match(source, /return blend \/ total;/);
    assert.match(source, /if \(total < 1e-5\) \{\n    return mat4\(1\.0\);/);
  }
  // o renderer sobe a paleta no caminho GLSL (sem upload, matrizes zeradas)
  const renderer = readRepoFile("src/components/viewport/webgpu_renderer.ts");
  // Issue #14: o upload do cel virou helper (o depth pre-pass usa os mesmos
  // valores); o helper aponta para o programa do cel.
  assert.match(renderer, /private uploadGlCelUniforms\(gl: WebGL2RenderingContext, viewProj: Float32Array \| number\[\]\): void \{\n    const program = this\.glCelProgram!;/);
  assert.match(renderer, /gl\.uniformMatrix4fv\(gl\.getUniformLocation\(program, "u_bones"\)/);
  assert.match(renderer, /gl\.uniformMatrix4fv\(gl\.getUniformLocation\(this\.glOutlineProgram, "u_bones"\)/);
  // o pre-pass reusa o helper: mesma transformação ⇒ z-buffer coerente
  assert.match(renderer, /this\.uploadGlCelUniforms\(gl, viewProj\);/);
});

test("viewport: bind groups usam o binding do contrato e sobem a paleta do núcleo", () => {
  const renderer = readRepoFile("src/components/viewport/webgpu_renderer.ts");
  assert.match(renderer, /const celSkinBinding = skinningBinding\("cel"\);/);
  assert.match(renderer, /const outlineSkinBinding = skinningBinding\("outline"\);/);
  assert.match(renderer, /binding: celSkinBinding, resource: \{ buffer: this\.bonesBuffer \}/);
  assert.match(renderer, /binding: outlineSkinBinding, resource: \{ buffer: this\.bonesBuffer \}/);
  // tamanho do buffer e upload vêm do contrato
  assert.match(renderer, /size: bonePaletteBytes\(\)/);
  assert.match(renderer, /writeBuffer\(this\.bonesBuffer, 0, safe\)/);
  assert.match(renderer, /this\.uploadSkinPalette\(geometry\.skinPalette\)/);
  // modo degradado: paleta neutra (nunca matrizes zeradas)
  assert.match(renderer, /this\.uploadSkinPalette\(identitySkinPalette\(\)\)/);
  // nenhum literal de binding para a paleta
  assert.ok(!/binding: 5,/.test(renderer));
  // a validação da paleta vive no viewport_mesh (uma só)
  assert.match(readRepoFile("src/services/viewport_mesh.ts"), /export function validateSkinPalette/);
});

test("snapshot do núcleo: a paleta chega ao viewport e é validada", () => {
  const fixture = JSON.parse(
    readRepoFile("contracts/fixtures/core_snapshot_v1.json")
  ) as CoreSnapshotWire;
  const decoded = decodeCoreSnapshot(fixture);
  const skin = decoded.geometry!.skin;
  assert.equal(skin.boneCount, skinning().joint_count);
  assert.equal(skin.bindPose, "canonical_rest");
  assert.equal(skin.proportionsBaked, true);
  assert.equal(skin.paletteIsIdentity, true);
  assert.equal(skin.palette.length, bonePaletteFloats());
  for (const value of skin.palette) assert.ok(Number.isFinite(value));

  const geometry = viewportGeometryFromDecoded(decoded.geometry!, decoded.morphWeights);
  assert.equal(geometry.boneCount, skinning().joint_count);
  assert.equal(geometry.skinPalette.length, bonePaletteFloats());
  assert.equal(geometry.paletteIsIdentity, true);
  assert.equal(geometry.skinPalette[0], 1);
  assert.equal(geometry.skinPalette[5], 1);

  // paleta torta é recusada antes de virar uniform buffer
  const shortPalette = new Float32Array(bonePaletteFloats() - 16);
  assert.throws(() => validateSkinPalette(shortPalette, skinning().joint_count), /BAD_SKIN_PALETTE/);
  assert.throws(() => validateSkinPalette(identitySkinPalette(), 12), /BAD_SKIN_PALETTE/);
  const poisoned = identitySkinPalette();
  poisoned[3] = Number.NaN;
  assert.throws(() => validateSkinPalette(poisoned, skinning().joint_count), /BAD_SKIN_PALETTE/);

  // snapshot sem skin: o decodificador falha explícito (nada de paleta vazia)
  const withoutSkin = structuredClone(fixture) as Record<string, any>;
  delete withoutSkin.static_payload.skin;
  assert.throws(() => decodeCoreSnapshot(withoutSkin as CoreSnapshotWire), /skin/);
});

test("paleta neutra é neutra: Σ wᵢ · (I · p) = p", () => {
  const palette = identitySkinPalette();
  const bones = skinning().joint_count;
  assert.equal(palette.length, bones * 16);
  for (let bone = 0; bone < bones; bone++) {
    const base = bone * 16;
    for (let index = 0; index < 16; index++) {
      const expected = index % 5 === 0 ? 1 : 0;
      assert.equal(palette[base + index], expected, `osso ${bone}, componente ${index}`);
    }
  }
  // pesos típicos (1 osso ou dois vizinhos) não mudam a posição
  const weights = [
    [1, 0, 0, 0],
    [0.65, 0.35, 0, 0],
  ];
  for (const matrix of [palette.subarray(0, 16), palette.subarray(16, 32)]) {
    for (const weight of weights) {
      const sum = weight[0] + weight[1] + weight[2] + weight[3];
      const position = [0.12, 1.31, -0.04];
      const skinned = [
        matrix[0] * position[0] + matrix[4] * position[1] + matrix[8] * position[2] + matrix[12],
        matrix[1] * position[0] + matrix[5] * position[1] + matrix[9] * position[2] + matrix[13],
        matrix[2] * position[0] + matrix[6] * position[1] + matrix[10] * position[2] + matrix[14],
      ];
      for (let axis = 0; axis < 3; axis++) {
        assert.ok(Math.abs(skinned[axis] * sum - position[axis] * sum) < 1e-6);
      }
    }
  }
});

test("Rust: a atribuição de ossos é do núcleo, com o layout do contrato", () => {
  const skinningRs = readRepoFile("crates/anigo-core/src/skinning.rs");
  const meshRs = readRepoFile("crates/anigo-core/src/mesh.rs");
  const snapshotRs = readRepoFile("crates/anigo-core/src/snapshot.rs");
  const boneSyncRs = readRepoFile("crates/anigo-core/src/bone_sync.rs");
  const headlessRs = readRepoFile("crates/anigo-renderer/src/headless.rs");
  const uniformsRs = readRepoFile("crates/anigo-renderer/src/uniforms.rs");

  // 24 ossos canônicos e no máximo 4 influências (o layout é de 72 B)
  assert.match(boneSyncRs, /pub const CANONICAL_JOINT_COUNT: usize = 24;/);
  assert.match(skinningRs, /pub const MAX_INFLUENCES: usize = 4;/);
  assert.match(skinningRs, /pub fn assign_canonical_skin_weights\(/);
  assert.match(skinningRs, /pub fn skin_for_vertex\(/);
  // a malha canônica nasce atribuída (os 16 B de skin significam algo)
  assert.match(meshRs, /crate::skinning::assign_legacy_skin_weights\(&mut mesh\);/);
  // a paleta canônica é do núcleo e viaja no snapshot
  assert.match(snapshotRs, /pub struct SkinPayload/);
  assert.match(snapshotRs, /pub fn canonical_base\(\)/);
  assert.match(snapshotRs, /skin: SkinPayload::canonical_base\(\)/);
  // e a cena carrega a mesma paleta (headless e viewport deformam igual)
  assert.match(readRepoFile("crates/anigo-core/src/scene.rs"), /pub skin: SkinPayload/);
  // tempo do renderer: paleta como uniform de 1536 B
  assert.match(uniformsRs, /pub const MAX_PALETTE_JOINTS: usize = 24;/);
  assert.match(uniformsRs, /pub struct BonePaletteUniform/);
  assert.match(headlessRs, /contract::skinning_binding\("cel"\)/);
  assert.match(headlessRs, /contract::skinning_binding\("outline"\)/);
  assert.match(headlessRs, /bone_palette_uniform\(scene\)/);
});
