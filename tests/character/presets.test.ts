/**
 * ANIGO P0-01 — factory presets validate against the catalog contract.
 * Run: node --experimental-strip-types --test tests/character/
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  CANONICAL_CHARACTER_PRESETS,
  getCharacterPreset,
  validateCharacterPreset,
} from "../../src/services/character_presets.ts";
import { CANONICAL_SLIDERS } from "../../src/services/morph_catalog.ts";
import { isValidSomatotype } from "../../src/services/character_state.ts";

const catalog = {
  get: (id: string) => CANONICAL_SLIDERS.find((s) => s.id === id),
  isSomatotypeValid: (e: number, m: number, c: number) => isValidSomatotype(e, m, c),
};

describe("P0-01 factory preset contract", () => {
  it("ships exactly 6 presets with unique ids", () => {
    assert.equal(CANONICAL_CHARACTER_PRESETS.length, 6);
    const ids = CANONICAL_CHARACTER_PRESETS.map((p) => p.id);
    assert.equal(new Set(ids).size, 6);
  });

  it("every preset validates with zero issues (ids exist, in range, soma=1, polarity)", () => {
    for (const p of CANONICAL_CHARACTER_PRESETS) {
      const issues = validateCharacterPreset(p, catalog);
      assert.deepEqual(issues, [], `preset ${p.id} must be clean`);
    }
  });

  it("somatotype of every preset sums to 1", () => {
    for (const p of CANONICAL_CHARACTER_PRESETS) {
      const s = p.somatotype.endo + p.somatotype.meso + p.somatotype.ecto;
      assert.ok(Math.abs(s - 1) < 1e-9, p.id);
    }
  });

  it("getCharacterPreset resolves all ids", () => {
    for (const p of CANONICAL_CHARACTER_PRESETS) {
      assert.equal(getCharacterPreset(p.id)?.name, p.name);
    }
    assert.equal(getCharacterPreset("missing"), undefined);
  });

  it("validator catches contract drift (unknown id, out-of-range, bad polarity)", () => {
    const base = CANONICAL_CHARACTER_PRESETS[0];
    const badIds = validateCharacterPreset(
      { ...base, sliders: { ...base.sliders, bogus_slider: 1 } },
      catalog
    );
    assert.ok(badIds.some((i) => i.field === "sliders.bogus_slider"));

    const badRange = validateCharacterPreset(
      { ...base, sliders: { ...base.sliders, head_width: 999 } },
      catalog
    );
    assert.ok(badRange.some((i) => i.field === "sliders.head_width"));

    const badPolarity = validateCharacterPreset({ ...base, genderDimorphism: 0.0 }, catalog);
    assert.ok(badPolarity.some((i) => i.field === "genderDimorphism"));

    const badSoma = validateCharacterPreset(
      { ...base, somatotype: { endo: 0, meso: 0, ecto: 0 } },
      catalog
    );
    assert.ok(badSoma.some((i) => i.field === "somatotype"));
  });
});
