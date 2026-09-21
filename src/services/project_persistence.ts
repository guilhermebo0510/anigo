/**
 * ANIGO — project persistence (P0 "Corrigir persistência").
 *
 * Everything that is written to disk or to the recovery cache goes through one
 * versioned envelope:
 *
 * ```text
 * {
 *   "schema_version": 3,          // envelope version (this file)
 *   "saved_at": 1712345678901,
 *   "app_version": "0.2.0",
 *   "source": "core" | "preview", // who authored the session block
 *   "core_project": { … } | null, // canonical ProjectState document (Rust)
 *   "session": { … },             // presentation state the renderer restores
 *   "ui": { … }                   // opaque UI preferences
 * }
 * ```
 *
 * Rules (ARQUITETURA_CANONICA_ANIGO §3.1 + P0 persistence list):
 *  1. versioned from the first release, with an explicit migration per version;
 *  2. validated *before* use — invalid payloads produce a typed, recoverable
 *     error instead of a half-applied scene;
 *  3. migrations are explicit and named, never implicit field sniffing;
 *  4. the session block carries character, scene, materials, lighting, camera
 *     and render — the domains the project needs to come back identical;
 *  5. `source` records whether the authoritative document came from the core:
 *     a session restored from a `preview` envelope is degraded by definition
 *     and the UI is told so.
 *
 * The canonical project document is *authored by the Rust core*. This module
 * only validates it (`readCanonicalProject`) and carries it verbatim; it never
 * builds or repairs one.
 */

import { NODE_KINDS, type NodeKindWire } from "../contracts/commands.v1";
import {
  CANONICAL_IDS,
  CANONICAL_URIS,
  ProjectContractError,
  isValidStableId,
  readCanonicalProject,
  type CanonicalProjectDocumentV1,
} from "../contracts/project_state.v1";
import {
  CHARACTER_SNAPSHOT_SCHEMA_VERSION,
  sanitizeCharacterState,
  type CharacterState,
} from "./character_state";
import { COMMAND_LOG_VERSION, type CommandLogWire } from "../contracts/command_log.v1";
import { isCommandLogEntry } from "./command_history";
import type { ProjectStateSnapshot } from "./autosave_service";

/** Current version of the persisted envelope. */
export const AUTOSAVE_ENVELOPE_VERSION = 3;
/** Version written by the first release (no `schema_version` field at all). */
export const LEGACY_ENVELOPE_VERSION = 1;
/** Version written before the envelope existed (flat payload, `schemaVersion`). */
export const FLAT_ENVELOPE_VERSION = 2;

/** Who authored the session block of an envelope. */
export type EnvelopeSource = "core" | "preview";

export type ColorSpaceName = "srgb" | "linear_srgb" | "display_p3";
export type TonemapName = "none" | "reinhard" | "neutral";

/**
 * Scene domain of the session: nodes, materials, assets, background and render
 * settings. This is what makes "personagem, cena, materiais, iluminação, câmera
 * e render" survive a restart — camera and lighting live in the flat part of the
 * session block (they are scalar/vector state), the rest lives here.
 */
export interface SceneDomainSnapshot {
  background_color: [number, number, number, number];
  msaa_samples: number;
  tonemap: TonemapName;
  color_management: {
    input_texture_space: ColorSpaceName;
    working_space: ColorSpaceName;
    display_space: ColorSpaceName;
  };
  nodes: SceneNodeSnapshot[];
  materials: SceneMaterialSnapshot[];
  assets: SceneAssetSnapshot[];
}

export interface SceneNodeSnapshot {
  node_id: string;
  name: string;
  visible: boolean;
  material_id: string | null;
  /** Canonical asset id of the mesh bound to the node (`ast_*`). */
  mesh_asset_id: string | null;
  /**
   * Issue #12: pai na árvore de transformações (`null` = raiz da cena) e tipo
   * especializado do nó. A lista de filhos é **derivada** de `parent_id` e
   * existe materializada só para a UI não refazer a travessia.
   */
  parent_id: string | null;
  children: string[];
  kind: NodeKindWire;
}

export interface SceneMaterialSnapshot {
  material_id: string;
  name: string;
  base_color: [number, number, number, number];
  shade_color: [number, number, number, number];
  outline_color: [number, number, number, number];
}

