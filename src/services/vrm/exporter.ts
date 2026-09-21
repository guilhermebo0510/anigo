/**
 * ANIGO — exportador determinístico glTF 2.0 / GLB / VRM 1.0 (Fase 2, #25).
 *
 * "Determinístico" aqui tem significado forte (critério de aceitação #25.3):
 * a MESMA cena de entrada produz BYTES idênticos em exportações sucessivas —
 * ordenação canônica de chaves, quantização f32 de todos os números, packing
 * canônico de buffers (uma bufferView por accessor, padding 4 B), e GLB com
 * padding fixo (JSON com 0x20, BIN com 0x00).
 *
 * Suporta:
 *  - `.glb` e `.gltf` (JSON + buffer binário externo para o caller gravar);
 *  - multi-malha, multi-primitiva, skinning (joints/weights + inverseBind),
 *    morph targets, materiais PBR e `.vrm` (VRMC_vrm + VRMC_materials_mtoon,
 *    com VRMC_springBone opcional);
 *  - índices uint16 quando cabem (max < 65536), uint32 caso contrário —
 *    regra única e estável.
 */

import { GLB_CHUNK_BIN, GLB_CHUNK_JSON, GLB_MAGIC, GLB_VERSION, GLTF_COMPONENT_TYPES, GLTF_PRIMITIVE_MODES } from "./gltf2";
import type { Gltf } from "./gltf2";
import type { Vrm1Meta, Vrm1SpringBone } from "./vrm1";
import { VRM1_EXPRESSION_PRESETS } from "./vrm1";

// ---------------------------------------------------------------------------
// Modelo de exportação (input do ANIGO: cena normalizada)
// ---------------------------------------------------------------------------

export interface ExportVertex {
  position: [number, number, number];
  normal: [number, number, number];
  uv: [number, number];
  color: [number, number, number, number];
  joints: [number, number, number, number];
  weights: [number, number, number, number];
}

export interface ExportMorphDelta {
  position: [number, number, number];
  normal: [number, number, number];
}

export interface ExportPrimitive {
  vertices: ExportVertex[];
  indices: number[];
  materialIndex: number;
  /** Morph targets: um array de deltas (posição+normal) por target. */
  morphTargets?: ExportMorphDelta[][];
}

export interface ExportMesh {
  name?: string;
  primitives: ExportPrimitive[];
}

export interface ExportMaterial {
  name?: string;
  /** baseColorFactor (sRGB 0..1, 4 componentes). */
  baseColorFactor: [number, number, number, number];
  metallicFactor?: number;
  roughnessFactor?: number;
  emissiveFactor?: [number, number, number];
  alphaMode?: "OPAQUE" | "MASK" | "BLEND";
  doubleSided?: boolean;
  /** Quando presente (export VRM), o material sai com VRMC_materials_mtoon. */
  mtoon?: {
    shadowColor?: [number, number, number, number];
    shadeShift?: number;
    shadeToony?: number;
    rimColor?: [number, number, number, number];
    rimPower?: number;
    rimLight?: boolean;
    specularColor?: [number, number, number, number];
    specularPower?: number;
    subEmission?: [number, number, number, number];
  };
}

export type Mat4 = number[]; // column-major, 16 floats

export interface ExportNode {
  name?: string;
  translation?: [number, number, number];
  /** quat xyzw (identidade [0,0,0,1]). */
  rotation?: [number, number, number, number];
  scale?: [number, number, number];
  children?: number[];
  meshIndex?: number;
  skinIndex?: number;
  morphWeights?: number[];
}

export interface ExportSkin {
  name?: string;
  joints: number[];
  inverseBindMatrices: Mat4[];
  skeleton?: number;
}

export interface ExportAnimationSampler {
  times: number[];
  values: number[];
  interp: "LINEAR" | "STEP";
}

export interface ExportAnimation {
  name?: string;
  /** node → track. path: translation | rotation | scale | weights. */
  tracks: Array<{ node: number; path: "translation" | "rotation" | "scale" | "weights"; sampler: ExportAnimationSampler }>;
}

export interface ExportScene {
  name?: string;
  rootNodes: number[];
  nodes: ExportNode[];
  meshes: ExportMesh[];
  skins: ExportSkin[];
  materials: ExportMaterial[];
  animations: ExportAnimation[];
}

