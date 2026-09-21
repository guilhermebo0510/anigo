/**
 * ANIGO — Persistência P0: envelope versionado, validação e migrações.
 *
 * Each item of the block is exercised here:
 *  1. schema versionado        → `createEnvelope` / `validateEnvelope`
 *  2. validação de carga       → `parseEnvelope` / `readRecovery`
 *  3. migrações                → `migrateEnvelope` (v1 → v2 → v3)
 *  4. domínios no autosave     → `parseSessionState` / `parseSceneDomain`
 *  5. recuperação de sessão    → `AutoSaveService.recoverSession`
 *
 * Run: node --experimental-strip-types --test tests/persistence/
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  AUTOSAVE_ENVELOPE_VERSION,
  PersistenceError,
  commandLogOf,
  createEnvelope,
  defaultSceneDomain,
  detectEnvelopeVersion,
  isDegradedRecovery,
  parseEnvelope,
  parseSceneDomain,
  parseSessionState,
  readRecovery,
  serializeEnvelope,
  toProjectSnapshot,
  toSessionState,
  validateEnvelope,
} from "../../src/services/project_persistence.ts";
import {
  AutoSaveService,
  parseProjectDocument,
  parseProjectSnapshot,
} from "../../src/services/autosave_service.ts";
import {
  CHARACTER_SNAPSHOT_SCHEMA_VERSION,
  createDefaultCharacterState,
} from "../../src/services/character_state.ts";
import { CANONICAL_SLIDERS } from "../../src/services/morph_catalog.ts";
import { CommandHistoryService, parseCommandLog } from "../../src/services/command_history.ts";
import { COMMAND_LOG_VERSION } from "../../src/contracts/command_log.v1.ts";
import type { CommandOutcomeWire, CommandWire } from "../../src/contracts/commands.v1.ts";
import { CANONICAL_IDS, PROJECT_SCHEMA_VERSION } from "../../src/contracts/project_state.v1.ts";

/** A session with every domain touched (character, camera, light, materials, scene). */
function alteredSession() {
  const character = createDefaultCharacterState();
  for (let i = 0; i < 12; i++) {
    const def = CANONICAL_SLIDERS[i * 5];
    character.morphSliders[def.id] = def.min + (def.max - def.min) * 0.42;
  }
  character.somatotype = { endo: 0.44, meso: 0.31, ecto: 0.25 };
  character.genderDimorphism = 0.62;
  character.baseGender = "female";

  const scene = defaultSceneDomain();
  scene.background_color = [0.1, 0.2, 0.3, 1];
  scene.msaa_samples = 2;
  scene.tonemap = "reinhard";
  scene.nodes[0] = { ...scene.nodes[0], name: "Character", visible: false };
  scene.materials[0] = {
    ...scene.materials[0],
    base_color: [0.1, 0.2, 0.3, 1],
    outline_width: undefined as unknown as number,
  };
  delete (scene.materials[0] as Record<string, unknown>)["outline_width"];

  // The renderer/UI side hands the persistence layer a *flat* snapshot; the
  // service wraps it into the session block (character schema version + scene).
  const flat = {
    preset: "mannequin",
      headScale: 1.17,
      headRatio: 6.1,
      outlineWidth: 4.25,
      shadowThreshold: 0.42,
      lightDir: [0.4, 0.7, -0.2] as [number, number, number],
      lightIntensity: 1.35,
      shadowColor: [0.9, 0.92, 1] as [number, number, number],
      cameraEye: [1.5, 1.8, 3.2] as [number, number, number],
      cameraTarget: [0.1, 1.05, 0] as [number, number, number],
      cameraUp: [0, 1, 0] as [number, number, number],
      fov: 38,
      timestamp: 1_700_000_000_000,
      version: "0.2.0",
      lightAzimuth: 60,
      lightElevation: 35,
      toonSmoothness: 0.05,
      specIntensity: 0.55,
      specExponent: 24,
      rimIntensity: 1.1,
      rimSpread: 0.3,
      hueShift: -20,
      toonSteps: 2,
      outlineColor: "#331122",
      baseColorHex: "#ffeedd",
      shadowColorHex: "#bbaacc",
      sunColor: "#fff7e0",
      character,
      scene,
  };

  return { session: toSessionState(flat as never), scene };
}

