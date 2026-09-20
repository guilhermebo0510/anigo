/**
 * ANIGO P0-08 — project snapshot validation + character round-trip.
 * Run: node --experimental-strip-types --test tests/character/
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { parseProjectSnapshot } from "../../src/services/autosave_service.ts";
import {
  CHARACTER_SNAPSHOT_SCHEMA_VERSION,
  characterStatesEqual,
  createDefaultCharacterState,
  sanitizeCharacterState,
} from "../../src/services/character_state.ts";
import { CANONICAL_SLIDERS } from "../../src/services/morph_catalog.ts";

function validPayload(): Record<string, unknown> {
  const character = createDefaultCharacterState();
  // 50 altered morphs + non-default somatotype + fractional gender.
  for (let i = 0; i < 50; i++) {
    const def = CANONICAL_SLIDERS[i * 3];
    character.morphSliders[def.id] = def.min + (def.max - def.min) * 0.7;
  }
  character.somatotype = { endo: 0.5, meso: 0.3, ecto: 0.2 };
  character.genderDimorphism = 0.37;
  character.baseGender = "female";
  character.activePresetId = "plus_size";
  return {
    preset: "mannequin",
    headScale: 1.1,
    headRatio: 6.2,
    outlineWidth: 3.5,
    shadowThreshold: 0.5,
    lightDir: [0.5, 0.5, 0.5],
    lightIntensity: 1.2,
    shadowColor: [1, 1, 1],
    cameraEye: [0, 1.5, 3.5],
    cameraTarget: [0, 1, 0],
    timestamp: Date.now(),
    version: "0.2.0",
    schemaVersion: CHARACTER_SNAPSHOT_SCHEMA_VERSION,
    character,
  };
}

describe("P0-08 project snapshot validation", () => {
  it("save → load round-trips the full character (50 morphs, soma, gender 0.37)", () => {
    const payload = validPayload();
    const parsed = parseProjectSnapshot(JSON.stringify(payload));
    assert.ok(parsed.character);
    const expected = sanitizeCharacterState(payload["character"]);
    assert.ok(characterStatesEqual(parsed.character!, expected));
    assert.equal(parsed.character!.genderDimorphism, 0.37);
    assert.equal(Object.keys(parsed.character!.morphSliders).length, 50);
    assert.equal(parsed.schemaVersion, CHARACTER_SNAPSHOT_SCHEMA_VERSION);
  });

  it("rejects malformed JSON and non-object roots", () => {
    assert.throws(() => parseProjectSnapshot("{oops"), /malformado/);
    assert.throws(() => parseProjectSnapshot("[1,2]"), /objeto/);
    assert.throws(() => parseProjectSnapshot("42"), /objeto/);
  });

  it("rejects projects from a newer schema", () => {
    const payload = validPayload();
    payload["schemaVersion"] = 999;
    assert.throws(() => parseProjectSnapshot(JSON.stringify(payload)), /mais nova/);
  });

  it("migrates v1 payloads (no character) to the default character", () => {
    const payload = validPayload();
    delete payload["character"];
    delete payload["schemaVersion"];
    payload["version"] = "0.1.0";
    const parsed = parseProjectSnapshot(JSON.stringify(payload));
    assert.ok(parsed.character);
    assert.ok(characterStatesEqual(parsed.character!, createDefaultCharacterState()));
  });

  it("sanitizes hostile numerics instead of propagating NaN/garbage", () => {
    const payload = validPayload();
    payload["headScale"] = NaN; // JSON.stringify(NaN) → null → fallback
    payload["lightDir"] = ["x", null, {}];
    payload["preset"] = "death-star";
    (payload["character"] as Record<string, unknown>)["genderDimorphism"] = "lots";
    const parsed = parseProjectSnapshot(JSON.stringify(payload));
    assert.equal(parsed.headScale, 1.0);
    assert.deepEqual(parsed.lightDir, [0.577, 0.577, 0.577]);
    assert.equal(parsed.preset, "mannequin");
    assert.ok(Number.isFinite(parsed.character!.genderDimorphism));
  });

  it("drops unknown morph ids and clamps out-of-range values on load", () => {
    const payload = validPayload();
    const c = payload["character"] as Record<string, unknown>;
    (c["morphSliders"] as Record<string, unknown>)["bogus"] = 1;
    (c["morphSliders"] as Record<string, unknown>)["head_width"] = 1e9;
    const parsed = parseProjectSnapshot(JSON.stringify(payload));
    assert.equal(parsed.character!.morphSliders["bogus"], undefined);
    const def = CANONICAL_SLIDERS.find((s) => s.id === "head_width")!;
    assert.equal(parsed.character!.morphSliders["head_width"], def.max);
  });
});