export interface VrmExportData {
  meta: Vrm1Meta;
  /** bone name → node index (null = mapeamento ausente). */
  humanoidBones: Record<string, number | null>;
  /** preset name → morph target index (os 16 presets são obrigatórios). */
  expressionPresets: Record<string, number>;
  expressionCustom?: Record<string, number>;
  springBone?: Vrm1SpringBone;
}

// ---------------------------------------------------------------------------
// Serialização canônica
// ---------------------------------------------------------------------------

/** Quantiza para f32 e remove `-0` (mesma forma em qualquer run). */
function stableNumber(value: number): number {
  const finite = Number.isFinite(value) ? value : 0;
  const rounded = Math.fround(finite);
  return Object.is(rounded, -0) ? 0 : rounded;
}

function stableArray(values: readonly number[]): number[] {
  return values.map(stableNumber);
}

// ---------------------------------------------------------------------------
// Packing do buffer binário
// ---------------------------------------------------------------------------

class BinWriter {
  private chunks: Uint8Array[] = [];
  private length = 0;

  get byteLength(): number {
    return this.length;
  }

  private push(bytes: Uint8Array): void {
    this.chunks.push(bytes);
    this.length += bytes.length;
    const remainder = this.length % 4;
    if (remainder !== 0) {
      const pad = new Uint8Array(4 - remainder);
      this.chunks.push(pad);
      this.length += pad.length;
    }
  }

  /** Escreve floats f32 (quantizados) e devolve o offset. */
  writeFloats(values: readonly number[]): number {
    const offset = this.length;
    const f32 = new Float32Array(values.length);
    for (let i = 0; i < values.length; i++) f32[i] = stableNumber(values[i]);
    this.push(new Uint8Array(f32.buffer));
    return offset;
  }

  writeUint16(values: readonly number[]): number {
    const offset = this.length;
    const u16 = new Uint16Array(values.length);
    for (let i = 0; i < values.length; i++) u16[i] = Math.max(0, Math.min(65535, Math.trunc(values[i])));
    this.push(new Uint8Array(u16.buffer));
    return offset;
  }

  writeUint32(values: readonly number[]): number {
    const offset = this.length;
    const u32 = new Uint32Array(values.length);
    for (let i = 0; i < values.length; i++) u32[i] = Math.max(0, Math.trunc(values[i])) >>> 0;
    this.push(new Uint8Array(u32.buffer));
    return offset;
  }

  finish(): Uint8Array {
    const out = new Uint8Array(this.length);
    let cursor = 0;
    for (const chunk of this.chunks) {
      out.set(chunk, cursor);
      cursor += chunk.length;
    }
    return out;
  }
}

// ---------------------------------------------------------------------------
// Exportador
// ---------------------------------------------------------------------------

export interface ExportResult {
  /** JSON glTF pronto (ordem de chaves canônica de construção). */
  json: Gltf;
  /** Bytes do buffer binário (BIN do GLB ou arquivo externo). */
  bin: Uint8Array;
  /** `true` quando a saída carrega as extensões VRM 1.0. */
  isVrm: boolean;
}

export interface ExportOptions {
  /**
   * URI do buffer binário no JSON — para exportação `.gltf` com arquivo
   * externo (o caller grava `bin` em disco/CDN e o parser resolve pela URI).
   * Sem esta opção, o buffer sai sem `uri` (caso GLB, que embute o BIN).
   */
  bufferUri?: string;
}

/**
 * Exporta a cena normalizada para glTF + bin. Com `vrm`, acrescenta as
 * extensões VRM 1.0 (meta/humanoid/expression + MToon + spring bones).
 *
 * Ordem canônica de accessors (a que o JSON referencia):
 *   meshes[i].primitives[j] → POSITION, NORMAL, TEXCOORD_0, COLOR_0,
 *   JOINTS_0, WEIGHTS_0, INDICES, [targets: POSITION, NORMAL]*
 *   skins[i]                → inverseBindMatrices
 *   animations[i]           → por track: times, values
 */
