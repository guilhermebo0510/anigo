/**
 * ANIGO — CoreSnapshot v1 contract (TypeScript side).
 *
 * Mirrors `crates/anigo-core/src/snapshot.rs`. The binary layouts are frozen
 * and validated on both sides against `contracts/fixtures/core_snapshot_v1.json`.
 *
 * This module only *decodes* what the Rust core produced. It never recomputes
 * deformation: that is the whole point of the snapshot contract
 * (ARQUITETURA_CANONICA_ANIGO §2.2 / §5).
 */

/** Version of the snapshot wire format. */
export const SNAPSHOT_FORMAT_VERSION = 1;
/** Bytes per packed vertex: pos f32x3 | normal f32x3 | uv f32x2 | color f32x4 | joints u16x4 | weights f32x4. */
export const VERTEX_STRIDE_BYTES = 72;
/** Bytes per sparse morph delta: vertex_index u32 | dpos f32x3 | dnormal f32x3 | pad f32. */
export const MORPH_DELTA_STRIDE_BYTES = 32;
/** Bytes per morph channel descriptor: weight f32 | start_offset u32 | delta_count u32 | pad u32. */
export const MORPH_CHANNEL_STRIDE_BYTES = 16;
/** Channels with |weight| below this value are treated as inactive (mirrors Rust `epsilon`). */
export const MORPH_WEIGHT_EPSILON = 1e-6;

export type BaseGenderWire = "Male" | "Female";

export type DeformationAuthority = "core" | "reference_ts";

export type ColorSpaceWire = "srgb" | "linear_srgb" | "display_p3";
export type TonemapOperatorWire = "none" | "reinhard" | "neutral";

export interface ColorManagementWire {
  input_texture_space: ColorSpaceWire;
  working_space: ColorSpaceWire;
  display_space: ColorSpaceWire;
}

export interface DeformationCoverage {
  total_sliders: number;
  sliders_with_geometry: number;
  morph_targets: number;
  /** Macro proportions are part of the core base geometry. */
  bakes_proportions: boolean;
  /** Gender dimorphism is part of the core base geometry. */
  bakes_gender: boolean;
  /** Somatotype is expanded into canonical macro sliders by the core. */
  somatotype_via_macro_sliders: boolean;
  proportion_policy_version: number;
  somatotype_policy_version: number;
}

export interface MorphChannelDescriptorWire {
  target: string;
  slider_id: string;
  start_offset: number;
  delta_count: number;
}

export interface MorphWeightWire {
  target: string;
  slider_id: string;
  value: number;
  weight: number;
}

/**
 * P1-04: skinning entregue pelo núcleo junto da geometria estática.
 *
 * `palette` é `world × inverse bind` por osso (16 floats por osso, ordem de
 * colunas), o mesmo layout do bloco `bones` do render contract. Enquanto o
 * núcleo assa as proporções na malha base, a paleta é a identidade — e a
 * atribuição de ossos é neutra (`Σ wᵢ · (I · p) = p`).
 */
export interface SkinPayloadWire {
  bone_count: number;
  palette: number[];
  bind_pose: string;
  proportions_baked: boolean;
  palette_is_identity: boolean;
}

export interface StaticGeometryPayloadWire {
  format_version: number;
  static_revision: number;
  base_gender: BaseGenderWire;
  mesh_asset_id: string;
  mesh_uri: string;
  vertex_count: number;
  index_count: number;
  vertex_stride_bytes: number;
  vertex_buffer_base64: string;
  index_buffer_base64: string;
  topology_hash: string;
  morph_delta_stride_bytes: number;
  morph_channels: MorphChannelDescriptorWire[];
  morph_deltas_base64: string;
  morph_total_deltas: number;
  catalog_fingerprint: string;
  /** P1-04: paleta de skinning (a malha já vem com joints/weights atribuídos). */
  skin: SkinPayloadWire;
}

export interface CameraSnapshotWire {
  eye: [number, number, number];
  target: [number, number, number];
  up: [number, number, number];
  fov_y_radians: number;
  z_near: number;
  z_far: number;
  aspect: number;
}

