/**
 * ANIGO — parser completo glTF 2.0 (Fase 2, issue #25).
 *
 * Supera o loader legado (`src/services/gltf_loader.ts`, que decodifica uma
 * única malha mesclada de um GLB simples) com:
 *
 *  - contêiner GLB (JSON + BIN) **e** `.gltf` JSON com buffers externos
 *    (data-URI ou `ArrayBuffer`/`Uint8Array` resolvidos por caller);
 *  - toda a árvore `buffers`/`bufferViews`/`accessors` (interleaved e
 *    de-interleaved, índices uint16 e uint32, atributos quantizados/normalizados,
 *    tipos SCALAR/VEC2/VEC3/VEC4/MAT3/MAT4 e sparsity);
 *  - `nodes` (TRS ou `matrix`), `scenes`, `meshes` (multi-primitiva, morph
 *    targets, modo de primitiva), `skins` (joints + inverseBindMatrices),
 *    `materials` (PBR metallic-roughness + alpha mode + extensions),
 *    `images`/`samplers`/`texturas`, `animations` (samplers + channels);
 *  - validação estrutural com **códigos de erro estáveis** (mesma disciplina
 *    do contrato de diagnósticos — sem falha silenciosa).
 *
 * Pure TypeScript: ArrayBuffer/Uint8Array entra, estruturas tipadas saem.
 * Unit-tested sob Node (`tests/character/vrm_interop.test.ts`).
 */

// ---------------------------------------------------------------------------
// Constantes do glTF 2.0
// ---------------------------------------------------------------------------

/** Constantes do contêiner GLB (spec glTF 2.0, §2). */
export const GLB_MAGIC = 0x46546c67; // "glTF"
export const GLB_VERSION = 2;
export const GLB_CHUNK_JSON = 0x4e4f534a; // "JSON"
export const GLB_CHUNK_BIN = 0x004e4942; // "BIN\0"

/**
 * Component types da spec glTF 2.0 (3.14.1) — GL constants:
 * 5120 BYTE, 5121 UNSIGNED_BYTE, 5122 UNSIGNED_SHORT, 5123 UNSIGNED_INT,
 * 5125 DOUBLE, 5126 FLOAT.
 */
export const GLTF_COMPONENT_TYPES = {
  U8: 5120,
  U8N: 5121,
  U16: 5122,
  U32: 5123,
  F64: 5125,
  F32: 5126,
} as const;

export type GltfComponentType = (typeof GLTF_COMPONENT_TYPES)[keyof typeof GLTF_COMPONENT_TYPES];

export const GLTF_PRIMITIVE_MODES = {
  POINTS: 0,
  LINES: 1,
  LINE_LOOP: 2,
  LINE_STRIP: 3,
  TRIANGLES: 4,
  TRIANGLE_STRIP: 5,
  TRIANGLE_FAN: 6,
} as const;

export const GLTF_TARGETS = { ARRAY_BUFFER: 34962, ELEMENT_ARRAY_BUFFER: 34963 } as const;

/** Componentes por tipo de accessor (spec 3.14.3). */
export function accessorComponentCount(type: string): number {
  switch (type) {
    case "SCALAR": return 1;
    case "VEC2": return 2;
    case "VEC3": return 3;
    case "VEC4": return 4;
    case "MAT2": return 4;
    case "MAT3": return 9;
    case "MAT4": return 16;
    default: return -1;
  }
}

export function accessorByteSize(type: string, componentType: number): number {
  const comps = accessorComponentCount(type);
  if (comps < 0) return -1;
  const byte =
    componentType === GLTF_COMPONENT_TYPES.F32 || componentType === GLTF_COMPONENT_TYPES.U32
      ? 4
      : componentType === GLTF_COMPONENT_TYPES.F64
        ? 8
        : componentType === GLTF_COMPONENT_TYPES.U16
          ? 2
          : 1;
  return comps * byte;
}

// ---------------------------------------------------------------------------
// Tipos do documento glTF 2.0 (subset completo consumido pelo ANIGO)
// ---------------------------------------------------------------------------

