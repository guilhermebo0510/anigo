/**
 * ANIGO Studio — Validating glTF 2.0 / GLB Loader (P0-10)
 *
 * Replaces the naive single-primitive loader with a complete, validated,
 * multi-primitive implementation that preserves nodes/transforms, skins
 * (joints + weights) and quantized attributes (VRM-compatible).
 *
 * Pure TypeScript (ArrayBuffer in → typed arrays out), unit-tested under Node.
 * The renderer adapts DecodedMesh → VertexData and surfaces GlbParseError
 * to the UI (no silent `return`, no unexplained fallback cube).
 */

export const GLB_MAGIC = 0x46546c67;
export const GLB_VERSION = 2;
export const GLB_CHUNK_JSON = 0x4e4f534a;
export const GLB_CHUNK_BIN = 0x004e4942;

export type GlbErrorCode =
  | "TOO_SMALL"
  | "BAD_MAGIC"
  | "BAD_VERSION"
  | "BAD_LENGTH"
  | "MISSING_JSON"
  | "BAD_JSON"
  | "MISSING_POSITION"
  | "BAD_ACCESSOR"
  | "NO_MESHES";

export class GlbParseError extends Error {
  readonly code: GlbErrorCode;
  readonly detail: string;
  constructor(code: GlbErrorCode, detail: string) {
    super(`[GLB ${code}] ${detail}`);
    this.name = "GlbParseError";
    this.code = code;
    this.detail = detail;
  }
}

export interface GlbContainer {
  json: GltfJson;
  bin: ArrayBuffer;
}

interface GltfJson {
  meshes?: Array<{
    primitives: Array<{
      attributes: Record<string, number>;
      indices?: number;
      material?: number;
    }>;
    name?: string;
  }>;
  accessors?: Array<{
    bufferView?: number;
    byteOffset?: number;
    componentType: number;
    normalized?: boolean;
    count: number;
    type: string;
  }>;
  bufferViews?: Array<{
    buffer: number;
    byteOffset?: number;
    byteLength: number;
    byteStride?: number;
  }>;
  nodes?: Array<{
    mesh?: number;
    matrix?: number[];
    rotation?: [number, number, number, number];
    scale?: [number, number, number];
    translation?: [number, number, number];
    children?: number[];
    skin?: number;
    name?: string;
  }>;
  scenes?: Array<{ nodes?: number[]; name?: string }>;
  scene?: number;
  skins?: Array<{ joints: number[]; skeleton?: number; name?: string }>;
  materials?: unknown[];
  images?: unknown[];
  textures?: unknown[];
}

/** Validates the GLB container and extracts JSON + BIN chunks by type scan. */
export function parseGlbContainer(buffer: ArrayBuffer): GlbContainer {
  if (buffer.byteLength < 20) {
    throw new GlbParseError("TOO_SMALL", `buffer has ${buffer.byteLength} bytes, need >= 20`);
  }
  const dv = new DataView(buffer);
  const magic = dv.getUint32(0, true);
  if (magic !== GLB_MAGIC) {
    throw new GlbParseError(
      "BAD_MAGIC",
      `expected 0x${GLB_MAGIC.toString(16)}, got 0x${magic.toString(16)}`
    );
  }
  const version = dv.getUint32(4, true);
  if (version !== GLB_VERSION) {
    throw new GlbParseError("BAD_VERSION", `expected 2, got ${version}`);
  }
  const length = dv.getUint32(8, true);
  if (length > buffer.byteLength || length < 20) {
    throw new GlbParseError(
      "BAD_LENGTH",
      `declared ${length} bytes, buffer holds ${buffer.byteLength}`
    );
  }

  let jsonBytes: Uint8Array | null = null;
  let bin: ArrayBuffer | null = null;
  let offset = 12;
  while (offset + 8 <= length) {
    const chunkLen = dv.getUint32(offset, true);
    const chunkType = dv.getUint32(offset + 4, true);
    const dataStart = offset + 8;
    const dataEnd = dataStart + chunkLen;
    if (dataEnd > buffer.byteLength) break; // truncated trailing chunk: stop
    if (chunkType === GLB_CHUNK_JSON && jsonBytes === null) {
      jsonBytes = new Uint8Array(buffer, dataStart, chunkLen);
    } else if (chunkType === GLB_CHUNK_BIN && bin === null) {
      bin = buffer.slice(dataStart, dataEnd);
    }
    offset = dataEnd;
  }
  if (!jsonBytes) throw new GlbParseError("MISSING_JSON", "no JSON chunk found");
  let json: GltfJson;
  try {
    json = JSON.parse(new TextDecoder().decode(jsonBytes));
  } catch (e) {
    throw new GlbParseError("BAD_JSON", String(e));
  }
  if (!bin) bin = new ArrayBuffer(0);
  return { json, bin };
}