export interface SceneAssetSnapshot {
  asset_id: string;
  /** `mesh` for now; the core owns the vocabulary (`AssetKind`). */
  kind: string;
  uri: string;
}

/** Session block: the presentation state restored into the renderer. */
export type SessionState = Omit<ProjectStateSnapshot, "schemaVersion"> & {
  /** Character schema version of this block (the envelope has its own). */
  characterSchemaVersion: number;
  scene: SceneDomainSnapshot;
  /**
   * P0 undo/redo: the accepted commands that rebuild this session by replay.
   * Additive and optional — an envelope without it is still a valid session
   * (older writers), and a session with it can be replayed onto the base state.
   */
  command_log?: CommandLogWire;
};

/** Persisted envelope (current version). */
export interface ProjectEnvelopeV3 {
  schema_version: typeof AUTOSAVE_ENVELOPE_VERSION;
  saved_at: number;
  app_version: string;
  source: EnvelopeSource;
  /** Canonical document authored by the Rust core, or `null` in preview mode. */
  core_project: CanonicalProjectDocumentV1 | null;
  session: SessionState;
  ui: Record<string, unknown>;
}

/** Everything `createEnvelope` needs; `source` is derived, never passed in. */
export interface EnvelopeInput {
  session: SessionState;
  coreProject?: CanonicalProjectDocumentV1 | null;
  ui?: Record<string, unknown>;
  savedAt?: number;
  appVersion?: string;
}

export type PersistenceErrorCode =
  | "NOT_JSON"
  | "NOT_OBJECT"
  | "UNSUPPORTED_VERSION"
  | "MISSING_FIELDS"
  | "INVALID_FIELD"
  | "CORE_PROJECT_MISSING"
  | "CORE_PROJECT_INVALID";

/** Typed, recoverable persistence failure. */
export class PersistenceError extends Error {
  readonly code: PersistenceErrorCode;
  readonly detail: string;
  /** Always true: the caller can fall back (default project) without losing data. */
  readonly recoverable = true;

  constructor(code: PersistenceErrorCode, detail: string) {
    super(`[ANIGO persistence ${code}] ${detail}`);
    this.name = "PersistenceError";
    this.code = code;
    this.detail = detail;
  }
}

function fail(code: PersistenceErrorCode, detail: string): never {
  throw new PersistenceError(code, detail);
}

export const DEFAULT_APP_VERSION = "0.2.0";

/** Canonical defaults used when a v1/v2 payload predates a domain. */
export function defaultSceneDomain(): SceneDomainSnapshot {
  return {
    background_color: [0.08, 0.09, 0.13, 1.0],
    msaa_samples: 4,
    tonemap: "none",
    color_management: {
      input_texture_space: "srgb",
      working_space: "linear_srgb",
      display_space: "srgb",
    },
    nodes: [
      {
        node_id: CANONICAL_IDS.characterNodeId,
        name: "Anime Mannequin",
        visible: true,
        material_id: CANONICAL_IDS.defaultMaterialId,
        mesh_asset_id: null,
        parent_id: null,
        children: [],
        kind: "character_root",
      },
    ],
    materials: [
      {
        material_id: CANONICAL_IDS.defaultMaterialId,
        name: "DefaultAnimeMaterial",
        base_color: [0.98, 0.92, 0.85, 1.0],
        shade_color: [0.82, 0.73, 0.78, 1.0],
        outline_color: [0.25, 0.15, 0.2, 1.0],
      },
    ],
    assets: [
      { asset_id: assetIdForUri(CANONICAL_URIS.baseMale), kind: "mesh", uri: CANONICAL_URIS.baseMale },
      { asset_id: assetIdForUri(CANONICAL_URIS.baseFemale), kind: "mesh", uri: CANONICAL_URIS.baseFemale },
    ],
  };
}

// ---------------------------------------------------------------------------
// Asset ids
// ---------------------------------------------------------------------------

/**
 * Normalization applied before hashing a URI — mirrors Rust `normalize_uri`
 * step by step: trim, backslashes to slashes, drop `?query`, drop `#fragment`,
 * strip leading `./`, lowercase. The order is part of the contract because the
 * hash is taken over the result.
 */