export interface GltfAsset {
  version: string;
  minVersion?: string;
  generator?: string;
  copyright?: string;
  extensionsUsed?: string[];
  extensionsRequired?: string[];
}

export interface GltfSparseIndices {
  bufferView: number;
  byteOffset?: number;
  componentType: number;
  count: number;
}

export interface GltfSparseValues {
  bufferView: number;
  byteOffset?: number;
}

export interface GltfAccessor {
  bufferView?: number;
  byteOffset?: number;
  componentType: number;
  normalized?: boolean;
  count: number;
  type: string;
  min?: number[];
  max?: number[];
  sparse?: { indices: GltfSparseIndices; values: GltfSparseValues };
  name?: string;
}

export interface GltfBuffer {
  uri?: string;
  byteLength: number;
  name?: string;
}

export interface GltfBufferView {
  buffer: number;
  byteOffset?: number;
  byteLength: number;
  byteStride?: number;
  target?: number;
  name?: string;
}

export interface GltfImage {
  uri?: string;
  bufferView?: number;
  mimeType?: string;
  name?: string;
}

export interface GltfSampler {
  magFilter?: number;
  minFilter?: number;
  wrapS?: number;
  wrapT?: number;
  name?: string;
}

export interface GltfTexture {
  sampler?: number;
  source?: number;
  name?: string;
}

export interface GltfTextureInfo {
  index: number;
  texCoord?: number;
}

export interface GltfMaterial {
  name?: string;
  pbrMetallicRoughness?: {
    baseColorFactor?: number[];
    baseColorTexture?: GltfTextureInfo;
    metallicFactor?: number;
    roughnessFactor?: number;
    metallicRoughnessTexture?: GltfTextureInfo;
  };
  normalTexture?: GltfTextureInfo & { scale?: number };
  occlusionTexture?: GltfTextureInfo & { strength?: number };
  emissiveFactor?: number[];
  emissiveTexture?: GltfTextureInfo;
  alphaMode?: "OPAQUE" | "MASK" | "BLEND";
  alphaCutoff?: number;
  doubleSided?: boolean;
  extensions?: Record<string, unknown>;
  extras?: unknown;
}

export interface GltfPrimitive {
  attributes: Record<string, number>;
  indices?: number;
  material?: number;
  mode?: number;
  /** Cada target é um objeto semântica → índice de accessor (spec 3.7.2). */
  targets?: Array<Record<string, number>>;
  extensions?: Record<string, unknown>;
  extras?: unknown;
}

export interface GltfMesh {
  name?: string;
  primitives: GltfPrimitive[];
  weights?: number[];
  extensions?: Record<string, unknown>;
  extras?: unknown;
}

export interface GltfNode {
  name?: string;
  children?: number[];
  matrix?: number[];
  translation?: number[];
  rotation?: number[];
  scale?: number[];
  mesh?: number;
  skin?: number;
  weights?: number[];
  camera?: number;
  extensions?: Record<string, unknown>;
  extras?: unknown;
}

export interface GltfSkin {
  name?: string;
  joints: number[];
  inverseBindMatrices?: number;
  skeleton?: number;
  extensions?: Record<string, unknown>;
  extras?: unknown;
}

export interface GltfScene {
  name?: string;
  nodes?: number[];
}

export interface GltfAnimationChannelTarget {
  node: number;
  path: "translation" | "rotation" | "scale" | "weights";
}

export interface GltfAnimationSampler {
  input: number;
  output: number;
  interpolation?: "LINEAR" | "STEP" | "CUBICSPLINE";
}

export interface GltfAnimation {
  name?: string;
  channels: Array<{ sampler: number; target: GltfAnimationChannelTarget }>;
  samplers: GltfAnimationSampler[];
}

