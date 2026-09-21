/**
 * ANIGO P0-09 — tactile engine emits only catalog-valid slider ids.
 * Run: node --experimental-strip-types --test tests/character/
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  assertTactileIdsValid,
  projectTactileDrag,
  type AnatomicalSegment,
} from "../../src/components/viewport/tactile.ts";
import { isKnownSliderId } from "../../src/services/character_state.ts";

const SEGMENTS: AnatomicalSegment[] = [
  "Head", "Neck", "Chest", "Waist", "Pelvis",
  "LeftUpperArm", "RightUpperArm", "LeftForearm", "RightForearm",
  "LeftThigh", "RightThigh", "LeftCalf", "RightCalf",
];

describe("P0-09 tactile slider contract", () => {
  it("zero orphan slider ids across all 13 segments", () => {
    assertTactileIdsValid();
  });

  it("every segment maps to known catalog ids with finite deltas", () => {
    for (const seg of SEGMENTS) {
      const r = projectTactileDrag(seg, 0.25, -0.4);
      assert.equal(isKnownSliderId(r.primarySlider), true, seg);
      assert.ok(Number.isFinite(r.primaryDelta), seg);
      if (r.secondarySlider) {
        assert.equal(isKnownSliderId(r.secondarySlider), true, seg);
        assert.ok(Number.isFinite(r.secondaryDelta), seg);
      }
    }
  });

  it("calf segments use canonical LowerLimbs sliders (orphan fix)", () => {
    for (const seg of ["LeftCalf", "RightCalf"] as AnatomicalSegment[]) {
      const r = projectTactileDrag(seg, 0.5, 0.5);
      assert.equal(r.primarySlider, "calf_circumference");
      assert.equal(r.secondarySlider, "gastrocnemius_height");
    }
  });

  it("drag projection is linear and sign-consistent", () => {
    const a = projectTactileDrag("Head", 0.2, 0);
    const b = projectTactileDrag("Head", -0.2, 0);
    assert.ok(Math.abs(a.primaryDelta + b.primaryDelta) < 1e-12);
    assert.ok(Math.abs(a.primaryDelta) > 0);
  });
});