export function normalizeUri(uri: string): string {
  let normalized = uri.trim().replace(/\\/g, "/");
  const query = normalized.indexOf("?");
  if (query !== -1) normalized = normalized.slice(0, query);
  const fragment = normalized.indexOf("#");
  if (fragment !== -1) normalized = normalized.slice(0, fragment);
  while (normalized.startsWith("./")) normalized = normalized.slice(2);
  return normalized.toLowerCase();
}

/** FNV-1a 64-bit over the UTF-8 bytes (mirrors Rust `fnv1a64`). */
export function fnv1a64(input: string): bigint {
  const bytes = new TextEncoder().encode(input);
  let hash = 0xcbf29ce484222325n;
  for (const byte of bytes) {
    hash ^= BigInt(byte);
    hash = (hash * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return hash;
}

/**
 * Content-addressed asset id: `AssetId::for_uri` in the core.
 * Kept in lockstep by `tests/contracts/project_state_contract.test.ts` against
 * `contracts/fixtures/asset_ids_v1.json`.
 */
export function assetIdForUri(uri: string): string {
  return `ast_${fnv1a64(normalizeUri(uri)).toString(16).padStart(16, "0")}`;
}

// ---------------------------------------------------------------------------
// Session block — tolerant reader (legacy payloads + preview sessions)
// ---------------------------------------------------------------------------

function num(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function vec3(value: unknown, fallback: [number, number, number]): [number, number, number] {
  if (!Array.isArray(value) || value.length < 3) return fallback;
  return [num(value[0], fallback[0]), num(value[1], fallback[1]), num(value[2], fallback[2])];
}

function vec4(value: unknown, fallback: [number, number, number, number]): [number, number, number, number] {
  if (!Array.isArray(value) || value.length < 4) return fallback;
  return [
    num(value[0], fallback[0]),
    num(value[1], fallback[1]),
    num(value[2], fallback[2]),
    num(value[3], fallback[3]),
  ];
}

function str(value: unknown, fallback: string): string {
  return typeof value === "string" && value.length > 0 ? value : fallback;
}

const COLOR_SPACES: readonly ColorSpaceName[] = ["srgb", "linear_srgb", "display_p3"];
const TONEMAPS: readonly TonemapName[] = ["none", "reinhard", "neutral"];

function colorSpace(value: unknown, fallback: ColorSpaceName): ColorSpaceName {
  return typeof value === "string" && (COLOR_SPACES as readonly string[]).includes(value)
    ? (value as ColorSpaceName)
    : fallback;
}

function tonemap(value: unknown, fallback: TonemapName): TonemapName {
  return typeof value === "string" && (TONEMAPS as readonly string[]).includes(value)
    ? (value as TonemapName)
    : fallback;
}

/**
 * Issue #12: tipo do nó — `mesh` é o valor neutro (`NodeKind::Mesh` é o
 * `Default` do Rust), então um payload antigo continua válido.
 */
function nodeKindOrDefault(value: unknown): NodeKindWire {
  return typeof value === "string" && (NODE_KINDS as readonly string[]).includes(value)
    ? (value as NodeKindWire)
    : "mesh";
}

/** Sanitizes the scene domain of a session block (never throws). */
export function parseSceneDomain(value: unknown): SceneDomainSnapshot {
  const defaults = defaultSceneDomain();
  if (typeof value !== "object" || value === null || Array.isArray(value)) return defaults;
  const source = value as Record<string, unknown>;

  const nodes = Array.isArray(source["nodes"])
    ? (source["nodes"] as unknown[]).flatMap((entry): SceneNodeSnapshot[] => {
        if (typeof entry !== "object" || entry === null) return [];
        const node = entry as Record<string, unknown>;
        const nodeId = str(node["node_id"], CANONICAL_IDS.characterNodeId);
        const parentId =
          typeof node["parent_id"] === "string" && node["parent_id"] !== nodeId
            ? node["parent_id"]
            : null;
        const kind = nodeKindOrDefault(node["kind"]);
        const children = Array.isArray(node["children"])
          ? (node["children"] as unknown[]).filter(
              (child): child is string => typeof child === "string" && child !== nodeId
            )
          : [];
        return [
          {
            node_id: nodeId,
            name: str(node["name"], nodeId),
            visible: node["visible"] !== false,
            material_id: typeof node["material_id"] === "string" ? node["material_id"] : null,
            mesh_asset_id: typeof node["mesh_asset_id"] === "string" ? node["mesh_asset_id"] : null,
            parent_id: parentId,
            children,
            kind,
          },
        ];
      })
    : defaults.nodes;

  const materials = Array.isArray(source["materials"])
    ? (source["materials"] as unknown[]).flatMap((entry): SceneMaterialSnapshot[] => {
        if (typeof entry !== "object" || entry === null) return [];
        const material = entry as Record<string, unknown>;
        const template = defaults.materials[0];
        return [
          {
            material_id: str(material["material_id"], template.material_id),
            name: str(material["name"], template.name),
            base_color: vec4(material["base_color"], template.base_color),
            shade_color: vec4(material["shade_color"], template.shade_color),
            outline_color: vec4(material["outline_color"], template.outline_color),
          },
        ];
      })
    : defaults.materials;

  const assets = Array.isArray(source["assets"])
    ? (source["assets"] as unknown[]).flatMap((entry): SceneAssetSnapshot[] => {
        if (typeof entry !== "object" || entry === null) return [];
        const asset = entry as Record<string, unknown>;
        const uri = str(asset["uri"], "");
        if (!uri) return [];
        return [
          {
            asset_id: str(asset["asset_id"], assetIdForUri(uri)),
            kind: str(asset["kind"], "mesh"),
            uri,
          },
        ];
      })
    : defaults.assets;

  const colorManagement = source["color_management"];
  const colorSource =
    typeof colorManagement === "object" && colorManagement !== null
      ? (colorManagement as Record<string, unknown>)
      : {};

  return {
    background_color: vec4(source["background_color"], defaults.background_color),
    msaa_samples: Math.max(1, Math.round(num(source["msaa_samples"], defaults.msaa_samples))),
    tonemap: tonemap(source["tonemap"], defaults.tonemap),
    color_management: {
      input_texture_space: colorSpace(
        colorSource["input_texture_space"],
        defaults.color_management.input_texture_space
      ),
      working_space: colorSpace(colorSource["working_space"], defaults.color_management.working_space),
      display_space: colorSpace(colorSource["display_space"], defaults.color_management.display_space),
    },
    nodes: nodes.length > 0 ? nodes : defaults.nodes,
    materials: materials.length > 0 ? materials : defaults.materials,
    assets,
  };
}

/**
 * Reads the session block of a project payload (v1/v2 flat payloads and v3
 * envelopes alike). Tolerant by design: this is the *runtime* state, and the
 * character schema already owns its own sanitization.
 */
export function parseSessionState(value: unknown): SessionState {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    fail("NOT_OBJECT", "a raiz do projeto precisa ser um objeto");
  }
  const source = value as Record<string, unknown>;
  const declaredSchema = source["characterSchemaVersion"] ?? source["schemaVersion"];
  const characterSchemaVersion = Math.max(
    1,
    Math.round(num(declaredSchema, CHARACTER_SNAPSHOT_SCHEMA_VERSION))
  );
  if (typeof declaredSchema === "number" && declaredSchema > CHARACTER_SNAPSHOT_SCHEMA_VERSION) {
    fail(
      "UNSUPPORTED_VERSION",
      `projeto criado por uma versão mais nova (schema ${declaredSchema}, suportado ${CHARACTER_SNAPSHOT_SCHEMA_VERSION})`
    );
  }

  const preset = source["preset"];

  return {
    preset: preset === "sphere" || preset === "cube" ? preset : "mannequin",
    headScale: num(source["headScale"], 1.0),
    headRatio: num(source["headRatio"], 6.5),
    outlineWidth: num(source["outlineWidth"], 3.5),
    shadowThreshold: num(source["shadowThreshold"], 0.5),
    lightDir: vec3(source["lightDir"], [0.577, 0.577, 0.577]),
    lightIntensity: num(source["lightIntensity"], 1.0),
    shadowColor: vec3(source["shadowColor"], [1, 1, 1]),
    cameraEye: vec3(source["cameraEye"], [0, 1.5, 3.5]),
    cameraTarget: vec3(source["cameraTarget"], [0, 1, 0]),
    cameraUp: vec3(source["cameraUp"], [0, 1, 0]),
    fov: num(source["fov"], 45),
    timestamp: num(source["timestamp"], Date.now()),
    version: str(source["version"], DEFAULT_APP_VERSION),
    characterSchemaVersion,
    lightAzimuth: num(source["lightAzimuth"], 45),
    lightElevation: num(source["lightElevation"], 45),
    toonSmoothness: num(source["toonSmoothness"], 0.02),
    specIntensity: num(source["specIntensity"], 0.4),
    specExponent: num(source["specExponent"], 32),
    rimIntensity: num(source["rimIntensity"], 0.8),
    rimSpread: num(source["rimSpread"], 0.4),
    hueShift: num(source["hueShift"], -15),
    toonSteps: num(source["toonSteps"], 1),
    outlineColor: str(source["outlineColor"], "#402633"),
    baseColorHex: str(source["baseColorHex"], "#faebd7"),
    shadowColorHex: str(source["shadowColorHex"], "#d1b8c7"),
    sunColor: str(source["sunColor"], "#fff2df"),
    shadowSaturation: num(source["shadowSaturation"], 1.15),
    ambientIntensity: num(source["ambientIntensity"], 0.35),
    outlineOpacity: num(source["outlineOpacity"], 1),
    outlineSmoothness: num(source["outlineSmoothness"], 0),
    outlineDepthBias: num(source["outlineDepthBias"], 0),
    specSoftness: num(source["specSoftness"], 0.05),
    specOffset: num(source["specOffset"], 0),
    specularSize: num(source["specularSize"], 0.45),
    aoIntensity: num(source["aoIntensity"], 0.85),
    ambientSky: vec3(source["ambientSky"], [0.52, 0.6, 0.78]),
    ambientGround: vec3(source["ambientGround"], [0.25, 0.2, 0.18]),
    specColorHex: str(source["specColorHex"], "#ffffff"),
    rimColor: str(source["rimColor"], "#93c5fd"),
    lightColor: vec3(source["lightColor"], [1, 0.98, 0.95]),
    character: sanitizeCharacterState(source["character"]),
    scene: parseSceneDomain(source["scene"]),
  };
}

// ---------------------------------------------------------------------------
// Envelope — creation, validation, migration
// ---------------------------------------------------------------------------

/** Builds the envelope for a session (derives `source` from the document). */
export function createEnvelope(input: EnvelopeInput): ProjectEnvelopeV3 {
  const coreProject = input.coreProject ?? null;
  const envelope: ProjectEnvelopeV3 = {
    schema_version: AUTOSAVE_ENVELOPE_VERSION,
    saved_at: input.savedAt ?? Date.now(),
    app_version: input.appVersion ?? DEFAULT_APP_VERSION,
    source: coreProject ? "core" : "preview",
    core_project: coreProject,
    session: input.session,
    ui: input.ui ?? {},
  };
  // Cheap insurance: never hand an envelope to the writer that a reader would
  // reject (the invariants are exactly the ones `validateEnvelope` checks).
  return validateEnvelope(envelope);
}

/** Serializes an envelope for disk / recovery cache. */
export function serializeEnvelope(envelope: ProjectEnvelopeV3): string {
  validateEnvelope(envelope);
  return JSON.stringify(envelope, null, 2);
}

function requireFinite(source: Record<string, unknown>, field: string): number {
  const value = source[field];
  if (typeof value !== "number" || !Number.isFinite(value)) {
    fail("INVALID_FIELD", `'${field}' precisa ser um número finito`);
  }
  return value;
}

/**
 * Validates an envelope before use. Throws `PersistenceError` with a code the
 * caller can branch on; never mutates its input.
 */
export function validateEnvelope(value: unknown): ProjectEnvelopeV3 {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    fail("NOT_OBJECT", "o envelope de projeto precisa ser um objeto");
  }
  const envelope = value as Record<string, unknown>;

  const version = envelope["schema_version"];
  if (typeof version !== "number" || !Number.isInteger(version)) {
    fail("MISSING_FIELDS", "o envelope precisa de 'schema_version' inteiro");
  }
  if (version > AUTOSAVE_ENVELOPE_VERSION) {
    fail(
      "UNSUPPORTED_VERSION",
      `projeto salvo por uma versão mais nova (envelope ${version}, suportado ${AUTOSAVE_ENVELOPE_VERSION})`
    );
  }
  if (version < AUTOSAVE_ENVELOPE_VERSION) {
    fail(
      "UNSUPPORTED_VERSION",
      `envelope ${version} precisa de migração antes do uso (atual ${AUTOSAVE_ENVELOPE_VERSION})`
    );
  }

  for (const field of ["saved_at", "app_version", "source", "session", "ui", "core_project"]) {
    if (!(field in envelope)) fail("MISSING_FIELDS", `envelope v${version} sem o campo '${field}'`);
  }

  const savedAt = requireFinite(envelope, "saved_at");
  if (savedAt <= 0) fail("INVALID_FIELD", "'saved_at' precisa ser um timestamp positivo");

  const appVersion = envelope["app_version"];
  if (typeof appVersion !== "string" || appVersion.length === 0) {
    fail("INVALID_FIELD", "'app_version' precisa ser uma string não vazia");
  }

  const source = envelope["source"];
  if (source !== "core" && source !== "preview") {
    fail("INVALID_FIELD", "'source' precisa ser 'core' ou 'preview'");
  }

  const ui = envelope["ui"];
  if (typeof ui !== "object" || ui === null || Array.isArray(ui)) {
    fail("INVALID_FIELD", "'ui' precisa ser um objeto");
  }

  const coreProject = envelope["core_project"];
  if (source === "core" && coreProject === null) {
    fail("CORE_PROJECT_MISSING", "envelope marcado como 'core' precisa do documento canônico");
  }
  if (coreProject !== null && coreProject !== undefined) {
    try {
      readCanonicalProject(coreProject);
    } catch (error) {
      if (error instanceof ProjectContractError) {
        fail("CORE_PROJECT_INVALID", `documento canônico inválido: ${error.message}`);
      }
      throw error;
    }
  }

  const session = validateSession(envelope["session"]);

  return {
    schema_version: AUTOSAVE_ENVELOPE_VERSION,
    saved_at: savedAt,
    app_version: appVersion,
    source,
    core_project: (coreProject ?? null) as CanonicalProjectDocumentV1 | null,
    session,
    ui: ui as Record<string, unknown>,
  };
}

/** Strict validation of the session block (structure + ids + finiteness). */
export function validateSession(value: unknown): SessionState {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    fail("INVALID_FIELD", "'session' precisa ser um objeto");
  }
  const session = value as Record<string, unknown>;

  for (const field of [
    "preset",
    "headScale",
    "headRatio",
    "outlineWidth",
    "shadowThreshold",
    "lightDir",
    "lightIntensity",
    "shadowColor",
    "cameraEye",
    "cameraTarget",
    "fov",
    "timestamp",
    "character",
    "scene",
  ]) {
    if (!(field in session)) fail("MISSING_FIELDS", `session sem o campo '${field}'`);
  }

  for (const field of [
    "headScale",
    "headRatio",
    "outlineWidth",
    "shadowThreshold",
    "lightIntensity",
    "fov",
    "timestamp",
  ]) {
    requireFinite(session, field);
  }

  const scene = session["scene"];
  if (typeof scene !== "object" || scene === null || Array.isArray(scene)) {
    fail("INVALID_FIELD", "session.scene precisa ser um objeto");
  }
  const sceneRecord = scene as Record<string, unknown>;
  for (const field of ["background_color", "msaa_samples", "tonemap", "color_management", "nodes", "materials", "assets"]) {
    if (!(field in sceneRecord)) fail("MISSING_FIELDS", `session.scene sem o campo '${field}'`);
  }
  for (const field of ["nodes", "materials", "assets"]) {
    if (!Array.isArray(sceneRecord[field])) {
      fail("INVALID_FIELD", `session.scene.${field} precisa ser uma lista`);
    }
  }
  const msaa = sceneRecord["msaa_samples"];
  if (typeof msaa !== "number" || !Number.isInteger(msaa) || msaa < 1) {
    fail("INVALID_FIELD", "session.scene.msaa_samples precisa ser um inteiro >= 1");
  }

  const nodes = sceneRecord["nodes"] as unknown[];
  for (const entry of nodes) {
    if (typeof entry !== "object" || entry === null) {
      fail("INVALID_FIELD", "session.scene.nodes precisa conter objetos");
    }
    const node = entry as Record<string, unknown>;
    if (!isValidStableId(node["node_id"], "node")) {
      fail("INVALID_FIELD", `node_id inválido em session.scene.nodes: ${String(node["node_id"])}`);
    }
    const materialId = node["material_id"];
    if (materialId !== null && materialId !== undefined && !isValidStableId(materialId, "material")) {
      fail("INVALID_FIELD", `material_id inválido no nó ${String(node["node_id"])}`);
    }
  }

  for (const entry of sceneRecord["materials"] as unknown[]) {
    if (typeof entry !== "object" || entry === null) {
      fail("INVALID_FIELD", "session.scene.materials precisa conter objetos");
    }
    const material = entry as Record<string, unknown>;
    if (!isValidStableId(material["material_id"], "material")) {
      fail("INVALID_FIELD", `material_id inválido: ${String(material["material_id"])}`);
    }
  }

  for (const entry of sceneRecord["assets"] as unknown[]) {
    if (typeof entry !== "object" || entry === null) {
      fail("INVALID_FIELD", "session.scene.assets precisa conter objetos");
    }
    const asset = entry as Record<string, unknown>;
    if (!isValidStableId(asset["asset_id"], "asset")) {
      fail("INVALID_FIELD", `asset_id inválido: ${String(asset["asset_id"])}`);
    }
    if (typeof asset["uri"] !== "string" || (asset["uri"] as string).length === 0) {
      fail("INVALID_FIELD", "asset sem uri");
    }
  }

  return value as unknown as SessionState;
}

