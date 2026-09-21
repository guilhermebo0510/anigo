/**
 * ANIGO — ProjectState v1 contract (TypeScript side).
 *
 * Mirrors `crates/anigo-core/src/project.rs` + `crates/anigo-core/src/ids.rs`.
 * The TypeScript layer never *authors* project data: it sends commands and
 * reads snapshots. This module exists so the UI can
 *  1. talk about entities with the same stable ids the core uses,
 *  2. carry the canonical project JSON inside autosave files without touching it,
 *  3. fail loudly when a payload does not match the versioned contract.
 */

/** Schema version of the canonical project document. */
export const PROJECT_SCHEMA_VERSION = 1;
/** Maximum accepted length of an encoded stable id (mirrors Rust `ID_MAX_LEN`). */
export const ID_MAX_LEN = 128;

/** Canonical id prefixes — contract-stable (`crates/anigo-core/src/ids.rs`). */
export const ID_PREFIXES = {
  project: "prj",
  character: "chr",
  morph: "mrf",
  scene: "scn",
  node: "nod",
  material: "mat",
  asset: "ast",
  camera: "cam",
  light: "lgt",
  animation: "ani",
} as const;

export type StableIdKind = keyof typeof ID_PREFIXES;

const SLUG_PATTERN = /^[a-z0-9_]+$/;

/** Validates the `<prefix>_<slug>` rule for a given entity kind. */
export function isValidStableId(raw: unknown, kind: StableIdKind): raw is string {
  if (typeof raw !== "string" || raw.length === 0 || raw.length > ID_MAX_LEN) return false;
  const prefix = ID_PREFIXES[kind];
  if (!raw.startsWith(`${prefix}_`)) return false;
  const slug = raw.slice(prefix.length + 1);
  return slug.length > 0 && SLUG_PATTERN.test(slug);
}

/** Stable morph id of a catalog slider: `head_width → mrf_head_width`. */
export function morphIdForSlider(sliderId: string): string {
  return `${ID_PREFIXES.morph}_${sliderId}`;
}

/** Recovers the catalog slider id from a stable morph id. */
export function sliderIdFromMorphId(morphId: string): string {
  const prefix = `${ID_PREFIXES.morph}_`;
  return morphId.startsWith(prefix) ? morphId.slice(prefix.length) : morphId;
}

/** Canonical ids created by `ProjectState::default()` in Rust. */
export const CANONICAL_IDS = {
  projectId: "prj_untitled",
  characterId: "chr_canonical",
  sceneId: "scn_main",
  cameraId: "cam_viewport",
  keyLightId: "lgt_key",
  defaultMaterialId: "mat_default_anime",
  characterNodeId: "nod_character_base",
} as const;

/** Canonical asset URIs (`crates/anigo-core/src/project.rs`). */
export const CANONICAL_URIS = {
  baseMale: "anigo://base/anigo_base_male.glb",
  baseFemale: "anigo://base/anigo_base_female.glb",
  presetCube: "anigo://preset/cube",
  presetSphere: "anigo://preset/uv_sphere",
} as const;

/**
 * Top-level fields of the canonical project document. A contract test asserts
 * this list stays identical to the Rust struct and to the generated JSON Schema.
 */
export const PROJECT_STATE_V1_FIELDS = [
  "schema_version",
  "project_id",
  "name",
  "character",
  "scene",
  "materials",
  "animation",
  "render",
  "assets",
  "settings",
] as const;

/** Unknown keys are preserved verbatim by the core (`extensions`). */
export const PROJECT_STATE_EXTENSIONS_FIELD = "extensions";

/** Envelope written by autosave: legacy TS fields + the canonical core document. */
export interface ProjectEnvelopeV1 {
  /** Legacy presentation fields kept for older builds (deprecated). */
  [legacyField: string]: unknown;
  /** Canonical project document produced by the Rust core (`ProjectState::to_value`). */
  coreProject?: CanonicalProjectDocumentV1;
}

export interface CanonicalProjectDocumentV1 {
  schema_version: number;
  project_id: string;
  name: string;
  character: Record<string, unknown>;
  scene: Record<string, unknown>;
  materials: Record<string, unknown>;
  animation: Record<string, unknown>;
  render: Record<string, unknown>;
  assets: Record<string, unknown>;
  settings: Record<string, unknown>;
  [extension: string]: unknown;
}

/** Raised when a payload cannot be interpreted as a project document. */
export class ProjectContractError extends Error {
  readonly code: string;
  constructor(code: string, detail: string) {
    super(`[ProjectState ${code}] ${detail}`);
    this.name = "ProjectContractError";
    this.code = code;
  }
}

/**
 * Validates the *envelope* of a canonical project document.
 *
 * Deep validation belongs to the Rust core (`ProjectState::validate`, which is
 * the only place allowed to decide what a valid project is); the UI only needs
 * to know whether it is holding a canonical document and which schema version
 * it claims, so it can refuse to load a project from a newer build.
 */
export function readCanonicalProject(value: unknown): CanonicalProjectDocumentV1 {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new ProjectContractError("NOT_AN_OBJECT", "project document must be a JSON object");
  }
  const document = value as Record<string, unknown>;
  const schemaVersion = document["schema_version"];
  if (typeof schemaVersion !== "number" || !Number.isInteger(schemaVersion)) {
    throw new ProjectContractError(
      "MISSING_SCHEMA_VERSION",
      "canonical project document must carry an integer 'schema_version'"
    );
  }
  if (schemaVersion > PROJECT_SCHEMA_VERSION) {
    throw new ProjectContractError(
      "NEWER_SCHEMA",
      `project schema_version ${schemaVersion} is newer than the supported ${PROJECT_SCHEMA_VERSION}`
    );
  }
  if (schemaVersion < PROJECT_SCHEMA_VERSION) {
    throw new ProjectContractError(
      "OLDER_SCHEMA",
      `project schema_version ${schemaVersion} requires a migration (supported: ${PROJECT_SCHEMA_VERSION})`
    );
  }

  const missing: string[] = [];
  for (const field of PROJECT_STATE_V1_FIELDS) {
    if (!(field in document)) missing.push(field);
  }
  // `extensions` is optional on the wire: the core re-emits it when present.
  if (missing.length > 0) {
    throw new ProjectContractError(
      "MISSING_FIELDS",
      `canonical project document is missing: ${missing.join(", ")}`
    );
  }

  const projectId = document["project_id"];
  if (!isValidStableId(projectId, "project")) {
    throw new ProjectContractError(
      "INVALID_PROJECT_ID",
      `project_id '${String(projectId)}' does not match the stable id contract`
    );
  }

  return document as unknown as CanonicalProjectDocumentV1;
}

/** Extracts the canonical document from an autosave envelope, when present. */
export function readProjectEnvelope(value: unknown): CanonicalProjectDocumentV1 | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return null;
  const envelope = value as ProjectEnvelopeV1;
  if (!("coreProject" in envelope) || envelope.coreProject == null) return null;
  return readCanonicalProject(envelope.coreProject);
}