// ---------------------------------------------------------------------------
// Accessor decoding (quantized-aware, stride-aware)
// ---------------------------------------------------------------------------

const COMPONENT_SIZES: Record<number, number> = {
  5120: 1, // BYTE (int8)
  5121: 1, // UNSIGNED_BYTE (u8)
  5122: 2, // UNSIGNED_SHORT (u16)
  5123: 4, // UNSIGNED_INT (u32)
  5125: 8, // DOUBLE (f64)
  5126: 4, // FLOAT (f32)
};

const TYPE_COMPONENTS: Record<string, number> = {
  SCALAR: 1,
  VEC2: 2,
  VEC3: 3,
  VEC4: 4,
  MAT2: 4,
  MAT3: 9,
  MAT4: 16,
};

function readComponent(
  dv: DataView,
  componentType: number,
  byteOffset: number,
  normalized: boolean
): number {
  switch (componentType) {
    case 5126:
      return dv.getFloat32(byteOffset, true);
    case 5124:
      return dv.getFloat64(byteOffset, true);
    case 5121: {
      const v = dv.getUint8(byteOffset);
      return normalized ? v / 255.0 : v;
    }
    case 5122: {
      const v = dv.getUint16(byteOffset, true);
      return normalized ? v / 65535.0 : v;
    }
    case 5120: {
      const v = dv.getInt8(byteOffset);
      return normalized ? Math.max(v / 127.0, -1.0) : v;
    }
    case 5123:
      return dv.getUint32(byteOffset, true);
    case 5125:
      return dv.getFloat64(byteOffset, true);
    default:
      throw new GlbParseError("BAD_ACCESSOR", `unsupported componentType ${componentType}`);
  }
}

/** Decodes an accessor to float triplets (positions/normals/uv/colors/weights). */
export function decodeAccessorFloat(
  json: GltfJson,
  bin: ArrayBuffer,
  accessorIdx: number
): { data: Float32Array; count: number; numComponents: number } {
  const accessor = json.accessors?.[accessorIdx];
  if (!accessor) throw new GlbParseError("BAD_ACCESSOR", `accessor ${accessorIdx} missing`);
  const numComponents = TYPE_COMPONENTS[accessor.type];
  const compSize = COMPONENT_SIZES[accessor.componentType];
  if (!numComponents || !compSize) {
    throw new GlbParseError(
      "BAD_ACCESSOR",
      `accessor ${accessorIdx}: bad type ${accessor.type}/${accessor.componentType}`
    );
  }
  if (accessor.bufferView === undefined) {
    // Spec: accessor without bufferView is zero-filled (e.g. sparse base).
    return {
      data: new Float32Array(accessor.count * numComponents),
      count: accessor.count,
      numComponents,
    };
  }
  const view = json.bufferViews?.[accessor.bufferView];
  if (!view) throw new GlbParseError("BAD_ACCESSOR", `bufferView ${accessor.bufferView} missing`);
  const stride = view.byteStride || numComponents * compSize;
  const base = (view.byteOffset || 0) + (accessor.byteOffset || 0);
  const normalized = accessor.normalized === true;
  const dv = new DataView(bin);
  const out = new Float32Array(accessor.count * numComponents);
  for (let i = 0; i < accessor.count; i++) {
    for (let c = 0; c < numComponents; c++) {
      out[i * numComponents + c] = readComponent(
        dv,
        accessor.componentType,
        base + i * stride + c * compSize,
        normalized
      );
    }
  }
  return { data: out, count: accessor.count, numComponents };
}