/** Result of a migration attempt. */
export interface MigrationResult {
  envelope: ProjectEnvelopeV3;
  /** Names of the migrations applied, in order (empty for v3 payloads). */
  applied: string[];
}

/** Detects the version of a persisted payload without parsing it. */
export function detectEnvelopeVersion(value: unknown): number {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return AUTOSAVE_ENVELOPE_VERSION;
  }
  const source = value as Record<string, unknown>;
  if (typeof source["schema_version"] === "number") return source["schema_version"] as number;
  // Before the envelope existed the flat payload carried the *character* schema
  // version; `version: "0.1.0"` (or nothing) means the very first format.
  if (typeof source["schemaVersion"] === "number" && source["schemaVersion"] >= 2) {
    return FLAT_ENVELOPE_VERSION;
  }
  if (typeof source["version"] === "string" && source["version"] !== "0.1.0") {
    return FLAT_ENVELOPE_VERSION;
  }
  return LEGACY_ENVELOPE_VERSION;
}

/**
 * Migrates any persisted payload to the current envelope.
 *
 * Explicit by design: each step has a name, and an unknown *newer* version is
 * refused instead of being silently reinterpreted.
 */
export function migrateEnvelope(value: unknown, options: { savedAt?: number; appVersion?: string } = {}): MigrationResult {
  const version = detectEnvelopeVersion(value);
  if (version > AUTOSAVE_ENVELOPE_VERSION) {
    fail(
      "UNSUPPORTED_VERSION",
      `projeto salvo por uma versão mais nova (${version}, suportado ${AUTOSAVE_ENVELOPE_VERSION})`
    );
  }
  if (version === AUTOSAVE_ENVELOPE_VERSION) {
    return { envelope: validateEnvelope(value), applied: [] };
  }

  const applied: string[] = [version === LEGACY_ENVELOPE_VERSION ? "v1_to_v2_character" : "v2_to_v3_envelope"];

  // Both legacy formats are flat payloads: the session block is read with the
  // tolerant reader, which already upgrades the v1 character (no `character`
  // field) to the canonical default character.
  const session = parseSessionState(value);
  if (version === LEGACY_ENVELOPE_VERSION) applied.push("v2_to_v3_envelope");

  const timestamp = num((value as Record<string, unknown>)["timestamp"], options.savedAt ?? Date.now());
  const appVersion = str((value as Record<string, unknown>)["version"], options.appVersion ?? DEFAULT_APP_VERSION);

  return {
    envelope: createEnvelope({
      session,
      coreProject: null,
      ui: {},
      savedAt: timestamp > 0 ? timestamp : Date.now(),
      appVersion,
    }),
    applied,
  };
}