export interface Gltf {
  asset: GltfAsset;
  buffers?: GltfBuffer[];
  bufferViews?: GltfBufferView[];
  accessors?: GltfAccessor[];
  images?: GltfImage[];
  samplers?: GltfSampler[];
  textures?: GltfTexture[];
  materials?: GltfMaterial[];
  meshes?: GltfMesh[];
  nodes?: GltfNode[];
  skins?: GltfSkin[];
  scenes?: GltfScene[];
  scene?: number;
  animations?: GltfAnimation[];
  cameras?: unknown[];
  lights?: unknown[];
  extensions?: Record<string, unknown>;
  extras?: unknown;
}

// ---------------------------------------------------------------------------
// Erros de parser (códigos estáveis — mesma disciplina do contrato)
// ---------------------------------------------------------------------------

export type GltfErrorCode =
  | "BAD_ASSET"
  | "UNSUPPORTED_VERSION"
  | "MISSING_BUFFER"
  | "BUFFER_LENGTH_MISMATCH"
  | "BAD_BUFFER_INDEX"
  | "BAD_BUFFERVIEW_INDEX"
  | "BUFFERVIEW_OUT_OF_RANGE"
  | "BAD_ACCESSOR_INDEX"
  | "ACCESSOR_OUT_OF_RANGE"
  | "BAD_ACCESSOR_COMPONENT"
  | "BAD_ACCESSOR_TYPE"
  | "BAD_ACCESSOR_COUNT"
  | "SPARSE_OUT_OF_RANGE"
  | "BAD_IMAGE_INDEX"
  | "BAD_SAMPLER_INDEX"
  | "BAD_TEXTURE_INDEX"
  | "BAD_MATERIAL_INDEX"
  | "BAD_MESH_INDEX"
  | "BAD_NODE_INDEX"
  | "BAD_SKIN_INDEX"
  | "BAD_SCENE_INDEX"
  | "BAD_ANIMATION_INDEX"
  | "NODE_GRAPH_CYCLE"
  | "MISSING_POSITION"
  | "MISSING_BIN"
  | "BAD_JSON"
  | "UNKNOWN_ERROR";

export class GltfError extends Error {
  readonly code: GltfErrorCode;
  constructor(code: GltfErrorCode, message: string) {
    super(`[gltf ${code}] ${message}`);
    this.name = "GltfError";
    this.code = code;
  }
}

function fail(code: GltfErrorCode, message: string): never {
  throw new GltfError(code, message);
}

/** `extensionsRequired` ausente ou sem glTF-2.0 obrigatórios que o ANIGO não implementa. */
export const UNSUPPORTED_EXTENSIONS: ReadonlySet<string> = new Set(["KHR_draco_mesh_compression"]);

// ---------------------------------------------------------------------------
// Resolção de buffers (GLB BIN, data-URI ou fornecido externamente)
// ---------------------------------------------------------------------------

/**
 * Fonte de bytes para buffers externos de um `.gltf`. O caller (Tauri,
 * viewport, teste) decide como resolver — o parser nunca faz I/O.
 */
export type ExternalBufferSource =
  | Map<string, Uint8Array | ArrayBuffer>
  | ((uri: string) => Uint8Array | ArrayBuffer | undefined);

function dataUriToBytes(uri: string): Uint8Array {
  const match = /^data:([^;,]*)(;base64)?,(.*)$/s.exec(uri);
  if (!match) fail("MISSING_BUFFER", `data URI inválido: ${uri.slice(0, 40)}…`);
  const [, , isBase64, payload] = match;
  if (isBase64) {
    const bin = atob(payload);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }
  return new TextEncoder().encode(decodeURIComponent(payload));
}

function coerceBytes(source: Uint8Array | ArrayBuffer): Uint8Array {
  return source instanceof Uint8Array ? source : new Uint8Array(source);
}

export interface ResolvedBuffer {
  bytes: Uint8Array;
  /** `true` quando os bytes vieram do chunk BIN do GLB. */
  embedded: boolean;
}

/**
 * Resolve todos os `buffers` do documento contra a fonte fornecida.
 * Um buffer com `byteLength === 0` é permitido (spec) e vira array vazio.
 */
