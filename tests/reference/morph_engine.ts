/**
 * ANIGO Studio — Reference morph deformation engine (TEST-ONLY).
 *
 * P0 §7.4 removed this engine from production: the canonical deformation lives
 * in the Rust core (`crates/anigo-core/src/{deformation,morph_catalog}.rs`) and
 * reaches the viewport as a core snapshot. This file is kept **exclusively** as
 * the reference oracle for contract tests — the parity between the Rust core and
 * this implementation is what the P0 tests assert.
 *
 * It must never be imported by `src/**`: the P0 contract tests fail if a
 * production module imports it (ARQUITETURA §2.2 / §4.1).
 *
 * Pure TypeScript — no DOM/WebGPU — unit-tested under Node.
 */

import type { MorphSlider } from "../../src/services/morph_catalog";

// ---------------------------------------------------------------------------
// Explicit (hand-authored) slider ids of the reference engine. The Rust core
// covers the whole catalog, so this set only documents which sliders the
// reference oracle implements with hand-authored math.
// ---------------------------------------------------------------------------

export const EXPLICIT_TS_MORPH_IDS: ReadonlySet<string> = new Set([
  "head_width",
  "head_depth",
  "face_lower_length",
  "forehead_height",
  "brow_ridge_prominence",
  "cheekbone_prominence",
  "jaw_v_line_taper",
  "chin_length",
  "chin_forward_projection",
  "eye_scale_uniform",
  "eye_canthal_tilt",
  "lower_eyelid_aegyosal",
  "nose_bridge_depth",
  "nose_tip_upturn",
  "anime_profile_slant",
  "ear_pointy_elf",
  "adams_apple_prominence",
  "trapezius_bulk",
  "neck_circumference",
  "bust_volume_cup",
  "bust_gravity_sag",
  "bust_separation_cleavage",
  "pectoral_muscle_bulk",
  "waist_pinch_width",
  "abs_sixpack_definition",
  "belly_visceral_protuberance",
  "hip_trochanteric_flare",
  "gluteus_volume_overall",
  "gluteus_shape_profile",
  "biceps_peak_volume",
  "upper_arm_thickness",
  "thigh_circumference",
  "inner_thigh_gap",
  "knee_patella_prominence",
  "knee_valgus_uchimata",
  "calf_circumference",
]);

export function isExplicitMorph(id: string): boolean {
  return EXPLICIT_TS_MORPH_IDS.has(id);
}

// ---------------------------------------------------------------------------
// Weight normalization → channel semantics (P0-05)
// ---------------------------------------------------------------------------

/**
 * Normalized weight: +1 at max, -1 at min, 0 at default.
 * Handles edge-default sliders (default == min) without division by zero.
 */
export function normalizeWeight(def: MorphSlider, value: number): number {
  const v = Number.isFinite(value) ? value : def.defaultValue;
  const d = v - def.defaultValue;
  if (d === 0) return 0;
  if (d > 0) {
    const span = def.max - def.defaultValue;
    return span > 0 ? d / span : 0;
  }
  const span = def.defaultValue - def.min;
  return span > 0 ? d / span : 0;
}

/**
 * Denominator mapping raw (value - default) to channel weight for the GPU
 * sparse path: delta_full_weight = deform(max) - base.
 */
export function channelDenominator(def: MorphSlider): number {
  const up = def.max - def.defaultValue;
  if (up > 0) return up;
  const down = def.defaultValue - def.min;
  if (down > 0) return down;
  return 1;
}

// ---------------------------------------------------------------------------
// Deterministic hashing (FNV-1a) — stable across sessions/platforms
// ---------------------------------------------------------------------------