export interface LightSnapshotWire {
  light_id: string;
  direction: [number, number, number];
  color: [number, number, number];
  intensity: number;
  shadow_color: [number, number, number];
  ambient_intensity: number;
  shadow_saturation: number;
  ambient_sky: [number, number, number];
  ambient_ground: [number, number, number];
}

export interface MaterialSnapshotWire {
  material_id: string;
  name: string;
  base_color: [number, number, number, number];
  shade_color: [number, number, number, number];
  outline_color: [number, number, number, number];
  outline_width: number;
  outline_opacity: number;
  outline_smoothness: number;
  outline_depth_bias: number;
  shadow_threshold: number;
  shadow_smoothness: number;
  specular_color: [number, number, number, number];
  spec_intensity: number;
  spec_power: number;
  specular_softness: number;
  specular_offset: number;
  specular_size: number;
  rim_color: [number, number, number, number];
  rim_intensity: number;
  rim_spread: number;
  hue_shift: number;
  toon_steps: number;
  ao_intensity: number;
  // Fase 2 (#18): material anime VRoid/MToon (VRMC_materials_mtoon)
  mtoon_emission_color: [number, number, number, number];
  mtoon_emission_intensity: number;
  mtoon_second_shade_shift: number;
  mtoon_second_shade_softness: number;
  mtoon_matcap_intensity: number;
  mtoon_main_texture_enabled: boolean;
  mtoon_shade_texture_enabled: boolean;
  mtoon_second_shade_texture_enabled: boolean;
  mtoon_emission_texture_enabled: boolean;
  mtoon_matcap_enabled: boolean;
  /** 0 = normal (mult), 1 = additive. */
  mtoon_matcap_mode: number;
  mtoon_shade_toony: boolean;
  // Fase 2 (#17): sombra facial SDF
  face_shadow_offset: number;
  face_shadow_smoothness: number;
  face_sdf_enabled: boolean;
  // Fase 2 (#43): olho anime + solver de olhar
  eye_depth_scale: number;
  eye_highlight_intensity: number;
  eye_enabled: boolean;
  gaze_tracking_enabled: boolean;
  gaze_saccade_amplitude: number;
  gaze_damping: number;
}

export interface NodeSnapshotWire {
  node_id: string;
  name: string;
  visible: boolean;
  material_id: string | null;
  translation: [number, number, number];
  rotation: [number, number, number, number];
  scale: [number, number, number];
}

export interface RenderSnapshotWire {
  settings_version: number;
  msaa_samples: number;
  background_color: [number, number, number, number];
  color: ColorManagementWire;
  tonemap: TonemapOperatorWire;
}

export interface DynamicStatePayloadWire {
  dynamic_revision: number;
  static_revision: number;
  project_id: string;
  project_name: string;
  character_id: string;
  base_gender: BaseGenderWire;
  gender_dimorphism: number;
  somatotype: { endomorph: number; mesomorph: number; ectomorph: number };
  morph_weights: MorphWeightWire[];
  camera: CameraSnapshotWire;
  lights: LightSnapshotWire[];
  materials: MaterialSnapshotWire[];
  nodes: NodeSnapshotWire[];
  render: RenderSnapshotWire;
  deformation_authority: DeformationAuthority;
  deformation_coverage: DeformationCoverage;
}

