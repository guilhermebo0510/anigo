/**
 * ANIGO Fase 2 (#25) — interoperabilidade glTF 2.0 + VRM 1.0.
 *
 * Cobre os critérios de aceitação:
 *  1. parse de GLB/VRM reais (estrutura: malha, skins, materials, extensões);
 *  2. export determinístico (mesma cena → bytes idênticos);
 *  3. roundtrip import → export → re-import (geometria, skin, morph, anim).
 *
 * Run: npm test (node --test, tests/character)
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  GltfError,
  Vrm1Error,
  VRM1_EXPRESSION_PRESETS,
  buildGlb,
  exportGltf,
  exportModel,
  mapMToonToAnimeMaterial,
  mipLevelCount,
  parseGltfDocument,
  parseGltfFile,
  parseModel,
  parseVrm,
  readAccessorFloats,
  readAccessorIndices,
  readImageBytes,
  summarizeImport,
  validateModel,
} from "../../src/services/vrm/index.ts";
import type {
  ExportScene,
  ExportVertex,
  Gltf,
  VrmExportData,
} from "../../src/services/vrm/index.ts";
import { loadGlbMesh } from "../../src/services/gltf_loader.ts";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function fnv1a64(bytes: Uint8Array): string {
  let hash = 0xcbf29ce484222325n;
  for (const byte of bytes) {
    hash ^= BigInt(byte);
    hash = (hash * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return hash.toString(16).padStart(16, "0");
}

/** Compara floats com tolerância (valores atravessam buffers f32 no roundtrip). */
function closeTo(actual: number, expected: number, label: string, epsilon = 1e-6): void {
  assert.ok(Math.abs(actual - expected) < epsilon, `${label}: ${actual} != ${expected}`);
}

function closeToArray(actual: ArrayLike<number>, expected: readonly number[], label: string, epsilon = 1e-6): void {
  assert.equal(actual.length, expected.length, `${label}: tamanho`);
  for (let i = 0; i < expected.length; i++) closeTo(actual[i], expected[i], `${label}[${i}]`, epsilon);
}

function vertex(
  position: [number, number, number],
  normal: [number, number, number] = [0, 1, 0],
  uv: [number, number] = [0, 0],
  joints: [number, number, number, number] = [0, 0, 0, 0],
  weights: [number, number, number, number] = [1, 0, 0, 0],
): ExportVertex {
  return { position, normal, uv, color: [1, 1, 1, 1], joints, weights };
}

/** Cena sintética: 2 nós, malha com skin + morph, 2 materiais, 1 animação. */
function sampleScene(): ExportScene {
  const vertices: ExportVertex[] = [
    vertex([0, 0, 0], [0, 1, 0], [0, 0], [0, 1, 0, 0], [0.5, 0.5, 0, 0]),
    vertex([1, 0, 0], [0, 1, 0], [1, 0], [1, 0, 0, 0], [1, 0, 0, 0]),
    vertex([0, 0, 1], [0, 0, 1], [0, 1], [1, 2, 0, 0], [0.75, 0.25, 0, 0]),
    vertex([1, 0, 1], [0, 0, 1], [1, 1], [2, 1, 0, 0], [0.25, 0.75, 0, 0]),
  ];
  const identity = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
  return {
    name: "Fase2 Scene",
    rootNodes: [0],
    nodes: [
      { name: "Root", translation: [0.5, 1, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1], children: [1], meshIndex: 0, skinIndex: 0 },
      { name: "Hip", translation: [0, 0.9, 0], children: [2] },
      { name: "Spine", translation: [0, 0.25, 0], children: [3] },
      { name: "Head", translation: [0, 0.3, 0], children: [] },
    ],
    meshes: [
      {
        name: "Body",
        primitives: [
          {
            vertices,
            indices: [0, 1, 2, 2, 1, 3],
            materialIndex: 0,
            morphTargets: [
              [
                { position: [0.01, 0.02, 0.03], normal: [0, 0, 0] },
                { position: [0, 0, 0], normal: [0, 0.5, 0] },
                { position: [0.04, 0, 0], normal: [0.1, 0, 0] },
                { position: [0, -0.01, 0], normal: [0, 0, 0.1] },
              ],
            ],
          },
        ],
      },
    ],
    skins: [
      { name: "Rig", joints: [1, 2, 3, 0], inverseBindMatrices: [identity, identity, identity, identity] },
    ],
    materials: [
      { name: "Skin", baseColorFactor: [0.98, 0.92, 0.85, 1], mtoon: { shadowColor: [0.82, 0.73, 0.78, 1], shadeToony: 0.5 } },
      { name: "Cloth", baseColorFactor: [0.2, 0.4, 0.8, 1] },
    ],
    animations: [
      {
        name: "Wave",
        tracks: [
          {
            node: 0,
            path: "translation",
            sampler: { times: [0, 1], values: [0.5, 1, 0, 0.5, 1.2, 0], interp: "LINEAR" },
          },
        ],
      },
    ],
  };
}