export function resolveBuffers(gltf: Gltf, source: ExternalBufferSource | null): ResolvedBuffer[] {
  const buffers = gltf.buffers ?? [];
  const out: ResolvedBuffer[] = [];
  for (let i = 0; i < buffers.length; i++) {
    const buffer = buffers[i];
    const uri = buffer.uri;
    let bytes: Uint8Array | null = null;
    let embedded = false;
    if (uri === undefined) {
      if (buffer.byteLength !== 0) {
        fail("MISSING_BIN", `buffers[${i}] sem uri precisa do chunk BIN do GLB (ou buffer externo vazio)`);
      }
      bytes = new Uint8Array(0);
    } else if (uri.startsWith("data:")) {
      bytes = dataUriToBytes(uri);
    } else if (source) {
      const resolved =
        typeof source === "function" ? source(uri) : source.get(uri);
      if (resolved === undefined || resolved === null) {
        fail("MISSING_BUFFER", `buffers[${i}]: fonte externa não resolveu a URI '${uri}'`);
      }
      bytes = coerceBytes(resolved);
    } else {
      fail("MISSING_BUFFER", `buffers[${i}]: URI externa '${uri}' sem fonte de buffers`);
    }
    if (bytes.byteLength !== buffer.byteLength) {
      fail(
        "BUFFER_LENGTH_MISMATCH",
        `buffers[${i}]: byteLength=${buffer.byteLength} mas a fonte tem ${bytes.byteLength} bytes`,
      );
    }
    out.push({ bytes, embedded });
  }
  return out;
}

// ---------------------------------------------------------------------------
// Modelo parseado
// ---------------------------------------------------------------------------

export interface ParsedGltfModel {
  gltf: Gltf;
  buffers: ResolvedBuffer[];
  warnings: string[];
}

/**
 * Valida a estrutura do documento e resolve os buffers.
 *
 * @param gltf  JSON glTF 2.0
 * @param source bytes externos (para `.gltf` com `buffers[i].uri` relativo);
 *        `null` quando tudo está embutido (GLB / data-URI).
 */