export interface CoreSnapshotWire {
  snapshot_version: number;
  static_payload: StaticGeometryPayloadWire | null;
  dynamic: DynamicStatePayloadWire;
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/** Raised when a snapshot coming from the core is malformed. */
export class SnapshotContractError extends Error {
  readonly code: string;
  constructor(code: string, detail: string) {
    super(`[CoreSnapshot ${code}] ${detail}`);
    this.name = "SnapshotContractError";
    this.code = code;
  }
}

/**
 * Decodes base64 into bytes. Works in the browser (`atob`) and under Node
 * (`Buffer`) so the same decoder is exercised by the unit tests.
 */
export function base64ToBytes(base64: string): Uint8Array {
  const globalBuffer = (globalThis as { Buffer?: { from(input: string, encoding: string): { length: number; [index: number]: number } } }).Buffer;
  if (globalBuffer) {
    const buffer = globalBuffer.from(base64, "base64");
    const out = new Uint8Array(buffer.length);
    for (let i = 0; i < buffer.length; i++) out[i] = buffer[i];
    return out;
  }
  const binary = atob(base64);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
  return out;
}

function toAlignedBuffer(bytes: Uint8Array): ArrayBuffer {
  const copy = new Uint8Array(bytes.length);
  copy.set(bytes);
  return copy.buffer;
}

function decodeWithStride(
  base64: string,
  stride: number,
  label: string
): { bytes: Uint8Array; buffer: ArrayBuffer } {
  const bytes = base64ToBytes(base64);
  if (stride > 0 && bytes.length % stride !== 0) {
    throw new SnapshotContractError(
      "BAD_STRIDE",
      `${label} length ${bytes.length} is not a multiple of the ${stride}-byte stride`
    );
  }
  return { bytes, buffer: toAlignedBuffer(bytes) };
}

/** Geometry decoded from the static payload, ready for GPU upload. */
export interface CoreGeometry {
  vertexCount: number;
  indexCount: number;
  vertexStrideBytes: number;
  /** Packed 72-byte vertices (18 floats per vertex) — uploadable as-is. */
  packedVertices: Float32Array;
  /** Triangle indices. */
  indices: Uint32Array;
  /** Base position/normal views extracted for picking and CPU reference paths. */
  positions: Float32Array;
  normals: Float32Array;
  /** Sparse morph channels in catalog order. */
  channels: MorphChannelDescriptorWire[];
  /** 8 floats per delta: [indexBits, dx, dy, dz, dnx, dny, dnz, pad]. */
  deltas: Float32Array;
  /** Raw delta words (bit-exact `vertex_index` access). */
  deltaWords: Uint32Array;
  totalVertices: number;
  totalDeltas: number;
  topologyHash: string;
  catalogFingerprint: string;
  baseGender: BaseGenderWire;
  meshUri: string;
  /** P1-04: paleta de ossos pronta para o uniform buffer (1536 B). */
  skin: DecodedSkin;
}

/** Paleta de ossos decodificada do snapshot. */
export interface DecodedSkin {
  boneCount: number;
  /** `boneCount * 16` floats, layout do shader (colunas). */
  palette: Float32Array;
  bindPose: string;
  /** O núcleo assa as proporções na malha base (skinning não deforma de novo). */
  proportionsBaked: boolean;
  /** A paleta entregue é a identidade. */
  paletteIsIdentity: boolean;
}

/** Decodes the static geometry payload of a snapshot. */
export function decodeStaticGeometry(payload: StaticGeometryPayloadWire): CoreGeometry {
  if (payload.format_version !== SNAPSHOT_FORMAT_VERSION) {
    throw new SnapshotContractError(
      "UNSUPPORTED_VERSION",
      `format_version ${payload.format_version} is not supported (expected ${SNAPSHOT_FORMAT_VERSION})`
    );
  }
  if (payload.vertex_stride_bytes !== VERTEX_STRIDE_BYTES) {
    throw new SnapshotContractError(
      "BAD_VERTEX_STRIDE",
      `vertex_stride_bytes is ${payload.vertex_stride_bytes}, expected ${VERTEX_STRIDE_BYTES}`
    );
  }
  if (payload.morph_delta_stride_bytes !== MORPH_DELTA_STRIDE_BYTES) {
    throw new SnapshotContractError(
      "BAD_DELTA_STRIDE",
      `morph_delta_stride_bytes is ${payload.morph_delta_stride_bytes}, expected ${MORPH_DELTA_STRIDE_BYTES}`
    );
  }

  const vertexBytes = decodeWithStride(
    payload.vertex_buffer_base64,
    VERTEX_STRIDE_BYTES,
    "vertex buffer"
  );
  if (vertexBytes.bytes.length / VERTEX_STRIDE_BYTES !== payload.vertex_count) {
    throw new SnapshotContractError(
      "VERTEX_COUNT_MISMATCH",
      `vertex buffer holds ${vertexBytes.bytes.length / VERTEX_STRIDE_BYTES} vertices, header says ${payload.vertex_count}`
    );
  }
  const indexBytes = decodeWithStride(payload.index_buffer_base64, 4, "index buffer");
  if (indexBytes.bytes.length / 4 !== payload.index_count) {
    throw new SnapshotContractError(
      "INDEX_COUNT_MISMATCH",
      `index buffer holds ${indexBytes.bytes.length / 4} indices, header says ${payload.index_count}`
    );
  }
  const deltaBytes = decodeWithStride(
    payload.morph_deltas_base64,
    MORPH_DELTA_STRIDE_BYTES,
    "delta buffer"
  );
  if (deltaBytes.bytes.length / MORPH_DELTA_STRIDE_BYTES !== payload.morph_total_deltas) {
    throw new SnapshotContractError(
      "DELTA_COUNT_MISMATCH",
      `delta buffer holds ${deltaBytes.bytes.length / MORPH_DELTA_STRIDE_BYTES} deltas, header says ${payload.morph_total_deltas}`
    );
  }

  const packedVertices = new Float32Array(vertexBytes.buffer);
  const indices = new Uint32Array(indexBytes.buffer);
  const deltas = new Float32Array(deltaBytes.buffer);
  const deltaWords = new Uint32Array(deltaBytes.buffer);

  const vertexCount = payload.vertex_count;
  const positions = new Float32Array(vertexCount * 3);
  const normals = new Float32Array(vertexCount * 3);
  for (let v = 0; v < vertexCount; v++) {
    const base = v * 18;
    positions[v * 3] = packedVertices[base];
    positions[v * 3 + 1] = packedVertices[base + 1];
    positions[v * 3 + 2] = packedVertices[base + 2];
    normals[v * 3] = packedVertices[base + 3];
    normals[v * 3 + 1] = packedVertices[base + 4];
    normals[v * 3 + 2] = packedVertices[base + 5];
  }

  // P1-04: a paleta precisa ter exatamente bone_count × 16 floats — uma paleta
  // torta deslocaria todas as matrizes seguintes em silêncio.
  if (!payload.skin) {
    throw new SnapshotContractError(
      "MISSING_SKIN",
      "static payload sem a seção 'skin' (paleta de skinning do núcleo)"
    );
  }
  const boneCount = payload.skin.bone_count;
  const paletteFloats = payload.skin.palette;
  if (!Number.isInteger(boneCount) || boneCount < 1) {
    throw new SnapshotContractError("BAD_SKIN", `skin.bone_count inválido: ${boneCount}`);
  }
  if (!Array.isArray(paletteFloats) || paletteFloats.length !== boneCount * 16) {
    throw new SnapshotContractError(
      "BAD_SKIN",
      `paleta com ${Array.isArray(paletteFloats) ? paletteFloats.length : "?"} floats, ` +
        `esperado ${boneCount * 16} (${boneCount} ossos × 16)`
    );
  }
  const palette = new Float32Array(paletteFloats);
  for (let index = 0; index < palette.length; index++) {
    if (!Number.isFinite(palette[index])) {
      throw new SnapshotContractError("BAD_SKIN", `paleta com valor não finito no índice ${index}`);
    }
  }
  const isIdentity =
    Math.abs(palette[0] - 1) < 1e-5 &&
    Math.abs(palette[5] - 1) < 1e-5 &&
    Math.abs(palette[10] - 1) < 1e-5 &&
    Math.abs(palette[15] - 1) < 1e-5;
  if (payload.skin.palette_is_identity !== isIdentity) {
    throw new SnapshotContractError(
      "BAD_SKIN",
      `skin.palette_is_identity = ${payload.skin.palette_is_identity}, mas a matriz 0 ${isIdentity ? "é" : "não é"} a identidade`
    );
  }

  // Channels must be contiguous and ordered (the shader binary-searches them).
  let expectedOffset = 0;
  for (const channel of payload.morph_channels) {
    if (channel.start_offset !== expectedOffset) {
      throw new SnapshotContractError(
        "NON_CONTIGUOUS_CHANNELS",
        `channel ${channel.slider_id} starts at ${channel.start_offset}, expected ${expectedOffset}`
      );
    }
    if (channel.start_offset + channel.delta_count > payload.morph_total_deltas) {
      throw new SnapshotContractError(
        "CHANNEL_OUT_OF_RANGE",
        `channel ${channel.slider_id} exceeds the delta buffer`
      );
    }
    expectedOffset += channel.delta_count;
  }

  return {
    vertexCount,
    indexCount: payload.index_count,
    vertexStrideBytes: payload.vertex_stride_bytes,
    packedVertices,
    indices,
    positions,
    normals,
    channels: payload.morph_channels,
    deltas,
    deltaWords,
    totalVertices: payload.vertex_count,
    totalDeltas: payload.morph_total_deltas,
    topologyHash: payload.topology_hash,
    catalogFingerprint: payload.catalog_fingerprint,
    baseGender: payload.base_gender,
    meshUri: payload.mesh_uri,
    skin: {
      boneCount,
      palette,
      bindPose: payload.skin.bind_pose,
      proportionsBaked: payload.skin.proportions_baked,
      paletteIsIdentity: payload.skin.palette_is_identity,
    },
  };
}

/** Snapshot decoded and normalized for the viewport. */
export interface DecodedCoreSnapshot {
  snapshotVersion: number;
  dynamicRevision: number;
  staticRevision: number;
  /** Present only when the client's static revision was stale. */
  geometry: CoreGeometry | null;
  /** Channel weights keyed by catalog slider id (only active channels). */
  morphWeights: Map<string, number>;
  /** Morph values keyed by catalog slider id. */
  morphValues: Map<string, number>;
  state: DynamicStatePayloadWire;
  authority: DeformationAuthority;
  coverage: DeformationCoverage;
}

/** Decodes a snapshot coming from the core (geometry decoded when present). */
export function decodeCoreSnapshot(snapshot: CoreSnapshotWire): DecodedCoreSnapshot {
  if (snapshot.snapshot_version !== SNAPSHOT_FORMAT_VERSION) {
    throw new SnapshotContractError(
      "UNSUPPORTED_VERSION",
      `snapshot_version ${snapshot.snapshot_version} is not supported`
    );
  }
  const morphWeights = new Map<string, number>();
  const morphValues = new Map<string, number>();
  for (const entry of snapshot.dynamic.morph_weights ?? []) {
    if (!Number.isFinite(entry.weight) || !Number.isFinite(entry.value)) continue;
    morphValues.set(entry.slider_id, entry.value);
    if (Math.abs(entry.weight) > MORPH_WEIGHT_EPSILON) {
      morphWeights.set(entry.slider_id, entry.weight);
    }
  }

  return {
    snapshotVersion: snapshot.snapshot_version,
    dynamicRevision: snapshot.dynamic.dynamic_revision,
    staticRevision: snapshot.dynamic.static_revision,
    geometry: snapshot.static_payload ? decodeStaticGeometry(snapshot.static_payload) : null,
    morphWeights,
    morphValues,
    state: snapshot.dynamic,
    authority: snapshot.dynamic.deformation_authority,
    coverage: snapshot.dynamic.deformation_coverage,
  };
}

/**
 * Whether the core deformation model can reproduce the whole character.
 * Mirrors `DeformationCoverage::is_complete` in Rust — kept in sync by the
 * contract test, not by convention.
 */
export function coverageIsComplete(coverage: DeformationCoverage): boolean {
  return (
    coverage.bakes_proportions === true &&
    coverage.bakes_gender === true &&
    coverage.somatotype_via_macro_sliders === true &&
    coverage.total_sliders > 0 &&
    coverage.sliders_with_geometry === coverage.total_sliders
  );
}

/** Sliders without core geometry — the exact size of the deformation gap. */
export function missingSliders(coverage: DeformationCoverage): number {
  return Math.max(0, coverage.total_sliders - coverage.sliders_with_geometry);
}

/** Authority implied by a coverage report (mirrors Rust `DeformationCoverage::authority`). */
export function authorityFor(coverage: DeformationCoverage): DeformationAuthority {
  return coverageIsComplete(coverage) ? "core" : "reference_ts";
}

/** Fraction of the catalog backed by core geometry. */
export function coverageRatio(coverage: DeformationCoverage): number {
  if (!coverage.total_sliders) return 0;
  return coverage.sliders_with_geometry / coverage.total_sliders;
}