/** Minimal canonical project document, as the Rust core would author it. */
function coreProject(name = "Aula 03 - Personagem") {
  return {
    schema_version: PROJECT_SCHEMA_VERSION,
    project_id: CANONICAL_IDS.projectId,
    name,
    character: { morph_values: { mrf_head_width: 1.2 } },
    scene: {},
    materials: {},
    animation: {},
    render: {},
    assets: {},
    settings: {},
  };
}

describe("P0 persistência — schema versionado", () => {
  it("writes the current envelope version and marks the author", () => {
    const { session } = alteredSession();
    const preview = createEnvelope({ session });
    assert.equal(preview.schema_version, AUTOSAVE_ENVELOPE_VERSION);
    assert.equal(preview.source, "preview");
    assert.equal(preview.core_project, null);

    const core = createEnvelope({ session, coreProject: coreProject() as never });
    assert.equal(core.source, "core");
    assert.equal(core.core_project?.name, "Aula 03 - Personagem");
  });

  it("refuses to serialize an envelope that would not be readable", () => {
    const { session } = alteredSession();
    const envelope = createEnvelope({ session });
    const broken = { ...envelope, source: "core" as const };
    assert.throws(() => serializeEnvelope(broken), (error: unknown) => {
      assert.ok(error instanceof PersistenceError);
      assert.equal(error.code, "CORE_PROJECT_MISSING");
      assert.equal(error.recoverable, true);
      return true;
    });
  });

  it("detects the version of every format the app has written", () => {
    assert.equal(detectEnvelopeVersion(createEnvelope({ session: alteredSession().session })), 3);
    assert.equal(detectEnvelopeVersion({ schemaVersion: 2, character: {} }), 2);
    assert.equal(detectEnvelopeVersion({ version: "0.2.0", character: {} }), 2);
    assert.equal(detectEnvelopeVersion({ version: "0.1.0", headScale: 1 }), 1);
    assert.equal(detectEnvelopeVersion({ headScale: 1 }), 1);
  });
});

