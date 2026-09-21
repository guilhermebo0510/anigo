/**
 * ANIGO Studio — Canonical Character State (P0 Remediation)
 *
 * Single source of truth for the Personagem workspace domain:
 * gender polarity, somatotype domain, slider clamping, snapshot validation.
 *
 * Pure TypeScript — no Svelte, DOM or WebGPU dependencies — so it can be
 * unit-tested under Node (`node --experimental-strip-types --test`).
 *
 * Covers audit findings:
 *  P0-03 polaridade de gênero única (0.0 = Feminino, 1.0 = Masculino)
 *  P0-02 sanitização numérica em todas as fronteiras
 *  P0-07/P0-08 estado de personagem serializável + versionado
 *  P0-09 clamp central por catálogo + validação de ids
 */

import { CANONICAL_SLIDERS, type MorphSlider } from "./morph_catalog";

// ---------------------------------------------------------------------------
// Gender dimorphism — CANONICAL polarity (matches Rust `somatotype.rs`):
//   0.0 = Canonical Female · 1.0 = Canonical Male · 0.5 = Androgynous
// ---------------------------------------------------------------------------

export const GENDER_FEMALE = 0.0;
export const GENDER_ANDROGYNOUS = 0.5;
export const GENDER_MALE = 1.0;

export type BaseGender = "male" | "female";

/** Canonical helper — the ONLY sanctioned mapping between model and scalar. */
export function genderToDimorphism(gender: BaseGender): number {
  return gender === "female" ? GENDER_FEMALE : GENDER_MALE;
}

/** Inverse mapping with hysteresis-free midpoint rule (>= 0.5 → male). */
export function dimorphismToGender(g: number): BaseGender {
  return sanitizeFinite(g, GENDER_ANDROGYNOUS) >= 0.5 ? "male" : "female";
}

export function clampGenderDimorphism(g: unknown): number {
  return clampNumber(sanitizeFinite(g, GENDER_ANDROGYNOUS), 0, 1);
}

// ---------------------------------------------------------------------------
// Numeric sanitation — every numeric boundary MUST use these (P0-02)
// ---------------------------------------------------------------------------

