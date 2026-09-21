/**
 * ANIGO — API de interoperabilidade glTF 2.0 / VRM 1.0 (Fase 2, issue #25).
 *
 * Fachada única usada pelo Tauri (`src-tauri/src/main.rs`), pelo viewport e
 * pelos testes:
 *
 *   - `parseModel`      — bytes `.glb`/`.vrm` → modelo parseado (+ VRM se for);
 *   - `parseGltfFile`   — JSON `.gltf` + buffers externos → modelo parseado;
 *   - `parseVrm`        — bytes `.vrm` → { model, vrm } (falha se não for VRM);
 *   - `validateModel`   — validação estrutural com resumo (nada de silêncio);
 *   - `exportModel`     — cena normalizada → bytes `.glb`/`.vrm` (determinístico).
 */

import { parseGlbContainer } from "../gltf_loader";
import { GltfError, parseGltfDocument } from "./gltf2";
import type { ExternalBufferSource, Gltf, ParsedGltfModel } from "./gltf2";
import { exportGlb, exportGltf, summarizeImport } from "./exporter";
import type { ExportScene, ImportSummary, VrmExportData } from "./exporter";
import { Vrm1Error, isVrmDocument, parseVrm1 } from "./vrm1";
import type { ParsedVrm1 } from "./vrm1";

export {
  GltfError,
  GLB_CHUNK_BIN,
  GLB_CHUNK_JSON,
  GLB_MAGIC,
  GLB_VERSION,
  GLTF_COMPONENT_TYPES,
  GLTF_PRIMITIVE_MODES,
  accessorByteSize,
  accessorComponentCount,
  mipLevelCount,
  parseGltfDocument,
  readAccessorFloats,
  readAccessorIndices,
  readImageBytes,
  resolveBuffers,
} from "./gltf2";
export { buildGlb, exportGlb, exportGltf, summarizeImport } from "./exporter";
export type {
  ExportAnimation,
  ExportMaterial,
  ExportMesh,
  ExportNode,
  ExportPrimitive,
  ExportScene,
  ExportSkin,
  ExportVertex,
  GltfMaterial,
  GltfMesh,
  GltfNode,
  ImportSummary,
  VrmExportData,
} from "./exporter";
export type { ExportOptions as VrmExporterOptions } from "./exporter";
export type {
  ExternalBufferSource,
  Gltf,
  GltfAnimation,
  GltfAnimationSampler,
  GltfAccessor,
  GltfBuffer,
  GltfBufferView,
  GltfErrorCode,
  GltfImage,
  GltfMaterial as GltfMaterialDoc,
  GltfSampler,
  GltfScene,
  GltfSkin,
  GltfSparseIndices,
  GltfSparseValues,
  GltfTexture,
  ParsedGltfModel,
} from "./gltf2";
export {
  VRM1_EXPRESSION_PRESETS,
  Vrm1Error,
  isVrmDocument,
  mapMToonToAnimeMaterial,
  parseVrm1,
} from "./vrm1";
export type {
  MToonMaterialMapping,
  ParsedVrm1,
  Vrm1CapsuleCollider,
  Vrm1Colliders,
  Vrm1Expression,
  Vrm1ExpressionPresetName,
  Vrm1Humanoid,
  Vrm1Meta,
  Vrm1MToon,
  Vrm1NodeConstraint,
  Vrm1SpringBone,
  Vrm1SpringGroup,
  Vrm1SphereCollider,
} from "./vrm1";

// ---------------------------------------------------------------------------
// Resolção de GLB → buffers (reusa o container parser validado do loader P0-10)
// ---------------------------------------------------------------------------

function glbToModel(json: Gltf, bin: ArrayBuffer): ParsedGltfModel {
  const source = new Map<string, Uint8Array | ArrayBuffer>();
  source.set("__embedded_bin__", bin);
  // GLB: buffers sem `uri` usam o chunk BIN; buffers com uri seguem sendo
  // externos (spec permite GLB com buffers externos — raro, mas legal).
  const buffers = json.buffers ?? [];
  let uriless = 0;
  for (let i = 0; i < buffers.length; i++) {
    if (buffers[i].uri === undefined) {
      uriless += 1;
      buffers[i] = { ...buffers[i], uri: "__embedded_bin__" };
    }
  }
  const model = parseGltfDocument(json, source);
  if (uriless > 1) {
    model.warnings.push("GLB com múltiplos buffers sem uri — a spec permite no máximo um no chunk BIN");
  }
  return model;
}