export function hashString32(str: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < str.length; i++) {
    h ^= str.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

// ---------------------------------------------------------------------------
// Zone regions — relative-height bands (robust to any GLB vertex order,
// unlike magic index ranges). t = (y - minY) / (maxY - minY) in [0, 1].
// Canonical mannequin spans y ≈ 0..1.7 (head center ≈ 1.60).
// ---------------------------------------------------------------------------

export interface MeshBounds {
  minY: number;
  maxY: number;
}

export interface ZoneRegion {
  tMin: number;
  tMax: number;
  /** |x| window (null = anywhere) */
  xAbsMin: number;
  xAbsMax: number;
  /** z window: "front" | "back" | "any" */
  zSide: "front" | "back" | "any";
  /** base amplitude in world units at |w| = 1 */
  amplitude: number;
}

const ZONE_REGIONS: Record<string, ZoneRegion> = {
  GlobalSilhouette: { tMin: 0.0, tMax: 1.0, xAbsMin: 0, xAbsMax: 99, zSide: "any", amplitude: 0.05 },
  Craniofacial: { tMin: 0.88, tMax: 1.0, xAbsMin: 0, xAbsMax: 99, zSide: "any", amplitude: 0.02 },
  Eyes: { tMin: 0.9, tMax: 0.975, xAbsMin: 0.005, xAbsMax: 0.1, zSide: "front", amplitude: 0.012 },
  Eyebrows: { tMin: 0.925, tMax: 0.995, xAbsMin: 0.005, xAbsMax: 0.1, zSide: "front", amplitude: 0.012 },
  Nose: { tMin: 0.885, tMax: 0.955, xAbsMin: 0, xAbsMax: 0.035, zSide: "front", amplitude: 0.012 },
  MouthLips: { tMin: 0.85, tMax: 0.9, xAbsMin: 0, xAbsMax: 0.06, zSide: "front", amplitude: 0.01 },
  JawChin: { tMin: 0.83, tMax: 0.9, xAbsMin: 0, xAbsMax: 99, zSide: "any", amplitude: 0.014 },
  Ears: { tMin: 0.89, tMax: 0.975, xAbsMin: 0.06, xAbsMax: 99, zSide: "any", amplitude: 0.014 },
  NeckTrapezius: { tMin: 0.79, tMax: 0.89, xAbsMin: 0, xAbsMax: 0.12, zSide: "any", amplitude: 0.018 },
  ShouldersClavicles: { tMin: 0.72, tMax: 0.85, xAbsMin: 0.05, xAbsMax: 99, zSide: "any", amplitude: 0.025 },
  ChestPectorals: { tMin: 0.64, tMax: 0.8, xAbsMin: 0, xAbsMax: 0.2, zSide: "any", amplitude: 0.025 },
  BustFemale: { tMin: 0.6, tMax: 0.79, xAbsMin: 0, xAbsMax: 0.18, zSide: "front", amplitude: 0.03 },
  AbdomenWaist: { tMin: 0.5, tMax: 0.68, xAbsMin: 0, xAbsMax: 0.2, zSide: "any", amplitude: 0.03 },
  PelvisHips: { tMin: 0.42, tMax: 0.57, xAbsMin: 0, xAbsMax: 99, zSide: "any", amplitude: 0.03 },
  Gluteus: { tMin: 0.4, tMax: 0.55, xAbsMin: 0, xAbsMax: 0.2, zSide: "back", amplitude: 0.03 },
  UpperLimbs: { tMin: 0.48, tMax: 0.82, xAbsMin: 0.1, xAbsMax: 99, zSide: "any", amplitude: 0.022 },
  HandsFingers: { tMin: 0.4, tMax: 0.58, xAbsMin: 0.16, xAbsMax: 99, zSide: "any", amplitude: 0.012 },
  LowerLimbs: { tMin: 0.0, tMax: 0.46, xAbsMin: 0, xAbsMax: 99, zSide: "any", amplitude: 0.028 },
};

export function getZoneRegion(zoneKey: string): ZoneRegion {
  return (
    ZONE_REGIONS[zoneKey] ?? {
      tMin: 0, tMax: 1, xAbsMin: 0, xAbsMax: 99, zSide: "any", amplitude: 0.02,
    }
  );
}

function smoothstep(a: number, b: number, x: number): number {
  const t = Math.min(1, Math.max(0, (x - a) / (b - a)));
  return t * t * (3 - 2 * t);
}

// Direction modes — 8 distinct unit-ish directions. The per-slider mode +
// jittered falloff center + amplitude scale guarantee pairwise-distinct
// deltas within a zone (P0-04 acceptance test).
const DIRECTION_MODES = 8;

function directionFor(
  mode: number,
  x: number,
  nx: number,
  ny: number,
  nz: number,
  out: [number, number, number]
): void {
  const sx = x > 0 ? 1 : x < 0 ? -1 : 1;
  switch (mode % DIRECTION_MODES) {
    case 0: out[0] = sx; out[1] = 0; out[2] = 0; break; // lateral outward
    case 1: out[0] = 0; out[1] = 1; out[2] = 0; break; // up
    case 2: out[0] = 0; out[1] = 0; out[2] = 1; break; // front
    case 3: out[0] = nx; out[1] = ny; out[2] = nz; break; // radial (normal)
    case 4: out[0] = -sx; out[1] = 0; out[2] = 0; break; // lateral pinch
    case 5: out[0] = 0; out[1] = sx; out[2] = 0; break; // vertical shear by side
    case 6: out[0] = 0.5 * sx; out[1] = -0.5; out[2] = 0.7; break; // fwd-down diag
    default: out[0] = -nx; out[1] = -ny; out[2] = -nz; break; // inward normal
  }
}

export interface GenericDelta {
  dx: number;
  dy: number;
  dz: number;
}

/**
 * Computes the generic (procedural) delta for one vertex.
 * Returns null when the vertex is outside the slider's support (sparsity).
 *
 * @param slider catalog definition (zone selects the region, id the direction)
 * @param normWeight normalized weight from `normalizeWeight` (≈ [-1, 1])
 */
export function genericMorphDelta(
  slider: MorphSlider,
  normWeight: number,
  x: number,
  y: number,
  z: number,
  nx: number,
  ny: number,
  nz: number,
  bounds: MeshBounds,
  out: GenericDelta = { dx: 0, dy: 0, dz: 0 }
): GenericDelta | null {
  if (!Number.isFinite(normWeight) || normWeight === 0) return null;
  const h = hashString32(slider.id);
  const region = getZoneRegion(slider.zoneKey);

  // Global silhouette sliders use whole-body scale modes (distinct per id).
  if (slider.zoneKey === "GlobalSilhouette") {
    return globalSilhouetteDelta(slider.id, normWeight, x, y, z, bounds, out);
  }

  const spanY = Math.max(1e-6, bounds.maxY - bounds.minY);
  const t = (y - bounds.minY) / spanY;

  // Per-slider jittered band: guarantees distinct support per slider.
  const jitter = ((h >>> 8) % 1000) / 1000 - 0.5; // [-0.5, 0.5)
  const bandW = Math.max(1e-3, region.tMax - region.tMin);
  const center = (region.tMin + region.tMax) / 2 + jitter * bandW * 0.35;
  const halfW = (bandW / 2) * (0.75 + ((h >>> 18) % 1000) / 1000 * 0.5);
  const edge = Math.max(1e-4, halfW * 0.35);
  const fallT =
    smoothstep(center - halfW, center - halfW + edge, t) *
    (1 - smoothstep(center + halfW - edge, center + halfW, t));
  if (fallT <= 1e-4) return null;

  const ax = Math.abs(x);
  let fallX = 1;
  if (region.xAbsMin > 0 || region.xAbsMax < 90) {
    fallX =
      smoothstep(region.xAbsMin * 0.7, region.xAbsMin + 0.01, ax) *
      (1 - smoothstep(region.xAbsMax, region.xAbsMax + 0.05, ax));
    if (fallX <= 1e-4) return null;
  }
  let fallZ = 1;
  if (region.zSide === "front") {
    fallZ = smoothstep(-0.01, 0.03, z);
    if (fallZ <= 1e-4) return null;
  } else if (region.zSide === "back") {
    fallZ = 1 - smoothstep(-0.03, 0.01, z);
    if (fallZ <= 1e-4) return null;
  }

  const ampScale = 0.8 + ((h >>> 4) % 1000) / 1000 * 0.4; // 0.8..1.2
  const k = region.amplitude * ampScale * normWeight * fallT * fallX * fallZ;
  if (Math.abs(k) < 1e-7) return null;

  const dir: [number, number, number] = [0, 0, 0];
  directionFor(h % DIRECTION_MODES, x, nx, ny, nz, dir);
  out.dx = dir[0] * k;
  out.dy = dir[1] * k;
  out.dz = dir[2] * k;
  return out;
}

function globalSilhouetteDelta(
  id: string,
  w: number,
  x: number,
  y: number,
  z: number,
  bounds: MeshBounds,
  out: GenericDelta
): GenericDelta | null {
  const spanY = Math.max(1e-6, bounds.maxY - bounds.minY);
  const t = (y - bounds.minY) / spanY;
  const rel = y - bounds.minY;
  switch (id) {
    case "height_overall": // Y scale about ground
      out.dx = 0; out.dy = rel * 0.08 * w; out.dz = 0;
      break;
    case "head_to_body_ratio": // head emphasis + slight leg counter-scale
      if (t > 0.885) { out.dx = 0; out.dy = (y - 1.5) * 0.12 * w; out.dz = 0; }
      else { out.dx = 0; out.dy = -rel * 0.015 * w; out.dz = 0; }
      break;
    case "head_scale_uniform": { // radial from head center
      if (t < 0.885) return null;
      const cx = 0, cy = bounds.minY + spanY * 0.94, cz = 0;
      out.dx = (x - cx) * 0.1 * w; out.dy = (y - cy) * 0.1 * w; out.dz = (z - cz) * 0.1 * w;
      break;
    }
    case "torso_to_limb_ratio": // torso up, legs down
      if (t >= 0.44 && t <= 0.82) { out.dx = 0; out.dy = 0.035 * w; out.dz = 0; }
      else if (t < 0.44) { out.dx = 0; out.dy = -0.03 * w; out.dz = 0; }
      else return null;
      break;
    case "somatotype_endomorph": // torso+thigh volume
      if (t >= 0.4 && t <= 0.82) { out.dx = x * 0.06 * w; out.dy = 0; out.dz = z * 0.06 * w; }
      else if (t < 0.4) { out.dx = x * 0.04 * w; out.dy = 0; out.dz = z * 0.04 * w; }
      else return null;
      break;
    case "somatotype_mesomorph": // chest/arms/legs athletic volume
      if ((t >= 0.55 && t <= 0.82) || (t < 0.45 && Math.abs(x) > 0.05)) {
        out.dx = x * 0.05 * w; out.dy = 0; out.dz = z * 0.05 * w;
      } else return null;
      break;
    case "somatotype_ectomorph": // slenderize (inverse volume)
      if (t >= 0.4 && t <= 0.85) { out.dx = -x * 0.045 * w; out.dy = 0; out.dz = -z * 0.045 * w; }
      else return null;
      break;
    case "spine_s_curvature": { // S-line posture curve
      if (t < 0.44 || t > 0.85) return null;
      const s = Math.sin(((t - 0.44) / 0.41) * Math.PI);
      out.dx = 0; out.dy = 0; out.dz = s * 0.03 * w;
      break;
    }
    default: { // future-proof fallback for new global sliders
      const h = hashString32(id);
      const dir: [number, number, number] = [0, 0, 0];
      directionFor(h % DIRECTION_MODES, x, 0, 1, 0, dir);
      const k = 0.03 * w;
      out.dx = dir[0] * k; out.dy = dir[1] * k; out.dz = dir[2] * k;
      break;
    }
  }
  if (Math.abs(out.dx) + Math.abs(out.dy) + Math.abs(out.dz) < 1e-9) return null;
  return out;
}

// ---------------------------------------------------------------------------
// Normal recomputation (P0-06) — area-independent face accumulation,
// identical math to the former inline renderer block, now pure + tested.
// ---------------------------------------------------------------------------

/**
 * Recomputes smooth vertex normals in place.
 * @param positions xyz per vertex (deformed positions)
 * @param indices triangle indices
 * @param normals xyz per vertex, overwritten with unit normals
 */
export function recomputeNormals(
  positions: Float32Array,
  indices: Uint32Array | number[] | Uint16Array,
  normals: Float32Array
): void {
  const count = normals.length / 3;
  const acc = new Float32Array(normals.length);
  const idx = indices as ArrayLike<number>;
  const triCount = Math.floor(idx.length / 3);
  for (let t = 0; t < triCount; t++) {
    const a = idx[t * 3] | 0;
    const b = idx[t * 3 + 1] | 0;
    const c = idx[t * 3 + 2] | 0;
    if (a < 0 || b < 0 || c < 0 || a >= count || b >= count || c >= count) continue;
    const ax = positions[a * 3], ay = positions[a * 3 + 1], az = positions[a * 3 + 2];
    const bx = positions[b * 3], by = positions[b * 3 + 1], bz = positions[b * 3 + 2];
    const cx = positions[c * 3], cy = positions[c * 3 + 1], cz = positions[c * 3 + 2];
    const abx = bx - ax, aby = by - ay, abz = bz - az;
    const acx = cx - ax, acy = cy - ay, acz = cz - az;
    let nx = aby * acz - abz * acy;
    let ny = abz * acx - abx * acz;
    let nz = abx * acy - aby * acx;
    const len = Math.hypot(nx, ny, nz);
    if (!(len > 1e-12)) continue;
    nx /= len; ny /= len; nz /= len;
    acc[a * 3] += nx; acc[a * 3 + 1] += ny; acc[a * 3 + 2] += nz;
    acc[b * 3] += nx; acc[b * 3 + 1] += ny; acc[b * 3 + 2] += nz;
    acc[c * 3] += nx; acc[c * 3 + 1] += ny; acc[c * 3 + 2] += nz;
  }
  for (let i = 0; i < count; i++) {
    const x = acc[i * 3], y = acc[i * 3 + 1], z = acc[i * 3 + 2];
    const l = Math.hypot(x, y, z);
    if (l > 1e-6) {
      normals[i * 3] = x / l;
      normals[i * 3 + 1] = y / l;
      normals[i * 3 + 2] = z / l;
    }
  }
}

// ---------------------------------------------------------------------------
// Sparse morph set (P0-05) — WGSL-compatible layout
//   SparseMorphDelta { vertex_index: u32, dpx/y/z: f32, dnx/y/z: f32, pad: f32 }
//   MorphChannel     { weight: f32, start_offset: u32, delta_count: u32, pad: u32 }
// Buffers are built with mixed Uint32/Float32 views over one ArrayBuffer so
// integer fields are bit-exact (never float-cast) for the GPU.
// ---------------------------------------------------------------------------

export interface SparseChannelDef {
  sliderId: string;
  startOffset: number;
  deltaCount: number;
}

export interface SparseMorphSet {
  channels: SparseChannelDef[];
  /** 8 floats per delta: [indexBits, dx,dy,dz, dnx,dny,dnz, pad] */
  deltasF32: Float32Array;
  totalVertices: number;
  totalDeltas: number;
}

/**
 * Builds a sparse set by diffing per-channel deformed positions/normals
 * against the base. Deltas are stored vertex-ordered (required by the
 * shader's binary search).
 */
export function buildSparseMorphSet(
  totalVertices: number,
  basePositions: Float32Array,
  baseNormals: Float32Array,
  perChannel: Array<{ sliderId: string; positions: Float32Array; normals: Float32Array }>,
  threshold = 1e-6
): SparseMorphSet {
  const channels: SparseChannelDef[] = [];
  const flat: number[] = [];
  for (const ch of perChannel) {
    const start = flat.length / 8;
    let count = 0;
    for (let v = 0; v < totalVertices; v++) {
      const dx = ch.positions[v * 3] - basePositions[v * 3];
      const dy = ch.positions[v * 3 + 1] - basePositions[v * 3 + 1];
      const dz = ch.positions[v * 3 + 2] - basePositions[v * 3 + 2];
      const nx = ch.normals[v * 3] - baseNormals[v * 3];
      const ny = ch.normals[v * 3 + 1] - baseNormals[v * 3 + 1];
      const nz = ch.normals[v * 3 + 2] - baseNormals[v * 3 + 2];
      if (
        Math.abs(dx) > threshold || Math.abs(dy) > threshold || Math.abs(dz) > threshold ||
        Math.abs(nx) > threshold || Math.abs(ny) > threshold || Math.abs(nz) > threshold
      ) {
        flat.push(v, dx, dy, dz, nx, ny, nz, 0);
        count++;
      }
    }
    channels.push({ sliderId: ch.sliderId, startOffset: start, deltaCount: count });
  }
  const buffer = new ArrayBuffer(flat.length * 4);
  const f32 = new Float32Array(buffer);
  const u32 = new Uint32Array(buffer);
  for (let i = 0; i < flat.length; i += 8) {
    u32[i] = flat[i] >>> 0; // vertex index — bit-exact u32
    f32[i + 1] = flat[i + 1];
    f32[i + 2] = flat[i + 2];
    f32[i + 3] = flat[i + 3];
    f32[i + 4] = flat[i + 4];
    f32[i + 5] = flat[i + 5];
    f32[i + 6] = flat[i + 6];
    f32[i + 7] = 0;
  }
  return {
    channels,
    deltasF32: f32,
    totalVertices,
    totalDeltas: flat.length / 8,
  };
}

/** Packs channel weights for the GPU (bit-exact offsets/counts). */
export function packChannelWeights(
  set: SparseMorphSet,
  weights: Map<string, number> | Record<string, number>
): Float32Array {
  const buffer = new ArrayBuffer(set.channels.length * 16);
  const f32 = new Float32Array(buffer);
  const u32 = new Uint32Array(buffer);
  const get = (id: string): number => {
    const w =
      weights instanceof Map
        ? weights.get(id)
        : (weights as Record<string, number>)[id];
    return typeof w === "number" && Number.isFinite(w) ? w : 0;
  };
  for (let i = 0; i < set.channels.length; i++) {
    const ch = set.channels[i];
    f32[i * 4] = get(ch.sliderId);
    u32[i * 4 + 1] = ch.startOffset >>> 0;
    u32[i * 4 + 2] = ch.deltaCount >>> 0;
    u32[i * 4 + 3] = 0;
  }
  return f32;
}

/** CPU reference of the WGSL accumulation (used by tests + fallback checks). */
export function applySparseCpu(
  basePositions: Float32Array,
  baseNormals: Float32Array,
  set: SparseMorphSet,
  weights: Map<string, number> | Record<string, number>,
  outPositions: Float32Array,
  outNormals: Float32Array
): void {
  outPositions.set(basePositions);
  outNormals.set(baseNormals);
  const get = (id: string): number => {
    const w =
      weights instanceof Map
        ? weights.get(id)
        : (weights as Record<string, number>)[id];
    return typeof w === "number" && Number.isFinite(w) ? w : 0;
  };
  const u32 = new Uint32Array(
    set.deltasF32.buffer,
    set.deltasF32.byteOffset,
    set.deltasF32.length
  );
  const f32 = set.deltasF32;
  for (const ch of set.channels) {
    const w = get(ch.sliderId);
    if (Math.abs(w) <= 1e-9 || ch.deltaCount === 0) continue;
    for (let k = 0; k < ch.deltaCount; k++) {
      const base = (ch.startOffset + k) * 8;
      const v = u32[base];
      outPositions[v * 3] += w * f32[base + 1];
      outPositions[v * 3 + 1] += w * f32[base + 2];
      outPositions[v * 3 + 2] += w * f32[base + 3];
      outNormals[v * 3] += w * f32[base + 4];
      outNormals[v * 3 + 1] += w * f32[base + 5];
      outNormals[v * 3 + 2] += w * f32[base + 6];
    }
  }
  // normalize (mirrors the shader epilogue)
  for (let v = 0; v < set.totalVertices; v++) {
    const x = outNormals[v * 3], y = outNormals[v * 3 + 1], z = outNormals[v * 3 + 2];
    const l2 = x * x + y * y + z * z;
    if (l2 > 1e-12) {
      const l = Math.sqrt(l2);
      outNormals[v * 3] = x / l;
      outNormals[v * 3 + 1] = y / l;
      outNormals[v * 3 + 2] = z / l;
    }
  }
}