/**
 * Parses a persisted project (file or cache) into an envelope.
 * Throws `PersistenceError` when the payload cannot be trusted.
 */
export function parseEnvelope(rawJson: string, options: { savedAt?: number } = {}): MigrationResult {
  let data: unknown;
  try {
    data = JSON.parse(rawJson);
  } catch (error) {
    fail("NOT_JSON", `JSON malformado (${String(error)})`);
  }
  if (typeof data !== "object" || data === null || Array.isArray(data)) {
    fail("NOT_OBJECT", "a raiz do projeto precisa ser um objeto");
  }
  return migrateEnvelope(data, options);
}

/** Outcome of reading the crash-recovery cache. */
export interface RecoveryResult {
  status: "empty" | "restored" | "invalid";
  envelope: ProjectEnvelopeV3 | null;
  /** Migrations applied while loading, in order. */
  migrations: string[];
  error: { code: PersistenceErrorCode; message: string } | null;
}

/**
 * Reads the recovery cache. A payload that cannot be validated yields
 * `status: "invalid"` **with the reason**, so the UI can tell the user what
 * happened instead of dropping the session silently.
 */
export function readRecovery(rawJson: string | null | undefined): RecoveryResult {
  if (!rawJson) return { status: "empty", envelope: null, migrations: [], error: null };
  try {
    const { envelope, applied } = parseEnvelope(rawJson);
    return { status: "restored", envelope, migrations: applied, error: null };
  } catch (error) {
    const persistenceError =
      error instanceof PersistenceError
        ? error
        : new PersistenceError("INVALID_FIELD", String(error));
    return {
      status: "invalid",
      envelope: null,
      migrations: [],
      error: { code: persistenceError.code, message: persistenceError.message },
    };
  }
}