describe("P0 persistência — validação de carga", () => {
  it("round-trips the full session through the envelope", () => {
    const { session } = alteredSession();
    const envelope = createEnvelope({ session, coreProject: coreProject() as never });
    const reloaded = parseEnvelope(serializeEnvelope(envelope)).envelope;

    assert.equal(reloaded.session.headScale, 1.17);
    assert.equal(reloaded.session.fov, 38);
    assert.deepEqual(reloaded.session.cameraEye, [1.5, 1.8, 3.2]);
    assert.deepEqual(reloaded.session.lightDir, [0.4, 0.7, -0.2]);
    assert.equal(reloaded.session.character?.genderDimorphism, 0.62);
    assert.deepEqual(reloaded.session.character?.somatotype, { endo: 0.44, meso: 0.31, ecto: 0.25 });
    assert.equal(Object.keys(reloaded.session.character?.morphSliders ?? {}).length, 12);
    assert.equal(reloaded.session.scene.msaa_samples, 2);
    assert.equal(reloaded.session.scene.tonemap, "reinhard");
    assert.equal(reloaded.session.scene.nodes[0].visible, false);
    assert.deepEqual(reloaded.session.scene.materials[0].base_color, [0.1, 0.2, 0.3, 1]);
    assert.equal(reloaded.core_project?.project_id, CANONICAL_IDS.projectId);
    assert.equal(isDegradedRecovery(reloaded), false);
  });

  it("rejects malformed and untrusted payloads with a typed error", () => {
    const cases: Array<[string, unknown, string]> = [
      ["{oops", undefined, "NOT_JSON"],
      ["[1,2]", undefined, "NOT_OBJECT"],
      ["42", undefined, "NOT_OBJECT"],
      [JSON.stringify({ schema_version: 99 }), undefined, "UNSUPPORTED_VERSION"],
    ];
    for (const [raw, , code] of cases) {
      assert.throws(() => parseEnvelope(raw), (error: unknown) => {
        assert.ok(error instanceof PersistenceError, `expected PersistenceError for ${raw}`);
        assert.equal(error.code, code);
        return true;
      });
    }
  });

  it("lists the fields a truncated envelope is missing", () => {
    const { session } = alteredSession();
    const envelope = createEnvelope({ session });
    const truncated = { ...envelope } as Record<string, unknown>;
    delete truncated["ui"];
    assert.throws(() => validateEnvelope(truncated), (error: unknown) => {
      assert.ok(error instanceof PersistenceError);
      assert.equal(error.code, "MISSING_FIELDS");
      assert.match(error.message, /ui/);
      return true;
    });
  });

  it("rejects a canonical document the core would reject", () => {
    const { session } = alteredSession();
    const invalid = { ...coreProject(), project_id: "chr_oops" };
    assert.throws(
      () => createEnvelope({ session, coreProject: invalid as never }),
      (error: unknown) => {
        assert.ok(error instanceof PersistenceError);
        assert.equal(error.code, "CORE_PROJECT_INVALID");
        assert.match(error.message, /project_id/);
        return true;
      }
    );
  });

  it("validates the scene domain structure (ids, samples, lists)", () => {
    const scene = defaultSceneDomain();
    scene.nodes[0] = { ...scene.nodes[0], node_id: "nope" };
    const { session } = alteredSession();
    assert.throws(() => validateEnvelope(createEnvelope({ session }).session && createEnvelope({
      session: { ...session, scene },
    })), (error: unknown) => {
      assert.ok(error instanceof PersistenceError);
      assert.equal(error.code, "INVALID_FIELD");
      assert.match(error.message, /node_id/);
      return true;
    });

    assert.throws(() => validateEnvelope(createEnvelope({
      session: { ...session, scene: { ...defaultSceneDomain(), msaa_samples: 0 } },
    })), /msaa_samples/);

    assert.throws(() => validateEnvelope(createEnvelope({
      session: { ...session, scene: { ...defaultSceneDomain(), assets: "nope" } },
    })), /assets/);
  });

  it("sanitizes a session block instead of trusting it", () => {
    const parsed = parseSessionState({
      preset: "death-star",
      headScale: Number.NaN,
      cameraEye: ["x", null, {}],
      scene: { nodes: [{ node_id: 7 }], materials: [{ material_id: 7 }], assets: [{ uri: 7 }] },
    });
    assert.equal(parsed.preset, "mannequin");
    assert.equal(parsed.headScale, 1.0);
    assert.deepEqual(parsed.cameraEye, [0, 1.5, 3.5]);
    // Unknown ids in the scene are dropped; the canonical defaults are kept.
    assert.equal(parsed.scene.nodes[0].node_id, CANONICAL_IDS.characterNodeId);
    assert.equal(parsed.scene.materials[0].material_id, CANONICAL_IDS.defaultMaterialId);
    assert.ok(parsed.character, "the character block is never null");
  });

  it("keeps background/materials/assets when only part of the scene is present", () => {
    const partial = parseSceneDomain({ msaa_samples: 8 });
    assert.equal(partial.msaa_samples, 8);
    assert.equal(partial.nodes.length, 1);
    assert.equal(partial.materials.length, 1);
    assert.equal(partial.assets.length, 2);
    assert.equal(parseSceneDomain(undefined).msaa_samples, 4);
  });
});