export function parseGltfDocument(gltf: Gltf, source: ExternalBufferSource | null = null): ParsedGltfModel {
  const warnings: string[] = [];
  if (!gltf || typeof gltf !== "object") fail("BAD_JSON", "documento glTF não é um objeto");
  const asset = gltf.asset;
  if (!asset || typeof asset.version !== "string") fail("BAD_ASSET", "campo `asset.version` ausente");
  const major = Number(asset.version.split(".")[0]);
  if (major !== 2) fail("UNSUPPORTED_VERSION", `asset.version ${asset.version} (esperado major 2)`);

  const required = asset.extensionsRequired ?? [];
  for (const ext of required) {
    if (UNSUPPORTED_EXTENSIONS.has(ext)) {
      fail("BAD_ASSET", `extensão obrigatória não suportada pelo ANIGO: ${ext}`);
    }
  }
  const used = asset.extensionsUsed ?? [];
  for (const ext of used) {
    if (UNSUPPORTED_EXTENSIONS.has(ext)) {
      warnings.push(`extensão usada não suportada (mantida crua): ${ext}`);
    }
  }

  const buffers = resolveBuffers(gltf, source);

  const bufferViews = gltf.bufferViews ?? [];
  for (let i = 0; i < bufferViews.length; i++) {
    const view = bufferViews[i];
    if (view.buffer < 0 || view.buffer >= buffers.length) {
      fail("BAD_BUFFER_INDEX", `bufferViews[${i}].buffer=${view.buffer} fora da faixa`);
    }
    const start = view.byteOffset ?? 0;
    if (start < 0 || start + view.byteLength > buffers[view.buffer].bytes.byteLength) {
      fail("BUFFERVIEW_OUT_OF_RANGE", `bufferViews[${i}] ultrapassa o buffer (start ${start} + len ${view.byteLength})`);
    }
  }

  const accessors = gltf.accessors ?? [];
  for (let i = 0; i < accessors.length; i++) {
    const accessor = accessors[i];
    if (accessor.bufferView !== undefined && (accessor.bufferView < 0 || accessor.bufferView >= bufferViews.length)) {
      fail("BAD_BUFFERVIEW_INDEX", `accessors[${i}].bufferView fora da faixa`);
    }
    if (accessorComponentCount(accessor.type) < 0) {
      fail("BAD_ACCESSOR_TYPE", `accessors[${i}].type='${accessor.type}' inválido`);
    }
    if (
      accessor.componentType !== GLTF_COMPONENT_TYPES.U8 &&
      accessor.componentType !== GLTF_COMPONENT_TYPES.U8N &&
      accessor.componentType !== GLTF_COMPONENT_TYPES.U16 &&
      accessor.componentType !== GLTF_COMPONENT_TYPES.U32 &&
      accessor.componentType !== GLTF_COMPONENT_TYPES.F64 &&
      accessor.componentType !== GLTF_COMPONENT_TYPES.F32
    ) {
      fail("BAD_ACCESSOR_COMPONENT", `accessors[${i}].componentType=${accessor.componentType} inválido`);
    }
    if (!Number.isInteger(accessor.count) || accessor.count < 0) {
      fail("BAD_ACCESSOR_COUNT", `accessors[${i}].count inválido`);
    }
  }

  // Índices de referência cruzada (falha cedo, com código estável).
  const inRange = (idx: unknown, len: number, where: string): number => {
    if (typeof idx !== "number" || !Number.isInteger(idx) || idx < 0 || idx >= len) {
      fail("UNKNOWN_ERROR", `${where} aponta para índice inválido`);
    }
    return idx;
  };
  for (let i = 0; i < (gltf.textures ?? []).length; i++) {
    const texture = gltf.textures![i];
    if (texture.sampler !== undefined) inRange(texture.sampler, (gltf.samplers ?? []).length, `textures[${i}].sampler`);
    if (texture.source !== undefined) inRange(texture.source, (gltf.images ?? []).length, `textures[${i}].source`);
  }
  for (let i = 0; i < (gltf.meshes ?? []).length; i++) {
    const mesh = gltf.meshes![i];
    if (!Array.isArray(mesh.primitives) || mesh.primitives.length === 0) {
      fail("BAD_MESH_INDEX", `meshes[${i}] sem primitivas`);
    }
    for (let p = 0; p < mesh.primitives.length; p++) {
      const prim = mesh.primitives[p];
      if (!prim.attributes || typeof prim.attributes !== "object") {
        fail("MISSING_POSITION", `meshes[${i}].primitives[${p}] sem attributes`);
      }
      if (prim.attributes["POSITION"] === undefined) {
        fail("MISSING_POSITION", `meshes[${i}].primitives[${p}] sem POSITION`);
      }
      const attributeAccessors = gltf.accessors ?? [];
      for (const [semantic, idx] of Object.entries(prim.attributes)) {
        if (idx < 0 || idx >= attributeAccessors.length) {
          fail("BAD_ACCESSOR_INDEX", `meshes[${i}].primitives[${p}].${semantic} fora da faixa`);
        }
      }
      if (prim.indices !== undefined && (prim.indices < 0 || prim.indices >= attributeAccessors.length)) {
        fail("BAD_ACCESSOR_INDEX", `meshes[${i}].primitives[${p}].indices fora da faixa`);
      }
      if (prim.material !== undefined) inRange(prim.material, (gltf.materials ?? []).length, `meshes[${i}].primitives[${p}].material`);
      if (prim.targets) {
        for (const target of prim.targets) {
          if (typeof target !== "object" || target === null) {
            fail("BAD_ACCESSOR_INDEX", `meshes[${i}].primitives[${p}].targets precisa ser objeto semântica→accessor`);
          }
          for (const idx of Object.values(target)) {
            if (typeof idx !== "number" || idx < 0 || idx >= attributeAccessors.length) {
              fail("BAD_ACCESSOR_INDEX", `meshes[${i}].primitives[${p}].targets fora da faixa`);
            }
          }
        }
      }
    }
  }
  const nodes = gltf.nodes ?? [];
  for (let i = 0; i < nodes.length; i++) {
    const node = nodes[i];
    if (node.matrix !== undefined && (node.translation || node.rotation || node.scale)) {
      warnings.push(`nodes[${i}]: matrix e TRS coexistem — matrix prevalece (spec)`);
    }
    if (node.mesh !== undefined) inRange(node.mesh, (gltf.meshes ?? []).length, `nodes[${i}].mesh`);
    if (node.skin !== undefined) inRange(node.skin, (gltf.skins ?? []).length, `nodes[${i}].skin`);
    for (const child of node.children ?? []) inRange(child, nodes.length, `nodes[${i}].children`);
  }
  for (let i = 0; i < (gltf.skins ?? []).length; i++) {
    const skin = gltf.skins![i];
    if (!Array.isArray(skin.joints) || skin.joints.length === 0) {
      fail("BAD_SKIN_INDEX", `skins[${i}].joints vazio`);
    }
    for (const joint of skin.joints) inRange(joint, nodes.length, `skins[${i}].joints`);
    if (skin.inverseBindMatrices !== undefined) {
      inRange(skin.inverseBindMatrices, accessors.length, `skins[${i}].inverseBindMatrices`);
    }
    if (skin.skeleton !== undefined) inRange(skin.skeleton, nodes.length, `skins[${i}].skeleton`);
  }
  if (gltf.scene !== undefined) inRange(gltf.scene, (gltf.scenes ?? []).length, "scene");
  for (let i = 0; i < (gltf.animations ?? []).length; i++) {
    const animation = gltf.animations![i];
    for (const channel of animation.channels) {
      inRange(channel.sampler, animation.samplers.length, `animations[${i}].channels.sampler`);
      inRange(channel.target.node, nodes.length, `animations[${i}].channels.target.node`);
    }
    for (const sampler of animation.samplers) {
      inRange(sampler.input, accessors.length, `animations[${i}].samplers.input`);
      inRange(sampler.output, accessors.length, `animations[${i}].samplers.output`);
    }
  }

  // Ciclo no grafo de nós: traversal com estados (branco/cinza/preto).
  {
    const WHITE = 0, GRAY = 1, BLACK = 2;
    const state = new Array(nodes.length).fill(WHITE);
    const walk = (idx: number): void => {
      state[idx] = GRAY;
      for (const child of nodes[idx].children ?? []) {
        if (state[child] === GRAY) fail("NODE_GRAPH_CYCLE", `ciclo detectado em nodes[${idx}] → nodes[${child}]`);
        if (state[child] === WHITE) walk(child);
      }
      state[idx] = BLACK;
    };
    for (let i = 0; i < nodes.length; i++) {
      if (state[i] === WHITE) walk(i);
    }
  }

  return { gltf, buffers, warnings };
}