/** True when a restored session is only as good as the browser preview. */
export function isDegradedRecovery(envelope: ProjectEnvelopeV3): boolean {
  return envelope.source !== "core" || envelope.core_project === null;
}

/**
 * Wraps a renderer/UI snapshot into a session block, filling the scene domain
 * the caller did not provide with the canonical defaults.
 */
export function toSessionState(
  snapshot: ProjectStateSnapshot & { scene?: unknown }
): SessionState {
  const { schemaVersion, character, scene, ...rest } = snapshot;
  return {
    ...rest,
    characterSchemaVersion: schemaVersion ?? CHARACTER_SNAPSHOT_SCHEMA_VERSION,
    character: character ?? sanitizeCharacterState(undefined),
    scene: parseSceneDomain(scene),
  };
}

/**
 * Accepted-command log of a session, validated before it is handed to the
 * history. A corrupt log is dropped (the session itself is still restored), and
 * a log from a newer build is preserved on disk but never partially replayed.
 */
export function commandLogOf(envelope: ProjectEnvelopeV3 | null | undefined): CommandLogWire | null {
  const raw = envelope?.session?.command_log;
  if (!raw) return null;
  if (typeof raw.version !== "number" || raw.version > COMMAND_LOG_VERSION) return null;
  if (!Array.isArray(raw.entries)) return null;
  const entries = raw.entries.filter((entry) => isCommandLogEntry(entry));
  if (entries.length !== raw.entries.length) return null;
  return { version: raw.version, entries };
}

/** Converts a session block back to the flat shape older readers expect. */
export function toProjectSnapshot(session: SessionState): ProjectStateSnapshot {
  const { characterSchemaVersion, ...rest } = session;
  return { ...rest, schemaVersion: characterSchemaVersion };
}

/** Character state of a restored envelope (never null: defaults are applied). */
export function characterOf(envelope: ProjectEnvelopeV3): CharacterState {
  return envelope.session.character ?? sanitizeCharacterState(undefined);
}
