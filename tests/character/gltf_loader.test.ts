/**
 * ANIGO P0-10 — validating GLB loader: container checks, multi-primitive
 * merge, quantized attributes, node transforms, skinning preservation.
 * Run: node --experimental-strip-types --test tests/character/
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  GlbParseError,
  decodeAccessorIndices,
  loadGlbMesh,
  parseGlbContainer,
} from "../../src/services/gltf_loader.ts";

// ---------------------------------------------------------------------------
// Minimal GLB builder (single triangle + optional extras)
// ---------------------------------------------------------------------------

interface TestAssetOpts {
  translation?: [number, number, number];
  twoPrimitives?: boolean;
  colorsU8?: boolean;
  skinned?: boolean;
}

function buildTestGlb(opts: TestAssetOpts = {}): ArrayBuffer {
  const positions = new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]);
  const normals = new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1]);
  const indices = new Uint16Array([0, 1, 2]);

  const binParts: Uint8Array[] = [];
  const push = (u8: Uint8Array) => {
    binParts.push(u8);
    return binParts.reduce((a, b) => a + b.length, 0) - u8.length;
  };
  const offPos = push(new Uint8Array(positions.buffer));
  const offNorm = push(new Uint8Array(normals.buffer));
  const offIdx = push(new Uint8Array(indices.buffer));

  const bufferViews: Array<{ buffer: number; byteOffset: number; byteLength: number }> = [
    { buffer: 0, byteOffset: offPos, byteLength: positions.byteLength },
    { buffer: 0, byteOffset: offNorm, byteLength: normals.byteLength },
    { buffer: 0, byteOffset: offIdx, byteLength: indices.byteLength },
  ];
  const accessors: Array<Record<string, unknown>> = [
    { bufferView: 0, componentType: 5126, count: 3, type: "VEC3" },
    { bufferView: 1, componentType: 5126, count: 3, type: "VEC3" },
    { bufferView: 2, componentType: 5123, count: 3, type: "SCALAR" },
  ];

  const attributes: Record<string, number> = { POSITION: 0, NORMAL: 1 };

  if (opts.colorsU8) {
    const colors = new Uint8Array([255, 128, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255]);
    const off = push(colors);
    bufferViews.push({ buffer: 0, byteOffset: off, byteLength: colors.length });
    accessors.push({ bufferView: 3, componentType: 5121, normalized: true, count: 3, type: "VEC4" });
    attributes["COLOR_0"] = 3;
  }

  if (opts.skinned) {
    const joints = new Uint16Array([1, 2, 0, 0, 3, 0, 0, 0, 5, 6, 7, 8]);
    const weights = new Float32Array([0.5, 0.5, 0, 0, 1, 0, 0, 0, 0.25, 0.25, 0.25, 0.25]);
    const offJ = push(new Uint8Array(joints.buffer));
    const offW = push(new Uint8Array(weights.buffer));
    const bvJ = bufferViews.length;
    bufferViews.push({ buffer: 0, byteOffset: offJ, byteLength: joints.byteLength });
    const bvW = bufferViews.length;
    bufferViews.push({ buffer: 0, byteOffset: offW, byteLength: weights.byteLength });
    const accJ = accessors.length;
    accessors.push({ bufferView: bvJ, componentType: 5123, count: 3, type: "VEC4" });
    const accW = accessors.length;
    accessors.push({ bufferView: bvW, componentType: 5126, count: 3, type: "VEC4" });
    attributes["JOINTS_0"] = accJ;
    attributes["WEIGHTS_0"] = accW;
  }

  const primitives: Array<Record<string, unknown>> = [{ attributes: { ...attributes }, indices: 2 }];
  if (opts.twoPrimitives) {
    primitives.push({ attributes: { ...attributes }, indices: 2 });
  }

  const nodes: Array<Record<string, unknown>> = [{ mesh: 0, name: "root" }];
  if (opts.translation) nodes[0].translation = opts.translation;

  const json = {
    asset: { version: "2.0" },
    scenes: [{ nodes: [0] }],
    scene: 0,
    nodes,
    meshes: [{ primitives, name: "tri" }],
    accessors,
    bufferViews,
    buffers: [{ byteLength: binParts.reduce((a, b) => a + b.length, 0) }],
  };

  const jsonBytes = new TextEncoder().encode(JSON.stringify(json));
  const jsonPadded = new Uint8Array(Math.ceil(jsonBytes.length / 4) * 4).fill(0x20);
  jsonPadded.set(jsonBytes);
  const binLen = binParts.reduce((a, b) => a + b.length, 0);
  const binPadded = Math.ceil(binLen / 4) * 4;
  const total = 12 + 8 + jsonPadded.length + 8 + binPadded;
  const out = new ArrayBuffer(total);
  const dv = new DataView(out);
  dv.setUint32(0, 0x46546c67, true);
  dv.setUint32(4, 2, true);
  dv.setUint32(8, total, true);
  dv.setUint32(12, jsonPadded.length, true);
  dv.setUint32(16, 0x4e4f534a, true);
  new Uint8Array(out, 20, jsonPadded.length).set(jsonPadded);
  const binOff = 20 + jsonPadded.length;
  dv.setUint32(binOff, binPadded, true);
  dv.setUint32(binOff + 4, 0x004e4942, true);
  const binDst = new Uint8Array(out, binOff + 8, binPadded);
  let at = 0;
  for (const p of binParts) {
    binDst.set(p, at);
    at += p.length;
  }
  return out;
}

describe("P0-10 GLB container validation", () => {
  it("parses a well-formed asset", () => {
    const c = parseGlbContainer(buildTestGlb());
    assert.equal(c.json.meshes!.length, 1);
    assert.ok(c.bin.byteLength > 0);
  });

  it("rejects bad magic", () => {
    const buf = buildTestGlb();
    new DataView(buf).setUint32(0, 0xdeadbeef, true);
    assert.throws(() => parseGlbContainer(buf), (e: unknown) => e instanceof GlbParseError && e.code === "BAD_MAGIC");
  });

  it("rejects bad version", () => {
    const buf = buildTestGlb();
    new DataView(buf).setUint32(4, 1, true);
    assert.throws(() => parseGlbContainer(buf), (e: unknown) => e instanceof GlbParseError && e.code === "BAD_VERSION");
  });

  it("rejects truncated buffers", () => {
    assert.throws(() => parseGlbContainer(new ArrayBuffer(8)), (e: unknown) => e instanceof GlbParseError);
  });
});

describe("P0-10 mesh decode", () => {
  it("decodes positions/normals/indices of a single primitive", () => {
    const { mesh, warnings } = loadGlbMesh(buildTestGlb());
    assert.equal(mesh.positions.length, 9);
    assert.equal(mesh.indices.length, 3);
    assert.equal(mesh.primitiveCount, 1);
    assert.deepEqual(Array.from(mesh.indices), [0, 1, 2]);
    assert.deepEqual(Array.from(mesh.positions.slice(0, 3)), [0, 0, 0]);
    assert.deepEqual(warnings, []);
  });

  it("applies node translation to positions (not silently dropped)", () => {
    const { mesh } = loadGlbMesh(buildTestGlb({ translation: [10, 0, 0] }));
    assert.deepEqual(Array.from(mesh.positions.slice(0, 3)), [10, 0, 0]);
  });

  it("merges multiple primitives with index offsets (no truncation)", () => {
    const { mesh } = loadGlbMesh(buildTestGlb({ twoPrimitives: true }));
    assert.equal(mesh.primitiveCount, 2);
    assert.equal(mesh.positions.length, 18);
    assert.deepEqual(Array.from(mesh.indices), [0, 1, 2, 3, 4, 5]);
  });

  it("decodes normalized UBYTE colors to float (VRM-style quantized)", () => {
    const { mesh } = loadGlbMesh(buildTestGlb({ colorsU8: true }));
    assert.ok(mesh.colors !== null);
    assert.equal(mesh.colorComps, 4);
    assert.ok(Math.abs(mesh.colors![0] - 1.0) < 1e-6);
    assert.ok(Math.abs(mesh.colors![1] - 128 / 255) < 1e-6);
  });

  it("preserves skins: real joints + normalized weights (rig not destroyed)", () => {
    const { mesh } = loadGlbMesh(buildTestGlb({ skinned: true }));
    assert.ok(mesh.joints !== null && mesh.weights !== null);
    assert.deepEqual(Array.from(mesh.joints!.slice(0, 4)), [1, 2, 0, 0]);
    for (let v = 0; v < 3; v++) {
      const s =
        mesh.weights![v * 4] + mesh.weights![v * 4 + 1] + mesh.weights![v * 4 + 2] + mesh.weights![v * 4 + 3];
      assert.ok(Math.abs(s - 1) < 1e-6, `vertex ${v} weights sum to ${s}`);
    }
  });

  it("indices decode supports USHORT", () => {
    const c = parseGlbContainer(buildTestGlb());
    const idx = decodeAccessorIndices(c.json as never, c.bin, 2);
    assert.deepEqual(Array.from(idx), [0, 1, 2]);
  });
});