// ---------------------------------------------------------------------------
// Leitura de accessors (interleaved, quantizado, sparsity)
// ---------------------------------------------------------------------------

function accessorRegion(
  model: ParsedGltfModel,
  accessor: GltfAccessor,
): { bytes: Uint8Array; view: GltfBufferView | null; byteOffset: number } {
  const offset = accessor.byteOffset ?? 0;
  if (accessor.bufferView === undefined) {
    if (accessor.sparse === undefined) {
      fail("BAD_ACCESSOR_INDEX", "accessor sem bufferView nem sparse");
    }
    // Sparse com apenas valores esparsos: base zero.
    const size = accessorByteSize(accessor.type, accessor.componentType) * accessor.count;
    return { bytes: new Uint8Array(size), view: null, byteOffset: 0 };
  }
  const view = model.gltf.bufferViews![accessor.bufferView];
  const buffer = model.buffers[view.buffer];
  const start = (view.byteOffset ?? 0) + offset;
  const stride = view.byteStride ?? accessorByteSize(accessor.type, accessor.componentType);
  const lastElementStart = (accessor.count - 1) * stride;
  const end = start + lastElementStart + accessorByteSize(accessor.type, accessor.componentType);
  if (start < 0 || end > buffer.bytes.byteLength) {
    fail("ACCESSOR_OUT_OF_RANGE", `accessor ultrapassa o buffer (end ${end} > ${buffer.bytes.byteLength})`);
  }
  return { bytes: buffer.bytes, view, byteOffset: start };
}