/** Decodes an index accessor (5121/5123/5125) to Uint32Array. */
export function decodeAccessorIndices(
  json: GltfJson,
  bin: ArrayBuffer,
  accessorIdx: number
): Uint32Array {
  const accessor = json.accessors?.[accessorIdx];
  if (!accessor) throw new GlbParseError("BAD_ACCESSOR", `index accessor ${accessorIdx} missing`);
  if (accessor.type !== "SCALAR") {
    throw new GlbParseError("BAD_ACCESSOR", `index accessor ${accessorIdx} must be SCALAR`);
  }
  if (accessor.bufferView === undefined) {
    const out = new Uint32Array(accessor.count);
    for (let i = 0; i < accessor.count; i++) out[i] = i;
    return out;
  }
  const view = json.bufferViews?.[accessor.bufferView];
  if (!view) throw new GlbParseError("BAD_ACCESSOR", `bufferView ${accessor.bufferView} missing`);
  const compSize = COMPONENT_SIZES[accessor.componentType];
  if (!compSize) throw new GlbParseError("BAD_ACCESSOR", `bad index componentType ${accessor.componentType}`);
  const stride = view.byteStride || compSize;
  const base = (view.byteOffset || 0) + (accessor.byteOffset || 0);
  const dv = new DataView(bin);
  const out = new Uint32Array(accessor.count);
  for (let i = 0; i < accessor.count; i++) {
    const off = base + i * stride;
    if (accessor.componentType === 5121) out[i] = dv.getUint8(off);
    else if (accessor.componentType === 5122) out[i] = dv.getUint16(off, true);
    else if (accessor.componentType === 5123) out[i] = dv.getUint32(off, true);
    else throw new GlbParseError("BAD_ACCESSOR", `bad index componentType ${accessor.componentType}`);
  }
  return out;
}

// ---------------------------------------------------------------------------
// Node transforms (column-major 4x4, glTF convention)
// ---------------------------------------------------------------------------

export type Mat4 = [
  number, number, number, number,
  number, number, number, number,
  number, number, number, number,
  number, number, number, number
];

export function mat4Identity(): Mat4 {
  return [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
}

export function mat4Multiply(a: Mat4, b: Mat4): Mat4 {
  const o = new Array(16).fill(0) as Mat4;
  for (let c = 0; c < 4; c++) {
    for (let r = 0; r < 4; r++) {
      o[c * 4 + r] =
        a[r] * b[c * 4] + a[4 + r] * b[c * 4 + 1] + a[8 + r] * b[c * 4 + 2] + a[12 + r] * b[c * 4 + 3];
    }
  }
  return o;
}

function quatToMat4(q: [number, number, number, number]): Mat4 {
  const [x, y, z, w] = q;
  const xx = x * x, yy = y * y, zz = z * z;
  const xy = x * y, xz = x * z, yz = y * z;
  const wx = w * x, wy = w * y, wz = w * z;
  return [
    1 - 2 * (yy + zz), 2 * (xy + wz), 2 * (xz - wy), 0,
    2 * (xy - wz), 1 - 2 * (xx + zz), 2 * (yz + wx), 0,
    2 * (xz + wy), 2 * (yz - wx), 1 - 2 * (xx + yy), 0,
    0, 0, 0, 1,
  ];
}

export function nodeLocalMatrix(node: NonNullable<GltfJson["nodes"]>[number]): Mat4 {
  if (node.matrix) {
    const m = node.matrix;
    return [m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8], m[9], m[10], m[11], m[12], m[13], m[14], m[15]];
  }
  const t = node.translation ?? [0, 0, 0];
  const s = node.scale ?? [1, 1, 1];
  const r = quatToMat4(node.rotation ?? [0, 0, 0, 1]);
  // M = T * R * S
  const m: Mat4 = [...r];
  for (let c = 0; c < 3; c++) {
    m[c * 4] *= s[c];
    m[c * 4 + 1] *= s[c];
    m[c * 4 + 2] *= s[c];
  }
  m[12] = t[0];
  m[13] = t[1];
  m[14] = t[2];
  return m;
}

