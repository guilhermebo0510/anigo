/**
 * ANIGO P0-04/P0-05/P0-06 — generic morph engine coverage, distinctness,
 * normal recomputation and sparse-set round-trip.
 * Run: node --experimental-strip-types --test tests/character/
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { CANONICAL_SLIDERS } from "../../src/services/morph_catalog.ts";
import {
  applySparseCpu,
  buildSparseMorphSet,
  channelDenominator,
  genericMorphDelta,
  normalizeWeight,
  packChannelWeights,
  recomputeNormals,
  type MeshBounds,
} from "../reference/morph_engine.ts";

// ---------------------------------------------------------------------------
// Synthetic body-like probe cloud covering every zone region.
// ---------------------------------------------------------------------------

const BOUNDS: MeshBounds = { minY: 0, maxY: 1.7 };

function buildProbeCloud(): Array<{ x: number; y: number; z: number; nx: number; ny: number; nz: number }> {
  const pts: Array<{ x: number; y: number; z: number; nx: number; ny: number; nz: number }> = [];
  const xs: number[] = [];
  for (let x = -0.3; x <= 0.3001; x += 0.05) xs.push(Number(x.toFixed(4)));
  const ys: number[] = [];
  for (let i = 0; i <= 50; i++) ys.push((i / 50) * 1.7);
  const zs = [-0.15, -0.05, 0.05, 0.15];
  for (const y of ys) {
    for (const x of xs) {
      for (const z of zs) {
        const l = Math.hypot(x, 0.5, z) || 1;
        pts.push({ x, y, z, nx: x / l, ny: 0.5 / l, nz: z / l });
      }
    }
  }
  return pts;
}

const CLOUD = buildProbeCloud();

function deltasForSlider(
  sliderId: string,
  normWeight: number
): Float64Array {
  const def = CANONICAL_SLIDERS.find((s) => s.id === sliderId)!;
  const out = new Float64Array(CLOUD.length * 3);
  const tmp = { dx: 0, dy: 0, dz: 0 };
  for (let i = 0; i < CLOUD.length; i++) {
    const p = CLOUD[i];
    const d = genericMorphDelta(def, normWeight, p.x, p.y, p.z, p.nx, p.ny, p.nz, BOUNDS, tmp);
    if (d) {
      out[i * 3] = d.dx;
      out[i * 3 + 1] = d.dy;
      out[i * 3 + 2] = d.dz;
    }
  }
  return out;
}

function movedCount(d: Float64Array, eps = 1e-4): number {
  let n = 0;
  for (let i = 0; i < d.length; i += 3) {
    if (Math.abs(d[i]) + Math.abs(d[i + 1]) + Math.abs(d[i + 2]) > eps) n++;
  }
  return n;
}

describe("P0-04 weight normalization", () => {
  it("maps default->0, max->+1, min->-1", () => {
    const def = CANONICAL_SLIDERS.find((s) => s.id === "head_width")!;
    assert.equal(normalizeWeight(def, def.defaultValue), 0);
    assert.equal(normalizeWeight(def, def.max), 1);
    assert.equal(normalizeWeight(def, def.min), -1);
  });

  it("handles edge-default sliders (default == min) without div-by-zero", () => {
    const def = CANONICAL_SLIDERS.find((s) => s.id === "somatotype_endomorph")!;
    assert.equal(normalizeWeight(def, def.defaultValue), 0);
    assert.equal(normalizeWeight(def, def.max), 1);
    assert.ok(Number.isFinite(channelDenominator(def)));
    assert.ok(channelDenominator(def) > 0);
  });

  it("every slider has a positive channel denominator", () => {
    for (const s of CANONICAL_SLIDERS) {
      assert.ok(channelDenominator(s) > 0, s.id);
    }
  });
});

describe("P0-04 generic coverage: all 157 sliders deform", () => {
  it("catalog holds 157 sliders", () => {
    assert.equal(CANONICAL_SLIDERS.length, 157);
  });

  it("every slider moves >= 8 probe verts at max weight", () => {
    const inert: string[] = [];
    for (const s of CANONICAL_SLIDERS) {
      const d = deltasForSlider(s.id, 1.0);
      if (movedCount(d) < 8) inert.push(`${s.id}(${movedCount(d)})`);
    }
    assert.deepEqual(inert, []);
  });

  it("every slider moves >= 8 probe verts at min weight", () => {
    const inert: string[] = [];
    for (const s of CANONICAL_SLIDERS) {
      const d = deltasForSlider(s.id, -1.0);
      if (movedCount(d) < 8) inert.push(`${s.id}(${movedCount(d)})`);
    }
    assert.deepEqual(inert, []);
  });

  it("zero weight moves nothing (sparsity)", () => {
    for (const s of CANONICAL_SLIDERS.slice(0, 20)) {
      const d = deltasForSlider(s.id, 0);
      assert.equal(movedCount(d, 0), 0, s.id);
    }
  });

  it("deltas within each zone are pairwise distinct (no generic-copy clones)", () => {
    const byZone = new Map<string, string[]>();
    for (const s of CANONICAL_SLIDERS) {
      const list = byZone.get(s.zoneKey) ?? [];
      list.push(s.id);
      byZone.set(s.zoneKey, list);
    }
    const clones: string[] = [];
    for (const [zone, ids] of byZone) {
      const vecs = ids.map((id) => deltasForSlider(id, 1.0));
      for (let a = 0; a < ids.length; a++) {
        for (let b = a + 1; b < ids.length; b++) {
          let diff = 0;
          const va = vecs[a];
          const vb = vecs[b];
          for (let i = 0; i < va.length; i++) diff += Math.abs(va[i] - vb[i]);
          if (!(diff > 1e-9)) clones.push(`${zone}: ${ids[a]} == ${ids[b]}`);
        }
      }
    }
    assert.deepEqual(clones, []);
  });
});

describe("P0-06 normal recomputation", () => {
  // 3x3 grid in XZ plane, single quad strip triangulation.
  function gridMesh(): { positions: Float32Array; indices: Uint32Array; normals: Float32Array } {
    const n = 3;
    const positions = new Float32Array(n * n * 3);
    for (let iz = 0; iz < n; iz++) {
      for (let ix = 0; ix < n; ix++) {
        const i = iz * n + ix;
        positions[i * 3] = ix - 1;
        positions[i * 3 + 1] = 0;
        positions[i * 3 + 2] = iz - 1;
      }
    }
    const idx: number[] = [];
    for (let iz = 0; iz < n - 1; iz++) {
      for (let ix = 0; ix < n - 1; ix++) {
        const a = iz * n + ix;
        const b = a + 1;
        const c = a + n;
        const d = c + 1;
        idx.push(a, c, b, b, c, d);
      }
    }
    return { positions, indices: new Uint32Array(idx), normals: new Float32Array(n * n * 3) };
  }

  it("produces unit normals on a flat grid", () => {
    const { positions, indices, normals } = gridMesh();
    recomputeNormals(positions, indices, normals);
    for (let i = 0; i < normals.length; i += 3) {
      const l = Math.hypot(normals[i], normals[i + 1], normals[i + 2]);
      assert.ok(Math.abs(l - 1) < 1e-5, `normal ${i / 3} not unit: ${l}`);
    }
  });

  it("normals react to deformation (tent pole raises center)", () => {
    const flat = gridMesh();
    recomputeNormals(flat.positions, flat.indices, flat.normals);
    const tent = gridMesh();
    tent.positions[4 * 3 + 1] = 1.0; // center vertex up
    recomputeNormals(tent.positions, tent.indices, tent.normals);
    let changed = 0;
    for (let i = 0; i < flat.normals.length; i += 3) {
      const d =
        Math.abs(flat.normals[i] - tent.normals[i]) +
        Math.abs(flat.normals[i + 1] - tent.normals[i + 1]) +
        Math.abs(flat.normals[i + 2] - tent.normals[i + 2]);
      if (d > 1e-3) changed++;
    }
    assert.ok(changed >= 5, `only ${changed} normals reacted`);
  });

  it("ignores degenerate triangles and out-of-range indices", () => {
    const positions = new Float32Array([0, 0, 0, 1, 0, 0, 0, 0, 1]);
    const normals = new Float32Array([0, 1, 0, 0, 1, 0, 0, 1, 0]);
    recomputeNormals(positions, new Uint32Array([0, 0, 0, 99, 100, 101]), normals);
    // untouched (degenerate/out-of-range contribute nothing, no NaN)
    for (const v of normals) assert.ok(Number.isFinite(v));
  });
});

describe("P0-05 sparse morph set (WGSL-compatible)", () => {
  it("builds vertex-ordered deltas with bit-exact indices and round-trips", () => {
    const totalVertices = 4;
    const basePos = new Float32Array([0, 0, 0, 1, 0, 0, 2, 0, 0, 3, 0, 0]);
    const baseNorm = new Float32Array([0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0]);
    const chA = {
      sliderId: "slider_a",
      positions: new Float32Array([0, 0.5, 0, 1, 0, 0, 2, 0, 0.25, 3, 0, 0]),
      normals: new Float32Array(baseNorm),
    };
    const chB = {
      sliderId: "slider_b",
      positions: new Float32Array(basePos),
      normals: new Float32Array([0, 1, 0, 0.1, 1, 0, 0, 1, 0, 0, 1, 0]),
    };
    const set = buildSparseMorphSet(totalVertices, basePos, baseNorm, [chA, chB]);
    assert.equal(set.channels.length, 2);
    assert.equal(set.totalVertices, 4);
    assert.equal(set.totalDeltas, 3); // A: v0+v2, B: v1
    assert.deepEqual(
      set.channels.map((c) => [c.sliderId, c.startOffset, c.deltaCount]),
      [
        ["slider_a", 0, 2],
        ["slider_b", 2, 1],
      ]
    );

    // indices bit-exact through the u32 view
    const u32 = new Uint32Array(set.deltasF32.buffer);
    assert.deepEqual([u32[0], u32[8], u32[16]], [0, 2, 1]);

    // CPU reference accumulation matches manual math
    const outP = new Float32Array(12);
    const outN = new Float32Array(12);
    applySparseCpu(basePos, baseNorm, set, { slider_a: 2.0, slider_b: 1.0 }, outP, outN);
    assert.ok(Math.abs(outP[0 * 3 + 1] - 1.0) < 1e-6); // v0.y = 0 + 2*0.5
    assert.ok(Math.abs(outP[2 * 3 + 2] - 0.5) < 1e-6); // v2.z = 0 + 2*0.25
    assert.ok(Math.abs(outN[1 * 3] - 0.1 / Math.hypot(0.1, 1)) < 1e-5); // normalized
  });

  it("packChannelWeights stores weight + bit-exact offsets", () => {
    const set = buildSparseMorphSet(
      2,
      new Float32Array(6),
      new Float32Array([0, 1, 0, 0, 1, 0]),
      [
        {
          sliderId: "a",
          positions: new Float32Array([0.5, 0, 0, 0, 0, 0]),
          normals: new Float32Array([0, 1, 0, 0, 1, 0]),
        },
      ]
    );
    const packed = packChannelWeights(set, new Map([["a", 0.75]]));
    assert.equal(packed.length, 4);
    assert.ok(Math.abs(packed[0] - 0.75) < 1e-9);
    const u = new Uint32Array(packed.buffer);
    assert.equal(u[1], 0); // start_offset
    assert.equal(u[2], 1); // delta_count
  });

  it("empty channels survive (zero deltas, still addressable)", () => {
    const base = new Float32Array([1, 2, 3]);
    const nrm = new Float32Array([0, 1, 0]);
    const set = buildSparseMorphSet(1, base, nrm, [
      { sliderId: "still", positions: new Float32Array(base), normals: new Float32Array(nrm) },
    ]);
    assert.equal(set.totalDeltas, 0);
    assert.equal(set.channels[0].deltaCount, 0);
    const outP = new Float32Array(3);
    const outN = new Float32Array(3);
    applySparseCpu(base, nrm, set, { still: 5 }, outP, outN);
    assert.deepEqual(Array.from(outP), [1, 2, 3]);
  });
});