export function exportGltf(scene: ExportScene, vrm?: Vrm1ExportData, options: ExportOptions = {}): ExportResult {
  const bin = new BinWriter();
  const bufferViews: Array<Record<string, unknown>> = [];
  const accessors: Array<Record<string, unknown>> = [];

  const pushView = (offset: number, byteLength: number, target?: number): number => {
    const entry: Record<string, unknown> = { buffer: 0, byteOffset: offset, byteLength };
    if (target !== undefined) entry["target"] = target;
    bufferViews.push(entry);
    return bufferViews.length - 1;
  };

  const pushAccessor = (bufferView: number, componentType: number, count: number, type: string): number => {
    accessors.push({ bufferView, componentType, count, type });
    return accessors.length - 1;
  };

  const pushFloatAccessor = (values: number[], count: number, type: string, target?: number): number => {
    const offset = bin.writeFloats(values);
    return pushAccessor(pushView(offset, values.length * 4, target), GLTF_COMPONENT_TYPES.F32, count, type);
  };

  // Regra única para índices: uint16 quando cabem, uint32 caso contrário.
  let maxIndex = 0;
  for (const mesh of scene.meshes) {
    for (const prim of mesh.primitives) {
      for (const index of prim.indices) maxIndex = Math.max(maxIndex, index);
    }
  }
  const useUint16Indices = maxIndex < 65536;

  const meshJson: Array<Record<string, unknown>> = [];
  scene.meshes.forEach((mesh, meshIndex) => {
    const primitivesJson: Array<Record<string, unknown>> = [];
    mesh.primitives.forEach((prim) => {
      const count = prim.vertices.length;
      const attributeAccessors: Record<string, number> = {};
      const writeVector = (get: (v: ExportVertex, c: number) => number, comps: number, type: string): number => {
        const values: number[] = [];
        for (let i = 0; i < count; i++) for (let c = 0; c < comps; c++) values.push(get(prim.vertices[i], c));
        return pushFloatAccessor(values, count, type, 34962);
      };
      attributeAccessors["POSITION"] = writeVector((v, c) => v.position[c], 3, "VEC3");
      attributeAccessors["NORMAL"] = writeVector((v, c) => v.normal[c], 3, "VEC3");
      attributeAccessors["TEXCOORD_0"] = writeVector((v, c) => v.uv[c], 2, "VEC2");
      attributeAccessors["COLOR_0"] = writeVector((v, c) => v.color[c], 4, "VEC4");
      {
        const values: number[] = [];
        for (const v of prim.vertices) values.push(...v.joints);
        const offset = bin.writeUint16(values);
        attributeAccessors["JOINTS_0"] = pushAccessor(pushView(offset, values.length * 2, 34962), GLTF_COMPONENT_TYPES.U16, count, "VEC4");
      }
      {
        const values: number[] = [];
        for (const v of prim.vertices) values.push(...v.weights);
        attributeAccessors["WEIGHTS_0"] = pushFloatAccessor(values, count, "VEC4", 34962);
      }
      const indexOffset = useUint16Indices ? bin.writeUint16(prim.indices) : bin.writeUint32(prim.indices);
      const indicesAccessor = pushAccessor(
        pushView(indexOffset, prim.indices.length * (useUint16Indices ? 2 : 4), 34963),
        useUint16Indices ? GLTF_COMPONENT_TYPES.U16 : GLTF_COMPONENT_TYPES.U32,
        prim.indices.length,
        "SCALAR",
      );
      const primitiveJson: Record<string, unknown> = {
        attributes: attributeAccessors,
        indices: indicesAccessor,
        mode: GLTF_PRIMITIVE_MODES.TRIANGLES,
      };
      if (prim.materialIndex >= 0) primitiveJson["material"] = prim.materialIndex;
      if (prim.morphTargets && prim.morphTargets.length > 0) {
        primitiveJson["targets"] = prim.morphTargets.map((target) => {
          const positionValues: number[] = [];
          const normalValues: number[] = [];
          for (const delta of target) {
            positionValues.push(...delta.position);
            normalValues.push(...delta.normal);
          }
          const positionAccessor = pushFloatAccessor(positionValues, target.length, "VEC3", 34962);
          const normalAccessor = pushFloatAccessor(normalValues, target.length, "VEC3", 34962);
          return { POSITION: positionAccessor, NORMAL: normalAccessor };
        });
      }
      primitivesJson.push(primitiveJson);
    });
    const meshEntry: Record<string, unknown> = {
      name: mesh.name ?? `mesh_${meshIndex}`,
      primitives: primitivesJson,
    };
    meshJson.push(meshEntry);
  });

  const skinJson: Array<Record<string, unknown>> = [];
  scene.skins.forEach((skin, skinIndex) => {
    const values: number[] = [];
    for (const matrix of skin.inverseBindMatrices) values.push(...matrix);
    const ibmAccessor = pushFloatAccessor(values, skin.inverseBindMatrices.length, "MAT4");
    const entry: Record<string, unknown> = {
      name: skin.name ?? `skin_${skinIndex}`,
      joints: skin.joints,
      inverseBindMatrices: ibmAccessor,
    };
    if (skin.skeleton !== undefined) entry["skeleton"] = skin.skeleton;
    skinJson.push(entry);
  });

  const animationJson: Array<Record<string, unknown>> = [];
  scene.animations.forEach((animation, animIndex) => {
    const samplers: Array<Record<string, unknown>> = [];
    const channels: Array<Record<string, unknown>> = [];
    animation.tracks.forEach((track, trackIndex) => {
      const timesAccessor = pushFloatAccessor(track.sampler.times, track.sampler.times.length, "SCALAR");
      const valueComps = track.path === "rotation" ? 4 : track.path === "weights" ? 1 : 3;
      const valuesAccessor = pushFloatAccessor(
        track.sampler.values,
        Math.trunc(track.sampler.values.length / valueComps),
        valueComps === 4 ? "VEC4" : valueComps === 3 ? "VEC3" : "SCALAR",
      );
      const sampler: Record<string, unknown> = { input: timesAccessor, output: valuesAccessor };
      if (track.sampler.interp === "STEP") sampler["interpolation"] = "STEP";
      samplers.push(sampler);
      channels.push({ sampler: trackIndex, target: { node: track.node, path: track.path } });
    });
    animationJson.push({
      name: animation.name ?? `anim_${animIndex}`,
      channels,
      samplers,
    });
  });

  const materialJson: Array<Record<string, unknown>> = scene.materials.map((material, index) => {
    const pbr: Record<string, unknown> = {
      baseColorFactor: stableArray(material.baseColorFactor),
    };
    if (material.metallicFactor !== undefined) pbr["metallicFactor"] = stableNumber(material.metallicFactor);
    if (material.roughnessFactor !== undefined) pbr["roughnessFactor"] = stableNumber(material.roughnessFactor);
    const entry: Record<string, unknown> = {
      name: material.name ?? `material_${index}`,
      pbrMetallicRoughness: pbr,
    };
    if (material.emissiveFactor !== undefined) entry["emissiveFactor"] = stableArray(material.emissiveFactor);
    if (material.alphaMode !== undefined) entry["alphaMode"] = material.alphaMode;
    if (material.doubleSided !== undefined) entry["doubleSided"] = material.doubleSided;
    if (vrm && material.mtoon) {
      const mtoon = material.mtoon;
      entry["extensions"] = {
        VRMC_materials_mtoon: {
          mainTex: null,
          subEmission: stableArray(mtoon.subEmission ?? [1, 1, 1, 1]),
          multiply: [1, 1, 1, 1],
          shadowColor: stableArray(mtoon.shadowColor ?? [0.718, 0.831, 1, 1]),
          shadeShift: stableNumber(mtoon.shadeShift ?? 0),
          shadeToony: stableNumber(mtoon.shadeToony ?? 1),
          lightColor: [1, 1, 1],
          rimColor: stableArray(mtoon.rimColor ?? [1, 1, 1, 0]),
          rimPower: stableNumber(mtoon.rimPower ?? 1),
          rimLight: mtoon.rimLight ?? false,
          rimLightColor: [1, 1, 1, 1],
          lightDirection: [0, 0, 1],
          specularColor: stableArray(mtoon.specularColor ?? [1, 1, 1, 1]),
          specularPower: stableNumber(mtoon.specularPower ?? 1),
          useSmooth: false,
          smoothColor: [0.7, 0.7, 0.7, 0.5],
          useSphere: false,
          sphereMode: "normal",
          useMatCap: false,
        },
      };
    }
    return entry;
  });

  const nodeJson: Array<Record<string, unknown>> = scene.nodes.map((node, index) => {
    const entry: Record<string, unknown> = { name: node.name ?? `node_${index}` };
    if (node.translation) entry["translation"] = stableArray(node.translation);
    if (node.rotation) entry["rotation"] = stableArray(node.rotation);
    if (node.scale) entry["scale"] = stableArray(node.scale);
    if (node.meshIndex !== undefined) entry["mesh"] = node.meshIndex;
    if (node.skinIndex !== undefined) entry["skin"] = node.skinIndex;
    if (node.morphWeights) entry["weights"] = stableArray(node.morphWeights);
    if (node.children && node.children.length > 0) entry["children"] = [...node.children];
    return entry;
  });

  const extensionsUsed: string[] = [];
  const extensionsRequired: string[] = [];
  const gltfExtensions: Record<string, unknown> = {};
  if (vrm) {
    extensionsUsed.push("VRMC_vrm", "VRMC_materials_mtoon");
    extensionsRequired.push("VRMC_vrm", "VRMC_materials_mtoon");
    const preset: Record<string, unknown> = {};
    for (const name of VRM1_EXPRESSION_PRESETS) {
      const index = vrm.expressionPresets[name];
      if (typeof index !== "number" || !Number.isInteger(index)) {
        throw new Error(`vrm.expressionPresets: preset obrigatório '${name}' ausente`);
      }
      preset[name] = { blendShape: index, isolated: false };
    }
    const custom: Record<string, unknown> = {};
    for (const [name, index] of Object.entries(vrm.expressionCustom ?? {})) {
      custom[name] = { blendShape: index, isolated: false };
    }
    const humanoidBones: Record<string, unknown> = {};
    for (const [boneName, nodeIndex] of Object.entries(vrm.humanoidBones)) {
      humanoidBones[boneName] = { node: nodeIndex };
    }
    gltfExtensions["VRMC_vrm"] = {
      meta: {
        title: vrm.meta.title,
        author: vrm.meta.author,
        version: vrm.meta.version,
        year: vrm.meta.year,
        license: vrm.meta.license,
        contactInformation: vrm.meta.contactInformation,
        metadata: vrm.meta.metadata,
        reference: vrm.meta.reference,
      },
      humanoid: { humanBones: humanoidBones },
      expression: { preset, custom },
    };
    if (vrm.springBone) {
      extensionsUsed.push("VRMC_springBone");
      extensionsRequired.push("VRMC_springBone");
      const springColliders: Array<Record<string, unknown>> = [];
      for (const sphere of vrm.springBone.colliders.spheres) {
        springColliders.push({
          spheres: [{ group: sphere.group, node: sphere.node, radius: sphere.radius, offset: sphere.offset ?? [0, 0, 0] }],
        });
      }
      for (const capsule of vrm.springBone.colliders.capsules) {
        springColliders.push({
          capsules: [{ group: capsule.group, node: capsule.node, from: capsule.from, to: capsule.to, radius: capsule.radius }],
        });
      }
      const springJoint = (joint: {
        node: number;
        distance: number;
        hitRadius?: number;
        gravityPower?: number;
        gravityDir?: [number, number, number];
        springStiffness?: number;
        springDamping?: number;
      }): Record<string, unknown> => {
        const out: Record<string, unknown> = { node: joint.node, distance: joint.distance };
        if (joint.hitRadius !== undefined) out["hitRadius"] = joint.hitRadius;
        if (joint.gravityPower !== undefined) out["gravityPower"] = joint.gravityPower;
        if (joint.gravityDir) out["gravityDir"] = [...joint.gravityDir];
        if (joint.springStiffness !== undefined) out["springStiffness"] = joint.springStiffness;
        if (joint.springDamping !== undefined) out["springDamping"] = joint.springDamping;
        return out;
      };
      gltfExtensions["VRMC_springBone"] = {
        secondaryRig: {
          groups: vrm.springBone.groups.map((group) => ({
            name: group.name,
            center: springJoint(group.center),
            joints: group.joints.map((joint) => springJoint(joint)),
            colliderGroups: group.colliderGroups,
          })),
          colliders: springColliders,
        },
      };
    }
  }

  const binBytes = bin.finish();
  const json: Record<string, unknown> = {
    asset: {
      version: "2.0",
      generator: "anigo-studio/0.1.0 (fase2-exporter)",
      ...(extensionsUsed.length > 0 ? { extensionsUsed } : {}),
      ...(extensionsRequired.length > 0 ? { extensionsRequired } : {}),
    },
    scene: 0,
    scenes: [{ name: scene.name ?? "ANIGO Scene", nodes: scene.rootNodes }],
    nodes: nodeJson,
    meshes: meshJson,
    ...(scene.skins.length > 0 ? { skins: skinJson } : {}),
    ...(scene.materials.length > 0 ? { materials: materialJson } : {}),
    ...(scene.animations.length > 0 ? { animations: animationJson } : {}),
    buffers: options.bufferUri
      ? [{ uri: options.bufferUri, byteLength: binBytes.byteLength }]
      : [{ byteLength: binBytes.byteLength }],
    bufferViews,
    accessors,
    ...(extensionsUsed.length > 0 ? { extensions: gltfExtensions } : {}),
  };

  return { json: json as unknown as Gltf, bin: binBytes, isVrm: vrm !== undefined };
}