describe("P0 persistência — migrações", () => {
  it("migrates a v1 flat payload (no character) to a v3 envelope", () => {
    const v1 = {
      version: "0.1.0",
      preset: "mannequin",
      headScale: 1.05,
      headRatio: 6.4,
      outlineWidth: 3.5,
      shadowThreshold: 0.5,
      lightDir: [0.5, 0.5, 0.5],
      lightIntensity: 1.1,
      shadowColor: [1, 1, 1],
      cameraEye: [0, 1.6, 3.4],
      cameraTarget: [0, 1, 0],
      timestamp: 1_600_000_000_000,
    };
    const { envelope, applied } = parseEnvelope(JSON.stringify(v1));
    assert.equal(envelope.schema_version, AUTOSAVE_ENVELOPE_VERSION);
    assert.deepEqual(applied, ["v1_to_v2_character", "v2_to_v3_envelope"]);
    assert.equal(envelope.saved_at, 1_600_000_000_000);
    assert.equal(envelope.source, "preview");
    assert.equal(envelope.session.headScale, 1.05);
    // v1 had no character and no scene domain: the canonical defaults are used.
    assert.equal(envelope.session.characterSchemaVersion, CHARACTER_SNAPSHOT_SCHEMA_VERSION);
    assert.equal(Object.keys(envelope.session.character?.morphSliders ?? {}).length, 0);
    assert.equal(envelope.session.scene.msaa_samples, 4);
  });

  it("migrates a v2 flat payload with the character domain", () => {
    const character = createDefaultCharacterState();
    character.genderDimorphism = 0.25;
    character.morphSliders["head_width"] = 1.3;
    const v2 = {
      schemaVersion: CHARACTER_SNAPSHOT_SCHEMA_VERSION,
      version: "0.2.0",
      preset: "mannequin",
      headScale: 1.0,
      headRatio: 6.5,
      outlineWidth: 3.5,
      shadowThreshold: 0.5,
      lightDir: [0.5, 0.5, 0.5],
      lightIntensity: 1.0,
      shadowColor: [1, 1, 1],
      cameraEye: [0, 1.5, 3.5],
      cameraTarget: [0, 1, 0],
      timestamp: 1_650_000_000_000,
      character,
    };
    const { envelope, applied } = parseEnvelope(JSON.stringify(v2));
    assert.deepEqual(applied, ["v2_to_v3_envelope"]);
    assert.equal(envelope.session.character?.genderDimorphism, 0.25);
    assert.equal(envelope.session.character?.morphSliders["head_width"], 1.3);
    assert.equal(envelope.session.scene.nodes.length, 1);
  });

  it("refuses a payload from a newer envelope version", () => {
    const future = { schema_version: AUTOSAVE_ENVELOPE_VERSION + 1, session: {} };
    assert.throws(() => parseEnvelope(JSON.stringify(future)), /mais nova/);
  });

  it("keeps the flat snapshot API working over the envelope", () => {
    const { session } = alteredSession();
    const envelope = createEnvelope({ session });
    const flat = parseProjectSnapshot(serializeEnvelope(envelope));
    assert.equal(flat.headScale, 1.17);
    assert.equal(flat.schemaVersion, CHARACTER_SNAPSHOT_SCHEMA_VERSION);
    assert.deepEqual(toSessionState(flat).scene.nodes[0].node_id, CANONICAL_IDS.characterNodeId);
    assert.equal(toProjectSnapshot(envelope.session).headScale, 1.17);

    const document = parseProjectDocument(serializeEnvelope(envelope));
    assert.deepEqual(document.migrations, []);
    assert.equal(document.envelope.source, "preview");
  });
});

describe("P0 persistência — recuperação de sessão", () => {
  it("reports empty, restored and invalid cache states with the reason", () => {
    assert.equal(readRecovery(null).status, "empty");
    assert.equal(readRecovery("").status, "empty");

    const restored = readRecovery(serializeEnvelope(createEnvelope({ session: alteredSession().session })));
    assert.equal(restored.status, "restored");
    assert.equal(restored.envelope?.session.headScale, 1.17);
    assert.deepEqual(restored.migrations, []);
    assert.equal(restored.error, null);

    const invalid = readRecovery("{not json");
    assert.equal(invalid.status, "invalid");
    assert.equal(invalid.envelope, null);
    assert.equal(invalid.error?.code, "NOT_JSON");
    assert.ok(invalid.error && invalid.error.message.length > 0, "the reason is reported to the UI");

    const legacy = readRecovery(JSON.stringify({ version: "0.1.0", headScale: 1.02 }));
    assert.equal(legacy.status, "restored");
    assert.deepEqual(legacy.migrations, ["v1_to_v2_character", "v2_to_v3_envelope"]);
  });

  it("the service reads the cache through the validated recovery path", () => {
    const service = new AutoSaveService();
    // No localStorage in this runtime: the cache is reported as empty, never as
    // an exception.
    const result = service.recoverSession();
    assert.equal(result.status, "empty");
    assert.equal(service.readRecoveryCache(), null);
  });

  it("builds an envelope that records when the core could not answer", () => {
    const service = new AutoSaveService();
    const withoutCore = service.buildEnvelope({ ...alteredSession().session, schemaVersion: CHARACTER_SNAPSHOT_SCHEMA_VERSION } as never);
    assert.equal(withoutCore.source, "preview");
    assert.equal(isDegradedRecovery(withoutCore), true);

    service.configure(true, 5, () => ({ ...alteredSession().session, schemaVersion: CHARACTER_SNAPSHOT_SCHEMA_VERSION }) as never, undefined, {
      getCoreProject: () => coreProject() as never,
      getUiState: () => ({ workspace: "personagem" }),
    });
    const withCore = service.buildEnvelope({ ...alteredSession().session, schemaVersion: CHARACTER_SNAPSHOT_SCHEMA_VERSION } as never);
    assert.equal(withCore.source, "core");
    assert.equal(isDegradedRecovery(withCore), false);
    assert.deepEqual(withCore.ui, { workspace: "personagem" });
    service.destroy();

    const failing = new AutoSaveService();
    failing.configure(true, 5, () => alteredSession().session as never, undefined, {
      getCoreProject: () => {
        throw new Error("core offline");
      },
    });
    const degraded = failing.buildEnvelope({ ...alteredSession().session, schemaVersion: CHARACTER_SNAPSHOT_SCHEMA_VERSION } as never);
    assert.equal(degraded.source, "preview", "a failing core must not lose the session");
    failing.destroy();
  });
});