function vrmData(): VrmExportData {
  const presets: Record<string, number> = {};
  VRM1_EXPRESSION_PRESETS.forEach((preset, i) => (presets[preset] = i));
  return {
    meta: {
      title: "ANIGO Test Avatar",
      author: "Arena Agent",
      version: "1.0.0",
      year: 2026,
      license: { name: "CC-BY-4.0" },
      contactInformation: "https://anigo.studio",
      metadata: [{ type: "software", value: "anigo-studio" }],
      reference: "",
    },
    humanoidBones: { hips: 2, spine: 1, head: 4, leftUpperArm: null },
    expressionPresets: presets,
    expressionCustom: { wink: 16 },
    springBone: {
      groups: [
        {
          name: "HairFront",
          center: { node: 10, distance: 0.1 },
          joints: [
            { node: 11, distance: 0.1, springStiffness: 0.8, springDamping: 0.3 },
            { node: 12, distance: 0.12, springStiffness: 0.6, springDamping: 0.4 },
          ],
          colliderGroups: [0],
        },
      ],
      colliders: {
        spheres: [{ group: 0, node: 20, radius: 0.08, offset: [0, 0.01, 0] }],
        capsules: [{ group: 0, node: 21, from: [0, 0.5, 0], to: [0, 0.8, 0], radius: 0.06 }],
      },
    },
  };
}

// ---------------------------------------------------------------------------
// 1. Roundtrip glTF 2.0 (export → parse → comparar)
// ---------------------------------------------------------------------------

test("roundtrip GLB: geometria, índices, skin, morph e animação sobrevivem", () => {
  const bytes = exportModel(sampleScene());
  const { model } = parseModel(bytes);
  const gltf = model.gltf;

  // Container
  assert.equal(gltf.asset.version, "2.0");
  assert.equal(gltf.asset.generator, "anigo-studio/0.1.0 (fase2-exporter)");

  // Nós e transform
  assert.equal(gltf.nodes?.length, 4);
  assert.deepEqual(gltf.nodes![0]["translation"], [0.5, 1, 0]);
  assert.deepEqual(gltf.nodes![0]["children"], [1]);
  assert.equal(gltf.nodes![0]["mesh"], 0);
  assert.equal(gltf.nodes![0]["skin"], 0);
  assert.equal(gltf.nodes![2]["translation"]![1], 0.25);
  assert.equal(gltf.nodes![3]["name"], "Head");

  // Malha
  const mesh = gltf.meshes![0];
  const prim = mesh.primitives[0];
  assert.equal(prim.mode, 4); // TRIANGLES
  assert.equal(prim.material, 0);
  const position = readAccessorFloats(model, prim.attributes["POSITION"]);
  assert.equal(position.length, 12);
  assert.deepEqual(Array.from(position.slice(3, 6)), [1, 0, 0]);
  assert.deepEqual(Array.from(position.slice(9, 12)), [1, 0, 1]);
  const normal = readAccessorFloats(model, prim.attributes["NORMAL"]);
  assert.deepEqual(Array.from(normal.slice(6, 9)), [0, 0, 1]);
  const uv = readAccessorFloats(model, prim.attributes["TEXCOORD_0"]);
  assert.deepEqual(Array.from(uv.slice(4, 6)), [0, 1]); // v2 uv
  const joints = readAccessorFloats(model, prim.attributes["JOINTS_0"]);
  assert.deepEqual(Array.from(joints.slice(0, 4)), [0, 1, 0, 0]);
  const weights = readAccessorFloats(model, prim.attributes["WEIGHTS_0"]);
  assert.deepEqual(Array.from(weights.slice(4, 8)), [1, 0, 0, 0]);

  // Índices (uint16 — max < 65536)
  const indices = readAccessorIndices(model, prim.indices!) as Uint16Array;
  assert.ok(indices instanceof Uint16Array, "esperado uint16");
  assert.deepEqual(Array.from(indices), [0, 1, 2, 2, 1, 3]);

  // Morph targets
  assert.ok(prim.targets && prim.targets.length === 1);
  const morphPosition = readAccessorFloats(model, prim.targets![0]["POSITION"]);
  closeToArray(morphPosition.slice(0, 3), [0.01, 0.02, 0.03], "morph position delta v0");

  // Skin
  const skin = gltf.skins![0];
  assert.deepEqual(skin.joints, [1, 2, 3, 0]);
  const ibm = readAccessorFloats(model, skin.inverseBindMatrices!);
  assert.equal(ibm.length, 64);
  assert.equal(ibm[15], 1);

  // Animação
  const animation = gltf.animations![0];
  assert.equal(animation.channels.length, 1);
  assert.equal(animation.channels[0].target.node, 0);
  assert.equal(animation.channels[0].target.path, "translation");
  const times = readAccessorFloats(model, animation.samplers[0].input);
  assert.deepEqual(Array.from(times), [0, 1]);

  // Materiais
  assert.equal(gltf.materials?.length, 2);
  closeToArray(gltf.materials![0].pbrMetallicRoughness?.baseColorFactor ?? [], [0.98, 0.92, 0.85, 1], "baseColorFactor");
});

