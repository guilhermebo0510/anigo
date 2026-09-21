/**
 * ANIGO — contrato do manifesto de exportação v1 (P0 "Consolidar o renderer").
 *
 * Critérios do documento que este módulo atende:
 *
 * * "viewport, exportação e headless usarem os mesmos contratos";
 * * "viewport e exportação passarem teste de paridade".
 *
 * O manifesto é a **identidade verificável** do artefato exportado: ele carrega
 * FNV-1a-64 dos mesmos bytes que o snapshot entrega ao viewport (vértices de
 * 72 B, índices, paleta de skinning) e a contabilidade da deformação aplicada.
 * Com isso o frontend consegue provar, sem receber a malha de volta, que o GLB
 * exportado é a geometria que ele está desenhando.
 *
 * Espelho exato de `crates/anigo-core/src/export.rs` (`ExportManifest`).
 * O fixture congelado é `contracts/fixtures/export_manifest_v1.json` e é
 * validado dos dois lados (Rust e aqui).
 *
 * Nada aqui deforma: são funções de *verificação* (checksum de bytes e
 * comparação de campo). A autoridade da geometria continua sendo o núcleo.
 */

import { fnv1a64 } from "./render_contract.v1";

/** Versão do documento que este módulo sabe interpretar. */
export const EXPORT_FORMAT_VERSION = 1;

/** Quem gera o artefato (rastro auditável; o núcleo é o único autor). */
export const EXPORT_GENERATOR = "anigo-core/export";

/** Bytes por vértice no layout canônico (posição, normal, uv, cor, skin). */
export const EXPORT_VERTEX_STRIDE_BYTES = 72;

export type ExportGenderWire = "Male" | "Female";

export interface ExportProjectInfoWire {
  project_id: string;
  project_name: string;
  character_id: string;
  base_gender: ExportGenderWire;
  static_revision: number;
  dynamic_revision: number;
  snapshot_version: number;
}

export interface ExportGeometryInfoWire {
  vertex_count: number;
  index_count: number;
  triangle_count: number;
  vertex_stride_bytes: number;
  /** FNV-1a-64 das posições + índices da base (`snapshot::mesh_topology_hash`). */
  topology_hash: string;
  /** FNV-1a-64 dos bytes de `pack_vertices` da base (72 B por vértice). */
  base_vertex_checksum: string;
  /** FNV-1a-64 dos bytes de `pack_indices` (u32 LE). */
  base_index_checksum: string;
  catalog_fingerprint: string;
  mesh_uri: string;
  mesh_asset_id: string;
}

export interface ExportMorphWeightWire {
  target: string;
  slider_id: string;
  value: number;
  weight: number;
}

export interface ExportDeformationInfoWire {
  /** Autoridade da deformação (`core`). */
  authority: string;
  morph_total_deltas: number;
  active_channels: number;
  morph_weights: ExportMorphWeightWire[];
  /** FNV-1a-64 dos 72 B por vértice da malha **deformada**. */
  deformed_vertex_checksum: string;
  /** FNV-1a-64 das posições (12 B por vértice) da malha deformada. */
  deformed_position_checksum: string;
}

export interface ExportSkinInfoWire {
  bone_count: number;
  bind_pose: string;
  palette_is_identity: boolean;
  /** FNV-1a-64 dos floats da paleta (f32 LE, 16 por osso). */
  palette_checksum: string;
  skinned_vertices: number;
  unskinned_vertices: number;
}

export interface ExportRenderInfoWire {
  width: number;
  height: number;
  color_format: string;
  depth_format: string;
  msaa_samples: number;
  render_passes: string[];
  clear_source: string;
  clear_color: number[];
  adapter_name: string;
  backend: string;
  /** Métrica opcional emitida por versões do renderer/headless. */
  draw_calls?: number;
  triangle_count?: number;
  render_time_ms: number;
}

export interface ExportManifestWire {
  format_version: number;
  generator: string;
  project: ExportProjectInfoWire;
  geometry: ExportGeometryInfoWire;
  deformation: ExportDeformationInfoWire;
  skin: ExportSkinInfoWire;
  render?: ExportRenderInfoWire;
}

/** Falha explícita e recuperável (o editor não corrompe estado por causa dela). */
export class ExportManifestError extends Error {
  readonly recoverable = true;
  readonly code: string;