/** Computes world matrix per node by walking the scene graph. */
export function computeWorldMatrices(json: GltfJson): Mat4[] {
  const nodes = json.nodes ?? [];
  const world: Mat4[] = nodes.map(() => mat4Identity());
  const visited = new Array(nodes.length).fill(false);
  const visit = (idx: number, parent: Mat4) => {
    if (visited[idx]) return;
    visited[idx] = true;
    const local = nodeLocalMatrix(nodes[idx]);
    const w = mat4Multiply(parent, local);
    world[idx] = w;
    for (const child of nodes[idx].children ?? []) {
      if (child >= 0 && child < nodes.length) visit(child, w);
    }
  };
  const scenes = json.scenes ?? [];
  const roots: number[] = [];
  if (json.scene !== undefined && scenes[json.scene]) {
    roots.push(...(scenes[json.scene].nodes ?? []));
  } else {
    for (const s of scenes) roots.push(...(s.nodes ?? []));
  }
  const identity = mat4Identity();
  if (roots.length === 0) {
    for (let i = 0; i < nodes.length; i++) visit(i, identity);
  } else {
    for (const r of roots) if (r >= 0 && r < nodes.length) visit(r, identity);
    // Orphan nodes (meshes not referenced by any scene) still get locals.
    for (let i = 0; i < nodes.length; i++) if (!visited[i]) visit(i, identity);
  }
  return world;
}

function transformPoint(m: Mat4, x: number, y: number, z: number): [number, number, number] {
  return [
    m[0] * x + m[4] * y + m[8] * z + m[12],
    m[1] * x + m[5] * y + m[9] * z + m[13],
    m[2] * x + m[6] * y + m[10] * z + m[14],
  ];
}

function transformDirection(m: Mat4, x: number, y: number, z: number): [number, number, number] {
  // Upper 3x3 (correct for rigid + uniform scale; non-uniform keeps shape,
  // acceptable for base-mesh loading — full inverse-transpose unnecessary here).
  let nx = m[0] * x + m[4] * y + m[8] * z;
  let ny = m[1] * x + m[5] * y + m[9] * z;
  let nz = m[2] * x + m[6] * y + m[10] * z;
  const l = Math.hypot(nx, ny, nz);
  if (l > 1e-9) {
    nx /= l;
    ny /= l;
    nz /= l;
  }
  return [nx, ny, nz];
}

function isIdentity(m: Mat4): boolean {
  return (
    m[0] === 1 && m[5] === 1 && m[10] === 1 && m[15] === 1 &&
    m[1] === 0 && m[2] === 0 && m[3] === 0 && m[4] === 0 &&
    m[6] === 0 && m[7] === 0 && m[8] === 0 && m[9] === 0 &&
    m[11] === 0 && m[12] === 0 && m[13] === 0 && m[14] === 0
  );
}

// ---------------------------------------------------------------------------
// Full decode: all meshes × all primitives, merged, transformed, skinned
// ---------------------------------------------------------------------------

export interface DecodedMesh {
  positions: Float32Array;
  normals: Float32Array;
  uvs: Float32Array;
  /** null when the asset carries no color channel (renderer uses neutral). */
  colors: Float32Array | null;
  colorComps: number;
  /** null when unskinned (renderer falls back to rigid slot 0). */
  joints: Uint16Array | null;
  weights: Float32Array | null;
  indices: Uint32Array;
  primitiveCount: number;
}

export interface DecodeResult {
  mesh: DecodedMesh;
  warnings: string[];
}