function decodeScalar(
  bytes: Uint8Array,
  byteOffset: number,
  componentType: number,
  normalized: boolean,
): number {
  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  switch (componentType) {
    case GLTF_COMPONENT_TYPES.U8: return dv.getUint8(byteOffset);
    case GLTF_COMPONENT_TYPES.U8N: return normalized ? dv.getUint8(byteOffset) / 255 : dv.getUint8(byteOffset);
    case GLTF_COMPONENT_TYPES.U16:
      // Na spec, 5122 é unsigned; `normalized` divide por 65535.
      return normalized ? dv.getUint16(byteOffset, true) / 65535 : dv.getUint16(byteOffset, true);
    case GLTF_COMPONENT_TYPES.U32: return dv.getUint32(byteOffset, true);
    case GLTF_COMPONENT_TYPES.F64: return dv.getFloat64(byteOffset, true);
    case GLTF_COMPONENT_TYPES.F32: return dv.getFloat32(byteOffset, true);
    default: fail("BAD_ACCESSOR_COMPONENT", `componentType ${componentType} não decodificável`);
  }
}

/**
 * Lê um accessor como `Float32Array` de `count * componentCount` elementos.
 * Suporta: tipos escalares/vetoriais/matrizes, quantização normalizada
 * (u8n/u8→0..1, u16→0..65535, u32→0..4294967295), `byteStride` (interleaved)
 * e `sparse`.
 */
export function readAccessorFloats(model: ParsedGltfModel, accessorIndex: number): Float32Array {
  const accessor = model.gltf.accessors![accessorIndex];
  const comps = accessorComponentCount(accessor.type);
  const scalarBytes = accessorByteSize(accessor.type, accessor.componentType) / comps;
  const out = new Float32Array(accessor.count * comps);
  const region = accessorRegion(model, accessor);
  const stride = region.view?.byteStride ?? accessorByteSize(accessor.type, accessor.componentType);
  if (stride < scalarBytes * comps) {
    fail("ACCESSOR_OUT_OF_RANGE", `byteStride ${stride} menor que o tamanho do accessor`);
  }
  for (let i = 0; i < accessor.count; i++) {
    const base = region.byteOffset + i * stride;
    for (let c = 0; c < comps; c++) {
      out[i * comps + c] = decodeScalar(region.bytes, base + c * scalarBytes, accessor.componentType, accessor.normalized ?? false);
    }
  }
  if (accessor.sparse) applySparse(model, accessor, out);
  return out;
}

function applySparse(model: ParsedGltfModel, accessor: GltfAccessor, out: Float32Array): void {
  const sparse = accessor.sparse!;
  const indexView = model.gltf.bufferViews![sparse.indices.bufferView];
  const indexBuffer = model.buffers[indexView.buffer];
  const indexStart = (indexView.byteOffset ?? 0) + (sparse.indices.byteOffset ?? 0);
  const indexDv = new DataView(indexBuffer.bytes.buffer, indexBuffer.bytes.byteOffset, indexBuffer.bytes.byteLength);
  const valueView = model.gltf.bufferViews![sparse.values.bufferView];
  const valueBuffer = model.buffers[valueView.buffer];
  const valueStart = (valueView.byteOffset ?? 0) + (sparse.values.byteOffset ?? 0);
  const comps = accessorComponentCount(accessor.type);
  const scalarBytes = accessorByteSize(accessor.type, accessor.componentType) / comps;
  const indexStride = indexView.byteStride ?? 0;
  for (let i = 0; i < sparse.indices.count; i++) {
    const indexOffset = indexStart + (indexStride > 0 ? i * indexStride : 0);
    let vertexIndex: number;
    switch (sparse.indices.componentType) {
      case GLTF_COMPONENT_TYPES.U8: vertexIndex = indexDv.getUint8(indexOffset); break;
      case GLTF_COMPONENT_TYPES.U16: vertexIndex = indexDv.getUint16(indexOffset, true); break;
      case GLTF_COMPONENT_TYPES.U32: vertexIndex = indexDv.getUint32(indexOffset, true); break;
      default: fail("SPARSE_OUT_OF_RANGE", "sparse.indices.componentType inválido");
    }
    if (vertexIndex >= accessor.count) fail("SPARSE_OUT_OF_RANGE", `sparse índice ${vertexIndex} fora da faixa`);
    for (let c = 0; c < comps; c++) {
      out[vertexIndex * comps + c] = decodeScalar(
        valueBuffer.bytes,
        valueStart + (i * comps + c) * scalarBytes,
        accessor.componentType,
        accessor.normalized ?? false,
      );
    }
  }
}