describe("Envelope v3 — o log de comandos viaja com a sessão (P0 undo/redo)", () => {
  const commands: CommandWire[] = [
    { kind: "set_morph_value", target: "mrf_head_width", value: 1.15 },
    { kind: "set_material_params", patch: { outline_width: 2 } },
  ];

  function filledHistory(): CommandHistoryService {
    const history = new CommandHistoryService(16);
    commands.forEach((command, index) => {
      const outcome: CommandOutcomeWire = {
        sequence: index + 1,
        revision: index + 1,
        base_geometry_revision: 0,
        description: `Comando ${index + 1}`,
        scope: index === 0 ? "deformation" : "shading",
        affected: [],
        can_undo: true,
        can_redo: false,
        undo_depth: index + 1,
        redo_depth: 0,
      };
      history.push(command, outcome);
    });
    return history;
  }

  it("só comandos aceitos chegam ao envelope, e voltam pelo replay", () => {
    const history = filledHistory();
    const service = new AutoSaveService();
    service.configure(false, 5, () => alteredSession().session as never, undefined, {
      getCommandLog: () => history.exportLog(),
    });

    const envelope = service.buildEnvelope(alteredSession().session as never);
    assert.equal(envelope.session.command_log?.version, COMMAND_LOG_VERSION);
    assert.deepEqual(
      envelope.session.command_log?.entries.map((entry) => entry.command.kind),
      ["set_morph_value", "set_material_params"]
    );

    // Round-trip: serialize → parse → log validado → replay no histórico.
    const restored = parseEnvelope(serializeEnvelope(envelope)).envelope;
    const log = commandLogOf(restored);
    assert.ok(log, "o log precisa sobreviver ao disco");
    const replay = new CommandHistoryService(16);
    replay.restoreLog(parseCommandLog(log));
    assert.deepEqual(
      replay.exportLog().entries.map((entry) => entry.command),
      commands,
      "base + log reconstroem a sessão"
    );
    assert.equal(replay.revision, 2);
    service.destroy();
  });

  it("log corrompido ou de versão futura é descartado, a sessão continua válida", () => {
    const history = filledHistory();
    const service = new AutoSaveService();
    service.configure(false, 5, () => alteredSession().session as never, undefined, {
      getCommandLog: () => history.exportLog(),
    });
    const envelope = service.buildEnvelope(alteredSession().session as never);
    service.destroy();

    assert.equal(commandLogOf(envelope)?.entries.length, 2);
    assert.equal(commandLogOf(null), null);
    assert.equal(commandLogOf({ ...envelope, session: { ...envelope.session, command_log: undefined } }), null);

    const newer = JSON.parse(JSON.stringify(envelope)) as typeof envelope;
    (newer.session.command_log as { version: number }).version = COMMAND_LOG_VERSION + 1;
    assert.equal(commandLogOf(newer), null, "um log mais novo nunca é lido pela metade");
    assert.equal(newer.session.character !== undefined, true, "a sessão em si continua legível");

    const corrupt = JSON.parse(JSON.stringify(envelope)) as typeof envelope;
    (corrupt.session.command_log as { entries: unknown[] }).entries[1] = { sequence: "dois" };
    assert.equal(commandLogOf(corrupt), null, "uma entrada inválida invalida o log inteiro");

    // Envelope antigo (sem o campo) continua válido: o campo é aditivo.
    const legacy = createEnvelope({ session: toSessionState(alteredSession().session as never) });
    assert.equal(legacy.session.command_log, undefined);
    assert.equal(commandLogOf(legacy), null);
    assert.equal(parseEnvelope(serializeEnvelope(legacy)).envelope.source, "preview");
  });

  it("uma sessão sem comandos não escreve o campo (envelope mínimo)", () => {
    const service = new AutoSaveService();
    service.configure(false, 5, () => alteredSession().session as never, undefined, {
      getCommandLog: () => new CommandHistoryService(8).exportLog(),
    });
    const envelope = service.buildEnvelope(alteredSession().session as never);
    assert.equal(envelope.session.command_log, undefined);
    service.destroy();
  });
});