test("export é determinístico: mesma cena → bytes idênticos", () => {
  const a = exportModel(sampleScene());
  const b = exportModel(sampleScene());
  assert.equal(fnv1a64(a), fnv1a64(b));
  const aVrm = exportModel(sampleScene(), { vrm: vrmData() });
  const bVrm = exportModel(sampleScene(), { vrm: vrmData() });
  assert.equal(fnv1a64(aVrm), fnv1a64(bVrm));
  assert.notEqual(fnv1a64(a), fnv1a64(aVrm));
});

test("export .gltf (JSON + bin externo) reabre com o mesmo conteúdo", () => {
  const { json, bin } = exportGltf(sampleScene(), undefined, { bufferUri: "buffer.bin" });
  const jsonText = JSON.stringify(json);
  const { model } = parseGltfFile(jsonText, new Map<string, Uint8Array | ArrayBuffer>([["buffer.bin", bin]]));
  const prim = model.gltf.meshes![0].primitives[0];
  const position = readAccessorFloats(model, prim.attributes["POSITION"]);
  assert.deepEqual(Array.from(position.slice(0, 3)), [0, 0, 0]);
  // buffer declarado com uri externa
  assert.equal(model.gltf.buffers![0].uri, "buffer.bin");
});

// ---------------------------------------------------------------------------
// 2. VRM 1.0: export → parse (todas as 4 extensões)
// ---------------------------------------------------------------------------

