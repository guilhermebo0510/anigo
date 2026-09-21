/**
 * ANIGO contract test — ProjectState v1 + stable ids (Rust core ⇄ TypeScript).
 *
 * The TypeScript layer never authors project data, but it does carry the
 * canonical document (autosave envelope), talk about entities by stable id and
 * refuse payloads it cannot understand. All three need to agree with
 * `crates/anigo-core/src/project.rs` and `ids.rs`.
 *
 * Run: node --experimental-strip-types --test tests/contracts/
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  CANONICAL_IDS,
  CANONICAL_URIS,
  ID_MAX_LEN,
  ID_PREFIXES,
  PROJECT_SCHEMA_VERSION,
  PROJECT_STATE_EXTENSIONS_FIELD,
  PROJECT_STATE_V1_FIELDS,
  ProjectContractError,
  isStableIdKind,
  isValidStableId,
  morphIdForSlider,
  readCanonicalProject,
  readProjectEnvelope,
  sliderIdFromMorphId,
  type CanonicalProjectDocumentV1,
} from "../../src/contracts/project_state.v1.ts";
import {
  assetIdForUri,
  defaultSceneDomain,
  fnv1a64,
  normalizeUri,
} from "../../src/services/project_persistence.ts";
import {
  diff,
  fieldsOf,
  numericConst,
  readRepoFile,
  sorted,
  stringConst,
} from "./rust_contract_source.ts";

const RUST_PROJECT = readRepoFile("crates/anigo-core/src/project.rs");
const RUST_IDS = readRepoFile("crates/anigo-core/src/ids.rs");

/** `pub const PREFIX_PROJECT: &str = "prj";` → `{ project: "prj", … }`. */
function rustPrefixes(): Record<string, string> {
  const prefixes: Record<string, string> = {};
  for (const match of RUST_IDS.matchAll(/pub const PREFIX_([A-Z]+):\s*&str\s*=\s*"([^"]*)"/g)) {
    prefixes[match[1].toLowerCase()] = match[2];
  }
  return prefixes;
}

/** `impl CharacterId { pub fn canonical() -> Self { Self::from_slug("canonical") } }`. */
function rustCanonicalId(typeName: string, method: string): string {
  const implMatch = new RegExp(`impl\\s+${typeName}\\s*\\{`).exec(RUST_IDS);
  assert.ok(implMatch, `impl ${typeName} not found in ids.rs`);
  const rest = RUST_IDS.slice(implMatch.index);
  const methodMatch = new RegExp(`fn\\s+${method}\\s*\\(\\s*\\)\\s*->\\s*Self\\s*\\{[^}]*from_slug\\("([^"]+)"\\)`).exec(
    rest
  );
  assert.ok(methodMatch, `${typeName}::${method} not found`);
  return methodMatch[1];
}

function document(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    schema_version: PROJECT_SCHEMA_VERSION,
    project_id: CANONICAL_IDS.projectId,
    name: "Untitled Project",
    character: {},
    scene: {},
    materials: {},
    animation: {},
    render: {},
    assets: {},
    settings: {},
    ...overrides,
  };
}

describe("ProjectState v1 — schema parity with the Rust core", () => {
  it("schema version is the Rust constant", () => {
    assert.equal(PROJECT_SCHEMA_VERSION, numericConst(RUST_PROJECT, "PROJECT_SCHEMA_VERSION"));
    assert.equal(PROJECT_SCHEMA_VERSION, 1);
  });

  it("top-level fields match the Rust struct (extensions is flattened, not a key)", () => {
    const rustFields = fieldsOf(RUST_PROJECT, "ProjectState");
    assert.ok(rustFields.includes(PROJECT_STATE_EXTENSIONS_FIELD));
    const rustWireFields = rustFields.filter((field) => field !== PROJECT_STATE_EXTENSIONS_FIELD);
    assert.deepEqual(
      sorted([...PROJECT_STATE_V1_FIELDS]),
      sorted(rustWireFields),
      `ProjectState drifted: ${diff(rustWireFields, [...PROJECT_STATE_V1_FIELDS])}`
    );

    // `extensions` must stay `#[serde(flatten)]`, otherwise unknown keys would
    // live under a named key and the JSON shape would change.
    const attribute = /#\[serde\(([^)]*flatten[^)]*)\)\]\s*pub\s+extensions/.exec(RUST_PROJECT);
    assert.ok(attribute, "ProjectState::extensions must keep #[serde(flatten)]");
  });

  it("character/scene/render blocks exist on the Rust side", () => {
    const character = fieldsOf(RUST_PROJECT, "CharacterState");
    for (const field of ["character_id", "base_gender", "gender_dimorphism", "somatotype", "morph_values"]) {
      assert.ok(character.includes(field), `CharacterState is missing ${field}`);
    }
    const render = fieldsOf(RUST_PROJECT, "RenderState");
    // Issue #14: o render graph (ativação/ordem dos passes) faz parte do estado
    // do projeto, então entra no undo/redo e no snapshot.
    assert.deepEqual(
      sorted(render),
      sorted([
        "settings_version",
        "msaa_samples",
        "background_color",
        "color",
        "tonemap",
        "render_graph",
      ])
    );
    const scene = fieldsOf(RUST_PROJECT, "SceneState");
    for (const field of ["scene_id", "camera", "lights", "nodes"]) {
      assert.ok(scene.includes(field), `SceneState is missing ${field}`);
    }
  });
});