  constructor(code: string, message: string) {
    super(`[ANIGO export manifest ${code}] ${message}`);
    this.code = code;
    this.name = "ExportManifestError";
  }
}

const CHECKSUM = /^[0-9a-f]{16}$/;

function requireObject(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new ExportManifestError("NOT_OBJECT", `${label} precisa ser um objeto`);
  }
  return value as Record<string, unknown>;
}

function requireString(source: Record<string, unknown>, key: string, label: string): string {
  const value = source[key];
  if (typeof value !== "string" || value.length === 0) {
    throw new ExportManifestError("INVALID_FIELD", `${label}.${key} precisa ser uma string não vazia`);
  }
  return value;
}

function requireFinite(source: Record<string, unknown>, key: string, label: string): number {
  const value = source[key];
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new ExportManifestError("INVALID_FIELD", `${label}.${key} precisa ser um número finito`);
  }
  return value;
}

function requireChecksum(source: Record<string, unknown>, key: string, label: string): string {
  const value = requireString(source, key, label);
  if (!CHECKSUM.test(value)) {
    throw new ExportManifestError(
      "INVALID_CHECKSUM",
      `${label}.${key} precisa ser FNV-1a-64 em hex de 16 dígitos ('${value}')`
    );
  }
  return value;
}

/**
 * Valida a forma do manifesto exportado.
 *
 * Recusa versão desconhecida, campo faltando e checksum fora do formato — o
 * mesmo contrato que o Rust valida do outro lado.
 */