// ---------------------------------------------------------------------------
// Empacotamento GLB
// ---------------------------------------------------------------------------

/**
 * Monta o byte stream GLB (header 12 B + JSON chunk + BIN chunk), com padding
 * canônico (JSON→0x20, BIN→0x00). Determinístico para a mesma cena.
 */
export function buildGlb(json: Gltf, bin: Uint8Array): Uint8Array {
  const jsonBytes = new TextEncoder().encode(JSON.stringify(json));
  const padJson = (4 - (jsonBytes.byteLength % 4)) % 4;
  const padBin = (4 - (bin.byteLength % 4)) % 4;
  const hasBin = bin.byteLength > 0 || padBin > 0;
  const total = 12 + 8 + jsonBytes.byteLength + padJson + (hasBin ? 8 + bin.byteLength + padBin : 0);
  const out = new Uint8Array(total);
  const dv = new DataView(out.buffer);
  dv.setUint32(0, GLB_MAGIC, true);
  dv.setUint32(4, GLB_VERSION, true);
  dv.setUint32(8, total, true);
  let cursor = 12;
  dv.setUint32(cursor, jsonBytes.byteLength + padJson, true);
  dv.setUint32(cursor + 4, GLB_CHUNK_JSON, true);
  cursor += 8;
  out.set(jsonBytes, cursor);
  for (let i = 0; i < padJson; i++) out[cursor + jsonBytes.byteLength + i] = 0x20;
  cursor += jsonBytes.byteLength + padJson;
  if (hasBin) {
    dv.setUint32(cursor, bin.byteLength + padBin, true);
    dv.setUint32(cursor + 4, GLB_CHUNK_BIN, true);
    cursor += 8;
    out.set(bin, cursor);
    for (let i = 0; i < padBin; i++) out[cursor + bin.byteLength + i] = 0x00;
  }
  return out;
}