describe("ProjectState v1 — stable ids match ids.rs", () => {
  it("prefixes are identical for every entity kind", () => {
    const prefixes = rustPrefixes();
    assert.deepEqual(sorted(Object.keys(prefixes)), sorted(Object.keys(ID_PREFIXES)));
    for (const [kind, prefix] of Object.entries(prefixes)) {
      assert.equal(
        ID_PREFIXES[kind as keyof typeof ID_PREFIXES],
        prefix,
        `prefix drift for ${kind}`
      );
    }
    assert.equal(ID_MAX_LEN, numericConst(RUST_IDS, "ID_MAX_LEN"));
  });

  it("canonical ids are derived the same way", () => {
    assert.equal(CANONICAL_IDS.projectId, `${rustPrefixes().project}_${"untitled"}`);
    assert.equal(CANONICAL_IDS.characterId, `${rustPrefixes().character}_${rustCanonicalId("CharacterId", "canonical")}`);
    assert.equal(CANONICAL_IDS.sceneId, `${rustPrefixes().scene}_${rustCanonicalId("SceneId", "canonical")}`);
    assert.equal(CANONICAL_IDS.cameraId, `${rustPrefixes().camera}_${rustCanonicalId("CameraId", "canonical")}`);
    assert.equal(CANONICAL_IDS.keyLightId, `${rustPrefixes().light}_${rustCanonicalId("LightId", "canonical_key")}`);
    assert.equal(
      CANONICAL_IDS.defaultMaterialId,
      `${rustPrefixes().material}_${rustCanonicalId("MaterialId", "canonical_default")}`
    );
    assert.equal(
      CANONICAL_IDS.characterNodeId,
      `${rustPrefixes().node}_${rustCanonicalId("NodeId", "canonical_character")}`
    );
    // The fixture and the Rust snapshot tests use these exact values.
    assert.equal(CANONICAL_IDS.characterId, "chr_canonical");
    assert.equal(CANONICAL_IDS.keyLightId, "lgt_key");
    assert.equal(CANONICAL_IDS.defaultMaterialId, "mat_default_anime");
    assert.equal(CANONICAL_IDS.characterNodeId, "nod_character_base");
  });

  it("canonical asset URIs are the Rust constants", () => {
    assert.equal(CANONICAL_URIS.baseMale, stringConst(RUST_PROJECT, "URI_BASE_MALE"));
    assert.equal(CANONICAL_URIS.baseFemale, stringConst(RUST_PROJECT, "URI_BASE_FEMALE"));
    assert.equal(CANONICAL_URIS.presetCube, stringConst(RUST_PROJECT, "URI_PRESET_CUBE"));
    assert.equal(CANONICAL_URIS.presetSphere, stringConst(RUST_PROJECT, "URI_PRESET_SPHERE"));
    // The viewport fetches the same files the core registers as assets.
    assert.equal(CANONICAL_URIS.baseMale, "anigo://base/anigo_base_male.glb");
    assert.equal(CANONICAL_URIS.baseFemale, "anigo://base/anigo_base_female.glb");
  });

  it("stable-id validation follows the Rust rules", () => {
    for (const [kind, prefix] of Object.entries(ID_PREFIXES)) {
      const typed = kind as keyof typeof ID_PREFIXES;
      assert.equal(isValidStableId(`${prefix}_valid_slug_9`, typed), true);
      assert.equal(isValidStableId("", typed), false);
      assert.equal(isValidStableId("   ", typed), false);
      assert.equal(isValidStableId(`${prefix}`, typed), false, "missing separator");
      assert.equal(isValidStableId(`xxx_slug`, typed), false, "wrong prefix");
      assert.equal(isValidStableId(`${prefix}_BadSlug`, typed), false, "uppercase is not canonical");
      assert.equal(isValidStableId(`${prefix}_has-dash`, typed), false, "dashes are not canonical");
      assert.equal(isValidStableId(`${prefix}_${"a".repeat(ID_MAX_LEN)}`, typed), false, "too long");
      assert.equal(isValidStableId(42 as unknown, typed), false, "not a string");
    }
    assert.equal(isStableIdKind("character"), true);
    assert.equal(isStableIdKind("nope"), false);
  });

  it("asset ids reproduce the frozen fixture (Rust and TS derive the same)", () => {
    const fixture = JSON.parse(readRepoFile("contracts/fixtures/asset_ids_v1.json")) as {
      algorithm: string;
      normalization: string;
      ids: Array<{ uri: string; asset_id: string }>;
    };
    assert.ok(fixture.ids.length > 0);
    for (const entry of fixture.ids) {
      assert.equal(assetIdForUri(entry.uri), entry.asset_id, `asset id drift for ${JSON.stringify(entry.uri)}`);
      assert.equal(isValidStableId(entry.asset_id, "asset"), true);
      assert.equal(entry.asset_id.length, 20, "asset id must be 'ast_' + 16 hex digits");
    }
    // Normalization collapses equivalent URIs (query, fragment, case, spacing).
    const ids = new Set(fixture.ids.map((entry) => entry.asset_id));
    assert.ok(ids.size < fixture.ids.length, "normalization must collapse equivalent URIs");
    // The scene domain registers exactly those ids for the canonical assets.
    const scene = defaultSceneDomain();
    assert.deepEqual(
      scene.assets.map((asset) => asset.asset_id),
      [assetIdForUri(CANONICAL_URIS.baseMale), assetIdForUri(CANONICAL_URIS.baseFemale)]
    );
    assert.equal(fnv1a64("").toString(16), "cbf29ce484222325");
    assert.equal(normalizeUri("anigo://preset/cube#lod1"), "anigo://preset/cube");
  });

  it("morph ids are the slider id with the morph prefix", () => {
    assert.equal(morphIdForSlider("head_width"), "mrf_head_width");
    assert.equal(isValidStableId(morphIdForSlider("jaw_v_line_taper"), "morph"), true);
    assert.equal(sliderIdFromMorphId("mrf_head_width"), "head_width");
    assert.equal(sliderIdFromMorphId("head_width"), "head_width");
    for (const slider of ["head_width", "somatotype_endomorph", "eye_size"]) {
      assert.equal(sliderIdFromMorphId(morphIdForSlider(slider)), slider, "round trip");
    }
  });
});