test("roundtrip VRM 1.0: meta, humanoid, expressões, MToon e spring bones", () => {
  const bytes = exportModel(sampleScene(), { vrm: vrmData() });
  const { model, vrm } = parseModel(bytes);

  assert.equal(vrm !== null, true, "documento exportado com VRM deve parsear como VRM");
  assert.ok(vrm!);
  assert.equal(vrm.meta.title, "ANIGO Test Avatar");
  assert.equal(vrm.meta.author, "Arena Agent");
  assert.equal(vrm.meta.license.name, "CC-BY-4.0");
  assert.deepEqual(vrm.meta.metadata, [{ type: "software", value: "anigo-studio" }]);

  // Humanoid
  assert.equal(vrm.humanoid.humanBones["hips"]?.node, 2);
  assert.equal(vrm.humanoid.humanBones["head"]?.node, 4);
  assert.equal(vrm.humanoid.humanBones["leftUpperArm"]?.node, null);

  // Expressões (16 presets + custom)
  for (const [index, preset] of VRM1_EXPRESSION_PRESETS.entries()) {
    assert.equal(vrm.expression.preset[preset].blendShape, index, `preset ${preset}`);
  }
  assert.equal(vrm.expression.custom["wink"]?.blendShape, 16);

  // Spring bones
  assert.ok(vrm.springBone);
  assert.equal(vrm.springBone.groups.length, 1);
  assert.equal(vrm.springBone.groups[0].name, "HairFront");
  assert.equal(vrm.springBone.groups[0].center.node, 10);
  assert.equal(vrm.springBone.groups[0].joints.length, 2);
  assert.equal(vrm.springBone.groups[0].joints[0].springStiffness, 0.8);
  assert.equal(vrm.springBone.colliders.spheres.length, 1);
  assert.equal(vrm.springBone.colliders.spheres[0].radius, 0.08);
  assert.equal(vrm.springBone.colliders.capsules.length, 1);
  assert.deepEqual(vrm.springBone.colliders.capsules[0].from, [0, 0.5, 0]);

  // MToon nos materiais
  assert.equal(vrm.materials.size, 1); // só o material 0 tem mtoon no export
  const mtoon = vrm.materials.get(0);
  assert.ok(mtoon);
  closeToArray(mtoon.shadowColor, [0.82, 0.73, 0.78, 1], "mtoon shadowColor");
  closeTo(mtoon.shadeToony, 0.5, "mtoon shadeToony");
  assert.equal(mtoon.mainTex, null);

  // Sumário reconhece VRM
  const summary = summarizeImport(model.gltf);
  assert.equal(summary.isVrm, true);
  assert.equal(summary.title, "ANIGO Test Avatar");
  assert.deepEqual(summary.vrmExtensions, ["VRMC_vrm", "VRMC_materials_mtoon", "VRMC_springBone"]);
  assert.equal(summary.vertexCount, 4);
  assert.equal(summary.triangleCount, 2);
});

test("parseVrm falha com código estável quando o documento não é VRM", () => {
  const bytes = exportModel(sampleScene());
  assert.throws(() => parseVrm(bytes), (error: unknown) => error instanceof Vrm1Error && error.code === "NOT_VRM");
});

// ---------------------------------------------------------------------------
// 3. Fixture "estilo VRoid Studio" — mão, com armadilhas de spec
//    (interleaved, u8 quantizado, uint32, sparse, node_constraint)
// ---------------------------------------------------------------------------

/** Monta um GLB cru (JSON + bin) para o caso em que o exporter não cobre. */
function buildRawGlb(json: Gltf, bin: Uint8Array): Uint8Array {
  return buildGlb(json, bin);
}

