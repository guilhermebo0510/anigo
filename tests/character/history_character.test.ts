/**
 * ANIGO P0-07 — undo/redo covers the Personagem domain (round-trip exact).
 * Run: node --experimental-strip-types --test tests/character/
 */
import { describe, it, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { historyService, type HistoryStateSnapshot } from "../../src/services/history_service.ts";
import {
  characterStatesEqual,
  createDefaultCharacterState,
  type CharacterState,
} from "../../src/services/character_state.ts";

function baseSnapshot(character: CharacterState): HistoryStateSnapshot {
  return {
    preset: "mannequin",
    headScale: 1.0,
    headRatio: 6.5,
    outlineWidth: 3.5,
    shadowThreshold: 0.5,
    lightDir: [0.577, 0.577, 0.577],
    lightIntensity: 1.0,
    shadowColor: [1, 1, 1],
    timestamp: Date.now(),
    character: JSON.parse(JSON.stringify(character)),
  };
}

describe("P0-07 character undo/redo", () => {
  beforeEach(() => {
    historyService.clear();
  });

  it("10 character mutations → 10 undos → deep-equal initial; 10 redos → final", () => {
    const initial = createDefaultCharacterState();
    historyService.init(baseSnapshot(initial));

    const states: CharacterState[] = [initial];
    for (let i = 1; i <= 10; i++) {
      const next = JSON.parse(JSON.stringify(states[i - 1]));
      next.morphSliders[`slider_${i}`] = i * 0.1;
      next.somatotype = { endo: 0.2 + i * 0.01, meso: 0.5 - i * 0.01, ecto: 0.3 };
      next.genderDimorphism = Math.min(1, 0.5 + i * 0.05);
      states.push(next);
      historyService.push(baseSnapshot(next), `mutation ${i}`);
    }

    for (let i = 9; i >= 0; i--) {
      const snap = historyService.undo();
      assert.ok(snap && snap.character, `undo ${10 - i} must restore a character`);
      assert.ok(characterStatesEqual(snap.character!, states[i]), `undo to state ${i} must match`);
    }
    assert.equal(historyService.canUndo(), false);

    for (let i = 1; i <= 10; i++) {
      const snap = historyService.redo();
      assert.ok(snap && snap.character, `redo ${i} must restore a character`);
      assert.ok(characterStatesEqual(snap.character!, states[i]), `redo to state ${i} must match`);
    }
  });

  it("undo/redo never wedge the service (P2-11 try/finally guard)", () => {
    historyService.init(baseSnapshot(createDefaultCharacterState()));
    assert.equal(historyService.undo(), null);
    assert.equal(historyService.redo(), null);
    assert.equal(historyService.isExecutingHistory, false);

    const next = createDefaultCharacterState();
    next.morphSliders["head_width"] = 1.2;
    historyService.push(baseSnapshot(next), "edit");
    const undone = historyService.undo();
    assert.ok(undone && undone.character);
    assert.equal(historyService.isExecutingHistory, false);
    assert.deepEqual(undone.character!.morphSliders, {});
  });

  it("continuous gestures coalesce (slider drag = one undo step baseline)", () => {
    historyService.init(baseSnapshot(createDefaultCharacterState()));
    for (let i = 0; i < 5; i++) {
      const c = createDefaultCharacterState();
      c.morphSliders["head_width"] = 1.0 + i * 0.01;
      historyService.push(baseSnapshot(c), "drag", true);
    }
    // First push is the pre-drag baseline; the rest coalesce → single undo.
    const undone = historyService.undo();
    assert.ok(undone);
    assert.equal(historyService.canUndo(), false);
  });
});