/**
 * Decodes every mesh/primitive referenced by the scene graph (or all meshes
 * when no scene exists) into ONE merged vertex/index soup in world space.
 */
export function decodeGltfMeshes(container: GlbContainer): DecodeResult {
  const { json, bin } = container;
  const warnings: string[] = [];
  if (!json.meshes || json.meshes.length === 0) {
    throw new GlbParseError("NO_MESHES", "asset contains no meshes");
  }

  const world = computeWorldMatrices(json);
  const nodes = json.nodes ?? [];

  // mesh index → world matrices of nodes instantiating it
  const meshInstances = new Map<number, Mat4[]>();
  if (nodes.length > 0) {
    nodes.forEach((n, i) => {
      if (n.mesh !== undefined && json.meshes?.[n.mesh]) {
        const list = meshInstances.get(n.mesh) ?? [];
        list.push(world[i]);
        meshInstances.set(n.mesh, list);
      }
    });
  }
  // Meshes never instantiated: still load once with identity.
  json.meshes.forEach((_m, i) => {
    if (!meshInstances.has(i)) meshInstances.set(i, [mat4Identity()]);
  });

  const posAll: number[] = [];
  const normAll: number[] = [];
  const uvAll: number[] = [];
  const colAll: number[] = [];
  const jointAll: number[] = [];
  const weightAll: number[] = [];
  const idxAll: number[] = [];
  let colorComps = 0;
  let hasSkin = false;
  let primitiveCount = 0;

  meshInstances.forEach((matrices, meshIdx) => {
    const mesh = json.meshes![meshIdx];
    for (const matrix of matrices) {
      for (const prim of mesh.primitives) {
        if (prim.attributes["POSITION"] === undefined) {
          warnings.push(`mesh ${meshIdx}: primitive without POSITION skipped`);
          continue;
        }
        const pos = decodeAccessorFloat(json, bin, prim.attributes["POSITION"]);
        if (pos.numComponents !== 3) {
          throw new GlbParseError("BAD_ACCESSOR", `mesh ${meshIdx}: POSITION must be VEC3`);
        }
        const count = pos.count;
        let norm: Float32Array;
        if (prim.attributes["NORMAL"] !== undefined) {
          const n = decodeAccessorFloat(json, bin, prim.attributes["NORMAL"]);
          norm = n.data;
        } else {
          warnings.push(`mesh ${meshIdx}: primitive without NORMAL, using +Y`);
          norm = new Float32Array(count * 3);
          for (let i = 0; i < count; i++) norm[i * 3 + 1] = 1;
        }
        let uv: Float32Array = new Float32Array(count * 2);
        if (prim.attributes["TEXCOORD_0"] !== undefined) {
          uv = decodeAccessorFloat(json, bin, prim.attributes["TEXCOORD_0"]).data;
        }
        // Color channel priority: _ANIGO_COLOR (canonical) → COLOR_0 (legacy).
        let col: Float32Array | null = null;
        let comps = 0;
        const colAttr =
          prim.attributes["_ANIGO_COLOR"] !== undefined
            ? "_ANIGO_COLOR"
            : prim.attributes["COLOR_0"] !== undefined
              ? "COLOR_0"
              : null;
        if (colAttr) {
          const c = decodeAccessorFloat(json, bin, prim.attributes[colAttr]);
          col = c.data;
          comps = c.numComponents;
        }
        // Skinning (rig preserved — P0-10).
        let joints: Uint16Array | null = null;
        let weights: Float32Array | null = null;
        if (prim.attributes["JOINTS_0"] !== undefined && prim.attributes["WEIGHTS_0"] !== undefined) {
          const j = decodeAccessorFloat(json, bin, prim.attributes["JOINTS_0"]);
          const w = decodeAccessorFloat(json, bin, prim.attributes["WEIGHTS_0"]);
          joints = new Uint16Array(count * 4);
          weights = new Float32Array(count * 4);
          const jc = Math.min(4, j.numComponents);
          const wc = Math.min(4, w.numComponents);
          for (let i = 0; i < count; i++) {
            let wsum = 0;
            for (let k = 0; k < 4; k++) {
              const jv = k < jc ? Math.round(j.data[i * j.numComponents + k]) : 0;
              const wv = k < wc ? w.data[i * w.numComponents + k] : k === 0 ? 1 : 0;
              joints[i * 4 + k] = Math.max(0, Math.min(65535, jv));
              weights[i * 4 + k] = wv;
              wsum += wv;
            }
            if (wsum > 1e-6) {
              for (let k = 0; k < 4; k++) weights[i * 4 + k] /= wsum;
            } else {
              weights[i * 4] = 1;
            }
          }
          hasSkin = true;
        }

        const base = posAll.length / 3;
        if (!isIdentity(matrix)) {
          for (let i = 0; i < count; i++) {
            const p = transformPoint(matrix, pos.data[i * 3], pos.data[i * 3 + 1], pos.data[i * 3 + 2]);
            posAll.push(p[0], p[1], p[2]);
            const n = transformDirection(matrix, norm[i * 3], norm[i * 3 + 1], norm[i * 3 + 2]);
            normAll.push(n[0], n[1], n[2]);
          }
        } else {
          for (let i = 0; i < pos.data.length; i++) posAll.push(pos.data[i]);
          for (let i = 0; i < norm.length; i++) normAll.push(norm[i]);
        }
        for (let i = 0; i < uv.length; i++) uvAll.push(uv[i]);
        if (col) {
          if (colorComps === 0) colorComps = comps;
          if (comps === colorComps) {
            for (let i = 0; i < col.length; i++) colAll.push(col[i]);
          } else {
            // Mixed channel widths across primitives: normalize to VEC4.
            for (let i = 0; i < count; i++) {
              for (let k = 0; k < 4; k++) {
                colAll.push(k < comps ? col[i * comps + k] : k === 3 ? 1 : 0.5);
              }
            }
            colorComps = 4;
          }
        }
        if (joints && weights) {
          for (let i = 0; i < joints.length; i++) jointAll.push(joints[i]);
          for (let i = 0; i < weights.length; i++) weightAll.push(weights[i]);
        }

        if (prim.indices !== undefined) {
          const idx = decodeAccessorIndices(json, bin, prim.indices);
          for (let i = 0; i < idx.length; i++) idxAll.push(base + idx[i]);
        } else {
          for (let i = 0; i < count; i++) idxAll.push(base + i);
        }
        primitiveCount++;
      }
    }
  });

  const vCount = posAll.length / 3;
  // Backfill colors when only some primitives carried the channel.
  let colors: Float32Array | null = null;
  if (colorComps > 0 && colAll.length > 0) {
    if (colAll.length < vCount * colorComps) {
      const filled = new Float32Array(vCount * colorComps);
      filled.fill(0.5);
      filled.set(colAll.slice(0, Math.min(colAll.length, filled.length)));
      colors = filled;
      warnings.push("mixed primitives with/without color channel; missing filled neutral");
    } else {
      colors = new Float32Array(colAll);
    }
  }
  let joints: Uint16Array | null = null;
  let weights: Float32Array | null = null;
  if (hasSkin && jointAll.length === vCount * 4) {
    joints = new Uint16Array(jointAll);
    weights = new Float32Array(weightAll);
  } else if (hasSkin) {
    warnings.push("partial skinning data ignored (primitive mismatch); rigid fallback");
  }

  return {
    mesh: {
      positions: new Float32Array(posAll),
      normals: new Float32Array(normAll),
      uvs: new Float32Array(uvAll),
      colors,
      colorComps,
      joints,
      weights,
      indices: new Uint32Array(idxAll),
      primitiveCount,
    },
    warnings,
  };
}

/** One-shot helper: buffer → decoded merged mesh. */
export function loadGlbMesh(buffer: ArrayBuffer): DecodeResult {
  return decodeGltfMeshes(parseGlbContainer(buffer));
}