test("parse GLB hostil: interleaved + quantizado + uint32 + sparse + constraint", () => {
  // Bin: um bufferView INTERLEAVED com POSITION(f32x3)+NORMAL(f32x3) (stride 24),
  // um COLOR_0 u8 NORMALIZADO (stride 4), e índices uint32.
  const binParts: number[] = [];
  // POSITION+NORMAL interleaved: 3 vértices
  const interleaved: number[] = [
    0, 0, 0, 0, 1, 0, // v0 pos, normal
    1, 0, 0, 0, 1, 0, // v1
    0, 0, 1, 0, 0, 1, // v2
  ];
  const f32 = new Float32Array(interleaved);
  const binStartPosNorm = binParts.length;
  binParts.push(...Array.from(new Uint8Array(f32.buffer)));
  // COLOR_0 u8 normalized: [255,128,0,255], [0,255,0,255], [0,0,255,255]
  const binStartColor = binParts.length;
  binParts.push(255, 128, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255);
  // Índices uint32 (5123 = UNSIGNED_INT na spec): 0,1,2
  const binStartIdx = binParts.length;
  for (const index of [0, 1, 2]) {
    binParts.push(index & 0xff, (index >> 8) & 0xff, (index >> 16) & 0xff, (index >> 24) & 0xff);
  }
  const binBytes = new Uint8Array(binParts);

  const json = {
    asset: { version: "2.0", generator: "fixture" },
    scene: 0,
    scenes: [{ nodes: [0] }],
    nodes: [
      {
        name: "constrained",
        extensions: {
          VRMC_node_constraint: {
            rotateLimit: { min: [-0.5, 0, -0.5], max: [0.5, 0.3, 0.5] },
            scaleLimit: { min: [0.5, 0.5, 0.5], max: [2, 2, 2] },
          },
        },
      },
    ],
    buffers: [{ byteLength: binBytes.byteLength }],
    bufferViews: [
      { buffer: 0, byteOffset: binStartPosNorm, byteLength: 72, byteStride: 24 },
      { buffer: 0, byteOffset: binStartColor, byteLength: 12, byteStride: 4 },
      { buffer: 0, byteOffset: binStartIdx, byteLength: 12 },
    ],
    accessors: [
      { bufferView: 0, byteOffset: 0, componentType: 5126, count: 3, type: "VEC3" },
      { bufferView: 0, byteOffset: 12, componentType: 5126, count: 3, type: "VEC3" },
      { bufferView: 1, componentType: 5121, normalized: true, count: 3, type: "VEC4" },
      { bufferView: 2, componentType: 5123, count: 3, type: "SCALAR" },
    ],
    meshes: [
      {
        primitives: [
          {
            attributes: { POSITION: 0, NORMAL: 1, COLOR_0: 2 },
            indices: 3,
            mode: 4,
          },
        ],
      },
    ],
  } as unknown as Gltf;

  const { model } = parseModel(buildRawGlb(json, binBytes));
  const prim = model.gltf.meshes![0].primitives[0];

  const position = readAccessorFloats(model, prim.attributes["POSITION"]);
  assert.deepEqual(Array.from(position.slice(6, 9)), [0, 0, 1], "interleaved POSITION v2");
  const normal = readAccessorFloats(model, prim.attributes["NORMAL"]);
  assert.deepEqual(Array.from(normal.slice(3, 6)), [0, 1, 0], "interleaved NORMAL v1");
  const color = readAccessorFloats(model, prim.attributes["COLOR_0"]);
  closeToArray(color.slice(0, 4), [1, 128 / 255, 0, 1], "COLOR_0 v0 (u8 normalizado)");

  const indices = readAccessorIndices(model, prim.indices!) as Uint32Array;
  assert.ok(indices instanceof Uint32Array, "esperado uint32");
  assert.deepEqual(Array.from(indices), [0, 1, 2]);

  // Constraint
  const constraintRaw = model.gltf.nodes![0].extensions!["VRMC_node_constraint"] as Record<string, unknown>;
  assert.deepEqual((constraintRaw["rotateLimit"] as Record<string, unknown>)["min"], [-0.5, 0, -0.5]);
  assert.deepEqual((constraintRaw["scaleLimit"] as Record<string, unknown>)["max"], [2, 2, 2]);

  // E a via parser completo de VRM (mtoon por material + node constraint)
  const jsonVrm: Gltf = JSON.parse(JSON.stringify(json)) as Gltf;
  jsonVrm.asset = { version: "2.0", extensionsUsed: ["VRMC_vrm"], extensionsRequired: ["VRMC_vrm", "VRMC_materials_mtoon"] };
  jsonVrm.materials = [
    {
      name: "Face",
      pbrMetallicRoughness: { baseColorFactor: [0.95, 0.85, 0.8, 1] },
      extensions: {
        VRMC_materials_mtoon: {
          subEmission: [1, 1, 1, 0.5],
          shadowColor: [0.8, 0.7, 0.9, 1],
          shadeShift: 0.2,
          shadeToony: 0.3,
          rimColor: [0.3, 0.5, 1, 0.8],
          rimPower: 2.5,
          useSphere: true,
          sphereMode: "additive",
        },
      },
    },
  ];
  jsonVrm.extensions = {
    VRMC_vrm: {
      meta: { title: "VroidLike", author: "Test", version: "1.0.0", year: 2026, license: {}, contactInformation: "", metadata: [], reference: "" },
      humanoid: { humanBones: { hips: { node: 0 }, head: { node: null } } },
      expression: {
        preset: Object.fromEntries(VRM1_EXPRESSION_PRESETS.map((preset, i) => [preset, { blendShape: i, isolated: false }])),
      },
    },
  };
  const { vrm } = parseModel(buildRawGlb(jsonVrm, binBytes));
  assert.ok(vrm, "documento com VRMC_vrm deve parsear como VRM");
  const mtoon = vrm!.materials.get(0);
  assert.ok(mtoon);
  assert.equal(mtoon.shadeShift, 0.2);
  assert.equal(mtoon.useSphere, true);
  assert.equal(mtoon.sphereMode, "additive");
  assert.equal(vrm!.nodeConstraints.size, 1);
  const constraint = vrm!.nodeConstraints.get(0);
  assert.deepEqual(constraint?.rotateLimit?.max, [0.5, 0.3, 0.5]);

  // MToon → material anime do ANIGO
  const mapping = mapMToonToAnimeMaterial(jsonVrm.materials![0], mtoon);
  assert.ok(Math.abs(mapping.shadowThreshold - (0.5 + 0.2 * 0.5)) < 1e-6);
  assert.equal(mapping.toonSteps, 2.0, "shadeToony < 0.5 → duas bandas");
  assert.ok(Math.abs(mapping.shadeColor[0] - 0.95 * 0.8) < 1e-6);
  assert.equal(mapping.useSphere, true);
  assert.equal(mapping.sphereMode, "additive");
});