/** Lê um accessor de índices como `Uint16Array`/`Uint32Array` (sem conversão). */
export function readAccessorIndices(model: ParsedGltfModel, accessorIndex: number): Uint16Array | Uint32Array {
  const accessor = model.gltf.accessors![accessorIndex];
  const region = accessorRegion(model, accessor);
  const stride = region.view?.byteStride ?? 0;
  if (accessor.componentType === GLTF_COMPONENT_TYPES.U16) {
    const out = new Uint16Array(accessor.count);
    const dv = new DataView(region.bytes.buffer, region.bytes.byteOffset, region.bytes.byteLength);
    for (let i = 0; i < accessor.count; i++) {
      const offset = region.byteOffset + (stride > 0 ? i * stride : i * 2);
      out[i] = dv.getUint16(offset, true);
    }
    return out;
  }
  if (accessor.componentType === GLTF_COMPONENT_TYPES.U32) {
    const out = new Uint32Array(accessor.count);
    const dv = new DataView(region.bytes.buffer, region.bytes.byteOffset, region.bytes.byteLength);
    for (let i = 0; i < accessor.count; i++) {
      const offset = region.byteOffset + (stride > 0 ? i * stride : i * 4);
      out[i] = dv.getUint32(offset, true);
    }
    return out;
  }
  fail("BAD_ACCESSOR_COMPONENT", `índices exigem uint16/uint32 (recebido ${accessor.componentType})`);
}

/**
 * Lê UM atributo de primitiva (POSITION, NORMAL, TEXCOORD_0, …) como
 * `Float32Array` de `count * comps`, já normalizado para f32 semântico.
 */
export function readPrimitiveAttribute(model: ParsedGltfModel, accessorIndex: number): Float32Array {
  return readAccessorFloats(model, accessorIndex);
}

/** Bytes de um image (via bufferView embutida) para o cache de texturas. */
export function readImageBytes(model: ParsedGltfModel, imageIndex: number): { bytes: Uint8Array; mimeType: string } {
  const image = model.gltf.images![imageIndex];
  if (image.bufferView !== undefined) {
    const view = model.gltf.bufferViews![image.bufferView];
    const buffer = model.buffers[view.buffer];
    const start = view.byteOffset ?? 0;
    const bytes = buffer.bytes.slice(start, start + view.byteLength);
    return { bytes, mimeType: image.mimeType ?? "image/png" };
  }
  if (image.uri !== undefined) {
    if (image.uri.startsWith("data:")) return { bytes: dataUriToBytes(image.uri), mimeType: image.mimeType ?? "image/png" };
    // URI externa: o caller resolve (o parser não faz I/O).
    return { bytes: new Uint8Array(0), mimeType: image.mimeType ?? "image/png" };
  }
  fail("BAD_IMAGE_INDEX", `images[${imageIndex}] sem uri nem bufferView`);
}

/** Contagem de mips esperada para uma textura (`floor(log2(max(w,h)))+1`). */
export function mipLevelCount(width: number, height: number): number {
  const maxSide = Math.max(width, height);
  if (maxSide <= 0) return 1;
  return Math.floor(Math.log2(maxSide)) + 1;
}