export function readExportManifest(value: unknown): ExportManifestWire {
  const manifest = requireObject(value, "manifest");
  const version = requireFinite(manifest, "format_version", "manifest");
  if (version !== EXPORT_FORMAT_VERSION) {
    throw new ExportManifestError(
      "UNSUPPORTED_VERSION",
      `format_version ${version} não é suportada (esperado ${EXPORT_FORMAT_VERSION})`
    );
  }
  const generator = requireString(manifest, "generator", "manifest");
  if (generator !== EXPORT_GENERATOR) {
    throw new ExportManifestError(
      "UNKNOWN_GENERATOR",
      `generator '${generator}' não é o núcleo canônico ('${EXPORT_GENERATOR}')`
    );
  }

  const project = requireObject(manifest.project, "project");
  const geometry = requireObject(manifest.geometry, "geometry");
  const deformation = requireObject(manifest.deformation, "deformation");
  const skin = requireObject(manifest.skin, "skin");

  const gender = requireString(project, "base_gender", "project");
  if (gender !== "Male" && gender !== "Female") {
    throw new ExportManifestError("INVALID_FIELD", `project.base_gender '${gender}' é inválido`);
  }

  const stride = requireFinite(geometry, "vertex_stride_bytes", "geometry");
  if (stride !== EXPORT_VERTEX_STRIDE_BYTES) {
    throw new ExportManifestError(
      "INVALID_STRIDE",
      `geometry.vertex_stride_bytes = ${stride} (o layout canônico tem ${EXPORT_VERTEX_STRIDE_BYTES})`
    );
  }
  const vertexCount = requireFinite(geometry, "vertex_count", "geometry");
  const indexCount = requireFinite(geometry, "index_count", "geometry");
  if (vertexCount <= 0 || indexCount <= 0 || indexCount % 3 !== 0) {
    throw new ExportManifestError(
      "INVALID_FIELD",
      `geometria exportada inconsistente: ${vertexCount} vértices, ${indexCount} índices`
    );
  }

  const morphWeights = deformation.morph_weights;
  if (!Array.isArray(morphWeights)) {
    throw new ExportManifestError("INVALID_FIELD", "deformation.morph_weights precisa ser uma lista");
  }
  for (const entry of morphWeights) {
    const weight = requireObject(entry, "deformation.morph_weights[]");
    requireString(weight, "slider_id", "deformation.morph_weights[]");
    requireFinite(weight, "weight", "deformation.morph_weights[]");
    requireFinite(weight, "value", "deformation.morph_weights[]");
  }

  const skinned = requireFinite(skin, "skinned_vertices", "skin");
  const unskinned = requireFinite(skin, "unskinned_vertices", "skin");
  if (skinned + unskinned !== vertexCount) {
    throw new ExportManifestError(
      "INVALID_FIELD",
      `skin cobre ${skinned + unskinned} vértices, o manifesto declara ${vertexCount}`
    );
  }

  const manifestOut: ExportManifestWire = {
    format_version: version,
    generator,
    project: {
      project_id: requireString(project, "project_id", "project"),
      project_name: requireString(project, "project_name", "project"),
      character_id: requireString(project, "character_id", "project"),
      base_gender: gender,
      static_revision: requireFinite(project, "static_revision", "project"),
      dynamic_revision: requireFinite(project, "dynamic_revision", "project"),
      snapshot_version: requireFinite(project, "snapshot_version", "project"),
    },
    geometry: {
      vertex_count: vertexCount,
      index_count: indexCount,
      triangle_count: requireFinite(geometry, "triangle_count", "geometry"),
      vertex_stride_bytes: stride,
      topology_hash: requireChecksum(geometry, "topology_hash", "geometry"),
      base_vertex_checksum: requireChecksum(geometry, "base_vertex_checksum", "geometry"),
      base_index_checksum: requireChecksum(geometry, "base_index_checksum", "geometry"),
      catalog_fingerprint: requireChecksum(geometry, "catalog_fingerprint", "geometry"),
      mesh_uri: requireString(geometry, "mesh_uri", "geometry"),
      mesh_asset_id: requireString(geometry, "mesh_asset_id", "geometry"),
    },
    deformation: {
      authority: requireString(deformation, "authority", "deformation"),
      morph_total_deltas: requireFinite(deformation, "morph_total_deltas", "deformation"),
      active_channels: requireFinite(deformation, "active_channels", "deformation"),
      morph_weights: morphWeights.map((entry) => {
        const weight = entry as Record<string, unknown>;
        return {
          target: requireString(weight, "target", "deformation.morph_weights[]"),
          slider_id: requireString(weight, "slider_id", "deformation.morph_weights[]"),
          value: requireFinite(weight, "value", "deformation.morph_weights[]"),
          weight: requireFinite(weight, "weight", "deformation.morph_weights[]"),
        };
      }),
      deformed_vertex_checksum: requireChecksum(
        deformation,
        "deformed_vertex_checksum",
        "deformation"
      ),
      deformed_position_checksum: requireChecksum(
        deformation,
        "deformed_position_checksum",
        "deformation"
      ),
    },
    skin: {
      bone_count: requireFinite(skin, "bone_count", "skin"),
      bind_pose: requireString(skin, "bind_pose", "skin"),
      palette_is_identity:
        skin.palette_is_identity === true || skin.palette_is_identity === false
          ? (skin.palette_is_identity as boolean)
          : (() => {
              throw new ExportManifestError(
                "INVALID_FIELD",
                "skin.palette_is_identity precisa ser booleano"
              );
            })(),
      palette_checksum: requireChecksum(skin, "palette_checksum", "skin"),
      skinned_vertices: skinned,
      unskinned_vertices: unskinned,
    },
  };

  if (manifest.render !== undefined) {
    const render = requireObject(manifest.render, "render");
    const passes = render.render_passes;
    const clearColor = render.clear_color;
    if (!Array.isArray(passes) || !passes.every((entry) => typeof entry === "string")) {
      throw new ExportManifestError("INVALID_FIELD", "render.render_passes precisa ser uma lista de strings");
    }
    if (!Array.isArray(clearColor) || clearColor.length !== 4 || !clearColor.every((c) => typeof c === "number")) {
      throw new ExportManifestError("INVALID_FIELD", "render.clear_color precisa ter 4 números");
    }
    manifestOut.render = {
      width: requireFinite(render, "width", "render"),
      height: requireFinite(render, "height", "render"),
      color_format: requireString(render, "color_format", "render"),
      depth_format: requireString(render, "depth_format", "render"),
      msaa_samples: requireFinite(render, "msaa_samples", "render"),
      render_passes: passes as string[],
      clear_source: requireString(render, "clear_source", "render"),
      clear_color: clearColor as number[],
      adapter_name: requireString(render, "adapter_name", "render"),
      backend: requireString(render, "backend", "render"),
      render_time_ms: requireFinite(render, "render_time_ms", "render"),
    };
  }

  return manifestOut;
}