/** Returns `value` when it is a finite number, otherwise `fallback`. */
export function sanitizeFinite(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

export function clampNumber(value: number, min: number, max: number): number {
  const lo = Math.min(min, max);
  const hi = Math.max(min, max);
  if (!Number.isFinite(value)) return lo;
  return Math.min(hi, Math.max(lo, value));
}

// ---------------------------------------------------------------------------
// Somatotype (Heath-Carter barycentric domain: components >= 0, sum = 1)
// ---------------------------------------------------------------------------

export interface SomatotypeCoords {
  endo: number;
  meso: number;
  ecto: number;
}

/** Neutral point shared by pad, inspector, App and (conceptually) Rust default. */
export const NEUTRAL_SOMATOTYPE: Readonly<SomatotypeCoords> = Object.freeze({
  endo: 1 / 3,
  meso: 1 / 3,
  ecto: 1 / 3,
});

/** Shared pad→inspector callback payload (P0-02: single object, never positional). */
export interface SomatotypeUpdate {
  endomorph: number;
  mesomorph: number;
  ectomorph: number;
  genderDimorphism: number;
  isContinuous?: boolean;
}

/** Clamp negatives to 0 and renormalize so endo+meso+ecto = 1. */
export function normalizeSomatotype(
  endo: unknown,
  meso: unknown,
  ecto: unknown
): SomatotypeCoords {
  let e = sanitizeFinite(endo, NEUTRAL_SOMATOTYPE.endo);
  let m = sanitizeFinite(meso, NEUTRAL_SOMATOTYPE.meso);
  let c = sanitizeFinite(ecto, NEUTRAL_SOMATOTYPE.ecto);
  e = Math.max(0, e);
  m = Math.max(0, m);
  c = Math.max(0, c);
  const sum = e + m + c;
  if (!(sum > 0)) return { ...NEUTRAL_SOMATOTYPE };
  return { endo: e / sum, meso: m / sum, ecto: c / sum };
}

export function isValidSomatotype(
  endo: unknown,
  meso: unknown,
  ecto: unknown,
  eps = 1e-4
): boolean {
  if (
    typeof endo !== "number" ||
    typeof meso !== "number" ||
    typeof ecto !== "number"
  )
    return false;
  if (!Number.isFinite(endo) || !Number.isFinite(meso) || !Number.isFinite(ecto))
    return false;
  if (endo < 0 || meso < 0 || ecto < 0) return false;
  return Math.abs(endo + meso + ecto - 1.0) <= eps;
}

// ---------------------------------------------------------------------------
// Slider catalog index — clamp + id validation (P0-04 / P0-09)
// ---------------------------------------------------------------------------

/** Nominal id type for morph sliders. Use `isKnownSliderId` at runtime. */
export type SliderId = string;

const sliderById = new Map<string, MorphSlider>();
for (const s of CANONICAL_SLIDERS) sliderById.set(s.id, s);

export function getSliderDef(id: string): MorphSlider | undefined {
  return sliderById.get(id);
}

export function isKnownSliderId(id: unknown): id is SliderId {
  return typeof id === "string" && sliderById.has(id);
}

/**
 * Central clamp (P0-09): every write path (slider, tactile, MCP, preset)
 * MUST route through here. Unknown ids return NaN-flagged via `null`.
 */
export function clampCatalog(id: string, value: unknown): number | null {
  const def = sliderById.get(id);
  if (!def) return null;
  return clampNumber(sanitizeFinite(value, def.defaultValue), def.min, def.max);
}

/** Returns the list of catalog ids that are NOT in `known` (contract tests). */
export function findUnknownSliderIds(ids: Iterable<string>): string[] {
  const out: string[] = [];
  for (const id of ids) if (!sliderById.has(id)) out.push(id);
  return out;
}

export function defaultSliderValues(): Record<string, number> {
  const out: Record<string, number> = {};
  for (const s of CANONICAL_SLIDERS) out[s.id] = s.defaultValue;
  return out;
}

// ---------------------------------------------------------------------------
// Canonical CharacterState — serializable, versioned, single shape (P0-07/08)
// ---------------------------------------------------------------------------

export const CHARACTER_SNAPSHOT_SCHEMA_VERSION = 2;

export interface CharacterProportions {
  headScale: number;
  headRatio: number;
  shoulderWidth: number;
  legLength: number;
  armLength: number;
  neckLength: number;
  torsoLength: number;
  heightOverall: number;
}

export interface CharacterState {
  schemaVersion: number;
  baseGender: BaseGender;
  activePresetId: string | null;
  somatotype: SomatotypeCoords;
  genderDimorphism: number;
  proportions: CharacterProportions;
  /** Sparse map: only values differing from catalog default need be stored. */
  morphSliders: Record<string, number>;
  hair: { volume: number; thickness: number; curvature: number; strands: number };
  cloth: { layer: string; tension: number; rigidity: number; gravity: number };
  accessory: {
    socket: string;
    scale: number;
    offsetX: number;
    offsetY: number;
    offsetZ: number;
  };
}

export function defaultProportions(): CharacterProportions {
  return {
    headScale: 1.0,
    headRatio: 6.5,
    shoulderWidth: 1.0,
    legLength: 1.0,
    armLength: 1.0,
    neckLength: 1.0,
    torsoLength: 1.0,
    heightOverall: 1.0,
  };
}

export function createDefaultCharacterState(): CharacterState {
  return {
    schemaVersion: CHARACTER_SNAPSHOT_SCHEMA_VERSION,
    baseGender: "male",
    activePresetId: null,
    somatotype: { ...NEUTRAL_SOMATOTYPE },
    genderDimorphism: GENDER_MALE,
    proportions: defaultProportions(),
    morphSliders: {},
    hair: { volume: 1.2, thickness: 0.05, curvature: 0.4, strands: 16 },
    cloth: { layer: "uniforme", tension: 0.5, rigidity: 0.3, gravity: 1.0 },
    accessory: { socket: "head", scale: 1.0, offsetX: 0, offsetY: 0, offsetZ: 0 },
  };
}

/**
 * Deep-sanitize an untrusted character payload (project file, autosave cache,
 * MCP bridge). Never throws; unknown/missing fields fall back to defaults.
 */
export function sanitizeCharacterState(input: unknown): CharacterState {
  const fallback = createDefaultCharacterState();
  if (typeof input !== "object" || input === null) return fallback;
  const o = input as Record<string, unknown>;

  const baseGender: BaseGender = o["baseGender"] === "female" ? "female" : "male";
  const som = (o["somatotype"] ?? {}) as Record<string, unknown>;
  const somatotype = normalizeSomatotype(som["endo"], som["meso"], som["ecto"]);
  const genderDimorphism = clampGenderDimorphism(o["genderDimorphism"]);

  const p = (o["proportions"] ?? {}) as Record<string, unknown>;
  const fp = fallback.proportions;
  const proportions: CharacterProportions = {
    headScale: sanitizeFinite(p["headScale"], fp.headScale),
    headRatio: sanitizeFinite(p["headRatio"], fp.headRatio),
    shoulderWidth: sanitizeFinite(p["shoulderWidth"], fp.shoulderWidth),
    legLength: sanitizeFinite(p["legLength"], fp.legLength),
    armLength: sanitizeFinite(p["armLength"], fp.armLength),
    neckLength: sanitizeFinite(p["neckLength"], fp.neckLength),
    torsoLength: sanitizeFinite(p["torsoLength"], fp.torsoLength),
    heightOverall: sanitizeFinite(p["heightOverall"], fp.heightOverall),
  };

  // Morph sliders: keep only known ids, clamped to catalog range.
  const morphSliders: Record<string, number> = {};
  const raw = o["morphSliders"];
  if (typeof raw === "object" && raw !== null) {
    for (const [id, v] of Object.entries(raw as Record<string, unknown>)) {
      const clamped = clampCatalog(id, v);
      if (clamped !== null) morphSliders[id] = clamped;
    }
  }

  const h = (o["hair"] ?? {}) as Record<string, unknown>;
  const fh = fallback.hair;
  const c = (o["cloth"] ?? {}) as Record<string, unknown>;
  const fc = fallback.cloth;
  const a = (o["accessory"] ?? {}) as Record<string, unknown>;
  const fa = fallback.accessory;

  return {
    schemaVersion: CHARACTER_SNAPSHOT_SCHEMA_VERSION,
    baseGender,
    activePresetId:
      typeof o["activePresetId"] === "string" ? o["activePresetId"] : null,
    somatotype,
    genderDimorphism,
    proportions,
    morphSliders,
    hair: {
      volume: sanitizeFinite(h["volume"], fh.volume),
      thickness: sanitizeFinite(h["thickness"], fh.thickness),
      curvature: sanitizeFinite(h["curvature"], fh.curvature),
      strands: Math.round(sanitizeFinite(h["strands"], fh.strands)),
    },
    cloth: {
      layer: typeof c["layer"] === "string" ? c["layer"] : fc.layer,
      tension: sanitizeFinite(c["tension"], fc.tension),
      rigidity: sanitizeFinite(c["rigidity"], fc.rigidity),
      gravity: sanitizeFinite(c["gravity"], fc.gravity),
    },
    accessory: {
      socket: typeof a["socket"] === "string" ? a["socket"] : fa.socket,
      scale: sanitizeFinite(a["scale"], fa.scale),
      offsetX: sanitizeFinite(a["offsetX"], fa.offsetX),
      offsetY: sanitizeFinite(a["offsetY"], fa.offsetY),
      offsetZ: sanitizeFinite(a["offsetZ"], fa.offsetZ),
    },
  };
}

/** Deep-equal for round-trip tests (JSON-stable shapes only). */
export function characterStatesEqual(a: CharacterState, b: CharacterState): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}