export interface ParseModelResult {
  model: ParsedGltfModel;
  /** Presente apenas quando o documento é um .vrm (VRMC_vrm). */
  vrm: ParsedVrm1 | null;
}

/**
 * Parse completo de bytes GLB/VRM: contêiner → glTF → camadas VRM 1.0.
 * Erros de contêiner viram `GltfError` com código estável.
 */
export function parseModel(bytes: Uint8Array | ArrayBuffer): ParseModelResult {
  const buffer = bytes instanceof Uint8Array ? bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) : bytes;
  let json: Gltf;
  let bin: ArrayBuffer;
  try {
    const container = parseGlbContainer(buffer as ArrayBuffer);
    json = container.json as unknown as Gltf;
    bin = container.bin;
  } catch (error) {
    if (error instanceof GltfError) throw error;
    const code = (error as { code?: string })?.code;
    const mapped: Record<string, GltfError["code"]> = {
      TOO_SMALL: "BAD_JSON",
      BAD_MAGIC: "BAD_JSON",
      BAD_VERSION: "UNSUPPORTED_VERSION",
      BAD_LENGTH: "BAD_JSON",
      MISSING_JSON: "BAD_JSON",
      BAD_JSON: "BAD_JSON",
    };
    throw new GltfError(mapped[code ?? ""] ?? "BAD_JSON", String((error as Error)?.message ?? error));
  }
  const model = glbToModel(json, bin);
  const vrm = isVrmDocument(model.gltf) ? parseVrm1(model) : null;
  return { model, vrm };
}

/** Parse de `.gltf` (JSON) com buffers externos resolvidos pelo caller. */
export function parseGltfFile(jsonText: string, externalBuffers?: ExternalBufferSource): ParseModelResult {
  let json: Gltf;
  try {
    json = JSON.parse(jsonText) as Gltf;
  } catch (error) {
    throw new GltfError("BAD_JSON", `JSON inválido: ${(error as Error).message}`);
  }
  const model = parseGltfDocument(json, externalBuffers ?? null);
  const vrm = isVrmDocument(model.gltf) ? parseVrm1(model) : null;
  return { model, vrm };
}

/** Parse de `.vrm` — falha com `Vrm1Error` quando o documento não é VRM 1.0. */
export function parseVrm(bytes: Uint8Array | ArrayBuffer): { model: ParsedGltfModel; vrm: ParsedVrm1 } {
  const result = parseModel(bytes);
  if (result.vrm === null) throw new Vrm1Error("NOT_VRM", "bytes não formam um .vrm válido (sem VRMC_vrm)");
  return { model: result.model, vrm: result.vrm };
}

// ---------------------------------------------------------------------------
// Validação (comando Tauri `validate_model`)
// ---------------------------------------------------------------------------

export interface ValidationResult {
  valid: boolean;
  error: { code: string; message: string } | null;
  summary: ImportSummary | null;
}

/**
 * Validação estrutural completa: contêiner + glTF + camadas VRM.
 * Nunca lança — o resultado é transportável pela fronteira Tauri.
 */
export function validateModel(bytes: Uint8Array | ArrayBuffer): ValidationResult {
  try {
    const { model, vrm } = parseModel(bytes);
    return { valid: true, error: null, summary: summarizeImport(model.gltf) };
  } catch (error) {
    const code =
      error instanceof GltfError || error instanceof Vrm1Error
        ? error.code
        : "UNKNOWN_ERROR";
    return { valid: false, error: { code, message: String((error as Error)?.message ?? error) }, summary: null };
  }
}

// ---------------------------------------------------------------------------
// Export (comando Tauri `export_model`)
// ---------------------------------------------------------------------------

export interface ExportOptions {
  /** Presente para emitir `.vrm` (camada VRM 1.0). */
  vrm?: VrmExportData;
  /** URI do buffer binário (para `.gltf` com buffer externo). */
  bufferUri?: string;
}

/** Exporta a cena para bytes `.glb` (ou `.vrm`). Determinístico. */
export function exportModel(scene: ExportScene, options: ExportOptions = {}): Uint8Array {
  return exportGlb(scene, options.vrm, { bufferUri: options.bufferUri });
}