describe("ProjectState v1 — loading is validated and errors are recoverable", () => {
  function expectCode(code: string, run: () => unknown) {
    assert.throws(run, (error: unknown) => {
      assert.ok(error instanceof ProjectContractError, `expected ProjectContractError, got ${error}`);
      assert.equal(error.code, code);
      return true;
    });
  }

  it("accepts a canonical document and returns it untouched", () => {
    const candidate = document({ extensions: { note: "kept" } });
    const loaded = readCanonicalProject(candidate);
    assert.equal(loaded.schema_version, 1);
    assert.equal(loaded.project_id, CANONICAL_IDS.projectId);
    assert.deepEqual((loaded as CanonicalProjectDocumentV1).extensions, { note: "kept" });
  });

  it("rejects non-objects", () => {
    for (const value of [null, 42, "x", []]) {
      expectCode("NOT_AN_OBJECT", () => readCanonicalProject(value));
    }
  });

  it("rejects a document without a schema version", () => {
    const candidate = document();
    delete candidate.schema_version;
    expectCode("MISSING_SCHEMA_VERSION", () => readCanonicalProject(candidate));
    expectCode("MISSING_SCHEMA_VERSION", () =>
      readCanonicalProject(document({ schema_version: 1.5 }))
    );
  });

  it("refuses a project saved by a newer build and asks for migration on older ones", () => {
    expectCode("NEWER_SCHEMA", () => readCanonicalProject(document({ schema_version: 2 })));
    expectCode("OLDER_SCHEMA", () => readCanonicalProject(document({ schema_version: 0 })));
  });

  it("lists the missing canonical fields", () => {
    const candidate = document();
    delete candidate.scene;
    delete candidate.render;
    assert.throws(
      () => readCanonicalProject(candidate),
      (error: unknown) => {
        assert.ok(error instanceof ProjectContractError);
        assert.equal(error.code, "MISSING_FIELDS");
        assert.match(error.message, /scene/);
        assert.match(error.message, /render/);
        return true;
      }
    );
  });

  it("rejects an invalid project id", () => {
    expectCode("INVALID_PROJECT_ID", () =>
      readCanonicalProject(document({ project_id: "character_1" }))
    );
  });

  it("reads the canonical document out of an autosave envelope only when present", () => {
    assert.equal(readProjectEnvelope({ schema_version: 2, morphs: {} }), null);
    assert.equal(readProjectEnvelope(null), null);
    assert.equal(readProjectEnvelope({ coreProject: null }), null);

    const loaded = readProjectEnvelope({ schema_version: 2, morphs: {}, coreProject: document() });
    assert.ok(loaded, "envelope with a canonical document must be readable");
    assert.equal(loaded?.schema_version, 1);

    expectCode("NEWER_SCHEMA", () =>
      readProjectEnvelope({ coreProject: document({ schema_version: 9 }) })
    );
  });
});