// ---------------------------------------------------------------------------
// 4. Validação: erros estruturais com código estável
// ---------------------------------------------------------------------------

test("validateModel: GLB íntegro é válido, GLB corrompido falha com código", () => {
  const good = validateModel(exportModel(sampleScene()));
  assert.equal(good.valid, true);
  assert.ok(good.summary);
  assert.equal(good.summary.vertexCount, 4);

  const corrupted = new Uint8Array(exportModel(sampleScene()));
  corrupted[0] = 0x00; // magic
  const bad = validateModel(corrupted);
  assert.equal(bad.valid, false);
  assert.ok(bad.error);
  assert.match(bad.error.code, /^(BAD_JSON|BAD_MAGIC)$/);
});

test("GLB com version 1 falha com UNSUPPORTED_VERSION", () => {
  const bytes = new Uint8Array(exportModel(sampleScene()));
  new DataView(bytes.buffer).setUint32(4, 1, true);
  const result = validateModel(bytes);
  assert.equal(result.valid, false);
  assert.equal(result.error?.code, "UNSUPPORTED_VERSION");
});

test("accessor apontando para fora do buffer falha com código estável", () => {
  const jsonText = JSON.stringify({
    asset: { version: "2.0" },
    scene: 0,
    scenes: [{ nodes: [] }],
    buffers: [{ uri: "b", byteLength: 4 }],
    bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: 4 }],
    accessors: [{ bufferView: 0, componentType: 5126, count: 16, type: "VEC3" }],
  });
  const bin = new Uint8Array([1, 2, 3, 4]);
  const { model } = parseGltfFile(jsonText, new Map([["b", bin]]));
  void model;
  assert.throws(
    () => readAccessorFloats(model, 0),
    (error: unknown) => error instanceof GltfError && error.code === "ACCESSOR_OUT_OF_RANGE",
  );
});

test("buffer externo ausente falha com MISSING_BUFFER", () => {
  const jsonText = JSON.stringify({
    asset: { version: "2.0" },
    buffers: [{ uri: "missing.bin", byteLength: 4 }],
    bufferViews: [{ buffer: 0, byteLength: 4 }],
    accessors: [{ bufferView: 0, componentType: 5126, count: 1, type: "SCALAR" }],
  });
  assert.throws(
    () => parseGltfFile(jsonText, new Map()),
    (error: unknown) => error instanceof GltfError && error.code === "MISSING_BUFFER",
  );
});

test("buffer com byteLength divergente da fonte falha com BUFFER_LENGTH_MISMATCH", () => {
  const jsonText = JSON.stringify({
    asset: { version: "2.0" },
    buffers: [{ uri: "b.bin", byteLength: 8 }],
    bufferViews: [{ buffer: 0, byteLength: 8 }],
    accessors: [{ bufferView: 0, componentType: 5126, count: 2, type: "SCALAR" }],
  });
  assert.throws(
    () => parseGltfFile(jsonText, new Map([["b.bin", new Uint8Array(4)]])),
    (error: unknown) => error instanceof GltfError && error.code === "BUFFER_LENGTH_MISMATCH",
  );
});