/** Resultado completo: bytes `.glb` (ou `.vrm` quando `vrm` foi passado). */
export function exportGlb(scene: ExportScene, vrm?: Vrm1ExportData, options: ExportOptions = {}): Uint8Array {
  const { json, bin } = exportGltf(scene, vrm, options);
  return buildGlb(json, bin);
}

// ---------------------------------------------------------------------------
// Sumário de importação (UI/Tauri — issue #59)
// ---------------------------------------------------------------------------

export interface ImportSummary {
  title: string;
  gltfVersion: string;
  generator: string;
  vertexCount: number;
  indexCount: number;
  triangleCount: number;
  meshCount: number;
  nodeCount: number;
  materialCount: number;
  skinCount: number;
  animationCount: number;
  textureCount: number;
  morphTargetCount: number;
  isVrm: boolean;
  vrmExtensions: string[];
  warnings: string[];
}

/** Resumo estrutural de um documento importado (mostrado na UI/Tauri). */
export function summarizeImport(gltf: Gltf): ImportSummary {
  let vertexCount = 0;
  let indexCount = 0;
  let morphTargetCount = 0;
  const accessors = gltf.accessors ?? [];
  const meshes = gltf.meshes ?? [];
  for (const mesh of meshes) {
    for (const prim of mesh.primitives) {
      const position = accessors[prim.attributes["POSITION"]];
      if (position) vertexCount += position.count;
      if (prim.indices !== undefined) indexCount += accessors[prim.indices].count;
      morphTargetCount += (prim.targets ?? []).length;
    }
  }
  const used = gltf.asset.extensionsUsed ?? [];
  const isVrm = used.includes("VRMC_vrm");
  let title = "";
  if (isVrm && gltf.extensions?.["VRMC_vrm"]) {
    const meta = (gltf.extensions["VRMC_vrm"] as Record<string, unknown>)["meta"] as Record<string, unknown> | undefined;
    if (meta && typeof meta["title"] === "string") title = meta["title"];
  }
  return {
    title,
    gltfVersion: gltf.asset.version,
    generator: gltf.asset.generator ?? "",
    vertexCount,
    indexCount,
    triangleCount: Math.floor(indexCount / 3),
    meshCount: meshes.length,
    nodeCount: (gltf.nodes ?? []).length,
    materialCount: (gltf.materials ?? []).length,
    skinCount: (gltf.skins ?? []).length,
    animationCount: (gltf.animations ?? []).length,
    textureCount: (gltf.textures ?? []).length,
    morphTargetCount,
    isVrm,
    vrmExtensions: used.filter((ext) => ext.startsWith("VRMC_")),
    warnings: [],
  };
}
