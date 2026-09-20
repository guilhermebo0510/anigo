/**
 * ANIGO P0 — character_state: gender polarity, sanitation, clamping, state shape.
 * Run: node --experimental-strip-types --test tests/character/
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  GENDER_ANDROGYNOUS,
  GENDER_FEMALE,
  GENDER_MALE,
  NEUTRAL_SOMATOTYPE,
  CHARACTER_SNAPSHOT_SCHEMA_VERSION,
  characterStatesEqual,
  clampCatalog,
  clampGenderDimorphism,
  clampNumber,
  createDefaultCharacterState,
  dimorphismToGender,
  findUnknownSliderIds,
  genderToDimorphism,
  getSliderDef,
  isKnownSliderId,
  isValidSomatotype,
  normalizeSomatotype,
  sanitizeCharacterState,
  sanitizeFinite,
} from "../../src/services/character_state.ts";

describe("P0-03 canonical gender polarity", () => {
  it("freezes 0.0=Female / 1.0=Male / 0.5=Androgynous", () => {
    assert.equal(GENDER_FEMALE, 0.0);
    assert.equal(GENDER_MALE, 1.0);
    assert.equal(GENDER_ANDROGYNOUS, 0.5);
  });

  it("genderToDimorphism maps male->1.0, female->0.0 (Rust-compatible)", () => {
    assert.equal(genderToDimorphism("male"), 1.0);
    assert.equal(genderToDimorphism("female"), 0.0);
  });

  it("dimorphismToGender round-trips {0, 0.25, 0.5, 0.75, 1}", () => {
    assert.equal(dimorphismToGender(0), "female");
    assert.equal(dimorphismToGender(0.25), "female");
    assert.equal(dimorphismToGender(0.5), "male");
    assert.equal(dimorphismToGender(0.75), "male");
    assert.equal(dimorphismToGender(1), "male");
  });

  it("clampGenderDimorphism rejects NaN/Inf/out-of-range", () => {
    assert.equal(clampGenderDimorphism(NaN), 0.5);
    assert.equal(clampGenderDimorphism(Infinity), 0.5);
    assert.equal(clampGenderDimorphism(undefined), 0.5);
    assert.equal(clampGenderDimorphism(-3), 0.0);
    assert.equal(clampGenderDimorphism(7), 1.0);
    assert.equal(clampGenderDimorphism(0.37), 0.37);
  });
});

describe("P0-02 numeric sanitation", () => {
  it("sanitizeFinite falls back on non-finite", () => {
    assert.equal(sanitizeFinite(1.5, 0), 1.5);
    assert.equal(sanitizeFinite(NaN, 0.25), 0.25);
    assert.equal(sanitizeFinite(Infinity, 9), 9);
    assert.equal(sanitizeFinite("x", 9), 9);
    assert.equal(sanitizeFinite(undefined, 9), 9);
    assert.equal(sanitizeFinite({}, 9), 9);
  });

  it("clampNumber clamps and never returns NaN", () => {
    assert.equal(clampNumber(5, 0, 1), 1);
    assert.equal(clampNumber(-5, 0, 1), 0);
    assert.equal(clampNumber(0.5, 0, 1), 0.5);
    assert.equal(clampNumber(NaN, 2, 8), 2);
  });

  it("NEUTRAL_SOMATOTYPE sums to 1", () => {
    const s = NEUTRAL_SOMATOTYPE.endo + NEUTRAL_SOMATOTYPE.meso + NEUTRAL_SOMATOTYPE.ecto;
    assert.ok(Math.abs(s - 1) < 1e-12);
  });

  it("normalizeSomatotype normalizes, clamps negatives, rescues (0,0,0)/NaN", () => {
    const n = normalizeSomatotype(2, 1, 1);
    assert.ok(Math.abs(n.endo + n.meso + n.ecto - 1) < 1e-12);
    assert.deepEqual([n.endo, n.meso, n.ecto], [0.5, 0.25, 0.25]);

    const z = normalizeSomatotype(0, 0, 0);
    assert.ok(Math.abs(z.endo + z.meso + z.ecto - 1) < 1e-12);

    const neg = normalizeSomatotype(-5, 1, 1);
    assert.equal(neg.endo, 0);
    assert.ok(Math.abs(neg.endo + neg.meso + neg.ecto - 1) < 1e-12);

    const nan = normalizeSomatotype(NaN, undefined, Infinity);
    assert.ok(Number.isFinite(nan.endo + nan.meso + nan.ecto));
    assert.ok(Math.abs(nan.endo + nan.meso + nan.ecto - 1) < 1e-12);
  });

  it("isValidSomatotype gates the barycentric domain", () => {
    assert.equal(isValidSomatotype(0.2, 0.3, 0.5), true);
    assert.equal(isValidSomatotype(0, 0, 0), false);
    assert.equal(isValidSomatotype(0.5, 0.5, 0.5), false);
    assert.equal(isValidSomatotype(NaN, 0.5, 0.5), false);
    assert.equal(isValidSomatotype(-0.1, 0.5, 0.6), false);
  });
});

describe("P0-09 catalog clamp + id validation", () => {
  it("clampCatalog clamps known ids to [min,max]", () => {
    const def = getSliderDef("head_width")!;
    assert.equal(clampCatalog("head_width", 99), def.max);
    assert.equal(clampCatalog("head_width", -99), def.min);
    assert.equal(clampCatalog("head_width", def.defaultValue), def.defaultValue);
  });

  it("clampCatalog falls back to default on NaN", () => {
    const def = getSliderDef("head_width")!;
    assert.equal(clampCatalog("head_width", NaN), def.defaultValue);
  });

  it("clampCatalog returns null for unknown ids (no orphan state)", () => {
    assert.equal(clampCatalog("calf_gastrocnemius_volume", 1), null);
    assert.equal(clampCatalog("ankle_achilles_definition", 1), null);
    assert.equal(clampCatalog("nope", 1), null);
  });

  it("isKnownSliderId / findUnknownSliderIds", () => {
    assert.equal(isKnownSliderId("eye_scale_uniform"), true);
    assert.equal(isKnownSliderId("bogus"), false);
    assert.deepEqual(findUnknownSliderIds(["head_width", "bogus", ""]), ["bogus", ""]);
  });
});

describe("P0-07/P0-08 canonical CharacterState", () => {
  it("default state is self-consistent", () => {
    const c = createDefaultCharacterState();
    assert.equal(c.schemaVersion, CHARACTER_SNAPSHOT_SCHEMA_VERSION);
    assert.equal(c.baseGender, "male");
    assert.equal(c.genderDimorphism, 1.0);
    assert.ok(isValidSomatotype(c.somatotype.endo, c.somatotype.meso, c.somatotype.ecto));
    assert.deepEqual(c.morphSliders, {});
  });

  it("sanitizeCharacterState never throws and drops garbage", () => {
    const c = sanitizeCharacterState({
      baseGender: "???",
      somatotype: { endo: NaN, meso: -1, ecto: 7 },
      genderDimorphism: Infinity,
      proportions: { headScale: "huge" },
      morphSliders: {
        head_width: 9999, // clamped
        bogus_slider: 1, // dropped
        eye_scale_uniform: NaN, // → default
      },
    });
    assert.equal(c.baseGender, "male");
    assert.ok(isValidSomatotype(c.somatotype.endo, c.somatotype.meso, c.somatotype.ecto));
    assert.ok(Number.isFinite(c.genderDimorphism));
    assert.equal(c.morphSliders["bogus_slider"], undefined);
    assert.equal(c.morphSliders["head_width"], getSliderDef("head_width")!.max);
    assert.equal(c.morphSliders["eye_scale_uniform"], getSliderDef("eye_scale_uniform")!.defaultValue);
  });

  it("sanitizeCharacterState(null) returns defaults", () => {
    assert.ok(characterStatesEqual(sanitizeCharacterState(null), createDefaultCharacterState()));
    assert.ok(characterStatesEqual(sanitizeCharacterState("x"), createDefaultCharacterState()));
  });

  it("characterStatesEqual compares structurally", () => {
    const a = createDefaultCharacterState();
    const b = createDefaultCharacterState();
    assert.equal(characterStatesEqual(a, b), true);
    b.morphSliders["head_width"] = 1.2;
    assert.equal(characterStatesEqual(a, b), false);
  });
});