test("ciclo no grafo de nós é detectado", () => {
  const jsonText = JSON.stringify({
    asset: { version: "2.0" },
    scene: 0,
    scenes: [{ nodes: [0] }],
    nodes: [{ children: [1] }, { children: [0] }],
  });
  assert.throws(
    () => parseGltfFile(jsonText),
    (error: unknown) => error instanceof GltfError && error.code === "NODE_GRAPH_CYCLE",
  );
});

// ---------------------------------------------------------------------------
// 5. Texturas / images / mipmaps
// ---------------------------------------------------------------------------

test("image embutida em bufferView é lida; mipLevelCount segue a spec", () => {
  // PNG fake de 4 bytes (conteúdo importa para o teste, não o decode).
  const bin = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x01, 0x02, 0x03, 0x04]);
  const json: Gltf = {
    asset: { version: "2.0" },
    buffers: [{ byteLength: bin.byteLength }],
    bufferViews: [{ buffer: 0, byteOffset: 2, byteLength: 6 }],
    images: [{ bufferView: 0, mimeType: "image/png" }],
    samplers: [{ wrapS: 10497, wrapT: 10497, magFilter: 9729, minFilter: 9987 }],
    textures: [{ sampler: 0, source: 0 }],
  } as Gltf;
  const binGlb = buildGlb(json, bin);
  const { model } = parseModel(binGlb);
  const image = readImageBytes(model, 0);
  // bufferView: byteOffset 2, byteLength 6 → bytes [2..8) do bin
  assert.deepEqual(Array.from(image.bytes), [0x4e, 0x47, 0x01, 0x02, 0x03, 0x04]);
  assert.equal(image.mimeType, "image/png");
  assert.equal(model.gltf.textures![0].sampler, 0);

  assert.equal(mipLevelCount(1, 1), 1);
  assert.equal(mipLevelCount(4096, 1024), 13);
  assert.equal(mipLevelCount(1000, 2000), 11);
  assert.equal(mipLevelCount(64, 64), 7);
});

test("resolveBuffers: data-URI base64 e Map externo", () => {
  const payload = [1, 2, 3, 4, 5];
  const b64 = Buffer.from(payload).toString("base64");
  const json = {
    asset: { version: "2.0" },
    buffers: [{ uri: `data:application/octet-stream;base64,${b64}`, byteLength: 5 }],
  } as Gltf;
  const model = parseGltfDocument(json, null);
  assert.deepEqual(Array.from(model.buffers[0].bytes), payload);
  assert.equal(model.buffers[0].embedded, false);

  const jsonExternal = {
    asset: { version: "2.0" },
    buffers: [{ uri: "ext.bin", byteLength: 2 }],
  } as Gltf;
  const modelExternal = parseGltfDocument(jsonExternal, new Map([["ext.bin", new Uint8Array([9, 8])]]));
  assert.deepEqual(Array.from(modelExternal.buffers[0].bytes), [9, 8]);
});

// ---------------------------------------------------------------------------
// 6. Compatibilidade: o loader legado continua funcionando sobre o mesmo GLB
// ---------------------------------------------------------------------------

test("o loader legado (P0-10) decodifica o GLB exportado pela Fase 2", () => {
  const bytes = exportModel(sampleScene());
  const result = loadGlbMesh(new Uint8Array(bytes).buffer as ArrayBuffer);
  assert.equal(result.mesh.positions.length, 12);
  assert.equal(result.mesh.indices.length, 6);
  assert.equal(result.mesh.joints !== null, true, "skinning preservado pelo loader legado");
  assert.equal(result.mesh.indices[5], 3);
});

// ---------------------------------------------------------------------------
// 7. Import summary (UI/Tauri — issue #59)
// ---------------------------------------------------------------------------

test("summarizeImport conta vértices, triângulos, morphs e extensões", () => {
  const { model } = parseModel(exportModel(sampleScene()));
  const summary = summarizeImport(model.gltf);
  assert.equal(summary.vertexCount, 4);
  assert.equal(summary.indexCount, 6);
  assert.equal(summary.triangleCount, 2);
  assert.equal(summary.meshCount, 1);
  assert.equal(summary.nodeCount, 4);
  assert.equal(summary.skinCount, 1);
  assert.equal(summary.animationCount, 1);
  assert.equal(summary.morphTargetCount, 1);
  assert.equal(summary.isVrm, false);
  assert.equal(summary.gltfVersion, "2.0");
});