// ---------------------------------------------------------------------------
// Checksums dos bytes canônicos
//
// O núcleo calcula FNV-1a-64 sobre os bytes little-endian que empacota em
// `pack_vertices`/`pack_indices`. Aqui os mesmos bytes são reconstruídos a
// partir das views do viewport — com `DataView` explicitamente little-endian,
// para que a comparação não dependa da plataforma.
// ---------------------------------------------------------------------------

/** Bytes LE de `vertexCount * 72` a partir dos floats empacotados (18 por vértice). */
export function packedVertexBytes(vertices: Float32Array, vertexCount: number): Uint8Array {
  const floatsPerVertex = EXPORT_VERTEX_STRIDE_BYTES / 4;
  const needed = vertexCount * floatsPerVertex;
  if (!Number.isInteger(vertexCount) || vertexCount <= 0) {
    throw new ExportManifestError("INVALID_COUNT", `vertex_count inválido: ${vertexCount}`);
  }
  if (vertices.length < needed) {
    throw new ExportManifestError(
      "SHORT_BUFFER",
      `buffer de vértices com ${vertices.length} floats, esperado ${needed} (${vertexCount} × ${floatsPerVertex})`
    );
  }
  const bytes = new Uint8Array(vertexCount * EXPORT_VERTEX_STRIDE_BYTES);
  const view = new DataView(bytes.buffer);
  for (let index = 0; index < needed; index++) {
    view.setFloat32(index * 4, vertices[index], true);
  }
  return bytes;
}

/** Bytes LE das posições (12 por vértice) — offset 0 do layout de 72 B. */
export function positionBytes(vertices: Float32Array, vertexCount: number): Uint8Array {
  const floatsPerVertex = EXPORT_VERTEX_STRIDE_BYTES / 4;
  const bytes = new Uint8Array(vertexCount * 12);
  const view = new DataView(bytes.buffer);
  for (let vertex = 0; vertex < vertexCount; vertex++) {
    for (let axis = 0; axis < 3; axis++) {
      view.setFloat32(vertex * 12 + axis * 4, vertices[vertex * floatsPerVertex + axis], true);
    }
  }
  return bytes;
}

/** Bytes LE dos índices (u32). */
export function indexBytes(indices: Uint32Array): Uint8Array {
  const bytes = new Uint8Array(indices.length * 4);
  const view = new DataView(bytes.buffer);
  for (let index = 0; index < indices.length; index++) {
    view.setUint32(index * 4, indices[index], true);
  }
  return bytes;
}

/** Bytes LE dos floats da paleta de skinning. */
export function paletteBytes(palette: Float32Array): Uint8Array {
  const bytes = new Uint8Array(palette.length * 4);
  const view = new DataView(bytes.buffer);
  for (let index = 0; index < palette.length; index++) {
    view.setFloat32(index * 4, palette[index], true);
  }
  return bytes;
}

/** FNV-1a-64 (hex de 16 dígitos) — mesmo algoritmo do núcleo. */
export function bufferChecksum(bytes: Uint8Array): string {
  return fnv1a64(bytes);
}

/** Atalhos usados pelo teste de paridade (e pelo app shell). */
export function packedVertexChecksum(vertices: Float32Array, vertexCount: number): string {
  return bufferChecksum(packedVertexBytes(vertices, vertexCount));
}

export function indexChecksum(indices: Uint32Array): string {
  return bufferChecksum(indexBytes(indices));
}

export function paletteChecksum(palette: Float32Array): string {
  return bufferChecksum(paletteBytes(palette));
}

/** Checksum das posições empacotadas no layout canônico de 72 B. */
export function packedPositionChecksum(vertices: Float32Array, vertexCount: number): string {
  const floatsPerVertex = EXPORT_VERTEX_STRIDE_BYTES / 4;
  const bytes = new Uint8Array(vertexCount * 12);
  const view = new DataView(bytes.buffer);
  for (let vertex = 0; vertex < vertexCount; vertex++) {
    for (let axis = 0; axis < 3; axis++) {
      view.setFloat32(vertex * 12 + axis * 4, vertices[vertex * floatsPerVertex + axis], true);
    }
  }
  return bufferChecksum(bytes);
}
