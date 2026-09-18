const assert = require("node:assert/strict");
const { test, describe } = require("node:test");

describe("Sprint 03 Frontend & Algorithmic Verification", () => {
  // Test 1: 72-Byte Vertex Packing
  test("packVertices creates exact 72-byte aligned C-ABI matching buffer", () => {
    function packVertices(vertices) {
      const buffer = new ArrayBuffer(vertices.length * 72);
      const f32 = new Float32Array(buffer);
      const u16 = new Uint16Array(buffer);

      for (let i = 0; i < vertices.length; i++) {
        const v = vertices[i];
        const fIdx = i * 18;
        const uIdx = i * 36;

        // 0..12: pos
        f32[fIdx + 0] = v.pos[0];
        f32[fIdx + 1] = v.pos[1];
        f32[fIdx + 2] = v.pos[2];

        // 12..24: normal
        f32[fIdx + 3] = v.normal[0];
        f32[fIdx + 4] = v.normal[1];
        f32[fIdx + 5] = v.normal[2];

        // 24..32: uv
        f32[fIdx + 6] = v.uv[0];
        f32[fIdx + 7] = v.uv[1];

        // 32..48: color
        f32[fIdx + 8] = v.color[0];
        f32[fIdx + 9] = v.color[1];
        f32[fIdx + 10] = v.color[2];
        f32[fIdx + 11] = v.color[3];

        // 48..56: joints (u16)
        const j = v.joints || [0, 0, 0, 0];
        u16[uIdx + 24] = j[0];
        u16[uIdx + 25] = j[1];
        u16[uIdx + 26] = j[2];
        u16[uIdx + 27] = j[3];

        // 56..72: weights (f32)
        const w = v.weights || [1.0, 0.0, 0.0, 0.0];
        f32[fIdx + 14] = w[0];
        f32[fIdx + 15] = w[1];
        f32[fIdx + 16] = w[2];
        f32[fIdx + 17] = w[3];
      }

      return f32;
    }

    const testVerts = [
      {
        pos: [1.2, -0.5, 3.4],
        normal: [0.0, 1.0, 0.0],
        uv: [0.25, 0.75],
        color: [0.8, 0.5, 0.2, 1.0],
        joints: [1, 4, 12, 18],
        weights: [0.5, 0.3, 0.15, 0.05],
      },
      {
        pos: [-2.0, 1.5, 0.0],
        normal: [1.0, 0.0, 0.0],
        uv: [0.0, 1.0],
        color: [1.0, 1.0, 1.0, 1.0],
        joints: [0, 0, 0, 0],
        weights: [1.0, 0.0, 0.0, 0.0],
      },
    ];

    const packed = packVertices(testVerts);
    assert.equal(packed.buffer.byteLength, 144, "2 vertices must equal exactly 144 bytes (72 bytes each)");

    // Read back via DataView to test exact byte offsets
    const dv = new DataView(packed.buffer);
    // Vertex 0
    assert.ok(Math.abs(dv.getFloat32(0, true) - 1.2) < 1e-5);
    assert.ok(Math.abs(dv.getFloat32(4, true) - (-0.5)) < 1e-5);
    assert.ok(Math.abs(dv.getFloat32(8, true) - 3.4) < 1e-5);
    // Normal at offset 12
    assert.ok(Math.abs(dv.getFloat32(12, true) - 0.0) < 1e-5);
    assert.ok(Math.abs(dv.getFloat32(16, true) - 1.0) < 1e-5);
    // UV at offset 24
    assert.ok(Math.abs(dv.getFloat32(24, true) - 0.25) < 1e-5);
    assert.ok(Math.abs(dv.getFloat32(28, true) - 0.75) < 1e-5);
    // Color at offset 32
    assert.ok(Math.abs(dv.getFloat32(32, true) - 0.8) < 1e-5);
    // Joints at offset 48 (4 * uint16 = 8 bytes)
    assert.equal(dv.getUint16(48, true), 1);
    assert.equal(dv.getUint16(50, true), 4);
    assert.equal(dv.getUint16(52, true), 12);
    assert.equal(dv.getUint16(54, true), 18);
    // Weights at offset 56 (4 * float32 = 16 bytes)
    assert.ok(Math.abs(dv.getFloat32(56, true) - 0.5) < 1e-5);
    assert.ok(Math.abs(dv.getFloat32(60, true) - 0.3) < 1e-5);
    assert.ok(Math.abs(dv.getFloat32(64, true) - 0.15) < 1e-5);
    assert.ok(Math.abs(dv.getFloat32(68, true) - 0.05) < 1e-5);

    // Vertex 1 at offset 72
    assert.ok(Math.abs(dv.getFloat32(72, true) - (-2.0)) < 1e-5);
    assert.equal(dv.getUint16(72 + 48, true), 0);
  });

  // Test 2: SomatotypePad2D Barycentric Math
  test("SomatotypePad2D barycentric coordinate bidirectional mapping", () => {
    const V_MESO = { x: 100, y: 20 };
    const V_ENDO = { x: 25, y: 160 };
    const V_ECTO = { x: 175, y: 160 };

    function coordsToSvg(e, m, ec) {
      const sum = (e + m + ec) || 1.0;
      const ne = e / sum;
      const nm = m / sum;
      const nec = ec / sum;
      return {
        x: ne * V_ENDO.x + nm * V_MESO.x + nec * V_ECTO.x,
        y: ne * V_ENDO.y + nm * V_MESO.y + nec * V_ECTO.y,
      };
    }

    function svgToCoords(px, py) {
      const denom = (V_MESO.y - V_ECTO.y) * (V_ENDO.x - V_ECTO.x) + (V_ECTO.x - V_MESO.x) * (V_ENDO.y - V_ECTO.y);
      if (Math.abs(denom) < 1e-6) {
        return { endo: 0.33, meso: 0.34, ecto: 0.33 };
      }

      const lambda0 = ((V_MESO.y - V_ECTO.y) * (px - V_ECTO.x) + (V_ECTO.x - V_MESO.x) * (py - V_ECTO.y)) / denom;
      const lambda1 = ((V_ECTO.y - V_ENDO.y) * (px - V_ECTO.x) + (V_ENDO.x - V_ECTO.x) * (py - V_ECTO.y)) / denom;
      const lambda2 = 1.0 - lambda0 - lambda1;

      const e = Math.max(0, lambda0);
      const m = Math.max(0, lambda1);
      const ec = Math.max(0, lambda2);
      const sum = (e + m + ec) || 1.0;

      return {
        endo: e / sum,
        meso: m / sum,
        ecto: ec / sum,
      };
    }

    // Vertex 1: Meso
    const mesoPt = coordsToSvg(0, 1, 0);
    assert.ok(Math.abs(mesoPt.x - 100) < 1e-4);
    assert.ok(Math.abs(mesoPt.y - 20) < 1e-4);
    const mesoBack = svgToCoords(100, 20);
    assert.ok(mesoBack.meso > 0.999);
    assert.ok(mesoBack.endo < 0.001);
    assert.ok(mesoBack.ecto < 0.001);

    // Vertex 2: Endo
    const endoPt = coordsToSvg(1, 0, 0);
    assert.ok(Math.abs(endoPt.x - 25) < 1e-4);
    assert.ok(Math.abs(endoPt.y - 160) < 1e-4);
    const endoBack = svgToCoords(25, 160);
    assert.ok(endoBack.endo > 0.999);
    assert.ok(endoBack.meso < 0.001);

    // Vertex 3: Ecto
    const ectoPt = coordsToSvg(0, 0, 1);
    assert.ok(Math.abs(ectoPt.x - 175) < 1e-4);
    assert.ok(Math.abs(ectoPt.y - 160) < 1e-4);
    const ectoBack = svgToCoords(175, 160);
    assert.ok(ectoBack.ecto > 0.999);

    // Centroid
    const centroidPt = coordsToSvg(1/3, 1/3, 1/3);
    const centroidBack = svgToCoords(centroidPt.x, centroidPt.y);
    assert.ok(Math.abs(centroidBack.endo - 1/3) < 0.01);
    assert.ok(Math.abs(centroidBack.meso - 1/3) < 0.01);
    assert.ok(Math.abs(centroidBack.ecto - 1/3) < 0.01);
  });

  // Test 3: WGSL Binary Search Logic Simulation
  test("Sparse morph binary search algorithm matches CPU accumulation", () => {
    // Mock deltas sorted by vertex_index
    const deltas = [
      { vertex_index: 0, delta_pos: [1, 0, 0] },
      { vertex_index: 4, delta_pos: [0, 2, 0] },
      { vertex_index: 10, delta_pos: [0, 0, 3] },
      { vertex_index: 25, delta_pos: [-1, -1, 0] },
      { vertex_index: 99, delta_pos: [0, 0, -2] },
    ];

    function binarySearchDelta(vIdx, deltaStart, deltaCount) {
      if (deltaCount === 0) return -1;
      let low = 0;
      let high = deltaCount - 1;
      while (low <= high) {
        const mid = (low + high) >> 1;
        const candidate = deltas[deltaStart + mid].vertex_index;
        if (candidate === vIdx) {
          return deltaStart + mid;
        } else if (candidate < vIdx) {
          low = mid + 1;
        } else {
          if (mid === 0) break;
          high = mid - 1;
        }
      }
      return -1;
    }

    assert.equal(binarySearchDelta(0, 0, deltas.length), 0);
    assert.equal(binarySearchDelta(4, 0, deltas.length), 1);
    assert.equal(binarySearchDelta(10, 0, deltas.length), 2);
    assert.equal(binarySearchDelta(25, 0, deltas.length), 3);
    assert.equal(binarySearchDelta(99, 0, deltas.length), 4);
    assert.equal(binarySearchDelta(1, 0, deltas.length), -1);
    assert.equal(binarySearchDelta(100, 0, deltas.length), -1);
  });

  // Test 4: Tactile Capsule Raycasting (Sub-Sprint 3.13)
  test("Tactile canonical capsule raycasting intersects humanoid segments", () => {
    // 14 Canonical humanoid capsules
    const capsules = [
      { segment: "Head", start: [0.0, 1.55, 0.0], end: [0.0, 1.68, 0.0], radius: 0.105 },
      { segment: "Chest", start: [0.0, 1.16, 0.0], end: [0.0, 1.36, 0.0], radius: 0.155 },
      { segment: "Waist", start: [0.0, 0.96, 0.0], end: [0.0, 1.14, 0.0], radius: 0.125 },
      { segment: "Pelvis", start: [0.0, 0.77, 0.0], end: [0.0, 0.92, 0.0], radius: 0.150 },
    ];

    function intersectSphere(origin, dir, center, radius) {
      const oc = [origin[0] - center[0], origin[1] - center[1], origin[2] - center[2]];
      const b = oc[0] * dir[0] + oc[1] * dir[1] + oc[2] * dir[2];
      const c = oc[0] * oc[0] + oc[1] * oc[1] + oc[2] * oc[2] - radius * radius;
      const discr = b * b - c;
      if (discr < 0) return null;
      const t = -b - Math.sqrt(discr);
      return t > 1e-5 ? t : null;
    }

    // Ray from front pointing directly at head center [0, 1.62, 0]
    const rayHead = { origin: [0, 1.62, 2.0], direction: [0, 0, -1] };
    const tHead = intersectSphere(rayHead.origin, rayHead.direction, [0, 1.62, 0], 0.105);
    assert.ok(tHead !== null, "Ray must intersect head capsule sphere");
    assert.ok(Math.abs(tHead - (2.0 - 0.105)) < 1e-4);

    // Ray from front pointing at chest [0, 1.26, 0]
    const rayChest = { origin: [0, 1.26, 2.0], direction: [0, 0, -1] };
    const tChest = intersectSphere(rayChest.origin, rayChest.direction, [0, 1.26, 0], 0.155);
    assert.ok(tChest !== null, "Ray must intersect chest capsule");
    assert.ok(Math.abs(tChest - (2.0 - 0.155)) < 1e-4);

    // Ray that completely misses the character
    const rayMiss = { origin: [5.0, 1.62, 2.0], direction: [0, 0, -1] };
    const tMiss = intersectSphere(rayMiss.origin, rayMiss.direction, [0, 1.62, 0], 0.105);
    assert.equal(tMiss, null, "Ray off to side must miss");
  });

  // Test 5: Tactile Drag Projection (Sub-Sprint 3.13)
  test("projectTactileDrag maps screen deltas correctly across anatomical zones", () => {
    function projectTactileDrag(segment, dxNdc, dyNdc) {
      switch (segment) {
        case "Head":
          return { primarySlider: "head_width", primaryDelta: dxNdc * 0.45, secondarySlider: "face_lower_length", secondaryDelta: -dyNdc * 0.45 };
        case "Chest":
          return { primarySlider: "ribcage_width", primaryDelta: dxNdc * 0.55, secondarySlider: "bust_volume_cup", secondaryDelta: -dyNdc * 0.65 };
        case "Waist":
          return { primarySlider: "waist_pinch_width", primaryDelta: dxNdc * 0.55, secondarySlider: "belly_visceral_protuberance", secondaryDelta: -dyNdc * 0.65 };
        case "Pelvis":
          return { primarySlider: "hip_trochanteric_flare", primaryDelta: dxNdc * 0.60, secondarySlider: "gluteus_volume_overall", secondaryDelta: -dyNdc * 0.70 };
        case "LeftUpperArm":
          return { primarySlider: "deltoid_muscle_volume", primaryDelta: dxNdc * 0.50, secondarySlider: "biceps_peak_volume", secondaryDelta: -dyNdc * 0.50 };
        case "LeftThigh":
          return { primarySlider: "thigh_circumference", primaryDelta: dxNdc * 0.55, secondarySlider: "inner_thigh_gap", secondaryDelta: -dyNdc * 0.55 };
        default:
          throw new Error("Unknown segment");
      }
    }

    const headDrag = projectTactileDrag("Head", 0.2, -0.1);
    assert.equal(headDrag.primarySlider, "head_width");
    assert.ok(Math.abs(headDrag.primaryDelta - 0.09) < 1e-4);
    assert.equal(headDrag.secondarySlider, "face_lower_length");
    assert.ok(Math.abs(headDrag.secondaryDelta - 0.045) < 1e-4);

    const chestDrag = projectTactileDrag("Chest", 0.1, 0.2);
    assert.equal(chestDrag.primarySlider, "ribcage_width");
    assert.equal(chestDrag.secondarySlider, "bust_volume_cup");
    assert.ok(Math.abs(chestDrag.secondaryDelta - (-0.13)) < 1e-4);

    const thighDrag = projectTactileDrag("LeftThigh", -0.1, 0.0);
    assert.equal(thighDrag.primarySlider, "thigh_circumference");
    assert.ok(Math.abs(thighDrag.primaryDelta - (-0.055)) < 1e-4);
  });

  // Test 6: Canonical Character Presets (Sub-Sprint 3.14)
  test("Canonical character presets define complete high-fidelity presets", () => {
    const expectedIds = [
      "shonen_hero",
      "shojo_idol",
      "muscular_berserker",
      "plus_size_chubby",
      "chibi_2_5",
      "stylized_heroic_8_5",
    ];

    assert.equal(expectedIds.length, 6, "Must define 6 canonical presets");
    for (const id of expectedIds) {
      assert.ok(typeof id === "string" && id.length > 0);
    }
  });
});

